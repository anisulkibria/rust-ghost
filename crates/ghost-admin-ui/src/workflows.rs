//! Tools → Workflow & Editorial Management: the admin screens.
//!
//! The rules are [`ghost_core::workflows`], the queries
//! [`ghost_db::repos::workflows`]. What is here:
//!
//! * **Board** (`/ghost/workflows/`) — every draft in its stage, then what is
//!   scheduled and what went out in the last fortnight. Cards move by drag
//!   and drop, or by the ← → buttons on each card, which are plain forms and
//!   work without the script.
//! * **Calendar** (`/ghost/workflows/calendar/`) — due dates, schedules and
//!   publications on a month grid.
//! * **An entry** (`/ghost/workflows/items/{post_id}/`) — `workflows/item.rs`:
//!   stage, people, due date, priority, checklist, notes, history.
//! * **Settings** — `workflows/settings.rs`: the stages, the approval gate,
//!   the checklist.
//!
//! It also owns [`publish_blocked`], which the editor's save path calls
//! before anything goes live, so the approval gate is enforced where
//! publishing happens rather than only drawn on the board.
//!
//! ## Who sees and does what
//!
//! Reaching the tool is the Permissions matrix's decision, like every tool.
//! Inside it, Editors and above see the whole board and may move, assign
//! and approve anything; Authors and Contributors who have been granted the
//! tool see only their own work (entries they wrote or are assigned) and may
//! move it up to review — `ghost_core::workflows::may_move` draws the line.
//! Every write is origin-checked. Titles, names and notes are escaped where
//! they are shown; ids are validated before they reach a query or a link.

use super::*;
use ghost_core::workflows::{self as rules, Due, Priority, Settings, Who};
use ghost_db::repos::workflows as repo;

mod item;
mod settings;
pub(crate) use item::*;
pub(crate) use settings::*;

const PATH: &str = "/ghost/workflows/";
/// How long a published entry stays in the board's last column.
const PUBLISHED_DAYS: i64 = 14;

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    // Any signed-in staff member: whether this role may reach the tool at
    // all was decided by the Permissions middleware before this ran.
    let actor = require_actor(state, headers).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

pub(crate) fn settings_of(state: &AppState) -> Settings {
    Settings::read(|key| state.settings.get_str(key))
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    crate::tools::tool_flash(path, kind, message)
}

fn today() -> chrono::NaiveDate {
    chrono::Utc::now().date_naive()
}

/// Staff by id: name, and the two letters on their avatar.
pub(crate) struct Staff {
    pub(crate) by_id: HashMap<String, String>,
    pub(crate) list: Vec<(String, String)>,
}

pub(crate) async fn staff(state: &AppState) -> Staff {
    let users = ghost_db::repos::users::list(&state.db, false).await.unwrap_or_default();
    let list: Vec<(String, String)> = users
        .into_iter()
        .filter(|u| u.status.as_deref() != Some("inactive"))
        .map(|u| (u.id.clone(), u.name.clone().unwrap_or_else(|| "Unnamed".into())))
        .collect();
    Staff { by_id: list.iter().cloned().collect(), list }
}

pub(crate) fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}

pub(crate) fn avatar(name: Option<&str>) -> String {
    match name {
        Some(n) => format!(r#"<span class="wf-avatar" title="{t}">{i}</span>"#, t = esc(n), i = esc(&initials(n))),
        None => r#"<span class="wf-avatar wf-avatar-none" title="Unassigned">—</span>"#.to_string(),
    }
}

/// What this actor may do with an entry they may or may not own.
pub(crate) fn who(actor: &Actor, owns: bool) -> Who {
    Who { editor: actor.role >= Role::Editor, admin: actor.role.is_admin(), owns }
}

pub(crate) fn owns(actor: &Actor, e: &repo::Entry, authors: &[String]) -> bool {
    e.assignee_id.as_deref() == Some(actor.user.id.as_str()) || authors.iter().any(|a| a == &actor.user.id)
}

fn tabs(active: &str, editor: bool) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}</div>"#,
        a = tab("board", "Board", PATH),
        b = tab("calendar", "Calendar", "/ghost/workflows/calendar/"),
        c = if editor { tab("settings", "Settings", "/ghost/workflows/settings/") } else { String::new() },
    )
}

/// The column an entry is drawn in: a stage for drafts, its status for the
/// rest.
fn column_of<'a>(s: &'a Settings, e: &'a repo::Entry) -> &'a str {
    match e.status.as_str() {
        "scheduled" => "scheduled",
        "published" | "sent" => "published",
        _ => s.resolve(e.stage.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// the board
// ---------------------------------------------------------------------------

/// The filters, reduced to known values before they are used or linked.
#[derive(Debug, Clone, PartialEq)]
struct Filters {
    view: &'static str,
    kind: &'static str,
    person: Option<String>,
}

fn filters(q: &HashMap<String, String>) -> Filters {
    let view = match q.get("view").map(String::as_str) {
        Some("mine") => "mine",
        Some("unassigned") => "unassigned",
        Some("overdue") => "overdue",
        _ => "all",
    };
    let kind = match q.get("type").map(String::as_str) {
        Some("post") => "post",
        Some("page") => "page",
        _ => "all",
    };
    let person = q.get("person").filter(|p| ghost_core::ids::is_objectid(p)).cloned();
    Filters { view, kind, person }
}

fn filter_href(f: &Filters, view: &str, kind: &str) -> String {
    let mut out = format!("{PATH}?view={view}&type={kind}");
    if let Some(p) = &f.person {
        out.push_str(&format!("&person={p}"));
    }
    out
}

pub(crate) async fn board_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = settings_of(&state);
    let f = filters(&q);
    let editor = actor.role >= Role::Editor;
    let since = ghost_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(PUBLISHED_DAYS)));
    let entries = repo::board(&state.db, &since).await.unwrap_or_default();
    let mut authors: HashMap<String, Vec<String>> = HashMap::new();
    for (post, author) in repo::authors(&state.db).await.unwrap_or_default() {
        authors.entry(post).or_default().push(author);
    }
    let notes: HashMap<String, i64> = repo::note_counts(&state.db).await.into_iter().collect();
    let ticks: HashMap<String, i64> = repo::check_counts(&state.db).await.into_iter().collect();
    let people = staff(&state).await;
    let today = today();
    let none: Vec<String> = Vec::new();

    let visible: Vec<&repo::Entry> = entries
        .iter()
        .filter(|e| editor || owns(&actor, e, authors.get(&e.post_id).unwrap_or(&none)))
        .filter(|e| f.kind == "all" || e.kind == f.kind)
        .filter(|e| f.person.as_deref().is_none_or(|p| e.assignee_id.as_deref() == Some(p)))
        .filter(|e| match f.view {
            "mine" => owns(&actor, e, authors.get(&e.post_id).unwrap_or(&none)),
            "unassigned" => e.assignee_id.is_none() && !matches!(e.status.as_str(), "published" | "sent"),
            "overdue" => e.due_on.as_deref().and_then(|d| rules::due(d, today)) == Some(Due::Overdue)
                && !matches!(e.status.as_str(), "published" | "sent"),
            _ => true,
        })
        .collect();

    let overdue = visible
        .iter()
        .filter(|e| !matches!(e.status.as_str(), "published" | "sent"))
        .filter(|e| e.due_on.as_deref().and_then(|d| rules::due(d, today)) == Some(Due::Overdue))
        .count();
    let mine_count = entries
        .iter()
        .filter(|e| !matches!(e.status.as_str(), "published" | "sent"))
        .filter(|e| owns(&actor, e, authors.get(&e.post_id).unwrap_or(&none)))
        .count();

    let mut columns: Vec<(String, String, String, i64, bool)> = s
        .stages
        .iter()
        .map(|st| (st.key.clone(), st.label.clone(), st.color.clone(), st.limit, true))
        .collect();
    columns.push(("scheduled".into(), "Scheduled".into(), "purple".into(), 0, false));
    columns.push(("published".into(), format!("Published · {PUBLISHED_DAYS} days"), "green".into(), 0, false));

    let board: String = columns
        .iter()
        .map(|(key, label, color, limit, movable)| {
            let mut in_col: Vec<&&repo::Entry> = visible.iter().filter(|e| column_of(&s, e) == key).collect();
            // Most urgent first: priority, then the nearest due date.
            in_col.sort_by(|a, b| {
                let pa = Priority::from_key(a.priority.as_deref().unwrap_or(""));
                let pb = Priority::from_key(b.priority.as_deref().unwrap_or(""));
                pb.cmp(&pa).then_with(|| {
                    a.due_on.as_deref().unwrap_or("9999").cmp(b.due_on.as_deref().unwrap_or("9999"))
                })
            });
            let over = *limit > 0 && in_col.len() as i64 > *limit;
            let cards: String = in_col
                .iter()
                .map(|e| card(&s, e, &people, &notes, &ticks, *movable, today))
                .collect();
            format!(
                r#"<section class="wf-col{over}" data-stage="{key}" data-movable="{movable}" aria-label="{label}">
<header class="wf-col-head"><span class="wf-dot tint-{color}"></span><h2>{label}</h2><span class="wf-count">{count}{lim}</span></header>
<div class="wf-cards" data-drop>{cards}{empty}</div></section>"#,
                over = if over { " wf-over" } else { "" },
                key = esc(key),
                label = esc(label),
                count = in_col.len(),
                lim = if *limit > 0 { format!(" / {limit}") } else { String::new() },
                empty = if in_col.is_empty() { r#"<p class="wf-empty">Nothing here</p>"# } else { "" },
            )
        })
        .collect();

    let chip = |view: &str, label: &str, n: Option<usize>| {
        format!(
            r#"<a class="wf-chip{on}" href="{href}">{label}{n}</a>"#,
            on = if f.view == view { " on" } else { "" },
            href = esc(&filter_href(&f, view, f.kind)),
            label = esc(label),
            n = n.map(|n| format!(r#"<b>{n}</b>"#)).unwrap_or_default(),
        )
    };
    let kind_chip = |kind: &str, label: &str| {
        format!(
            r#"<a class="wf-chip{on}" href="{href}">{label}</a>"#,
            on = if f.kind == kind { " on" } else { "" },
            href = esc(&filter_href(&f, f.view, kind)),
            label = esc(label),
        )
    };
    let person_opts: String = std::iter::once(format!(r#"<option value="">Anyone</option>"#))
        .chain(people.list.iter().map(|(id, name)| {
            format!(
                r#"<option value="{id}"{sel}>{name}</option>"#,
                id = esc(id),
                sel = if f.person.as_deref() == Some(id.as_str()) { " selected" } else { "" },
                name = esc(name),
            )
        }))
        .collect();

    let idea = if editor || actor.role >= Role::Contributor {
        format!(
            r#"<form class="wf-idea" method="post" action="{PATH}ideas/">
<input name="title" required maxlength="{max}" placeholder="New idea — a working title" aria-label="New idea">
<button class="btn" type="submit">{add} Add to {first}</button></form>"#,
            max = rules::MAX_IDEA,
            add = svg("add"),
            first = esc(s.stages.first().map(|st| st.label.as_str()).unwrap_or("the board")),
        )
    } else {
        String::new()
    };

    page_for_role(
        actor.role,
        "Editorial board",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="wf-bar rise" style="--i:2">
<div class="wf-chips">{all}{mine}{unassigned}{overdue_chip}<span class="wf-sep"></span>{k_all}{k_post}{k_page}</div>
{person}
</div>
{idea}
<div class="wf-board rise" style="--i:3" id="wf-board">{board}</div>
<p class="muted rise" style="--i:4;margin:14px 4px 0">{note}</p>
<script src="{assets}/workflows-board.js?v={hash}" defer></script>"#,
            head = crate::tools::tool_head("workflows", None, &format!(
                r#"<a class="btn text" href="/ghost/workflows/calendar/">{cal} Calendar</a><a class="btn tonal" href="/ghost/posts/new/">{add} New post</a>"#,
                cal = svg("calendar"),
                add = svg("add"),
            )),
            tabs = tabs("board", editor),
            banner = flash_banner(&q),
            all = chip("all", "Everything", None),
            mine = chip("mine", "Mine", Some(mine_count)),
            unassigned = chip("unassigned", "Unassigned", None),
            overdue_chip = chip("overdue", "Overdue", (overdue > 0).then_some(overdue)),
            k_all = kind_chip("all", "Posts & pages"),
            k_post = kind_chip("post", "Posts"),
            k_page = kind_chip("page", "Pages"),
            person = if editor {
                format!(
                    r#"<form class="wf-person" method="get" action="{PATH}"><input type="hidden" name="view" value="{view}"><input type="hidden" name="type" value="{kind}">
<label class="field fixed" style="margin:0"><select name="person" onchange="this.form.submit()">{person_opts}</select><span>Assigned to</span>{caret}</label>
<noscript><button class="btn text" type="submit">Filter</button></noscript></form>"#,
                    view = f.view,
                    kind = f.kind,
                    caret = SELECT_CARET,
                )
            } else {
                String::new()
            },
            idea = idea,
            note = if editor {
                "Drag a card to move it, or use its arrows. Scheduled and published columns follow each post's own status."
            } else {
                "You see your own work. Move it to review when it is ready; an editor approves it."
            },
            assets = ASSETS,
            hash = board_script_hash(),
        ),
        true,
    )
    .into_response()
}

fn board_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/workflows-board.js")));
    &HASH
}

fn card(
    s: &Settings,
    e: &repo::Entry,
    people: &Staff,
    notes: &HashMap<String, i64>,
    ticks: &HashMap<String, i64>,
    movable: bool,
    today: chrono::NaiveDate,
) -> String {
    let title = e.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("(Untitled)");
    let priority = Priority::from_key(e.priority.as_deref().unwrap_or(""));
    let due = e
        .due_on
        .as_deref()
        .filter(|_| movable)
        .and_then(|d| rules::due(d, today).map(|state| (d, state)))
        .map(|(d, state)| {
            format!(
                r#"<span class="wf-due wf-due-{cls}" title="{label}">{ic}{date}</span>"#,
                cls = match state {
                    Due::Overdue => "over",
                    Due::Today => "today",
                    Due::Soon => "soon",
                    Due::Later => "later",
                },
                label = esc(state.label()),
                ic = svg("calendar"),
                date = esc(&short_date(d)),
            )
        })
        .unwrap_or_default();
    let date_line = match e.status.as_str() {
        "scheduled" | "published" | "sent" => e
            .published_at
            .as_deref()
            .map(|d| format!(r#"<span class="wf-when">{}</span>"#, esc(&short_date(d))))
            .unwrap_or_default(),
        _ => String::new(),
    };
    let n_notes = notes.get(&e.post_id).copied().unwrap_or(0);
    let n_ticks = ticks.get(&e.post_id).copied().unwrap_or(0);
    let arrows = if movable {
        let i = s.index(column_of(s, e)).unwrap_or(0);
        let mv = |to: Option<&rules::Stage>, label: &str, icon_rot: &str| match to {
            Some(st) => format!(
                r#"<form method="post" action="{PATH}items/{id}/move/"><input type="hidden" name="to" value="{to}"><button class="wf-arrow" type="submit" title="{label} {name}" aria-label="{label} {name}"{rot}>{chev}</button></form>"#,
                id = esc(&e.post_id),
                to = esc(&st.key),
                label = label,
                name = esc(&st.label),
                rot = icon_rot,
                chev = svg("chevron"),
            ),
            None => r#"<span class="wf-arrow-gap"></span>"#.to_string(),
        };
        format!(
            r#"<div class="wf-move">{back}{fwd}</div>"#,
            back = mv(i.checked_sub(1).and_then(|j| s.stages.get(j)), "Move back to", r#" style="transform:rotate(180deg)""#),
            fwd = mv(s.stages.get(i + 1), "Move to", ""),
        )
    } else {
        String::new()
    };
    format!(
        r#"<article class="wf-card wf-p-{prio}" {drag} data-id="{id}">
<a class="wf-title" href="{PATH}items/{id}/">{title}</a>
<div class="wf-meta">{kind}{prio_chip}{due}{date_line}</div>
<div class="wf-foot">{avatar}<span class="wf-stats">{notes}{ticks}</span>{arrows}</div>
</article>"#,
        prio = priority.key(),
        drag = if movable { r#"draggable="true""# } else { "" },
        id = esc(&e.post_id),
        title = esc(title),
        kind = if e.kind == "page" { r#"<span class="chip tonal">Page</span>"# } else { "" },
        prio_chip = match priority {
            Priority::Urgent | Priority::High => format!(r#"<span class="wf-prio tint-{}">{}</span>"#, priority.tint(), priority.label()),
            _ => String::new(),
        },
        avatar = avatar(e.assignee_id.as_ref().and_then(|id| people.by_id.get(id)).map(String::as_str)),
        notes = if n_notes > 0 { format!(r#"<span title="Notes">{} {n_notes}</span>"#, svg("comment")) } else { String::new() },
        ticks = if n_ticks > 0 { format!(r#"<span title="Checklist ticks">{} {n_ticks}</span>"#, svg("check")) } else { String::new() },
    )
}

/// `2026-11-21…` → `21 Nov`.
pub(crate) fn short_date(d: &str) -> String {
    chrono::NaiveDate::parse_from_str(d.get(..10).unwrap_or(""), "%Y-%m-%d")
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|_| d.to_string())
}

// ---------------------------------------------------------------------------
// moving, and new ideas
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct MoveForm {
    to: String,
    /// `item` when the entry's own page asked, so the redirect goes back
    /// there rather than to the board.
    back: String,
}

fn wants_json(headers: &HeaderMap) -> bool {
    headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("application/json"))
}

/// Move one card. The board's script asks for JSON; the arrows get a
/// redirect back to the board.
pub(crate) async fn move_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<MoveForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let back = if form.back == "item" && ghost_core::ids::is_objectid(&id) { format!("{PATH}items/{id}/") } else { PATH.to_string() };
    let reply = |ok: bool, message: &str| -> Response {
        if wants_json(&headers) {
            let status = if ok { StatusCode::OK } else { StatusCode::UNPROCESSABLE_ENTITY };
            (status, axum::Json(serde_json::json!({ "ok": ok, "message": message }))).into_response()
        } else {
            flash_to(&back, if ok { "ok" } else { "error" }, message)
        }
    };
    if !ghost_core::ids::is_objectid(&id) {
        return reply(false, "That entry could not be found.");
    }
    let Ok(Some(e)) = repo::entry(&state.db, &id).await else {
        return reply(false, "That entry could not be found.");
    };
    if !matches!(e.status.as_str(), "draft") {
        return reply(false, "Scheduled and published entries follow their own status. Unschedule or unpublish it in the editor first.");
    }
    let s = settings_of(&state);
    let authors = repo::authors_of(&state.db, &id).await;
    let from = s.resolve(e.stage.as_deref()).to_string();
    if let Err(message) = rules::may_move(&s, who(&actor, owns(&actor, &e, &authors)), &from, &form.to) {
        return reply(false, message);
    }
    if from == form.to {
        return reply(true, "Already there.");
    }
    let input = repo::ItemInput {
        stage: form.to.clone(),
        assignee_id: e.assignee_id.clone(),
        reviewer_id: e.reviewer_id.clone(),
        due_on: e.due_on.clone(),
        priority: e.priority.clone().unwrap_or_else(|| "normal".into()),
    };
    if let Err(err) = repo::save_item(&state.db, &id, &input).await {
        tracing::error!("workflows: moving {id} failed: {}", err.message());
        return reply(false, "That could not be saved.");
    }
    let label = |k: &str| s.stage(k).map(|st| st.label.clone()).unwrap_or_else(|| k.to_string());
    let detail = format!("{} → {}", label(&from), label(&form.to));
    repo::record(&state.db, &id, &actor.user.id, "moved", &detail).await;
    reply(true, &format!("Moved to {}.", label(&form.to)))
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct IdeaForm {
    title: String,
}

pub(crate) async fn create_idea(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<IdeaForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let title = rules::clean(&form.title, rules::MAX_IDEA);
    if title.is_empty() {
        return flash_to(PATH, "error", "Give the idea a working title.");
    }
    let s = settings_of(&state);
    let first = s.stages.first().map(|st| st.key.clone()).unwrap_or_default();
    // A writer's idea is assigned to them; an editor's is left open for
    // whoever picks it up.
    let assignee = (actor.role < Role::Editor).then(|| actor.user.id.clone());
    let item = repo::ItemInput { stage: first, assignee_id: assignee, reviewer_id: None, due_on: None, priority: "normal".into() };
    match repo::create_idea(&state.db, &title, &actor.user.id, &item).await {
        Ok(id) => {
            repo::record(&state.db, &id, &actor.user.id, "created", &format!("Idea “{title}”")).await;
            flash_to(PATH, "ok", &format!("“{title}” added."))
        }
        Err(e) => {
            tracing::error!("workflows: creating an idea failed: {}", e.message());
            flash_to(PATH, "error", "The idea could not be saved.")
        }
    }
}

// ---------------------------------------------------------------------------
// the calendar
// ---------------------------------------------------------------------------

pub(crate) async fn calendar_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = settings_of(&state);
    let editor = actor.role >= Role::Editor;
    let today = today();
    let first = rules::month_of(q.get("m").map(String::as_str), today);
    let weeks = rules::month_grid(first);
    let from = weeks.first().map(|w| w[0]).unwrap_or(first).format("%Y-%m-%d").to_string();
    let to = weeks.last().map(|w| w[6]).unwrap_or(first).format("%Y-%m-%d").to_string();
    let entries = repo::dated(&state.db, &from, &to).await.unwrap_or_default();
    let mut authors: HashMap<String, Vec<String>> = HashMap::new();
    for (post, author) in repo::authors(&state.db).await.unwrap_or_default() {
        authors.entry(post).or_default().push(author);
    }
    let none: Vec<String> = Vec::new();
    let visible: Vec<&repo::Entry> = entries
        .iter()
        .filter(|e| editor || owns(&actor, e, authors.get(&e.post_id).unwrap_or(&none)))
        .collect();

    let pill = |e: &repo::Entry, kind: &str| {
        let title = e.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("(Untitled)");
        let color = match kind {
            "published" => "green".to_string(),
            "scheduled" => "purple".to_string(),
            _ => s.stage(s.resolve(e.stage.as_deref())).map(|st| st.color.clone()).unwrap_or_else(|| "slate".into()),
        };
        format!(
            r#"<a class="wf-pill wf-pill-{kind} tint-{color}" href="{PATH}items/{id}/" title="{label}: {t}">{t}</a>"#,
            id = esc(&e.post_id),
            label = match kind {
                "published" => "Published",
                "scheduled" => "Scheduled",
                _ => "Due",
            },
            t = esc(title),
        )
    };
    let cells: String = weeks
        .iter()
        .map(|week| {
            let days: String = week
                .iter()
                .map(|d| {
                    let key = d.format("%Y-%m-%d").to_string();
                    let mut pills = String::new();
                    for e in &visible {
                        let pub_day = e.published_at.as_deref().and_then(|p| p.get(..10));
                        match e.status.as_str() {
                            "published" | "sent" if pub_day == Some(key.as_str()) => pills.push_str(&pill(e, "published")),
                            "scheduled" if pub_day == Some(key.as_str()) => pills.push_str(&pill(e, "scheduled")),
                            "draft" if e.due_on.as_deref() == Some(key.as_str()) => pills.push_str(&pill(e, "due")),
                            _ => {}
                        }
                    }
                    format!(
                        r#"<div class="wf-day{out}{now}"><span class="wf-daynum">{n}</span>{pills}</div>"#,
                        out = if d.format("%m").to_string() != first.format("%m").to_string() { " wf-out" } else { "" },
                        now = if *d == today { " wf-today" } else { "" },
                        n = d.format("%-d"),
                    )
                })
                .collect();
            format!(r#"<div class="wf-week">{days}</div>"#)
        })
        .collect();
    let month_link = |by: i32| rules::shift_month(first, by).format("%Y-%m").to_string();

    page_for_role(
        actor.role,
        "Editorial calendar",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}
<div class="wf-cal-bar rise" style="--i:2">
<a class="icon-btn" style="transform:rotate(180deg)" href="{PATH}calendar/?m={prev}" title="Previous month">{chev}</a>
<h2>{month}</h2>
<a class="icon-btn" href="{PATH}calendar/?m={next}" title="Next month">{chev}</a>
<a class="btn text" href="{PATH}calendar/">Today</a>
<span class="wf-legend"><i class="wf-pill-due"></i>Due<i class="wf-pill-scheduled tint-purple"></i>Scheduled<i class="wf-pill-published tint-green"></i>Published</span>
</div>
<div class="card wf-cal rise" style="--i:3">
<div class="wf-week wf-weekdays"><span>Mon</span><span>Tue</span><span>Wed</span><span>Thu</span><span>Fri</span><span>Sat</span><span>Sun</span></div>
{cells}
</div>"#,
            head = crate::tools::tool_head("workflows", None, ""),
            tabs = tabs("calendar", editor),
            prev = month_link(-1),
            next = month_link(1),
            chev = svg("chevron"),
            month = esc(&first.format("%B %Y").to_string()),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// the approval gate
// ---------------------------------------------------------------------------

/// Why this actor may not publish this entry now, or `None` when they may.
/// Called by the editor's save path before a post goes live; `post_id` is
/// `None` for a post being created and published in one step, which has no
/// stage and so is never approved.
pub(crate) async fn publish_blocked(state: &AppState, role: Role, post_id: Option<&str>) -> Option<String> {
    let s = settings_of(state);
    if !s.require_approval {
        return None;
    }
    let stage = match post_id {
        Some(id) => repo::entry(&state.db, id).await.ok().flatten().and_then(|e| e.stage),
        None => None,
    };
    rules::publish_gate(&s, stage.as_deref(), role.is_admin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forged_filters_collapse_to_defaults() {
        let q: HashMap<String, String> = [("view", "\"><script>"), ("type", "x"), ("person", "../../etc")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(filters(&q), Filters { view: "all", kind: "all", person: None });
    }

    #[test]
    fn initials_are_two_letters_and_never_empty() {
        assert_eq!(initials("ada king lovelace"), "AK");
        assert_eq!(initials("   "), "?");
    }
}
