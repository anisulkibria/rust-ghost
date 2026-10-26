//! Member metafields: definitions, values and the change events they raise.
//!
//! Ghost 6.72's `members-metafields` service. The vocabulary — types, parts,
//! access levels, validation — lives in [`ghost_core::metafields`]; this module
//! is only storage.
use ghost_core::metafields as mf;
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::HashMap;

use super::dberr;

/// A field a publisher has defined.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Metafield {
    pub id: String,
    pub key: String,
    pub name: String,
    pub field_type: String,
    pub status: String,
    pub member_access: String,
    pub sort_order: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl Metafield {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// `custom.company`. What a CSV header, a filter or an error path writes.
    pub fn identity(&self) -> String {
        mf::format_identity(mf::CUSTOM_NAMESPACE, &self.key, None)
    }

    /// The parts this field is made of, or `None` for a scalar.
    pub fn parts(&self) -> Option<&'static [(&'static str, &'static str, &'static str)]> {
        mf::parts_of(&self.field_type)
    }
}

// `type` is a keyword in Rust, so the column is aliased on the way out.
const COLS: &str = "id, key, name, type AS field_type, status, member_access, sort_order, \
     created_at, updated_at";

/// Every definition, in the publisher's order.
///
/// Ties break on `created_at`: `sort_order` defaults to 0, so a site that has
/// never reordered has one value repeated and falls back to the order the
/// fields were created in.
pub async fn list_definitions(pool: &SqlitePool) -> Result<Vec<Metafield>> {
    let rows = sqlx::query_as::<_, Metafield>(&format!(
        "SELECT {COLS} FROM members_metafields ORDER BY sort_order, created_at, id"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows)
}

/// The fields a given audience may see. Staff see everything; a member sees
/// only what their access level allows, and never an archived field.
pub async fn readable_definitions(pool: &SqlitePool, is_member: bool) -> Result<Vec<Metafield>> {
    let all = list_definitions(pool).await?;
    Ok(all
        .into_iter()
        .filter(|f| {
            if !is_member {
                return true;
            }
            f.is_active() && mf::member_can_read(&f.member_access)
        })
        .collect())
}

pub async fn find_definition(pool: &SqlitePool, id: &str) -> Result<Option<Metafield>> {
    let row = sqlx::query_as::<_, Metafield>(&format!(
        "SELECT {COLS} FROM members_metafields WHERE id = ? LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    Ok(row)
}

pub async fn find_definition_by_key(pool: &SqlitePool, key: &str) -> Result<Option<Metafield>> {
    let row = sqlx::query_as::<_, Metafield>(&format!(
        "SELECT {COLS} FROM members_metafields WHERE key = ? LIMIT 1"
    ))
    .bind(key)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    Ok(row)
}

/// Whether `key` or `name` is already taken by a different field. Both are
/// unique in Ghost, and the key is immutable once values reference it.
pub async fn definition_conflict(
    pool: &SqlitePool,
    key: &str,
    name: &str,
    self_id: Option<&str>,
) -> Result<Option<&'static str>> {
    let id = self_id.unwrap_or("");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members_metafields WHERE key = ? AND id != ?")
        .bind(key)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(dberr)?;
    if n > 0 {
        return Ok(Some("key"));
    }
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM members_metafields WHERE name = ? AND id != ?")
            .bind(name)
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(dberr)?;
    if n > 0 {
        return Ok(Some("name"));
    }
    Ok(None)
}

/// Create a definition. New fields append past the highest rank.
pub async fn create_definition(
    pool: &SqlitePool,
    key: &str,
    name: &str,
    field_type: &str,
    member_access: &str,
) -> Result<Metafield> {
    let id = ids::new_id();
    let now = dates::now_db();
    let next: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM members_metafields")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    sqlx::query(
        "INSERT INTO members_metafields (id, key, name, type, status, member_access, sort_order, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'active', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(key)
    .bind(name)
    .bind(field_type)
    .bind(member_access)
    .bind(next)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    find_definition(pool, &id).await?.ok_or_else(|| GhostError::Internal("metafield vanished".into()))
}

/// Update a definition's presentation and access.
///
/// The key and type are not editable: values reference the key and are shaped
/// by the type, so changing either would orphan or misread every value already
/// stored. Ghost takes the same line.
pub async fn update_definition(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    member_access: &str,
    status: &str,
) -> Result<Metafield> {
    sqlx::query(
        "UPDATE members_metafields SET name = ?, member_access = ?, status = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(member_access)
    .bind(status)
    .bind(dates::now_db())
    .bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    find_definition(pool, id).await?.ok_or_else(|| GhostError::Internal("metafield vanished".into()))
}

/// Delete a definition and, by cascade, every value held against it.
pub async fn delete_definition(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM members_metafields WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Move a field one place up or down in the publisher's order.
///
/// Ranks are rewritten densely across every row, which is what makes a swap
/// well-defined on a site whose `sort_order` is still all zeroes.
pub async fn reorder_definition(pool: &SqlitePool, id: &str, up: bool) -> Result<()> {
    let mut all = list_definitions(pool).await?;
    let Some(pos) = all.iter().position(|f| f.id == id) else {
        return Ok(());
    };
    let swap_with = if up {
        if pos == 0 {
            return Ok(());
        }
        pos - 1
    } else {
        if pos + 1 >= all.len() {
            return Ok(());
        }
        pos + 1
    };
    all.swap(pos, swap_with);
    let now = dates::now_db();
    for (rank, field) in all.iter().enumerate() {
        sqlx::query("UPDATE members_metafields SET sort_order = ?, updated_at = ? WHERE id = ?")
            .bind(rank as i64)
            .bind(&now)
            .bind(&field.id)
            .execute(pool)
            .await
            .map_err(dberr)?;
    }
    Ok(())
}

/// How many members hold any value for a field.
pub async fn value_count(pool: &SqlitePool, key: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(DISTINCT member_id) FROM members_metafield_values WHERE metafield_key = ?",
    )
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// One member's values, as `key` → (`path` → value).
///
/// A scalar lands under the empty path; a record lands one entry per part the
/// member has filled in.
pub async fn values_for_member(
    pool: &SqlitePool,
    member_id: &str,
) -> Result<HashMap<String, HashMap<String, String>>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT metafield_key, path, value_text FROM members_metafield_values WHERE member_id = ?",
    )
    .bind(member_id)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    for (key, path, value) in rows {
        out.entry(key).or_default().insert(path, value.unwrap_or_default());
    }
    Ok(out)
}

/// Who wrote a value: a type, and an id resolving in that type's namespace.
#[derive(Debug, Clone)]
pub struct Writer {
    pub writer_type: String,
    pub writer_id: Option<String>,
    /// Where the change was made, as the activity feed names it.
    pub source: String,
}

impl Writer {
    /// A staff member editing from the admin.
    pub fn staff(user_id: &str) -> Self {
        Writer {
            writer_type: "user".into(),
            writer_id: Some(user_id.to_string()),
            source: "admin".into(),
        }
    }

    /// The member themselves, through the portal.
    pub fn member(member_id: &str) -> Self {
        Writer {
            writer_type: "member".into(),
            writer_id: Some(member_id.to_string()),
            source: "portal".into(),
        }
    }
}

/// Write one leaf.
///
/// An empty value deletes the row rather than storing a blank: a part a member
/// has not filled in has no row at all, which is what makes "is set" a
/// question storage can answer.
async fn write_leaf(
    pool: &SqlitePool,
    member_id: &str,
    key: &str,
    path: &str,
    value: &str,
    writer: &Writer,
) -> Result<bool> {
    let existing: Option<Option<String>> = sqlx::query_scalar(
        "SELECT value_text FROM members_metafield_values WHERE member_id = ? AND metafield_key = ? AND path = ?",
    )
    .bind(member_id)
    .bind(key)
    .bind(path)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    let current = existing.clone().map(|v| v.unwrap_or_default());

    if value.is_empty() {
        if current.is_none() {
            return Ok(false);
        }
        sqlx::query(
            "DELETE FROM members_metafield_values WHERE member_id = ? AND metafield_key = ? AND path = ?",
        )
        .bind(member_id)
        .bind(key)
        .bind(path)
        .execute(pool)
        .await
        .map_err(dberr)?;
        return Ok(true);
    }

    if current.as_deref() == Some(value) {
        return Ok(false);
    }

    let now = dates::now_db();
    if current.is_some() {
        sqlx::query(
            "UPDATE members_metafield_values SET value_text = ?, written_by_type = ?, written_by_id = ?, updated_at = ? \
             WHERE member_id = ? AND metafield_key = ? AND path = ?",
        )
        .bind(value)
        .bind(&writer.writer_type)
        .bind(&writer.writer_id)
        .bind(&now)
        .bind(member_id)
        .bind(key)
        .bind(path)
        .execute(pool)
        .await
        .map_err(dberr)?;
    } else {
        sqlx::query(
            "INSERT INTO members_metafield_values (id, metafield_key, member_id, path, value_text, written_by_type, written_by_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(key)
        .bind(member_id)
        .bind(path)
        .bind(value)
        .bind(&writer.writer_type)
        .bind(&writer.writer_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(dberr)?;
    }
    Ok(true)
}

/// What one write says about one field: the leaves it names, by path.
///
/// A path the write does not mention is left as it was — a write names what it
/// means to change.
pub type FieldWrite = HashMap<String, String>;

/// Apply a write across several fields and record the change event.
///
/// Returns the fields that actually changed. Validation happens against each
/// field's declared type, and a key the site has not defined is refused rather
/// than silently dropped.
pub async fn write_values(
    pool: &SqlitePool,
    member_id: &str,
    writes: &HashMap<String, FieldWrite>,
    writer: &Writer,
) -> Result<Vec<Metafield>> {
    let definitions = list_definitions(pool).await?;
    let mut changed: Vec<Metafield> = Vec::new();

    for (key, leaves) in writes {
        let Some(def) = definitions.iter().find(|d| d.key == *key) else {
            return Err(GhostError::BadRequest(format!("No such field: {key}")));
        };
        let mut touched = false;
        match def.parts() {
            Some(parts) => {
                for (path, raw) in leaves {
                    let Some((_, _, part_type)) = parts.iter().find(|(p, _, _)| p == path) else {
                        return Err(GhostError::BadRequest(format!(
                            "No such part of {}: {path}",
                            def.name
                        )));
                    };
                    let value = mf::validate_part(part_type, raw.trim())
                        .map_err(|e| GhostError::BadRequest(format!("{}: {e}", def.name)))?;
                    touched |= write_leaf(pool, member_id, key, path, &value, writer).await?;
                }
            }
            None => {
                // A scalar is addressed by the empty path, whatever the write
                // chose to call it.
                let raw = leaves.get("").or_else(|| leaves.values().next());
                if let Some(raw) = raw {
                    let value = mf::validate_scalar(&def.field_type, raw.trim())
                        .map_err(|e| GhostError::BadRequest(format!("{}: {e}", def.name)))?;
                    touched |= write_leaf(pool, member_id, key, "", &value, writer).await?;
                }
            }
        }
        if touched {
            changed.push(def.clone());
        }
    }

    if !changed.is_empty() {
        record_change(pool, member_id, &changed, writer).await?;
    }
    Ok(changed)
}

/// Add the member's activity-feed entry for a write.
///
/// Each field's name is copied into the entry rather than joined, so the entry
/// still reads after the field is renamed or deleted.
async fn record_change(
    pool: &SqlitePool,
    member_id: &str,
    changed: &[Metafield],
    writer: &Writer,
) -> Result<()> {
    let payload = serde_json::to_string(
        &changed
            .iter()
            .map(|f| json!({ "namespace": mf::CUSTOM_NAMESPACE, "key": f.key, "name": f.name }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into());
    sqlx::query(
        "INSERT INTO members_metafield_change_events (id, member_id, written_by_type, written_by_id, source, metafields, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(member_id)
    .bind(&writer.writer_type)
    .bind(&writer.writer_id)
    .bind(&writer.source)
    .bind(payload)
    .bind(dates::now_db())
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn test_pool() -> (SqlitePool, TempDir) {
        let dir = std::env::temp_dir().join(format!("ghost-mf-test-{}", ids::new_id()));
        let pool = crate::pool::connect(&dir.join("test.db")).await.expect("open test db");
        (pool, TempDir(dir))
    }

    async fn a_member(pool: &SqlitePool) -> String {
        let id = ids::new_id();
        let now = dates::now_db();
        sqlx::query(
            "INSERT INTO members (id, uuid, transient_id, email, status, created_at) \
             VALUES (?, ?, ?, ?, 'free', ?)",
        )
        .bind(&id)
        .bind(ids::uuid_v4())
        .bind(ids::uuid_v4())
        .bind(format!("{id}@example.com"))
        .bind(&now)
        .execute(pool)
        .await
        .expect("insert member");
        id
    }

    fn write_of(pairs: &[(&str, &str, &str)]) -> HashMap<String, FieldWrite> {
        let mut out: HashMap<String, FieldWrite> = HashMap::new();
        for (key, path, value) in pairs {
            out.entry(key.to_string())
                .or_default()
                .insert(path.to_string(), value.to_string());
        }
        out
    }

    #[tokio::test]
    async fn a_scalar_round_trips_under_the_empty_path() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "company", "Company", "short_text", "none").await.unwrap();
        let member = a_member(&pool).await;

        let changed = write_values(
            &pool,
            &member,
            &write_of(&[("company", "", "Ghost")]),
            &Writer::staff("u1"),
        )
        .await
        .unwrap();
        assert_eq!(changed.len(), 1);

        let values = values_for_member(&pool, &member).await.unwrap();
        assert_eq!(values["company"][""], "Ghost");
    }

    #[tokio::test]
    async fn an_address_stores_one_row_per_filled_part() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "ship", "Shipping", "address", "write").await.unwrap();
        let member = a_member(&pool).await;

        write_values(
            &pool,
            &member,
            &write_of(&[
                ("ship", "line1", "1 Test Street"),
                ("ship", "city", "Dhaka"),
                ("ship", "country", "bd"),
            ]),
            &Writer::staff("u1"),
        )
        .await
        .unwrap();

        let values = values_for_member(&pool, &member).await.unwrap();
        let ship = &values["ship"];
        assert_eq!(ship["line1"], "1 Test Street");
        assert_eq!(ship["city"], "Dhaka");
        // the country is normalised on the way in
        assert_eq!(ship["country"], "BD");
        // a part that was never filled has no row at all
        assert!(!ship.contains_key("line2"));
        assert_eq!(ship.len(), 3);
    }

    /// Emptying a part removes its row, so "is set" stays a question storage
    /// can answer.
    #[tokio::test]
    async fn clearing_a_value_deletes_its_row() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "company", "Company", "short_text", "none").await.unwrap();
        let member = a_member(&pool).await;
        let w = Writer::staff("u1");

        write_values(&pool, &member, &write_of(&[("company", "", "Ghost")]), &w).await.unwrap();
        write_values(&pool, &member, &write_of(&[("company", "", "")]), &w).await.unwrap();

        let values = values_for_member(&pool, &member).await.unwrap();
        assert!(values.get("company").map(|f| f.is_empty()).unwrap_or(true));
    }

    /// A write that changes nothing raises no activity-feed entry.
    #[tokio::test]
    async fn an_unchanged_write_is_not_an_event() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "company", "Company", "short_text", "none").await.unwrap();
        let member = a_member(&pool).await;
        let w = Writer::staff("u1");

        write_values(&pool, &member, &write_of(&[("company", "", "Ghost")]), &w).await.unwrap();
        let again =
            write_values(&pool, &member, &write_of(&[("company", "", "Ghost")]), &w).await.unwrap();
        assert!(again.is_empty());

        let events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM members_metafield_change_events WHERE member_id = ?")
                .bind(&member)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(events, 1);
    }

    #[tokio::test]
    async fn an_unknown_field_or_part_is_refused() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "ship", "Shipping", "address", "none").await.unwrap();
        let member = a_member(&pool).await;
        let w = Writer::staff("u1");

        assert!(write_values(&pool, &member, &write_of(&[("nope", "", "x")]), &w).await.is_err());
        assert!(write_values(&pool, &member, &write_of(&[("ship", "planet", "x")]), &w)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn an_invalid_value_is_refused_with_ghosts_sentence() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "ship", "Shipping", "address", "none").await.unwrap();
        let member = a_member(&pool).await;

        let err = write_values(
            &pool,
            &member,
            &write_of(&[("ship", "country", "Bangladesh")]),
            &Writer::staff("u1"),
        )
        .await
        .unwrap_err();
        assert!(err.message().contains("2-letter country code"), "got: {}", err.message());
    }

    #[tokio::test]
    async fn members_see_only_fields_their_access_allows() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "internal", "Internal", "short_text", "none").await.unwrap();
        create_definition(&pool, "shown", "Shown", "short_text", "read").await.unwrap();
        create_definition(&pool, "editable", "Editable", "short_text", "write").await.unwrap();
        let archived =
            create_definition(&pool, "old", "Old", "short_text", "write").await.unwrap();
        update_definition(&pool, &archived.id, "Old", "write", "archived").await.unwrap();

        let staff: Vec<String> =
            readable_definitions(&pool, false).await.unwrap().into_iter().map(|f| f.key).collect();
        assert_eq!(staff, ["internal", "shown", "editable", "old"]);

        let member: Vec<String> =
            readable_definitions(&pool, true).await.unwrap().into_iter().map(|f| f.key).collect();
        // `none` is hidden and an archived field is never shown, even at write
        assert_eq!(member, ["shown", "editable"]);
    }

    #[tokio::test]
    async fn reordering_swaps_neighbours_and_densifies_ranks() {
        let (pool, _d) = test_pool().await;
        let a = create_definition(&pool, "a", "A", "short_text", "none").await.unwrap();
        create_definition(&pool, "b", "B", "short_text", "none").await.unwrap();
        create_definition(&pool, "c", "C", "short_text", "none").await.unwrap();

        reorder_definition(&pool, &a.id, false).await.unwrap();
        let keys: Vec<String> =
            list_definitions(&pool).await.unwrap().into_iter().map(|f| f.key).collect();
        assert_eq!(keys, ["b", "a", "c"]);

        // moving the first up is a no-op rather than an error
        let first = list_definitions(&pool).await.unwrap()[0].id.clone();
        reorder_definition(&pool, &first, true).await.unwrap();
        let keys: Vec<String> =
            list_definitions(&pool).await.unwrap().into_iter().map(|f| f.key).collect();
        assert_eq!(keys, ["b", "a", "c"]);
    }

    #[tokio::test]
    async fn deleting_a_definition_takes_its_values_with_it() {
        let (pool, _d) = test_pool().await;
        let def = create_definition(&pool, "company", "Company", "short_text", "none").await.unwrap();
        let member = a_member(&pool).await;
        write_values(&pool, &member, &write_of(&[("company", "", "Ghost")]), &Writer::staff("u1"))
            .await
            .unwrap();
        assert_eq!(value_count(&pool, "company").await, 1);

        delete_definition(&pool, &def.id).await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members_metafield_values")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn keys_and_names_are_unique() {
        let (pool, _d) = test_pool().await;
        create_definition(&pool, "company", "Company", "short_text", "none").await.unwrap();
        assert_eq!(definition_conflict(&pool, "company", "Other", None).await.unwrap(), Some("key"));
        assert_eq!(definition_conflict(&pool, "other", "Company", None).await.unwrap(), Some("name"));
        assert_eq!(definition_conflict(&pool, "other", "Other", None).await.unwrap(), None);
    }
}
