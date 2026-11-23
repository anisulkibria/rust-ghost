//! The `newsletters` table.
//!
//! Column-for-column with Ghost's `newsletters` schema (see
//! `core/server/data/schema/schema.js`), because the whole point of the
//! design settings is that they feed the same email template Ghost uses — a
//! renamed or missing column would quietly change how a newsletter looks.
//!
//! The colour columns are deliberately *strings*, not hex codes: Ghost stores
//! sentinels (`light`, `transparent`, `accent`) alongside real `#rrggbb`
//! values and resolves them at render time. `vuuut_mail::newsletter` holds
//! that resolution logic, so it must see the raw value.

use vuuut_core::Result;
use crate::Db;

use super::dberr;

/// One newsletter. Field order follows the table, not Ghost's API output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

/// Read by hand rather than derived, because of the boolean columns.
///
/// `#[derive(sqlx::FromRow)]` would decode each `bool` field with
/// `Decode<Any> for bool`, which fails on both engines: a boolean is stored
/// as a 0/1 integer and `Any` will not turn an integer into a `bool`. See
/// [`crate::types::flag`]. Everything else is read exactly as the derive
/// would have read it.
impl<'r> sqlx::FromRow<'r, sqlx::any::AnyRow> for Newsletter {
    fn from_row(row: &'r sqlx::any::AnyRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            uuid: row.try_get("uuid")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            slug: row.try_get("slug")?,
            sender_name: row.try_get("sender_name")?,
            sender_email: row.try_get("sender_email")?,
            sender_reply_to: row.try_get("sender_reply_to")?,
            status: row.try_get("status")?,
            visibility: row.try_get("visibility")?,
            subscribe_on_signup: crate::types::flag(row, "subscribe_on_signup"),
            sort_order: row.try_get("sort_order")?,
            header_image: row.try_get("header_image")?,
            show_header_icon: crate::types::flag(row, "show_header_icon"),
            show_header_title: crate::types::flag(row, "show_header_title"),
            show_header_name: crate::types::flag(row, "show_header_name"),
            show_post_title_section: crate::types::flag(row, "show_post_title_section"),
            show_excerpt: crate::types::flag(row, "show_excerpt"),
            show_feature_image: crate::types::flag(row, "show_feature_image"),
            title_font_category: row.try_get("title_font_category")?,
            title_alignment: row.try_get("title_alignment")?,
            title_font_weight: row.try_get("title_font_weight")?,
            body_font_category: row.try_get("body_font_category")?,
            background_color: row.try_get("background_color")?,
            header_background_color: row.try_get("header_background_color")?,
            post_title_color: row.try_get("post_title_color")?,
            section_title_color: row.try_get("section_title_color")?,
            divider_color: row.try_get("divider_color")?,
            link_color: row.try_get("link_color")?,
            link_style: row.try_get("link_style")?,
            button_color: row.try_get("button_color")?,
            button_style: row.try_get("button_style")?,
            button_corners: row.try_get("button_corners")?,
            image_corners: row.try_get("image_corners")?,
            footer_content: row.try_get("footer_content")?,
            show_badge: crate::types::flag(row, "show_badge"),
            show_comment_cta: crate::types::flag(row, "show_comment_cta"),
            show_subscription_details: crate::types::flag(row, "show_subscription_details"),
            show_latest_posts: crate::types::flag(row, "show_latest_posts"),
            show_share_button: crate::types::flag(row, "show_share_button"),
            feedback_enabled: crate::types::flag(row, "feedback_enabled"),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
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
pub async fn list(pool: &Db) -> Result<Vec<Newsletter>> {
    sqlx::query_as::<_, Newsletter>(&format!(
        "SELECT {NEWSLETTER_COLS} FROM newsletters ORDER BY status = 'archived', sort_order, created_at"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

pub async fn find_by_id(pool: &Db, id: &str) -> Result<Option<Newsletter>> {
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
pub async fn default_newsletter(pool: &Db) -> Result<Option<Newsletter>> {
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
pub async fn name_after_site(pool: &Db, seeded_title: &str, title: &str) -> Result<()> {
    let new_name = default_name(title);
    let seeded = default_name(seeded_title);
    sqlx::query(
        "UPDATE newsletters SET name = ?, updated_at = ? \
         WHERE name IN (?, 'Default Newsletter') AND name <> ?",
    )
    .bind(&new_name)
    .bind(vuuut_core::dates::now_db())
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
pub async fn update(pool: &Db, n: &Newsletter) -> Result<()> {
    let now = vuuut_core::dates::now_db();
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
pub async fn create(pool: &Db, name: &str) -> Result<Newsletter> {
    let name = name.trim();
    if name.is_empty() {
        return Err(vuuut_core::GhostError::ValidationError {
            message: "A newsletter needs a name.".into(),
            context: None,
        });
    }
    let id = vuuut_core::ids::new_id();
    let now = vuuut_core::dates::now_db();
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
    .bind(vuuut_core::ids::uuid_v4())
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
        .ok_or_else(|| vuuut_core::GhostError::Db("newsletter vanished after insert".into()))
}

async fn slug_taken(pool: &Db, slug: &str) -> bool {
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

/// How many members are subscribed, for one newsletter.
///
/// Prefer [`stats`] when the caller wants this for every newsletter: that is
/// one query where this is one per newsletter.
///
/// "Subscribed" is the same definition [`SUBSCRIBED`] gives — confirmed, and
/// able to receive email. See its comment for why both halves are needed.
pub async fn subscriber_count(pool: &Db, newsletter_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM members_newsletters mn JOIN members m ON m.id = mn.member_id \
         WHERE mn.newsletter_id = ? AND {SUBSCRIBED}"
    ))
    .bind(newsletter_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// the sending programme — what the Newsletters module reports on
// ---------------------------------------------------------------------------

/// What it means to be subscribed, as one SQL fragment.
///
/// **Both halves are needed and they were not always both applied.** A row in
/// `members_newsletters` whose status is not `confirmed` is somebody who has
/// not agreed yet, and a member with `email_disabled` has had sending to them
/// switched off after messages bounced. Either one means the next send does
/// not reach them, so either one counted as reach overstates it — and reach
/// is the figure a decision to send is made on.
///
/// One constant rather than the condition written out at each call site,
/// because this admin briefly had two definitions: the settings page counted
/// `email_disabled = 0` and ignored the status, while the Memberships card
/// counted `status = 'confirmed'` and ignored the disabled flag. The same
/// newsletter could therefore show two different subscriber counts on two
/// screens. There is no request data in it — it is a literal interpolated
/// into a literal, never anything bound.
pub const SUBSCRIBED: &str = "mn.status = 'confirmed' AND m.email_disabled = 0";

/// One newsletter, with how many members it actually reaches.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct NewsletterStat {
    pub id: String,
    pub name: String,
    /// `active` or `archived`.
    pub status: String,
    /// `members` or `paid`.
    pub visibility: String,
    pub subscribers: i64,
}

impl NewsletterStat {
    /// Whether this newsletter is one a post can be sent to.
    pub fn is_sending(&self) -> bool {
        self.status == "active"
    }
}

/// Every newsletter with its subscriber count, sending ones first.
///
/// One query with a correlated subquery rather than a query per newsletter: a
/// site with six newsletters would otherwise cost seven round trips to draw
/// one card, which is what the settings page used to do.
pub async fn stats(pool: &Db) -> Result<Vec<NewsletterStat>> {
    sqlx::query_as(&format!(
        "SELECT n.id, n.name, n.status, n.visibility, \
         (SELECT COUNT(*) FROM members_newsletters mn JOIN members m ON m.id = mn.member_id \
           WHERE mn.newsletter_id = n.id AND {SUBSCRIBED}) AS subscribers \
         FROM newsletters n \
         ORDER BY CASE n.status WHEN 'active' THEN 0 ELSE 1 END, n.sort_order, n.created_at"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// Every count the Newsletters overview shows, in one round trip.
///
/// A struct rather than a tuple of fourteen integers, for the reason
/// [`super::dashboard::Snapshot`] gives: nothing catches a transposed pair in
/// a tuple that wide, and `(delivered, failed)` the wrong way round compiles,
/// runs and quietly lies on the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct Counts {
    pub newsletters: i64,
    /// Newsletters with `status = 'active'`.
    pub sending_newsletters: i64,
    /// Distinct members with a confirmed subscription to at least one.
    pub subscribers: i64,
    /// Confirmed subscription rows: a member on three newsletters is three.
    pub subscriptions: i64,
    pub members: i64,
    /// Every `emails` row — one per post ever emailed.
    pub editions: i64,
    /// Editions created inside the window.
    pub editions_in_window: i64,
    /// Editions whose own status is `failed`: the send came to nothing, as
    /// opposed to individual recipients bouncing.
    pub failed_editions: i64,
    /// Editions accepted and not finished.
    pub pending_editions: i64,
    /// Recipient rows written in the window — messages attempted.
    pub attempted: i64,
    /// Attempts that reached somebody.
    pub delivered: i64,
    /// Attempts with an `opened_at`. Always zero when open tracking is off,
    /// which is why the rules refuse to turn it into a rate — see
    /// [`vuuut_core::newsletters::Programme::open_rate`].
    pub opened: i64,
    pub failed: i64,
    /// Distinct members who clicked a newsletter link in the window.
    pub clicked: i64,
}

/// Count everything the Newsletters overview reports.
///
/// `since` opens the window, as a `'YYYY-MM-DD HH:MM:SS'` UTC string computed
/// by the caller — SQLite would take `datetime('now', ?)` and MySQL's
/// `INTERVAL` is syntax rather than a value, so the whole project computes
/// these in Rust. See [`vuuut_core::dialect`].
///
/// One statement, so one failure mode: if the read fails the screen shows
/// nothing rather than a page silently mixing fresh counts with missing ones.
///
/// Nothing here reads a wide column. `emails.html`, `plaintext`, `source`,
/// `recipient_filter` and `error_data` are all `MEDIUMTEXT` on MySQL and
/// cannot be decoded as a `String` through the `Any` driver at all; a report
/// has no use for the body of an email that was sent last March.
pub async fn programme(pool: &Db, since: &str) -> Result<Counts> {
    sqlx::query_as(&format!(
        "SELECT \
         (SELECT COUNT(*) FROM newsletters) AS newsletters, \
         (SELECT COUNT(*) FROM newsletters WHERE status = 'active') AS sending_newsletters, \
         (SELECT COUNT(DISTINCT mn.member_id) FROM members_newsletters mn \
           JOIN members m ON m.id = mn.member_id WHERE {SUBSCRIBED}) AS subscribers, \
         (SELECT COUNT(*) FROM members_newsletters mn \
           JOIN members m ON m.id = mn.member_id WHERE {SUBSCRIBED}) AS subscriptions, \
         (SELECT COUNT(*) FROM members) AS members, \
         (SELECT COUNT(*) FROM emails) AS editions, \
         (SELECT COUNT(*) FROM emails WHERE created_at >= ?) AS editions_in_window, \
         (SELECT COUNT(*) FROM emails WHERE status = 'failed') AS failed_editions, \
         (SELECT COUNT(*) FROM emails WHERE status IN ('pending', 'submitting')) AS pending_editions, \
         (SELECT COUNT(*) FROM email_recipients WHERE created_at >= ?) AS attempted, \
         (SELECT COUNT(*) FROM email_recipients WHERE created_at >= ? AND status = 'sent') AS delivered, \
         (SELECT COUNT(*) FROM email_recipients WHERE created_at >= ? AND opened_at IS NOT NULL) AS opened, \
         (SELECT COUNT(*) FROM email_recipients WHERE created_at >= ? AND status = 'failed') AS failed, \
         (SELECT COUNT(DISTINCT member_uuid) FROM newsletter_clicks WHERE created_at >= ?) AS clicked"
    ))
    .bind(since)
    .bind(since)
    .bind(since)
    .bind(since)
    .bind(since)
    .bind(since)
    .fetch_one(pool)
    .await
    .map_err(dberr)
}

/// When the last edition was created, or `None` if nothing has ever gone out.
///
/// Its own scalar rather than a column on [`Counts`]: it is the one nullable
/// value among fifteen integers, and `MAX` over an empty table comes back as
/// NULL, which would make the whole struct fail to decode.
pub async fn last_sent(pool: &Db) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT MAX(created_at) FROM emails")
        .fetch_one(pool)
        .await
        .ok()
        .flatten()
}

/// One edition: a post that was emailed, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Edition {
    pub id: String,
    pub post_id: String,
    /// The post's title. `None` if the post has been deleted under it.
    pub post_title: Option<String>,
    pub post_slug: Option<String>,
    /// The subject line actually sent, which may differ from the title.
    pub subject: Option<String>,
    /// The newsletter it went to. `None` for an edition that predates the
    /// column or whose newsletter was removed.
    pub newsletter_name: Option<String>,
    /// `submitted`, `sent`, `failed`, `pending` — see
    /// [`vuuut_core::newsletters::edition_status`].
    pub status: String,
    /// Why it failed, when it did. `VARCHAR(2000)` on both engines, so it is
    /// safe to read as a `String` — unlike `error_data` beside it, which is
    /// `MEDIUMTEXT` and is deliberately not selected.
    pub error: Option<String>,
    pub created_at: String,
    pub submitted_at: Option<String>,
    /// Recipient rows written for this edition.
    pub attempted: i64,
    pub delivered: i64,
    pub opened: i64,
    pub failed: i64,
}

impl Edition {
    /// What to call this edition on screen.
    ///
    /// The post's title, then the subject line it was sent under, then a
    /// placeholder. Never empty, because the title is a link's text and a
    /// link with no text cannot be clicked.
    pub fn title(&self) -> &str {
        for candidate in [self.post_title.as_deref(), self.subject.as_deref()] {
            if let Some(text) = candidate.map(str::trim).filter(|t| !t.is_empty()) {
                return text;
            }
        }
        "Untitled edition"
    }
}

/// The column list and joins every edition read shares.
///
/// The counts are correlated subqueries rather than a `GROUP BY` over a join,
/// so that an edition with no recipient rows still comes back — as a row of
/// zeroes, which is exactly what a failed send looks like and exactly what
/// somebody is looking for. A `JOIN … GROUP BY` would drop it.
const EDITION_SELECT: &str = "SELECT e.id, e.post_id, p.title AS post_title, p.slug AS post_slug, \
     e.subject, n.name AS newsletter_name, e.status, e.error, e.created_at, e.submitted_at, \
     (SELECT COUNT(*) FROM email_recipients r WHERE r.email_id = e.id) AS attempted, \
     (SELECT COUNT(*) FROM email_recipients r WHERE r.email_id = e.id AND r.status = 'sent') AS delivered, \
     (SELECT COUNT(*) FROM email_recipients r WHERE r.email_id = e.id AND r.opened_at IS NOT NULL) AS opened, \
     (SELECT COUNT(*) FROM email_recipients r WHERE r.email_id = e.id AND r.status = 'failed') AS failed \
     FROM emails e LEFT JOIN posts p ON p.id = e.post_id \
     LEFT JOIN newsletters n ON n.id = e.newsletter_id";

/// Editions, newest first.
///
/// `limit` and `offset` are clamped, as every list query in this project
/// clamps them: a limit that came from a request and was passed through would
/// let one page ask for the whole table.
pub async fn editions(pool: &Db, limit: i64, offset: i64) -> Result<Vec<Edition>> {
    sqlx::query_as(&format!("{EDITION_SELECT} ORDER BY e.created_at DESC, e.id DESC LIMIT ? OFFSET ?"))
        .bind(limit.clamp(1, 100))
        .bind(offset.max(0))
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

/// One edition by id, for the screen that is about to retry it.
pub async fn find_edition(pool: &Db, id: &str) -> Result<Option<Edition>> {
    sqlx::query_as(&format!("{EDITION_SELECT} WHERE e.id = ? LIMIT 1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// How many editions there are, for the pager.
pub async fn edition_count(pool: &Db) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM emails").fetch_one(pool).await.unwrap_or(0)
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
