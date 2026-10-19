//! Ghost date conventions.
//!
//! DB storage: TEXT `YYYY-MM-DD HH:mm:ss` in UTC (Ghost's knex format).
//! API serialization: ISO8601 with millis + `Z`, e.g. `2024-05-01T12:30:00.000Z`.
//! Templates (`date` helper) additionally format in `@site.timezone`.
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

/// Current time in Ghost's DB string format.
pub fn now_db() -> String {
    to_db_string(&Utc::now())
}

pub fn to_db_string(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Parse a DB string; returns None on malformed input.
pub fn from_db_string(s: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|n| Utc.from_utc_datetime(&n))
}

/// DB string -> ISO8601 millis API shape (Ghost's serialization).
pub fn db_to_api(s: Option<&str>) -> Option<String> {
    s.and_then(from_db_string).map(|dt| api_from_datetime(&dt))
}

pub fn api_from_datetime(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn now_api() -> String {
    api_from_datetime(&Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let now = Utc::now();
        let db = to_db_string(&now);
        let back = from_db_string(&db).unwrap();
        assert_eq!(to_db_string(&back), db);
    }

    #[test]
    fn api_shape() {
        assert_eq!(
            db_to_api(Some("2024-05-01 12:30:00")).as_deref(),
            Some("2024-05-01T12:30:00.000Z")
        );
    }
}
