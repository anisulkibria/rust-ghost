//! Update-collision protection for post saves.
//!
//! Ported from `@tryghost/bookshelf-collision`, which Ghost registers for the
//! `posts` table only. The problem it solves: two editors open the same post,
//! both save, and the second save silently overwrites the first — no error,
//! no trace, and the first author's work is only recoverable from revision
//! history if they think to look.
//!
//! The rule is upstream's, including its two deliberate escape hatches:
//!
//! * A request that carries no `updated_at` is **not** checked. Partial
//!   updates from scripts and integrations rely on that; requiring it would
//!   break every `PUT {"posts":[{"featured":true}]}` in the wild.
//! * A request that changes nothing is not checked either, so re-saving a
//!   stale copy with no edits in it is not an error.
//!
//! Everything else — a stale `updated_at` plus a real change — is a 409.
use serde_json::Value;

/// Fields Ghost excludes from the "did anything change?" test.
///
/// They are all either server-owned (`html`/`plaintext` are rendered from the
/// submitted body, the timestamps and `*_by` columns are set by the save
/// itself) or immutable (`id`). Counting them would make every save collide.
const IGNORED: &[&str] =
    &["created_at", "updated_at", "author_id", "id", "published_by", "updated_by", "html", "plaintext", "uuid"];

/// Compare two timestamps the way upstream's `moment(...).diff(...) !== 0`
/// does, after normalising the two spellings this port stores and serves.
///
/// The API hands out `2026-10-30T09:30:58.000Z` and the column holds
/// `2026-10-30 09:30:58`, so a client that echoes back exactly what it was
/// given must compare equal — otherwise protection would reject every save.
fn same_instant(client: &str, server: &str) -> bool {
    normalise(client) == normalise(server)
}

fn normalise(stamp: &str) -> String {
    let stamp = stamp.trim().replace('T', " ");
    let stamp = stamp.trim_end_matches('Z').trim().to_string();
    // Drop fractional seconds: the API always serialises `.000`, and SQLite
    // rows do not carry them at all.
    match stamp.split_once('.') {
        Some((whole, _)) => whole.to_string(),
        None => stamp,
    }
}

/// Did the client submit a value for `field` that differs from the stored one?
///
/// A field the payload omits is not a change — these are partial updates.
fn differs(submitted: &Value, stored: &Value, field: &str) -> bool {
    match submitted.get(field) {
        None => false,
        Some(incoming) => {
            let current = stored.get(field).unwrap_or(&Value::Null);
            // `null` and a missing stored value mean the same thing here, and
            // a client that echoes `null` for an empty column is not editing.
            if incoming.is_null() && current.is_null() {
                return false;
            }
            incoming != current
        }
    }
}

/// Whether this save would overwrite a newer edit from another session.
///
/// `submitted` is the client's post object, `stored` the serialized post as it
/// is in the database right now.
pub fn collides(submitted: &Value, stored: &Value) -> bool {
    let Some(client_stamp) = submitted.get("updated_at").and_then(Value::as_str) else {
        return false;
    };
    let server_stamp = stored.get("updated_at").and_then(Value::as_str).unwrap_or("");
    if same_instant(client_stamp, server_stamp) {
        return false;
    }
    // The stamps disagree, so this client is working from an older copy. It is
    // only a collision if the request actually carries an edit.
    let Some(fields) = submitted.as_object() else { return false };
    fields
        .keys()
        .any(|field| !IGNORED.contains(&field.as_str()) && differs(submitted, stored, field))
}

/// Ghost's own wording and error code, which the editor matches on to offer
/// the publisher a choice between copies.
pub const MESSAGE: &str = "Saving failed! Someone else is editing this post.";
pub const CODE: &str = "UPDATE_COLLISION";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stored() -> Value {
        json!({
            "id": "p1",
            "title": "Original",
            "updated_at": "2026-10-30 09:30:58",
            "featured": false,
            "custom_excerpt": Value::Null,
        })
    }

    #[test]
    fn a_stale_stamp_with_an_edit_collides() {
        let submitted = json!({ "updated_at": "2026-10-30 09:00:00", "title": "Mine" });
        assert!(collides(&submitted, &stored()));
    }

    #[test]
    fn a_current_stamp_never_collides() {
        let submitted = json!({ "updated_at": "2026-10-30 09:30:58", "title": "Mine" });
        assert!(!collides(&submitted, &stored()));
    }

    #[test]
    fn the_api_timestamp_format_matches_the_stored_one() {
        // The editor echoes back what the API gave it, which is this spelling.
        let submitted = json!({ "updated_at": "2026-10-30T09:30:58.000Z", "title": "Mine" });
        assert!(!collides(&submitted, &stored()), "a client echoing its own read must be able to save");
    }

    #[test]
    fn a_stale_stamp_with_no_edit_is_allowed() {
        // Upstream only rolls back when a field actually changed; re-saving an
        // unchanged stale copy is a no-op, not a conflict.
        let submitted = json!({ "updated_at": "2026-10-30 09:00:00", "title": "Original" });
        assert!(!collides(&submitted, &stored()));
    }

    #[test]
    fn server_owned_fields_do_not_count_as_edits() {
        let submitted = json!({
            "updated_at": "2026-10-30 09:00:00",
            "html": "<p>re-rendered</p>",
            "plaintext": "re-rendered",
            "created_at": "2020-01-01 00:00:00",
            "id": "p1",
        });
        assert!(!collides(&submitted, &stored()), "rendered and immutable columns are excluded upstream");
    }

    #[test]
    fn omitting_updated_at_skips_protection() {
        // Integrations that PATCH one field rely on this; Ghost does the same.
        let submitted = json!({ "featured": true });
        assert!(!collides(&submitted, &stored()));
    }

    #[test]
    fn echoing_null_for_an_empty_column_is_not_an_edit() {
        let submitted = json!({ "updated_at": "2026-10-30 09:00:00", "custom_excerpt": Value::Null });
        assert!(!collides(&submitted, &stored()));
    }

    #[test]
    fn clearing_a_set_field_is_an_edit() {
        let mut stored = stored();
        stored["custom_excerpt"] = json!("kept");
        let submitted = json!({ "updated_at": "2026-10-30 09:00:00", "custom_excerpt": Value::Null });
        assert!(collides(&submitted, &stored));
    }

    #[test]
    fn a_missing_server_stamp_does_not_block_saves() {
        // Legacy rows can have a null updated_at; they must stay editable.
        let submitted = json!({ "updated_at": "2026-10-30 09:00:00", "title": "Mine" });
        assert!(collides(&submitted, &json!({ "title": "Original" })), "an absent stamp still differs");
        assert!(!collides(&json!({ "title": "Mine" }), &json!({ "title": "Original" })));
    }
}
