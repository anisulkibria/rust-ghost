//! The Forms module: build forms, and read what visitors send through them.
//!
//! Two halves share `/ghost/forms/`:
//!
//! * the **builder** — the list of forms, and one screen per form for its
//!   fields, wording and notifications;
//! * the **responses** — every response, filterable by form and by state,
//!   and one screen per response.
//!
//! The site-wide switches (who is notified by default, what is kept and for
//! how long) live under Settings → Forms, with the other settings.
//!
//! Everything here is Administrator-only. Responses are other people's
//! personal data — names, addresses, whatever they chose to write — so they
//! are not shown to roles that cannot see members either.
//!
//! Every value a visitor supplied is escaped where it is shown. Nothing in a
//! response is ever rendered as markup, used as a link target, or put in a
//! header: the one link built from a response is a `mailto:` to an address
//! that has passed [`ghost_core::forms::is_email`].
//!
//! State-changing requests are checked for origin as well as for the session
//! (see [`guard`]). The session cookie is `SameSite=Lax`, which already keeps
//! it off cross-site POSTs; the explicit check is so that deleting a form's
//! responses does not rest on a single browser behaviour.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use ghost_api::AppState;
use ghost_core::forms as rules;
use ghost_db::repos::forms as repo;
use std::collections::HashMap;

use crate::{countable, esc, flash_banner, page_for_role, require_role, select_options, svg, Actor, Role, SELECT_CARET};

const PER_PAGE: i64 = 50;
/// The most rows one export carries.
const EXPORT_LIMIT: i64 = 5000;

/// The role check, plus — for anything that changes state — the origin check.
async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

fn server_error(e: &ghost_core::GhostError) -> Response {
    tracing::error!("forms admin: {}", e.message());
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

/// The two halves of the module, as tabs.
fn tabs(active: &str, new_count: i64) -> String {
    let tab = |key: &str, href: &str, label: &str, count: Option<i64>| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{count}</a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
            count = count.map(|n| format!(r#"<span class="seg-count">{n}</span>"#)).unwrap_or_else(|| r#"<span style="width:8px"></span>"#.into()),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{forms}{responses}</div>"#,
        forms = tab("forms", "/ghost/forms/", "Forms", None),
        responses = tab("responses", "/ghost/forms/responses/", "Responses", Some(new_count)),
    )
}

fn status_chip(status: &str) -> &'static str {
    match status {
        "active" => r#"<span class="chip published">accepting responses</span>"#,
        _ => r#"<span class="chip draft">closed</span>"#,
    }
}

// ---------------------------------------------------------------------------
// the list of forms
// ---------------------------------------------------------------------------

pub async fn forms_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let forms = match repo::list_with_counts(&state.db).await {
        Ok(f) => f,
        Err(e) => return server_error(&e),
    };
    let (new_count, _, _) = repo::totals(&state.db, None).await.unwrap_or((0, 0, 0));

    let rows: String = forms
        .iter()
        .map(|f| {
            let unread = if f.unread > 0 {
                format!(r#"<span class="chip tonal">{} new</span>"#, f.unread)
            } else {
                String::new()
            };
            format!(
                r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><a class="title" style="color:inherit;text-decoration:none" href="/ghost/forms/{id}/">{name}</a>
<span class="detail">{fields} · <a href="/ghost/forms/responses/?form={id}">{responses}</a></span></div>
<div class="trailing">{unread}{chip}<a class="btn tonal small" href="/ghost/forms/responses/?form={id}">Responses</a>
<a class="icon-btn" href="/ghost/forms/{id}/" title="Edit form" aria-label="Edit form">{edit}</a></div></div>"#,
                ic = svg("form"),
                id = esc(&f.form.id),
                name = esc(&f.form.name),
                fields = countable(f.form.fields.len() as i64, "field"),
                responses = countable(f.total, "response"),
                unread = unread,
                chip = status_chip(&f.form.status),
                edit = svg("edit"),
            )
        })
        .collect();

    let list = if forms.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:2;text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">No forms yet</p>
<p class="muted" style="margin:0 0 16px">Create a form, then add it to any post or page from the editor’s card menu.</p>
<form method="post" action="/ghost/forms/new/" style="display:inline-block"><button class="btn" type="submit">{add} Create a contact form</button></form>
</div>"#,
            add = svg("add"),
        )
    } else {
        format!(
            r#"<div class="card rows rise" style="--i:2">{rows}</div>
<p class="muted rise" style="--i:3;margin:14px 4px 0">To put a form on your site, open a post or page, click <b>+</b> (or type <code>/form</code>) and choose <b>Form</b>.</p>"#
        )
    };

    page_for_role(
        actor.role,
        "Forms",
        "forms",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/modules/" title="Back to modules">{back}</a>
<div><h1>Forms</h1><p class="sub">Build forms and read what visitors send you.</p></div>
<div class="actions"><a class="btn text" href="/ghost/settings/forms/">{tune} Settings</a>
<form method="post" action="/ghost/forms/new/"><button class="btn" type="submit">{add} New form</button></form></div></header>
{banner}{tabs}{list}"#,
            back = svg("chevron"),
            tune = svg("tune"),
            add = svg("add"),
            banner = flash_banner(&q),
            tabs = tabs("forms", new_count),
        ),
        true,
    )
    .into_response()
}

/// Create a form with the usual three fields and open it in the builder.
pub async fn form_create(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let existing = repo::list(&state.db).await.map(|f| f.len()).unwrap_or(0);
    if existing >= 200 {
        return flash_to("/ghost/forms/", "error", "This site already has 200 forms. Delete one to add another.");
    }
    let fields = rules::default_fields();
    let name = if existing == 0 { "Contact form".to_string() } else { format!("Contact form {}", existing + 1) };
    let input = repo::FormInput {
        name: &name,
        status: "active",
        fields: &fields,
        submit_label: rules::DEFAULT_SUBMIT_LABEL,
        success_message: rules::DEFAULT_SUCCESS_MESSAGE,
        notify_emails: "",
        success_action: "message",
        success_url: "",
        success_link_label: "",
    };
    match repo::create(&state.db, &input).await {
        Ok(id) => flash_to(&format!("/ghost/forms/{id}/"), "ok", "Form created. Adjust its fields, then add it to a post or page."),
        Err(e) => server_error(&e),
    }
}

// ---------------------------------------------------------------------------
// the builder
// ---------------------------------------------------------------------------

pub async fn form_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let form = match repo::find(&state.db, &id).await {
        Ok(Some(f)) => f,
        Ok(None) => return crate::not_found_page_for(actor.role, &actor.user, "That form could not be found."),
        Err(e) => return server_error(&e),
    };
    let (new_count, read_count, _) = repo::totals(&state.db, Some(&form.id)).await.unwrap_or((0, 0, 0));

    // The builder script draws the fields from this, and offers these types.
    let builder = serde_json::json!({
        "fields": form.fields,
        "kinds": rules::FieldKind::ALL.iter().map(|k| serde_json::json!({
            "value": k.as_str(), "label": k.label(), "options": k.has_options(),
            "placeholder": !k.has_options() && !matches!(k, rules::FieldKind::Consent | rules::FieldKind::Date),
        })).collect::<Vec<_>>(),
        "maxFields": rules::MAX_FIELDS,
        "maxOptions": rules::MAX_OPTIONS,
        // the upload button posts here; the script cannot read the base path
        // off its own, already rewritten, URL
        "uploadUrl": format!("{}/ghost/api/admin/files/upload/", ghost_core::url::subdir()),
        "basePath": ghost_core::url::subdir(),
    });

    page_for_role(
        actor.role,
        &form.name,
        "forms",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/forms/" title="Back to forms">{back}</a>
<div><h1>{name}</h1><p class="sub">{fields} · <a href="/ghost/forms/responses/?form={id}">{responses}</a></p></div>
<div class="actions">{chip}<a class="btn tonal" href="/ghost/forms/responses/?form={id}">Responses</a>
<button class="btn" type="submit" form="form-builder">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/forms/{id}/" id="form-builder" data-builder="{builder}">
<input type="hidden" name="fields" id="fb-fields-json" value="">

<div class="section">Form</div>
<div class="card padded rise" style="--i:1">
<label class="field"><input name="name" value="{name}" placeholder=" " required maxlength="{max_name}"><span>Name · only you see this</span></label>
<label class="field fixed" style="margin:0;max-width:320px"><select name="status">{status_opts}</select><span>Status</span>{car}</label>
</div>

<div class="section">Fields</div>
<noscript><div class="banner error">{flag}The form builder needs JavaScript.</div></noscript>
<div id="fb-list" class="fb-list rise" style="--i:2"></div>
<div class="fb-add rise" style="--i:3">
<label class="field fixed" style="margin:0;flex:1;max-width:260px"><select id="fb-add-kind" aria-label="Type of field to add"></select><span>Field type</span>{car}</label>
<button class="btn tonal" type="button" id="fb-add">{add} Add field</button>
<span class="muted" id="fb-count"></span>
</div>

<div class="section">Sending</div>
<div class="card padded rise" style="--i:4">
<label class="field" style="margin:0"><input name="submit_label" value="{submit_label}" placeholder=" " maxlength="{max_submit}"><span>Button label</span></label>
</div>

<div class="section">After a response is sent</div>
<div class="card padded rise" style="--i:4" id="fb-after">
<label class="field fixed" style="max-width:420px"><select name="success_action" id="fb-action">{action_opts}</select><span>Then</span>{car}</label>
<label class="field" data-fb-for="message link"><textarea name="success_message" placeholder=" " maxlength="{max_success}" rows="2">{success_message}</textarea><span>Success message</span></label>
<div data-fb-for="link redirect">
<div class="fb-url">
<label class="field" style="margin:0;flex:1;min-width:0"><input name="success_url" id="fb-url" value="{success_url}" placeholder=" " maxlength="{max_url}" inputmode="url" autocomplete="off" spellcheck="false"><span>Link · https://… or a path on this site</span></label>
<input type="file" id="fb-file" hidden>
<button class="btn tonal" type="button" id="fb-upload">{upload} Upload a file</button>
</div>
<p class="muted" id="fb-upload-state" style="margin:8px 0 14px">Paste an address, or upload a file — a PDF, say — to link to it.</p>
</div>
<label class="field" data-fb-for="link" style="margin:0 0 8px"><input name="success_link_label" value="{success_link_label}" placeholder=" " maxlength="{max_link_label}"><span>Link text · e.g. Download the guide</span></label>
<p class="muted" style="margin:0" data-fb-for="link">The link is only shown once a response has been accepted; it is not in the page before that.</p>
<p class="muted" style="margin:0" data-fb-for="redirect">The visitor is taken to this address as soon as their response is accepted.</p>
</div>

<div class="section">Notifications</div>
<div class="card padded rise" style="--i:5">
<label class="field" style="margin:0 0 8px"><input name="notify_emails" value="{notify_emails}" placeholder=" " maxlength="900" inputmode="email" autocomplete="off"><span>Email these addresses about each response</span></label>
<p class="muted" style="margin:0">Separate addresses with commas. Leave empty to use the addresses in <a href="/ghost/settings/forms/">Settings → Forms</a> — by default, the site owner. Sent with the site’s <a href="/ghost/settings/email/">transactional email</a>.</p>
</div>
</form>

<div class="section">Add it to your site</div>
<div class="card padded rise" style="--i:6">
<p style="margin:0 0 6px">Open a post or page, click <b>+</b> on an empty line (or type <code>/form</code>), choose <b>Form</b>, and pick <b>{name}</b>.</p>
<p class="muted" style="margin:0">The card stores a reference, not a copy — changes saved here appear everywhere the form is used.</p>
</div>

<div class="section">Delete</div>
<div class="card padded rise" style="--i:7">
<p class="muted" style="margin:0 0 12px">Deleting a form also deletes its {all} and removes it from every post and page it was added to.</p>
<form method="post" action="/ghost/forms/{id}/delete/" data-confirm="Delete form|“{name}” and all of its responses will be permanently deleted.">
<button class="btn tonal small danger" type="submit">{del} Delete form</button></form>
</div>
<script src="{assets}/forms-builder.js?v={builder_hash}" defer></script>"#,
            back = svg("chevron"),
            name = esc(&form.name),
            id = esc(&form.id),
            fields = countable(form.fields.len() as i64, "field"),
            responses = countable(new_count + read_count, "response"),
            all = countable(new_count + read_count, "response"),
            chip = status_chip(&form.status),
            check = svg("check"),
            banner = flash_banner(&q),
            builder = esc(&builder.to_string()),
            max_name = rules::MAX_NAME,
            status_opts = select_options(&[("active", "Accepting responses"), ("closed", "Closed")], &form.status),
            car = SELECT_CARET,
            flag = svg("flag"),
            add = svg("add"),
            submit_label = esc(&form.submit_label),
            max_submit = rules::MAX_SUBMIT_LABEL,
            success_message = esc(&form.success_message),
            max_success = rules::MAX_SUCCESS_MESSAGE,
            action_opts = select_options(
                &[
                    ("message", "Show a success message"),
                    ("link", "Show the message with a link to a URL or file"),
                    ("redirect", "Go to a URL"),
                ],
                rules::SuccessAction::parse(&form.success_action).as_str(),
            ),
            success_url = esc(&form.success_url),
            max_url = rules::MAX_URL,
            upload = svg("upload"),
            success_link_label = esc(&form.success_link_label),
            max_link_label = rules::MAX_LINK_LABEL,
            notify_emails = esc(&form.notify_emails),
            del = svg("delete"),
            assets = crate::ASSETS,
            builder_hash = builder_script_hash(),
        ),
        true,
    )
    .into_response()
}

/// A hash of the builder script, for its URL.
///
/// The admin's scripts are served `immutable` behind a version in the path,
/// which is only as good as someone remembering to bump it: a script changed
/// without a bump is served from the browser's cache for a year, and the
/// page quietly runs the old one. Hashing the contents into the query string
/// makes a changed script a new URL by construction.
fn builder_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(include_str!("../assets/forms-builder.js").as_bytes()))[..12].to_string()
    });
    &HASH
}

pub async fn form_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let back = format!("/ghost/forms/{id}/");
    match repo::find(&state.db, &id).await {
        Ok(Some(_)) => {}
        Ok(None) => return flash_to("/ghost/forms/", "error", "That form no longer exists."),
        Err(e) => return server_error(&e),
    }
    let value = |key: &str| posted.get(key).map(String::as_str).unwrap_or("");

    let name = rules::clean_bounded(value("name"), false, rules::MAX_NAME);
    if name.is_empty() {
        return flash_to(&back, "error", "Give the form a name.");
    }
    // The definition is rebuilt from what was posted, never stored as sent.
    let fields = match rules::normalize_fields(value("fields")) {
        Ok(fields) => fields,
        Err(message) => return flash_to(&back, "error", &message),
    };
    let submit_label = match rules::clean_bounded(value("submit_label"), false, rules::MAX_SUBMIT_LABEL) {
        label if label.is_empty() => rules::DEFAULT_SUBMIT_LABEL.to_string(),
        label => label,
    };
    let success_message = match rules::clean_bounded(value("success_message"), false, rules::MAX_SUCCESS_MESSAGE) {
        message if message.is_empty() => rules::DEFAULT_SUCCESS_MESSAGE.to_string(),
        message => message,
    };
    let (recipients, refused) = rules::parse_recipients(value("notify_emails"));
    if !refused.is_empty() {
        return flash_to(&back, "error", &format!("Not an email address: {}. Nothing was saved.", refused.join(", ")));
    }
    let notify_emails = recipients.join(", ");

    // Where a response leads. The address is checked here, on the way in,
    // and again each time it is used.
    let action = rules::SuccessAction::parse(value("success_action"));
    let success_url = match rules::normalize_success_url(value("success_url")) {
        Ok(url) => url,
        Err(message) => return flash_to(&back, "error", &format!("{message} Nothing was saved.")),
    };
    if action != rules::SuccessAction::Message && success_url.is_empty() {
        return flash_to(&back, "error", "Add the link this form should lead to after a response, or choose “Show a success message”. Nothing was saved.");
    }
    let success_link_label = rules::clean_bounded(value("success_link_label"), false, rules::MAX_LINK_LABEL);

    let input = repo::FormInput {
        name: &name,
        status: value("status"),
        fields: &fields,
        submit_label: &submit_label,
        success_message: &success_message,
        notify_emails: &notify_emails,
        success_action: action.as_str(),
        success_url: &success_url,
        success_link_label: &success_link_label,
    };
    match repo::update(&state.db, &id, &input).await {
        Ok(true) => flash_to(&back, "ok", "Form saved."),
        Ok(false) => flash_to("/ghost/forms/", "error", "That form no longer exists."),
        Err(e) => server_error(&e),
    }
}

pub async fn form_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    match repo::delete(&state.db, &id).await {
        Ok(true) => flash_to("/ghost/forms/", "ok", "Form and its responses deleted."),
        Ok(false) => flash_to("/ghost/forms/", "error", "That form no longer exists."),
        Err(e) => server_error(&e),
    }
}

// ---------------------------------------------------------------------------
// responses
// ---------------------------------------------------------------------------

const STATUS_FILTERS: &[(&str, &str)] = &[("all", "All"), ("new", "New"), ("read", "Read"), ("spam", "Spam")];

/// The `form` and `status` a list is filtered by, reduced to values that are
/// safe to put back into a link: an id that is an id, a status that is one
/// of the four.
fn filters(q: &HashMap<String, String>) -> (Option<String>, &'static str) {
    let form = q.get("form").filter(|id| ghost_core::ids::is_objectid(id)).cloned();
    let status = q.get("status").map(String::as_str).unwrap_or("all");
    let status = STATUS_FILTERS.iter().map(|(v, _)| *v).find(|v| *v == status).unwrap_or("all");
    (form, status)
}

fn filter_query(form: Option<&str>, status: &str) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    if let Some(form) = form {
        s.append_pair("form", form);
    }
    if status != "all" {
        s.append_pair("status", status);
    }
    s.finish()
}

pub async fn responses_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (form_filter, status) = filters(&q);
    let page = q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).clamp(1, 100_000);

    let forms = repo::list(&state.db).await.unwrap_or_default();
    let browse = repo::Browse {
        form_id: form_filter.as_deref(),
        status: (status != "all").then_some(status),
        page,
        limit: PER_PAGE,
    };
    let (rows, total) = match repo::browse(&state.db, &browse).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };
    let (new_count, read_count, spam_count) = repo::totals(&state.db, form_filter.as_deref()).await.unwrap_or((0, 0, 0));
    let (all_new, _, _) = if form_filter.is_some() {
        repo::totals(&state.db, None).await.unwrap_or((0, 0, 0))
    } else {
        (new_count, read_count, spam_count)
    };

    let status_tabs: String = STATUS_FILTERS
        .iter()
        .map(|(value, label)| {
            let count = match *value {
                "new" => new_count,
                "read" => read_count,
                "spam" => spam_count,
                _ => new_count + read_count,
            };
            format!(
                r#"<a class="seg{on}" href="/ghost/forms/responses/?{query}">{label}<span class="seg-count">{count}</span></a>"#,
                on = if *value == status { " on" } else { "" },
                query = esc(&filter_query(form_filter.as_deref(), value)),
            )
        })
        .collect();

    let mut form_options: Vec<(&str, &str)> = vec![("", "All forms")];
    form_options.extend(forms.iter().map(|f| (f.id.as_str(), f.name.as_str())));

    let list_rows: String = rows.iter().map(response_row).collect();
    let list = if rows.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:4;text-align:center"><p class="muted" style="margin:8px 0">{}</p></div>"#,
            match status {
                "new" => "Nothing new. You are all caught up.",
                "read" => "No responses have been read yet.",
                "spam" => "Nothing is filed as spam.",
                _ if forms.is_empty() => "No forms yet — create one and its responses will arrive here.",
                _ => "No responses yet. They appear here as soon as someone sends a form.",
            }
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:4">{list_rows}</div>"#)
    };

    let export_query = filter_query(form_filter.as_deref(), status);
    page_for_role(
        actor.role,
        "Responses",
        "forms",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/modules/" title="Back to modules">{back}</a>
<div><h1>Forms</h1><p class="sub">{new} new · {read} read · {spam} spam</p></div>
<div class="actions"><a class="btn tonal" href="/ghost/forms/responses/export/?{export_query}">{import} Export CSV</a></div></header>
{banner}{tabs}
<div class="card rows rise" style="--i:2"><div class="row"><span class="ic tint-blue">{form_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px">
<select id="fm-form-filter" data-status="{status}">{form_opts}</select><span>Form</span>{car}</label></div></div></div>
<div class="seg-bar sub rise" style="--i:3;margin-top:14px">{status_tabs}</div>
{list}
{pager}
<script>
(function () {{
  var select = document.getElementById('fm-form-filter');
  if (!select) return;
  select.addEventListener('change', function () {{
    var query = new URLSearchParams();
    if (select.value) query.set('form', select.value);
    if (select.dataset.status && select.dataset.status !== 'all') query.set('status', select.dataset.status);
    var text = query.toString();
    location.href = select.dataset.base + (text ? '?' + text : '');
  }});
  select.dataset.base = location.pathname;
}})();
</script>"#,
            back = svg("chevron"),
            new = new_count,
            read = read_count,
            spam = spam_count,
            export_query = esc(&export_query),
            import = svg("import"),
            banner = flash_banner(&q),
            tabs = tabs("responses", all_new),
            form_ic = svg("form"),
            status = esc(status),
            form_opts = select_options(&form_options, form_filter.as_deref().unwrap_or("")),
            car = SELECT_CARET,
            pager = pager(form_filter.as_deref(), status, page, total),
        ),
        true,
    )
    .into_response()
}

fn response_row(s: &repo::Submission) -> String {
    let who = s
        .answers
        .iter()
        .find(|a| a.kind == "text")
        .or_else(|| s.answers.iter().find(|a| a.kind == "email"))
        .map(|a| a.value.clone())
        .unwrap_or_else(|| "Response".into());
    let initial = who.chars().next().unwrap_or('?').to_uppercase().to_string();
    let chip = match s.status.as_str() {
        "new" => r#"<span class="chip tonal">new</span>"#,
        "spam" => r#"<span class="chip error">spam</span>"#,
        _ => "",
    };
    format!(
        r#"<a class="row" href="/ghost/forms/responses/{id}/"{weight}><span class="avatar">{initial}</span>
<div class="body"><span class="title">{who}</span>
<span class="detail">{summary}</span>
<span class="detail">{form} · {date}</span></div>
<div class="trailing">{chip}{chev}</div></a>"#,
        id = esc(&s.id),
        weight = if s.status == "new" { r#" style="font-weight:600""# } else { "" },
        initial = esc(&initial),
        who = esc(&rules::clean_bounded(&who, false, 80)),
        summary = esc(&rules::summary(&s.answers, 140)),
        form = esc(&s.form_name),
        date = esc(&s.created_at),
        chev = svg("chevron"),
    )
}

fn pager(form: Option<&str>, status: &str, page: i64, total: i64) -> String {
    let pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    if pages <= 1 {
        return String::new();
    }
    let base = filter_query(form, status);
    let link = |p: i64, label: &str| {
        // Written into an attribute, so the separator is escaped like the
        // rest of the query.
        let joiner = if base.is_empty() { "" } else { "&amp;" };
        format!(r#"<a class="btn tonal small" href="/ghost/forms/responses/?{base}{joiner}page={p}">{label}</a>"#, base = esc(&base))
    };
    format!(
        r#"<div style="display:flex;gap:10px;align-items:center;padding:14px 4px">
{prev}<span class="muted">Page {page} of {pages}</span>{next}</div>"#,
        prev = if page > 1 { link(page - 1, "← Newer") } else { String::new() },
        next = if page < pages { link(page + 1, "Older →") } else { String::new() },
    )
}

/// One response in full. Opening it is what marks it read.
pub async fn response_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let mut s = match repo::find_submission(&state.db, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return crate::not_found_page_for(actor.role, &actor.user, "That response could not be found."),
        Err(e) => return server_error(&e),
    };
    if s.status == "new" {
        let _ = repo::mark_read_if_new(&state.db, &s.id).await;
        s.status = "read".into();
    }

    let answers: String = s
        .answers
        .iter()
        .map(|a| {
            format!(
                r#"<div class="fm-answer"><div class="fm-q">{label}</div><div class="fm-a">{value}</div></div>"#,
                label = esc(&a.label),
                value = esc(&a.value),
            )
        })
        .collect();

    let status_form = |status: &str, label: &str, icon: &str| {
        format!(
            r#"<form method="post" action="/ghost/forms/responses/{id}/status/"><input type="hidden" name="status" value="{status}">
<button class="btn tonal small" type="submit">{icon} {label}</button></form>"#,
            id = esc(&s.id),
            icon = svg(icon),
        )
    };
    let actions = match s.status.as_str() {
        "spam" => status_form("read", "Not spam", "check"),
        _ => format!("{}{}", status_form("new", "Mark as unread", "mail"), status_form("spam", "Mark as spam", "flag")),
    };
    // The only link built from a response, and only to an address that
    // passes the strict check — so it can carry no extra headers or scheme.
    let reply = match rules::reply_address(&s.answers) {
        Some(address) if s.status != "spam" => format!(
            r#"<a class="btn small" href="mailto:{address}">{send} Reply by email</a>"#,
            address = esc(address),
            send = svg("send"),
        ),
        _ => String::new(),
    };

    let row = |icon: &str, tint: &str, title: &str, detail: String| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div></div>"#,
            icon = svg(icon),
        )
    };
    let form_exists = repo::find(&state.db, &s.form_id).await.ok().flatten().is_some();
    let form_row = row(
        "form",
        "blue",
        "Form",
        if form_exists {
            format!(
                r#"<a href="/ghost/forms/{id}/">{name}</a> · <a href="/ghost/forms/responses/?form={id}">all its responses</a>"#,
                id = esc(&s.form_id),
                name = esc(&s.form_name),
            )
        } else {
            esc(&s.form_name)
        },
    );
    let page_row = s
        .page_path
        .as_deref()
        // Shown as text and linked only as a path on this site: it came from
        // a Referer header, which the sender controls.
        .filter(|p| p.starts_with('/') && !p.starts_with("//"))
        .map(|p| row("link", "slate", "Sent from", format!(r#"<a href="{p}" target="_blank" rel="noopener noreferrer">{p}</a>"#, p = esc(p))))
        .unwrap_or_default();
    let member_row = s
        .member_id
        .as_deref()
        .filter(|id| ghost_core::ids::is_objectid(id))
        .map(|id| row("ghost", "teal", "Signed-in member", format!(r#"<a href="/ghost/members/{id}/">Open their profile</a>"#, id = esc(id))))
        .unwrap_or_default();
    let ip_row = s.ip.as_deref().map(|ip| row("globe", "slate", "IP address", esc(ip))).unwrap_or_default();
    let ua_row = s.user_agent.as_deref().map(|ua| row("code", "slate", "Browser", esc(ua))).unwrap_or_default();

    let chip = match s.status.as_str() {
        "spam" => r#"<span class="chip error">spam</span>"#,
        _ => r#"<span class="chip published">read</span>"#,
    };

    page_for_role(
        actor.role,
        "Response",
        "forms",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/forms/responses/?form={form_id}" title="Back to responses">{back}</a>
<div><h1>Response</h1><p class="sub">{form_name} · {created}</p></div>
<div class="actions">{chip}{reply}</div></header>
{banner}
<div class="section">Answers</div>
<div class="card padded rise" style="--i:1">{answers}
<div style="display:flex;gap:10px;align-items:center;margin-top:18px;flex-wrap:wrap">
{actions}
<form method="post" action="/ghost/forms/responses/{id}/delete/" data-confirm="Delete response|This response will be permanently deleted.">
<button class="btn tonal small danger" type="submit">{del} Delete</button></form>
</div></div>
<div class="section">Details</div>
<div class="card rows rise" style="--i:2">
{form_row}{page_row}{member_row}{ip_row}{ua_row}
<div class="row"><span class="ic tint-slate">{history}</span>
<div class="body"><span class="title">Received</span><span class="detail">{created} UTC · id {id}</span></div></div>
</div>
<p class="muted" style="margin:12px 4px 0">This was written by a visitor. Be as careful with links and requests in it as with any message from someone you do not know.</p>"#,
            form_id = esc(&s.form_id),
            back = svg("chevron"),
            form_name = esc(&s.form_name),
            created = esc(&s.created_at),
            banner = flash_banner(&q),
            id = esc(&s.id),
            del = svg("delete"),
            history = svg("history"),
        ),
        true,
    )
    .into_response()
}

pub async fn response_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let status = posted.get("status").map(String::as_str).unwrap_or("");
    let (message, back) = match status {
        "new" => ("Marked as unread.", "/ghost/forms/responses/".to_string()),
        "read" => ("Moved out of spam.", format!("/ghost/forms/responses/{id}/")),
        "spam" => ("Filed as spam.", "/ghost/forms/responses/".to_string()),
        _ => return flash_to("/ghost/forms/responses/", "error", "That is not something a response can be marked as."),
    };
    if !ghost_core::ids::is_objectid(&id) {
        return flash_to("/ghost/forms/responses/", "error", "That response no longer exists.");
    }
    match repo::set_submission_status(&state.db, &id, status).await {
        Ok(true) => flash_to(&back, "ok", message),
        Ok(false) => flash_to("/ghost/forms/responses/", "error", "That response no longer exists."),
        Err(e) => server_error(&e),
    }
}

pub async fn response_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return flash_to("/ghost/forms/responses/", "error", "That response no longer exists.");
    }
    match repo::delete_submission(&state.db, &id).await {
        Ok(true) => flash_to("/ghost/forms/responses/", "ok", "Response deleted."),
        Ok(false) => flash_to("/ghost/forms/responses/", "error", "That response no longer exists."),
        Err(e) => server_error(&e),
    }
}

/// The responses a list is showing, as a spreadsheet.
///
/// One column per question that appears in the exported rows, in the order
/// first met. Every cell goes through [`rules::csv_safe`]: a response is
/// text from a stranger, and a cell beginning `=` would otherwise run as a
/// formula on the computer of whoever opens the file.
pub async fn responses_export(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    let (form_filter, status) = filters(&q);
    let browse = repo::Browse {
        form_id: form_filter.as_deref(),
        status: (status != "all").then_some(status),
        page: 1,
        limit: EXPORT_LIMIT,
    };
    let (rows, _) = match repo::browse(&state.db, &browse).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };
    match export_csv(&rows) {
        Some(body) => (
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                (header::CONTENT_DISPOSITION, format!("attachment; filename=\"form-responses.{}.csv\"", crate::export_date_stamp())),
                (header::CACHE_CONTROL, "no-store".to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            ],
            body,
        )
            .into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "The export could not be built.").into_response(),
    }
}

fn export_csv(rows: &[repo::Submission]) -> Option<Vec<u8>> {
    // A question is identified by its field key, and labelled by the label
    // it had most recently among these rows.
    let mut columns: Vec<(String, String)> = Vec::new();
    for s in rows {
        for a in &s.answers {
            if !columns.iter().any(|(key, _)| *key == a.key) {
                columns.push((a.key.clone(), a.label.clone()));
            }
        }
    }
    let mut w = csv::Writer::from_writer(Vec::new());
    let mut head: Vec<String> = vec!["Received (UTC)".into(), "Form".into(), "Status".into()];
    head.extend(columns.iter().map(|(_, label)| rules::csv_safe(label)));
    head.extend(["Sent from".to_string(), "IP address".to_string()]);
    w.write_record(&head).ok()?;
    for s in rows {
        let mut record: Vec<String> = vec![s.created_at.clone(), rules::csv_safe(&s.form_name), s.status.clone()];
        for (key, _) in &columns {
            let value = s.answers.iter().find(|a| a.key == *key).map(|a| a.value.as_str()).unwrap_or("");
            record.push(rules::csv_safe(value));
        }
        record.push(rules::csv_safe(s.page_path.as_deref().unwrap_or("")));
        record.push(rules::csv_safe(s.ip.as_deref().unwrap_or("")));
        w.write_record(&record).ok()?;
    }
    w.into_inner().ok()
}

// ---------------------------------------------------------------------------
// Settings → Forms
// ---------------------------------------------------------------------------

const RETENTION: &[(&str, &str)] = &[
    ("0", "Keep until deleted"),
    ("30", "30 days"),
    ("90", "90 days"),
    ("180", "6 months"),
    ("365", "1 year"),
    ("730", "2 years"),
];

pub async fn settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let config = match repo::config(&state.db).await {
        Ok(c) => c,
        Err(e) => return server_error(&e),
    };
    let group = crate::SETTINGS_GROUPS.iter().find(|g| g.slug == "forms").expect("forms settings group");
    let switch = |name: &str, icon: &str, tint: &str, title: &str, detail: &str, on: bool| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="{name}" value="1"{ck}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            icon = svg(icon),
            title = esc(title),
            detail = esc(detail),
            ck = if on { " checked" } else { "" },
        )
    };
    let retention = config.retention_days.to_string();

    page_for_role(
        actor.role,
        "Forms",
        "settings",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<p class="rise" style="--i:1"><a class="btn tonal" href="/ghost/forms/">{form_ic} Open the form builder &amp; responses</a></p>
<form method="post" action="/ghost/settings/forms/" id="forms-settings">
<div class="section">Notifications</div>
<div class="card rows rise" style="--i:2">{notify}</div>
<div class="card padded rise" style="--i:3">
<label class="field" style="margin:0 0 8px"><input name="notify_emails" value="{emails}" placeholder=" " maxlength="900" inputmode="email" autocomplete="off"><span>Default notification addresses</span></label>
<p class="muted" style="margin:0 0 6px">Separate addresses with commas. Leave empty to notify the site owner{owner}. A form can name its own addresses in the builder.</p>
<p class="muted" style="margin:0">Notifications are sent with the site’s transactional email, as set up in <a href="/ghost/settings/email/">Settings → Email</a>.</p>
</div>

<div class="section">Spam protection</div>
<div class="card rows rise" style="--i:4">
{shield}
</div>

<div class="section">Privacy &amp; retention</div>
<div class="card rows rise" style="--i:5">{store_ip}
<div class="row"><span class="ic tint-slate">{history}</span>
<div class="body"><span class="title">Keep responses for</span><span class="detail">Older responses are deleted automatically</span></div>
<div class="trailing"><div class="selbox"><select name="retention_days" aria-label="Keep responses for">{retention_opts}</select>{car}</div></div></div>
</div>
</form>"#,
            head = crate::group_head(group, Some("forms-settings")),
            banner = flash_banner(&q),
            form_ic = svg("form"),
            notify = switch(
                "notify_enabled",
                "mail",
                "blue",
                "Email me about new responses",
                "Send a message each time a form is submitted",
                config.notify_enabled,
            ),
            emails = esc(&config.notify_emails),
            owner = owner_email(&state)
                .await
                .map(|email| format!(" (<b>{}</b>)", esc(&email)))
                .unwrap_or_default(),
            shield = [
                ("Single-use signed tokens", "Each form carries a token that works once, for that form only, and expires"),
                ("Minimum fill time", "A response sent within moments of the page loading is refused"),
                ("Hidden decoy field", "Responses that fill in a field people cannot see are discarded"),
                ("Rate limits", "Repeated attempts from one address, or a flood at one form, are slowed down"),
                ("Same-site only", "A form can be sent from your own pages, not from another website"),
            ]
            .iter()
            .map(|(title, detail)| format!(
                r#"<div class="row"><span class="ic tint-green">{check}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip published">always on</span></div></div>"#,
                check = svg("check"),
            ))
            .collect::<String>(),
            store_ip = switch(
                "store_ip",
                "globe",
                "slate",
                "Store the sender’s IP address",
                "Helps trace abuse; turn off to keep less personal data",
                config.store_ip,
            ),
            history = svg("history"),
            retention_opts = select_options(RETENTION, &retention),
            car = SELECT_CARET,
        ),
        true,
    )
    .into_response()
}

/// The owner's address: who is notified when nobody else is named.
async fn owner_email(state: &AppState) -> Option<String> {
    sqlx::query_scalar(
        "SELECT u.email FROM users u JOIN roles_users ru ON ru.user_id = u.id JOIN roles r ON r.id = ru.role_id \
         WHERE r.name = 'Owner' LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

pub async fn settings_save(State(state): State<AppState>, headers: HeaderMap, Form(posted): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let on = |key: &str| posted.get(key).map(|v| v == "1").unwrap_or(false);
    let (recipients, refused) = rules::parse_recipients(posted.get("notify_emails").map(String::as_str).unwrap_or(""));
    if !refused.is_empty() {
        return crate::settings_flash("forms", "error", &format!("Not an email address: {}. Nothing was saved.", refused.join(", ")));
    }
    // Only the offered windows are accepted; anything else keeps everything.
    let retention_days = posted
        .get("retention_days")
        .filter(|v| RETENTION.iter().any(|(value, _)| value == &v.as_str()))
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let config = repo::Config {
        notify_enabled: on("notify_enabled"),
        notify_emails: recipients.join(", "),
        store_ip: on("store_ip"),
        retention_days,
    };
    if let Err(e) = repo::save_config(&state.db, &config).await {
        tracing::error!("forms settings: {}", e.message());
        return crate::settings_flash("forms", "error", "Saving failed — check the logs.");
    }
    // Shortening the window takes effect now, not at the next response.
    if retention_days > 0 {
        let _ = repo::purge_older_than(&state.db, retention_days).await;
    }
    crate::settings_flash("forms", "ok", "Forms settings saved.")
}

/// The forms the editor's card offers: enough to pick one, nothing more.
pub async fn editor_choices(state: &AppState) -> serde_json::Value {
    let forms = repo::list(&state.db).await.unwrap_or_default();
    serde_json::Value::Array(
        forms
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "name": f.name,
                    "active": f.is_active(),
                    "submitLabel": f.submit_label,
                    "fields": f.fields.iter().map(|field| serde_json::json!({
                        "label": field.label,
                        "kind": field.kind.as_str(),
                        "required": field.required,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_core::forms::Answer;

    fn submission(answers: Vec<Answer>) -> repo::Submission {
        repo::Submission {
            id: "0123456789abcdef01234567".into(),
            form_id: "76543210fedcba9876543210".into(),
            form_name: "Contact".into(),
            answers,
            status: "new".into(),
            ip: Some("203.0.113.7".into()),
            user_agent: None,
            page_path: Some("/contact/".into()),
            member_id: None,
            created_at: "2026-11-12 10:00:00".into(),
        }
    }

    fn answer(key: &str, label: &str, kind: &str, value: &str) -> Answer {
        Answer { key: key.into(), label: label.into(), kind: kind.into(), value: value.into() }
    }

    #[test]
    fn a_response_row_escapes_what_the_visitor_wrote() {
        let hostile = r#"<img src=x onerror=alert(1)>"#;
        let row = response_row(&submission(vec![answer("f_00000001", "Name", "text", hostile), answer("f_00000002", "Msg", "textarea", hostile)]));
        assert!(!row.contains("<img"), "{row}");
        assert!(row.contains("&lt;img"));
    }

    #[test]
    fn the_export_defuses_formulas_and_lines_answers_up_by_question() {
        let rows = vec![
            submission(vec![answer("f_00000001", "Name", "text", "=cmd|' /C calc'!A0"), answer("f_00000002", "Message", "textarea", "hello, \"world\"\nline two")]),
            submission(vec![answer("f_00000002", "Message", "textarea", "@SUM(1)"), answer("f_00000003", "+Phone", "phone", "+44 20 7946 0000")]),
        ];
        let csv = String::from_utf8(export_csv(&rows).unwrap()).unwrap();
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        let head: Vec<String> = reader.headers().unwrap().iter().map(str::to_string).collect();
        assert_eq!(head, ["Received (UTC)", "Form", "Status", "Name", "Message", "'+Phone", "Sent from", "IP address"]);
        let records: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
        assert_eq!(&records[0][3], "'=cmd|' /C calc'!A0");
        assert_eq!(&records[0][4], "hello, \"world\"\nline two");
        assert_eq!(&records[0][5], "");
        assert_eq!(&records[1][3], "");
        assert_eq!(&records[1][4], "'@SUM(1)");
        assert_eq!(&records[1][5], "'+44 20 7946 0000");
    }

    #[test]
    fn list_filters_only_ever_carry_known_values() {
        let q = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert_eq!(filters(&q(&[])), (None, "all"));
        assert_eq!(filters(&q(&[("form", "0123456789abcdef01234567"), ("status", "spam")])), (Some("0123456789abcdef01234567".into()), "spam"));
        // neither a forged id nor a forged status survives to be linked
        assert_eq!(filters(&q(&[("form", "\"><script>"), ("status", "'; DROP TABLE forms;--")])), (None, "all"));
        assert_eq!(filter_query(Some("0123456789abcdef01234567"), "new"), "form=0123456789abcdef01234567&status=new");
        assert_eq!(filter_query(None, "all"), "");
    }

    #[test]
    fn the_pager_only_appears_when_there_is_more_than_one_page() {
        assert_eq!(pager(None, "all", 1, PER_PAGE), "");
        let two = pager(None, "new", 1, PER_PAGE + 1);
        assert!(two.contains("Page 1 of 2") && two.contains("Older") && !two.contains("Newer"), "{two}");
        assert!(two.contains("status=new&amp;page=2"), "{two}");
    }
}
