//! bson-ObjectId-compatible 24-hex identifiers, generated app-side
//! like Ghost does (no auto-increment PKs anywhere).
use rand::RngCore;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU32 = AtomicU32::new(0);
static PROCESS_RAND: OnceLock<[u8; 5]> = OnceLock::new();

fn process_rand() -> [u8; 5] {
    *PROCESS_RAND.get_or_init(|| {
        let mut buf = [0u8; 5];
        rand::rng().fill_bytes(&mut buf);
        buf
    })
}

/// Generate a new 24-char lowercase hex ObjectId string.
pub fn new_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let rand5 = process_rand();
    let inc = COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(1);

    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&secs.to_be_bytes());
    bytes[4..9].copy_from_slice(&rand5);
    bytes[9..12].copy_from_slice(&inc.to_be_bytes()[1..]); // low 3 bytes
    hex::encode(bytes)
}

/// True when the string looks like a Ghost id (24 hex chars).
pub fn is_objectid(s: &str) -> bool {
    s.len() == 24 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Random hex string of `n` bytes (2n chars).
pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// An RFC 4122 version-4 UUID. Ghost gives members and newsletters a `uuid`
/// alongside their ObjectId, and it is the public handle in unsubscribe and
/// feedback links, so it has to be well-formed for other tools to parse.
pub fn uuid_v4() -> String {
    let mut c: Vec<char> = random_hex(16).chars().collect();
    c[12] = '4'; // version
    // variant: top two bits must be 10, so the nibble is one of 8/9/a/b
    c[16] = char::from_digit((c[16].to_digit(16).unwrap_or(8) & 0b11) | 0b1000, 16).unwrap();
    let s: String = c.into_iter().collect();
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_24_hex() {
        let id = new_id();
        assert_eq!(id.len(), 24);
        assert!(is_objectid(&id));
    }

    #[test]
    fn ids_are_distinct() {
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn uuids_are_v4_shaped() {
        // every draw, not just one: the variant nibble is derived from random
        // input, so a single sample hides a quarter of the failure space
        for _ in 0..200 {
            let id = uuid_v4();
            assert_eq!(id.len(), 36);
            assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
            assert_eq!(&id[14..15], "4", "version nibble");
            assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "variant nibble in {id}");
        }
    }
}
