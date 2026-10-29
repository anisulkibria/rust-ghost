//! Signed identity tokens, and the public key to check them with.
//!
//! Three endpoints that exist so another service can trust something this
//! site says, without being given a key of its own:
//!
//! * `GET /ghost/api/admin/identities` — a short-lived token asserting
//!   "this is staff member X on this site". Ghost's admin uses it to hand off
//!   to a service it does not share credentials with.
//! * `GET /members/api/entitlements` — the same idea for a signed-in member:
//!   "this reader has these tiers". Something that gates content outside the
//!   theme can verify it rather than being told to trust a header.
//! * `GET /.well-known/jwks.json` — the public half, so either token can be
//!   verified by anybody without asking this server.
//!
//! RS256 rather than a shared secret, precisely because the verifier should
//! not need anything secret. The keypair is this site's own, kept apart from
//! the ActivityPub one: reusing a key across two protocols means a weakness
//! in either is a weakness in both.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use base64::Engine;
use ghost_core::{dates, GhostError, Result};
use rsa::{
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
    traits::PublicKeyParts,
    RsaPrivateKey, RsaPublicKey,
};
use serde_json::json;

const B64URL: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// How long an identity token is good for. Ghost's is minutes, not hours:
/// it is a hand-off, not a credential to keep.
const TOKEN_VALIDITY_SECS: i64 = 5 * 60;

pub fn admin_router() -> Router<AppState> {
    Router::new().route("/identities", get(identities)).route("/identities/", get(identities))
}

/// Mounted at the site root: a verifier has to be able to fetch the key
/// without being signed in to anything.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/.well-known/jwks.json", get(jwks))
        .route("/members/api/entitlements", get(entitlements))
        .route("/members/api/entitlements/", get(entitlements))
}

static KEY_CREATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// This site's signing keypair, generated on first use.
///
/// Serialised behind a mutex because two simultaneous first requests would
/// otherwise each generate a key and one would win, invalidating tokens the
/// other had already signed.
pub async fn keys(state: &AppState) -> Result<(String, String)> {
    let _lock = KEY_CREATION.lock().await;
    if let Some(row) =
        sqlx::query_as::<_, (String, String)>("SELECT private_pem, public_pem FROM signing_keys WHERE id = 'identity'")
            .fetch_optional(&state.db)
            .await
            .map_err(|e| GhostError::Db(e.to_string()))?
    {
        return Ok(row);
    }
    // 2048 bits takes long enough to block a worker thread noticeably.
    let generated = tokio::task::spawn_blocking(|| {
        let private = RsaPrivateKey::new(&mut rand_08::rngs::OsRng, 2048).map_err(|e| e.to_string())?;
        let public = RsaPublicKey::from(&private).to_public_key_pem(LineEnding::LF).map_err(|e| e.to_string())?;
        let private = private.to_pkcs8_pem(LineEnding::LF).map_err(|e| e.to_string())?.to_string();
        Ok::<_, String>((private, public))
    })
    .await
    .map_err(|e| GhostError::Internal(e.to_string()))?
    .map_err(GhostError::Internal)?;
    sqlx::query("INSERT INTO signing_keys (id, private_pem, public_pem, created_at) VALUES ('identity', ?, ?, ?)")
        .bind(&generated.0)
        .bind(&generated.1)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(generated)
}

/// The key's id, as it appears in both the JWKS and a token's header.
///
/// Derived from the public key, so it changes when the key does and a
/// verifier caching the old JWKS can tell that it needs to refetch.
fn key_id(public_pem: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(public_pem.as_bytes()))[..16].to_string()
}

/// Sign a set of claims as an RS256 JWT.
fn sign(private_pem: &str, key_id: &str, claims: &serde_json::Value) -> Result<String> {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(key_id.to_string());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_pem.as_bytes())
        .map_err(|e| GhostError::Internal(format!("identity key is unusable: {e}")))?;
    jsonwebtoken::encode(&header, claims, &key).map_err(|e| GhostError::Internal(e.to_string()))
}

/// `GET /identities` — a token asserting who the signed-in staff member is.
async fn identities(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match permissions::require(&state, &headers, Role::Author).await {
        Ok(user) => user,
        Err(r) => return r,
    };
    let (private, public) = match keys(&state).await {
        Ok(keys) => keys,
        Err(e) => {
            tracing::error!("identities: no signing key: {}", e.message());
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let now = chrono::Utc::now().timestamp();
    let site = state.site_url();
    let claims = json!({
        // `sub` is the email rather than the id, because that is the
        // identifier a receiving service can match a person on.
        "sub": user.email,
        "kid": key_id(&public),
        "iss": format!("{site}/ghost/api/admin/"),
        "aud": site,
        "iat": now,
        "exp": now + TOKEN_VALIDITY_SECS,
    });
    match sign(&private, &key_id(&public), &claims) {
        Ok(token) => {
            ([("cache-control", "private, no-store")], Json(json!({"identities": [{"token": token}]}))).into_response()
        }
        Err(e) => {
            tracing::error!("identities: {}", e.message());
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /members/api/entitlements` — what the signed-in reader is entitled to.
///
/// Signed so that whatever reads it does not have to trust the caller. A
/// visitor who is not signed in gets a 401 rather than an anonymous token:
/// "no entitlements" and "not signed in" are different answers.
async fn entitlements(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(member) = crate::members::current_member(&state, &headers).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"errors":[{"message":"Unable to find member","errorType":"UnauthorizedError"}]})),
        )
            .into_response();
    };
    let (private, public) = match keys(&state).await {
        Ok(keys) => keys,
        Err(e) => {
            tracing::error!("entitlements: no signing key: {}", e.message());
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let tiers: Vec<String> = sqlx::query_scalar(
        "SELECT p.slug FROM members_products mp JOIN products p ON p.id = mp.product_id \
         WHERE mp.member_id = ? AND (mp.expiry_at IS NULL OR datetime(mp.expiry_at) > datetime('now'))",
    )
    .bind(&member.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    let site = state.site_url();
    let claims = json!({
        "sub": member.uuid,
        "email": member.email,
        "status": member.status,
        "tiers": tiers,
        "iss": format!("{site}/members/api/"),
        "aud": site,
        "iat": now,
        "exp": now + TOKEN_VALIDITY_SECS,
    });
    match sign(&private, &key_id(&public), &claims) {
        Ok(token) => ([("cache-control", "private, no-store")], Json(json!({"token": token}))).into_response(),
        Err(e) => {
            tracing::error!("entitlements: {}", e.message());
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /.well-known/jwks.json` — the public half of the signing key.
async fn jwks(State(state): State<AppState>) -> Response {
    let Ok((_, public_pem)) = keys(&state).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Some(jwk) = jwk_of(&public_pem) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // Cacheable, and meant to be cached: a verifier refetches when it meets
    // a `kid` it does not know.
    ([("cache-control", "public, max-age=3600")], Json(json!({"keys": [jwk]}))).into_response()
}

/// An RSA public key as a JWK.
///
/// The modulus and exponent are big-endian, base64url, unpadded — RFC 7518's
/// encoding, and the part that is easy to get subtly wrong.
pub fn jwk_of(public_pem: &str) -> Option<serde_json::Value> {
    use rsa::pkcs8::DecodePublicKey;
    let public = RsaPublicKey::from_public_key_pem(public_pem).ok()?;
    Some(json!({
        "kty": "RSA",
        "use": "sig",
        "alg": "RS256",
        "kid": key_id(public_pem),
        "n": B64URL.encode(public.n().to_bytes_be()),
        "e": B64URL.encode(public.e().to_bytes_be()),
    }))
}

/// Verify a token this site issued. Used by the tests, and by anything
/// in-process that would otherwise re-implement the check.
pub fn verify(public_pem: &str, token: &str, audience: &str) -> Option<serde_json::Value> {
    let key = jsonwebtoken::DecodingKey::from_rsa_pem(public_pem.as_bytes()).ok()?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[audience]);
    jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation).ok().map(|data| data.claims)
}

/// A keypair for a test, without a database behind it.
#[doc(hidden)]
pub fn generate_for_test() -> (String, String) {
    let private = RsaPrivateKey::new(&mut rand_08::rngs::OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private).to_public_key_pem(LineEnding::LF).unwrap();
    (private.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(), public)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_verifies_against_the_published_key_and_nothing_else() {
        let (private, public) = generate_for_test();
        let kid = key_id(&public);
        let now = chrono::Utc::now().timestamp();
        let claims = json!({"sub":"staff@example.test","aud":"https://example.test","iat":now,"exp":now+300});
        let token = sign(&private, &kid, &claims).unwrap();

        let verified = verify(&public, &token, "https://example.test").unwrap();
        assert_eq!(verified["sub"], "staff@example.test");
        // A different audience is a different site; the token is not for it.
        assert!(verify(&public, &token, "https://other.test").is_none());
        // And a different key cannot vouch for it.
        let (_, other) = generate_for_test();
        assert!(verify(&other, &token, "https://example.test").is_none());

        // An expired token is refused.
        let stale = sign(&private, &kid, &json!({"sub":"x","aud":"https://example.test","iat":now-600,"exp":now-300})).unwrap();
        assert!(verify(&public, &stale, "https://example.test").is_none());
    }

    #[test]
    fn the_published_key_is_a_well_formed_jwk_naming_the_key_that_signed() {
        let (private, public) = generate_for_test();
        let jwk = jwk_of(&public).unwrap();
        assert_eq!(jwk["kty"], "RSA");
        assert_eq!(jwk["alg"], "RS256");
        assert_eq!(jwk["use"], "sig");
        // The id in the key matches the one in a token's header, which is how
        // a verifier picks the right key out of the set.
        let kid = jwk["kid"].as_str().unwrap();
        let token = sign(&private, kid, &json!({"sub":"x","aud":"a","exp":chrono::Utc::now().timestamp()+60})).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.kid.as_deref(), Some(kid));
        assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);

        // Base64url, unpadded, and decodes to a 2048-bit modulus.
        let n = jwk["n"].as_str().unwrap();
        assert!(!n.contains('=') && !n.contains('+') && !n.contains('/'), "{n}");
        assert_eq!(B64URL.decode(n).unwrap().len(), 256);
        assert_eq!(B64URL.decode(jwk["e"].as_str().unwrap()).unwrap(), vec![1, 0, 1]);

        // A different key is a different id, so a cached set can be told
        // apart from a rotated one.
        let (_, other) = generate_for_test();
        assert_ne!(jwk_of(&other).unwrap()["kid"], jwk["kid"]);
    }
}
