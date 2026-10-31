//! HS256 JWT signing, for tokens a third party verifies.
//!
//! The admin's feedback widget is the caller that makes this necessary: the
//! token goes to an external service, which checks it against a standard JWT
//! library. The port's other signed blobs are only ever verified by this
//! server, so they could get away with a hex signature; a real JWT cannot —
//! the signature is base64url with no padding, like the two segments before
//! it, and a hex one is rejected outright.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// base64url without padding, per RFC 7515 §2.
fn b64url(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, v) in idx.iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[*v as usize] as char);
            }
        }
    }
    out
}

/// HMAC-SHA256 over `message`, as raw bytes.
fn hmac_sha256(secret: &str, message: &str) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK {
        key = Sha256::digest(&key).to_vec();
    }
    key.resize(BLOCK, 0);
    let ipad: Vec<u8> = key.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = key.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::digest([&ipad[..], message.as_bytes()].concat());
    Sha256::digest([&opad[..], &inner[..]].concat()).to_vec()
}

/// Sign `claims` as a compact HS256 JWT.
pub fn sign_hs256(claims: &Value, secret: &str) -> String {
    let header = json!({ "alg": "HS256", "typ": "JWT" });
    let signing_input =
        format!("{}.{}", b64url(header.to_string().as_bytes()), b64url(claims.to_string().as_bytes()));
    let signature = b64url(&hmac_sha256(secret, &signing_input));
    format!("{signing_input}.{signature}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_rfc_7515_and_omits_padding() {
        assert_eq!(b64url(b""), "");
        assert_eq!(b64url(b"a"), "YQ");
        assert_eq!(b64url(b"ab"), "YWI");
        assert_eq!(b64url(b"abc"), "YWJj");
        // the two characters that distinguish base64url from base64
        assert_eq!(b64url(&[0xfb, 0xef, 0xbe]), "----");
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vector() {
        // Case 2, so the hand-rolled HMAC is pinned to the spec.
        assert_eq!(
            hex::encode(hmac_sha256("Jefe", "what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn a_signed_token_has_three_base64url_segments() {
        let token = sign_hs256(&json!({ "sub": "a@b.test" }), "secret");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        for part in &parts {
            assert!(!part.contains('='), "JWT segments are unpadded: {part}");
            assert!(!part.contains('+') && !part.contains('/'), "base64url alphabet only: {part}");
        }
    }

    #[test]
    fn the_header_declares_hs256() {
        let token = sign_hs256(&json!({}), "secret");
        let header = token.split('.').next().unwrap();
        // {"alg":"HS256","typ":"JWT"} — a verifier reads this before anything
        // else, and rejects the token if it does not name the algorithm it was
        // told to expect.
        assert_eq!(header, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
    }

    #[test]
    fn the_signature_covers_both_segments() {
        let a = sign_hs256(&json!({ "sub": "one" }), "secret");
        let b = sign_hs256(&json!({ "sub": "two" }), "secret");
        assert_ne!(a.split('.').nth(2), b.split('.').nth(2), "different claims, different signature");
        let c = sign_hs256(&json!({ "sub": "one" }), "other");
        assert_ne!(a.split('.').nth(2), c.split('.').nth(2), "different secret, different signature");
    }

    #[test]
    fn a_known_token_verifies_against_an_independent_computation() {
        // Recomputing the signature the way a verifying library would.
        let claims = json!({ "sub": "1234567890", "name": "John Doe", "iat": 1_516_239_022 });
        let token = sign_hs256(&claims, "a-string-secret-at-least-256-bits-long");
        let (signing_input, signature) = token.rsplit_once('.').unwrap();
        let expected = b64url(&hmac_sha256("a-string-secret-at-least-256-bits-long", signing_input));
        assert_eq!(signature, expected);
    }
}
