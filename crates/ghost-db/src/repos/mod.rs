//! Repositories: async data access returning ghost-core domain models.
//!
//! Row structs are private to ghost-db; repos map them to domain structs.
pub mod posts;
pub mod publishing;
pub mod gift_links;
pub mod members;
pub mod subscriptions;
pub mod tags;
pub mod tiers;
pub mod users;
pub mod settings;
pub mod staff;
pub mod api_keys;
pub mod sessions;
pub mod actions;
pub mod automations;
pub mod comments;
pub mod forms;
pub mod integrations;
pub mod metafields;
pub mod newsletters;
pub mod offers;
pub mod recommendations;
pub mod webhooks;

use ghost_core::models::{Post, PostMeta, Tag, User};

/// Map sqlx errors into GhostError (orphan rules forbid a From impl).
pub(crate) fn dberr(e: sqlx::Error) -> ghost_core::GhostError {
    ghost_core::GhostError::Db(e.to_string())
}

#[derive(Debug)]
pub(crate) struct PostRow {
    pub id: String,
    pub uuid: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub mobiledoc: Option<String>,
    pub lexical: Option<String>,
    pub markdown: Option<String>,
    pub html: Option<String>,
    pub plaintext: Option<String>,
    pub feature_image: Option<String>,
    pub featured: bool,
    pub post_type: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub published_at: Option<String>,
    pub published_by: Option<String>,
    pub custom_excerpt: Option<String>,
    pub email_recipient_filter: Option<String>,
    pub newsletter_id: Option<String>,
    pub reading_time: Option<i64>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub custom_template: Option<String>,
    pub canonical_url: Option<String>,
    pub show_title_and_feature_image: Option<bool>,
}

/// Read by hand rather than derived, because of the boolean columns.
///
/// `#[derive(sqlx::FromRow)]` would decode each `bool` field with
/// `Decode<Any> for bool`, which fails on both engines: a boolean is stored as
/// a 0/1 integer and `Any` will not turn an integer into a `bool`. See
/// [`crate::types::flag`]. Every other field is read exactly as the derive
/// would have read it.
impl<'r> sqlx::FromRow<'r, sqlx::any::AnyRow> for PostRow {
    fn from_row(row: &'r sqlx::any::AnyRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            uuid: row.try_get("uuid")?,
            title: row.try_get("title")?,
            slug: row.try_get("slug")?,
            mobiledoc: crate::types::long_opt(row, "mobiledoc")?,
            lexical: crate::types::long_opt(row, "lexical")?,
            markdown: crate::types::long_opt(row, "markdown")?,
            html: crate::types::long_opt(row, "html")?,
            plaintext: crate::types::long_opt(row, "plaintext")?,
            feature_image: row.try_get("feature_image")?,
            featured: crate::types::flag(row, "featured"),
            // The column is `type`, a reserved word, which is why the
            // field is named differently.
            post_type: row.try_get("type")?,
            status: row.try_get("status")?,
            visibility: row.try_get("visibility")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            published_at: row.try_get("published_at")?,
            published_by: row.try_get("published_by")?,
            custom_excerpt: row.try_get("custom_excerpt")?,
            email_recipient_filter: crate::types::long_opt(row, "email_recipient_filter")?,
            newsletter_id: row.try_get("newsletter_id")?,
            reading_time: row.try_get("reading_time")?,
            codeinjection_head: crate::types::long_opt(row, "codeinjection_head")?,
            codeinjection_foot: crate::types::long_opt(row, "codeinjection_foot")?,
            custom_template: row.try_get("custom_template")?,
            canonical_url: row.try_get("canonical_url")?,
            show_title_and_feature_image: crate::types::flag_opt(row, "show_title_and_feature_image"),
        })
    }
}

pub(crate) const POST_COLS: &str = "id, uuid, title, slug, mobiledoc, lexical, markdown, html, plaintext, feature_image, featured, type, status, visibility, created_at, updated_at, published_at, published_by, custom_excerpt, reading_time, codeinjection_head, codeinjection_foot, custom_template, canonical_url, show_title_and_feature_image, email_recipient_filter, newsletter_id";

impl From<PostRow> for Post {
    fn from(r: PostRow) -> Self {
        Post {
            id: r.id,
            uuid: r.uuid,
            title: r.title,
            slug: r.slug,
            mobiledoc: r.mobiledoc,
            lexical: r.lexical,
            markdown: r.markdown,
            html: r.html,
            plaintext: r.plaintext,
            feature_image: r.feature_image,
            featured: r.featured,
            post_type: r.post_type.unwrap_or_else(|| "post".into()),
            status: r.status.unwrap_or_else(|| "draft".into()),
            visibility: r.visibility.unwrap_or_else(|| "public".into()),
            tier_ids: Vec::new(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
            published_by: r.published_by,
            custom_excerpt: r.custom_excerpt,
            reading_time: r.reading_time.unwrap_or(1),
            codeinjection_head: r.codeinjection_head,
            codeinjection_foot: r.codeinjection_foot,
            custom_template: r.custom_template,
            canonical_url: r.canonical_url,
            show_title_and_feature_image: r.show_title_and_feature_image.unwrap_or(true),
            meta: None,
            email_recipient_filter: r.email_recipient_filter,
            auto_excerpt: None,
            locale: None,
            comment_id: None,
            newsletter_id: r.newsletter_id,
        }
    }
}

#[derive(Debug)]
pub(crate) struct MetaRow {
    pub post_id: String,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub twitter_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub email_subject: Option<String>,
    pub email_preview: Option<String>,
    pub frontmatter: Option<String>,
    pub feature_image_alt: Option<String>,
    pub feature_image_caption: Option<String>,
}

/// Read by hand rather than derived, because of the long columns.
///
/// `#[derive(sqlx::FromRow)]` would decode them as `String`, which fails on
/// MySQL: a `MEDIUMTEXT` arrives as a blob. See [`crate::types::long`].
impl<'r> sqlx::FromRow<'r, sqlx::any::AnyRow> for MetaRow {
    fn from_row(row: &'r sqlx::any::AnyRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            post_id: row.try_get("post_id")?,
            og_image: row.try_get("og_image")?,
            og_title: row.try_get("og_title")?,
            og_description: row.try_get("og_description")?,
            twitter_image: row.try_get("twitter_image")?,
            twitter_title: row.try_get("twitter_title")?,
            twitter_description: row.try_get("twitter_description")?,
            meta_title: row.try_get("meta_title")?,
            meta_description: row.try_get("meta_description")?,
            email_subject: row.try_get("email_subject")?,
            email_preview: row.try_get("email_preview")?,
            frontmatter: crate::types::long_opt(row, "frontmatter")?,
            feature_image_alt: row.try_get("feature_image_alt")?,
            feature_image_caption: row.try_get("feature_image_caption")?,
        })
    }
}

impl From<MetaRow> for PostMeta {
    fn from(r: MetaRow) -> Self {
        PostMeta {
            post_id: r.post_id,
            og_image: r.og_image,
            og_title: r.og_title,
            og_description: r.og_description,
            twitter_image: r.twitter_image,
            twitter_title: r.twitter_title,
            twitter_description: r.twitter_description,
            meta_title: r.meta_title,
            meta_description: r.meta_description,
            email_subject: r.email_subject,
            email_preview: r.email_preview,
            frontmatter: r.frontmatter,
            feature_image_alt: r.feature_image_alt,
            feature_image_caption: r.feature_image_caption,
        }
    }
}

#[derive(Debug)]
pub(crate) struct TagRow {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub feature_image: Option<String>,
    pub parent_id: Option<String>,
    pub visibility: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub twitter_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub canonical_url: Option<String>,
    pub accent_color: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Read by hand rather than derived, because of the long columns.
///
/// `#[derive(sqlx::FromRow)]` would decode them as `String`, which fails on
/// MySQL: a `MEDIUMTEXT` arrives as a blob. See [`crate::types::long`].
impl<'r> sqlx::FromRow<'r, sqlx::any::AnyRow> for TagRow {
    fn from_row(row: &'r sqlx::any::AnyRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            slug: row.try_get("slug")?,
            description: row.try_get("description")?,
            feature_image: row.try_get("feature_image")?,
            parent_id: row.try_get("parent_id")?,
            visibility: row.try_get("visibility")?,
            meta_title: row.try_get("meta_title")?,
            meta_description: row.try_get("meta_description")?,
            og_image: row.try_get("og_image")?,
            og_title: row.try_get("og_title")?,
            og_description: row.try_get("og_description")?,
            twitter_image: row.try_get("twitter_image")?,
            twitter_title: row.try_get("twitter_title")?,
            twitter_description: row.try_get("twitter_description")?,
            codeinjection_head: crate::types::long_opt(row, "codeinjection_head")?,
            codeinjection_foot: crate::types::long_opt(row, "codeinjection_foot")?,
            canonical_url: row.try_get("canonical_url")?,
            accent_color: row.try_get("accent_color")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

pub(crate) const TAG_COLS: &str = "id, name, slug, description, feature_image, parent_id, visibility, meta_title, meta_description, og_image, og_title, og_description, twitter_image, twitter_title, twitter_description, codeinjection_head, codeinjection_foot, canonical_url, accent_color, created_at, updated_at";

impl From<TagRow> for Tag {
    fn from(r: TagRow) -> Self {
        Tag {
            id: r.id,
            name: r.name,
            slug: r.slug,
            description: r.description,
            feature_image: r.feature_image,
            parent_id: r.parent_id,
            visibility: r.visibility.unwrap_or_else(|| "public".into()),
            meta_title: r.meta_title,
            meta_description: r.meta_description,
            og_image: r.og_image,
            og_title: r.og_title,
            og_description: r.og_description,
            twitter_image: r.twitter_image,
            twitter_title: r.twitter_title,
            twitter_description: r.twitter_description,
            codeinjection_head: r.codeinjection_head,
            codeinjection_foot: r.codeinjection_foot,
            canonical_url: r.canonical_url,
            accent_color: r.accent_color,
            created_at: r.created_at,
            updated_at: r.updated_at,
            count_posts: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct UserRow {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub password: Option<String>,
    pub email: Option<String>,
    pub profile_image: Option<String>,
    pub cover_image: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub location: Option<String>,
    pub facebook: Option<String>,
    pub twitter: Option<String>,
    pub threads: Option<String>,
    pub bluesky: Option<String>,
    pub mastodon: Option<String>,
    pub tiktok: Option<String>,
    pub youtube: Option<String>,
    pub instagram: Option<String>,
    pub linkedin: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub accessibility: Option<String>,
    pub last_seen: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comment_notifications: bool,
    pub free_member_signup_notification: bool,
    pub paid_subscription_started_notification: bool,
    pub paid_subscription_canceled_notification: bool,
    pub mention_notifications: bool,
    pub recommendation_notifications: bool,
    pub milestone_notifications: bool,
    pub donation_notifications: bool,
    pub gift_subscription_notifications: bool,
}

/// Read by hand rather than derived, because of the boolean columns.
///
/// `#[derive(sqlx::FromRow)]` would decode each `bool` field with
/// `Decode<Any> for bool`, which fails on both engines: a boolean is stored as
/// a 0/1 integer and `Any` will not turn an integer into a `bool`. See
/// [`crate::types::flag`]. Every other field is read exactly as the derive
/// would have read it.
impl<'r> sqlx::FromRow<'r, sqlx::any::AnyRow> for UserRow {
    fn from_row(row: &'r sqlx::any::AnyRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            slug: row.try_get("slug")?,
            password: row.try_get("password")?,
            email: row.try_get("email")?,
            profile_image: row.try_get("profile_image")?,
            cover_image: row.try_get("cover_image")?,
            bio: row.try_get("bio")?,
            website: row.try_get("website")?,
            location: row.try_get("location")?,
            facebook: row.try_get("facebook")?,
            twitter: row.try_get("twitter")?,
            threads: row.try_get("threads")?,
            bluesky: row.try_get("bluesky")?,
            mastodon: row.try_get("mastodon")?,
            tiktok: row.try_get("tiktok")?,
            youtube: row.try_get("youtube")?,
            instagram: row.try_get("instagram")?,
            linkedin: row.try_get("linkedin")?,
            status: row.try_get("status")?,
            visibility: row.try_get("visibility")?,
            meta_title: row.try_get("meta_title")?,
            meta_description: row.try_get("meta_description")?,
            accessibility: crate::types::long_opt(row, "accessibility")?,
            last_seen: row.try_get("last_seen")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            comment_notifications: crate::types::flag(row, "comment_notifications"),
            free_member_signup_notification: crate::types::flag(row, "free_member_signup_notification"),
            paid_subscription_started_notification: crate::types::flag(row, "paid_subscription_started_notification"),
            paid_subscription_canceled_notification: crate::types::flag(row, "paid_subscription_canceled_notification"),
            mention_notifications: crate::types::flag(row, "mention_notifications"),
            recommendation_notifications: crate::types::flag(row, "recommendation_notifications"),
            milestone_notifications: crate::types::flag(row, "milestone_notifications"),
            donation_notifications: crate::types::flag(row, "donation_notifications"),
            gift_subscription_notifications: crate::types::flag(row, "gift_subscription_notifications"),
        })
    }
}

pub(crate) const USER_COLS: &str = "id, name, slug, password, email, profile_image, cover_image, bio, website, location, facebook, twitter, threads, bluesky, mastodon, tiktok, youtube, instagram, linkedin, status, visibility, meta_title, meta_description, accessibility, last_seen, created_at, updated_at, comment_notifications, free_member_signup_notification, paid_subscription_started_notification, paid_subscription_canceled_notification, mention_notifications, recommendation_notifications, milestone_notifications, donation_notifications, gift_subscription_notifications";

impl From<UserRow> for User {
    fn from(r: UserRow) -> Self {
        User {
            id: r.id,
            name: r.name,
            slug: r.slug,
            password: r.password,
            email: r.email,
            profile_image: r.profile_image,
            cover_image: r.cover_image,
            bio: r.bio,
            website: r.website,
            location: r.location,
            facebook: r.facebook,
            twitter: r.twitter,
            threads: r.threads,
            bluesky: r.bluesky,
            mastodon: r.mastodon,
            tiktok: r.tiktok,
            youtube: r.youtube,
            instagram: r.instagram,
            linkedin: r.linkedin,
            status: r.status,
            locale: None,
            visibility: r.visibility,
            meta_title: r.meta_title,
            meta_description: r.meta_description,
            tour: None,
            last_seen: r.last_seen,
            created_at: r.created_at,
            updated_at: r.updated_at,
            accessibility: r.accessibility,
            comment_notifications: r.comment_notifications,
            free_member_signup_notification: r.free_member_signup_notification,
            paid_subscription_started_notification: r.paid_subscription_started_notification,
            paid_subscription_canceled_notification: r.paid_subscription_canceled_notification,
            mention_notifications: r.mention_notifications,
            recommendation_notifications: r.recommendation_notifications,
            milestone_notifications: r.milestone_notifications,
            donation_notifications: r.donation_notifications,
            gift_subscription_notifications: r.gift_subscription_notifications,
        }
    }
}

/// Batch-loaded relations for a set of posts.
#[derive(Debug, Default)]
pub struct PostRelations {
    pub post_tiers: Vec<(String, String)>,
    pub authors: Vec<User>,
    pub tags: Vec<Tag>,
    /// (post_id, author_id) / (post_id, tag_id) pairs in sort order
    pub post_authors: Vec<(String, String)>,
    pub post_tags: Vec<(String, String)>,
    pub meta: Vec<PostMeta>,
}

impl PostRelations {
    pub fn authors_for(&self, post_id: &str) -> Vec<User> {
        self.post_authors
            .iter()
            .filter(|(p, _)| p == post_id)
            .filter_map(|(_, a)| self.authors.iter().find(|u| u.id == *a))
            .cloned()
            .collect()
    }
    pub fn tags_for(&self, post_id: &str) -> Vec<Tag> {
        self.post_tags
            .iter()
            .filter(|(p, _)| p == post_id)
            .filter_map(|(_, t)| self.tags.iter().find(|tg| tg.id == *t))
            .cloned()
            .collect()
    }
    pub fn meta_for(&self, post_id: &str) -> Option<&PostMeta> {
        self.meta.iter().find(|m| m.post_id == post_id)
    }
}

/// Attach posts_meta onto each post (flattened later by serializers).
pub fn attach_meta(posts: &mut [Post], rel: &PostRelations) {
    for p in posts {
        p.meta = rel.meta_for(&p.id).cloned();
        p.tier_ids = rel.post_tiers.iter().filter(|(id, _)| id == &p.id).map(|(_, tier)| tier.clone()).collect();
    }
}
