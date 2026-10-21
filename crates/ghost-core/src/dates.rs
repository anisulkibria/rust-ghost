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

/// Parse an API-shaped date back to a timestamp.
///
/// Accepts what a client can realistically send: the ISO form this module
/// emits, the same without milliseconds, an explicit numeric offset, and the
/// DB form — so a filter value echoed back from an earlier response round
/// trips whichever of the two shapes it came from.
pub fn from_api_string(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.fZ", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(Utc.from_utc_datetime(&n));
        }
    }
    None
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

/// `20 Oct 2026` in `timezone` — the byline date in a newsletter.
///
/// Ghost renders it with Luxon's
/// `toLocaleString({year: 'numeric', month: 'short', day: 'numeric'})` in the
/// site timezone. Doing it in the site timezone rather than UTC matters more
/// than it looks: a post published at 23:30 in New York is already the next
/// day in UTC, and the email would date it a day late.
///
/// An unrecognised timezone falls back to UTC rather than failing, since a
/// wrong-by-hours date is better than no email.
pub fn short_date_in_zone(db: &str, timezone: &str) -> String {
    let Some(dt) = from_db_string(db) else {
        return String::new();
    };
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    dt.with_timezone(&tz).format("%-d %b %Y").to_string()
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
