use super::*;
use base64::Engine;
use rsa::{
    pkcs1v15::{Signature, SigningKey, VerifyingKey},
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding},
    signature::{SignatureEncoding, Signer, Verifier},
    RsaPrivateKey, RsaPublicKey,
};
use sha2::{Digest, Sha256};
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;
static KEY_CREATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub async fn keys(state: &AppState) -> ghost_core::Result<(String, String)> {
    let _lock = KEY_CREATION.lock().await;
    if let Some(row) = sqlx::query_as::<_, (String, String)>(
        "SELECT private_pem,public_pem FROM federation_keys WHERE id='publication'",
    )
    .fetch_optional(&state.db)
    .await
    .map_err(db_error)?
    {
        return Ok(row);
    }
    let (private, public) = tokio::task::spawn_blocking(|| {
        let private =
            RsaPrivateKey::new(&mut rand_08::rngs::OsRng, 2048).map_err(|e| e.to_string())?;
        let public = RsaPublicKey::from(&private)
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| e.to_string())?;
        let private = private
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| e.to_string())?
            .to_string();
        Ok::<_, String>((private, public))
    })
    .await
    .map_err(|e| ghost_core::GhostError::Internal(e.to_string()))?
    .map_err(ghost_core::GhostError::Internal)?;
    sqlx::query("INSERT INTO federation_keys (id,private_pem,public_pem,created_at) VALUES ('publication',?,?,?)").bind(&private).bind(&public).bind(ghost_core::dates::now_db()).execute(&state.db).await.map_err(db_error)?;
    Ok((private, public))
}
pub fn digest(body: &str) -> String {
    format!("SHA-256={}", B64.encode(Sha256::digest(body.as_bytes())))
}
pub fn sign(private: &str, message: &str) -> ghost_core::Result<String> {
    let key = RsaPrivateKey::from_pkcs8_pem(private)
        .map_err(|e| ghost_core::GhostError::Internal(e.to_string()))?;
    Ok(B64.encode(
        SigningKey::<Sha256>::new(key)
            .sign(message.as_bytes())
            .to_bytes(),
    ))
}
fn parameters(header: &str) -> Option<std::collections::HashMap<&str, &str>> {
    if header.len() > 8192 {
        return None;
    }
    header
        .split(',')
        .map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            Some((key, value.strip_prefix('"')?.strip_suffix('"')?))
        })
        .collect()
}
pub fn key_id(headers: &HeaderMap) -> Option<String> {
    let signature = headers.get("signature")?.to_str().ok()?;
    parameters(signature)?.get("keyId").map(|s| s.to_string())
}
/// Require freshness, a body digest and all request-binding headers before RSA verification.
pub fn verify(headers: &HeaderMap, path: &str, body: &str, key_id: &str, public_pem: &str) -> bool {
    let signature = headers
        .get("signature")
        .and_then(|v| v.to_str().ok())
        .and_then(parameters);
    let Some(signature) = signature else {
        return false;
    };
    if signature.get("keyId").copied() != Some(key_id)
        || !matches!(
            signature.get("algorithm").copied(),
            Some("rsa-sha256" | "hs2019")
        )
    {
        return false;
    }
    let Some(date) = headers
        .get("date")
        .and_then(|v| v.to_str().ok())
        .and_then(|date| httpdate::parse_http_date(date).ok())
    else {
        return false;
    };
    let age = std::time::SystemTime::now()
        .duration_since(date)
        .or_else(|_| date.duration_since(std::time::SystemTime::now()));
    if !age.is_ok_and(|age| age.as_secs() <= 300) {
        return false;
    }
    if !headers
        .get("digest")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|actual| crate::members::secure_eq(actual, &digest(body)))
    {
        return false;
    }
    let names = signature
        .get("headers")
        .copied()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>();
    if !["(request-target)", "host", "date", "digest"]
        .iter()
        .all(|name| names.contains(name))
    {
        return false;
    }
    let mut lines = Vec::new();
    for name in names {
        if name == "(request-target)" {
            lines.push(format!("(request-target): post {path}"));
        } else {
            let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) else {
                return false;
            };
            lines.push(format!("{name}: {value}"));
        }
    }
    let Some(bytes) = signature
        .get("signature")
        .and_then(|value| B64.decode(value).ok())
    else {
        return false;
    };
    let Ok(signature) = Signature::try_from(bytes.as_slice()) else {
        return false;
    };
    let Ok(key) = RsaPublicKey::from_public_key_pem(public_pem) else {
        return false;
    };
    if key.size() < 256 {
        return false;
    }
    VerifyingKey::<Sha256>::new(key)
        .verify(lines.join("\n").as_bytes(), &signature)
        .is_ok()
}
use rsa::traits::PublicKeyParts;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_bind_the_body_path_and_fresh_date() {
        let key = RsaPrivateKey::new(&mut rand_08::rngs::OsRng, 2048).unwrap();
        let public = RsaPublicKey::from(&key)
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        let private = key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let body = r#"{"type":"Follow"}"#;
        let date = httpdate::fmt_http_date(std::time::SystemTime::now());
        let digest = digest(body);
        let content = format!(
            "(request-target): post /inbox\nhost: example.test\ndate: {date}\ndigest: {digest}"
        );
        let signature = sign(&private, &content).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", "example.test".parse().unwrap());
        headers.insert("date", date.parse().unwrap());
        headers.insert("digest", digest.parse().unwrap());
        headers.insert("signature",format!("keyId=\"https://example.test/actor#main-key\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date digest\",signature=\"{signature}\"").parse().unwrap());
        assert!(verify(
            &headers,
            "/inbox",
            body,
            "https://example.test/actor#main-key",
            &public
        ));
        assert!(!verify(
            &headers,
            "/other",
            body,
            "https://example.test/actor#main-key",
            &public
        ));
        assert!(!verify(
            &headers,
            "/inbox",
            "{}",
            "https://example.test/actor#main-key",
            &public
        ));
        headers.insert("date", "Thu, 01 Jan 1970 00:00:00 GMT".parse().unwrap());
        assert!(!verify(
            &headers,
            "/inbox",
            body,
            "https://example.test/actor#main-key",
            &public
        ));
    }
}
