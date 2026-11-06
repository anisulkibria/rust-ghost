//! Server-side member queries shared by the Admin API and member list.
use super::{dberr, posts::push_clause_with_binds};
use ghost_core::Result;
use serde::Serialize;
use crate::Db;
use sqlx::{Any, QueryBuilder};
#[derive(Serialize)]
pub struct Member {
    pub id: String,
    pub uuid: Option<String>,
    pub name: Option<String>,
    pub email: String,
    pub status: String,
    pub note: Option<String>,
    pub email_disabled: bool,
    pub email_count: i64,
    pub email_opened_count: i64,
    pub email_open_rate: Option<i64>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub last_seen_at: Option<String>,
}

/// Read by hand rather than derived, because of the boolean columns.
///
/// `#[derive(sqlx::FromRow)]` would decode each `bool` field with
/// `Decode<Any> for bool`, which fails on both engines: a boolean is stored
/// as a 0/1 integer and `Any` will not turn an integer into a `bool`. See
/// [`crate::types::flag`]. Everything else is read exactly as the derive
/// would have read it.
impl<'r> sqlx::FromRow<'r, sqlx::any::AnyRow> for Member {
    fn from_row(row: &'r sqlx::any::AnyRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            uuid: row.try_get("uuid")?,
            name: row.try_get("name")?,
            email: row.try_get("email")?,
            status: row.try_get("status")?,
            note: row.try_get("note")?,
            email_disabled: crate::types::flag(row, "email_disabled"),
            email_count: row.try_get("email_count")?,
            email_opened_count: row.try_get("email_opened_count")?,
            email_open_rate: row.try_get("email_open_rate")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            last_seen_at: row.try_get("last_seen_at")?,
        })
    }
}
const COLS:&str="id,uuid,name,email,status,note,email_disabled,email_count,email_opened_count,email_open_rate,created_at,updated_at,last_seen_at";
pub async fn browse(
    pool: &Db,
    filter: Option<&str>,
    limit: i64,
    page: i64,
) -> Result<(Vec<Member>, i64)> {
    let compiled = filter
        .filter(|s| !s.is_empty())
        .map(ghost_nql::parse)
        .transpose()?
        .map(|expr| ghost_nql::compile(&expr, "members"))
        .transpose()?;
    let mut count = QueryBuilder::<Any>::new("SELECT COUNT(*) FROM members");
    if let Some(sql) = &compiled {
        count.push(" WHERE ");
        push_clause_with_binds(&mut count, &sql.clause, &sql.binds);
    }
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .map_err(dberr)?;
    let mut query = QueryBuilder::<Any>::new(format!("SELECT {COLS} FROM members"));
    if let Some(sql) = compiled {
        query.push(" WHERE ");
        push_clause_with_binds(&mut query, &sql.clause, &sql.binds);
    }
    query.push(" ORDER BY created_at DESC,id DESC");
    if limit > 0 {
        query
            .push(" LIMIT ")
            .push_bind(limit.clamp(1, 1000))
            .push(" OFFSET ")
            .push_bind((page.max(1) - 1) * limit.clamp(1, 1000));
    }
    let rows = query
        .build_query_as()
        .fetch_all(pool)
        .await
        .map_err(dberr)?;
    Ok((rows, total))
}
pub async fn find(pool: &Db, id: &str) -> Result<Option<Member>> {
    sqlx::query_as(&format!("SELECT {COLS} FROM members WHERE id=?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}
