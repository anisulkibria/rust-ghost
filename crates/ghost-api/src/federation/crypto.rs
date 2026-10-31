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
// ---------------------------------------------------------------------------
// RFC 9421 — HTTP Message Signatures
// ---------------------------------------------------------------------------
//
// The successor to the draft-cavage `Signature` header above. It is what new
// implementations sign with, and it is wire-incompatible with the old one:
// two headers (`Signature-Input` naming the covered components, `Signature`
// carrying the bytes), structured-field syntax, and a signature base built
// from `"name": value` lines plus a trailing `"@signature-params"` line.
//
// Accepted on input only. Outbound requests still use draft-cavage, because
// that is what the deployed network verifies today; sending 9421 to Mastodon
// would simply fail.

/// `sig1=("@method" "@path");created=1;keyid="k"` → the label's components
/// and the raw parameter string that has to be signed verbatim.
fn signature_input(header: &str) -> Option<(Vec<String>, String)> {
    if header.len() > 8192 {
        return None;
    }
    // One label is all that is ever sent here; take the first.
    let (_label, rest) = header.split_once('=')?;
    let rest = rest.trim();
    let close = rest.find(')')?;
    let components: Vec<String> = rest[1..close]
        .split_whitespace()
        .map(|c| c.trim_matches('"').to_string())
        .filter(|c| !c.is_empty())
        .collect();
    if components.is_empty() {
        return None;
    }
    Some((components, rest[..=close].to_string() + &rest[close + 1..]))
}

/// A named parameter out of a structured-field parameter list.
fn sf_param<'a>(params: &'a str, name: &str) -> Option<&'a str> {
    params.split(';').skip(1).find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key.trim() == name).then(|| value.trim().trim_matches('"'))
    })
}

/// The `keyid` from an RFC 9421 `Signature-Input`.
pub fn rfc9421_key_id(headers: &HeaderMap) -> Option<String> {
    let input = headers.get("signature-input")?.to_str().ok()?;
    let (_, params) = signature_input(input)?;
    sf_param(&params, "keyid").map(str::to_string)
}

/// RFC 9421's digest header: `sha-256=:base64:`.
pub fn content_digest(body: &str) -> String {
    format!("sha-256=:{}:", B64.encode(Sha256::digest(body.as_bytes())))
}

/// Verify an RFC 9421 signature over this request.
///
/// Holds the same line as the legacy path: the method, the target and a
/// digest of the body must all be covered, or a valid signature over a
/// different request would be accepted for this one.
pub fn verify_rfc9421(
    headers: &HeaderMap,
    method: &str,
    authority: &str,
    path: &str,
    body: &str,
    key_id: &str,
    public_pem: &str,
) -> bool {
    let Some(input) = headers.get("signature-input").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some((components, params)) = signature_input(input) else {
        return false;
    };
    if sf_param(&params, "keyid") != Some(key_id) {
        return false;
    }
    // `created` is this format's freshness, in place of the Date header.
    let created = sf_param(&params, "created").and_then(|c| c.parse::<i64>().ok());
    let Some(created) = created else { return false };
    let now = chrono::Utc::now().timestamp();
    if (now - created).abs() > 300 {
        return false;
    }

    // The request binding. Without the method and the target, a signature
    // lifted from a GET would authenticate a POST.
    let covers = |name: &str| components.iter().any(|c| c == name);
    if !covers("@method") || !(covers("@target-uri") || covers("@path")) {
        return false;
    }
    let digest_covered = covers("content-digest") || covers("digest");
    if !digest_covered {
        return false;
    }
    if let Some(actual) = headers.get("content-digest").and_then(|v| v.to_str().ok()) {
        // Several digests may be offered; ours has to be among them.
        let expected = content_digest(body);
        if !actual.split(',').any(|d| crate::members::secure_eq(d.trim(), &expected)) {
            return false;
        }
    } else if let Some(actual) = headers.get("digest").and_then(|v| v.to_str().ok()) {
        if !crate::members::secure_eq(actual, &digest(body)) {
            return false;
        }
    } else {
        return false;
    }

    // Build the signature base exactly as the signer did.
    let mut lines = Vec::new();
    for component in &components {
        let value = match component.as_str() {
            "@method" => method.to_ascii_uppercase(),
            "@authority" => authority.to_string(),
            "@path" => path.split('?').next().unwrap_or(path).to_string(),
            "@query" => match path.split_once('?') {
                Some((_, query)) => format!("?{query}"),
                None => "?".to_string(),
            },
            "@target-uri" => format!("https://{authority}{path}"),
            name if name.starts_with('@') => return false, // unsupported derived component
            name => match headers.get(name).and_then(|v| v.to_str().ok()) {
                Some(value) => value.trim().to_string(),
                None => return false,
            },
        };
        lines.push(format!("\"{component}\": {value}"));
    }
    lines.push(format!("\"@signature-params\": {params}"));
    let base = lines.join("\n");

    let Some(bytes) = headers
        .get("signature")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| {
            // `sig1=:base64:`
            let (_, rest) = value.split_once('=')?;
            let inner = rest.trim().trim_matches(':');
            B64.decode(inner).ok()
        })
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
        .verify(base.as_bytes(), &signature)
        .is_ok()
}

/// Whichever signature format this request carries.
///
/// RFC 9421 is recognised by its `Signature-Input` header; anything else is
/// read as draft-cavage. A request carrying neither is unsigned and fails.
pub fn verify_any(
    headers: &HeaderMap,
    method: &str,
    authority: &str,
    path: &str,
    body: &str,
    key_id: &str,
    public_pem: &str,
) -> bool {
    if headers.contains_key("signature-input") {
        return verify_rfc9421(headers, method, authority, path, body, key_id, public_pem);
    }
    verify(headers, path, body, key_id, public_pem)
}

/// The key id from whichever signature format is present.
pub fn any_key_id(headers: &HeaderMap) -> Option<String> {
    rfc9421_key_id(headers).or_else(|| key_id(headers))
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

    /// Sign an RFC 9421 base the way a conforming peer would, so the test
    /// verifies against an independently constructed signature rather than
    /// against this module's own idea of one.
    fn sign_9421(
        private: &str,
        components: &[&str],
        params_tail: &str,
        values: &[(&str, String)],
    ) -> (String, String) {
        let list = components
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(" ");
        let params = format!("({list}){params_tail}");
        let mut lines: Vec<String> = components
            .iter()
            .map(|c| {
                let value = values
                    .iter()
                    .find(|(name, _)| name == c)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                format!("\"{c}\": {value}")
            })
            .collect();
        lines.push(format!("\"@signature-params\": {params}"));
        let signature = sign(private, &lines.join("\n")).unwrap();
        (params, signature)
    }

    fn keypair() -> (String, String) {
        let key = RsaPrivateKey::new(&mut rand_08::rngs::OsRng, 2048).unwrap();
        let public = RsaPublicKey::from(&key).to_public_key_pem(LineEnding::LF).unwrap();
        (key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(), public)
    }

    const KEY: &str = "https://peer.test/actor#main-key";

    fn rfc9421_request(private: &str, body: &str, created: i64) -> HeaderMap {
        let cd = content_digest(body);
        let (params, signature) = sign_9421(
            private,
            &["@method", "@path", "content-digest"],
            &format!(";created={created};keyid=\"{KEY}\""),
            &[
                ("@method", "POST".into()),
                ("@path", "/inbox".into()),
                ("content-digest", cd.clone()),
            ],
        );
        let mut headers = HeaderMap::new();
        headers.insert("host", "us.test".parse().unwrap());
        headers.insert("content-digest", cd.parse().unwrap());
        headers.insert("signature-input", format!("sig1={params}").parse().unwrap());
        headers.insert("signature", format!("sig1=:{signature}:").parse().unwrap());
        headers
    }

    #[test]
    fn an_rfc9421_signature_from_a_conforming_peer_verifies() {
        let (private, public) = keypair();
        let body = r#"{"type":"Follow"}"#;
        let headers = rfc9421_request(&private, body, chrono::Utc::now().timestamp());
        assert!(verify_rfc9421(&headers, "POST", "us.test", "/inbox", body, KEY, &public));
        // and through the dispatcher, which is what the inbox calls
        assert!(verify_any(&headers, "POST", "us.test", "/inbox", body, KEY, &public));
        assert_eq!(rfc9421_key_id(&headers).as_deref(), Some(KEY));
        assert_eq!(any_key_id(&headers).as_deref(), Some(KEY));
    }

    #[test]
    fn an_rfc9421_signature_is_bound_to_its_request() {
        let (private, public) = keypair();
        let body = r#"{"type":"Follow"}"#;
        let headers = rfc9421_request(&private, body, chrono::Utc::now().timestamp());
        // A signature lifted onto another method, path or body must not pass.
        assert!(!verify_rfc9421(&headers, "GET", "us.test", "/inbox", body, KEY, &public));
        assert!(!verify_rfc9421(&headers, "POST", "us.test", "/other", body, KEY, &public));
        assert!(!verify_rfc9421(&headers, "POST", "us.test", "/inbox", "{}", KEY, &public));
        // nor for a different key id than the one it names
        assert!(!verify_rfc9421(&headers, "POST", "us.test", "/inbox", body, "other", &public));
    }

    #[test]
    fn a_stale_rfc9421_signature_is_refused() {
        let (private, public) = keypair();
        let body = "{}";
        let old = chrono::Utc::now().timestamp() - 600;
        let headers = rfc9421_request(&private, body, old);
        assert!(!verify_rfc9421(&headers, "POST", "us.test", "/inbox", body, KEY, &public));
    }

    #[test]
    fn an_rfc9421_signature_that_covers_too_little_is_refused() {
        let (private, public) = keypair();
        let body = "{}";
        // No digest component: a valid signature over the headers alone
        // would authenticate any body at all.
        let (params, signature) = sign_9421(
            &private,
            &["@method", "@path"],
            &format!(";created={};keyid=\"{KEY}\"", chrono::Utc::now().timestamp()),
            &[("@method", "POST".into()), ("@path", "/inbox".into())],
        );
        let mut headers = HeaderMap::new();
        headers.insert("content-digest", content_digest(body).parse().unwrap());
        headers.insert("signature-input", format!("sig1={params}").parse().unwrap());
        headers.insert("signature", format!("sig1=:{signature}:").parse().unwrap());
        assert!(!verify_rfc9421(&headers, "POST", "us.test", "/inbox", body, KEY, &public));
    }

    #[test]
    fn the_two_formats_are_told_apart_by_signature_input() {
        let (private, public) = keypair();
        let body = r#"{"type":"Like"}"#;
        // Legacy, as the deployed network sends it.
        let date = httpdate::fmt_http_date(std::time::SystemTime::now());
        let legacy_digest = digest(body);
        let canonical = format!(
            "(request-target): post /inbox\nhost: us.test\ndate: {date}\ndigest: {legacy_digest}"
        );
        let signature = sign(&private, &canonical).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", "us.test".parse().unwrap());
        headers.insert("date", date.parse().unwrap());
        headers.insert("digest", legacy_digest.parse().unwrap());
        headers.insert("signature", format!("keyId=\"{KEY}\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date digest\",signature=\"{signature}\"").parse().unwrap());
        // No `signature-input`, so the dispatcher takes the legacy path.
        assert!(verify_any(&headers, "POST", "us.test", "/inbox", body, KEY, &public));
        assert_eq!(any_key_id(&headers).as_deref(), Some(KEY));
    }

    #[test]
    fn a_request_with_no_signature_at_all_is_refused() {
        let (_, public) = keypair();
        let headers = HeaderMap::new();
        assert!(!verify_any(&headers, "POST", "us.test", "/inbox", "{}", KEY, &public));
        assert_eq!(any_key_id(&headers), None);
    }

    #[test]
    fn the_rfc9421_digest_header_is_the_structured_field_form() {
        // `sha-256=:base64:`, not the legacy `SHA-256=base64`.
        let d = content_digest("hello");
        assert!(d.starts_with("sha-256=:") && d.ends_with(':'), "{d}");
        assert_ne!(d, digest("hello"));
    }

    #[test]
    fn signature_input_is_parsed_into_components_and_the_exact_parameters() {
        let (components, params) =
            signature_input(r#"sig1=("@method" "@path");created=1;keyid="k""#).unwrap();
        assert_eq!(components, vec!["@method", "@path"]);
        // The parameter string has to come back byte-for-byte: it is signed.
        assert_eq!(params, r#"("@method" "@path");created=1;keyid="k""#);
        assert_eq!(sf_param(&params, "keyid"), Some("k"));
        assert_eq!(sf_param(&params, "created"), Some("1"));
        assert_eq!(sf_param(&params, "absent"), None);
    }

    #[test]
    fn an_outbound_get_is_signed_over_the_target_and_date() {
        let (private, public) = keypair();
        let signer = super::super::network::Signer {
            key_id: KEY.to_string(),
            private_pem: private,
        };
        let url = reqwest::Url::parse("https://peer.test/users/alice").unwrap();
        let date = httpdate::fmt_http_date(std::time::SystemTime::now());
        let header = super::super::network::sign_get_for_test(&signer, &url, &date).unwrap();
        let params = parameters(&header).unwrap();
        assert_eq!(params.get("keyId").copied(), Some(KEY));
        // A GET has no body, so no digest is claimed or signed.
        assert_eq!(params.get("headers").copied(), Some("(request-target) host date"));
        let canonical =
            format!("(request-target): get /users/alice\nhost: peer.test\ndate: {date}");
        let bytes = B64.decode(params.get("signature").unwrap()).unwrap();
        let signature = Signature::try_from(bytes.as_slice()).unwrap();
        let key = RsaPublicKey::from_public_key_pem(&public).unwrap();
        assert!(VerifyingKey::<Sha256>::new(key)
            .verify(canonical.as_bytes(), &signature)
            .is_ok(), "a peer must be able to verify what we send");
    }
}
