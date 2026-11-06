//! Historical article gift links and the single live link for each post.
use super::dberr;
use ghost_core::{dates, Result};
use sha2::{Digest, Sha256};
use crate::Db;

#[derive(Debug, sqlx::FromRow)]
pub struct GiftLink {
    pub token: String,
    pub created_at: String,
}

pub async fn read(pool: &Db, post: &str) -> Result<Option<GiftLink>> {
    sqlx::query_as("SELECT g.token,g.created_at FROM gift_links g JOIN post_gift_links p ON p.gift_link_token=g.token WHERE p.post_id=?")
        .bind(post).fetch_optional(pool).await.map_err(dberr)
}

/// Acquire the write lock before reading so concurrent ensures reuse one token.
pub async fn mint(
    pool: &Db,
    post: &str,
    token: &str,
    ensure: bool,
) -> Result<(GiftLink, bool)> {
    // `BEGIN IMMEDIATE` takes SQLite's write lock before the read, so two
    // concurrent ensures cannot both decide the post has no live link. MySQL
    // has no such transaction start; locking the post's own row is the
    // equivalent, because every mint names a post that exists — the children
    // rows may not exist yet, and InnoDB cannot lock an absent row.
    let mut tx = match crate::pool::engine() {
        ghost_core::database::Engine::Sqlite => {
            pool.begin_with("BEGIN IMMEDIATE").await.map_err(dberr)?
        }
        ghost_core::database::Engine::MySql => {
            let mut tx = pool.begin().await.map_err(dberr)?;
            sqlx::query("SELECT id FROM posts WHERE id=? FOR UPDATE")
                .bind(post)
                .execute(&mut *tx)
                .await
                .map_err(dberr)?;
            tx
        }
    };
    if ensure {
        let current = sqlx::query_as::<_,GiftLink>("SELECT g.token,g.created_at FROM gift_links g JOIN post_gift_links p ON p.gift_link_token=g.token WHERE p.post_id=?")
            .bind(post).fetch_optional(&mut *tx).await.map_err(dberr)?;
        if let Some(link) = current {
            tx.commit().await.map_err(dberr)?;
            return Ok((link, false));
        }
    }
    let created_at = dates::now_db();
    sqlx::query("INSERT INTO gift_links(token,post_id,created_at) VALUES(?,?,?)")
        .bind(token)
        .bind(post)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;
    sqlx::query(crate::dialect::sql("INSERT INTO post_gift_links(post_id,gift_link_token,created_at) VALUES(?,?,?) ON CONFLICT(post_id) DO UPDATE SET gift_link_token=excluded.gift_link_token,updated_at=excluded.created_at"))
        .bind(post).bind(token).bind(&created_at).execute(&mut *tx).await.map_err(dberr)?;
    sqlx::query("DELETE FROM article_gift_links WHERE post_id=?")
        .bind(post)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;
    tx.commit().await.map_err(dberr)?;
    Ok((
        GiftLink {
            token: token.into(),
            created_at,
        },
        true,
    ))
}

/// Revocation removes live associations, preserving history for audits/backups.
pub async fn revoke(pool: &Db, post: Option<&str>) -> Result<u64> {
    // Only writes, so the engine difference is the transaction start: MySQL
    // locks the rows it deletes without needing SQLite's eager write lock.
    let mut tx = match crate::pool::engine() {
        ghost_core::database::Engine::Sqlite => {
            pool.begin_with("BEGIN IMMEDIATE").await.map_err(dberr)?
        }
        ghost_core::database::Engine::MySql => pool.begin().await.map_err(dberr)?,
    };
    let current = sqlx::query("DELETE FROM post_gift_links WHERE (? IS NULL OR post_id=?)")
        .bind(post)
        .bind(post)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?
        .rows_affected();
    let legacy = sqlx::query("DELETE FROM article_gift_links WHERE (? IS NULL OR post_id=?)")
        .bind(post)
        .bind(post)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?
        .rows_affected();
    tx.commit().await.map_err(dberr)?;
    Ok(current + legacy)
}

pub async fn valid(pool: &Db, post: &str, token: &str) -> Result<bool> {
    match token.len() {
        // // `EXISTS` yields 0/1; `Any` does not decode an integer as a `bool`.
        32 => sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM post_gift_links WHERE post_id=? AND gift_link_token=?)",
        )
        .bind(post)
        .bind(token)
        .fetch_one(pool)
        .await
        .map(|n| n != 0)
        .map_err(dberr),
        // Only the previous native implementation generated these hashed tokens.
        64 => sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM article_gift_links WHERE post_id=? AND token_hash=?)",
        )
        .bind(post)
        .bind(hex::encode(Sha256::digest(token.as_bytes())))
        .fetch_one(pool)
        .await
        .map(|n| n != 0)
        .map_err(dberr),
        _ => Ok(false),
    }
}
