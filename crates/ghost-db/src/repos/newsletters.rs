//! The `newsletters` table.
//!
//! Column-for-column with Ghost's `newsletters` schema (see
//! `core/server/data/schema/schema.js`), because the whole point of the
//! design settings is that they feed the same email template Ghost uses — a
//! renamed or missing column would quietly change how a newsletter looks.
//!
//! The colour columns are deliberately *strings*, not hex codes: Ghost stores
//! sentinels (`light`, `transparent`, `accent`) alongside real `#rrggbb`
//! values and resolves them at render time. `ghost_mail::newsletter` holds
//! that resolution logic, so it must see the raw value.

use ghost_core::Result;
use sqlx::SqlitePool;

use super::dberr;

/// One newsletter. Field order follows the table, not Ghost's API output.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Newsletter {
    pub id: String,
    pub uuid: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub slug: String,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub sender_reply_to: String,
    pub status: String,
    pub visibility: String,
    pub subscribe_on_signup: bool,
    pub sort_order: i64,

    // --- header ---
    pub header_image: Option<String>,
    pub show_header_icon: bool,
    pub show_header_title: bool,
    pub show_header_name: bool,
    pub show_post_title_section: bool,
    pub show_excerpt: bool,
    pub show_feature_image: bool,

    // --- typography ---
    pub title_font_category: String,
    pub title_alignment: String,
    pub title_font_weight: String,
    pub body_font_category: String,

    // --- colours & shapes ---
    pub background_color: String,
    pub header_background_color: String,
    pub post_title_color: Option<String>,
    pub section_title_color: Option<String>,
    pub divider_color: Option<String>,
    pub link_color: Option<String>,
    pub link_style: String,
    pub button_color: Option<String>,
    pub button_style: String,
    pub button_corners: String,
    pub image_corners: String,

    // --- footer / extras ---
    pub footer_content: Option<String>,
    pub show_badge: bool,
    pub show_comment_cta: bool,
    pub show_subscription_details: bool,
    pub show_latest_posts: bool,
    pub show_share_button: bool,
    pub feedback_enabled: bool,

    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Every column the struct reads, in struct order. `FromRow` matches by name,
/// so the order is only for readability — but the list must stay complete.
pub const NEWSLETTER_COLS: &str = "id, uuid, name, description, slug, sender_name, sender_email, \
sender_reply_to, status, visibility, subscribe_on_signup, sort_order, header_image, \
show_header_icon, show_header_title, show_header_name, show_post_title_section, show_excerpt, \
show_feature_image, title_font_category, title_alignment, title_font_weight, body_font_category, \
background_color, header_background_color, post_title_color, section_title_color, divider_color, \
link_color, link_style, button_color, button_style, button_corners, image_corners, footer_content, \
show_badge, show_comment_cta, show_subscription_details, show_latest_posts, show_share_button, \
feedback_enabled, created_at, updated_at";

/// All newsletters, active first, in Ghost's own ordering.
pub async fn list(pool: &SqlitePool) -> Result<Vec<Newsletter>> {
    sqlx::query_as::<_, Newsletter>(&format!(
        "SELECT {NEWSLETTER_COLS} FROM newsletters ORDER BY status = 'archived', sort_order, created_at"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<Newsletter>> {
    sqlx::query_as::<_, Newsletter>(&format!(
        "SELECT {NEWSLETTER_COLS} FROM newsletters WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)
}

/// The newsletter a post goes out on when it does not name one: the first
/// active one by sort order, which is what Ghost calls the default.
pub async fn default_newsletter(pool: &SqlitePool) -> Result<Option<Newsletter>> {
    sqlx::query_as::<_, Newsletter>(&format!(
        "SELECT {NEWSLETTER_COLS} FROM newsletters WHERE status = 'active' \
         ORDER BY sort_order, created_at LIMIT 1"
    ))
    .fetch_optional(pool)
    .await
    .map_err(dberr)
}

/// What an install's first newsletter is called: "Acme Newsletter" for a site
/// titled Acme.
///
/// Ghost's fixture calls it "Default Newsletter" and hides the name in the
/// email header, so nobody ever reads it. This port shows the name, and a
/// subscriber should see the publication they subscribed to rather than a
/// line out of a fixture file.
pub fn default_name(site_title: &str) -> String {
    let title = site_title.trim();
    if title.is_empty() {
        // Nothing to name it after. Ghost's own fixture name is the better
        // placeholder than a stray leading space.
        return "Default Newsletter".to_string();
    }
    format!("{title} Newsletter")
}

/// Name the seeded newsletter after the site, during setup.
///
/// Called once, from the setup handlers: seeding runs before anyone has said
/// what the site is called, so the name it writes is based on the placeholder
/// title and has to be redone when the owner gives the real one. Afterwards
/// the name belongs to the owner — a later title change in Settings leaves it
/// alone, because a newsletter people subscribe to is not something to rename
/// behind their backs.
///
/// Even here only the seeded name is replaced, never one that is already
/// something else.
pub async fn name_after_site(pool: &SqlitePool, seeded_title: &str, title: &str) -> Result<()> {
    let new_name = default_name(title);
    let seeded = default_name(seeded_title);
    sqlx::query(
        "UPDATE newsletters SET name = ?, updated_at = ? \
         WHERE name IN (?, 'Default Newsletter') AND name <> ?",
    )
    .bind(&new_name)
    .bind(ghost_core::dates::now_db())
    .bind(&seeded)
    .bind(&new_name)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// Write back every author-editable column.
///
/// All of them at once rather than a diff: the design editor posts the whole
/// form, and a partial update would leave a half-saved design if a field were
/// ever added to the form and forgotten here.
pub async fn update(pool: &SqlitePool, n: &Newsletter) -> Result<()> {
    let now = ghost_core::dates::now_db();
    sqlx::query(
        "UPDATE newsletters SET \
         name = ?, description = ?, slug = ?, sender_name = ?, sender_email = ?, \
         sender_reply_to = ?, status = ?, visibility = ?, subscribe_on_signup = ?, sort_order = ?, \
         header_image = ?, show_header_icon = ?, show_header_title = ?, show_header_name = ?, \
         show_post_title_section = ?, show_excerpt = ?, show_feature_image = ?, \
         title_font_category = ?, title_alignment = ?, title_font_weight = ?, \
         body_font_category = ?, background_color = ?, header_background_color = ?, \
         post_title_color = ?, section_title_color = ?, divider_color = ?, link_color = ?, \
         link_style = ?, button_color = ?, button_style = ?, button_corners = ?, \
         image_corners = ?, footer_content = ?, show_badge = ?, show_comment_cta = ?, \
         show_subscription_details = ?, show_latest_posts = ?, show_share_button = ?, \
         feedback_enabled = ?, updated_at = ? \
         WHERE id = ?",
    )
    .bind(&n.name)
    .bind(&n.description)
    .bind(&n.slug)
    .bind(&n.sender_name)
    .bind(&n.sender_email)
    .bind(&n.sender_reply_to)
    .bind(&n.status)
    .bind(&n.visibility)
    .bind(n.subscribe_on_signup)
    .bind(n.sort_order)
    .bind(&n.header_image)
    .bind(n.show_header_icon)
    .bind(n.show_header_title)
    .bind(n.show_header_name)
    .bind(n.show_post_title_section)
    .bind(n.show_excerpt)
    .bind(n.show_feature_image)
    .bind(&n.title_font_category)
    .bind(&n.title_alignment)
    .bind(&n.title_font_weight)
    .bind(&n.body_font_category)
    .bind(&n.background_color)
    .bind(&n.header_background_color)
    .bind(&n.post_title_color)
    .bind(&n.section_title_color)
    .bind(&n.divider_color)
    .bind(&n.link_color)
    .bind(&n.link_style)
    .bind(&n.button_color)
    .bind(&n.button_style)
    .bind(&n.button_corners)
    .bind(&n.image_corners)
    .bind(&n.footer_content)
    .bind(n.show_badge)
    .bind(n.show_comment_cta)
    .bind(n.show_subscription_details)
    .bind(n.show_latest_posts)
    .bind(n.show_share_button)
    .bind(n.feedback_enabled)
    .bind(&now)
    .bind(&n.id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// Create a newsletter with Ghost's defaults for everything but the name.
///
/// Returns the new row. The slug is derived from the name and made unique by
/// suffixing, the way Ghost does, because `slug` is a unique index and a
/// second "Weekly digest" is a perfectly reasonable thing to ask for.
pub async fn create(pool: &SqlitePool, name: &str) -> Result<Newsletter> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ghost_core::GhostError::ValidationError {
            message: "A newsletter needs a name.".into(),
            context: None,
        });
    }
    let id = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
    let base = slugify(name);
    let mut slug = base.clone();
    let mut n = 1;
    while slug_taken(pool, &slug).await {
        n += 1;
        slug = format!("{base}-{n}");
    }
    // Sort newcomers to the end of the list.
    let next: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM newsletters")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    sqlx::query(
        "INSERT INTO newsletters (id, uuid, name, slug, sort_order, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(name)
    .bind(&slug)
    .bind(next)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;

    find_by_id(pool, &id)
        .await?
        .ok_or_else(|| ghost_core::GhostError::Db("newsletter vanished after insert".into()))
}

async fn slug_taken(pool: &SqlitePool, slug: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM newsletters WHERE slug = ?")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

fn slugify(s: &str) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "newsletter".to_string()
    } else {
        slug
    }
}

/// How many members are subscribed, for the list page's "N subscribers".
pub async fn subscriber_count(pool: &SqlitePool, newsletter_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM members_newsletters mn JOIN members m ON m.id = mn.member_id \
         WHERE mn.newsletter_id = ? AND m.email_disabled = 0",
    )
    .bind(newsletter_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_url_safe_and_never_empty() {
        assert_eq!(slugify("Weekly Digest"), "weekly-digest");
        assert_eq!(slugify("  Café & Bar!  "), "caf-bar");
        // a name with nothing sluggable in it still needs a slug, because the
        // column is NOT NULL and uniquely indexed
        assert_eq!(slugify("***"), "newsletter");
    }

    #[test]
    fn the_default_newsletter_is_named_after_the_site() {
        assert_eq!(default_name("Acme"), "Acme Newsletter");
        assert_eq!(default_name("  Acme  "), "Acme Newsletter");
        // a site with no title yet would otherwise be called " Newsletter"
        assert_eq!(default_name(""), "Default Newsletter");
        assert_eq!(default_name("   "), "Default Newsletter");
    }

    #[test]
    fn column_list_mentions_every_design_column() {
        for col in [
            "background_color",
            "header_background_color",
            "post_title_color",
            "section_title_color",
            "divider_color",
            "link_color",
            "link_style",
            "button_color",
            "button_style",
            "button_corners",
            "image_corners",
            "title_font_weight",
            "title_font_category",
            "title_alignment",
            "body_font_category",
        ] {
            assert!(
                NEWSLETTER_COLS.contains(col),
                "{col} missing from NEWSLETTER_COLS — it would read as a default"
            );
        }
    }
}
