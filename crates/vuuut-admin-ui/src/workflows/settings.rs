//! Workflow settings: the stages, the approval gate and the checklist.
//!
//! Administrator and above — the stages are the shape of everybody's work,
//! and the gate decides who may publish. Saved through
//! `vuuut_core::workflows::Settings::read`, the same reader the board and
//! the editor's save path use.

use super::*;

const SETTINGS: &str = "/ghost/workflows/settings/";

async fn admin(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = guard(state, headers, writes).await?;
    if !actor.role.is_admin() {
        return Err(forbidden_page(&actor, Role::Administrator));
    }
    Ok(actor)
}

pub(crate) async fn wf_settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match admin(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = settings_of(&state);
    let colors = |current: &str| -> String {
        rules::COLORS
            .iter()
            .map(|c| format!(r#"<option value="{c}"{sel}>{label}</option>"#, sel = if *c == current { " selected" } else { "" }, label = c[..1].to_uppercase() + &c[1..]))
            .collect()
    };
    let rows: String = s
        .stages
        .iter()
        .enumerate()
        .map(|(i, st)| {
            format!(
                r#"<div class="wf-stage-row"><span class="wf-dot tint-{color}"></span>
<input type="hidden" name="key_{i}" value="{key}">
<label class="field" style="margin:0"><input name="label_{i}" value="{label}" maxlength="{max}" required placeholder=" "><span>Stage {n}</span></label>
<label class="field fixed" style="margin:0"><select name="color_{i}">{colors}</select><span>Colour</span>{caret}</label>
<label class="field" style="margin:0"><input type="number" name="limit_{i}" value="{limit}" min="0" max="999" placeholder=" "><span>Limit</span></label>
<label class="field" style="margin:0"><input type="number" name="order_{i}" value="{n}" min="1" max="10" placeholder=" "><span>Order</span></label>
<label class="wf-remove"><input type="checkbox" name="remove_{i}" value="1"> Remove</label></div>"#,
                color = esc(&st.color),
                key = esc(&st.key),
                label = esc(&st.label),
                max = rules::MAX_LABEL,
                n = i + 1,
                colors = colors(&st.color),
                caret = SELECT_CARET,
                limit = st.limit,
            )
        })
        .collect();
    let approval_opts: String = s
        .stages
        .iter()
        .map(|st| format!(r#"<option value="{k}"{sel}>{l}</option>"#, k = esc(&st.key), sel = if st.key == s.approval_stage { " selected" } else { "" }, l = esc(&st.label)))
        .collect();
    let switch = |name: &str, on: bool| {
        format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
    };
    page_for_role(
        actor.role,
        "Workflow settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="wf-settings">
<input type="hidden" name="count" value="{count}">
<div class="section">Stages</div>
<p class="muted wf-lead">The columns a draft moves through, in order. Scheduled and Published follow each post's status and are always last. A limit warns when a column holds more than that; 0 is no limit. Entries in a removed stage move to the second stage.</p>
<div class="card padded rise" style="--i:2">{rows}
<div class="wf-stage-row wf-stage-new"><span class="wf-dot tint-slate"></span>
<label class="field" style="margin:0;grid-column:2 / span 3"><input name="new_label" maxlength="{max}" placeholder=" "><span>Add a stage</span></label></div>
</div>
<div class="section">Approval</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:center"><span class="ic tint-red">{lock}</span>
<div class="body"><span class="title">Require approval to publish</span><span class="detail">Publishing from the editor is refused until the entry reaches the approval stage. Off by default.</span></div>
<div class="trailing">{require}</div></div>
<div class="row" style="align-items:center"><span class="ic tint-green">{check}</span>
<div class="body"><span class="title">Approval stage</span><span class="detail">Reaching this stage means an editor has approved it. Writers can move work up to the stage before it.</span></div>
<div class="trailing wf-control"><label class="field fixed" style="margin:0"><select name="{approval_key}">{approval_opts}</select><span>Approved means</span>{caret}</label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-purple">{person}</span>
<div class="body"><span class="title">Administrators may publish without approval</span><span class="detail">For the breaking story at midnight. Editors and below always need approval while it is required.</span></div>
<div class="trailing">{bypass}</div></div>
</div>
<div class="section">Checklist</div>
<div class="card padded rise" style="--i:4">
<label class="field" style="margin-bottom:0"><textarea name="{checklist_key}" rows="5" placeholder=" ">{checklist}</textarea><span>One item per line</span></label>
<p class="muted wf-lead" style="margin:8px 2px 0">Ticked by hand on each entry, beside the automatic checks (title, length, feature image, excerpt, tags). Up to {max_checks} items; renaming one resets its ticks.</p>
</div>
</form>"#,
            head = crate::tools::tool_head("workflows", Some("wf-settings"), ""),
            tabs = tabs("settings", true),
            banner = flash_banner(&q),
            count = s.stages.len(),
            max = rules::MAX_LABEL,
            lock = svg("lock"),
            check = svg("check"),
            person = svg("person"),
            require = switch(rules::SETTING_REQUIRE_APPROVAL, s.require_approval),
            bypass = switch(rules::SETTING_ADMINS_BYPASS, s.admins_bypass),
            approval_key = rules::SETTING_APPROVAL_STAGE,
            caret = SELECT_CARET,
            checklist_key = rules::SETTING_CHECKLIST,
            checklist = esc(&s.checklist.join("\n")),
            max_checks = rules::MAX_CHECKS,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn wf_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = admin(&state, &headers, true).await {
        return r;
    }
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let before = settings_of(&state);
    let count: usize = get("count").parse().unwrap_or(0).min(rules::MAX_STAGES);
    let mut ordered: Vec<(i64, usize, rules::Stage)> = Vec::new();
    for i in 0..count {
        if get(&format!("remove_{i}")) == "1" {
            continue;
        }
        let stage = rules::Stage {
            key: get(&format!("key_{i}")).to_string(),
            label: get(&format!("label_{i}")).to_string(),
            color: get(&format!("color_{i}")).to_string(),
            limit: get(&format!("limit_{i}")).trim().parse().unwrap_or(0),
        };
        let order: i64 = get(&format!("order_{i}")).trim().parse().unwrap_or(i as i64 + 1);
        ordered.push((order, i, stage));
    }
    let new_label = rules::clean(get("new_label"), rules::MAX_LABEL);
    if !new_label.is_empty() {
        ordered.push((i64::MAX, usize::MAX, rules::Stage { key: String::new(), label: new_label, color: String::new(), limit: 0 }));
    }
    ordered.sort_by_key(|(o, i, _)| (*o, *i));
    let stages = rules::normalize_stages(ordered.into_iter().map(|(_, _, s)| s).collect());
    let stages_json = serde_json::to_string(&stages).unwrap_or_default();
    let flag = |k: &str| get(k) == "1";
    // Through the reader, so an approval stage that was just removed falls
    // back the same way it would on the next read.
    let s = Settings::read(|key| match key {
        rules::SETTING_STAGES => Some(stages_json.clone()),
        rules::SETTING_REQUIRE_APPROVAL | rules::SETTING_ADMINS_BYPASS => Some(flag(key).to_string()),
        _ => form.get(key).cloned(),
    });
    let writes = [
        (rules::SETTING_STAGES, serde_json::json!(stages_json)),
        (rules::SETTING_REQUIRE_APPROVAL, serde_json::json!(s.require_approval)),
        (rules::SETTING_APPROVAL_STAGE, serde_json::json!(s.approval_stage)),
        (rules::SETTING_ADMINS_BYPASS, serde_json::json!(s.admins_bypass)),
        (rules::SETTING_CHECKLIST, serde_json::json!(s.checklist.join("\n"))),
    ];
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("workflows: saving {key} failed: {}", e.message());
            return flash_to(SETTINGS, "error", "Saving failed — check the logs.");
        }
    }
    // Work in a stage that no longer exists moves where an untracked draft
    // would sit, rather than vanishing from the board.
    let gone: Vec<String> = before.stages.iter().filter(|b| s.stage(&b.key).is_none()).map(|b| b.key.clone()).collect();
    let moved = repo::rehome(&state.db, &gone, s.default_stage()).await.unwrap_or(0);
    let message = if moved > 0 {
        format!("Workflow saved. {} moved to {}.", countable(moved as i64, "entry"), s.stage(s.default_stage()).map(|st| st.label.as_str()).unwrap_or(""))
    } else {
        "Workflow saved.".to_string()
    };
    crate::tools::tool_saved(&state, SETTINGS, &message)
}
