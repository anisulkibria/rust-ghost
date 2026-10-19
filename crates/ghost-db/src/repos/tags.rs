//! Tags repository.
use ghost_core::{dates, ids, models::Tag, slug, GhostError, Result};
use ghost_nql::Expr;
use sqlx::SqlitePool;
use std::collections::BTreeSet;

use super::{dberr, TagRow, TAG_COLS};

pub async fn find_by_ids(pool: &SqlitePool, tag_ids: &BTreeSet<String>) -> Result<Vec<Tag>> {
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
    pool: &SqlitePool,
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

    if include_count {
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

    if let Some(expr) = filter {
        // Filter in memory on serialized tags (tag filters are rare and small).
        tags.retain(|t| ghost_nql::evaluate(expr, &ghost_core::models::tag_to_json(t, None)));
    }

    let total = tags.len() as i64;
    let limit = if limit > 0 { limit } else { 15 };
    let start = ((page.max(1) - 1) * limit) as usize;
    let page_tags: Vec<Tag> = if limit < 0 { tags } else { tags.into_iter().skip(start).take(limit as usize).collect() };
    Ok((page_tags, total))
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

pub async fn find_by_slug(pool: &SqlitePool, slug_: &str) -> Result<Option<Tag>> {
    let tag = sqlx::query_as::<_, TagRow>(&format!("SELECT {TAG_COLS} FROM tags WHERE slug = ? LIMIT 1"))
        .bind(slug_)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .map(Into::into);
    Ok(tag)
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<Tag>> {
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
}

pub async fn slug_exists(pool: &SqlitePool, slug_: &str, self_id: Option<&str>) -> Result<bool> {
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

async fn unique_slug(pool: &SqlitePool, base: &str, self_id: Option<&str>) -> Result<String> {
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

pub async fn insert(pool: &SqlitePool, new: &NewTag) -> Result<Tag> {
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
    sqlx::query("INSERT INTO tags (id, name, slug, description, feature_image, visibility, accent_color, meta_title, meta_description, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(&slug_)
        .bind(&new.description)
        .bind(&new.feature_image)
        .bind(if visibility.is_empty() { "public" } else { visibility.as_str() })
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
}

pub async fn update(pool: &SqlitePool, id: &str, changes: &TagChanges) -> Result<Tag> {
    let existing = find_by_id(pool, id).await?.ok_or(ghost_core::GhostError::NotFoundError { message: "not found".into(), context: None })?;
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
         codeinjection_head = ?, codeinjection_foot = ?, updated_at = ? WHERE id = ?",
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
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    find_by_id(pool, id).await?.ok_or_else(|| GhostError::Internal("tag vanished".into()))
}

pub async fn destroy(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM posts_tags WHERE tag_id = ?").bind(id).execute(pool).await.map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    let res = sqlx::query("DELETE FROM tags WHERE id = ?").bind(id).execute(pool).await.map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(ghost_core::GhostError::NotFoundError { message: "not found".into(), context: None });
    }
    Ok(())
}

pub async fn all_public(pool: &SqlitePool) -> Result<Vec<Tag>> {
    let rows = sqlx::query_as::<_, TagRow>(&format!(
        "SELECT {TAG_COLS} FROM tags WHERE visibility = 'public' ORDER BY name"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
