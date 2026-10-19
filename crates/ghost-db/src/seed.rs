//! First-boot seeding: default settings (with dynamic secrets), roles,
//! permissions, fixture posts, and the placeholder Owner user.
use ghost_core::{dates, ids, Result};
use serde::Deserialize;
use sqlx::SqlitePool;

#[derive(Debug, Deserialize)]
struct SettingSeed {
    group: String,
    key: String,
    #[serde(rename = "type")]
    value_type: String,
    #[serde(default)]
    flags: String,
    #[serde(default)]
    value: serde_json::Value,
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => if *b { "true".into() } else { "false".into() },
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Run seeding on a database that has not been seeded yet.
///
/// The probe is the `roles` table, not `settings`: migrations insert settings
/// rows of their own (`0004_mail_settings.sql` adds the mail defaults), so on
/// a fresh database the settings table is already non-empty by the time this
/// runs. Probing it meant a new install was declared "already seeded" and
/// silently came up with no roles, no permissions, no placeholder owner and no
/// fixture content — which left the setup wizard unable to complete at all.
///
/// `roles` is written only here, so it answers the question actually being
/// asked. The steps below are individually idempotent anyway, but the guard
/// keeps fixture content from reappearing after someone deletes it.
pub async fn seed_if_empty(pool: &SqlitePool) -> Result<bool> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if count > 0 {
        return Ok(false);
    }
    seed_settings(pool).await?;
    seed_roles_and_permissions(pool).await?;
    seed_owner(pool).await?;
    seed_content(pool).await?;
    seed_integrations(pool).await?;
    Ok(true)
}

async fn seed_settings(pool: &SqlitePool) -> Result<()> {
    let raw = include_str!("../assets/default-settings.json");
    let seeds: Vec<SettingSeed> = serde_json::from_str(raw)?;

    let now = dates::now_db();
    for s in seeds {
        let value = value_to_string(&s.value);
        sqlx::query("INSERT OR IGNORE INTO settings (id, `group`, key, value, type, flags, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(&s.group)
            .bind(&s.key)
            .bind(&value)
            .bind(if s.value_type == "bool" { "boolean".to_string() } else { s.value_type.clone() })
            .bind(if s.flags.is_empty() { None } else { Some(s.flags.clone()) })
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }

    // Dynamic secret settings (Ghost generates these at first boot).
    let dynamic: Vec<(&str, &str, String, &str)> = vec![
        ("core", "db_hash", ids::random_hex(16), "string"),
        ("core", "admin_session_secret", ids::random_hex(32), "string"),
        ("core", "theme_session_secret", ids::random_hex(32), "string"),
        ("core", "members_email_auth_secret", ids::random_hex(64), "string"),
        ("core", "members_otc_secret", ids::random_hex(64), "string"),
        ("core", "site_uuid", uuid_v4(), "string"),
        ("private", "public_hash", ids::random_hex(15), "string"),
        ("indexnow", "indexnow_api_key", ids::random_hex(16), "string"),
    ];
    for (group, key, value, typ) in dynamic {
        sqlx::query("INSERT INTO settings (id, `group`, key, value, type, flags, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'RO', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, flags = 'RO', updated_at = excluded.updated_at")
            .bind(&ids::new_id())
            .bind(group)
            .bind(key)
            .bind(&value)
            .bind(typ)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }
    Ok(())
}

fn uuid_v4() -> String {
    let mut b = hex::decode(ids::random_hex(16)).unwrap();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hexs = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &hexs[0..8],
        &hexs[8..12],
        &hexs[12..16],
        &hexs[16..20],
        &hexs[20..32]
    )
}

const ROLES: &[&str] = &[
    "Administrator",
    "Editor",
    "Author",
    "Contributor",
    "Owner",
    "Admin Integration",
    "Self-Serve Migration Integration",
    "DB Backup Integration",
    "Scheduler Integration",
    "Super Editor",
];

async fn seed_roles_and_permissions(pool: &SqlitePool) -> Result<()> {
    let now = dates::now_db();
    for name in ROLES {
        sqlx::query("INSERT INTO roles (id, name, description, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(name)
            .bind(format!("{name} role"))
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }

    // Core permission set + role mapping (simplified matrix: staff roles
    // progressively narrower; integration roles read-mostly).
    let perms: &[(&str, &str)] = &[
        ("post", "browse"), ("post", "read"), ("post", "add"), ("post", "edit"), ("post", "destroy"), ("post", "publish"),
        ("page", "browse"), ("page", "read"), ("page", "add"), ("page", "edit"), ("page", "destroy"), ("page", "publish"),
        ("tag", "browse"), ("tag", "read"), ("tag", "add"), ("tag", "edit"), ("tag", "destroy"),
        ("user", "browse"), ("user", "read"), ("user", "add"), ("user", "edit"), ("user", "destroy"),
        ("setting", "browse"), ("setting", "read"), ("setting", "edit"),
        ("theme", "browse"), ("theme", "read"), ("theme", "activate"), ("theme", "edit"), ("theme", "add"), ("theme", "destroy"),
        ("integration", "browse"), ("integration", "read"), ("integration", "add"), ("integration", "edit"), ("integration", "destroy"),
        ("api_key", "browse"), ("api_key", "read"), ("api_key", "add"), ("api_key", "edit"), ("api_key", "destroy"),
        ("webhook", "browse"), ("webhook", "add"), ("webhook", "edit"), ("webhook", "destroy"),
        ("db", "exportContent"), ("db", "importContent"), ("db", "backupContent"), ("db", "deleteAllContent"),
        ("mail", "send"), ("notification", "browse"), ("notification", "add"), ("notification", "destroy"),
        ("redirect", "upload"), ("redirect", "download"), ("slug", "generate"), ("role", "browse"), ("role", "assign"),
        ("invite", "browse"), ("invite", "add"), ("invite", "edit"), ("invite", "destroy"), ("invite", "read"),
        ("member", "browse"), ("member", "read"), ("member", "add"), ("member", "edit"), ("member", "destroy"),
        ("newsletter", "browse"), ("newsletter", "read"), ("newsletter", "add"), ("newsletter", "edit"),
        ("product", "browse"), ("product", "read"), ("product", "add"), ("product", "edit"), ("product", "destroy"),
        ("comment", "browse"), ("comment", "read"), ("comment", "edit"), ("comment", "destroy"),
        ("image", "upload"), ("media", "upload"), ("file", "upload"),
    ];

    let mut perm_ids = Vec::new();
    for (object_type, action_type) in perms {
        let id = ids::new_id();
        perm_ids.push((id.clone(), *object_type, *action_type));
        sqlx::query("INSERT INTO permissions (id, name, object_type, action_type, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(format!("{action_type}_{object_type}"))
            .bind(object_type)
            .bind(action_type)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }

    // role → permissions
    let roles: Vec<(String, String)> =
        sqlx::query_as("SELECT id, name FROM roles").fetch_all(pool).await.map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    let by_name = |n: &str| roles.iter().find(|(_, name)| name == n).map(|(id, _)| id.clone());

    for (role_name, predicate) in [
        ("Administrator", Box::new(|_: &str, _: &str| true) as Box<dyn Fn(&str, &str) -> bool>),
        ("Owner", Box::new(|_: &str, _: &str| true)),
        ("Super Editor", Box::new(|_: &str, _: &str| true)),
        (
            "Editor",
            Box::new(|o: &str, a: &str| {
                !matches!(o, "setting" | "user" | "role" | "theme" | "db" | "integration" | "api_key" | "invite")
                    && a != "destroy"
            }),
        ),
        (
            "Author",
            Box::new(|o: &str, a: &str| {
                (o == "post" && matches!(a, "browse" | "read" | "add" | "edit"))
                    || (o == "tag" && matches!(a, "browse" | "read" | "add"))
                    || (o == "image" && a == "upload")
            }),
        ),
        (
            "Contributor",
            Box::new(|o: &str, a: &str| {
                (o == "post" && matches!(a, "browse" | "read" | "add"))
                    || (o == "tag" && a == "browse")
            }),
        ),
        (
            "Admin Integration",
            Box::new(|o: &str, a: &str| {
                (o == "post" && matches!(a, "browse" | "read" | "add" | "edit" | "publish"))
                    || (o == "tag" && matches!(a, "browse" | "read" | "add" | "edit"))
                    || (o == "image" && a == "upload")
            }),
        ),
        (
            "Scheduler Integration",
            Box::new(|o: &str, a: &str| o == "post" && a == "publish"),
        ),
        (
            "DB Backup Integration",
            Box::new(|o: &str, a: &str| o == "db" && a == "exportContent"),
        ),
        (
            "Self-Serve Migration Integration",
            Box::new(|o: &str, _: &str| o == "post" || o == "tag" || o == "user"),
        ),
    ] {
        let Some(role_id) = by_name(role_name) else { continue };
        for (pid, o, a) in &perm_ids {
            if predicate(o, a) {
                sqlx::query("INSERT INTO permissions_roles (id, role_id, permission_id) VALUES (?, ?, ?)")
                    .bind(&ids::new_id())
                    .bind(&role_id)
                    .bind(pid)
                    .execute(pool)
                    .await
                    .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
            }
        }
    }
    Ok(())
}

/// Placeholder Owner (inactive) — the setup flow activates this user.
async fn seed_owner(pool: &SqlitePool) -> Result<()> {
    let now = dates::now_db();
    let owner_id = ids::new_id();
    let password = bcrypt::hash(&ids::random_hex(12), 10).map_err(|e| ghost_core::GhostError::Internal(e.to_string()))?;
    sqlx::query("INSERT INTO users (id, name, slug, password, email, status, visibility, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&owner_id)
        .bind("Ghost")
        .bind("ghost")
        .bind(&password)
        .bind("ghost@example.com")
        .bind("inactive")
        .bind("public")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;

    let role_id: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Owner'")
        .fetch_optional(pool)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    if let Some(role_id) = role_id {
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
            .bind(&ids::new_id())
            .bind(&role_id)
            .bind(&owner_id)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }
    Ok(())
}

async fn seed_content(pool: &SqlitePool) -> Result<()> {
    let now = dates::now_db();
    let owner_id: String = sqlx::query_scalar("SELECT user_id FROM roles_users ru JOIN roles r ON r.id = ru.role_id WHERE r.name = 'Owner' LIMIT 1")
        .fetch_one(pool)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;

    // News tag
    let tag_id = ids::new_id();
    sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at, updated_at) VALUES (?, ?, ?, 'public', ?, ?)")
        .bind(&tag_id)
        .bind("News")
        .bind("news")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;

    // Free + default tiers
    for (name, slug, monthly, yearly, typ) in [
        ("Free", "free", None::<i64>, None::<i64>, "free"),
        ("Default Product", "default-product", Some(500), Some(5000), "paid"),
    ] {
        sqlx::query("INSERT INTO products (id, name, slug, active, visibility, type, currency, monthly_price, yearly_price, created_at, updated_at) VALUES (?, ?, ?, 1, 'none', ?, 'usd', ?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(name)
            .bind(slug)
            .bind(typ)
            .bind(monthly)
            .bind(yearly)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }

    // Default newsletter
    sqlx::query("INSERT INTO newsletters (id, uuid, name, slug, sender_reply_to, status, visibility, subscribe_on_signup, created_at, updated_at) VALUES (?, ?, ?, ?, 'newsletter', 'active', 'members', 1, ?, ?)")
        .bind(&ids::new_id())
        .bind(&uuid_v4())
        .bind("Default Newsletter")
        .bind("default-newsletter")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;

    // Fixture posts: "Coming soon" (post) + "About this site" (page)
    let coming_soon_md = "### Welcome!\n\nThis is your brand new Ghost site. You can start editing it right now by heading to the admin area.\n\nHead to [the admin area](/ghost) to start publishing.";
    let about_md = "This is an **about** page, great for telling readers a little more about who you are and what you publish.";

    for (slug, title, post_type, md) in [
        ("coming-soon", "Coming soon", "post", coming_soon_md),
        ("about", "About this site", "page", about_md),
    ] {
        let post_id = ids::new_id();
        let html = ghost_render_placeholder(md);
        let plaintext = html_plain(&html);
        let reading_time = ((plaintext.split_whitespace().count() as f64 / 213.0).ceil() as i64).max(1);
        let published = dates::to_db_string(&chrono::Utc::now());
        sqlx::query("INSERT INTO posts (id, uuid, title, slug, markdown, html, plaintext, type, status, visibility, reading_time, created_at, updated_at, published_at, published_by, featured, show_title_and_feature_image) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'published', 'public', ?, ?, ?, ?, ?, 0, 1)")
            .bind(&post_id)
            .bind(&uuid_v4())
            .bind(title)
            .bind(slug)
            .bind(md)
            .bind(&html)
            .bind(&plaintext)
            .bind(post_type)
            .bind(reading_time)
            .bind(&now)
            .bind(&now)
            .bind(&published)
            .bind(&owner_id)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
        sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, 0)")
            .bind(&ids::new_id())
            .bind(&post_id)
            .bind(&owner_id)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
        if slug == "coming-soon" {
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, 0)")
                .bind(&ids::new_id())
                .bind(&post_id)
                .bind(&tag_id)
                .execute(pool)
                .await
                .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
        }
    }
    Ok(())
}

async fn seed_integrations(pool: &SqlitePool) -> Result<()> {
    let now = dates::now_db();
    for (name, slug, typ, key_type) in [
        ("Ghost Internal Frontend", "ghost-internal-frontend", "internal", "content"),
        ("Ghost Core Content API", "ghost-core-content", "core", "content"),
        ("Ghost Scheduler", "ghost-scheduler", "internal", "admin"),
        ("Ghost Backup", "ghost-backup", "internal", "admin"),
        ("Zapier", "zapier", "builtin", "admin"),
    ] {
        let integration_id = ids::new_id();
        sqlx::query("INSERT INTO integrations (id, type, name, slug, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&integration_id)
            .bind(typ)
            .bind(name)
            .bind(slug)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;

        let secret_len = if key_type == "admin" { 64 } else { 26 };
        let role_id: Option<String> = if key_type == "admin" {
            sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Admin Integration'")
                .fetch_optional(pool)
                .await
                .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?
        } else {
            None
        };
        let secret = ids::random_hex(secret_len / 2 + if secret_len % 2 == 1 { 1 } else { 0 });
        sqlx::query("INSERT INTO api_keys (id, type, secret, role_id, integration_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(key_type)
            .bind(&secret)
            .bind(&role_id)
            .bind(&integration_id)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }
    Ok(())
}

fn ghost_render_placeholder(md: &str) -> String {
    // comrak lives in ghost-render; avoid the dependency here by doing the
    // trivially equivalent conversion for the two fixture documents.
    let mut html = String::new();
    for line in md.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(rest) = l.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>", rest));
        } else {
            let linked = l.replace(
                "[the admin area](/ghost)",
                r#"<a href="/ghost">the admin area</a>"#,
            );
            html.push_str(&format!("<p>{}</p>", linked));
        }
    }
    html
}

fn html_plain(html: &str) -> String {
    // strip tags for plaintext (fixture-only, tiny inputs)
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Re-seed settings rows that are missing (mirrors Ghost's populateDefaults).
pub async fn populate_missing_settings(pool: &SqlitePool) -> Result<()> {
    let raw = include_str!("../assets/default-settings.json");
    let seeds: Vec<SettingSeed> = serde_json::from_str(raw)?;
    let now = dates::now_db();
    for s in seeds {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings WHERE key = ?")
            .bind(&s.key)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
        if exists == 0 {
            let value = value_to_string(&s.value);
            sqlx::query("INSERT INTO settings (id, `group`, key, value, type, flags, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&ids::new_id())
                .bind(&s.group)
                .bind(&s.key)
                .bind(&value)
                .bind(if s.value_type == "bool" { "boolean".to_string() } else { s.value_type.clone() })
                .bind(if s.flags.is_empty() { None } else { Some(s.flags.clone()) })
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
        }
    }
    Ok(())
}

/// UUID v4 helper for other modules.
pub fn new_uuid_v4() -> String {
    uuid_v4()
}
