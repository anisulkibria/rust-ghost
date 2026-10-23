//! Posts repository — browse with NQL filters, read, add, edit, destroy.
use ghost_core::{dates, ids, models::Post, slug, GhostError, Result};
use ghost_nql::{self, Expr};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use std::collections::BTreeSet;

use super::{attach_meta, dberr, PostRow, PostRelations, POST_COLS};

#[derive(Debug, Default, Clone)]
pub struct BrowsePosts {
    /// Extra NQL filters ANDed to the enforced ones.
    pub filter: Option<Expr>,
    pub order: Option<String>,
    /// 15 default; negative = all.
    pub limit: i64,
    pub page: i64,
    /// Restrict to type: post/page (None = both).
    pub post_type: Option<String>,
    /// Content-API semantics: only published.
    pub public: bool,
    pub include_authors: bool,
    pub include_tags: bool,
}

#[derive(Debug)]
pub struct PostsPage {
    pub posts: Vec<Post>,
    pub total: i64,
    pub limit: i64,
    pub page: i64,
}

fn enforce_exprs(q: &BrowsePosts) -> Vec<Expr> {
    let mut parts = Vec::new();
    if q.public {
        parts.push(cond("status", "published"));
    }
    if let Some(t) = &q.post_type {
        parts.push(cond("type", t));
    }
    if let Some(f) = &q.filter {
        parts.push(f.clone());
    }
    parts
}

fn cond(key: &str, value: &str) -> Expr {
    Expr::Cond(ghost_nql::Cond {
        key: key.into(),
        cmp: ghost_nql::Cmp::Eq,
        value: ghost_nql::NqlValue::Str(value.into()),
    })
}

fn order_clause(order: Option<&str>) -> String {
    let allowed = [
        "published_at", "updated_at", "created_at", "title", "slug", "featured", "reading_time", "id", "status",
    ];
    let default = "posts.published_at DESC, posts.id DESC".to_string();
    let Some(order) = order else { return default };
    let mut parts = Vec::new();
    for part in order.split(',') {
        let mut tokens = part.trim().split_whitespace();
        let (Some(col), dir) = (tokens.next(), tokens.next().unwrap_or("DESC")) else {
            continue;
        };
        if !allowed.contains(&col) {
            continue;
        }
        let dir = if dir.eq_ignore_ascii_case("asc") { "ASC" } else { "DESC" };
        parts.push(format!("posts.{col} {dir}"));
    }
    if parts.is_empty() {
        default
    } else {
        parts.join(", ")
    }
}

/// Push a JSON bind value onto a QueryBuilder (writes the `?` too).
pub(crate) fn push_bind(qb: &mut QueryBuilder<'_, Sqlite>, v: &serde_json::Value) {
    match v {
        serde_json::Value::String(s) => {
            qb.push_bind(s.clone());
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                qb.push_bind(i);
            } else {
                qb.push_bind(n.as_f64().unwrap_or(0.0));
            }
        }
        serde_json::Value::Bool(b) => {
            qb.push_bind(*b);
        }
        serde_json::Value::Null => {
            qb.push_bind(Option::<String>::None);
        }
        other => {
            qb.push_bind(other.to_string());
        }
    }
}

/// Push a compiled NQL clause (with embedded `?` marks) and its binds,
/// interleaving text and placeholders — push_bind writes its own `?`, so
/// the clause must be split rather than pushed whole.
pub(crate) fn push_clause_with_binds(qb: &mut QueryBuilder<'_, Sqlite>, clause: &str, binds: &[serde_json::Value]) {
    let mut parts = clause.split('?');
    if let Some(first) = parts.next() {
        qb.push(first);
    }
    for (part, val) in parts.zip(binds.iter()) {
        push_bind(qb, val);
        qb.push(part);
    }
}

pub async fn browse(pool: &SqlitePool, q: &BrowsePosts) -> Result<PostsPage> {
    let enforced = enforce_exprs(q);
    let combined = enforced.into_iter().reduce(|a, b| Expr::And(vec![a, b]));

    let mut qb = QueryBuilder::<Sqlite>::new("SELECT ");
    qb.push(POST_COLS);
    qb.push(" FROM posts");

    if let Some(expr) = combined {
        let sql = ghost_nql::compile(&expr, "posts")?;
        qb.push(" WHERE ");
        push_clause_with_binds(&mut qb, &sql.clause, &sql.binds);
    }
    let limit = if q.limit > 0 { q.limit } else { 15 };
    let page = q.page.max(1);
    let order = order_clause(q.order.as_deref());
    qb.push(" ORDER BY ");
    qb.push(order);

    if q.limit >= 0 {
        qb.push(" LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind((page - 1) * limit);
    }

    let rows: Vec<PostRow> = qb.build_query_as::<PostRow>().fetch_all(pool).await.map_err(dberr)?;

    // total count for pagination meta
    let mut cqb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM posts");
    if let Some(expr) = enforce_count_expr(q) {
        let sql = ghost_nql::compile(&expr, "posts")?;
        cqb.push(" WHERE ");
        push_clause_with_binds(&mut cqb, &sql.clause, &sql.binds);
    }
    let total: i64 = cqb.build_query_scalar().fetch_one(pool).await.map_err(dberr)?;

    let mut posts: Vec<Post> = rows.into_iter().map(Into::into).collect();
    let rel = load_relations(pool, &posts).await?;
    attach_meta(&mut posts, &rel);
    Ok(PostsPage { posts, total, limit, page })
}

fn enforce_count_expr(q: &BrowsePosts) -> Option<Expr> {
    let parts = enforce_exprs(q);
    parts.into_iter().reduce(|a, b| Expr::And(vec![a, b]))
}

/// Load authors/tags/meta for a batch of posts (avoids N+1).
pub async fn load_relations(pool: &SqlitePool, posts: &[Post]) -> Result<PostRelations> {
    if posts.is_empty() {
        return Ok(PostRelations::default());
    }
    let post_ids: BTreeSet<&str> = posts.iter().map(|p| p.id.as_str()).collect();
    let mut rel = PostRelations::default();

    let author_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT post_id, author_id FROM posts_authors ORDER BY sort_order, id",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let author_ids: BTreeSet<String> = author_rows
        .iter()
        .filter(|(p, _)| post_ids.contains(p.as_str()))
        .map(|(_, a)| a.clone())
        .collect();
    rel.post_authors = author_rows
        .into_iter()
        .filter(|(p, _)| post_ids.contains(p.as_str()))
        .collect();
    if !author_ids.is_empty() {
        rel.authors = super::users::find_by_ids(pool, &author_ids).await?;
    }

    let tag_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT post_id, tag_id FROM posts_tags ORDER BY sort_order, id",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let tag_ids: BTreeSet<String> = tag_rows
        .iter()
        .filter(|(p, _)| post_ids.contains(p.as_str()))
        .map(|(_, t)| t.clone())
        .collect();
    rel.post_tags = tag_rows
        .into_iter()
        .filter(|(p, _)| post_ids.contains(p.as_str()))
        .collect();
    if !tag_ids.is_empty() {
        rel.tags = super::tags::find_by_ids(pool, &tag_ids).await?;
    }

    let meta_rows = sqlx::query_as::<_, super::MetaRow>(
        "SELECT post_id, og_image, og_title, og_description, twitter_image, twitter_title, twitter_description, meta_title, meta_description, email_subject, email_preview, frontmatter, feature_image_alt, feature_image_caption FROM posts_meta",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    rel.meta = meta_rows
        .into_iter()
        .filter(|m| post_ids.contains(m.post_id.as_str()))
        .map(Into::into)
        .collect();

    Ok(rel)
}

pub async fn find_by_slug(pool: &SqlitePool, slug_: &str, post_type: &str, public: bool) -> Result<Option<Post>> {
    let post = sqlx::query_as::<_, PostRow>(&format!(
        "SELECT {POST_COLS} FROM posts WHERE slug = ? AND type = ? {} LIMIT 1",
        if public { "AND status = 'published'" } else { "" }
    ))
    .bind(slug_)
    .bind(post_type)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?
    .map(Into::into);
    let Some(mut post) = post else { return Ok(None) };
    let rel = load_relations(pool, std::slice::from_ref(&post)).await?;
    post.meta = rel.meta_for(&post.id).cloned();
    Ok(Some(post))
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<Post>> {
    let post = sqlx::query_as::<_, PostRow>(&format!("SELECT {POST_COLS} FROM posts WHERE id = ? LIMIT 1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .map(Into::into);
    let Some(mut post) = post else { return Ok(None) };
    let rel = load_relations(pool, std::slice::from_ref(&post)).await?;
    post.meta = rel.meta_for(&post.id).cloned();
    Ok(Some(post))
}

pub async fn find_by_uuid(pool: &SqlitePool, uuid: &str) -> Result<Option<Post>> {
    let post = sqlx::query_as::<_, PostRow>(&format!("SELECT {POST_COLS} FROM posts WHERE uuid = ? LIMIT 1"))
        .bind(uuid)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .map(Into::into);
    let Some(mut post) = post else { return Ok(None) };
    let rel = load_relations(pool, std::slice::from_ref(&post)).await?;
    post.meta = rel.meta_for(&post.id).cloned();
    Ok(Some(post))
}

/// Input for creating a post; html/plaintext/reading-time are precomputed
/// by the caller (the markdown pipeline lives in ghost-render).
#[derive(Debug, Default, Clone)]
pub struct NewPost {
    pub title: String,
    pub slug: Option<String>,
    /// Koenig's editor state. Posts written in the admin editor carry this;
    /// `markdown` is the older path and the two are mutually exclusive in
    /// practice — whichever is set is the source the editor reopens.
    pub lexical: Option<String>,
    pub markdown: Option<String>,
    pub html: Option<String>,
    pub plaintext: Option<String>,
    pub custom_excerpt: Option<String>,
    pub feature_image: Option<String>,
    pub featured: bool,
    pub post_type: String,
    pub status: String,
    pub visibility: String,
    pub published_at: Option<String>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub custom_template: Option<String>,
    pub canonical_url: Option<String>,
    /// `None` keeps Ghost's default of showing both.
    pub show_title_and_feature_image: Option<bool>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub twitter_image: Option<String>,
    pub email_subject: Option<String>,
    pub feature_image_alt: Option<String>,
    pub feature_image_caption: Option<String>,
    pub author_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub reading_time: i64,
}

pub async fn slug_exists(pool: &SqlitePool, slug_: &str, post_type: &str, self_id: Option<&str>) -> Result<bool> {
    let count: i64 = match self_id {
        Some(id) => {
            sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE slug = ? AND type = ? AND id != ?")
                .bind(slug_)
                .bind(post_type)
                .bind(id)
                .fetch_one(pool)
                .await
        }
        None => {
            sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE slug = ? AND type = ?")
                .bind(slug_)
                .bind(post_type)
                .fetch_one(pool)
                .await
        }
    }
    .map_err(dberr)?;
    Ok(count > 0)
}

pub async fn generate_unique_slug(pool: &SqlitePool, title: &str, post_type: &str, self_id: Option<&str>) -> Result<String> {
    let mut s = slug::safe_string(title);
    if s.len() > 185 {
        s.truncate(185);
        s = s.trim_matches('-').to_string();
    }
    if s.is_empty() {
        s = format!("{post_type}-1");
    }
    let protected = ["ghost", "rss", "amp", "feed"];
    if protected.contains(&s.as_str()) {
        s = format!("{s}-{post_type}");
    }
    let mut candidate = s.clone();
    let mut i = 1;
    while slug_exists(pool, &candidate, post_type, self_id).await? {
        i += 1;
        candidate = format!("{s}-{i}");
    }
    Ok(candidate)
}

/// Return a content field to stored form before it is persisted.
///
/// Everything this port serves carries the install's base path, so content
/// round-tripped through the editor or the Admin API comes back with
/// `/blog/content/images/…` in it. Normalizing on the way in keeps the base
/// path out of the database, so moving the site to a different path stays a
/// config change rather than a data migration.
///
/// A no-op for an install at a domain root.
fn stored(v: &Option<String>) -> Option<String> {
    v.as_deref().map(ghost_core::url::unbase_assets)
}

/// [`stored`] for rendered HTML, which also carries internal links.
///
/// `unbase_assets` only normalizes Ghost's own asset trees, where a textual
/// match is unambiguous. HTML can be read more precisely than that, so every
/// URL attribute is normalized here — including `href="/blog/about/"`, which
/// is what someone gets by pasting a link out of their address bar. Without
/// this it would be stored with the base path already on it and come back out
/// of the renderer as `/blog/blog/about/`.
fn stored_html(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(ghost_core::url::unrewrite_html)
        .map(|h| ghost_core::url::unbase_assets(&h))
}

pub async fn insert(pool: &SqlitePool, new: &NewPost, author_id: &str) -> Result<Post> {
    let id = ids::new_id();
    let now = dates::now_db();
    let now_dt = chrono::Utc::now();
    let post_type = if new.post_type.is_empty() { "post".into() } else { new.post_type.clone() };
    let slug_ = match &new.slug {
        Some(s) if !s.trim().is_empty() => {
            let s2 = slug::safe_string(s);
            if slug_exists(pool, &s2, &post_type, None).await? {
                generate_unique_slug(pool, &s2, &post_type, None).await?
            } else {
                s2
            }
        }
        _ => generate_unique_slug(pool, &new.title, &post_type, None).await?,
    };

    let status = if new.status.is_empty() { "draft".to_string() } else { new.status.clone() };
    let published_at = if status == "published" {
        Some(new.published_at.clone().unwrap_or_else(|| dates::to_db_string(&now_dt)))
    } else {
        new.published_at.clone()
    };

    let reading_time = if new.reading_time > 0 { new.reading_time } else { 1 };

    sqlx::query("INSERT INTO posts (id, uuid, title, slug, lexical, markdown, html, plaintext, type, status, visibility, featured, feature_image, custom_excerpt, reading_time, codeinjection_head, codeinjection_foot, custom_template, canonical_url, created_at, updated_at, published_at, published_by, show_title_and_feature_image) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&ids::random_hex(16))
        .bind(&new.title)
        .bind(&slug_)
        .bind(stored(&new.lexical))
        .bind(stored(&new.markdown))
        .bind(stored_html(&new.html))
        .bind(&new.plaintext)
        .bind(&post_type)
        .bind(&status)
        .bind(if new.visibility.is_empty() { "public" } else { new.visibility.as_str() })
        .bind(new.featured)
        .bind(stored(&new.feature_image))
        .bind(&new.custom_excerpt)
        .bind(reading_time)
        .bind(&new.codeinjection_head)
        .bind(&new.codeinjection_foot)
        .bind(&new.custom_template)
        .bind(&new.canonical_url)
        .bind(&now)
        .bind(&now)
        .bind(&published_at)
        .bind(author_id)
        .bind(new.show_title_and_feature_image.unwrap_or(true))
        .execute(pool)
        .await
        .map_err(dberr)?;

    let meta = [
        &new.meta_title,
        &new.meta_description,
        &new.og_title,
        &new.og_description,
        &new.og_image,
        &new.twitter_title,
        &new.twitter_description,
        &new.twitter_image,
        &new.email_subject,
        &new.feature_image_alt,
        &new.feature_image_caption,
    ];
    if meta.iter().any(|v| v.is_some()) {
        sqlx::query("INSERT INTO posts_meta (id, post_id, meta_title, meta_description, og_title, og_description, og_image, twitter_title, twitter_description, twitter_image, email_subject, feature_image_alt, feature_image_caption) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(&id)
            .bind(&new.meta_title)
            .bind(&new.meta_description)
            .bind(&new.og_title)
            .bind(&new.og_description)
            .bind(&new.og_image)
            .bind(&new.twitter_title)
            .bind(&new.twitter_description)
            .bind(&new.twitter_image)
            .bind(&new.email_subject)
            .bind(&new.feature_image_alt)
            .bind(&new.feature_image_caption)
            .execute(pool)
            .await
            .map_err(dberr)?;
    }

    let mut author_ids = new.author_ids.clone();
    if author_ids.is_empty() {
        author_ids.push(author_id.to_string());
    }
    for (i, aid) in author_ids.iter().enumerate() {
        sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(&id)
            .bind(aid)
            .bind(i as i64)
            .execute(pool)
            .await
            .map_err(dberr)?;
    }

    for (i, tid) in new.tag_ids.iter().enumerate() {
        sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(&id)
            .bind(tid)
            .bind(i as i64)
            .execute(pool)
            .await
            .map_err(dberr)?;
    }

    find_by_id(pool, &id).await?.ok_or_else(|| GhostError::Internal("post vanished after insert".into()))
}

/// Editable fields (None = unchanged).
#[derive(Debug, Default, Clone)]
pub struct PostChanges {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub lexical: Option<String>,
    pub markdown: Option<String>,
    pub html: Option<String>,
    pub plaintext: Option<String>,
    pub custom_excerpt: Option<String>,
    pub feature_image: Option<String>,
    pub featured: Option<bool>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub published_at: Option<String>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub custom_template: Option<String>,
    pub canonical_url: Option<String>,
    pub show_title_and_feature_image: Option<bool>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub twitter_image: Option<String>,
    pub email_subject: Option<String>,
    pub feature_image_alt: Option<String>,
    pub feature_image_caption: Option<String>,
    pub reading_time: Option<i64>,
    pub tag_ids: Option<Vec<String>>,
    pub author_ids: Option<Vec<String>>,
    /// Fields the caller is editing: a key listed here is written even when
    /// its value is `None`, which is how a settings field gets *cleared*.
    /// Everything else keeps the value already in the row, so a partial
    /// update cannot blank a column it never offered.
    pub clearing: Vec<&'static str>,
}

pub async fn update(pool: &SqlitePool, id: &str, changes: &PostChanges) -> Result<Post> {
    let existing = find_by_id(pool, id).await?.ok_or(ghost_core::GhostError::NotFoundError { message: "not found".into(), context: None })?;

    let now = dates::now_db();
    let title = changes.title.clone().unwrap_or_else(|| existing.title.clone().unwrap_or_default());
    let slug_ = match &changes.slug {
        Some(s) if !s.trim().is_empty() && s != &existing.slug.clone().unwrap_or_default() => {
            let s2 = slug::safe_string(s);
            if slug_exists(pool, &s2, &existing.post_type, Some(id)).await? {
                generate_unique_slug(pool, &s2, &existing.post_type, Some(id)).await?
            } else {
                s2
            }
        }
        _ => existing.slug.clone().unwrap_or_default(),
    };
    let status = changes.status.clone().unwrap_or_else(|| existing.status.clone());
    let published_at = if status == "published" && existing.published_at.is_none() {
        Some(changes.published_at.clone().unwrap_or_else(|| now.clone()))
    } else if let Some(pa) = &changes.published_at {
        Some(pa.clone())
    } else {
        existing.published_at.clone()
    };

    // `None` normally means "leave it alone", but a field the caller named in
    // `clearing` is one it is editing, so an empty value there is an erasure.
    let pick = |new: &Option<String>, old: &Option<String>, field: &str| -> Option<String> {
        match new {
            Some(v) => Some(v.clone()),
            None if changes.clearing.contains(&field) => None,
            None => old.clone(),
        }
    };

    sqlx::query("UPDATE posts SET title = ?, slug = ?, lexical = ?, markdown = ?, html = ?, plaintext = ?, custom_excerpt = ?, feature_image = ?, featured = ?, status = ?, visibility = ?, published_at = ?, codeinjection_head = ?, codeinjection_foot = ?, custom_template = ?, canonical_url = ?, show_title_and_feature_image = ?, reading_time = ?, updated_at = ? WHERE id = ?")
        .bind(&title)
        .bind(&slug_)
        .bind(stored(&pick(&changes.lexical, &existing.lexical, "lexical")))
        .bind(stored(&pick(&changes.markdown, &existing.markdown, "markdown")))
        .bind(stored_html(&pick(&changes.html, &existing.html, "html")))
        .bind(pick(&changes.plaintext, &existing.plaintext, "plaintext"))
        .bind(pick(&changes.custom_excerpt, &existing.custom_excerpt, "custom_excerpt"))
        .bind(stored(&pick(&changes.feature_image, &existing.feature_image, "feature_image")))
        .bind(changes.featured.unwrap_or(existing.featured))
        .bind(&status)
        .bind(changes.visibility.clone().unwrap_or_else(|| existing.visibility.clone()))
        .bind(&published_at)
        .bind(pick(&changes.codeinjection_head, &existing.codeinjection_head, "codeinjection_head"))
        .bind(pick(&changes.codeinjection_foot, &existing.codeinjection_foot, "codeinjection_foot"))
        .bind(pick(&changes.custom_template, &existing.custom_template, "custom_template"))
        .bind(pick(&changes.canonical_url, &existing.canonical_url, "canonical_url"))
        .bind(changes.show_title_and_feature_image.unwrap_or(existing.show_title_and_feature_image))
        .bind(changes.reading_time.unwrap_or(existing.reading_time))
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;

    let old_meta = existing.meta.clone().unwrap_or_default();
    let meta_fields: [(&str, Option<String>, Option<String>); 9] = [
        ("meta_title", changes.meta_title.clone(), old_meta.meta_title.clone()),
        ("meta_description", changes.meta_description.clone(), old_meta.meta_description.clone()),
        ("og_title", changes.og_title.clone(), old_meta.og_title.clone()),
        ("og_description", changes.og_description.clone(), old_meta.og_description.clone()),
        ("og_image", changes.og_image.clone(), old_meta.og_image.clone()),
        ("twitter_title", changes.twitter_title.clone(), old_meta.twitter_title.clone()),
        ("twitter_description", changes.twitter_description.clone(), old_meta.twitter_description.clone()),
        ("twitter_image", changes.twitter_image.clone(), old_meta.twitter_image.clone()),
        ("email_subject", changes.email_subject.clone(), old_meta.email_subject.clone()),
    ];
    let touches_meta = meta_fields.iter().any(|(field, new, _)| new.is_some() || changes.clearing.contains(field))
        || changes.feature_image_alt.is_some()
        || changes.feature_image_caption.is_some()
        || changes.clearing.contains(&"feature_image_alt")
        || changes.clearing.contains(&"feature_image_caption");

    if touches_meta {
        let resolved: Vec<Option<String>> = meta_fields.iter().map(|(field, new, old)| pick(new, old, field)).collect();
        sqlx::query(
            "INSERT INTO posts_meta (id, post_id, meta_title, meta_description, og_title, og_description, og_image, twitter_title, twitter_description, twitter_image, email_subject, feature_image_alt, feature_image_caption) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(post_id) DO UPDATE SET meta_title = excluded.meta_title, meta_description = excluded.meta_description, og_title = excluded.og_title, og_description = excluded.og_description, og_image = excluded.og_image, twitter_title = excluded.twitter_title, twitter_description = excluded.twitter_description, twitter_image = excluded.twitter_image, email_subject = excluded.email_subject, feature_image_alt = excluded.feature_image_alt, feature_image_caption = excluded.feature_image_caption",
        )
        .bind(&ids::new_id())
        .bind(id)
        .bind(&resolved[0])
        .bind(&resolved[1])
        .bind(&resolved[2])
        .bind(&resolved[3])
        .bind(&resolved[4])
        .bind(&resolved[5])
        .bind(&resolved[6])
        .bind(&resolved[7])
        .bind(&resolved[8])
        .bind(pick(&changes.feature_image_alt, &old_meta.feature_image_alt, "feature_image_alt"))
        .bind(pick(&changes.feature_image_caption, &old_meta.feature_image_caption, "feature_image_caption"))
        .execute(pool)
        .await
        .map_err(dberr)?;
    }

    if let Some(tag_ids) = &changes.tag_ids {
        sqlx::query("DELETE FROM posts_tags WHERE post_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
        for (i, tid) in tag_ids.iter().enumerate() {
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, ?)")
                .bind(&ids::new_id())
                .bind(id)
                .bind(tid)
                .bind(i as i64)
                .execute(pool)
                .await
                .map_err(dberr)?;
        }
    }
    if let Some(author_ids) = &changes.author_ids {
        sqlx::query("DELETE FROM posts_authors WHERE post_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
        for (i, aid) in author_ids.iter().enumerate() {
            sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, ?)")
                .bind(&ids::new_id())
                .bind(id)
                .bind(aid)
                .bind(i as i64)
                .execute(pool)
                .await
                .map_err(dberr)?;
        }
    }

    find_by_id(pool, id).await?.ok_or_else(|| GhostError::Internal("post vanished after update".into()))
}

// ---------------------------------------------------------------------------
// scheduling
// ---------------------------------------------------------------------------

/// What should happen to a post when its scheduled time arrives.
///
/// A scheduled post is written now and delivered later, so the decision the
/// author made at writing time has to be stored rather than acted on. Ghost
/// keeps the same two facts in the same two columns: `email_recipient_filter`
/// on the post, and `email_only` on its meta row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Delivery {
    /// Email it to the newsletter's subscribers. The post's own visibility
    /// still decides which of them, at send time.
    pub email: bool,
    /// Deliver it to subscribers without listing it on the site — Ghost's
    /// `sent` status.
    pub email_only: bool,
}

impl Delivery {
    /// How it is stored on the post row. `all` is Ghost's filter for "every
    /// subscriber the post's visibility allows".
    fn filter(self) -> Option<&'static str> {
        self.email.then_some("all")
    }

    /// The status the post takes when it goes out.
    pub fn status(self) -> &'static str {
        if self.email_only {
            "sent"
        } else {
            "published"
        }
    }
}

/// Record what a scheduled post should do when its time comes.
pub async fn set_delivery(pool: &SqlitePool, post_id: &str, delivery: Delivery) -> Result<()> {
    sqlx::query("UPDATE posts SET email_recipient_filter = ? WHERE id = ?")
        .bind(delivery.filter())
        .bind(post_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    // `post_id` is unique, so this is an upsert: a post whose meta row does
    // not exist yet is the common case for one scheduled straight away.
    sqlx::query(
        "INSERT INTO posts_meta (id, post_id, email_only) VALUES (?, ?, ?) \
         ON CONFLICT(post_id) DO UPDATE SET email_only = excluded.email_only",
    )
    .bind(ids::new_id())
    .bind(post_id)
    .bind(delivery.email_only)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// What one post is set to do, so the editor can show the choice it is
/// holding rather than offering a fresh one and quietly dropping it.
pub async fn delivery(pool: &SqlitePool, post_id: &str) -> Delivery {
    let row: Option<(Option<String>, Option<bool>)> = sqlx::query_as(
        "SELECT p.email_recipient_filter, m.email_only FROM posts p \
         LEFT JOIN posts_meta m ON m.post_id = p.id WHERE p.id = ?",
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    match row {
        Some((filter, email_only)) => Delivery {
            email: filter.is_some_and(|f| !f.is_empty() && f != "none"),
            email_only: email_only.unwrap_or(false),
        },
        None => Delivery::default(),
    }
}

/// A scheduled post whose time has come.
#[derive(Debug, Clone)]
pub struct DuePost {
    pub id: String,
    pub delivery: Delivery,
}

/// Scheduled posts due at or before `now`, oldest first.
///
/// "At or before" rather than "since the last check" on purpose: anything
/// whose moment passed while the server was down is picked up by the first
/// run after it starts, which is the behaviour an author expects from a date
/// they set days ago.
pub async fn due_scheduled(pool: &SqlitePool, now: &str) -> Result<Vec<DuePost>> {
    let rows: Vec<(String, Option<String>, Option<bool>)> = sqlx::query_as(
        "SELECT p.id, p.email_recipient_filter, m.email_only \
         FROM posts p LEFT JOIN posts_meta m ON m.post_id = p.id \
         WHERE p.status = 'scheduled' AND p.published_at IS NOT NULL AND p.published_at <= ? \
         ORDER BY p.published_at",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;

    Ok(rows
        .into_iter()
        .map(|(id, filter, email_only)| DuePost {
            id,
            delivery: Delivery {
                // Any filter at all means "email this"; Ghost writes `none`
                // for a post that is not going out, and so do we.
                email: filter.is_some_and(|f| !f.is_empty() && f != "none"),
                email_only: email_only.unwrap_or(false),
            },
        })
        .collect())
}

pub async fn destroy(pool: &SqlitePool, id: &str) -> Result<()> {
    let res = sqlx::query("DELETE FROM posts WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    if res.rows_affected() == 0 {
        return Err(ghost_core::GhostError::NotFoundError { message: "not found".into(), context: None });
    }
    Ok(())
}

/// All published posts (posts + pages) for sitemap/RSS/url registries.
pub async fn all_published(pool: &SqlitePool) -> Result<Vec<Post>> {
    let rows = sqlx::query_as::<_, PostRow>(&format!(
        "SELECT {POST_COLS} FROM posts WHERE status = 'published' ORDER BY published_at DESC"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

