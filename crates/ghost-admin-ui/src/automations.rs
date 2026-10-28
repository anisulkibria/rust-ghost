//! Automations: the list, and the flow editor.
//!
//! Ghost draws the chain as a React Flow canvas — a trigger node, a column of
//! wait and email nodes, an exit node. This is the same model in the port's
//! own idiom: the chain is a vertical list of cards, each a small form, with
//! the trigger pinned at the top and the exit at the bottom. The shape a
//! reader walks is identical; only the drawing differs.
//!
//! Everything posts and redirects, with no client-side state, which is what
//! makes the chain safe to edit: every save goes through the same validation
//! the API uses, so a half-built branch is rejected by the one rule rather
//! than by two that might disagree.
//!
//! Email bodies are written in Markdown here and stored as Ghost's
//! `email_lexical` — wrapped in a Koenig markdown card, the same way the port
//! carries markdown for posts. A body written here opens in Ghost's own
//! editor, and one written in Ghost renders here.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use ghost_api::AppState;
use ghost_core::automations::{self as rules, Action, ActionKind, Status, TriggerScope};
use ghost_db::repos::automations as repo;
use std::collections::HashMap;

use crate::{esc, flash_banner, page_for_role, require_role, svg, Role, SELECT_CARET};

/// The list's view selector, mirroring Ghost's.
const VIEWS: &[(&str, &str)] = &[
    ("active", "Active automations"),
    ("archived", "Archived automations"),
    ("all", "All automations"),
];

// ---------------------------------------------------------------------------
// the list
// ---------------------------------------------------------------------------

pub async fn automations_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };

    let view = q.get("view").map(String::as_str).unwrap_or("active");
    let statuses: Vec<&str> = match view {
        "archived" => vec!["archived"],
        "all" => vec![],
        // Ghost's default list is everything not archived: a paused
        // automation is still something you are working on.
        _ => vec!["active", "inactive"],
    };

    let list = match repo::browse(&state.db, &statuses).await {
        Ok(l) => l,
        Err(e) => {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.message()).into_response()
        }
    };

    let mut rows = String::new();
    for a in &list {
        let last = repo::last_run_at(&state.db, &a.id).await.ok().flatten();
        rows.push_str(&row(&state, a, last.as_deref()).await);
    }

    let empty = match view {
        "archived" => (
            "Nothing archived",
            "Archiving an automation takes it off your list without deleting it.",
        ),
        "active" if automation_count(&state).await > 0 => (
            "No active automations",
            "Every automation you have is archived. Switch views to see them.",
        ),
        _ => ("No automations yet", "Automations you create will show up here."),
    };

    let body = if list.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3;text-align:center">
<p style="margin:8px 0 4px;font-weight:600">{title}</p>
<p class="muted" style="margin:0 0 16px">{detail}</p></div>"#,
            title = esc(empty.0),
            detail = esc(empty.1),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:3">{rows}</div>"#)
    };

    // The system switch from Features comes before anything a single flow
    // can do: stopped means no enrolment and no sends, and a reader of this
    // screen should not have to discover that from the logs.
    let system_notice = if state.settings.get_str("automations_enabled").as_deref() == Some("false") {
        format!(
            r#"<div class="banner error rise">{stop}The automations system is switched off — nothing runs and nobody is enrolled. <a href="/ghost/settings/features/">Turn it on in Features</a>.</div>"#,
            stop = svg("hide"),
        )
    } else {
        String::new()
    };

    page_for_role(
        actor_role,
        "Automations",
        "automations",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Automations</h1>
<p class="sub">Email sequences that run themselves when someone joins</p></div>
<div class="actions"><form method="post" action="/ghost/automations/new/">
<button class="btn" type="submit">{add} New automation</button></form></div></header>
{banner}
{system_notice}
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-purple">{flow}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:260px">
<select onchange="location.href='/ghost/automations/?view='+this.value">{views}</select><span>Show</span>{car}</label></div></div>
</div>
{body}"#,
            add = svg("add"),
            flow = svg("flow"),
            car = SELECT_CARET,
            banner = flash_banner(&q),
            system_notice = system_notice,
            views = crate::select_options(VIEWS, view),
            body = body,
        ),
        true,
    )
    .into_response()
}

async fn automation_count(state: &AppState) -> i64 {
    repo::count(&state.db).await.unwrap_or(0)
}

/// One automation in the list: what it is, and how it is doing.
async fn row(state: &AppState, a: &repo::AutomationSummary, last_run_at: Option<&str>) -> String {
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let (chip_class, chip) = match a.status.as_str() {
        "active" => ("published", "active"),
        "archived" => ("tonal", "archived"),
        _ => ("draft", "paused"),
    };
    let trigger = a
        .trigger_tier_scope
        .as_deref()
        .and_then(TriggerScope::parse)
        .map(TriggerScope::label)
        .unwrap_or("Free member signed up");

    let last = match last_run_at {
        Some(at) => format!(
            "Last started {}",
            esc(&ghost_core::dates::short_date_in_zone(at, &timezone))
        ),
        None => "Never started".to_string(),
    };

    // The built-in welcome flows have no delete button — the seeder would
    // bring them straight back, and archive already covers "put it away".
    let del_form = if a.slug.is_none() {
        format!(
            r#"<form method="post" action="/ghost/automations/{id}/delete/" data-confirm="Delete automation|&#8220;{name}&#8221; and its run history will be removed.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>"#,
            id = esc(&a.id),
            name = esc(&a.name),
            del = svg("delete"),
        )
    } else {
        String::new()
    };

    format!(
        r#"<div class="row"><span class="ic tint-purple">{flow}</span>
<a class="body" href="/ghost/automations/{id}/"><span class="title">{name}</span>
<span class="detail">{trigger} · {last}</span></a>
<div class="trailing">
<span class="chip tonal" title="Members currently partway through">{in_progress} in progress</span>
<span class="chip tonal" title="Members who have ever entered">{total} total</span>
<span class="chip {chip_class}">{chip}</span>{del_form}{chev}</div></div>"#,
        flow = svg("flow"),
        id = esc(&a.id),
        name = esc(&a.name),
        trigger = esc(trigger),
        last = last,
        in_progress = a.stats.in_progress_run_count,
        total = a.stats.total_run_count,
        chip_class = chip_class,
        chip = chip,
        del_form = del_form,
        chev = svg("chevron"),
    )
}

// ---------------------------------------------------------------------------
// creating
// ---------------------------------------------------------------------------

/// Create an empty draft and open it.
///
/// Ghost's "New automation" goes straight into the editor on an unsaved
/// draft; here the row exists first, so the chain's actions have an
/// automation to belong to. It is inactive, so nothing runs until the author
/// turns it on.
pub async fn automation_create(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    if automation_count(&state).await >= rules::MAX_AUTOMATIONS {
        return flash(
            "error",
            &format!("You can have at most {} automations.", rules::MAX_AUTOMATIONS),
        );
    }
    let input = repo::AutomationInput {
        name: "New automation",
        description: "",
        status: Status::Inactive,
        trigger_tier_scope: Some(TriggerScope::Free),
        trigger_tier_ids: &[],
        actions: &[],
        edges: &[],
    };
    match repo::create(&state.db, &input).await {
        Ok(id) => Redirect::to(&format!("/ghost/automations/{id}/")).into_response(),
        Err(e) => flash("error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// the editor
// ---------------------------------------------------------------------------

pub async fn automation_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(automation)) = repo::find(&state.db, &id).await else {
        return crate::not_found_page(&user, "That automation could not be found.");
    };

    let ordered = ordered(&automation);
    let scope = automation
        .summary
        .trigger_tier_scope
        .as_deref()
        .and_then(TriggerScope::parse)
        .unwrap_or(TriggerScope::Free);

    let chain = format!(r#"{}<script src="{}/react.js" defer></script><script src="{}/react-jsx.js" defer></script><script src="{}/react-dom.js" defer></script><script src="{}/koenig-lexical.js" defer></script><script src="{}/automation-editor.js" defer></script>"#,chain_markup(&id,&ordered,&automation),crate::ASSETS,crate::ASSETS,crate::ASSETS,crate::ASSETS,crate::ASSETS);
    let runs = runs_panel(&state, &id).await;
    let tiers = tier_checkboxes(&state, &automation).await;

    let status = Status::parse(&automation.summary.status).unwrap_or(Status::Inactive);
    let (toggle_label, toggle_status, toggle_class) = match status {
        Status::Active => ("Turn off", "inactive", "btn tonal small danger"),
        // restoring lands on paused, not on: a flow that was put away should
        // not come back live without someone choosing that
        Status::Archived => ("Restore", "inactive", "btn small"),
        Status::Inactive => ("Turn on", "active", "btn small"),
    };
    // Turning one on is the point at which half-written emails stop being
    // acceptable, so say what will be checked before they press it.
    let activation_hint = match status {
        Status::Active => {
            r#"<p class="hint">New members matching the trigger start this sequence. Turning it off stops everyone partway through.</p>"#
        }
        Status::Archived => {
            r#"<p class="hint">This flow is archived: it is off the default list and triggers nothing. Restoring brings it back paused.</p>"#
        }
        Status::Inactive => {
            r#"<p class="hint">Turning this on checks that every email has a subject and a body. Members who sign up from then on will start the sequence.</p>"#
        }
    };
    // Archive is how a flow leaves the default list without deleting the run
    // history, so it is offered everywhere it is not already the state.
    let archive_form = if status != Status::Archived {
        format!(
            r#"<form method="post" action="/ghost/automations/{id}/status/">
<input type="hidden" name="status" value="archived">
<button class="btn tonal small" type="submit">{archive_ic} Archive</button></form>"#,
            id = esc(&id),
            archive_ic = svg("archive"),
        )
    } else {
        String::new()
    };
    // same rule as the list: a built-in welcome flow has no delete, only
    // pause and archive, because the seeder owns it
    let del_form = if rules::is_welcome_email_slug(automation.summary.slug.as_deref()) {
        String::new()
    } else {
        format!(
            r#"<form method="post" action="/ghost/automations/{id}/delete/" data-confirm="Delete automation|&#8220;{name}&#8221; and its run history will be removed.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>"#,
            id = esc(&id),
            name = esc(&automation.summary.name),
            del = svg("delete"),
        )
    };

    page_for_role(
        actor_role,
        "Automation",
        "automations",
        Some(&user),
        &format!(
            r#"<header class="page-head rise">
<a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/automations/" title="Back to automations">{back}</a>
<div><h1>{name}</h1><p class="sub">{chip} · {total} total · {in_progress} in progress</p></div>
<div class="actions">
<form method="post" action="/ghost/automations/{id}/status/">
<input type="hidden" name="status" value="{toggle_status}">
<button class="{toggle_class}" type="submit">{toggle_label}</button></form>
{archive_form}
<button class="btn" type="submit" form="automation-form">{check} Save</button>{del_form}</div></header>
{banner}
<form method="post" action="/ghost/automations/{id}/" id="automation-form"></form>

<div class="section">Details</div>
<div class="card padded rise" style="--i:1">
<label class="field"><input name="name" form="automation-form" value="{name_v}" placeholder=" " required maxlength="191"><span>Automation name</span></label>
<label class="field"><input name="description" form="automation-form" value="{description}" placeholder=" " maxlength="2000"><span>Description · staff only</span></label>
{activation_hint}
</div>

<div class="section">Trigger</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-green">{bolt}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px">
<select name="trigger_tier_scope" form="automation-form">{scope_opts}</select><span>Starts when</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Automations run once per member. Someone who has already been through this one will not start it again.</p></div></div>
{tiers}
</div>

<div class="section">Sequence</div>
{chain}

{runs}"#,
            back = svg("chevron"),
            check = svg("check"),
            bolt = svg("flow"),
            car = SELECT_CARET,
            banner = flash_banner(&q),
            id = esc(&id),
            name = esc(&automation.summary.name),
            name_v = esc(&automation.summary.name),
            description = esc(&automation.summary.description),
            chip = match status {
                Status::Active => "Active",
                Status::Archived => "Archived",
                Status::Inactive => "Paused",
            },
            total = automation.summary.stats.total_run_count,
            in_progress = automation.summary.stats.in_progress_run_count,
            toggle_label = toggle_label,
            toggle_status = toggle_status,
            toggle_class = toggle_class,
            archive_form = archive_form,
            del_form = del_form,
            activation_hint = activation_hint,
            scope_opts = crate::select_options(
                &[
                    ("free", "A free member signs up"),
                    ("all_paid", "A paid subscription starts — any tier"),
                    ("selected_paid", "A paid subscription starts — selected tiers"),
                ],
                scope.as_str(),
            ),
            tiers = tiers,
            chain = chain,
            runs = runs,
        ),
        true,
    )
    .into_response()
}

/// The automation's actions in running order.
fn ordered(automation: &repo::Automation) -> Vec<repo::StoredAction> {
    let as_actions: Vec<Action> = automation
        .actions
        .iter()
        .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
        .collect();
    rules::ordered_actions(&as_actions, &automation.edges)
        .iter()
        .filter_map(|o| automation.actions.iter().find(|s| s.id == o.id).cloned())
        .collect()
}

/// The tier picker, shown only when the trigger is tier-scoped.
///
/// It is always in the DOM rather than toggled by script: with no JavaScript
/// the author still needs to be able to pick tiers, and an unticked box on a
/// non-tier scope is simply ignored on save.
async fn tier_checkboxes(state: &AppState, automation: &repo::Automation) -> String {
    let tiers: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM products WHERE type = 'paid' AND active = 1 ORDER BY name",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    if tiers.is_empty() {
        return format!(
            r#"<div class="row"><span class="ic tint-slate">{coin}</span>
<div class="body"><span class="detail">No paid tiers available. Create one in <a href="/ghost/settings/membership/">Settings → Membership</a> to scope an automation to a tier.</span></div></div>"#,
            coin = svg("chart"),
        );
    }

    let chips: String = tiers
        .iter()
        .map(|(tid, name)| {
            format!(
                r#"<label class="fchip"><input type="checkbox" name="trigger_tier_ids" form="automation-form" value="{tid}" {checked}><span class="ck">{check}</span>{name}</label>"#,
                tid = esc(tid),
                checked = if automation.trigger_tier_ids.contains(tid) { "checked" } else { "" },
                check = svg("check"),
                name = esc(name),
            )
        })
        .collect();

    format!(
        r#"<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{tags}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Tiers</span>
<div class="chips">{chips}</div>
<p class="muted" style="margin:10px 0 0">Used only when the trigger above is scoped to selected tiers.</p></div></div>"#,
        tags = svg("tags"),
        chips = chips,
    )
}

/// The sequence: trigger, the chain, exit — plus the controls to extend it.
fn chain_markup(id: &str, ordered: &[repo::StoredAction], automation: &repo::Automation) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        r#"<div class="card rows rise native-flow" style="--i:3">
<div class="row"><span class="ic tint-green">{bolt}</span>
<div class="body"><span class="title">Trigger</span><span class="detail">{trigger}</span></div></div>"#,
        bolt = svg("flow"),
        trigger = esc(
            automation
                .summary
                .trigger_tier_scope
                .as_deref()
                .and_then(TriggerScope::parse)
                .map(TriggerScope::label)
                .unwrap_or("Free member signed up")
        ),
    ));

    for (index, action) in ordered.iter().enumerate() {
        out.push_str(&step_markup(id, index, ordered.len(), action));
    }

    if ordered.is_empty() {
        out.push_str(
            r#"<div class="row"><span class="ic tint-slate"></span>
<div class="body"><span class="detail">Nothing happens yet. Add a wait or an email below.</span></div></div>"#,
        );
    }

    out.push_str(&format!(
        r#"<div class="row"><span class="ic tint-slate">{stop}</span>
<div class="body"><span class="title">Exit automation</span><span class="detail">The member leaves the sequence here</span></div></div>
</div>
<div style="display:flex;gap:10px;padding:12px 4px">
<form method="post" action="/ghost/automations/{id}/actions/add/"><input type="hidden" name="type" value="wait">
<button class="btn tonal small" type="submit">{add} Add a wait</button></form>
<form method="post" action="/ghost/automations/{id}/actions/add/"><input type="hidden" name="type" value="send_email">
<button class="btn tonal small" type="submit">{add} Add an email</button></form>
</div>
<p class="muted" style="margin:0 0 4px 16px">Steps run top to bottom. A wait holds the member for a while; an email sends and moves straight on.</p>"#,
        stop = svg("check"),
        add = svg("add"),
        id = esc(id),
    ));
    out
}

/// One step of the chain, as an editable card.
fn step_markup(id: &str, index: usize, total: usize, action: &repo::StoredAction) -> String {
    let move_buttons = format!(
        r#"{up}{down}
<form method="post" action="/ghost/automations/{id}/actions/{aid}/delete/" data-confirm="Remove this step|Members partway through the sequence keep the version they started on.">
<button class="icon-btn danger" title="Remove step" aria-label="Remove step">{del}</button></form>"#,
        up = if index > 0 {
            format!(
                r#"<form method="post" action="/ghost/automations/{id}/actions/{aid}/move/"><input type="hidden" name="direction" value="up">
<button class="icon-btn" title="Move up" aria-label="Move up">{ic}</button></form>"#,
                id = esc(id),
                aid = esc(&action.id),
                ic = svg("up"),
            )
        } else {
            String::new()
        },
        down = if index + 1 < total {
            format!(
                r#"<form method="post" action="/ghost/automations/{id}/actions/{aid}/move/"><input type="hidden" name="direction" value="down">
<button class="icon-btn" title="Move down" aria-label="Move down">{ic}</button></form>"#,
                id = esc(id),
                aid = esc(&action.id),
                ic = svg("down"),
            )
        } else {
            String::new()
        },
        id = esc(id),
        aid = esc(&action.id),
        del = svg("delete"),
    );

    match &action.kind {
        ActionKind::Wait { wait_hours } => format!(
            r#"<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{clock}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Wait</span>
<label class="field fixed" style="margin:0;max-width:220px">
<input type="number" name="wait_hours__{aid}" form="automation-form" value="{hours}" min="1" max="{max}" step="1">
<span>Hours · up to 30 days</span></label>
<p class="muted" style="margin:6px 0 0">Currently {label}.</p></div>
<div class="trailing" style="align-items:flex-start">{moves}</div></div>"#,
            clock = svg("history"),
            aid = esc(&action.id),
            hours = wait_hours,
            max = rules::MAX_WAIT_HOURS,
            label = esc(&rules::wait_label(*wait_hours)),
            moves = move_buttons,
        ),
        ActionKind::SendEmail { subject, lexical } => {
            let markdown = markdown_of(lexical);
            let stats = if action.stats.sent > 0 {
                format!(
                    r#"<p class="muted" style="margin:8px 0 0">Sent {sent} · opened {opened}{rate}</p>"#,
                    sent = action.stats.sent,
                    opened = action.stats.opened,
                    rate = action
                        .stats
                        .opened_rate()
                        .map(|r| format!(" ({r}%)"))
                        .unwrap_or_default(),
                )
            } else {
                String::new()
            };
            format!(
                r#"<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{mail}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Send email</span>
<label class="field" style="margin:0"><input name="subject__{aid}" form="automation-form" value="{subject}" placeholder=" " maxlength="300"><span>Subject line</span></label>
<label class="field" style="margin:12px 0 0"><textarea data-automation-markdown name="markdown__{aid}" form="automation-form" rows="6" placeholder=" ">{markdown}</textarea><span>Message · Markdown</span></label>
<input type="hidden" data-automation-lexical name="lexical__{aid}" form="automation-form" value="{lexical}" disabled>
<div style="display:flex;gap:10px;align-items:center;margin-top:10px;flex-wrap:wrap">
<form method="post" action="/ghost/automations/{id}/actions/{aid}/test/" style="display:flex;gap:8px;align-items:center">
<input class="inline-input" type="email" name="email" placeholder="you@example.com" aria-label="Send a test to">
<button class="btn tonal small" type="submit">{send} Send test</button></form></div>
<p class="hint">Save the automation before sending a test — the test uses what is stored, not what is typed above.</p>
{stats}</div>
<div class="trailing" style="align-items:flex-start">{moves}</div></div>"#,
                mail = svg("mail"),
                send = svg("send"),
                id = esc(id),
                aid = esc(&action.id),
                subject = esc(subject),
                markdown = esc(&markdown),
                lexical = esc(lexical),
                stats = stats,
                moves = move_buttons,
            )
        }
    }
}

/// Pull the markdown back out of a stored Lexical body.
///
/// Bodies written here are a single markdown card, so this is usually exact.
/// A body written in Ghost's rich editor has no markdown to recover, so it is
/// shown as rendered HTML instead — editable, and lossless for anything this
/// screen does not understand.
fn markdown_of(lexical: &str) -> String {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(lexical) else {
        return String::new();
    };
    let children = doc.pointer("/root/children").and_then(|c| c.as_array());
    if let Some(children) = children {
        if children.len() == 1 {
            if let Some(markdown) = children[0].get("markdown").and_then(|m| m.as_str()) {
                return markdown.to_string();
            }
        }
    }
    ghost_render::lexical::to_html(lexical)
}

/// The most recent members to enter this automation, and where they got to.
async fn runs_panel(state: &AppState, id: &str) -> String {
    let (runs, total) = match repo::browse_runs(&state.db, id, None, None, 20, 1).await {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    if total == 0 {
        return format!(
            r#"<div class="section">Entries</div>
<div class="card padded rise" style="--i:4;text-align:center">
<p class="muted" style="margin:8px 0">{msg}</p></div>"#,
            msg = "Once someone starts your automation, they'll show here.",
        );
    }

    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let rows: String = runs
        .iter()
        .map(|run| {
            let (chip_class, chip) = match run.status.as_str() {
                "completed" => ("published", "completed"),
                "in_progress" => ("tonal", "in progress"),
                _ if run.failed => ("draft", "failed"),
                _ => ("draft", "exited early"),
            };
            let who = run
                .member_name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| run.member_email.clone());
            let body = if run.member_exists {
                format!(
                    r#"<a class="body" href="/ghost/members/{mid}/"><span class="title">{who}</span><span class="detail">Entered {when}</span></a>"#,
                    mid = esc(run.member_id.as_deref().unwrap_or("")),
                    who = esc(&who),
                    when = esc(&ghost_core::dates::short_date_in_zone(&run.created_at, &timezone)),
                )
            } else {
                // Ghost shows a deleted member as deleted rather than
                // substituting the address they had at the time.
                format!(
                    r#"<div class="body"><span class="title muted">Deleted member</span><span class="detail">Entered {when}</span></div>"#,
                    when = esc(&ghost_core::dates::short_date_in_zone(&run.created_at, &timezone)),
                )
            };
            format!(
                r#"<div class="row"><span class="ic tint-blue">{person}</span>{body}
<div class="trailing"><span class="chip {chip_class}">{chip}</span></div></div>"#,
                person = svg("person"),
                body = body,
                chip_class = chip_class,
                chip = chip,
            )
        })
        .collect();

    format!(
        r#"<div class="section">Entries</div>
<div class="card rows rise" style="--i:4">{rows}</div>
<p class="muted" style="margin:6px 0 0 16px">{total} member{plural} have entered this automation.</p>"#,
        rows = rows,
        total = total,
        plural = if total == 1 { "" } else { "s" },
    )
}

// ---------------------------------------------------------------------------
// saving
// ---------------------------------------------------------------------------

/// Save the whole automation: details, trigger and every step's contents.
///
/// Step fields are posted under `subject__<action id>` / `markdown__<id>` /
/// `wait_hours__<id>` so one form can carry a chain of any length without the
/// server having to trust a submitted ordering.
pub async fn automation_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<Vec<(String, String)>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };

    let single = |key: &str| {
        form.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default()
    };
    let multi = |key: &str| -> Vec<String> {
        form.iter().filter(|(k, _)| k == key).map(|(_, v)| v.clone()).collect()
    };

    let scope = TriggerScope::parse(&single("trigger_tier_scope")).unwrap_or(TriggerScope::Free);
    let tier_ids = multi("trigger_tier_ids");
    if scope == TriggerScope::SelectedPaid && tier_ids.is_empty() {
        return flash_to(&id, "error", "Select at least one tier for a tier-scoped trigger.");
    }

    // Rebuild the chain from the stored order, taking each step's contents
    // from the form. Order is never read from the request — reordering has
    // its own endpoint, so a stale form cannot silently rearrange the chain.
    let ordered = ordered(&existing);
    let actions: Vec<Action> = ordered
        .iter()
        .map(|stored| {
            let kind = match &stored.kind {
                ActionKind::Wait { wait_hours } => ActionKind::Wait {
                    wait_hours: single(&format!("wait_hours__{}", stored.id))
                        .parse()
                        .unwrap_or(*wait_hours),
                },
                ActionKind::SendEmail { subject, lexical } => {
                    let markdown = single(&format!("markdown__{}", stored.id));
                    ActionKind::SendEmail {
                        subject: match form.iter().find(|(k, _)| *k == format!("subject__{}", stored.id)) {
                            Some((_, v)) => v.clone(),
                            None => subject.clone(),
                        },
                        lexical: form.iter().find(|(key,_)|*key==format!("lexical__{}",stored.id))
                            .and_then(|(_,value)|serde_json::from_str::<serde_json::Value>(value).ok().filter(|v|v.get("root").is_some()).map(|_|value.clone()))
                            .or_else(||(markdown==markdown_of(lexical)).then(||lexical.clone()))
                            .or_else(||ghost_render::lexical::wrap_existing(Some(&markdown), None))
                            // An emptied body keeps its shape as an empty
                            // Lexical document rather than becoming invalid.
                            .unwrap_or_else(|| {
                                if markdown.trim().is_empty() {
                                    empty_lexical()
                                } else {
                                    lexical.clone()
                                }
                            }),
                    }
                }
            };
            Action { id: stored.id.clone(), kind }
        })
        .collect();

    let status = Status::parse(&existing.summary.status).unwrap_or(Status::Inactive);
    let edges = rules::edges_for_chain(&actions);
    let name = single("name");
    let description = single("description");

    if let Err(e) = rules::validate(status, &name, &description, &actions, &edges) {
        return flash_to(&id, "error", &e.message());
    }

    let input = repo::AutomationInput {
        name: name.trim(),
        description: description.trim(),
        status,
        trigger_tier_scope: Some(scope),
        trigger_tier_ids: &tier_ids,
        actions: &actions,
        edges: &edges,
    };
    match repo::update(&state.db, &id, &input).await {
        Ok(true) => {
            ghost_api::automations_engine::request_poll();
            flash_to(&id, "ok", "Automation saved.")
        }
        Ok(false) => flash("error", "That automation no longer exists."),
        Err(e) => flash_to(&id, "error", &e.message()),
    }
}

fn empty_lexical() -> String {
    r#"{"root":{"children":[],"direction":null,"format":"","indent":0,"type":"root","version":1}}"#
        .to_string()
}

#[derive(serde::Deserialize)]
pub struct StatusForm {
    status: String,
}

/// Turn an automation on, pause it, or archive it.
///
/// Activation re-validates the whole chain, because that is the moment an
/// unfinished email stops being a private draft and starts being something
/// the next signup receives.
pub async fn automation_set_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<StatusForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(status) = Status::parse(&form.status) else {
        return flash_to(&id, "error", "Unknown automation status.");
    };
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };

    if status == Status::Active {
        let actions: Vec<Action> = ordered(&existing)
            .iter()
            .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
            .collect();
        let edges = rules::edges_for_chain(&actions);
        if let Err(e) = rules::validate(
            status,
            &existing.summary.name,
            &existing.summary.description,
            &actions,
            &edges,
        ) {
            return flash_to(&id, "error", &e.message());
        }
    }

    match repo::set_status(&state.db, &id, status).await {
        Ok(true) => {
            ghost_api::automations_engine::request_poll();
            let was_archived =
                Status::parse(&existing.summary.status) == Some(Status::Archived);
            let message = match status {
                Status::Active => "Automation is on. New members matching the trigger will start it.",
                // landing on paused means one of two things, and the reader
                // should not have to reconstruct which from the old status
                Status::Inactive if was_archived => {
                    "Automation restored. It is paused — turn it on when you are ready."
                }
                Status::Inactive => "Automation turned off. Members partway through have been stopped.",
                Status::Archived => "Automation archived.",
            };
            if status == Status::Archived {
                return flash("ok", message);
            }
            flash_to(&id, "ok", message)
        }
        Ok(false) => flash("error", "That automation no longer exists."),
        Err(e) => flash_to(&id, "error", &e.message()),
    }
}

/// Delete a user-made automation outright.
///
/// The built-in welcome flows are refused: the seeder re-creates them on
/// every boot, so deleting one would only look deleted until the next
/// restart. Pausing or archiving is the durable way to silence those — and
/// archiving is what keeps the run history this delete does not.
pub async fn automation_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };
    if rules::is_welcome_email_slug(existing.summary.slug.as_deref()) {
        return flash(
            "error",
            "The built-in welcome flows cannot be deleted — pause or archive them instead.",
        );
    }
    match repo::delete(&state.db, &id).await {
        Ok(true) => {
            ghost_api::automations_engine::request_poll();
            flash("ok", "Automation deleted.")
        }
        Ok(false) => flash("error", "That automation no longer exists."),
        Err(e) => flash("error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// editing the chain
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct AddActionForm {
    #[serde(rename = "type")]
    action_type: String,
}

/// Append a step to the end of the chain.
pub async fn action_add(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<AddActionForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };
    let mut actions: Vec<Action> = ordered(&existing)
        .iter()
        .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
        .collect();
    if actions.len() >= rules::MAX_ACTIONS {
        return flash_to(
            &id,
            "error",
            &format!("An automation can have at most {} steps.", rules::MAX_ACTIONS),
        );
    }

    let kind = match form.action_type.as_str() {
        // One day is Ghost's own starting point for a new wait.
        "wait" => ActionKind::Wait { wait_hours: 24 },
        "send_email" => ActionKind::SendEmail {
            subject: String::new(),
            lexical: empty_lexical(),
        },
        _ => return flash_to(&id, "error", "Unknown step type."),
    };
    actions.push(Action { id: ghost_core::ids::new_id(), kind });
    save_chain(&state, &id, &existing, actions, "Step added.").await
}

/// Remove a step. Its revisions stay behind for the runs that used them.
pub async fn action_delete(
    State(state): State<AppState>,
    Path((id, action_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };
    let actions: Vec<Action> = ordered(&existing)
        .iter()
        .filter(|s| s.id != action_id)
        .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
        .collect();
    save_chain(&state, &id, &existing, actions, "Step removed.").await
}

#[derive(serde::Deserialize)]
pub struct MoveForm {
    direction: String,
}

/// Move a step one place up or down the chain.
pub async fn action_move(
    State(state): State<AppState>,
    Path((id, action_id)): Path<(String, String)>,
    headers: HeaderMap,
    Form(form): Form<MoveForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(existing)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };
    let mut actions: Vec<Action> = ordered(&existing)
        .iter()
        .map(|s| Action { id: s.id.clone(), kind: s.kind.clone() })
        .collect();
    let Some(index) = actions.iter().position(|a| a.id == action_id) else {
        return flash_to(&id, "error", "That step no longer exists.");
    };
    let target = match form.direction.as_str() {
        "up" if index > 0 => index - 1,
        "down" if index + 1 < actions.len() => index + 1,
        // Already at the end it was heading for; nothing to say about it.
        _ => return Redirect::to(&format!("/ghost/automations/{id}/")).into_response(),
    };
    actions.swap(index, target);
    save_chain(&state, &id, &existing, actions, "Step moved.").await
}

/// Write a rebuilt chain back, re-deriving the edges from its order.
async fn save_chain(
    state: &AppState,
    id: &str,
    existing: &repo::Automation,
    actions: Vec<Action>,
    message: &str,
) -> Response {
    let status = Status::parse(&existing.summary.status).unwrap_or(Status::Inactive);
    let edges = rules::edges_for_chain(&actions);
    if let Err(e) = rules::validate(
        status,
        &existing.summary.name,
        &existing.summary.description,
        &actions,
        &edges,
    ) {
        return flash_to(id, "error", &e.message());
    }
    let scope = existing
        .summary
        .trigger_tier_scope
        .as_deref()
        .and_then(TriggerScope::parse);
    let input = repo::AutomationInput {
        name: &existing.summary.name,
        description: &existing.summary.description,
        status,
        trigger_tier_scope: scope,
        trigger_tier_ids: &existing.trigger_tier_ids,
        actions: &actions,
        edges: &edges,
    };
    match repo::update(&state.db, id, &input).await {
        Ok(true) => {
            ghost_api::automations_engine::request_poll();
            flash_to(id, "ok", message)
        }
        Ok(false) => flash("error", "That automation no longer exists."),
        Err(e) => flash_to(id, "error", &e.message()),
    }
}

#[derive(serde::Deserialize)]
pub struct TestForm {
    email: Option<String>,
}

/// Send one step's email to an address, to see what it looks like.
pub async fn action_test(
    State(state): State<AppState>,
    Path((id, action_id)): Path<(String, String)>,
    headers: HeaderMap,
    Form(form): Form<TestForm>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Ok(Some(automation)) = repo::find(&state.db, &id).await else {
        return flash("error", "That automation no longer exists.");
    };
    let Some(action) = automation.actions.iter().find(|a| a.id == action_id) else {
        return flash_to(&id, "error", "That step no longer exists.");
    };
    let ActionKind::SendEmail { subject, lexical } = &action.kind else {
        return flash_to(&id, "error", "Only email steps can be tested.");
    };
    if subject.trim().is_empty() || rules::is_empty_lexical(lexical) {
        return flash_to(
            &id,
            "error",
            "Add a subject line and a message before this email can be sent.",
        );
    }

    // Default to the signed-in user's own address, which is almost always
    // who a test is for.
    let to = form
        .email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .or_else(|| actor.user.email.clone())
        .unwrap_or_default();
    if !to.contains('@') {
        return flash_to(&id, "error", "Please enter a valid email address.");
    }

    let recipient = ghost_api::automations_engine::MemberSnapshotLike {
        uuid: "preview",
        email: &to,
        name: actor.user.name.as_deref(),
        status: "free",
    };
    match ghost_api::automations_engine::send_automation_email_with_design(&state, &recipient, subject, lexical,action.email_design_setting_id.as_deref())
        .await
    {
        Ok(()) => flash_to(&id, "ok", &format!("Test email sent to {to}.")),
        Err(e) => flash_to(&id, "error", &e),
    }
}

// ---------------------------------------------------------------------------
// redirect helpers
// ---------------------------------------------------------------------------

fn flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/automations/?{encoded}")).into_response()
}

fn flash_to(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/automations/{id}/?{encoded}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_round_trips_through_the_stored_lexical() {
        let source = "# Welcome\n\nThanks for joining **us**.";
        let lexical = ghost_render::lexical::wrap_existing(Some(source), None).unwrap();
        assert_eq!(markdown_of(&lexical), source);
    }

    #[test]
    fn a_rich_body_falls_back_to_its_rendered_html() {
        // a document Ghost's own editor produced, with no markdown to recover
        let rich = r#"{"root":{"children":[{"type":"paragraph","children":[{"type":"text","text":"Hi"}]}]}}"#;
        assert_eq!(markdown_of(rich), "<p>Hi</p>");
        // and junk yields nothing rather than panicking
        assert_eq!(markdown_of("not json"), "");
    }

    #[test]
    fn an_empty_body_is_a_valid_but_empty_document() {
        let empty = empty_lexical();
        assert!(serde_json::from_str::<serde_json::Value>(&empty).is_ok());
        assert!(rules::is_empty_lexical(&empty));
        // which is exactly what blocks activation
        let action = Action {
            id: "a".into(),
            kind: ActionKind::SendEmail { subject: "Hi".into(), lexical: empty },
        };
        assert!(rules::validate(Status::Active, "n", "", &[action.clone()], &[]).is_err());
        assert!(rules::validate(Status::Inactive, "n", "", &[action], &[]).is_ok());
    }

    #[test]
    fn the_view_selector_covers_every_status() {
        let labelled: Vec<&str> = VIEWS.iter().map(|(v, _)| *v).collect();
        assert_eq!(labelled, ["active", "archived", "all"]);
        assert!(VIEWS.iter().all(|(_, label)| !label.is_empty()));
    }
}
