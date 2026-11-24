//! Tags repository.
use vuuut_core::{dates, ids, models::Tag, slug, GhostError, Result};
use vuuut_nql::Expr;
use crate::Db;
use std::collections::BTreeSet;

use super::{dberr, TagRow, TAG_COLS};

pub async fn find_by_ids(pool: &Db, tag_ids: &BTreeSet<String>) -> Result<Vec<Tag>> {
    let mut out = Vec::new();
    // SQLite has a practical variable limit; chunk large id sets
    for chunk in tag_ids.iter().collect::<Vec<_>>().chunks(100) {
        let mut q = format!("SELECT {TAG_COLS} FROM tags WHERE id IN (");
        q.push_str(&vec!["?"; chunk.len()].join(","));
        q.push_str(") ORDER BY name");
        let mut query = sqlx::query_as::<_, TagRow>(&q);
        for id in chunk {
            query = query.bind(*id);
        }
        let rows = query.fetch_all(pool).await.map_err(dberr)?;
        out.extend(rows.into_iter().map(Into::into));
    }
    Ok(out)
}

pub async fn browse(
    pool: &Db,
    filter: Option<&Expr>,
    order: Option<&str>,
    limit: i64,
    page: i64,
    public: bool,
    include_count: bool,
) -> Result<(Vec<Tag>, i64)> {
    let all = sqlx::query_as::<_, TagRow>(&format!(
        "SELECT {TAG_COLS} FROM tags {} ORDER BY {}",
        if public { "WHERE visibility = 'public'" } else { "" },
        order_clause(order),
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;

    let mut tags: Vec<Tag> = all.into_iter().map(Into::into).collect();

    // `order="count.posts desc"` is how themes list their biggest tags. The
    // count is not a column, so that order is applied here, after counting,
    // with the SQL order (name by default) kept as the tie-break.
    let by_count = count_order(order);

    if include_count || by_count.is_some() {
        for t in tags.iter_mut() {
            t.count_posts = Some(
                sqlx::query_scalar(
                    "SELECT COUNT(*) FROM posts_tags pt JOIN posts p ON p.id = pt.post_id WHERE pt.tag_id = ? AND p.status = 'published'",
                )
                .bind(&t.id)
                .fetch_one(pool)
                .await
                .map_err(dberr)?,
            );
        }
    }

    if let Some(desc) = by_count {
        tags.sort_by(|a, b| {
            let (x, y) = (a.count_posts.unwrap_or(0), b.count_posts.unwrap_or(0));
            if desc { y.cmp(&x) } else { x.cmp(&y) }
        });
        if !include_count {
            for t in tags.iter_mut() {
                t.count_posts = None;
            }
        }
    }

    if let Some(expr) = filter {
        // Filter in memory on serialized tags (tag filters are rare and small).
        tags.retain(|t| vuuut_nql::evaluate(expr, &vuuut_core::models::tag_to_json(t, None)));
    }

    let total = tags.len() as i64;
    let limit = if limit > 0 { limit } else { 15 };
    let start = ((page.max(1) - 1) * limit) as usize;
    let page_tags: Vec<Tag> = if limit < 0 { tags } else { tags.into_iter().skip(start).take(limit as usize).collect() };
    Ok((page_tags, total))
}


/// `Some(true)` for `count.posts desc`, `Some(false)` for ascending, when
/// the post count is the first sort key.
fn count_order(order: Option<&str>) -> Option<bool> {
    let first = order?.split(',').next()?.trim().to_ascii_lowercase();
    let mut tokens = first.split_whitespace();
    if tokens.next()? != "count.posts" {
        return None;
    }
    Some(tokens.next().is_some_and(|d| d == "desc"))
}

fn order_clause(order: Option<&str>) -> String {
    let allowed = ["name", "slug", "created_at", "updated_at", "id"];
    let default = "name ASC, id ASC".to_string();
    let Some(order) = order else { return default };
    let mut parts = Vec::new();
    for part in order.split(',') {
        let mut tokens = part.trim().split_whitespace();
        let (Some(col), dir) = (tokens.next(), tokens.next().unwrap_or("ASC")) else {
            continue;
        };
        if !allowed.contains(&col) {
            continue;
        }
        let dir = if dir.eq_ignore_ascii_case("desc") { "DESC" } else { "ASC" };
        parts.push(format!("tags.{col} {dir}"));
    }
    if parts.is_empty() { default } else { parts.join(", ") }
}

pub async fn find_by_slug(pool: &Db, slug_: &str) -> Result<Option<Tag>> {
    let tag = sqlx::query_as::<_, TagRow>(&format!("SELECT {TAG_COLS} FROM tags WHERE slug = ? LIMIT 1"))
        .bind(slug_)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .map(Into::into);
    Ok(tag)
}

pub async fn find_by_id(pool: &Db, id: &str) -> Result<Option<Tag>> {
    let tag = sqlx::query_as::<_, TagRow>(&format!("SELECT {TAG_COLS} FROM tags WHERE id = ? LIMIT 1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .map(Into::into);
    Ok(tag)
}

#[derive(Debug, Default, Clone)]
pub struct NewTag {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub feature_image: Option<String>,
    pub visibility: String,
    pub accent_color: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    /// The language this tag belongs to. `None` — the default — means every
    /// language, which is what an existing site's tags all are.
    pub locale: Option<String>,
}

pub async fn slug_exists(pool: &Db, slug_: &str, self_id: Option<&str>) -> Result<bool> {
    let count: i64 = match self_id {
        Some(id) => sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE slug = ? AND id != ?")
            .bind(slug_)
            .bind(id)
            .fetch_one(pool)
            .await,
        None => sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE slug = ?")
            .bind(slug_)
            .fetch_one(pool)
            .await,
    }
    .map_err(dberr)?;
    Ok(count > 0)
}

async fn unique_slug(pool: &Db, base: &str, self_id: Option<&str>) -> Result<String> {
    let mut s = slug::safe_string(base);
    if s.len() > 185 {
        s.truncate(185);
        s = s.trim_matches('-').to_string();
    }
    if s.is_empty() {
        s = "tag-1".into();
    }
    for p in ["ghost", "rss", "amp", "new"] {
        if s == p {
            s = format!("{s}-tag");
        }
    }
    let mut candidate = s.clone();
    let mut i = 1;
    while slug_exists(pool, &candidate, self_id).await? {
        i += 1;
        candidate = format!("{s}-{i}");
    }
    Ok(candidate)
}

pub async fn insert(pool: &Db, new: &NewTag) -> Result<Tag> {
    crate::content::touch();
    // Internal tag: name starting with '#'
    let (name, visibility, slug_base) = if let Some(rest) = new.name.strip_prefix('#') {
        (format!("#{rest}"), "internal".to_string(), format!("hash-{rest}"))
    } else {
        (new.name.clone(), new.visibility.clone(), new.name.clone())
    };
    let slug_ = match &new.slug {
        Some(s) if !s.trim().is_empty() => slug::safe_string(s),
        _ => unique_slug(pool, &slug_base, None).await?,
    };
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query("INSERT INTO tags (id, name, slug, description, feature_image, visibility, locale, accent_color, meta_title, meta_description, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(&slug_)
        .bind(&new.description)
        .bind(&new.feature_image)
        .bind(if visibility.is_empty() { "public" } else { visibility.as_str() })
        .bind(&new.locale)
        .bind(&new.accent_color)
        .bind(&new.meta_title)
        .bind(&new.meta_description)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(dberr)?;
    find_by_id(pool, &id).await?.ok_or_else(|| GhostError::Internal("tag vanished".into()))
}

/// Every field Ghost's tag editor can change. `None` leaves the stored value
/// alone; `Some("")` clears it, which is how a cleared form field is meant to
/// read.
#[derive(Debug, Default, Clone)]
pub struct TagChanges {
    pub name: Option<String>,
    /// Re-slugging changes the tag's public URL, so it is only ever done when
    /// the editor explicitly asks.
    pub slug: Option<String>,
    pub description: Option<String>,
    pub feature_image: Option<String>,
    pub visibility: Option<String>,
    pub accent_color: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub canonical_url: Option<String>,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub twitter_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    /// `Some("")` clears it back to "every language", which is how the
    /// picker's blank option is meant to read.
    pub locale: Option<String>,
}

pub async fn update(pool: &Db, id: &str, changes: &TagChanges) -> Result<Tag> {
    crate::content::touch();
    let existing = find_by_id(pool, id).await?.ok_or(vuuut_core::GhostError::NotFoundError { message: "not found".into(), context: None })?;
    let now = dates::now_db();

    // A leading '#' is how Ghost marks a tag internal, so renaming into or
    // out of that form has to move `visibility` with it — otherwise a tag
    // called "#internal" would keep showing up on the site.
    let name = changes.name.clone().unwrap_or(existing.name.clone().unwrap_or_default());
    let visibility = match &changes.visibility {
        Some(v) if !v.is_empty() => v.clone(),
        _ if name.starts_with('#') => "internal".to_string(),
        _ if existing.visibility == "internal" => "public".to_string(),
        _ => existing.visibility.clone(),
    };

    // Slugs must stay unique; an explicit one that collides gets a numeric
    // suffix rather than failing the save.
    let slug_ = match changes.slug.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(requested) => {
            let cleaned = slug::safe_string(requested);
            let current = existing.slug.clone().unwrap_or_default();
            if cleaned == current {
                current
            } else {
                unique_slug(pool, &cleaned, Some(id)).await?
            }
        }
        None => existing.slug.clone().unwrap_or_default(),
    };

    sqlx::query(
        "UPDATE tags SET name = ?, slug = ?, description = ?, feature_image = ?, visibility = ?, \
         accent_color = ?, meta_title = ?, meta_description = ?, canonical_url = ?, \
         og_image = ?, og_title = ?, og_description = ?, \
         twitter_image = ?, twitter_title = ?, twitter_description = ?, \
         codeinjection_head = ?, codeinjection_foot = ?, locale = ?, updated_at = ? WHERE id = ?",
    )
        .bind(&name)
        .bind(&slug_)
        .bind(changes.description.clone().or(existing.description.clone()))
        .bind(changes.feature_image.clone().or(existing.feature_image.clone()))
        .bind(&visibility)
        .bind(changes.accent_color.clone().or(existing.accent_color.clone()))
        .bind(changes.meta_title.clone().or(existing.meta_title.clone()))
        .bind(changes.meta_description.clone().or(existing.meta_description.clone()))
        .bind(changes.canonical_url.clone().or(existing.canonical_url.clone()))
        .bind(changes.og_image.clone().or(existing.og_image.clone()))
        .bind(changes.og_title.clone().or(existing.og_title.clone()))
        .bind(changes.og_description.clone().or(existing.og_description.clone()))
        .bind(changes.twitter_image.clone().or(existing.twitter_image.clone()))
        .bind(changes.twitter_title.clone().or(existing.twitter_title.clone()))
        .bind(changes.twitter_description.clone().or(existing.twitter_description.clone()))
        .bind(changes.codeinjection_head.clone().or(existing.codeinjection_head.clone()))
        .bind(changes.codeinjection_foot.clone().or(existing.codeinjection_foot.clone()))
        // An empty string is an erasure here, not "unchanged": it is the
        // blank "every language" option in the picker.
        .bind(match changes.locale.as_deref() {
            Some("") => None,
            Some(code) => Some(code.to_string()),
            None => existing.locale.clone(),
        })
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    find_by_id(pool, id).await?.ok_or_else(|| GhostError::Internal("tag vanished".into()))
}

pub async fn destroy(pool: &Db, id: &str) -> Result<()> {
    crate::content::touch();
    sqlx::query("DELETE FROM posts_tags WHERE tag_id = ?").bind(id).execute(pool).await.map_err(|e| vuuut_core::GhostError::Db(e.to_string()))?;
    let res = sqlx::query("DELETE FROM tags WHERE id = ?").bind(id).execute(pool).await.map_err(|e| vuuut_core::GhostError::Db(e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(vuuut_core::GhostError::NotFoundError { message: "not found".into(), context: None });
    }
    Ok(())
}

pub async fn all_public(pool: &Db) -> Result<Vec<Tag>> {
    let rows = sqlx::query_as::<_, TagRow>(&format!(
        "SELECT {TAG_COLS} FROM tags WHERE visibility = 'public' ORDER BY name"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Staff tag pickers include internal tags; public theme queries use all_public.
pub async fn all(pool: &Db) -> Result<Vec<Tag>> {
    let rows = sqlx::query_as::<_, TagRow>(&crate::dialect::sql_owned(&format!(
        "SELECT {TAG_COLS} FROM tags ORDER BY name COLLATE NOCASE"
    )))
        .fetch_all(pool).await.map_err(dberr)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_count_order_is_recognised_only_as_the_first_key() {
        assert_eq!(count_order(Some("count.posts desc")), Some(true));
        assert_eq!(count_order(Some("count.posts DESC, name asc")), Some(true));
        assert_eq!(count_order(Some("count.posts")), Some(false));
        assert_eq!(count_order(Some("name asc, count.posts desc")), None);
        assert_eq!(count_order(None), None);
    }

    #[tokio::test]
    async fn tags_order_by_published_post_count() {
        let test_db = crate::pool::connect_test().await.unwrap();
        let pool = test_db.pool();
        let mut ids = Vec::new();
        for name in ["Alpha", "Beta", "Gamma"] {
            ids.push(insert(&pool, &NewTag { name: name.into(), ..Default::default() }).await.unwrap().id);
        }
        // Gamma gets two published posts, Beta one, Alpha none.
        for (n, tag) in [(1, &ids[2]), (2, &ids[2]), (3, &ids[1])] {
            let post = format!("p{n}");
            sqlx::query(
                "INSERT INTO posts (id, uuid, title, slug, status, type, visibility, created_at) \
                 VALUES (?, ?, ?, ?, 'published', 'post', 'public', '2026-01-01 00:00:00')",
            )
            .bind(&post).bind(&post).bind(&post).bind(&post)
            .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, 0)")
                .bind(format!("pt{n}")).bind(&post).bind(tag)
                .execute(&pool).await.unwrap();
        }
        let (tags, _) = browse(&pool, None, Some("count.posts desc"), 15, 1, true, false).await.unwrap();
        let names: Vec<_> = tags.iter().map(|t| t.name.clone().unwrap_or_default()).collect();
        assert_eq!(names, ["Gamma", "Beta", "Alpha"]);
        assert!(tags.iter().all(|t| t.count_posts.is_none()), "counts stay out unless included");
    }

    #[tokio::test]
    async fn staff_picker_includes_internal_tags_while_public_queries_exclude_them() {
        let test_db = crate::pool::connect_test().await.unwrap();
        let pool = test_db.pool();
        for name in ["News", "#Internal"] {
            insert(&pool, &NewTag { name: name.into(), ..Default::default() }).await.unwrap();
        }
        let staff = all(&pool).await.unwrap();
        assert_eq!(staff.len(), 2);
        assert!(staff.iter().any(|t| t.name.as_deref() == Some("#Internal") && t.visibility == "internal"));
        let public = all_public(&pool).await.unwrap();
        assert_eq!(public.len(), 1);
        assert_eq!(public[0].name.as_deref(), Some("News"));
    }

    /// The visibility rule `update` applies, lifted out so it can be checked
    /// without a database.
    fn resolved_visibility(name: &str, explicit: Option<&str>, existing: &str) -> String {
        match explicit {
            Some(v) if !v.is_empty() => v.to_string(),
            _ if name.starts_with('#') => "internal".to_string(),
            _ if existing == "internal" => "public".to_string(),
            _ => existing.to_string(),
        }
    }

    #[test]
    fn a_hash_name_makes_a_tag_internal_and_renaming_back_makes_it_public() {
        // Ghost treats the leading '#' as the switch, so visibility has to
        // follow the name or an "#internal" tag would still show on the site
        assert_eq!(resolved_visibility("#private", None, "public"), "internal");
        assert_eq!(resolved_visibility("Public again", None, "internal"), "public");
        // a name with no '#' on an already-public tag is left alone
        assert_eq!(resolved_visibility("News", None, "public"), "public");
        // an explicit value always wins, which is what the Admin API relies on
        assert_eq!(resolved_visibility("#private", Some("public"), "public"), "public");
    }
}
