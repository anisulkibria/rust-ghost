//! Forms and the responses sent through them.
//!
//! Every query binds its values; nothing from a request is ever spliced into
//! SQL. The two JSON columns (`forms.fields`, `form_submissions.data`) are
//! `MEDIUMTEXT` on MySQL and so are read through [`crate::types::Text`].
use super::dberr;
use crate::types::Text;
use crate::Db;
use ghost_core::forms::{Answer, Field};
use ghost_core::{dates, ids, Result};

#[derive(Debug, Clone)]
pub struct Form {
    pub id: String,
    pub name: String,
    /// `active` or `closed`.
    pub status: String,
    pub fields: Vec<Field>,
    pub submit_label: String,
    pub success_message: String,
    /// Comma-separated; empty means "use the site-wide list".
    pub notify_emails: String,
    /// `message`, `link` or `redirect`. See `ghost_core::forms::SuccessAction`.
    pub success_action: String,
    /// Where `link` and `redirect` point. Checked when it was saved.
    pub success_url: String,
    pub success_link_label: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Form {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }
}

/// A form with its response counts, for the module's list.
#[derive(Debug, Clone)]
pub struct FormWithCounts {
    pub form: Form,
    pub total: i64,
    pub unread: i64,
}

#[derive(Debug, Clone)]
pub struct Submission {
    pub id: String,
    pub form_id: String,
    pub form_name: String,
    pub answers: Vec<Answer>,
    /// `new`, `read` or `spam`.
    pub status: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub page_path: Option<String>,
    pub member_id: Option<String>,
    pub created_at: String,
}

/// Site-wide forms settings.
#[derive(Debug, Clone)]
pub struct Config {
    pub notify_enabled: bool,
    pub notify_emails: String,
    pub store_ip: bool,
    /// Days a response is kept; 0 keeps them until deleted by hand.
    pub retention_days: i64,
}

type FormRow = (String, String, String, Text, String, String, String, String, String, String, String, String);

const FORM_COLUMNS: &str = "id, name, status, fields, submit_label, success_message, notify_emails, \
     success_action, success_url, success_link_label, created_at, updated_at";

fn to_form(
    (id, name, status, fields, submit_label, success_message, notify_emails, success_action, success_url, success_link_label, created_at, updated_at): FormRow,
) -> Form {
    Form {
        id,
        name,
        status,
        fields: ghost_core::forms::parse_fields(&fields),
        submit_label,
        success_message,
        notify_emails,
        success_action,
        success_url,
        success_link_label,
        created_at,
        updated_at,
    }
}

pub async fn find(pool: &Db, id: &str) -> Result<Option<Form>> {
    // An id that is not an id cannot match a row; answering here keeps
    // arbitrary path segments away from the database altogether.
    if !ids::is_objectid(id) {
        return Ok(None);
    }
    let row: Option<FormRow> = sqlx::query_as(&format!("SELECT {FORM_COLUMNS} FROM forms WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    Ok(row.map(to_form))
}

pub async fn list(pool: &Db) -> Result<Vec<Form>> {
    let rows: Vec<FormRow> = sqlx::query_as(&format!("SELECT {FORM_COLUMNS} FROM forms ORDER BY created_at DESC, id DESC"))
        .fetch_all(pool)
        .await
        .map_err(dberr)?;
    Ok(rows.into_iter().map(to_form).collect())
}

pub async fn list_with_counts(pool: &Db) -> Result<Vec<FormWithCounts>> {
    let forms = list(pool).await?;
    let counts: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT form_id, status, COUNT(*) FROM form_submissions GROUP BY form_id, status")
            .fetch_all(pool)
            .await
            .map_err(dberr)?;
    Ok(forms
        .into_iter()
        .map(|form| {
            let mine = counts.iter().filter(|(id, _, _)| *id == form.id);
            // Spam is filed, not counted: the totals are what a person reads.
            let total = mine.clone().filter(|(_, status, _)| status != "spam").map(|(_, _, n)| n).sum();
            let unread = mine.filter(|(_, status, _)| status == "new").map(|(_, _, n)| n).sum();
            FormWithCounts { form, total, unread }
        })
        .collect())
}

pub struct FormInput<'a> {
    pub name: &'a str,
    pub status: &'a str,
    pub fields: &'a [Field],
    pub submit_label: &'a str,
    pub success_message: &'a str,
    pub notify_emails: &'a str,
    pub success_action: &'a str,
    pub success_url: &'a str,
    pub success_link_label: &'a str,
}

fn status_or_active(status: &str) -> &'static str {
    if status == "closed" { "closed" } else { "active" }
}

pub async fn create(pool: &Db, input: &FormInput<'_>) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO forms (id, name, status, fields, submit_label, success_message, notify_emails, \
         success_action, success_url, success_link_label, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(input.name)
    .bind(status_or_active(input.status))
    .bind(serde_json::to_string(input.fields)?)
    .bind(input.submit_label)
    .bind(input.success_message)
    .bind(input.notify_emails)
    .bind(ghost_core::forms::SuccessAction::parse(input.success_action).as_str())
    .bind(input.success_url)
    .bind(input.success_link_label)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

pub async fn update(pool: &Db, id: &str, input: &FormInput<'_>) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE forms SET name = ?, status = ?, fields = ?, submit_label = ?, success_message = ?, notify_emails = ?, \
         success_action = ?, success_url = ?, success_link_label = ?, updated_at = ? WHERE id = ?",
    )
    .bind(input.name)
    .bind(status_or_active(input.status))
    .bind(serde_json::to_string(input.fields)?)
    .bind(input.submit_label)
    .bind(input.success_message)
    .bind(input.notify_emails)
    .bind(ghost_core::forms::SuccessAction::parse(input.success_action).as_str())
    .bind(input.success_url)
    .bind(input.success_link_label)
    .bind(dates::now_db())
    .bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(res.rows_affected() > 0)
}

/// Delete a form and every response to it.
///
/// The responses are deleted explicitly rather than left to the foreign
/// key's cascade, so the outcome does not depend on the engine enforcing it.
pub async fn delete(pool: &Db, id: &str) -> Result<bool> {
    let mut tx = pool.begin().await.map_err(dberr)?;
    sqlx::query("DELETE FROM form_submissions WHERE form_id = ?").bind(id).execute(&mut *tx).await.map_err(dberr)?;
    let res = sqlx::query("DELETE FROM forms WHERE id = ?").bind(id).execute(&mut *tx).await.map_err(dberr)?;
    tx.commit().await.map_err(dberr)?;
    Ok(res.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// responses
// ---------------------------------------------------------------------------

type SubmissionRow = (
    String,
    String,
    String,
    Text,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

const SUBMISSION_COLUMNS: &str = "id, form_id, form_name, data, status, ip, user_agent, page_path, member_id, created_at";

fn to_submission((id, form_id, form_name, data, status, ip, user_agent, page_path, member_id, created_at): SubmissionRow) -> Submission {
    Submission {
        id,
        form_id,
        form_name,
        answers: ghost_core::forms::parse_answers(&data),
        status,
        ip,
        user_agent,
        page_path,
        member_id,
        created_at,
    }
}

pub struct NewSubmission<'a> {
    pub form_id: &'a str,
    pub form_name: &'a str,
    pub answers: &'a [Answer],
    pub nonce: &'a str,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub page_path: Option<&'a str>,
    pub member_id: Option<&'a str>,
}

/// What became of an attempt to file a response.
#[derive(Debug, PartialEq, Eq)]
pub enum Filed {
    Stored(String),
    /// The token's nonce has already been spent: this is a replay (or a
    /// double-click), and nothing was written.
    Replayed,
}

/// File a response.
///
/// The nonce column is `UNIQUE`, so two requests carrying the same token
/// cannot both succeed however they interleave — the database decides, not a
/// check-then-insert that could race.
pub async fn file(pool: &Db, new: &NewSubmission<'_>) -> Result<Filed> {
    let id = ids::new_id();
    let res = sqlx::query(
        "INSERT INTO form_submissions (id, form_id, form_name, data, status, nonce, ip, user_agent, page_path, member_id, created_at) \
         VALUES (?, ?, ?, ?, 'new', ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(new.form_id)
    .bind(new.form_name)
    .bind(serde_json::to_string(new.answers)?)
    .bind(new.nonce)
    .bind(new.ip)
    .bind(new.user_agent)
    .bind(new.page_path)
    .bind(new.member_id)
    .bind(dates::now_db())
    .execute(pool)
    .await;
    match res {
        Ok(_) => Ok(Filed::Stored(id)),
        Err(e) => {
            let unique = e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false);
            if unique {
                Ok(Filed::Replayed)
            } else {
                Err(dberr(e))
            }
        }
    }
}

pub async fn find_submission(pool: &Db, id: &str) -> Result<Option<Submission>> {
    if !ids::is_objectid(id) {
        return Ok(None);
    }
    let row: Option<SubmissionRow> =
        sqlx::query_as(&format!("SELECT {SUBMISSION_COLUMNS} FROM form_submissions WHERE id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(dberr)?;
    Ok(row.map(to_submission))
}

/// Which responses a list shows.
#[derive(Debug, Clone, Default)]
pub struct Browse<'a> {
    /// One form's responses, or every form's.
    pub form_id: Option<&'a str>,
    /// `new`, `read`, `spam`; `None` is everything except spam.
    pub status: Option<&'a str>,
    pub page: i64,
    pub limit: i64,
}

/// The WHERE clause and its binds, shared by the page query and the count so
/// the two cannot disagree.
fn browse_where(browse: &Browse<'_>) -> (String, Vec<String>) {
    let mut clauses: Vec<&str> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(form_id) = browse.form_id {
        clauses.push("form_id = ?");
        binds.push(form_id.to_string());
    }
    match browse.status {
        // Only the three known statuses ever reach the query, and as a bound
        // value even then.
        Some(status @ ("new" | "read" | "spam")) => {
            clauses.push("status = ?");
            binds.push(status.to_string());
        }
        _ => clauses.push("status <> 'spam'"),
    }
    (format!("WHERE {}", clauses.join(" AND ")), binds)
}

pub async fn browse(pool: &Db, browse: &Browse<'_>) -> Result<(Vec<Submission>, i64)> {
    let (where_sql, binds) = browse_where(browse);
    let limit = browse.limit.clamp(1, 5000);
    let offset = (browse.page.max(1) - 1).saturating_mul(limit);

    let sql = format!(
        "SELECT {SUBMISSION_COLUMNS} FROM form_submissions {where_sql} ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
    );
    let mut query = sqlx::query_as::<_, SubmissionRow>(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    let rows = query.bind(limit).bind(offset).fetch_all(pool).await.map_err(dberr)?;

    let count_sql = format!("SELECT COUNT(*) FROM form_submissions {where_sql}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql);
    for bind in &binds {
        count = count.bind(bind);
    }
    let total = count.fetch_one(pool).await.map_err(dberr)?;

    Ok((rows.into_iter().map(to_submission).collect(), total))
}

/// `(new, read, spam)` across every form, or one form.
pub async fn totals(pool: &Db, form_id: Option<&str>) -> Result<(i64, i64, i64)> {
    let rows: Vec<(String, i64)> = match form_id {
        Some(id) => sqlx::query_as("SELECT status, COUNT(*) FROM form_submissions WHERE form_id = ? GROUP BY status")
            .bind(id)
            .fetch_all(pool)
            .await,
        None => sqlx::query_as("SELECT status, COUNT(*) FROM form_submissions GROUP BY status").fetch_all(pool).await,
    }
    .map_err(dberr)?;
    let of = |status: &str| rows.iter().find(|(s, _)| s == status).map(|(_, n)| *n).unwrap_or(0);
    Ok((of("new"), of("read"), of("spam")))
}

pub async fn set_submission_status(pool: &Db, id: &str, status: &str) -> Result<bool> {
    if !matches!(status, "new" | "read" | "spam") {
        return Ok(false);
    }
    let res = sqlx::query("UPDATE form_submissions SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(res.rows_affected() > 0)
}

/// Mark a response read, but only if it is still new — opening a response
/// filed as spam must not quietly un-file it.
pub async fn mark_read_if_new(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("UPDATE form_submissions SET status = 'read' WHERE id = ? AND status = 'new'")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

pub async fn delete_submission(pool: &Db, id: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM form_submissions WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(res.rows_affected() > 0)
}

/// Drop responses older than `days`. Returns how many went.
pub async fn purge_older_than(pool: &Db, days: i64) -> Result<u64> {
    if days <= 0 {
        return Ok(0);
    }
    let cutoff = dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(days)));
    let res = sqlx::query("DELETE FROM form_submissions WHERE created_at < ?").bind(cutoff).execute(pool).await.map_err(dberr)?;
    Ok(res.rows_affected())
}

// ---------------------------------------------------------------------------
// site-wide settings
// ---------------------------------------------------------------------------

pub async fn config(pool: &Db) -> Result<Config> {
    let row: Option<(i64, String, i64, i64)> =
        sqlx::query_as("SELECT notify_enabled, notify_emails, store_ip, retention_days FROM forms_config WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(dberr)?;
    let (notify_enabled, notify_emails, store_ip, retention_days) = row.unwrap_or((1, String::new(), 1, 0));
    Ok(Config { notify_enabled: notify_enabled != 0, notify_emails, store_ip: store_ip != 0, retention_days })
}

pub async fn save_config(pool: &Db, config: &Config) -> Result<()> {
    sqlx::query("UPDATE forms_config SET notify_enabled = ?, notify_emails = ?, store_ip = ?, retention_days = ? WHERE id = 1")
        .bind(i64::from(config.notify_enabled))
        .bind(&config.notify_emails)
        .bind(i64::from(config.store_ip))
        .bind(config.retention_days.clamp(0, 3650))
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// The key that signs submission tokens, created on first use.
///
/// It has a column of its own rather than reusing one of the session
/// secrets, so that a token minted for a form can never be mistaken for —
/// or replayed as — anything else this server signs. It is never returned by
/// [`config`], so no settings screen or API can leak it.
pub async fn signing_secret(pool: &Db) -> Result<String> {
    let read = || async {
        sqlx::query_scalar::<_, String>("SELECT secret FROM forms_config WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(dberr)
    };
    if let Some(secret) = read().await?.filter(|s| !s.is_empty()) {
        return Ok(secret);
    }
    // Only fills an empty slot, so two requests racing here agree on
    // whichever write landed first.
    sqlx::query("UPDATE forms_config SET secret = ? WHERE id = 1 AND secret = ''")
        .bind(ids::random_hex(32))
        .execute(pool)
        .await
        .map_err(dberr)?;
    read()
        .await?
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ghost_core::GhostError::Internal("forms: no signing secret".into()))
}
