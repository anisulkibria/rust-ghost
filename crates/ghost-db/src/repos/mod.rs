//! Repositories: async data access returning ghost-core domain models.
//!
//! Row structs are private to ghost-db; repos map them to domain structs.
pub mod posts;
pub mod tags;
pub mod users;
pub mod settings;
pub mod staff;
pub mod api_keys;
pub mod sessions;
pub mod actions;
pub mod comments;
pub mod integrations;
pub mod newsletters;

use ghost_core::models::{Post, PostMeta, Tag, User};

/// Map sqlx errors into GhostError (orphan rules forbid a From impl).
pub(crate) fn dberr(e: sqlx::Error) -> ghost_core::GhostError {
    ghost_core::GhostError::Db(e.to_string())
}

#[derive(Debug, sqlx::FromRow)]
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
    #[sqlx(rename = "type")]
    pub post_type: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub published_at: Option<String>,
    pub published_by: Option<String>,
    pub custom_excerpt: Option<String>,
    pub reading_time: Option<i64>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub custom_template: Option<String>,
    pub canonical_url: Option<String>,
    pub show_title_and_feature_image: Option<bool>,
}

pub(crate) const POST_COLS: &str = "id, uuid, title, slug, mobiledoc, lexical, markdown, html, plaintext, feature_image, featured, type, status, visibility, created_at, updated_at, published_at, published_by, custom_excerpt, reading_time, codeinjection_head, codeinjection_foot, custom_template, canonical_url, show_title_and_feature_image";

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
            email_recipient_filter: None,
            auto_excerpt: None,
            locale: None,
            comment_id: None,
            newsletter_id: None,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
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

#[derive(Debug, sqlx::FromRow)]
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

#[derive(Debug, sqlx::FromRow)]
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
    pub last_seen: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

pub(crate) const USER_COLS: &str = "id, name, slug, password, email, profile_image, cover_image, bio, website, location, facebook, twitter, threads, bluesky, mastodon, tiktok, youtube, instagram, linkedin, status, visibility, meta_title, meta_description, last_seen, created_at, updated_at";

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
            accessibility: None,
        }
    }
}

/// Batch-loaded relations for a set of posts.
#[derive(Debug, Default)]
pub struct PostRelations {
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
    }
}
