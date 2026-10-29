//! Transactional Ghost content transfer. Identifiers and relation ordering survive
//! round trips; only an explicit list of content tables can be imported.
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json, Map, Value};
use sqlx::{Column, Row, SqlitePool, TypeInfo, ValueRef};
use std::collections::HashMap;

const TABLES: &[&str] = &[
    "roles",
    "users",
    "roles_users",
    "settings",
    "tags",
    "products",
    "benefits",
    "products_benefits",
    "newsletters",
    "posts",
    "posts_meta",
    "posts_authors",
    "posts_tags",
    "posts_products",
    "snippets",
    "custom_theme_settings",
    "stripe_products",
    "stripe_prices",
    "offers",
];
fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}

pub async fn export(pool: &SqlitePool) -> Result<Value> {
    let mut tx = pool.begin().await.map_err(db)?;
    let mut data = Map::new();
    for table in TABLES {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        )
        .bind(table)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        if !exists {
            continue;
        }
        let rows = sqlx::query(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        let mut values = Vec::new();
        for row in rows {
            let mut obj = Map::new();
            for col in row.columns() {
                let raw = row.try_get_raw(col.ordinal()).map_err(db)?;
                let value = if raw.is_null() {
                    Value::Null
                } else {
                    match raw.type_info().name() {
                        "INTEGER" | "BOOLEAN" => {
                            json!(row.try_get::<i64, _>(col.ordinal()).map_err(db)?)
                        }
                        "REAL" => json!(row.try_get::<f64, _>(col.ordinal()).map_err(db)?),
                        _ => json!(row.try_get::<String, _>(col.ordinal()).map_err(db)?),
                    }
                };
                obj.insert(col.name().to_owned(), value);
            }
            values.push(Value::Object(obj));
        }
        data.insert((*table).into(), json!(values));
    }
    tx.commit().await.map_err(db)?;
    Ok(
        json!({"db":[{"meta":{"version":ghost_core::GHOST_VERSION,"exported_on":chrono::Utc::now().timestamp_millis()},"data":data}]}),
    )
}

/// Merge supported content, mapping natural-key conflicts to existing ids.
/// Imported profiles cannot replace the current owner's password or role.
pub async fn import(pool: &SqlitePool, input: &Value, owner_id: &str) -> Result<()> {
    let mut data = input
        .as_object()
        .cloned()
        .ok_or_else(|| GhostError::BadRequest("Expected content data object".into()))?;
    // Older API-style exports can contain nested relations and post metadata.
    if let Some(posts) = data.get("posts").and_then(Value::as_array).cloned() {
        for p in posts {
            let Some(id) = p["id"].as_str() else { continue };
            for (key, table, col) in [
                ("tags", "posts_tags", "tag_id"),
                ("authors", "posts_authors", "author_id"),
                ("tiers", "posts_products", "product_id"),
            ] {
                if !input.get(table).is_some() {
                    for (sort, item) in p[key].as_array().into_iter().flatten().enumerate() {
                        if let Some(target) = item["id"].as_str().or_else(|| item.as_str()) {
                            data.entry(table).or_insert(json!([])).as_array_mut().unwrap().push(json!({"id":ids::new_id(),"post_id":id,col:target,"sort_order":sort}));
                        }
                    }
                }
            }
            if input.get("posts_meta").is_none() {
                let mut meta = p["meta"].as_object().cloned().unwrap_or_default();
                for key in [
                    "meta_title",
                    "meta_description",
                    "og_title",
                    "og_description",
                    "og_image",
                    "twitter_title",
                    "twitter_description",
                    "twitter_image",
                    "feature_image_alt",
                    "feature_image_caption",
                    "email_subject",
                    "email_preview",
                ] {
                    if let Some(v) = p.get(key) {
                        meta.insert(key.into(), v.clone());
                    }
                }
                if !meta.is_empty() {
                    meta.insert("id".into(), json!(ids::new_id()));
                    meta.insert("post_id".into(), json!(id));
                    data.entry("posts_meta")
                        .or_insert(json!([]))
                        .as_array_mut()
                        .unwrap()
                        .push(json!(meta));
                }
            }
        }
    }
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("PRAGMA defer_foreign_keys=ON")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let mut remap: HashMap<String, String> = HashMap::new();
    for table in TABLES {
        let Some(rows) = data.get(*table).and_then(Value::as_array) else {
            continue;
        };
        let schema = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        if schema.is_empty() {
            continue;
        }
        let columns: Vec<String> = schema.iter().map(|r| r.get("name")).collect();
        if [
            "posts_meta",
            "posts_authors",
            "posts_tags",
            "posts_products",
        ]
        .contains(table)
        {
            if let Some(posts) = data.get("posts").and_then(Value::as_array) {
                for post in posts {
                    if let Some(id) = post["id"].as_str().and_then(|id| remap.get(id)) {
                        sqlx::query(&format!("DELETE FROM {table} WHERE post_id=?"))
                            .bind(id)
                            .execute(&mut *tx)
                            .await
                            .map_err(db)?;
                    }
                }
            }
        }
        for row in rows {
            let Some(mut obj) = row.as_object().cloned() else {
                return Err(GhostError::BadRequest(format!("Invalid {table} row")));
            };
            let original = obj
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(ids::new_id);
            obj.insert("id".into(), json!(&original));
            let natural = match *table {
                "roles" | "snippets" => Some("name"),
                "users" => Some("email"),
                "settings" => Some("key"),
                "stripe_products" => Some("stripe_product_id"),
                "stripe_prices" => Some("stripe_price_id"),
                "offers" => Some("code"),
                "tags" | "products" | "newsletters" | "benefits" => Some("slug"),
                _ => None,
            };
            let existing = if *table == "custom_theme_settings" {
                sqlx::query_scalar::<_, String>(
                    "SELECT id FROM custom_theme_settings WHERE theme=? AND key=?",
                )
                .bind(obj.get("theme").and_then(Value::as_str))
                .bind(obj.get("key").and_then(Value::as_str))
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
            } else if *table == "posts" {
                sqlx::query_scalar::<_, String>(
                    "SELECT id FROM posts WHERE slug=? AND type=? LIMIT 1",
                )
                .bind(obj.get("slug").and_then(Value::as_str))
                .bind(obj.get("type").and_then(Value::as_str).unwrap_or("post"))
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
            } else if let Some(key) = natural {
                if let Some(value) = obj.get(key).and_then(Value::as_str) {
                    sqlx::query_scalar::<_, String>(&format!(
                        "SELECT id FROM \"{table}\" WHERE \"{key}\"=? COLLATE NOCASE LIMIT 1"
                    ))
                    .bind(value)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(db)?
                } else {
                    None
                }
            } else {
                None
            };
            let target = existing.unwrap_or_else(|| original.clone());
            remap.insert(original.clone(), target.clone());
            obj.insert("id".into(), json!(&target));
            for key in [
                "post_id",
                "author_id",
                "tag_id",
                "product_id",
                "benefit_id",
                "newsletter_id",
                "user_id",
                "role_id",
                "published_by",
                "created_by",
                "updated_by",
            ] {
                if let Some(mapped) = obj
                    .get(key)
                    .and_then(Value::as_str)
                    .and_then(|id| remap.get(id))
                    .cloned()
                {
                    obj.insert(key.into(), json!(mapped));
                }
            }
            // Existing staff credentials and roles are never replaced by a content import.
            if *table == "users" {
                if target == owner_id
                    || sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?)",
                    )
                    .bind(&target)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db)?
                {
                    let (password, status): (String, String) =
                        sqlx::query_as("SELECT password,status FROM users WHERE id=?")
                            .bind(&target)
                            .fetch_one(&mut *tx)
                            .await
                            .map_err(db)?;
                    obj.insert("password".into(), json!(password));
                    obj.insert("status".into(), json!(status));
                } else {
                    obj.entry("password").or_insert(json!(bcrypt::hash(
                        ids::random_hex(32),
                        bcrypt::DEFAULT_COST
                    )
                    .map_err(|e| GhostError::Internal(e.to_string()))?));
                    obj.entry("name").or_insert(json!("Imported author"));
                    obj.entry("slug")
                        .or_insert(json!(format!("author-{target}")));
                    obj.entry("email")
                        .or_insert(json!(format!("{target}@invalid.local")));
                    obj.entry("status").or_insert(json!("active"));
                }
            }
            if *table == "roles_users" {
                if obj.get("user_id").and_then(Value::as_str) == Some(owner_id) {
                    continue;
                }
                let role = obj.get("role_id").and_then(Value::as_str).unwrap_or("");
                let is_owner: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM roles WHERE id=? AND name='Owner')",
                )
                .bind(role)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
                if is_owner {
                    let author: String =
                        sqlx::query_scalar("SELECT id FROM roles WHERE name='Author'")
                            .fetch_one(&mut *tx)
                            .await
                            .map_err(db)?;
                    obj.insert("role_id".into(), json!(author));
                }
            }
            if columns.contains(&"created_at".into()) {
                obj.entry("created_at").or_insert(json!(dates::now_db()));
            }
            if *table == "snippets" {
                obj.entry("mobiledoc").or_insert(json!("{}"));
            }
            if *table == "posts" {
                obj.entry("uuid").or_insert(json!(ids::uuid_v4()));
                obj.entry("slug").or_insert(json!(format!("post-{target}")));
            }
            for column in &schema {
                let kind: String = column.get("type");
                if kind.to_ascii_uppercase().contains("DATE") {
                    let key: String = column.get("name");
                    if let Some(value) = obj
                        .get(&key)
                        .and_then(Value::as_str)
                        .and_then(dates::from_api_string)
                    {
                        obj.insert(key, json!(dates::to_db_string(&value)));
                    }
                }
            }
            obj.retain(|k, _| columns.contains(k));
            let keys: Vec<_> = obj.keys().cloned().collect();
            let updates = keys
                .iter()
                .filter(|k| k.as_str() != "id")
                .map(|k| format!("\"{k}\"=excluded.\"{k}\""))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "INSERT INTO \"{table}\" ({}) VALUES ({}) ON CONFLICT(id) DO {}",
                keys.iter()
                    .map(|k| format!("\"{k}\""))
                    .collect::<Vec<_>>()
                    .join(","),
                vec!["?"; keys.len()].join(","),
                if updates.is_empty() {
                    "NOTHING".into()
                } else {
                    format!("UPDATE SET {updates}")
                }
            );
            let mut query = sqlx::query(&sql);
            for key in keys {
                query = match &obj[&key] {
                    Value::Null => query.bind(None::<String>),
                    Value::Bool(v) => query.bind(*v),
                    Value::Number(v) if v.is_i64() => query.bind(v.as_i64().unwrap()),
                    Value::Number(v) => query.bind(v.as_f64().unwrap_or_default()),
                    Value::String(v) => query.bind(v.clone()),
                    v => query.bind(v.to_string()),
                };
            }
            query.execute(&mut *tx).await.map_err(db)?;
        }
    }
    sqlx::query("UPDATE posts SET status='draft' WHERE visibility='tiers' AND status IN ('published','scheduled') AND NOT EXISTS(SELECT 1 FROM posts_products pp WHERE pp.post_id=posts.id)").execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (SqlitePool, String) {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&db).await.unwrap();
        crate::seed::seed_if_empty(&db).await.unwrap();
        let owner = crate::repos::users::setup_owner(
            &db,
            "Owner",
            "owner@example.test",
            "original-password",
        )
        .await
        .unwrap();
        (db, owner.id)
    }
    #[tokio::test]
    async fn ghost_content_round_trip_preserves_source_metadata_and_relations() {
        let (source, owner) = fixture().await;
        let lexical =
            r#"{"root":{"children":[{"type":"paragraph","children":[{"text":"Keep me"}]}]}}"#;
        let data = json!({
            "tags":[{"id":"tag-import","name":"Imported","slug":"imported","visibility":"public"}],
            "products":[{"id":"tier-import","name":"Gold","slug":"gold","type":"paid"}],
            "posts":[{"id":"post-import","slug":"round-trip","title":"Keep me","lexical":lexical,"html":"<p>Keep me</p>","type":"post","status":"published","visibility":"tiers","published_at":"2026-10-01 12:34:56","custom_excerpt":"Excerpt"}],
            "posts_meta":[{"id":"meta-import","post_id":"post-import","meta_title":"SEO","feature_image_alt":"Alt"}],
            "posts_tags":[{"id":"pt-import","post_id":"post-import","tag_id":"tag-import","sort_order":2}],
            "posts_authors":[{"id":"pa-import","post_id":"post-import","author_id":owner,"sort_order":0}],
            "posts_products":[{"id":"pp-import","post_id":"post-import","product_id":"tier-import"}],
            "settings":[{"id":"setting-import","group":"site","key":"round_trip_setting","value":"preserved","type":"string"}],
            "snippets":[{"id":"snippet-import","name":"Snippet","lexical":lexical}]
        });
        import(&source, &data, &owner).await.unwrap();
        let exported = export(&source).await.unwrap();
        let (target, new_owner) = fixture().await;
        import(&target, &exported["db"][0]["data"], &new_owner)
            .await
            .unwrap();
        // A second import replaces the relation set instead of duplicating it.
        import(&target, &exported["db"][0]["data"], &new_owner)
            .await
            .unwrap();
        let row: (String, String, String) =
            sqlx::query_as("SELECT lexical,published_at,status FROM posts WHERE id='post-import'")
                .fetch_one(&target)
                .await
                .unwrap();
        assert_eq!(
            row,
            (
                lexical.into(),
                "2026-10-01 12:34:56".into(),
                "published".into()
            )
        );
        let meta: (String, String) = sqlx::query_as(
            "SELECT meta_title,feature_image_alt FROM posts_meta WHERE post_id='post-import'",
        )
        .fetch_one(&target)
        .await
        .unwrap();
        assert_eq!(meta, ("SEO".into(), "Alt".into()));
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM posts_tags WHERE post_id='post-import'")
                .fetch_one(&target)
                .await
                .unwrap();
        assert_eq!(count, 1);
        let author: String =
            sqlx::query_scalar("SELECT author_id FROM posts_authors WHERE post_id='post-import'")
                .fetch_one(&target)
                .await
                .unwrap();
        assert_eq!(author, new_owner);
        let secret: String = sqlx::query_scalar("SELECT password FROM users WHERE id=?")
            .bind(&new_owner)
            .fetch_one(&target)
            .await
            .unwrap();
        assert_eq!(secret, "original-password");
        let settings: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='round_trip_setting'")
                .fetch_one(&target)
                .await
                .unwrap();
        assert_eq!(settings, "preserved");
    }
    #[tokio::test]
    async fn malformed_relation_rolls_back_the_entire_import() {
        let (db, owner) = fixture().await;
        let data = json!({"tags":[{"id":"rollback-tag","name":"Rollback","slug":"rollback"}],"posts_tags":[{"id":"bad-relation","post_id":"missing","tag_id":"rollback-tag"}]});
        assert!(import(&db, &data, &owner).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE id='rollback-tag'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
