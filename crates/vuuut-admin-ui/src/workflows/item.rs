//! One entry on the board: where it is, who has it, what is left to check,
//! what people said about it, and what happened to it.
//!
//! Editors change everything here. A writer who owns the entry can move it
//! (within `may_move`), tick its checklist and add notes; the assignment
//! fields are shown to them but not offered.

use super::*;

const ITEMS: &str = "/ghost/workflows/items/";

/// Load an entry this actor may see, or the response that says why not.
async fn load(state: &AppState, actor: &Actor, id: &str) -> Result<(repo::Entry, Vec<String>), Response> {
    let missing = || not_found_page_for(actor.role, &actor.user, "That entry could not be found.");
    if !vuuut_core::ids::is_objectid(id) {
        return Err(missing());
    }
    let Ok(Some(e)) = repo::entry(&state.db, id).await else { return Err(missing()) };
    let authors = repo::authors_of(&state.db, id).await;
    if actor.role < Role::Editor && !owns(actor, &e, &authors) {
        // Not theirs: answered exactly as a missing entry, so the page does
        // not confirm what exists.
        return Err(missing());
    }
    Ok((e, authors))
}

pub(crate) async fn wf_item_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (e, authors) = match load(&state, &actor, &id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let s = settings_of(&state);
    let w = who(&actor, owns(&actor, &e, &authors));
    let people = staff(&state).await;
    let name_of = |id: Option<&str>| id.and_then(|i| people.by_id.get(i)).cloned();
    let title = e.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("(Untitled)");
    let draft = e.status == "draft";
    let current = column_of(&s, &e).to_string();
    let today = today();

    // ---- the pipeline, as a stepper ----
    let mut steps: Vec<(String, String)> = s.stages.iter().map(|st| (st.key.clone(), st.label.clone())).collect();
    steps.push(("scheduled".into(), "Scheduled".into()));
    steps.push(("published".into(), "Published".into()));
    let at = steps.iter().position(|(k, _)| *k == current).unwrap_or(0);
    let stepper: String = steps
        .iter()
        .enumerate()
        .map(|(i, (key, label))| {
            let state_cls = if i < at { "done" } else if i == at { "now" } else { "todo" };
            let can = draft && i < s.stages.len() && i != at && rules::may_move(&s, w, &current, key).is_ok();
            let inner = format!(r#"<span class="wf-step-dot">{}</span><span class="wf-step-label">{}</span>"#, i + 1, esc(label));
            if can {
                format!(
                    r#"<form method="post" action="{ITEMS}{id}/move/" class="wf-step wf-step-{state_cls}"><input type="hidden" name="to" value="{key}"><input type="hidden" name="back" value="item">
<button type="submit" title="Move to {label}">{inner}</button></form>"#,
                    id = esc(&e.post_id),
                    key = esc(key),
                    label = esc(label),
                )
            } else {
                format!(r#"<div class="wf-step wf-step-{state_cls}">{inner}</div>"#)
            }
        })
        .collect();

    // ---- assignment ----
    let opt_people = |selected: Option<&str>| -> String {
        std::iter::once(format!(r#"<option value="">Nobody</option>"#))
            .chain(people.list.iter().map(|(pid, name)| {
                format!(
                    r#"<option value="{pid}"{sel}>{name}</option>"#,
                    pid = esc(pid),
                    sel = if selected == Some(pid.as_str()) { " selected" } else { "" },
                    name = esc(name),
                )
            }))
            .collect()
    };
    let priority = Priority::from_key(e.priority.as_deref().unwrap_or(""));
    let prio_opts: String = Priority::ALL
        .iter()
        .map(|p| format!(r#"<option value="{}"{}>{}</option>"#, p.key(), if *p == priority { " selected" } else { "" }, p.label()))
        .collect();
    let stage_opts: String = s
        .stages
        .iter()
        .map(|st| format!(r#"<option value="{k}"{sel}>{l}</option>"#, k = esc(&st.key), sel = if st.key == current { " selected" } else { "" }, l = esc(&st.label)))
        .collect();
    let disabled = if rules::may_assign(w) { "" } else { " disabled" };
    let due_state = e.due_on.as_deref().and_then(|d| rules::due(d, today));
    let assign = format!(
        r#"<form class="card padded wf-assign" method="post" action="{ITEMS}{id}/">
{stage}
<label class="field fixed"><select name="assignee_id"{disabled}>{assignee}</select><span>Assigned to</span>{caret}</label>
<label class="field fixed"><select name="reviewer_id"{disabled}>{reviewer}</select><span>Reviewer</span>{caret}</label>
<div class="wf-two">
<label class="field"><input type="date" name="due_on" value="{due}"{disabled}><span>Due</span></label>
<label class="field fixed"><select name="priority"{disabled}>{prio_opts}</select><span>Priority</span>{caret}</label>
</div>
{due_note}
{save}
</form>"#,
        id = esc(&e.post_id),
        stage = if draft && w.editor {
            format!(r#"<label class="field fixed"><select name="stage">{stage_opts}</select><span>Stage</span>{caret}</label>"#, caret = SELECT_CARET)
        } else {
            String::new()
        },
        assignee = opt_people(e.assignee_id.as_deref()),
        reviewer = opt_people(e.reviewer_id.as_deref()),
        due = esc(e.due_on.as_deref().unwrap_or("")),
        caret = SELECT_CARET,
        due_note = match due_state {
            Some(d @ (Due::Overdue | Due::Today | Due::Soon)) if draft => format!(r#"<p class="wf-due-note wf-due-{c}">{l}</p>"#, c = match d { Due::Overdue => "over", Due::Today => "today", _ => "soon" }, l = esc(d.label())),
            _ => String::new(),
        },
        save = if rules::may_assign(w) {
            format!(r#"<div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{} Save</button></div>"#, svg("check"))
        } else {
            r#"<p class="muted" style="margin:0;font-size:12.5px">An editor assigns work and sets due dates.</p>"#.to_string()
        },
    );

    // ---- checklist ----
    let facts = repo::facts(&state.db, &e.post_id).await;
    let done = repo::checks(&state.db, &e.post_id).await;
    let auto: String = rules::auto_checks(&facts)
        .iter()
        .map(|(label, ok)| {
            format!(
                r#"<li class="wf-check wf-auto{on}"><span class="wf-tick">{ic}</span>{label}<span class="wf-auto-tag">automatic</span></li>"#,
                on = if *ok { " on" } else { "" },
                ic = if *ok { svg("check") } else { String::new() },
                label = esc(label),
            )
        })
        .collect();
    let manual: String = s
        .checklist
        .iter()
        .map(|label| {
            let key = rules::check_key(label);
            let on = done.contains(&key);
            format!(
                r#"<li class="wf-check{cls}"><form method="post" action="{ITEMS}{id}/check/"><input type="hidden" name="key" value="{key}"><input type="hidden" name="done" value="{next}">
<button type="submit" class="wf-tick-btn" aria-pressed="{on}" aria-label="{label}"><span class="wf-tick">{ic}</span>{label}</button></form></li>"#,
                cls = if on { " on" } else { "" },
                id = esc(&e.post_id),
                key = esc(&key),
                next = if on { "0" } else { "1" },
                ic = if on { svg("check") } else { String::new() },
                label = esc(label),
            )
        })
        .collect();

    // ---- notes ----
    let notes = repo::notes(&state.db, &e.post_id).await;
    let tz = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let when = |d: &str| format!("{} {}", vuuut_core::dates::short_date_in_zone(d, &tz), vuuut_core::dates::clock_in_zone(d, &tz));
    let note_rows: String = notes
        .iter()
        .map(|n| {
            let author = people.by_id.get(&n.author_id).cloned().unwrap_or_else(|| "Former staff".into());
            let mine = n.author_id == actor.user.id;
            format!(
                r#"<div class="wf-note">{av}<div class="wf-note-body"><div class="wf-note-head"><b>{author}</b><span>{when}</span>{del}</div><p>{body}</p></div></div>"#,
                av = avatar(Some(&author)),
                author = esc(&author),
                when = esc(&when(&n.created_at)),
                del = if mine || w.editor {
                    format!(
                        r#"<form method="post" action="{ITEMS}{id}/notes/{nid}/delete/" data-confirm="Delete note|It is removed for everybody."><button class="wf-note-del" type="submit" aria-label="Delete note">{x}</button></form>"#,
                        id = esc(&e.post_id),
                        nid = esc(&n.id),
                        x = svg("close"),
                    )
                } else {
                    String::new()
                },
                body = esc(&n.body).replace('\n', "<br>"),
            )
        })
        .collect();

    // ---- history ----
    let events = repo::events(&state.db, &e.post_id, 50).await;
    let history: String = events
        .iter()
        .map(|ev| {
            let actor_name = people.by_id.get(&ev.actor_id).cloned().unwrap_or_else(|| "Former staff".into());
            format!(
                r#"<li><span class="wf-ev-ic tint-{tint}">{ic}</span><div><b>{actor}</b> {what}<span class="wf-ev-when">{when}</span></div></li>"#,
                tint = match ev.kind.as_str() {
                    "moved" => "blue",
                    "assigned" => "purple",
                    "created" => "green",
                    "note" => "amber",
                    _ => "slate",
                },
                ic = svg(match ev.kind.as_str() {
                    "moved" => "trending",
                    "assigned" => "person",
                    "created" => "add",
                    "note" => "comment",
                    "checked" => "check",
                    "due" => "calendar",
                    _ => "edit",
                }),
                actor = esc(&actor_name),
                what = esc(&match ev.kind.as_str() {
                    "moved" => format!("moved it: {}", ev.detail),
                    "created" => format!("created it — {}", ev.detail),
                    "note" => "added a note".to_string(),
                    _ => ev.detail.clone(),
                }),
                when = esc(&when(&ev.created_at)),
            )
        })
        .collect();

    let editor_href = format!("/ghost/editor/{}/{}/", if e.kind == "page" { "page" } else { "post" }, esc(&e.post_id));
    let gate = if draft {
        rules::publish_gate(&s, e.stage.as_deref(), actor.role.is_admin())
            .map(|m| format!(r#"<div class="card padded wf-gate">{}<p>{}</p></div>"#, svg("lock"), esc(&m)))
            .unwrap_or_default()
    } else {
        String::new()
    };

    page_for_role(
        actor.role,
        title,
        "tools",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="{PATH}" title="Back to the board">{back}</a>
<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions"><a class="btn" href="{editor_href}">{edit} Open in editor</a></div></header>
{banner}
<div class="wf-stepper rise" style="--i:1">{stepper}</div>
{gate}
<div class="wf-item">
<div>
<div class="section">Notes</div>
<div class="card padded rise" style="--i:2">
<div class="wf-notes">{note_rows}{no_notes}</div>
<form class="wf-note-form" method="post" action="{ITEMS}{id}/notes/">
<label class="field" style="margin:0"><textarea name="body" rows="3" required maxlength="{max_note}" placeholder=" "></textarea><span>Add a note — only staff see it</span></label>
<div style="display:flex;justify-content:flex-end;margin-top:8px"><button class="btn tonal" type="submit">{comment} Add note</button></div>
</form>
</div>
<div class="section">History</div>
<div class="card padded rise" style="--i:3"><ul class="wf-history">{history}{no_history}</ul></div>
</div>
<aside>
<div class="section">Assignment</div>
<div class="rise" style="--i:2">{assign}</div>
<div class="section">Checklist</div>
<div class="card padded rise" style="--i:3"><ul class="wf-checks">{auto}{manual}</ul>
{edit_list}</div>
</aside>
</div>"#,
            back = svg("chevron"),
            title = esc(title),
            sub = esc(&format!(
                "{} · {} · by {}",
                if e.kind == "page" { "Page" } else { "Post" },
                match e.status.as_str() {
                    "scheduled" => format!("scheduled for {}", e.published_at.as_deref().map(short_date).unwrap_or_default()),
                    "published" | "sent" => format!("published {}", e.published_at.as_deref().map(short_date).unwrap_or_default()),
                    _ => s.stage(&current).map(|st| st.label.clone()).unwrap_or_default(),
                },
                authors.iter().filter_map(|a| name_of(Some(a))).collect::<Vec<_>>().join(", "),
            )),
            edit = svg("edit"),
            banner = flash_banner(&q),
            id = esc(&e.post_id),
            no_notes = if notes.is_empty() { r#"<p class="muted" style="margin:0 0 12px">No notes yet. Questions, edits and decisions go here, beside the work.</p>"# } else { "" },
            max_note = rules::MAX_NOTE,
            comment = svg("comment"),
            no_history = if events.is_empty() { r#"<li class="muted">Nothing recorded yet.</li>"# } else { "" },
            edit_list = if actor.role.is_admin() {
                r#"<p class="muted" style="margin:10px 0 0;font-size:12.5px"><a href="/ghost/workflows/settings/">Change the checklist</a></p>"#
            } else {
                ""
            },
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ItemForm {
    stage: String,
    assignee_id: String,
    reviewer_id: String,
    due_on: String,
    priority: String,
}

pub(crate) async fn wf_item_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<ItemForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (e, authors) = match load(&state, &actor, &id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let back = format!("{ITEMS}{id}/");
    let w = who(&actor, owns(&actor, &e, &authors));
    if !rules::may_assign(w) {
        return flash_to(&back, "error", "An editor assigns work and sets due dates.");
    }
    let s = settings_of(&state);
    let people = staff(&state).await;
    // A person is one of the staff, or nobody — never an id the form made up.
    let person = |raw: &str| Some(raw.trim().to_string()).filter(|p| people.by_id.contains_key(p));
    let from = s.resolve(e.stage.as_deref()).to_string();
    let stage = if e.status == "draft" && s.stage(&form.stage).is_some() { form.stage.clone() } else { from.clone() };
    let input = repo::ItemInput {
        stage: stage.clone(),
        assignee_id: person(&form.assignee_id),
        reviewer_id: person(&form.reviewer_id),
        due_on: rules::parse_date(&form.due_on),
        priority: Priority::from_key(&form.priority).key().to_string(),
    };
    if let Err(err) = repo::save_item(&state.db, &id, &input).await {
        tracing::error!("workflows: saving {id} failed: {}", err.message());
        return flash_to(&back, "error", "That could not be saved.");
    }
    // One history line per thing that changed, in words.
    let name = |p: &Option<String>| p.as_ref().and_then(|i| people.by_id.get(i)).cloned().unwrap_or_else(|| "nobody".into());
    let label = |k: &str| s.stage(k).map(|st| st.label.clone()).unwrap_or_else(|| k.to_string());
    if stage != from {
        repo::record(&state.db, &id, &actor.user.id, "moved", &format!("{} → {}", label(&from), label(&stage))).await;
    }
    if input.assignee_id != e.assignee_id {
        repo::record(&state.db, &id, &actor.user.id, "assigned", &format!("assigned it to {}", name(&input.assignee_id))).await;
    }
    if input.reviewer_id != e.reviewer_id {
        repo::record(&state.db, &id, &actor.user.id, "assigned", &format!("asked {} to review", name(&input.reviewer_id))).await;
    }
    if input.due_on != e.due_on {
        let what = match &input.due_on {
            Some(d) => format!("set it due {}", short_date(d)),
            None => "cleared the due date".into(),
        };
        repo::record(&state.db, &id, &actor.user.id, "due", &what).await;
    }
    if Some(input.priority.as_str()) != e.priority.as_deref() && !(e.priority.is_none() && input.priority == "normal") {
        repo::record(&state.db, &id, &actor.user.id, "priority", &format!("made it {} priority", Priority::from_key(&input.priority).label().to_lowercase())).await;
    }
    flash_to(&back, "ok", "Saved.")
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct CheckForm {
    key: String,
    done: String,
}

pub(crate) async fn wf_item_check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CheckForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = load(&state, &actor, &id).await {
        return r;
    }
    let back = format!("{ITEMS}{id}/");
    let s = settings_of(&state);
    // Only a key the checklist actually has.
    let Some(label) = s.checklist.iter().find(|l| rules::check_key(l) == form.key) else {
        return flash_to(&back, "error", "That is not on the checklist.");
    };
    let done = form.done == "1";
    if let Err(e) = repo::set_check(&state.db, &id, &form.key, done, &actor.user.id).await {
        tracing::error!("workflows: ticking {id} failed: {}", e.message());
        return flash_to(&back, "error", "That could not be saved.");
    }
    repo::record(&state.db, &id, &actor.user.id, "checked", &format!("{} “{label}”", if done { "ticked" } else { "unticked" })).await;
    Redirect::to(&back).into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct NoteForm {
    body: String,
}

pub(crate) async fn wf_note_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<NoteForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = load(&state, &actor, &id).await {
        return r;
    }
    let back = format!("{ITEMS}{id}/");
    let body = rules::clean(&form.body, rules::MAX_NOTE);
    if body.is_empty() {
        return flash_to(&back, "error", "A note needs some words.");
    }
    match repo::add_note(&state.db, &id, &actor.user.id, &body).await {
        Ok(_) => {
            repo::record(&state.db, &id, &actor.user.id, "note", "").await;
            Redirect::to(&back).into_response()
        }
        Err(e) => {
            tracing::error!("workflows: adding a note to {id} failed: {}", e.message());
            flash_to(&back, "error", "The note could not be saved.")
        }
    }
}

pub(crate) async fn wf_note_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, note)): Path<(String, String)>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if let Err(r) = load(&state, &actor, &id).await {
        return r;
    }
    let back = format!("{ITEMS}{id}/");
    if !vuuut_core::ids::is_objectid(&note) {
        return flash_to(&back, "error", "That note could not be found.");
    }
    if repo::delete_note(&state.db, &note, &actor.user.id, actor.role >= Role::Editor).await {
        flash_to(&back, "ok", "Note deleted.")
    } else {
        flash_to(&back, "error", "Only its author or an editor can delete that note.")
    }
}
