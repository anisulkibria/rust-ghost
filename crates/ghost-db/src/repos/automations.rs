//! Automations repository — the chain, the runs, and the steps that move
//! them forward.
//!
//! Mirrors Ghost's `database-automations-repository.ts`. Three behaviours
//! here carry most of the weight, and none of them are obvious from the
//! table definitions:
//!
//! * **Saving an automation revises rather than overwrites.** An action whose
//!   contents changed gets a *new* revision row; one that did not keep its
//!   existing one. A run already in flight holds a revision id, so the email
//!   a member is waiting for is the one that was there when they entered.
//!   Actions dropped from the chain are soft-deleted for the same reason.
//!
//! * **Steps are claimed, not just read.** The poller may be re-entered (a
//!   timer and a signup can land together), so a step is locked to one worker
//!   for the length of an attempt, and a lock older than the timeout is
//!   treated as abandoned rather than blocking forever.
//!
//! * **A member enters an automation at most once.** Checked at trigger time
//!   against the run table, which is why runs are kept even after a member is
//!   deleted.

use ghost_core::automations::{Action, ActionKind, Edge, Status, TriggerScope};
use ghost_core::{dates, ids, GhostError, Result};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::dberr;

/// Ghost's `LOCK_TIMEOUT_MS` — how long a claimed step stays claimed before
/// another worker may take it.
pub const LOCK_TIMEOUT_SECS: i64 = 30 * 60;
/// Ghost's `MAX_ATTEMPTS`.
pub const MAX_ATTEMPTS: i64 = 10;
/// Ghost's `MAX_STEPS_PER_BATCH`.
pub const MAX_STEPS_PER_BATCH: i64 = 100;
/// Ghost's `RETRY_DELAY_MS`.
pub const RETRY_DELAY_SECS: i64 = 10 * 60;

// ---------------------------------------------------------------------------
// shapes
// ---------------------------------------------------------------------------

/// An automation without its chain — what the list screen shows.
#[derive(Debug, Clone)]
pub struct AutomationSummary {
    pub id: String,
    pub slug: Option<String>,
    pub name: String,
    pub description: String,
    pub status: String,
    pub trigger_tier_scope: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub stats: RunStats,
}

/// The counts Ghost shows beside each automation.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunStats {
    pub total_run_count: i64,
    pub in_progress_run_count: i64,
    pub completed_run_count: i64,
    pub exited_early_run_count: i64,
}

/// An automation with everything needed to render or run it.
#[derive(Debug, Clone)]
pub struct Automation {
    pub summary: AutomationSummary,
    pub trigger_tier_ids: Vec<String>,
    pub actions: Vec<StoredAction>,
    pub edges: Vec<Edge>,
}

/// One action as stored, with the per-email counters from every revision it
/// has ever had.
#[derive(Debug, Clone)]
pub struct StoredAction {
    pub id: String,
    pub kind: ActionKind,
    pub stats: EmailStats,
    pub email_design_setting_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EmailStats {
    pub sent: i64,
    pub opened: i64,
    pub clicked: i64,
}

impl EmailStats {
    /// Ghost rounds these to whole percentages and reports `null` rather than
    /// 0% when nothing has been sent — an unopened email and an unsent one
    /// are different facts.
    pub fn opened_rate(&self) -> Option<i64> {
        (self.sent > 0).then(|| (self.opened as f64 / self.sent as f64 * 100.0).round() as i64)
    }
    pub fn clicked_rate(&self) -> Option<i64> {
        (self.sent > 0).then(|| (self.clicked as f64 / self.sent as f64 * 100.0).round() as i64)
    }
}

/// A step the poller has claimed, with everything it needs to act.
#[derive(Debug, Clone)]
pub struct StepToRun {
    pub id: String,
    pub locked_by: String,
    pub step_attempts: i64,
    pub automation_run_id: String,
    pub automation_id: String,
    pub automation_status: String,
    pub automation_trigger_tier_scope: Option<String>,
    pub automation_trigger_tier_ids: Vec<String>,
    pub member_id: Option<String>,
    pub action_id: String,
    pub automation_action_revision_id: String,
    pub kind: ActionKind,
}

/// The outcome of one claim: what to run now, and when to look again.
#[derive(Debug, Default)]
pub struct ClaimedSteps {
    pub steps: Vec<StepToRun>,
    /// The soonest `ready_at` still pending, so the caller can sleep exactly
    /// that long instead of polling blindly.
    pub next_ready_at: Option<String>,
}

/// A run joined to its member, with the step-status tallies that classify it.
type RunRow = (String, String, Option<String>, String, Option<String>, bool, i64, i64, i64);

/// One member's journey, as the runs list shows it.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: String,
    pub created_at: String,
    pub member_id: Option<String>,
    pub member_email: String,
    pub member_name: Option<String>,
    /// Whether the member row still exists.
    pub member_exists: bool,
    /// `in_progress` | `completed` | `exited_early`
    pub status: String,
    /// Ghost surfaces this separately from the status: an exited run that
    /// exited because a send failed is a different problem from one whose
    /// member unsubscribed.
    pub failed: bool,
}

/// `(id, type, wait_hours, email_subject, email_lexical)` — the latest
/// revision of one action, joined to its owning action row.
type ActionRow = (String, String, Option<i64>, Option<String>, Option<String>, Option<String>);

/// A recorded step joined to the revision it ran, plus the earliest send
/// timestamp recorded against it.
type RunStepRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
);

/// One recorded step of a run, for the history panel.
#[derive(Debug, Clone)]
pub struct RunStep {
    pub id: String,
    pub created_at: String,
    pub ready_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub action_type: String,
    pub wait_hours: Option<i64>,
    pub email_subject: Option<String>,
    pub email_sent_at: Option<String>,
}

// ---------------------------------------------------------------------------
// reading
// ---------------------------------------------------------------------------

type SummaryRow = (
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
);

const SUMMARY_COLS: &str =
    "id, slug, name, description, status, trigger_tier_scope, created_at, updated_at";

fn row_to_summary(r: SummaryRow, stats: RunStats) -> AutomationSummary {
    AutomationSummary {
        id: r.0,
        slug: r.1,
        name: r.2,
        description: r.3,
        status: r.4,
        trigger_tier_scope: r.5,
        created_at: r.6,
        updated_at: r.7,
        stats,
    }
}

/// Every automation, newest first, each with its run counts.
///
/// `statuses` filters the list the way the screen's Active / Archived / All
/// selector does; an empty slice means no filter.
pub async fn browse(pool: &SqlitePool, statuses: &[&str]) -> Result<Vec<AutomationSummary>> {
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT ");
    qb.push(SUMMARY_COLS);
    qb.push(" FROM automations");
    if !statuses.is_empty() {
        qb.push(" WHERE status IN (");
        let mut sep = qb.separated(", ");
        for s in statuses {
            sep.push_bind(s.to_string());
        }
        qb.push(")");
    }
    qb.push(" ORDER BY created_at DESC, id DESC");
    let rows: Vec<SummaryRow> = qb.build_query_as().fetch_all(pool).await.map_err(dberr)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let stats = run_stats(pool, &row.0).await?;
        out.push(row_to_summary(row, stats));
    }
    Ok(out)
}

/// Classify every run of one automation and count the buckets.
///
/// Ghost reads these from Tinybird when it is configured and falls back to
/// SQL otherwise; this port only has the fallback, so it is the whole story
/// here. The classification is Ghost's: a run with any pending step is in
/// progress, one whose steps all finished is complete, and anything else
/// exited early.
pub async fn run_stats(pool: &SqlitePool, automation_id: &str) -> Result<RunStats> {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           COUNT(*), \
           COALESCE(SUM(CASE WHEN pending > 0 THEN 1 ELSE 0 END), 0), \
           COALESCE(SUM(CASE WHEN pending = 0 AND exited = 0 AND steps > 0 THEN 1 ELSE 0 END), 0) \
         FROM ( \
           SELECT r.id, \
             (SELECT COUNT(*) FROM automation_run_steps s WHERE s.automation_run_id = r.id) AS steps, \
             (SELECT COUNT(*) FROM automation_run_steps s WHERE s.automation_run_id = r.id AND s.status = 'pending') AS pending, \
             (SELECT COUNT(*) FROM automation_run_steps s WHERE s.automation_run_id = r.id \
                AND s.status NOT IN ('pending', 'finished')) AS exited \
           FROM automation_runs r WHERE r.automation_id = ? \
         ) \
         WHERE steps > 0",
    )
    .bind(automation_id)
    .fetch_one(pool)
    .await
    .map_err(dberr)?;

    let (total, in_progress, completed) = row;
    Ok(RunStats {
        total_run_count: total,
        in_progress_run_count: in_progress,
        completed_run_count: completed,
        exited_early_run_count: total - in_progress - completed,
    })
}

/// When this automation last took someone in. `None` means nobody yet.
pub async fn last_run_at(pool: &SqlitePool, automation_id: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT MAX(created_at) FROM automation_runs WHERE automation_id = ?")
        .bind(automation_id)
        .fetch_one(pool)
        .await
        .map_err(dberr)
}

pub async fn count(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM automations")
        .fetch_one(pool)
        .await
        .map_err(dberr)
}

/// One automation with its chain, in running order.
pub async fn find(pool: &SqlitePool, id: &str) -> Result<Option<Automation>> {
    let Some(row) = sqlx::query_as::<_, SummaryRow>(&format!(
        "SELECT {SUMMARY_COLS} FROM automations WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?
    else {
        return Ok(None);
    };

    let stats = run_stats(pool, id).await?;
    let trigger_tier_ids = sqlx::query_scalar::<_, String>(
        "SELECT product_id FROM automation_trigger_tiers WHERE automation_id = ? ORDER BY product_id",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;

    Ok(Some(Automation {
        summary: row_to_summary(row, stats),
        trigger_tier_ids,
        actions: load_actions(pool, id).await?,
        edges: load_edges(pool, id).await?,
    }))
}

/// The live actions of an automation — latest revision of each, soft-deleted
/// ones excluded — in creation order.
async fn load_actions(pool: &SqlitePool, automation_id: &str) -> Result<Vec<StoredAction>> {
    let rows: Vec<ActionRow> = sqlx::query_as(
        "SELECT a.id, a.type, r.wait_hours, r.email_subject, r.email_lexical, r.email_design_setting_id \
         FROM automation_actions a \
         JOIN automation_action_revisions r ON r.action_id = a.id \
         WHERE a.automation_id = ? AND a.deleted_at IS NULL \
           AND r.created_at = (SELECT MAX(created_at) FROM automation_action_revisions \
                               WHERE action_id = a.id) \
         ORDER BY a.created_at, a.id",
    )
    .bind(automation_id)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, action_type, wait_hours, subject, lexical, email_design_setting_id) in rows {
        // Counters are summed across every revision, so editing an email
        // does not reset the numbers it has already earned.
        let stats: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT SUM(email_sent_count), SUM(email_opened_count), SUM(email_clicked_count) \
             FROM automation_action_revisions WHERE action_id = ?",
        )
        .bind(&id)
        .fetch_one(pool)
        .await
        .map_err(dberr)?;

        let kind = build_kind(&action_type, wait_hours, subject, lexical, &id)?;
        out.push(StoredAction {
            id,
            kind,
            email_design_setting_id,
            stats: EmailStats {
                sent: stats.0.unwrap_or(0),
                opened: stats.1.unwrap_or(0),
                clicked: stats.2.unwrap_or(0),
            },
        });
    }
    Ok(out)
}

fn build_kind(
    action_type: &str,
    wait_hours: Option<i64>,
    subject: Option<String>,
    lexical: Option<String>,
    action_id: &str,
) -> Result<ActionKind> {
    match action_type {
        "wait" => Ok(ActionKind::Wait {
            wait_hours: wait_hours.ok_or_else(|| {
                GhostError::Internal(format!("wait action {action_id} has no wait_hours"))
            })?,
        }),
        "send_email" => Ok(ActionKind::SendEmail {
            subject: subject.unwrap_or_default(),
            lexical: lexical.unwrap_or_default(),
        }),
        other => Err(GhostError::Internal(format!(
            "unexpected automation action type {other}"
        ))),
    }
}

/// Edges between live actions only — a soft-deleted node's links are history.
async fn load_edges(pool: &SqlitePool, automation_id: &str) -> Result<Vec<Edge>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT e.source_action_id, e.target_action_id FROM automation_action_edges e \
         JOIN automation_actions s ON s.id = e.source_action_id AND s.deleted_at IS NULL \
         JOIN automation_actions t ON t.id = e.target_action_id AND t.deleted_at IS NULL \
           AND t.automation_id = s.automation_id \
         WHERE s.automation_id = ? ORDER BY e.source_action_id, e.target_action_id",
    )
    .bind(automation_id)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(source_action_id, target_action_id)| Edge { source_action_id, target_action_id })
        .collect())
}

// ---------------------------------------------------------------------------
// writing
// ---------------------------------------------------------------------------

/// What `create` and `update` accept. Both take a whole automation: the
/// editor saves the entire chain in one request, since a half-applied chain
/// would be a graph that fails its own validation.
pub struct AutomationInput<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub status: Status,
    pub trigger_tier_scope: Option<TriggerScope>,
    pub trigger_tier_ids: &'a [String],
    pub actions: &'a [Action],
    pub edges: &'a [Edge],
}

pub async fn create(pool: &SqlitePool, input: &AutomationInput<'_>) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO automations (id, status, name, description, slug, trigger_tier_scope, created_at, updated_at) \
         VALUES (?, ?, ?, ?, NULL, ?, ?, ?)",
    )
    .bind(&id)
    .bind(input.status.as_str())
    .bind(input.name)
    .bind(input.description)
    .bind(input.trigger_tier_scope.map(|s| s.as_str()))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;

    replace_trigger_tiers(pool, &id, input.trigger_tier_scope, input.trigger_tier_ids).await?;
    replace_graph(pool, &id, input.actions, input.edges).await?;
    Ok(id)
}

pub async fn update(pool: &SqlitePool, id: &str, input: &AutomationInput<'_>) -> Result<bool> {
    let now = dates::now_db();
    let changed = sqlx::query(
        "UPDATE automations SET name = ?, description = ?, status = ?, trigger_tier_scope = ?, updated_at = ? \
         WHERE id = ?",
    )
    .bind(input.name)
    .bind(input.description)
    .bind(input.status.as_str())
    .bind(input.trigger_tier_scope.map(|s| s.as_str()))
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?
    .rows_affected();
    if changed == 0 {
        return Ok(false);
    }

    replace_trigger_tiers(pool, id, input.trigger_tier_scope, input.trigger_tier_ids).await?;
    replace_graph(pool, id, input.actions, input.edges).await?;

    // Pausing or archiving stops the runs in flight. Ghost leaves a step that
    // another worker currently holds alone — it will notice the status when it
    // finishes — so only unlocked or abandoned steps are cancelled here.
    if input.status != Status::Active {
        cancel_pending_steps(pool, id).await?;
    }
    Ok(true)
}

/// Set only the status, for the list screen's pause/archive buttons.
pub async fn set_status(pool: &SqlitePool, id: &str, status: Status) -> Result<bool> {
    let changed = sqlx::query("UPDATE automations SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?
        .rows_affected();
    if changed > 0 && status != Status::Active {
        cancel_pending_steps(pool, id).await?;
    }
    Ok(changed > 0)
}

/// Remove an automation and everything hanging off it — the opposite of
/// `archived`, which keeps the runs as the record of emails real people were
/// sent. Deleting takes that history with it, which is why only the admin
/// UI offers it, only for flows without a reserved slug, and behind a
/// confirm that says so.
///
/// Children go before parents because foreign keys are enforced: recipients
/// and run steps reference revisions, revisions and edges reference actions,
/// and actions, runs and tiers reference the automation. Returns `false` if
/// the automation did not exist, like `set_status` does.
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool> {
    let mut tx = pool.begin().await.map_err(dberr)?;

    // emails recorded against this automation's steps or revisions — test
    // sends can carry a revision without a run step, so both are checked
    sqlx::query(
        "DELETE FROM automated_email_recipients WHERE \
         automation_run_step_id IN (SELECT rs.id FROM automation_run_steps rs \
             JOIN automation_runs r ON rs.automation_run_id = r.id WHERE r.automation_id = ?) \
         OR automation_action_revision_id IN (SELECT ar.id FROM automation_action_revisions ar \
             JOIN automation_actions a ON ar.action_id = a.id WHERE a.automation_id = ?)",
    )
    .bind(id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(dberr)?;

    sqlx::query(
        "DELETE FROM automation_run_steps WHERE automation_run_id IN \
         (SELECT id FROM automation_runs WHERE automation_id = ?)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(dberr)?;
    sqlx::query("DELETE FROM automation_runs WHERE automation_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;
    sqlx::query(
        "DELETE FROM automation_action_edges WHERE source_action_id IN \
         (SELECT id FROM automation_actions WHERE automation_id = ?) \
         OR target_action_id IN (SELECT id FROM automation_actions WHERE automation_id = ?)",
    )
    .bind(id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(dberr)?;
    sqlx::query(
        "DELETE FROM automation_action_revisions WHERE action_id IN \
         (SELECT id FROM automation_actions WHERE automation_id = ?)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(dberr)?;
    sqlx::query("DELETE FROM automation_actions WHERE automation_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;
    sqlx::query("DELETE FROM automation_trigger_tiers WHERE automation_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;

    let changed = sqlx::query("DELETE FROM automations WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?
        .rows_affected();
    tx.commit().await.map_err(dberr)?;
    Ok(changed > 0)
}

async fn replace_trigger_tiers(
    pool: &SqlitePool,
    automation_id: &str,
    scope: Option<TriggerScope>,
    tier_ids: &[String],
) -> Result<()> {
    sqlx::query("DELETE FROM automation_trigger_tiers WHERE automation_id = ?")
        .bind(automation_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    if scope != Some(TriggerScope::SelectedPaid) {
        return Ok(());
    }
    let mut seen = std::collections::BTreeSet::new();
    for product_id in tier_ids {
        if !seen.insert(product_id.as_str()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO automation_trigger_tiers (automation_id, product_id) VALUES (?, ?)",
        )
        .bind(automation_id)
        .bind(product_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    }
    Ok(())
}

/// Apply a submitted chain to an automation.
///
/// Three cases per action, which is where the revisioning lives:
///
/// * **New** — insert the action and its first revision.
/// * **Changed** — keep the action, add a revision. Runs holding the old
///   revision keep using it.
/// * **Unchanged** — touch nothing, so the counters and history stay put.
///
/// Actions no longer in the chain are soft-deleted, never removed: a finished
/// run still points at their revisions.
async fn replace_graph(
    pool: &SqlitePool,
    automation_id: &str,
    actions: &[Action],
    edges: &[Edge],
) -> Result<()> {
    let now = dates::now_db();

    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, type FROM automation_actions WHERE automation_id = ? AND deleted_at IS NULL",
    )
    .bind(automation_id)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;

    let submitted: std::collections::HashSet<&str> =
        actions.iter().map(|a| a.id.as_str()).collect();

    for (id, _) in existing.iter().filter(|(id, _)| !submitted.contains(id.as_str())) {
        sqlx::query("UPDATE automation_actions SET deleted_at = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(dberr)?;
    }

    for action in actions {
        let known = existing.iter().find(|(id, _)| id == &action.id);
        if known.is_none() {
            sqlx::query(
                "INSERT INTO automation_actions (id, created_at, updated_at, deleted_at, automation_id, type) \
                 VALUES (?, ?, ?, NULL, ?, ?)",
            )
            .bind(&action.id)
            .bind(&now)
            .bind(&now)
            .bind(automation_id)
            .bind(action.kind.type_str())
            .execute(pool)
            .await
            .map_err(dberr)?;
        } else {
            sqlx::query("UPDATE automation_actions SET type = ?, updated_at = ? WHERE id = ?")
                .bind(action.kind.type_str())
                .bind(&now)
                .bind(&action.id)
                .execute(pool)
                .await
                .map_err(dberr)?;
        }
        insert_revision_if_changed(pool, action, &now).await?;
    }

    // Edges carry no identity of their own, so the set is replaced wholesale.
    sqlx::query(
        "DELETE FROM automation_action_edges WHERE source_action_id IN \
         (SELECT id FROM automation_actions WHERE automation_id = ?)",
    )
    .bind(automation_id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    for edge in edges {
        sqlx::query(
            "INSERT OR IGNORE INTO automation_action_edges (source_action_id, target_action_id) VALUES (?, ?)",
        )
        .bind(&edge.source_action_id)
        .bind(&edge.target_action_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    }
    Ok(())
}

/// `(created_at, wait_hours, email_subject, email_lexical)` — one action
/// revision's contents, for comparing against a submitted one.
type RevisionRow = (String, Option<i64>, Option<String>, Option<String>);

/// Add a revision only when the contents actually differ from the latest one.
///
/// Without this check every save would fork every run's revision pointer and
/// multiply the rows that stats are summed over, for no change at all.
async fn insert_revision_if_changed(pool: &SqlitePool, action: &Action, now: &str) -> Result<()> {
    let latest: Option<RevisionRow> = sqlx::query_as(
        "SELECT created_at, wait_hours, email_subject, email_lexical \
         FROM automation_action_revisions WHERE action_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&action.id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;

    let (wait_hours, subject, lexical) = match &action.kind {
        ActionKind::Wait { wait_hours } => (Some(*wait_hours), None, None),
        ActionKind::SendEmail { subject, lexical } => {
            (None, Some(subject.clone()), Some(lexical.clone()))
        }
    };

    if let Some((latest_at, prev_wait, prev_subject, prev_lexical)) = &latest {
        let unchanged = *prev_wait == wait_hours
            && prev_subject.as_deref().unwrap_or_default() == subject.as_deref().unwrap_or_default()
            && prev_lexical.as_deref().unwrap_or_default() == lexical.as_deref().unwrap_or_default();
        if unchanged {
            return Ok(());
        }
        // `(created_at, action_id)` is unique, and two saves inside one second
        // are entirely possible from a form. Step forward rather than collide.
        let created_at = next_timestamp(latest_at, now);
        return insert_revision(pool, &action.id, &created_at, wait_hours, subject, lexical).await;
    }

    insert_revision(pool, &action.id, now, wait_hours, subject, lexical).await
}

/// The next usable revision timestamp: `now`, unless that is not after the
/// previous revision, in which case one second later than it.
fn next_timestamp(latest_at: &str, now: &str) -> String {
    if now > latest_at {
        return now.to_string();
    }
    dates::from_db_string(latest_at)
        .map(|d| dates::to_db_string(&(d + chrono::Duration::seconds(1))))
        .unwrap_or_else(|| now.to_string())
}

async fn insert_revision(
    pool: &SqlitePool,
    action_id: &str,
    created_at: &str,
    wait_hours: Option<i64>,
    subject: Option<String>,
    lexical: Option<String>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO automation_action_revisions \
         (id, created_at, action_id, wait_hours, email_subject, email_lexical, email_design_setting_id, \
          email_sent_count, email_opened_count, email_clicked_count) \
         VALUES (?, ?, ?, ?, ?, ?, (SELECT email_design_setting_id FROM automation_action_revisions WHERE action_id=? ORDER BY created_at DESC LIMIT 1), 0, 0, 0)",
    )
    .bind(ids::new_id())
    .bind(created_at)
    .bind(action_id)
    .bind(wait_hours)
    .bind(subject)
    .bind(lexical)
    .bind(action_id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// Stop the runs of an automation that has been paused or archived.
async fn cancel_pending_steps(pool: &SqlitePool, automation_id: &str) -> Result<u64> {
    let now = dates::now_db();
    let stale = stale_lock_cutoff();
    let res = sqlx::query(
        "UPDATE automation_run_steps \
         SET status = 'automation disabled', finished_at = ?, updated_at = ?, locked_by = NULL, locked_at = NULL \
         WHERE status = 'pending' \
           AND automation_run_id IN (SELECT id FROM automation_runs WHERE automation_id = ?) \
           AND (locked_by IS NULL OR locked_at < ?)",
    )
    .bind(&now)
    .bind(&now)
    .bind(automation_id)
    .bind(&stale)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(res.rows_affected())
}

fn stale_lock_cutoff() -> String {
    dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::seconds(LOCK_TIMEOUT_SECS)))
}

// ---------------------------------------------------------------------------
// triggering
// ---------------------------------------------------------------------------

/// The first action of an automation a member has just qualified for.
#[derive(Debug, Clone)]
struct FirstAction {
    automation_id: String,
    revision_id: String,
    kind_type: String,
    wait_hours: Option<i64>,
}

/// Start every automation this member now qualifies for.
///
/// Returns the number of runs created. Called on signup and on the move to
/// paid, which is why the member's status and tiers come in as arguments
/// rather than being read here: the caller knows what just changed.
pub async fn trigger(
    pool: &SqlitePool,
    member_id: &str,
    member_email: &str,
    member_status: &str,
    member_tier_ids: &[String],
) -> Result<usize> {
    let scope = if member_status == "free" { "free" } else { "all_paid" };

    // Scope-matched automations, plus the selected-tier ones this member's
    // tiers satisfy. Only the head action counts — the node with no live
    // predecessor is where a run begins.
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT a.id, r.id, act.type, r.wait_hours \
         FROM automations a \
         JOIN automation_actions act ON act.automation_id = a.id AND act.deleted_at IS NULL \
         JOIN automation_action_revisions r ON r.action_id = act.id \
         WHERE a.status = 'active' \
           AND r.created_at = (SELECT MAX(created_at) FROM automation_action_revisions WHERE action_id = act.id) \
           AND NOT EXISTS ( \
             SELECT 1 FROM automation_action_edges e \
             JOIN automation_actions src ON src.id = e.source_action_id AND src.deleted_at IS NULL \
             WHERE e.target_action_id = act.id) \
           AND (a.trigger_tier_scope = ",
    );
    qb.push_bind(scope.to_string());
    if member_status != "free" && !member_tier_ids.is_empty() {
        qb.push(
            " OR (a.trigger_tier_scope = 'selected_paid' AND EXISTS ( \
               SELECT 1 FROM automation_trigger_tiers t \
               WHERE t.automation_id = a.id AND t.product_id IN (",
        );
        let mut sep = qb.separated(", ");
        for tier in member_tier_ids {
            sep.push_bind(tier.clone());
        }
        qb.push(")))");
    }
    qb.push(") ORDER BY act.created_at, act.id");

    let rows: Vec<(String, String, String, Option<i64>)> =
        qb.build_query_as().fetch_all(pool).await.map_err(dberr)?;
    let firsts: Vec<FirstAction> = rows
        .into_iter()
        .map(|(automation_id, revision_id, kind_type, wait_hours)| FirstAction {
            automation_id,
            revision_id,
            kind_type,
            wait_hours,
        })
        .collect();

    let mut started = 0;
    for first in firsts {
        // One entry per member per automation, for all time. Re-running
        // someone who already received the emails is the failure mode this
        // prevents.
        let already: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM automation_runs WHERE automation_id = ? AND member_id = ?",
        )
        .bind(&first.automation_id)
        .bind(member_id)
        .fetch_one(pool)
        .await
        .map_err(dberr)?;
        if already > 0 {
            continue;
        }

        let now = chrono::Utc::now();
        let run_id = ids::new_id();
        let now_db = dates::to_db_string(&now);
        sqlx::query(
            "INSERT INTO automation_runs (id, created_at, updated_at, automation_id, member_id, member_email) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&run_id)
        .bind(&now_db)
        .bind(&now_db)
        .bind(&first.automation_id)
        .bind(member_id)
        .bind(member_email)
        .execute(pool)
        .await
        .map_err(dberr)?;

        let wait_secs = if first.kind_type == "wait" {
            first.wait_hours.unwrap_or(0) * 3600
        } else {
            0
        };
        let ready_at = dates::to_db_string(&(now + chrono::Duration::seconds(wait_secs)));
        insert_step(pool, &run_id, &first.revision_id, &ready_at).await?;
        started += 1;
    }
    Ok(started)
}

async fn insert_step(
    pool: &SqlitePool,
    run_id: &str,
    revision_id: &str,
    ready_at: &str,
) -> Result<()> {
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO automation_run_steps \
         (id, created_at, updated_at, automation_run_id, automation_action_revision_id, ready_at, \
          step_attempts, status) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 'pending')",
    )
    .bind(ids::new_id())
    .bind(&now)
    .bind(&now)
    .bind(run_id)
    .bind(revision_id)
    .bind(ready_at)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// the step queue
// ---------------------------------------------------------------------------

/// Claim up to `limit` due steps, and report when the next one comes due.
///
/// Ghost's three-phase approach, for the same reason: SQL cannot `UPDATE` a
/// bounded number of rows portably, so this picks candidates, stamps a lock
/// id onto the ones still eligible, and then reads back only what it won.
/// Two concurrent pollers therefore split the batch rather than both running
/// it.
pub async fn claim_steps(pool: &SqlitePool, limit: i64) -> Result<ClaimedSteps> {
    let now = dates::now_db();
    let stale = stale_lock_cutoff();
    let lock_id = ids::uuid_v4();

    let candidates: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM automation_run_steps \
         WHERE status = 'pending' AND ready_at <= ? AND (locked_by IS NULL OR locked_at < ?) \
         ORDER BY ready_at, created_at, id LIMIT ?",
    )
    .bind(&now)
    .bind(&stale)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;

    if candidates.is_empty() {
        return Ok(ClaimedSteps {
            steps: Vec::new(),
            next_ready_at: next_pending_ready_at(pool, &stale).await?,
        });
    }

    let mut qb = QueryBuilder::<Sqlite>::new(
        "UPDATE automation_run_steps SET locked_by = ",
    );
    qb.push_bind(lock_id.clone());
    qb.push(", locked_at = ");
    qb.push_bind(now.clone());
    qb.push(", started_at = ");
    qb.push_bind(now.clone());
    qb.push(", updated_at = ");
    qb.push_bind(now.clone());
    qb.push(", step_attempts = step_attempts + 1 WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in &candidates {
        sep.push_bind(id.clone());
    }
    qb.push(") AND status = 'pending' AND ready_at <= ");
    qb.push_bind(now.clone());
    qb.push(" AND (locked_by IS NULL OR locked_at < ");
    qb.push_bind(stale.clone());
    qb.push(")");
    qb.build().execute(pool).await.map_err(dberr)?;

    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT s.id, s.locked_by, s.step_attempts, s.automation_run_id, run.automation_id, \
                a.status, a.trigger_tier_scope, run.member_id, act.id, rev.id, act.type, \
                rev.wait_hours, rev.email_subject, rev.email_lexical \
         FROM automation_run_steps s \
         JOIN automation_runs run ON run.id = s.automation_run_id \
         JOIN automations a ON a.id = run.automation_id \
         JOIN automation_action_revisions rev ON rev.id = s.automation_action_revision_id \
         JOIN automation_actions act ON act.id = rev.action_id \
         WHERE s.locked_by = ",
    );
    qb.push_bind(lock_id);
    qb.push(" AND s.id IN (");
    let mut sep = qb.separated(", ");
    for id in &candidates {
        sep.push_bind(id.clone());
    }
    qb.push(") ORDER BY s.ready_at, s.created_at, s.id");

    type StepRow = (
        String,
        String,
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<StepRow> = qb.build_query_as().fetch_all(pool).await.map_err(dberr)?;

    let mut steps = Vec::with_capacity(rows.len());
    for r in rows {
        let kind = build_kind(&r.10, r.11, r.12, r.13, &r.8)?;
        let tier_ids = if r.6.as_deref() == Some("selected_paid") {
            sqlx::query_scalar::<_, String>(
                "SELECT product_id FROM automation_trigger_tiers WHERE automation_id = ?",
            )
            .bind(&r.4)
            .fetch_all(pool)
            .await
            .map_err(dberr)?
        } else {
            Vec::new()
        };
        steps.push(StepToRun {
            id: r.0,
            locked_by: r.1,
            step_attempts: r.2,
            automation_run_id: r.3,
            automation_id: r.4,
            automation_status: r.5,
            automation_trigger_tier_scope: r.6,
            automation_trigger_tier_ids: tier_ids,
            member_id: r.7,
            action_id: r.8,
            automation_action_revision_id: r.9,
            kind,
        });
    }

    Ok(ClaimedSteps {
        steps,
        next_ready_at: next_pending_ready_at(pool, &stale).await?,
    })
}

async fn next_pending_ready_at(pool: &SqlitePool, stale: &str) -> Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT ready_at FROM automation_run_steps \
         WHERE status = 'pending' AND (locked_by IS NULL OR locked_at < ?) \
         ORDER BY ready_at LIMIT 1",
    )
    .bind(stale)
    .fetch_optional(pool)
    .await
    .map_err(dberr)
}

/// Write a terminal status onto a step this worker still holds.
///
/// The lock check is what makes a lost lock safe: if the lock expired and
/// another worker picked the step up, this update matches nothing and the
/// late worker's result is discarded rather than overwriting the new one's.
pub async fn finish_step(pool: &SqlitePool, step: &StepToRun, status: &str) -> Result<bool> {
    let now = dates::now_db();
    let changed = sqlx::query(
        "UPDATE automation_run_steps \
         SET status = ?, finished_at = ?, updated_at = ?, locked_by = NULL, locked_at = NULL \
         WHERE id = ? AND status = 'pending' AND locked_by = ?",
    )
    .bind(status)
    .bind(&now)
    .bind(&now)
    .bind(&step.id)
    .bind(&step.locked_by)
    .execute(pool)
    .await
    .map_err(dberr)?
    .rows_affected();
    Ok(changed > 0)
}

/// Put a failed step back in the queue. Returns false when the automation was
/// switched off meanwhile, in which case the step is closed instead.
pub async fn retry_step(pool: &SqlitePool, step: &StepToRun, retry_at: &str) -> Result<bool> {
    let still_active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM automation_run_steps s \
         JOIN automation_runs r ON r.id = s.automation_run_id \
         JOIN automations a ON a.id = r.automation_id \
         WHERE s.id = ? AND a.status = 'active'",
    )
    .bind(&step.id)
    .fetch_one(pool)
    .await
    .map_err(dberr)?;
    if still_active == 0 {
        finish_step(pool, step, "automation disabled").await?;
        return Ok(false);
    }

    let now = dates::now_db();
    let changed = sqlx::query(
        "UPDATE automation_run_steps \
         SET status = 'pending', started_at = NULL, finished_at = NULL, ready_at = ?, \
             updated_at = ?, locked_by = NULL, locked_at = NULL \
         WHERE id = ? AND status = 'pending' AND locked_by = ?",
    )
    .bind(retry_at)
    .bind(&now)
    .bind(&step.id)
    .bind(&step.locked_by)
    .execute(pool)
    .await
    .map_err(dberr)?
    .rows_affected();
    Ok(changed > 0)
}

/// Close a finished step and queue whatever comes next.
///
/// Returns the moment the next step comes due, so the poller can sleep until
/// then instead of guessing. `None` means this run is over.
pub async fn finish_and_enqueue_next(
    pool: &SqlitePool,
    step: &StepToRun,
) -> Result<Option<String>> {
    if !finish_step(pool, step, "finished").await? {
        return Ok(None);
    }
    // The author may have switched the automation off while this step ran.
    let still_active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM automation_runs r JOIN automations a ON a.id = r.automation_id \
         WHERE r.id = ? AND a.status = 'active'",
    )
    .bind(&step.automation_run_id)
    .fetch_one(pool)
    .await
    .map_err(dberr)?;
    if still_active == 0 {
        return Ok(None);
    }

    let next: Option<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT rev.id, act.type, rev.wait_hours \
         FROM automation_action_edges e \
         JOIN automation_actions act ON act.id = e.target_action_id AND act.deleted_at IS NULL \
         JOIN automation_action_revisions rev ON rev.action_id = act.id \
         WHERE e.source_action_id = ? \
           AND rev.created_at = (SELECT MAX(created_at) FROM automation_action_revisions WHERE action_id = act.id) \
         ORDER BY rev.created_at DESC, rev.id DESC LIMIT 1",
    )
    .bind(&step.action_id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;

    let Some((revision_id, kind_type, wait_hours)) = next else {
        return Ok(None);
    };
    let wait_secs = if kind_type == "wait" { wait_hours.unwrap_or(0) * 3600 } else { 0 };
    let ready_at = dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::seconds(wait_secs)));
    insert_step(pool, &step.automation_run_id, &revision_id, &ready_at).await?;
    Ok(Some(ready_at))
}

/// Record that an automation email went out: bump the revision's counter and
/// keep a recipient row, which is what per-action stats are built from.
pub async fn record_email_sent(
    pool: &SqlitePool,
    step: &StepToRun,
    member_id: &str,
    member_uuid: &str,
    member_email: &str,
    member_name: Option<&str>,
    track_opens: bool,
    track_clicks: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE automation_action_revisions \
         SET email_sent_count = COALESCE(email_sent_count, 0) + 1 WHERE id = ?",
    )
    .bind(&step.automation_action_revision_id)
    .execute(pool)
    .await
    .map_err(dberr)?;

    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO automated_email_recipients \
         (id, automation_action_revision_id, automation_run_step_id, member_id, member_uuid, \
          member_email, member_name, track_opens, track_clicks, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&step.automation_action_revision_id)
    .bind(&step.id)
    .bind(member_id)
    .bind(member_uuid)
    .bind(member_email)
    .bind(member_name)
    .bind(track_opens)
    .bind(track_clicks)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// runs, for the admin screens
// ---------------------------------------------------------------------------

/// One page of runs, newest first.
///
/// Status is derived from the recorded steps, the same way the counts are —
/// so a run's row and the cards above it can never disagree.
pub async fn browse_runs(
    pool: &SqlitePool,
    automation_id: &str,
    status_filter: Option<&str>,
    search: Option<&str>,
    limit: i64,
    page: i64,
) -> Result<(Vec<Run>, i64)> {
    let limit = if limit > 0 { limit } else { 50 };
    let page = page.max(1);

    // Runs with no recorded steps are excluded, as in Ghost: there is nothing
    // to classify them by.
    //
    // The automation id is bound rather than written into this fragment: a
    // literal `?` in text pushed to a QueryBuilder is not a placeholder it
    // knows about, so it would shift every later bind by one.
    const FROM_AND_WHERE: &str = "FROM automation_runs r \
         LEFT JOIN members m ON m.id = r.member_id \
         WHERE EXISTS (SELECT 1 FROM automation_run_steps s WHERE s.automation_run_id = r.id) \
           AND r.automation_id = ";

    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT r.id, r.created_at, r.member_id, r.member_email, m.name, (m.id IS NOT NULL), \
                (SELECT COUNT(*) FROM automation_run_steps s WHERE s.automation_run_id = r.id AND s.status = 'pending'), \
                (SELECT COUNT(*) FROM automation_run_steps s WHERE s.automation_run_id = r.id \
                   AND s.status NOT IN ('pending', 'finished')), \
                (SELECT COUNT(*) FROM automation_run_steps s WHERE s.automation_run_id = r.id AND s.status = 'failed') ",
    );
    qb.push(FROM_AND_WHERE);
    qb.push_bind(automation_id.to_string());
    push_search(&mut qb, search);
    qb.push(" ORDER BY r.created_at DESC, r.id DESC LIMIT ");
    qb.push_bind(limit);
    qb.push(" OFFSET ");
    qb.push_bind((page - 1) * limit);

    let rows: Vec<RunRow> = qb.build_query_as().fetch_all(pool).await.map_err(dberr)?;

    let mut runs: Vec<Run> = rows
        .into_iter()
        .map(|(id, created_at, member_id, member_email, member_name, member_exists, pending, exited, failed)| {
            let status = if pending > 0 {
                "in_progress"
            } else if exited > 0 {
                "exited_early"
            } else {
                "completed"
            };
            Run {
                id,
                created_at,
                member_id,
                member_email,
                member_name,
                member_exists,
                status: status.to_string(),
                // Ghost only flags a failure on a run that actually exited
                // because of one; a failed attempt that later succeeded is
                // not a failed run.
                failed: pending == 0 && failed > 0,
            }
        })
        .collect();

    // The status filter is applied after classification, since the
    // classification is what it filters on.
    if let Some(filter) = status_filter {
        runs.retain(|r| r.status == filter);
    }

    let mut cqb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) ");
    cqb.push(FROM_AND_WHERE);
    cqb.push_bind(automation_id.to_string());
    push_search(&mut cqb, search);
    let total: i64 = cqb.build_query_scalar().fetch_one(pool).await.map_err(dberr)?;
    Ok((runs, total))
}

/// Match a literal substring of the member's current name or email.
///
/// `%` and `_` in the query are escaped, so searching for `a_b` finds the
/// address containing `a_b` rather than every three-character run.
fn push_search(qb: &mut QueryBuilder<'_, Sqlite>, search: Option<&str>) {
    let Some(search) = search.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let pattern = format!(
        "%{}%",
        search.replace('!', "!!").replace('%', "!%").replace('_', "!_")
    );
    qb.push(" AND (m.name LIKE ");
    qb.push_bind(pattern.clone());
    qb.push(" ESCAPE '!' OR m.email LIKE ");
    qb.push_bind(pattern);
    qb.push(" ESCAPE '!')");
}

/// The recorded steps of one run, oldest first.
pub async fn run_history(pool: &SqlitePool, run_id: &str) -> Result<Vec<RunStep>> {
    let rows: Vec<RunStepRow> = sqlx::query_as(
            "SELECT s.id, s.created_at, s.ready_at, s.finished_at, s.status, act.type, \
                    rev.wait_hours, rev.email_subject, \
                    (SELECT MIN(created_at) FROM automated_email_recipients aer \
                     WHERE aer.automation_run_step_id = s.id) \
             FROM automation_run_steps s \
             JOIN automation_action_revisions rev ON rev.id = s.automation_action_revision_id \
             JOIN automation_actions act ON act.id = rev.action_id \
             WHERE s.automation_run_id = ? ORDER BY s.created_at, s.id",
        )
        .bind(run_id)
        .fetch_all(pool)
        .await
        .map_err(dberr)?;

    Ok(rows
        .into_iter()
        .map(|(id, created_at, ready_at, finished_at, status, action_type, wait_hours, email_subject, email_sent_at)| {
            RunStep {
                id,
                created_at,
                ready_at,
                finished_at,
                status,
                action_type,
                wait_hours,
                email_subject,
                email_sent_at,
            }
        })
        .collect())
}

/// Entry counts per day, for the chart on the automation screen.
pub async fn entries_by_day(
    pool: &SqlitePool,
    automation_id: &str,
    days: i64,
) -> Result<Vec<(String, i64)>> {
    let since = dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(days)));
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT substr(created_at, 1, 10), COUNT(*) FROM automation_runs \
         WHERE automation_id = ? AND created_at >= ? GROUP BY substr(created_at, 1, 10) \
         ORDER BY 1",
    )
    .bind(automation_id)
    .bind(&since)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_timestamp_always_advances() {
        // the ordinary case: now is later, so use it
        assert_eq!(next_timestamp("2026-01-01 00:00:00", "2026-01-01 00:00:05"), "2026-01-01 00:00:05");
        // two saves inside one second must not collide on the unique index
        assert_eq!(next_timestamp("2026-01-01 00:00:05", "2026-01-01 00:00:05"), "2026-01-01 00:00:06");
        // a clock that went backwards still moves the revision forward
        assert_eq!(next_timestamp("2026-01-01 00:00:09", "2026-01-01 00:00:05"), "2026-01-01 00:00:10");
    }

    #[test]
    fn email_rates_are_whole_percentages_and_null_when_unsent() {
        let none = EmailStats::default();
        assert_eq!(none.opened_rate(), None);
        assert_eq!(none.clicked_rate(), None);

        let some = EmailStats { sent: 8, opened: 2, clicked: 1 };
        assert_eq!(some.opened_rate(), Some(25));
        // 1/8 = 12.5%, which rounds to 13
        assert_eq!(some.clicked_rate(), Some(13));
    }

    #[test]
    fn the_stale_lock_cutoff_is_in_the_past() {
        assert!(stale_lock_cutoff() < dates::now_db());
    }
}
