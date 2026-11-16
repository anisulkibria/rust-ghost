//! The reporting windows a module's screen may offer, and the reduction of a
//! `?days=` parameter to one of them.
//!
//! Three lines of code with one job, in one place for two reasons.
//!
//! The first is rule 8: a query parameter that comes back as a link has to be
//! reduced to a value the page offered, and the reduction and the list of
//! offered values must be the *same list* — otherwise a tab leads to a page
//! that silently reports a different window than the one it names. Keeping
//! both here makes that true by construction.
//!
//! The second is that more than one module reports over a window now
//! (Memberships, Newsletters), and two copies of "7, 30 or 90, default 30"
//! would be two chances for a forged value to be believed in one of them.
//! When a third module wants a different ladder it gets its own named
//! constant beside `OFFERED` rather than its own reducer.

/// The windows offered, as `(query value, days, label)`.
pub const OFFERED: &[(&str, i64, &str)] =
    &[("7", 7, "7 days"), ("30", 30, "30 days"), ("90", 90, "90 days")];

/// How many days a window the `?days=` parameter asks for.
///
/// Anything unrecognised — a forged value, a stale bookmark, a typo —
/// collapses to 30 rather than being believed. Nothing is parsed as a number:
/// the raw string is matched against [`OFFERED`], so there is no arithmetic to
/// overflow and no way to ask for a window the screen cannot draw.
pub fn days(raw: Option<&str>) -> i64 {
    raw.and_then(|v| OFFERED.iter().find(|(key, _, _)| *key == v.trim()))
        .map(|(_, days, _)| *days)
        .unwrap_or(30)
}

/// The query value for a window, for building a segmented bar's links.
pub fn key(days: i64) -> &'static str {
    OFFERED.iter().find(|(_, d, _)| *d == days).map(|(k, _, _)| *k).unwrap_or("30")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offered_window_is_taken_at_its_word() {
        assert_eq!(days(Some("7")), 7);
        assert_eq!(days(Some("30")), 30);
        assert_eq!(days(Some("90")), 90);
        // Trimmed, because a link built by hand often carries a space.
        assert_eq!(days(Some(" 90 ")), 90);
    }

    #[test]
    fn anything_else_collapses_to_the_default_rather_than_being_believed() {
        for forged in ["3650", "-1", "0", "31", "'; DROP TABLE members; --", "", "٧", "7.0"] {
            assert_eq!(days(Some(forged)), 30, "{forged} was believed");
        }
        assert_eq!(days(None), 30);
    }

    #[test]
    fn every_offered_window_round_trips_through_its_key() {
        // If it did not, the segmented bar would mark the wrong tab current.
        for (k, d, _) in OFFERED {
            assert_eq!(key(*d), *k);
            assert_eq!(days(Some(k)), *d);
        }
    }

    #[test]
    fn a_window_nobody_offers_has_no_key_and_falls_back() {
        assert_eq!(key(365), "30");
    }
}
