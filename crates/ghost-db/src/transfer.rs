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

/// Tables a full backup can additionally carry, from upstream's
/// `BACKUP_TABLES` (`core/server/data/exporter/table-lists.js`).
///
/// They are not in the default export because they are not *content*: they
/// are the operational state around it — who read what, which emails went
/// where, every member event, every automation run. Ghost excludes them from
/// the portable export and includes them when the caller asks for a complete
/// one, and this is that list, narrowed to the tables this port actually has.
///
/// A table named here that does not exist is skipped rather than failing, so
/// the list can stay in upstream's order and a schema addition needs no
/// change here.
///
/// Credentials ride along deliberately: `api_keys`, `sessions`, `tokens` and
/// `integrations` are the difference between a backup you can restore and a
/// backup that locks every integration out. That is also why this is opt-in
/// and why such an archive must be treated as a secret.
pub const BACKUP_TABLES: &[&str] = &[
    "actions",
    "api_keys",
    "automated_email_recipients",
    "brute",
    "donation_payment_events",
    "email_design_settings",
    "emails",
    "integrations",
    "invites",
    "labels",
    "machine_payment_events",
    "members",
    "members_labels",
    "members_products",
    "members_stripe_customers",
    "members_stripe_customers_subscriptions",
    "permissions",
    "permissions_roles",
    "permissions_users",
    "webhooks",
    "tokens",
    "sessions",
    "subscriptions",
    "mobiledoc_revisions",
    "post_revisions",
    "email_batches",
    "email_recipients",
    "email_recipient_failures",
    "members_cancel_events",
    "members_payment_events",
    "members_login_events",
    "members_email_change_events",
    "members_metafield_change_events",
    "members_status_events",
    "members_paid_subscription_events",
    "members_subscribe_events",
    "members_product_events",
    "members_created_events",
    "members_current_subscription",
    "members_subscription_created_events",
    "members_newsletters",
    "members_metafields",
    "members_metafield_values",
    "members_metafield_bindings",
    "mentions",
    "comments",
    "comment_likes",
    "comment_reports",
    "jobs",
    "redirects",
    "members_click_events",
    "members_feedback",
    "suppressions",
    "email_suppressions",
    "email_spam_complaint_events",
    "milestones",
    "collections",
    "collections_posts",
    "recommendations",
    "recommendation_click_events",
    "recommendation_subscribe_events",
    "outbox",
    "jobs_outbox",
    "gift_deliveries",
    "gifts",
    "gift_links",
    "post_gift_links",
    "post_gift_link_history",
    "app_installations",
    "app_installation_manifests",
    "automations",
    "automation_actions",
    "automation_action_edges",
    "automation_action_revisions",
    "automation_run_steps",
    "automation_runs",
    "automation_trigger_tiers",
    "welcome_email_automation_runs",
    "welcome_email_automated_emails",
    "tinybird_syncs",
    "stripe_checkout_config",
    "stripe_checkout_config_tiers",
    // This port's own operational state, which a complete backup is no more
    // complete without than it is without the tables above.
    "page_views",
    "publishing_workflows",
    "federation_profiles",
    "federation_follows",
    "federation_notes",
    "federation_deliveries",
    "machine_payments",
    "automation_email_designs",
    "staff_password_resets",
    "newsletter_click_events",
    "newsletter_open_events",
];

/// Content moves between installations; host credentials and signing keys do not.
fn portable_setting(row: &Map<String, Value>) -> bool {
    let key = row.get("key").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    !row.get("group").and_then(Value::as_str).unwrap_or("").eq_ignore_ascii_case("core")
        && !matches!(key.as_str(),
            "stripe_connect_publishable_key" | "stripe_connect_account_id" |
            "stripe_publishable_key" | "stripe_billing_portal_configuration_id" |
            "email_verification_required" | "indexnow_api_key" |
            "mail_smtp_pass" | "mail_smtp_user" | "unsplash_access_key" | "password")
        && !["secret", "private_key", "password", "api_key", "access_token", "refresh_token", "signing_key"]
            .iter().any(|part| key.contains(part))
}

fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}

pub async fn export(pool: &SqlitePool) -> Result<Value> {
    export_with(pool, &[]).await
}

/// Export the content tables, plus any extra `include` tables by name.
///
/// `include` is upstream's `options.include`: the caller names tables from
/// [`BACKUP_TABLES`] to lift the export from "portable content" to "complete
/// state". Anything not on that list is ignored rather than honoured — the
/// parameter must not become a way to read an arbitrary table out of the
/// database through the export endpoint.
pub async fn export_with(pool: &SqlitePool, include: &[String]) -> Result<Value> {
    let extra: Vec<&str> = BACKUP_TABLES
        .iter()
        .copied()
        .filter(|table| include.iter().any(|wanted| wanted == table))
        .collect();
    let tables: Vec<&str> = TABLES.iter().copied().chain(extra).collect();

    let mut tx = pool.begin().await.map_err(db)?;
    let mut data = Map::new();
    for table in &tables {
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
            if *table != "settings" || portable_setting(&obj) {
                values.push(Value::Object(obj));
            }
        }
        data.insert((*table).to_string(), json!(values));
    }

    // Ghost's default allowlist includes `offer_redemptions`, a table this
    // port does not have: a member here arrives through at most one offer, so
    // the link is a column on the member row. Projecting it back into
    // upstream's shape is what makes the redemption history survive a round
    // trip — and makes an export from here readable by Ghost, which looks for
    // this table and would otherwise see no redemptions at all.
    let redemptions = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT m.id, m.offer_id, m.created_at FROM members m \
         WHERE m.offer_id IS NOT NULL AND m.offer_id != ''",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap_or_default();
    if !redemptions.is_empty() {
        let rows: Vec<Value> = redemptions
            .into_iter()
            .map(|(member_id, offer_id, created_at)| {
                json!({
                    // Deterministic, so re-exporting the same site twice does
                    // not look like two different redemptions.
                    "id": ids::deterministic_id(&format!("offer_redemption:{offer_id}:{member_id}")),
                    "offer_id": offer_id,
                    "member_id": member_id,
                    "subscription_id": Value::Null,
                    "created_at": created_at,
                })
            })
            .collect();
        data.insert("offer_redemptions".into(), json!(rows));
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
    // Content first, then any extended backup tables the archive carries.
    // Order matters: the content tables are what the extended ones reference,
    // and a members row cannot point at a product that is not in yet.
    let importable: Vec<&str> = TABLES.iter().copied().chain(BACKUP_TABLES.iter().copied()).collect();
    for table in &importable {
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
            if *table == "settings" {
                if !portable_setting(&obj) {
                    continue;
                }
                // Old exports may supply a misleading group for an existing key.
                let group: Option<String> = sqlx::query_scalar("SELECT `group` FROM settings WHERE key=? COLLATE NOCASE")
                    .bind(obj.get("key").and_then(Value::as_str))
                    .fetch_optional(&mut *tx).await.map_err(db)?;
                if group.is_some_and(|group|group.eq_ignore_ascii_case("core")) {
                    continue;
                }
            }
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
            if *table == "members" {
                obj.entry("uuid").or_insert(json!(ids::uuid_v4()));
                // `transient_id` is NOT NULL and UNIQUE: it is what a signed
                // member cookie names, and rolling it is how "sign out
                // everywhere" works. An archive that predates the column, or
                // one hand-assembled, would otherwise fail the whole import —
                // and a fresh value is the right default anyway, since a
                // restored member should not arrive with live sessions.
                obj.entry("transient_id").or_insert(json!(ids::uuid_v4()));
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

    // An archive from Ghost carries redemptions in their own table, which this
    // port keeps as a column on the member. Reading them back is the other
    // half of the projection in `export_with`: without it, importing a Ghost
    // export would drop which offer every paid member signed up through, and
    // the Offers screen would report nothing.
    if let Some(redemptions) = data.get("offer_redemptions").and_then(Value::as_array) {
        for row in redemptions {
            let member = row.get("member_id").and_then(Value::as_str);
            let offer = row.get("offer_id").and_then(Value::as_str);
            let (Some(member), Some(offer)) = (member, offer) else { continue };
            // Ids may have been remapped on the way in.
            let member = remap.get(member).map(String::as_str).unwrap_or(member);
            let offer = remap.get(offer).map(String::as_str).unwrap_or(offer);
            // Only when the offer survived the import and the member has no
            // offer yet: a member here signed up through one offer, and the
            // first is the one the Offers screen counts.
            let _ = sqlx::query(
                "UPDATE members SET offer_id = ? WHERE id = ? AND offer_id IS NULL \
                   AND EXISTS(SELECT 1 FROM offers WHERE id = ?)",
            )
            .bind(offer)
            .bind(member)
            .bind(offer)
            .execute(&mut *tx)
            .await;
        }
    }

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
    async fn portable_exports_and_imports_preserve_host_secrets() {
        let (source, owner) = fixture().await;
        let protected = [
            ("core", "instance_marker"),
            ("email", "mail_smtp_pass"),
            ("email", "mail_smtp_user"),
            ("email", "mailgun_api_key"),
            ("email", "mailgun_webhook_signing_key"),
            ("members", "stripe_connect_account_id"),
            ("members", "stripe_publishable_key"),
            ("members", "email_verification_required"),
            ("analytics", "service_access_token"),
        ];
        for (group, key) in protected {
            sqlx::query("INSERT INTO settings(id,`group`,key,value,type,created_at) VALUES(?,?,?,'host-only','string',datetime('now')) ON CONFLICT(key) DO UPDATE SET value='host-only'")
                .bind(ids::new_id()).bind(group).bind(key).execute(&source).await.unwrap();
        }
        let exported = export(&source).await.unwrap();
        let settings = exported["db"][0]["data"]["settings"].as_array().unwrap();
        assert!(settings.iter().all(|s| s["group"] != "core"));
        for (_, key) in protected {
            assert!(settings.iter().all(|s| s["key"] != key), "exported {key}");
        }
        assert!(settings.iter().any(|s| s["key"] == "title"));
        let mut rows: Vec<Value> = protected.iter().map(|(_, key)| json!({
            "id": ids::new_id(), "group": "site", "key": key, "value": "foreign-host", "type": "string"
        })).collect();
        rows.push(json!({"id":ids::new_id(),"group":"site","key":"title","value":"Imported publication","type":"string"}));
        rows.push(json!({"id":ids::new_id(),"group":"core","key":"new_core_key","value":"foreign-host","type":"string"}));
        rows.push(json!({"id":ids::new_id(),"group":"site","key":"MAIL_SMTP_PASS","value":"foreign-host","type":"string"}));
        rows.push(json!({"id":ids::new_id(),"group":"site","key":"INSTANCE_MARKER","value":"foreign-host","type":"string"}));
        import(&source, &json!({"settings":rows}), &owner).await.unwrap();
        for (_, key) in protected {
            let value: String = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
                .bind(key).fetch_one(&source).await.unwrap();
            assert_eq!(value, "host-only", "overwrote {key}");
        }
        let title: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='title'").fetch_one(&source).await.unwrap();
        assert_eq!(title, "Imported publication");
        let core: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings WHERE key='new_core_key'").fetch_one(&source).await.unwrap();
        assert_eq!(core, 0);
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

    #[tokio::test]
    async fn the_default_export_leaves_operational_state_out() {
        let (db, owner) = fixture().await;
        import(&db, &json!({
            "tags":[{"id":"t1","name":"T","slug":"t","visibility":"public"}],
        }), &owner).await.unwrap();
        sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, created_at) VALUES ('m1','u1','tr1','a@b.test','free',?)")
            .bind(dates::now_db()).execute(&db).await.unwrap();

        let data = &export(&db).await.unwrap()["db"][0]["data"];
        // A content export is the portable one, and a publisher may well hand
        // it to somebody else; their members' addresses are not content.
        assert!(data.get("members").is_none(), "members are not in a content export");
        assert!(data.get("api_keys").is_none(), "nor are credentials");
        assert!(data.get("sessions").is_none());
        assert!(data.get("tags").is_some(), "content still is");
    }

    #[tokio::test]
    async fn an_extended_export_carries_the_backup_tables() {
        let (db, _owner) = fixture().await;
        sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, created_at) VALUES ('m1','u1','tr1','a@b.test','free',?)")
            .bind(dates::now_db()).execute(&db).await.unwrap();

        let include: Vec<String> = BACKUP_TABLES.iter().map(|t| t.to_string()).collect();
        let data = &export_with(&db, &include).await.unwrap()["db"][0]["data"];
        assert_eq!(data["members"][0]["email"], json!("a@b.test"));
        // api_keys exist from seeding, and a restore without them locks every
        // integration out.
        assert!(data.get("api_keys").is_some_and(|v| v.is_array()));
        assert!(data.get("tags").is_some(), "content is still included as well");
    }

    #[tokio::test]
    async fn only_named_backup_tables_are_included() {
        let (db, _owner) = fixture().await;
        sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, created_at) VALUES ('m1','u1','tr1','a@b.test','free',?)")
            .bind(dates::now_db()).execute(&db).await.unwrap();

        let data = &export_with(&db, &["members".to_string()]).await.unwrap()["db"][0]["data"];
        assert!(data.get("members").is_some(), "asked for");
        assert!(data.get("sessions").is_none(), "not asked for");
    }

    #[tokio::test]
    async fn an_arbitrary_table_name_cannot_be_exported() {
        let (db, _owner) = fixture().await;
        // `include` reaches the export from an API query string, so a name
        // outside the backup list must be ignored rather than read.
        let data = &export_with(&db, &["sqlite_master".to_string(), "staff_password_resets".to_string()])
            .await
            .unwrap()["db"][0]["data"];
        assert!(data.get("sqlite_master").is_none(), "not a backup table");
        assert!(data.get("staff_password_resets").is_some(), "this one is on the list");
    }

    #[tokio::test]
    async fn offer_redemptions_survive_a_round_trip() {
        let (source, owner) = fixture().await;
        let now = dates::now_db();
        let tier: String = sqlx::query_scalar("SELECT id FROM products LIMIT 1")
            .fetch_one(&source).await.unwrap();
        sqlx::query("INSERT INTO offers (id, name, code, discount_type, discount_amount, interval, duration, product_id, created_at) VALUES ('off1','Half off','half','percent',50,'month','once',?,?)")
            .bind(&tier).bind(&now).execute(&source).await.unwrap();
        sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, offer_id, created_at) VALUES ('m1','u1','tr1','a@b.test','paid','off1',?)")
            .bind(&now).execute(&source).await.unwrap();

        let exported = export(&source).await.unwrap();
        let data = &exported["db"][0]["data"];
        // Ghost keeps these in a table of their own; this port keeps the link
        // on the member. The export speaks Ghost's shape so a Ghost install
        // reading it sees the redemption.
        let redemptions = data["offer_redemptions"].as_array().expect("projected");
        assert_eq!(redemptions.len(), 1);
        assert_eq!(redemptions[0]["offer_id"], json!("off1"));
        assert_eq!(redemptions[0]["member_id"], json!("m1"));
        assert!(ids::is_objectid(redemptions[0]["id"].as_str().unwrap()), "a Ghost-shaped id");

        // And reading one back sets the column again.
        let (target, target_owner) = fixture().await;
        let mut incoming = data.as_object().cloned().unwrap();
        incoming.insert("members".into(), json!([
            {"id":"m1","uuid":"u1","email":"a@b.test","status":"paid","created_at":now}
        ]));
        import(&target, &Value::Object(incoming), &target_owner).await.unwrap();
        let offer: Option<String> = sqlx::query_scalar("SELECT offer_id FROM members WHERE id='m1'")
            .fetch_one(&target).await.unwrap();
        assert_eq!(offer.as_deref(), Some("off1"), "the redemption came back");
        let _ = owner;
    }

    #[tokio::test]
    async fn the_projected_redemption_id_is_stable_across_exports() {
        let (db, _owner) = fixture().await;
        let now = dates::now_db();
        let tier: String = sqlx::query_scalar("SELECT id FROM products LIMIT 1")
            .fetch_one(&db).await.unwrap();
        sqlx::query("INSERT INTO offers (id, name, code, discount_type, discount_amount, interval, duration, product_id, created_at) VALUES ('off1','N','n','percent',10,'month','once',?,?)")
            .bind(&tier).bind(&now).execute(&db).await.unwrap();
        sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, offer_id, created_at) VALUES ('m1','u1','tr1','a@b.test','paid','off1',?)")
            .bind(&now).execute(&db).await.unwrap();
        let first = export(&db).await.unwrap()["db"][0]["data"]["offer_redemptions"][0]["id"].clone();
        let second = export(&db).await.unwrap()["db"][0]["data"]["offer_redemptions"][0]["id"].clone();
        // Random ids here would make two exports of one site look like two
        // different redemptions, and importing both would double-count.
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn an_extended_import_restores_members() {
        let (db, owner) = fixture().await;
        let now = dates::now_db();
        import(&db, &json!({
            "members":[{"id":"m1","uuid":"u1","email":"restored@b.test","status":"free","created_at":now}]
        }), &owner).await.unwrap();
        let email: Option<String> = sqlx::query_scalar("SELECT email FROM members WHERE id='m1'")
            .fetch_optional(&db).await.unwrap().flatten();
        assert_eq!(email.as_deref(), Some("restored@b.test"), "a backup is useless if it cannot be restored");
    }
}
