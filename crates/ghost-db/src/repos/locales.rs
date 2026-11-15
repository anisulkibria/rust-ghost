//! Translations of an entry, and what each language has.
//!
//! Every query the Multilingual module runs. Two jobs:
//!
//! * **Translation groups** — the answer to "what is this post called in
//!   German", which is the feature the editor's language switcher is built
//!   on. A group is just a shared `posts.translation_group`; see
//!   `migrations/*/0014_multilingual.sql` for why that is a column rather
//!   than a join table.
//! * **Coverage** — how much of the site exists in each language, and which
//!   entries are missing a translation. That is the report on
//!   `/ghost/languages/`, and it is computed on demand from `posts` rather
//!   than stored, like the SEO report.
//!
//! ## The invariant every write here keeps
//!
//! **One language per group.** Two German posts in one group would make
//! "the German version of this" ambiguous, and the editor's switcher would
//! have to pick one arbitrarily. [`link`] refuses rather than choosing, and
//! the uniqueness is checked in the same transaction as the write so two
//! simultaneous links cannot both pass the check and then both commit.
use super::{dberr, POST_COLS};
use crate::Db;
use ghost_core::models::Post;
use ghost_core::{ids, Result};

/// One sibling in a translation group, as the editor's panel lists it.
#[derive(Debug, Clone)]
pub struct Sibling {
    pub id: String,
    pub title: Option<String>,
    pub slug: Option<String>,
    /// NULL in the row means the site's default language; left as-is here
    /// because only the caller knows what the default is.
    pub locale: Option<String>,
    pub status: String,
    pub post_type: String,
    pub updated_at: Option<String>,
}

/// What one language has, for the module's overview.
#[derive(Debug, Clone, Default)]
pub struct Coverage {
    pub posts: i64,
    pub pages: i64,
    pub drafts: i64,
    pub tags: i64,
}

/// Every entry in the same translation group as `post_id`, excluding it.
///
/// Empty for an entry with no group, which is every entry on a site that has
/// never linked a translation — so the editor's panel is empty rather than
/// wrong.
pub async fn siblings(db: &Db, post_id: &str) -> Result<Vec<Sibling>> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, String, String, Option<String>)>(
        "SELECT p.id, p.title, p.slug, p.locale, p.status, p.type, p.updated_at \
         FROM posts p \
         WHERE p.translation_group IS NOT NULL \
           AND p.translation_group = (SELECT translation_group FROM posts WHERE id = ?) \
           AND p.id != ? \
         ORDER BY p.locale",
    )
    .bind(post_id)
    .bind(post_id)
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(id, title, slug, locale, status, post_type, updated_at)| Sibling {
            id,
            title,
            slug,
            locale,
            status,
            post_type,
            updated_at,
        })
        .collect())
}

/// The entry in `group` written in `locale`, if there is one.
///
/// `locale` is matched against both the stored code and NULL when it is the
/// site's default, because the default language is stored as NULL — see the
/// migration for why it is never backfilled. `default_locale` is passed in
/// rather than read from settings so this stays a pure query.
pub async fn in_locale(
    db: &Db,
    group: &str,
    locale: &str,
    default_locale: &str,
) -> Result<Option<Post>> {
    let sql = if locale.eq_ignore_ascii_case(default_locale) {
        format!(
            "SELECT {POST_COLS} FROM posts \
             WHERE translation_group = ? AND (locale IS NULL OR locale = ?) LIMIT 1"
        )
    } else {
        format!("SELECT {POST_COLS} FROM posts WHERE translation_group = ? AND locale = ? LIMIT 1")
    };
    let row = sqlx::query_as::<_, super::PostRow>(&sql)
        .bind(group)
        .bind(locale)
        .fetch_optional(db)
        .await
        .map_err(dberr)?;
    Ok(row.map(Post::from))
}

/// Why a link was refused, so the admin can say which it was.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkRefusal {
    /// One of the two ids is not a post.
    Missing,
    /// Linking something to itself.
    Same,
    /// Both entries are in the same language, so "the German one" would be
    /// ambiguous.
    SameLocale(String),
    /// The target already has a translation in this entry's language.
    Occupied(String),
    /// A post and a page are different kinds of thing.
    KindMismatch,
}

/// Put two entries in one translation group.
///
/// Both sides keep their own language; what changes is that each becomes
/// reachable from the other. If either already belongs to a group, the two
/// groups are merged into the existing one rather than a new id being minted,
/// so linking A→B and then B→C leaves one group of three rather than two
/// groups that disagree.
///
/// Refuses rather than guessing whenever the result would be ambiguous: see
/// [`LinkRefusal`] and the invariant in the module docs.
pub async fn link(
    db: &Db,
    a_id: &str,
    b_id: &str,
    default_locale: &str,
) -> Result<std::result::Result<String, LinkRefusal>> {
    if a_id == b_id {
        return Ok(Err(LinkRefusal::Same));
    }
    // The whole decision is made inside one transaction, so two links
    // running at once cannot both find the slot free and both take it.
    let mut tx = db.begin().await.map_err(dberr)?;

    let member = "SELECT locale, type, translation_group FROM posts WHERE id = ?";
    let a = sqlx::query_as::<_, (Option<String>, String, Option<String>)>(member)
        .bind(a_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr)?;
    let b = sqlx::query_as::<_, (Option<String>, String, Option<String>)>(member)
        .bind(b_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr)?;
    let (Some((a_locale, a_type, a_group)), Some((b_locale, b_type, b_group))) = (a, b) else {
        return Ok(Err(LinkRefusal::Missing));
    };
    if a_type != b_type {
        return Ok(Err(LinkRefusal::KindMismatch));
    }
    // A NULL locale and the default code are the same language.
    let settled = |stored: &Option<String>| {
        stored.as_deref().map(str::to_string).unwrap_or_else(|| default_locale.to_string())
    };
    let a_code = settled(&a_locale);
    let b_code = settled(&b_locale);
    if a_code.eq_ignore_ascii_case(&b_code) {
        return Ok(Err(LinkRefusal::SameLocale(a_code)));
    }

    // Reuse whichever group already exists, so linking A→B and then B→C
    // converges on one group of three rather than splitting the set.
    let group = a_group.clone().or_else(|| b_group.clone()).unwrap_or_else(ids::new_id);

    // Would the merged group hold two of any one language? Every member of
    // both sets counts — the two entries being linked *and* everything
    // already linked to either — because linking merges the sets whole: an
    // English post with a French translation linked to a Bengali post that
    // also has one would otherwise make a group with two French entries.
    // Read out and decided in Rust: as one SQL predicate it was unreadable,
    // and this is the invariant the whole module rests on.
    let mut members: Vec<(String, String)> = vec![(a_id.to_string(), a_code.clone()), (b_id.to_string(), b_code.clone())];
    for existing in [a_group.as_deref(), b_group.as_deref()].into_iter().flatten() {
        let rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT id, locale FROM posts WHERE translation_group = ?",
        )
        .bind(existing)
        .fetch_all(&mut *tx)
        .await
        .map_err(dberr)?;
        for (id, locale) in rows {
            if !members.iter().any(|(m, _)| *m == id) {
                members.push((id, settled(&locale)));
            }
        }
    }
    for (i, (_, code)) in members.iter().enumerate() {
        if members[..i].iter().any(|(_, earlier)| earlier.eq_ignore_ascii_case(code)) {
            return Ok(Err(LinkRefusal::Occupied(code.clone())));
        }
    }

    // Move both sides, and anything already grouped with either, onto one id.
    sqlx::query(
        "UPDATE posts SET translation_group = ? \
         WHERE id IN (?, ?) \
            OR (translation_group IS NOT NULL AND translation_group IN (?, ?))",
    )
    .bind(&group)
    .bind(a_id)
    .bind(b_id)
    .bind(a_group.as_deref().unwrap_or(""))
    .bind(b_group.as_deref().unwrap_or(""))
    .execute(&mut *tx)
    .await
    .map_err(dberr)?;

    tx.commit().await.map_err(dberr)?;
    Ok(Ok(group))
}

/// Take one entry out of its translation group.
///
/// The others keep the group. A group left holding a single entry is
/// harmless — it reads as "no translations" everywhere — so it is not tidied
/// up, which keeps this a one-statement write that cannot half-succeed.
pub async fn unlink(db: &Db, post_id: &str) -> Result<()> {
    sqlx::query("UPDATE posts SET translation_group = NULL WHERE id = ?")
        .bind(post_id)
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// The languages that use a tag, with how many published posts each has
/// with it — which is which language archives of that tag have anything on
/// them. Keyed by the code as the site knows it (NULL read as the default).
pub async fn tag_usage(db: &Db, tag_id: &str, default_locale: &str) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT COALESCE(NULLIF(p.locale, ''), ?) AS code, COUNT(*) \
         FROM posts p JOIN posts_tags pt ON pt.post_id = p.id \
         WHERE pt.tag_id = ? AND p.status = 'published' AND p.type = 'post' \
         GROUP BY COALESCE(NULLIF(p.locale, ''), ?)",
    )
    .bind(default_locale)
    .bind(tag_id)
    .bind(default_locale)
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    Ok(rows)
}

/// Write `code` into every entry that has no language of its own.
///
/// A NULL locale means "the site's default language" (see the migration),
/// which is right until the default changes. On a multilingual site that
/// change must not relabel every existing entry as the new language — the
/// English posts would suddenly claim to be Bengali, and sit in the same
/// translation group as the real Bengali ones. So the old default is
/// written in, once, at the moment it stops being the default.
///
/// Returns how many entries were stamped.
pub async fn stamp_default(db: &Db, code: &str) -> Result<u64> {
    let done = sqlx::query("UPDATE posts SET locale = ? WHERE locale IS NULL OR locale = ''")
        .bind(code)
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected())
}

/// The group `post_id` belongs to, minting one if it has none.
///
/// Used when creating a translation: the new entry needs a group to join,
/// and the original may not have one yet.
pub async fn group_for(db: &Db, post_id: &str) -> Result<String> {
    let existing: Option<Option<String>> =
        sqlx::query_scalar("SELECT translation_group FROM posts WHERE id = ?")
            .bind(post_id)
            .fetch_optional(db)
            .await
            .map_err(dberr)?;
    if let Some(Some(group)) = existing.clone() {
        return Ok(group);
    }
    let group = ids::new_id();
    sqlx::query("UPDATE posts SET translation_group = ? WHERE id = ?")
        .bind(&group)
        .bind(post_id)
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(group)
}

/// What each language has, keyed by the code as stored (the default language
/// reported under `default_locale` rather than as an empty key).
pub async fn coverage(db: &Db, default_locale: &str) -> Result<Vec<(String, Coverage)>> {
    let rows = sqlx::query_as::<_, (Option<String>, String, String, i64)>(
        "SELECT locale, type, status, COUNT(*) FROM posts \
         WHERE type IN ('post', 'page') GROUP BY locale, type, status",
    )
    .fetch_all(db)
    .await
    .map_err(dberr)?;

    let mut out: std::collections::BTreeMap<String, Coverage> = std::collections::BTreeMap::new();
    for (locale, kind, status, count) in rows {
        let code = locale.unwrap_or_else(|| default_locale.to_string());
        let entry = out.entry(code).or_default();
        let published = status == "published" || status == "sent";
        match (kind.as_str(), published) {
            ("page", true) => entry.pages += count,
            ("post", true) => entry.posts += count,
            _ => entry.drafts += count,
        }
    }

    let tag_rows = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT locale, COUNT(*) FROM tags WHERE visibility = 'public' GROUP BY locale",
    )
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    for (locale, count) in tag_rows {
        // A tag with no locale belongs to every language, so it is not
        // counted against any one of them.
        if let Some(code) = locale {
            out.entry(code).or_default().tags += count;
        }
    }
    Ok(out.into_iter().collect())
}

/// Published entries in the default language with no translation in
/// `locale` — the module's "what is left to translate" list.
///
/// Only published entries, and only ones in the default language: a draft is
/// not yet missing anything, and a German post with no English version is a
/// different question from the one this answers.
pub async fn untranslated(
    db: &Db,
    locale: &str,
    default_locale: &str,
    limit: i64,
) -> Result<Vec<Sibling>> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, String, String, Option<String>)>(
        "SELECT p.id, p.title, p.slug, p.locale, p.status, p.type, p.updated_at \
         FROM posts p \
         WHERE p.type IN ('post', 'page') \
           AND p.status IN ('published', 'sent') \
           AND (p.locale IS NULL OR p.locale = ?) \
           AND NOT EXISTS ( \
             SELECT 1 FROM posts t \
             WHERE t.translation_group IS NOT NULL \
               AND t.translation_group = p.translation_group \
               AND t.locale = ? \
           ) \
         ORDER BY COALESCE(p.published_at, p.updated_at, p.created_at) DESC LIMIT ?",
    )
    .bind(default_locale)
    .bind(locale)
    .bind(limit.clamp(1, 500))
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(id, title, slug, locale, status, post_type, updated_at)| Sibling {
            id,
            title,
            slug,
            locale,
            status,
            post_type,
            updated_at,
        })
        .collect())
}

/// Entries that could become a translation of `post_id`.
///
/// Only ones that [`link`] would actually accept: a different language, the
/// same kind, and not the entry itself. Filtering here rather than in the
/// browser is what keeps the editor's search honest — offering a result that
/// would then be refused is worse than not offering it.
///
/// `term` matches the title or the slug, case-insensitively, anywhere in the
/// value. Empty lists the most recent candidates, which is what an empty
/// search box should show rather than nothing.
pub async fn candidates(
    db: &Db,
    post_id: &str,
    post_type: &str,
    own_locale: &str,
    default_locale: &str,
    term: &str,
    limit: i64,
) -> Result<Vec<Sibling>> {
    // Bounded and bound, never spliced: the term comes straight from a
    // keystroke. `%` and `_` are escaped so a typed wildcard searches for
    // itself rather than matching everything.
    let term: String = term.chars().take(200).collect();
    let pattern = format!(
        "%{}%",
        term.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
    );
    // Only entries [`link`] would accept, so it is the same from whichever
    // side somebody starts. "Mine" is this entry and everything already
    // linked to it; a candidate qualifies when neither it nor anything
    // linked to *it* is in a language mine already has. That excludes
    // what is already linked (its language is taken by definition) and
    // anything whose own set would collide on merging.
    //
    // A NULL locale is the default language, so every comparison goes
    // through COALESCE to the default code.
    let _ = own_locale;
    let mine = "SELECT COALESCE(NULLIF(m.locale, ''), ?) FROM posts m \
                WHERE m.id = ? OR (m.translation_group IS NOT NULL \
                  AND m.translation_group = (SELECT g.translation_group FROM posts g WHERE g.id = ?))";
    let sql = format!(
        "SELECT p.id, p.title, p.slug, p.locale, p.status, p.type, p.updated_at \
         FROM posts p \
         WHERE p.type = ? AND p.id != ? \
           AND COALESCE(NULLIF(p.locale, ''), ?) NOT IN ({mine}) \
           AND NOT EXISTS (SELECT 1 FROM posts q \
                 WHERE p.translation_group IS NOT NULL AND q.translation_group = p.translation_group \
                   AND COALESCE(NULLIF(q.locale, ''), ?) IN ({mine})) \
           AND (LOWER(COALESCE(p.title, '')) LIKE LOWER(?) ESCAPE '\\' \
             OR LOWER(COALESCE(p.slug, '')) LIKE LOWER(?) ESCAPE '\\') \
         ORDER BY COALESCE(p.published_at, p.updated_at, p.created_at) DESC LIMIT ?"
    );
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, String, String, Option<String>)>(&sql)
        .bind(post_type)
        .bind(post_id)
        .bind(default_locale)
        .bind(default_locale)
        .bind(post_id)
        .bind(post_id)
        .bind(default_locale)
        .bind(default_locale)
        .bind(post_id)
        .bind(post_id)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit.clamp(1, 100))
        .fetch_all(db)
        .await
        .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(id, title, slug, locale, status, post_type, updated_at)| Sibling {
            id,
            title,
            slug,
            locale,
            status,
            post_type,
            updated_at,
        })
        .collect())
}

/// Slugs that a language prefix has made unreachable.
///
/// A post slugged `de` on a German-publishing site cannot be opened: `/de/`
/// is read as the German home page first. Reported so the module can say
/// which entries are affected rather than leaving them to be noticed as a
/// mystery 404.
pub async fn shadowed_slugs(db: &Db, locales: &ghost_core::locales::Locales) -> Result<Vec<Sibling>> {
    if !locales.multilingual() {
        return Ok(Vec::new());
    }
    let reserved: Vec<String> = locales
        .enabled()
        .iter()
        .filter(|code| !locales.is_default(code))
        .map(|code| code.to_ascii_lowercase())
        .collect();
    if reserved.is_empty() {
        return Ok(Vec::new());
    }
    // Built with one placeholder per code rather than interpolated, so a
    // locale string can never reach the SQL text itself.
    let holes = reserved.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT p.id, p.title, p.slug, p.locale, p.status, p.type, p.updated_at \
         FROM posts p WHERE p.type IN ('post', 'page') AND LOWER(p.slug) IN ({holes})"
    );
    let mut query = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, String, String, Option<String>)>(&sql);
    for code in &reserved {
        query = query.bind(code);
    }
    let rows = query.fetch_all(db).await.map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(id, title, slug, locale, status, post_type, updated_at)| Sibling {
            id,
            title,
            slug,
            locale,
            status,
            post_type,
            updated_at,
        })
        .collect())
}
