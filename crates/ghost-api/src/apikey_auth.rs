//! Admin API key authentication — `Authorization: Ghost <JWT>`.
//!
//! This is how every non-browser client reaches the Admin API: Zapier, a
//! headless frontend publishing posts, a deploy script, the official
//! `@tryghost/admin-api` SDK. Without it the Admin API is reachable only
//! from a browser holding a session cookie, which is to say not reachable by
//! an integration at all.
//!
//! Ghost's scheme (`core/server/auth/api-key/admin.js`), reproduced exactly
//! because the SDKs mint these tokens and will not negotiate:
//!
//! * `Authorization: Ghost <token>`, the scheme matched case-insensitively.
//! * HS256. The signing key is the api_key's `secret` **decoded from hex into
//!   bytes**, not the hex text — signing the text produces a token every
//!   Ghost client would reject, and vice versa.
//! * The `kid` header names the api_key row, so the key is found before the
//!   signature is checked.
//! * `aud` must match the API path being called, which stops a token minted
//!   for the Content API being replayed against the Admin API.
//! * `maxAge` 5 minutes, measured from `iat`. Ghost's SDKs mint short-lived
//!   tokens per request for exactly this reason.
//! * The key must be of type `admin`.

use ghost_db::repos;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::AppState;

/// What a verified token grants.
pub struct VerifiedKey {
    pub key_id: String,
    /// The staff user the key acts as, when it names one.
    pub user_id: Option<String>,
    pub integration_id: Option<String>,
}

/// Pull the token out of an `Authorization: Ghost <token>` header.
///
/// Returns None for any other scheme so a session-cookie request, or a
/// `Bearer` token meant for something else, falls through untouched rather
/// than erroring.
pub fn token_from_header(header: &str) -> Option<&str> {
    let (scheme, token) = header.split_once(' ')?;
    scheme.eq_ignore_ascii_case("Ghost").then(|| token.trim()).filter(|t| !t.is_empty())
}

/// Verify a token and resolve the key behind it.
///
/// `api_path` is the API the request is actually hitting (`admin` or
/// `content`); the token's `aud` has to agree.
pub async fn verify(state: &AppState, token: &str, api_path: &str) -> Option<VerifiedKey> {
    let mut parts = token.split('.');
    let (header_b64, claims_b64, sig_b64) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }

    let header: Value = serde_json::from_slice(&b64url::decode(header_b64)?).ok()?;
    if header.get("alg").and_then(|a| a.as_str()) != Some("HS256") {
        // Refusing anything else is what stops the `alg: none` forgery.
        return None;
    }
    let kid = header.get("kid").and_then(|k| k.as_str())?;

    let key = repos::api_keys::find_by_id(&state.db, kid).await.ok().flatten()?;
    if key.key_type != "admin" {
        return None;
    }

    // Ghost signs with the secret's *bytes*, decoded from its hex text.
    let secret = hex::decode(&key.secret).ok()?;
    let expected = hmac_sha256(&secret, format!("{header_b64}.{claims_b64}").as_bytes());
    let given = b64url::decode(sig_b64)?;
    if !constant_time_eq(&expected, &given) {
        return None;
    }

    let claims: Value = serde_json::from_slice(&b64url::decode(claims_b64)?).ok()?;
    if !claims_valid_at(&claims, api_path, chrono::Utc::now().timestamp()) {
        return None;
    }

    Some(VerifiedKey {
        key_id: key.id,
        user_id: key.user_id,
        integration_id: key.integration_id,
    })
}

/// Ghost's five minute `maxAge`.
const MAX_AGE_SECS: i64 = 5 * 60;
/// Tolerance for a client whose clock runs slightly fast.
const CLOCK_SKEW_SECS: i64 = 60;

/// Whether `aud` names `api` — the tail of the path, slashes optional.
pub fn audience_matches(aud: &str, api: &str) -> bool {
    aud.trim_end_matches('/').rsplit('/').next().map(|last| last == api).unwrap_or(false)
}

fn claims_valid_at(claims: &Value, api: &str, now: i64) -> bool {
    // jsonwebtoken's maxAge requires iat even when an expiration is supplied.
    let Some(iat) = claims.get("iat").and_then(Value::as_i64) else { return false };
    if now.saturating_sub(iat) >= MAX_AGE_SECS || iat.saturating_sub(now) > CLOCK_SKEW_SECS {
        return false;
    }
    for (key, expired) in [("exp", true), ("nbf", false)] {
        if let Some(value) = claims.get(key) {
            let Some(time) = value.as_i64() else { return false };
            if (expired && now >= time) || (!expired && now < time) { return false; }
        }
    }
    match claims.get("aud") {
        Some(Value::String(aud)) => audience_matches(aud, api),
        Some(Value::Array(audiences)) => audiences.iter().any(|v| v.as_str().is_some_and(|a| audience_matches(a, api))),
        _ => false,
    }
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = key.to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::digest([&ipad[..], msg].concat());
    Sha256::digest([&opad[..], &inner[..]].concat()).to_vec()
}

/// Compared without an early exit, so a wrong signature cannot be narrowed
/// down by timing how long the rejection took.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Unpadded base64url, decode side. JWTs are always unpadded, but padding is
/// tolerated rather than rejected — some clients add it.
mod b64url {
    pub fn decode(input: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len() * 3 / 4);
        let mut buf = 0u32;
        let mut bits = 0u32;
        for c in input.bytes() {
            if c == b'=' {
                break;
            }
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'-' => 62,
                b'_' => 63,
                _ => return None,
            } as u32;
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_ghost_scheme_yields_a_token() {
        assert_eq!(token_from_header("Ghost abc.def.ghi"), Some("abc.def.ghi"));
        // Ghost matches the scheme case-insensitively.
        assert_eq!(token_from_header("ghost abc.def.ghi"), Some("abc.def.ghi"));
        // Anything else must fall through so cookie auth still gets a turn.
        assert_eq!(token_from_header("Bearer abc.def.ghi"), None);
        assert_eq!(token_from_header("Ghost"), None);
        assert_eq!(token_from_header("Ghost "), None);
    }

    #[test]
    fn the_audience_must_name_the_api_being_called() {
        for aud in ["/admin/", "admin", "/v5/admin/", "https://site.test/ghost/api/admin/"] {
            assert!(audience_matches(aud, "admin"), "{aud} names the Admin API");
        }
        // A Content API token replayed against the Admin API is refused —
        // the whole point of the claim.
        assert!(!audience_matches("/content/", "admin"));
        assert!(!audience_matches("", "admin"));
        assert!(audience_matches("/content/", "content"));
    }

    #[test]
    fn base64url_decodes_what_the_members_encoder_produces() {
        for raw in [&b"ab"[..], b"abc", b"a", &[251, 255, 190]] {
            let encoded = encode_for_test(raw);
            assert_eq!(b64url::decode(&encoded).as_deref(), Some(raw), "{encoded}");
        }
        // Padding is tolerated rather than refused.
        assert_eq!(b64url::decode("YWI=").as_deref(), Some(&b"ab"[..]));
        // A character outside the alphabet is not silently skipped.
        assert_eq!(b64url::decode("YW!="), None);
    }

    #[test]
    fn hmac_matches_the_spec() {
        // RFC 4231 test case 2, so the hand-rolled HMAC is pinned to the
        // standard rather than to itself.
        assert_eq!(
            hex::encode(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn constant_time_compare_still_compares() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    fn encode_for_test(input: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
            for (i, shift) in [18, 12, 6, 0].iter().enumerate() {
                if i <= chunk.len() {
                    out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
                }
            }
        }
        out
    }
}
