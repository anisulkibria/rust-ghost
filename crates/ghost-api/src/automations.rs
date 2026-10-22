//! The Admin API's automation endpoints (`/ghost/api/admin/automations/*`).
//!
//! Paths, payload shapes and error messages follow Ghost 6.72's
//! `core/server/web/api/endpoints/admin/routes.js` and
//! `services/automations/automations-api.ts`, so an external tool written
//! against Ghost's API works here unchanged.
//!
//! Two deliberate differences from Ghost, both because of what this port
//! does not have:
//!
//! * **Stats come from SQL, not Tinybird.** Ghost reads run statistics from
//!   its analytics warehouse and keeps a SQL fallback for the automation
//!   list. This port only ever has the fallback, so `runs` and
//!   `performance-stats` answer from the local tables rather than returning
//!   the error Ghost gives when Tinybird is unconfigured — an answer the
//!   publisher can act on beats a 500 they cannot.
//!
//! * **No labs gate.** Ghost keeps automations behind a private flag during
//!   its beta. Here the feature is either built or not, and hiding a built
//!   feature behind a flag the UI then has to explain is worse than showing
//!   it.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use ghost_core::automations::{self as rules, Action, ActionKind, Edge, Status, TriggerScope};
use ghost_db::repos::automations as repo;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::AppState;

pub fn automations_router() -> Router<AppState> {
    Router::new()
        .route("/automations", get(browse).post(add))
        .route("/automations/", get(browse).post(add))
        // `poll` is a literal segment and must be declared before `{id}`,
        // or an automation whose id happened to be "poll" would shadow it.
        .route("/automations/poll", put(poll))
        .route("/automations/poll/", put(poll))
        .route("/automations/{id}", get(read).put(edit))
        .route("/automations/{id}/", get(read).put(edit))
        .route("/automations/{id}/runs", get(browse_runs))
        .route("/automations/{id}/runs/", get(browse_runs))
        .route("/automations/{id}/runs/{run_id}", get(read_run))
        .route("/automations/{id}/runs/{run_id}/", get(read_run))
        .route("/automations/{id}/performance-stats", get(performance_stats))
        .route("/automations/{id}/performance-stats/", get(performance_stats))
        .route("/automations/{id}/email_test", post(send_test_email))
        .route("/automations/{id}/email_test/", post(send_test_email))
}

// ---------------------------------------------------------------------------
// responses
// ---------------------------------------------------------------------------

fn err(status: StatusCode, message: &str, error_type: &str) -> Response {
    (
        status,
        Json(json!({ "errors": [{ "message": message, "errorType": error_type }] })),
    )
        .into_response()
}

fn api_error(e: &ghost_core::GhostError) -> Response {
    (
        StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(json!({ "errors": [{
            "message": e.message(),
            "errorType": e.error_type(),
            // Ghost reports the offending field here, which is what lets the
            // editor put the message next to the input that caused it.
            "property": e.context(),
        }] })),
    )
        .into_response()
}

fn ok_json(body: Value) -> Response {
    (StatusCode::OK, [(header::CACHE_CONTROL, "private, no-cache, no-store")], Json(body))
        .into_response()
}

/// Automations are staff-only, like every other Admin API resource here.
async fn require_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ghost_core::models::User, Response> {
    crate::admin::current_user(state, headers)
        .await
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Authentication required", "NoPermissionError"))
}

// ---------------------------------------------------------------------------
// serialization
// ---------------------------------------------------------------------------

/// Ghost's `buildAutomationSummary` plus the browse stats.
pub fn summary_json(a: &repo::AutomationSummary, last_run_at: Option<&str>) -> Value {
    json!({
        "id": a.id,
        "slug": a.slug,
        "name": a.name,
        "description": a.description,
        "status": a.status,
        "trigger_tier_scope": a.trigger_tier_scope,
        "created_at": ghost_core::dates::db_to_api(Some(&a.created_at)),
        "updated_at": ghost_core::dates::db_to_api(a.updated_at.as_deref()),
        "stats": {
            "last_run_created_at": ghost_core::dates::db_to_api(last_run_at),
            "total_run_count": a.stats.total_run_count,
            "in_progress_run_count": a.stats.in_progress_run_count,
        },
    })
}

/// One action, in the shape the editor sends back.
pub fn action_json(action: &repo::StoredAction) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(action.id));
    o.insert("type".into(), json!(action.kind.type_str()));
    match &action.kind {
        ActionKind::Wait { wait_hours } => {
            o.insert("data".into(), json!({ "wait_hours": wait_hours }));
        }
        ActionKind::SendEmail { subject, lexical } => {
            o.insert(
                "data".into(),
                json!({
                    "email_subject": subject,
                    "email_lexical": lexical,
                    "email_design_setting_id": Value::Null,
                }),
            );
            o.insert(
                "stats".into(),
                json!({
                    "email_sent_count": action.stats.sent,
                    "email_opened_count": action.stats.opened,
                    "email_clicked_count": action.stats.clicked,
                    "opened_rate": action.stats.opened_rate(),
                    "clicked_rate": action.stats.clicked_rate(),
                }),
            );
        }
    }
    Value::Object(o)
}

/// A whole automation, chain included.
pub fn automation_json(a: &repo::Automation, last_run_at: Option<&str>) -> Value {
    let mut o = summary_json(&a.summary, last_run_at);
    let obj = o.as_object_mut().expect("summary_json builds an object");
    obj.insert(
        "trigger_tier_ids".into(),
        if a.summary.trigger_tier_scope.as_deref() == Some("selected_paid") {
            json!(a.trigger_tier_ids)
        } else {
            Value::Null
        },
    );
    // Ghost returns actions in storage order and leaves the ordering to the
    // edges; the port sorts them into running order as a convenience, since
    // the list is what the admin screen renders directly.
    let actions = rules::ordered_actions(
        &a.actions
            .iter()
            .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
            .collect::<Vec<_>>(),
        &a.edges,
    )
    .iter()
    .filter_map(|ordered| a.actions.iter().find(|s| s.id == ordered.id))
    .map(action_json)
    .collect::<Vec<_>>();
    obj.insert("actions".into(), Value::Array(actions));
    obj.insert(
        "edges".into(),
        json!(a
            .edges
            .iter()
            .map(|e| json!({
                "source_action_id": e.source_action_id,
                "target_action_id": e.target_action_id,
            }))
            .collect::<Vec<_>>()),
    );
    o
}

fn run_json(run: &repo::Run) -> Value {
    json!({
        "id": run.id,
        "created_at": ghost_core::dates::db_to_api(Some(&run.created_at)),
        "status": run.status,
        "failed": run.failed,
        // A deleted member leaves a run with no member, which Ghost reports
        // as null rather than substituting the historical address.
        "member": if run.member_exists {
            json!({ "id": run.member_id, "name": run.member_name, "email": run.member_email })
        } else {
            Value::Null
        },
    })
}

// ---------------------------------------------------------------------------
// request parsing
// ---------------------------------------------------------------------------

/// Pull the one automation out of Ghost's `{automations: [ … ]}` envelope.
fn body_automation(body: &Value) -> Option<&Value> {
    body.get("automations").and_then(|a| a.as_array()).and_then(|a| a.first())
}

/// Parsed and validated submission, ready for the repository.
#[derive(Debug)]
struct Submission {
    name: String,
    description: String,
    status: Status,
    trigger_tier_scope: Option<TriggerScope>,
    trigger_tier_ids: Vec<String>,
    actions: Vec<Action>,
    edges: Vec<Edge>,
}

fn validation(message: &str, property: &str) -> ghost_core::GhostError {
    ghost_core::GhostError::ValidationError {
        message: message.to_string(),
        context: Some(property.to_string()),
    }
}

/// Parse a submitted automation. `existing` supplies the fields an edit may
/// legitimately omit.
fn parse_submission(
    data: &Value,
    existing: Option<&repo::Automation>,
) -> ghost_core::Result<Submission> {
    let str_field = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_string);

    let name = str_field("name")
        .or_else(|| existing.map(|e| e.summary.name.clone()))
        .unwrap_or_default();
    let description = str_field("description")
        .or_else(|| existing.map(|e| e.summary.description.clone()))
        .unwrap_or_default();

    let status_str = str_field("status")
        .or_else(|| existing.map(|e| e.summary.status.clone()))
        .unwrap_or_else(|| "inactive".into());
    let status = Status::parse(&status_str).ok_or_else(|| {
        validation("Automation status must be one of: active, inactive, archived.", "status")
    })?;

    // `trigger_tier_scope` absent on an edit means "leave it alone"; present
    // and null means free signups, which is Ghost's default.
    let (trigger_tier_scope, scope_given) = match data.get("trigger_tier_scope") {
        Some(Value::Null) => (None, true),
        Some(Value::String(s)) => (
            Some(TriggerScope::parse(s).ok_or_else(|| {
                validation(
                    "Trigger tier scope must be one of: free, all_paid, selected_paid.",
                    "trigger_tier_scope",
                )
            })?),
            true,
        ),
        Some(_) => return Err(validation("Invalid trigger tier scope.", "trigger_tier_scope")),
        None => (
            existing.and_then(|e| e.summary.trigger_tier_scope.as_deref()).and_then(TriggerScope::parse),
            false,
        ),
    };

    let trigger_tier_ids: Vec<String> = match data.get("trigger_tier_ids") {
        Some(Value::Array(ids)) => ids
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ if !scope_given => existing.map(|e| e.trigger_tier_ids.clone()).unwrap_or_default(),
        _ => Vec::new(),
    };
    if trigger_tier_scope == Some(TriggerScope::SelectedPaid) && trigger_tier_ids.is_empty() {
        return Err(validation(
            "Select at least one tier for a tier-scoped automation.",
            "trigger_tier_ids",
        ));
    }

    let (actions, edges) = match data.get("actions") {
        Some(Value::Array(items)) => {
            let actions = items.iter().map(parse_action).collect::<ghost_core::Result<Vec<_>>>()?;
            let edges = match data.get("edges") {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|e| {
                        Ok(Edge {
                            source_action_id: e
                                .get("source_action_id")
                                .and_then(Value::as_str)
                                .ok_or_else(|| validation("Edge is missing a source.", "edges"))?
                                .to_string(),
                            target_action_id: e
                                .get("target_action_id")
                                .and_then(Value::as_str)
                                .ok_or_else(|| validation("Edge is missing a target.", "edges"))?
                                .to_string(),
                        })
                    })
                    .collect::<ghost_core::Result<Vec<_>>>()?,
                // No `edges` with `actions` present means the caller is
                // describing a straight chain in order, which is how the
                // port's own admin UI submits.
                _ => rules::edges_for_chain(&actions),
            };
            (actions, edges)
        }
        // A status-only edit keeps the chain it already has.
        _ => match existing {
            Some(e) => (
                e.actions
                    .iter()
                    .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
                    .collect(),
                e.edges.clone(),
            ),
            None => (Vec::new(), Vec::new()),
        },
    };

    rules::validate(status, &name, &description, &actions, &edges)?;

    Ok(Submission {
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        status,
        trigger_tier_scope,
        trigger_tier_ids,
        actions,
        edges,
    })
}

fn parse_action(item: &Value) -> ghost_core::Result<Action> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| ghost_core::ids::is_objectid(s))
        .map(str::to_string)
        // The editor generates ids client-side so edges can reference new
        // nodes; a missing or malformed one gets a fresh one rather than a
        // rejection, since the chain is still well-defined without it.
        .unwrap_or_else(ghost_core::ids::new_id);
    let data = item.get("data").cloned().unwrap_or(Value::Null);

    match item.get("type").and_then(Value::as_str) {
        Some("wait") => {
            let wait_hours = data
                .get("wait_hours")
                .and_then(Value::as_i64)
                .ok_or_else(|| validation("Enter a wait between 1 and 30 days.", "actions"))?;
            Ok(Action { id, kind: ActionKind::Wait { wait_hours } })
        }
        Some("send_email") => Ok(Action {
            id,
            kind: ActionKind::SendEmail {
                subject: data
                    .get("email_subject")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                lexical: data
                    .get("email_lexical")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            },
        }),
        other => Err(validation(
            &format!(
                "Automation action type must be wait or send_email, not {}.",
                other.unwrap_or("nothing")
            ),
            "actions",
        )),
    }
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

async fn browse(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    // Ghost's list defaults to everything that is not archived, which is what
    // makes archiving a way of tidying up rather than a hidden delete.
    let statuses: Vec<&str> = match q.get("filter").map(String::as_str) {
        Some("archived") => vec!["archived"],
        Some("all") => vec![],
        _ => vec!["active", "inactive"],
    };
    match repo::browse(&state.db, &statuses).await {
        Ok(list) => {
            let mut items = Vec::with_capacity(list.len());
            for a in &list {
                let last = repo::last_run_at(&state.db, &a.id).await.ok().flatten();
                items.push(summary_json(a, last.as_deref()));
            }
            ok_json(json!({ "automations": items }))
        }
        Err(e) => api_error(&e),
    }
}

async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repo::find(&state.db, &id).await {
        Ok(Some(a)) => {
            let last = repo::last_run_at(&state.db, &id).await.ok().flatten();
            ok_json(json!({ "automations": [automation_json(&a, last.as_deref())] }))
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    // Ghost's soft cap. Racing past it by one is acceptable and intended —
    // the alternative is serialising every create behind a lock.
    match repo::count(&state.db).await {
        Ok(n) if n >= rules::MAX_AUTOMATIONS => {
            return err(
                StatusCode::FORBIDDEN,
                &format!("Cannot create more than {} automations.", rules::MAX_AUTOMATIONS),
                "HostLimitError",
            )
        }
        Err(e) => return api_error(&e),
        Ok(_) => {}
    }

    let Some(data) = body_automation(&body) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Invalid automation payload.", "ValidationError");
    };
    let submission = match parse_submission(data, None) {
        Ok(s) => s,
        Err(e) => return api_error(&e),
    };
    let input = repo::AutomationInput {
        name: &submission.name,
        description: &submission.description,
        status: submission.status,
        trigger_tier_scope: submission.trigger_tier_scope,
        trigger_tier_ids: &submission.trigger_tier_ids,
        actions: &submission.actions,
        edges: &submission.edges,
    };
    match repo::create(&state.db, &input).await {
        Ok(id) => match repo::find(&state.db, &id).await {
            Ok(Some(a)) => (
                StatusCode::CREATED,
                Json(json!({ "automations": [automation_json(&a, None)] })),
            )
                .into_response(),
            Ok(None) => err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError"),
            Err(e) => api_error(&e),
        },
        Err(e) => api_error(&e),
    }
}

async fn edit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError");
    };
    let Some(data) = body_automation(&body) else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Invalid automation edit payload.",
            "ValidationError",
        );
    };
    let submission = match parse_submission(data, Some(&existing)) {
        Ok(s) => s,
        Err(e) => return api_error(&e),
    };
    let input = repo::AutomationInput {
        name: &submission.name,
        description: &submission.description,
        status: submission.status,
        trigger_tier_scope: submission.trigger_tier_scope,
        trigger_tier_ids: &submission.trigger_tier_ids,
        actions: &submission.actions,
        edges: &submission.edges,
    };
    match repo::update(&state.db, &id, &input).await {
        Ok(true) => {
            // Turning one on may make steps due that were cancelled while it
            // was off, and a newly active chain should not wait for a tick.
            crate::automations_engine::request_poll();
            match repo::find(&state.db, &id).await {
                Ok(Some(a)) => {
                    let last = repo::last_run_at(&state.db, &id).await.ok().flatten();
                    ok_json(json!({ "automations": [automation_json(&a, last.as_deref())] }))
                }
                Ok(None) => err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError"),
                Err(e) => api_error(&e),
            }
        }
        Ok(false) => err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

/// Ghost's wake-the-poller endpoint. Its scheduler calls this after a
/// restart; here it is a manual nudge, and it answers 204 either way.
async fn poll(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    crate::automations_engine::request_poll();
    StatusCode::NO_CONTENT.into_response()
}

async fn browse_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let status = q.get("status").map(String::as_str);
    if let Some(s) = status {
        if !matches!(s, "in_progress" | "completed" | "exited_early") {
            return err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Automation run status must be one of: in_progress, completed, exited_early.",
                "ValidationError",
            );
        }
    }
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(50).clamp(1, 50);
    let page = q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).max(1);

    match repo::browse_runs(&state.db, &id, status, q.get("search").map(String::as_str), limit, page)
        .await
    {
        Ok((runs, total)) => ok_json(json!({
            "automation_runs": runs.iter().map(run_json).collect::<Vec<_>>(),
            "meta": { "pagination": ghost_core::models::pagination_json(page, limit, total) },
        })),
        Err(e) => api_error(&e),
    }
}

async fn read_run(
    State(state): State<AppState>,
    Path((id, run_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    // The run must belong to the automation in the path, or a run id would be
    // enough to read any site's history regardless of which automation it is
    // nested under.
    let owned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM automation_runs WHERE id = ? AND automation_id = ?",
    )
    .bind(&run_id)
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    if owned == 0 {
        return err(StatusCode::NOT_FOUND, "Automation run not found.", "NotFoundError");
    }

    match repo::run_history(&state.db, &run_id).await {
        Ok(steps) => {
            let pending = steps.iter().filter(|s| s.status == "pending").count();
            let exited = steps
                .iter()
                .filter(|s| s.status != "pending" && s.status != "finished")
                .count();
            let status = if pending > 0 {
                "in_progress"
            } else if exited > 0 {
                "exited_early"
            } else {
                "completed"
            };
            ok_json(json!({ "automation_run_history": [{
                "id": run_id,
                "automation_id": id,
                "status": status,
                "failed": pending == 0 && steps.iter().any(|s| s.status == "failed"),
                "steps": steps.iter().map(|s| json!({
                    "id": s.id,
                    "type": s.action_type,
                    "status": s.status,
                    "created_at": ghost_core::dates::db_to_api(Some(&s.created_at)),
                    "ready_at": ghost_core::dates::db_to_api(Some(&s.ready_at)),
                    "finished_at": ghost_core::dates::db_to_api(s.finished_at.as_deref()),
                    "wait_hours": s.wait_hours,
                    "email_subject": s.email_subject,
                    "email_sent_at": ghost_core::dates::db_to_api(s.email_sent_at.as_deref()),
                })).collect::<Vec<_>>(),
            }] }))
        }
        Err(e) => api_error(&e),
    }
}

/// Ghost's `performance-stats`, answered from SQL.
async fn performance_stats(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repo::find(&state.db, &id).await {
        Ok(None) => return err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError"),
        Err(e) => return api_error(&e),
        Ok(Some(_)) => {}
    }
    let days = q.get("days").and_then(|d| d.parse::<i64>().ok()).unwrap_or(30).clamp(1, 365);

    let stats = match repo::run_stats(&state.db, &id).await {
        Ok(s) => s,
        Err(e) => return api_error(&e),
    };
    let entries = match repo::entries_by_day(&state.db, &id, days).await {
        Ok(e) => e,
        Err(e) => return api_error(&e),
    };
    ok_json(json!({ "automation_performance_stats": [{
        "automation_id": id,
        "total_run_count": stats.total_run_count,
        "in_progress_run_count": stats.in_progress_run_count,
        "completed_run_count": stats.completed_run_count,
        "exited_early_run_count": stats.exited_early_run_count,
        "entries": entries.iter().map(|(date, count)| json!({ "date": date, "count": count }))
            .collect::<Vec<_>>(),
    }] }))
}

/// Send one of an automation's emails to an address, so the author can see
/// what it looks like before anyone else does.
async fn send_test_email(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Ok(Some(automation)) = repo::find(&state.db, &id).await else {
        return err(StatusCode::NOT_FOUND, "Automation not found.", "NotFoundError");
    };
    // Default to the signed-in user's own address: the common case is "show
    // me", and asking them to type their own address is friction.
    let to = body
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| user.email.clone())
        .unwrap_or_default();
    if !to.contains('@') {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Please enter a valid email address", "ValidationError");
    }

    let action_id = body.get("action_id").and_then(Value::as_str);
    let email = automation.actions.iter().find(|a| {
        matches!(a.kind, ActionKind::SendEmail { .. })
            && action_id.map(|want| want == a.id).unwrap_or(true)
    });
    let Some(repo::StoredAction { kind: ActionKind::SendEmail { subject, lexical }, .. }) = email
    else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This automation has no email to test.",
            "ValidationError",
        );
    };
    if subject.trim().is_empty() || rules::is_empty_lexical(lexical) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Add a subject line and a message before this email can be sent.",
            "ValidationError",
        );
    }

    // A test send is addressed to a staff user, who is not a member, so the
    // personalisation tokens get placeholder values rather than a lookup.
    let recipient = crate::automations_engine::MemberSnapshotLike {
        uuid: "preview",
        email: &to,
        name: user.name.as_deref(),
        status: "free",
    };
    match crate::automations_engine::send_automation_email(&state, &recipient, subject, lexical)
        .await
    {
        Ok(()) => ok_json(json!({ "sent": true, "email": to })),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e, "EmailError"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(actions: Value, extra: Value) -> Value {
        let mut o = json!({ "name": "Welcome", "description": "", "status": "inactive", "actions": actions });
        if let (Value::Object(target), Value::Object(src)) = (&mut o, extra) {
            target.extend(src);
        }
        o
    }

    #[test]
    fn parses_a_chain_and_wires_it_in_order() {
        let data = body(
            json!([
                { "id": "aaaaaaaaaaaaaaaaaaaaaaaa", "type": "send_email",
                  "data": { "email_subject": "Hi", "email_lexical": "{}" } },
                { "id": "bbbbbbbbbbbbbbbbbbbbbbbb", "type": "wait", "data": { "wait_hours": 48 } }
            ]),
            json!({}),
        );
        let s = parse_submission(&data, None).unwrap();
        assert_eq!(s.actions.len(), 2);
        // no `edges` submitted, so the chain is inferred from the order
        assert_eq!(s.edges.len(), 1);
        assert_eq!(s.edges[0].source_action_id, "aaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(s.edges[0].target_action_id, "bbbbbbbbbbbbbbbbbbbbbbbb");
    }

    #[test]
    fn rejects_an_unknown_action_type() {
        let data = body(json!([{ "id": "aaaaaaaaaaaaaaaaaaaaaaaa", "type": "send_sms" }]), json!({}));
        let e = parse_submission(&data, None).unwrap_err();
        assert!(e.message().contains("wait or send_email"), "{}", e.message());
    }

    #[test]
    fn rejects_an_unknown_status_or_scope() {
        let bad_status = body(json!([]), json!({ "status": "paused" }));
        assert!(parse_submission(&bad_status, None).is_err());
        let bad_scope = body(json!([]), json!({ "trigger_tier_scope": "sometimes" }));
        assert!(parse_submission(&bad_scope, None).is_err());
    }

    #[test]
    fn a_tier_scoped_automation_needs_tiers() {
        let without = body(json!([]), json!({ "trigger_tier_scope": "selected_paid" }));
        assert!(parse_submission(&without, None).is_err());
        let with = body(
            json!([]),
            json!({ "trigger_tier_scope": "selected_paid", "trigger_tier_ids": ["t1"] }),
        );
        let s = parse_submission(&with, None).unwrap();
        assert_eq!(s.trigger_tier_scope, Some(TriggerScope::SelectedPaid));
        assert_eq!(s.trigger_tier_ids, vec!["t1".to_string()]);
    }

    #[test]
    fn an_explicitly_null_scope_means_free_signups() {
        let data = body(json!([]), json!({ "trigger_tier_scope": Value::Null }));
        assert_eq!(parse_submission(&data, None).unwrap().trigger_tier_scope, None);
    }

    #[test]
    fn an_action_without_a_usable_id_gets_one() {
        let data = body(
            json!([{ "type": "wait", "data": { "wait_hours": 24 } }]),
            json!({}),
        );
        let s = parse_submission(&data, None).unwrap();
        assert!(ghost_core::ids::is_objectid(&s.actions[0].id));
    }

    #[test]
    fn validation_errors_name_the_field_that_caused_them() {
        let data = body(json!([{ "id": "aaaaaaaaaaaaaaaaaaaaaaaa", "type": "wait", "data": { "wait_hours": 9999 } }]), json!({}));
        let e = parse_submission(&data, None).unwrap_err();
        assert_eq!(e.context(), Some("actions"));
        assert_eq!(e.status_code(), 422);
    }

    #[test]
    fn email_stats_only_appear_on_email_actions() {
        let wait = repo::StoredAction {
            id: "a".into(),
            kind: ActionKind::Wait { wait_hours: 24 },
            stats: Default::default(),
        };
        let j = action_json(&wait);
        assert_eq!(j["data"]["wait_hours"], json!(24));
        assert!(j.get("stats").is_none());

        let email = repo::StoredAction {
            id: "b".into(),
            kind: ActionKind::SendEmail { subject: "Hi".into(), lexical: "{}".into() },
            stats: repo::EmailStats { sent: 4, opened: 1, clicked: 0 },
        };
        let j = action_json(&email);
        assert_eq!(j["stats"]["email_sent_count"], json!(4));
        assert_eq!(j["stats"]["opened_rate"], json!(25));
        // nothing clicked out of four sent is 0%, not "unknown"
        assert_eq!(j["stats"]["clicked_rate"], json!(0));
    }

    #[test]
    fn a_deleted_member_leaves_a_run_with_no_member() {
        let run = repo::Run {
            id: "r1".into(),
            created_at: "2026-01-01 00:00:00".into(),
            member_id: None,
            member_email: "gone@x.test".into(),
            member_name: None,
            member_exists: false,
            status: "completed".into(),
            failed: false,
        };
        // the historical address is deliberately not substituted
        assert_eq!(run_json(&run)["member"], Value::Null);
    }
}
