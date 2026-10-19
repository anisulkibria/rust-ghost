//! Demo content seeding.
//!
//! Separate from `seed`, which only runs on an empty database: this is an
//! explicit, repeatable action (`ghost seed-demo`) that fills a site with
//! enough real-shaped content — tags, featured images, excerpts, spread-out
//! publish dates — to see a theme working properly.
//!
//! Re-running it replaces the demo posts rather than duplicating them, so it
//! is safe to run after editing `assets/demo-content.json`.
use ghost_core::{dates, ids, GhostError, Result};
use serde::Deserialize;
use sqlx::SqlitePool;

#[derive(Debug, Deserialize)]
struct DemoContent {
    #[serde(default)]
    tags: Vec<DemoTag>,
    #[serde(default)]
    posts: Vec<DemoPost>,
}

#[derive(Debug, Deserialize)]
struct DemoTag {
    name: String,
    slug: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    feature_image: Option<String>,
    #[serde(default)]
    accent_color: Option<String>,
    #[serde(default = "public_visibility")]
    visibility: String,
}

fn public_visibility() -> String {
    "public".to_string()
}

#[derive(Debug, Deserialize)]
struct DemoPost {
    slug: String,
    title: String,
    html: String,
    /// How long ago this was published, so the feed has a believable spread.
    #[serde(default)]
    days_ago: i64,
    #[serde(default)]
    featured: bool,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    feature_image: Option<String>,
    #[serde(default)]
    feature_image_alt: Option<String>,
    #[serde(default)]
    feature_image_caption: Option<String>,
    #[serde(default)]
    custom_excerpt: Option<String>,
    #[serde(default)]
    post_type: Option<String>,
}

/// What a run of the seeder did, for the caller to report.
#[derive(Debug, Default)]
pub struct DemoSummary {
    pub tags: usize,
    pub posts: usize,
    pub featured: usize,
    pub replaced: usize,
}

fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}

/// Insert (or replace) the demo tags and posts.
pub async fn seed_demo(pool: &SqlitePool) -> Result<DemoSummary> {
    let content: DemoContent = serde_json::from_str(include_str!("../assets/demo-content.json"))?;
    let now = dates::now_db();
    let mut summary = DemoSummary::default();

    let author_id = owner_id(pool).await?;

    // tags first — posts reference them by slug
    let mut tag_ids = std::collections::HashMap::new();
    for tag in &content.tags {
        let id = upsert_tag(pool, tag, &now).await?;
        tag_ids.insert(tag.slug.clone(), id);
        summary.tags += 1;
    }

    for post in &content.posts {
        let post_type = post.post_type.as_deref().unwrap_or("post");

        // a previous run's copy goes first; the join tables cascade
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM posts WHERE slug = ? AND type = ?")
                .bind(&post.slug)
                .bind(post_type)
                .fetch_optional(pool)
                .await
                .map_err(db)?;
        if let Some(id) = existing {
            sqlx::query("DELETE FROM posts WHERE id = ?")
                .bind(&id)
                .execute(pool)
                .await
                .map_err(db)?;
            summary.replaced += 1;
        }

        let post_id = ids::new_id();
        let plaintext = html_to_text(&post.html);
        let reading_time = reading_minutes(&post.html, post.feature_image.is_some());
        let published = dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(post.days_ago)));

        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, custom_excerpt, \
             feature_image, type, status, visibility, featured, reading_time, \
             created_at, updated_at, published_at, published_by, show_title_and_feature_image) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'published', 'public', ?, ?, ?, ?, ?, ?, 1)",
        )
        .bind(&post_id)
        .bind(uuid_v4())
        .bind(&post.title)
        .bind(&post.slug)
        .bind(&post.html)
        .bind(&plaintext)
        .bind(post.custom_excerpt.as_deref())
        .bind(post.feature_image.as_deref())
        .bind(post_type)
        .bind(post.featured)
        .bind(reading_time)
        .bind(&published)
        .bind(&published)
        .bind(&published)
        .bind(&author_id)
        .execute(pool)
        .await
        .map_err(db)?;

        // the image alt text and caption themes render live in posts_meta
        if post.feature_image_alt.is_some()
            || post.feature_image_caption.is_some()
            || post.custom_excerpt.is_some()
        {
            sqlx::query(
                "INSERT INTO posts_meta (id, post_id, feature_image_alt, feature_image_caption, meta_description) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(ids::new_id())
            .bind(&post_id)
            .bind(post.feature_image_alt.as_deref())
            .bind(post.feature_image_caption.as_deref())
            .bind(post.custom_excerpt.as_deref().map(truncate_meta))
            .execute(pool)
            .await
            .map_err(db)?;
        }

        sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, 0)")
            .bind(ids::new_id())
            .bind(&post_id)
            .bind(&author_id)
            .execute(pool)
            .await
            .map_err(db)?;

        for (order, slug) in post.tags.iter().enumerate() {
            let Some(tag_id) = tag_ids.get(slug) else {
                tracing::warn!("demo post '{}' references unknown tag '{slug}'", post.slug);
                continue;
            };
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, ?)")
                .bind(ids::new_id())
                .bind(&post_id)
                .bind(tag_id)
                .bind(order as i64)
                .execute(pool)
                .await
                .map_err(db)?;
        }

        summary.posts += 1;
        if post.featured {
            summary.featured += 1;
        }
    }

    Ok(summary)
}

/// Create the tag, or update the one already using that slug.
async fn upsert_tag(pool: &SqlitePool, tag: &DemoTag, now: &str) -> Result<String> {
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM tags WHERE slug = ?")
        .bind(&tag.slug)
        .fetch_optional(pool)
        .await
        .map_err(db)?;

    if let Some(id) = existing {
        sqlx::query(
            "UPDATE tags SET name = ?, description = ?, feature_image = ?, accent_color = ?, \
             visibility = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&tag.name)
        .bind(tag.description.as_deref())
        .bind(tag.feature_image.as_deref())
        .bind(tag.accent_color.as_deref())
        .bind(&tag.visibility)
        .bind(now)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(db)?;
        return Ok(id);
    }

    let id = ids::new_id();
    sqlx::query(
        "INSERT INTO tags (id, name, slug, description, feature_image, accent_color, visibility, \
         created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&tag.name)
    .bind(&tag.slug)
    .bind(tag.description.as_deref())
    .bind(tag.feature_image.as_deref())
    .bind(tag.accent_color.as_deref())
    .bind(&tag.visibility)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(id)
}

async fn owner_id(pool: &SqlitePool) -> Result<String> {
    sqlx::query_scalar(
        "SELECT user_id FROM roles_users ru JOIN roles r ON r.id = ru.role_id \
         WHERE r.name = 'Owner' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| GhostError::Internal("no owner user — seed the database first".into()))
}

/// Port of `@tryghost/helpers` estimatedReadingTimeInMinutes: 275 words a
/// minute, plus 12 seconds for the first image tapering to 3, rounded to the
/// nearest minute. The feature image counts as one.
fn reading_minutes(html: &str, has_feature_image: bool) -> i64 {
    let word_count = html_to_text(html).split_whitespace().count() as f64;
    let image_count = html.matches("<img").count() as i64 + i64::from(has_feature_image);

    let mut seconds = word_count / (275.0 / 60.0);
    for i in 0..image_count {
        seconds += f64::from(std::cmp::max(12 - i as i32, 3));
    }
    (seconds / 60.0).round() as i64
}

/// posts_meta.meta_description is capped at 500 characters by the schema.
fn truncate_meta(s: &str) -> String {
    s.chars().take(499).collect()
}

/// Strip tags for the plaintext column, keeping block boundaries as spaces so
/// word counts and excerpts stay sensible.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn uuid_v4() -> String {
    let mut b = hex::decode(ids::random_hex(16)).unwrap_or_else(|_| vec![0u8; 16]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_content_parses_and_is_self_consistent() {
        let content: DemoContent =
            serde_json::from_str(include_str!("../assets/demo-content.json")).expect("parse");

        assert!(content.posts.len() >= 5, "want a feed worth looking at");
        assert!(content.posts.iter().any(|p| p.featured), "want a featured post");

        let slugs: Vec<&str> = content.tags.iter().map(|t| t.slug.as_str()).collect();
        for post in &content.posts {
            assert!(post.feature_image.is_some(), "{} has no feature image", post.slug);
            assert!(post.custom_excerpt.is_some(), "{} has no excerpt", post.slug);
            assert!(!post.tags.is_empty(), "{} has no tags", post.slug);
            for tag in &post.tags {
                assert!(slugs.contains(&tag.as_str()), "{} references unknown tag {tag}", post.slug);
            }
        }

        // every public tag should be used by something, or the archive 404s
        for tag in content.tags.iter().filter(|t| t.visibility == "public") {
            assert!(
                content.posts.iter().any(|p| p.tags.contains(&tag.slug)),
                "tag {} is never used",
                tag.slug
            );
        }
    }

    #[test]
    fn plaintext_keeps_word_boundaries_across_tags() {
        assert_eq!(html_to_text("<p>One</p><p>two</p>"), "One two");
        assert_eq!(html_to_text("<em>a</em>b"), "a b");
    }
}
