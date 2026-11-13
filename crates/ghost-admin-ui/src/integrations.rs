//! Settings -> Integrations: the content API keys, custom integrations and their webhooks.

use super::*;

// ---------------------------------------------------------------------------
// integrations (shows the content API keys) — a Settings → Advanced sub-page
// ---------------------------------------------------------------------------

/// Integrations used to be a top-level rail item, then lived under Settings →
/// Advanced before becoming a settings page of its own; this keeps links from
/// both eras working.
pub(crate) async fn integrations_redirect() -> Response {
    Redirect::permanent("/ghost/settings/integrations/").into_response()
}

/// The three apps Ghost ships with. Slug, label, blurb, icon, tint.
///
/// Zapier is an integration row in the database because it authenticates with
/// an API key; Slack and Unsplash are configuration only, which is the same
/// split Ghost makes — so the hub lists all three but each page works
/// differently underneath.
pub(crate) const BUILTIN_APPS: &[(&str, &str, &str, &str, &str)] = &[
    ("zapier", "Zapier", "Automate Ghost with 6,000+ apps", "share", "amber"),
    ("slack", "Slack", "Post to a channel when you publish", "send", "purple"),
    ("unsplash", "Unsplash", "Free photography inside the editor", "image", "green"),
    ("firstpromoter", "FirstPromoter", "Referral and affiliate tracking for members", "share", "blue"),
    ("pintura", "Pintura", "Edit images without leaving the editor", "image", "pink"),
    ("transistor", "Transistor.fm", "Private podcast feeds for your members", "send", "teal"),
];

pub(crate) fn integrations_flash(sub: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/integrations/{sub}?{encoded}")).into_response()
}

/// A secret with a copy button. Long keys are shown in full — they have to be
/// copied somewhere, and hiding them behind a reveal only adds a click.
pub(crate) fn key_chip(key_type: &str, secret: &str) -> String {
    format!(
        r#"<span class="key"><span class="chip tonal">{}</span>{}<button type="button" data-copy="{secret}" title="Copy" aria-label="Copy key">{copy}</button></span>"#,
        esc(key_type),
        esc(secret),
        secret = esc(secret),
        copy = svg("copy"),
    )
}

pub(crate) async fn integrations_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };

    // --- built-in apps -----------------------------------------------------
    let slack_on = ghost_api::slack::SlackConfig::from_settings(&state.settings).is_some();
    let unsplash_on = state.settings.get_bool("unsplash");
    let mut apps = String::new();
    for (slug, name, blurb, icon, tint) in BUILTIN_APPS {
        let status = match *slug {
            "slack" if slack_on => r#"<span class="chip ok">Connected</span>"#,
            "unsplash" if unsplash_on => r#"<span class="chip ok">On</span>"#,
            "unsplash" => r#"<span class="chip tonal">Off</span>"#,
            s if state.settings.get_bool(s) => r#"<span class="chip ok">On</span>"#,
            "firstpromoter" | "pintura" | "transistor" => r#"<span class="chip tonal">Off</span>"#,
            "zapier" => r#"<span class="chip tonal">API key ready</span>"#,
            _ => r#"<span class="chip tonal">Not set up</span>"#,
        };
        apps.push_str(&format!(
            r#"<a class="row" href="/ghost/settings/integrations/{slug}/"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{blurb}</span></div>
<div class="trailing">{status}</div></a>"#,
            slug = slug,
            tint = tint,
            ic = svg(icon),
            name = esc(name),
            blurb = esc(blurb),
            status = status,
        ));
    }

    // --- custom integrations ----------------------------------------------
    let customs = repos::integrations::list_custom(&state.db).await.unwrap_or_default();
    let mut custom_rows = String::new();
    for i in &customs {
        let detail = i
            .description
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| format!("/{}", i.slug.clone().unwrap_or_default()));
        custom_rows.push_str(&format!(
            r#"<a class="row" href="/ghost/settings/integrations/custom/{id}/"><span class="ic tint-teal">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{detail}</span></div>
<span class="chip tonal">2 keys</span></a>"#,
            id = esc(&i.id),
            ic = svg("integrations"),
            name = esc(&i.name),
            detail = esc(&detail),
        ));
    }
    if custom_rows.is_empty() {
        custom_rows = r#"<div class="row"><div class="body"><span class="detail">No custom integrations yet. Add one to get a Content API and Admin API key for your own scripts.</span></div></div>"#.to_string();
    }

    // --- the frontend's own key -------------------------------------------
    let mut internal = String::new();
    if let Ok(Some(key)) = repos::integrations::internal_content_key(&state.db).await {
        internal = format!(
            r#"<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-green">{ic}</span>
<div class="body"><span class="title">Ghost Core Content API</span>
<div style="margin-top:6px">{chip}</div>
<span class="detail">curl "{site}/ghost/api/content/posts/?key={key}"</span></div></div>"#,
            ic = svg("code"),
            chip = key_chip("content", &key),
            site = esc(&state.site_url()),
            key = esc(&key),
        );
    }

    page_for_role(
        actor_role,
        "Integrations",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Integrations</h1><p class="sub">Built-in apps, API keys &amp; your own scripts</p></div>
<div class="actions"><a class="btn text" href="https://ghost.org/integrations/" target="_blank" rel="noopener">Browse integrations</a></div></header>
{banner}
<div class="section">Built-in apps</div>
<div class="card rows rise" style="--i:1">{apps}</div>
<div class="section">Custom integrations</div>
<div class="card rows rise" style="--i:2">{custom_rows}</div>
<div class="card padded rise" style="--i:3"><form method="post" action="/ghost/settings/integrations/">
<div class="section" style="margin-top:0">Add a custom integration</div>
<p class="muted" style="margin:0 0 12px">Creates a Content API key and an Admin API key you can use from your own code.</p>
<div class="inline-form">
{name_field}{desc_field}
<button class="btn" type="submit">{add} Add integration</button></div></form></div>
<div class="section">Content API</div>
<div class="card rows rise" style="--i:4">{internal}</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            apps = apps,
            custom_rows = custom_rows,
            name_field = text_field("name", "Name", ""),
            desc_field = text_field("description", "Description (optional)", ""),
            add = svg("add"),
            internal = internal,
        ),
        true,
    )
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct IntegrationForm {
    name: String,
    #[serde(default)]
    description: String,
}

pub(crate) async fn integration_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<IntegrationForm>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match repos::integrations::create_custom(&state.db, &form.name, Some(&form.description)).await {
        Ok(i) => {
            let _ = repos::actions::record(&state.db, "integrations", &i.id, &actor.user.id, "added").await;
            integrations_flash(&format!("custom/{}/", i.id), "ok", "Integration created.")
        }
        Err(e) => integrations_flash("", "error", &e.message()),
    }
}

/// One custom integration: rename it, read its keys, delete it.
pub(crate) async fn custom_integration_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(i)) = repos::integrations::find_by_id(&state.db, &id).await else {
        return not_found_page_for(actor_role, &user, "That integration does not exist.");
    };

    let mut keys = String::new();
    if let Ok(list) = repos::api_keys::find_by_integration(&state.db, &i.id).await {
        for k in list {
            // an admin key is an `id:secret` pair; both halves sign the JWT
            let shown = match k.key_type.as_str() {
                "admin" => format!("{}:{}", k.id, k.secret),
                _ => k.secret.clone(),
            };
            keys.push_str(&format!(
                r#"<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-teal">{ic}</span>
<div class="body"><span class="title">{label}</span><div style="margin-top:6px">{chip}</div></div></div>"#,
                ic = svg("code"),
                label = if k.key_type == "admin" { "Admin API key" } else { "Content API key" },
                chip = key_chip(&k.key_type, &shown),
            ));
        }
    }

    // Each webhook row reports its last delivery, which is the only way to
    // tell a listener that is working from one quietly failing.
    let hooks = repos::webhooks::list_for_integration(&state.db, &i.id).await.unwrap_or_default();
    let mut webhook_rows = String::new();
    for h in &hooks {
        let last = match (&h.last_triggered_at, &h.last_triggered_status) {
            (Some(at), Some(status)) => {
                let ok = status.starts_with('2');
                format!(
                    r#"<span class="chip {chip}">{status}</span><span class="detail" style="margin-left:8px">{at}{err}</span>"#,
                    chip = if ok { "published" } else { "draft" },
                    status = esc(status),
                    at = esc(&at[..16.min(at.len())]),
                    err = match h.last_triggered_error.as_deref().filter(|e| !e.is_empty()) {
                        Some(e) => format!(" · {}", esc(e)),
                        None => String::new(),
                    },
                )
            }
            _ => r#"<span class="detail">never fired</span>"#.to_string(),
        };
        webhook_rows.push_str(&format!(
            r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{label}{name}</span><span class="detail">{url}</span>
<div style="margin-top:6px">{last}</div></div>
<div class="trailing">
<form method="post" action="/ghost/settings/integrations/custom/{iid}/webhooks/{wid}/delete/" data-confirm="Delete webhook|Nothing will be sent to {url} when this event fires.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form></div></div>"#,
            ic = svg("share"),
            label = esc(ghost_api::webhooks::event_label(&h.event)),
            name = match h.name.as_deref().filter(|n| !n.is_empty()) {
                Some(n) => format!(" · {}", esc(n)),
                None => String::new(),
            },
            url = esc(&h.target_url),
            last = last,
            iid = esc(&i.id),
            wid = esc(&h.id),
            del = svg("delete"),
        ));
    }
    if webhook_rows.is_empty() {
        webhook_rows = r#"<div class="row"><div class="body"><span class="muted">No webhooks yet — add one below to be told when something changes.</span></div></div>"#.into();
    }

    page_for_role(
        actor_role,
        &i.name,
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>{name}</h1><p class="sub">Custom integration</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/custom/{id}/">
{name_field}{desc_field}
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>
<div class="section">API keys</div>
<div class="card rows rise" style="--i:2">{keys}</div>
<div class="card rows rise" style="--i:3"><div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-blue">{globe}</span>
<div class="body"><span class="title">API URL</span><div style="margin-top:6px">{api_chip}</div></div></div></div>
<div class="section">Webhooks</div>
<div class="card rows rise" style="--i:4">{webhook_rows}</div>
<div class="card padded rise" style="--i:4">
<form method="post" action="/ghost/settings/integrations/custom/{id}/webhooks/new/" class="inline-form">
<label class="field" style="min-width:160px;margin:0"><input name="name" placeholder=" " maxlength="191"><span>Name · optional</span></label>
<label class="field fixed" style="min-width:230px;margin:0"><select name="event" required>{event_opts}</select><span>Event</span>{car}</label>
<label class="field" style="flex:1;min-width:220px;margin:0"><input name="target_url" type="url" placeholder="https://example.com/hook" maxlength="2000" required><span>Target URL</span></label>
<label class="field" style="min-width:150px;margin:0"><input name="secret" placeholder=" " maxlength="191"><span>Secret · optional</span></label>
<button class="btn tonal" type="submit">{add} Add webhook</button>
</form>
<p class="muted" style="margin:12px 0 0">Ghost POSTs the changed resource as JSON when the event fires. Give a secret and each
delivery carries an <code>X-Ghost-Signature</code> header — HMAC-SHA256 of the body — so your endpoint can tell a real
delivery from anyone who learned the URL.</p>
</div>
<div class="section">Danger zone</div>
<div class="card padded rise" style="--i:4">
<form method="post" action="/ghost/settings/integrations/custom/{id}/delete/" data-confirm="Delete integration|&#8220;{name}&#8221; and its API keys will be removed. Anything using them will stop working.">
<button class="btn danger" type="submit">{del} Delete integration</button></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            name = esc(&i.name),
            id = esc(&i.id),
            name_field = text_field("name", "Name", &i.name),
            desc_field = text_field("description", "Description", i.description.as_deref().unwrap_or("")),
            keys = keys,
            globe = svg("globe"),
            api_chip = key_chip("url", &format!("{}/ghost/api", state.site_url())),
            webhook_rows = webhook_rows,
            event_opts = ghost_api::webhooks::EVENT_GROUPS
                .iter()
                .map(|(group, events)| format!(
                    r#"<optgroup label="{g}">{opts}</optgroup>"#,
                    g = esc(group),
                    opts = events
                        .iter()
                        .map(|(v, l)| format!(
                            r#"<option value="{v}">{l}</option>"#,
                            v = esc(v),
                            l = esc(l)
                        ))
                        .collect::<String>(),
                ))
                .collect::<String>(),
            car = SELECT_CARET,
            add = svg("add"),
            del = svg("delete"),
        ),
        true,
    )
    .into_response()
}

/// Fire the webhooks a post or page change should raise.
///
/// Ghost sends the resource under `{post: {current, previous}}`, and raises
/// both the generic `edited` and whichever publish transition happened — a
/// listener for "post published" should hear about it without also having to
/// subscribe to every edit and diff the status itself.
pub(crate) fn fire_post_webhooks(
    state: &AppState,
    post_type: &str,
    post: &ghost_core::models::Post,
    change: &str,
    was_published: bool,
) {
    fire_post_webhooks_from(state, post_type, post, change, was_published, "", None)
}

/// As above, but told what the status *was*, so the scheduling transitions
/// can be worked out rather than guessed.
///
/// `previous_status` empty means "not known", which is how the delete and
/// create paths call in — neither can schedule anything.
pub(crate) fn fire_post_webhooks_from(
    state: &AppState,
    post_type: &str,
    post: &ghost_core::models::Post,
    change: &str,
    was_published: bool,
    previous_status: &str,
    before: Option<&ghost_core::models::Post>,
) {
    ghost_api::federation::post_changed(state,&post.id,change=="deleted");
    let is_page = post_type == "page";
    let now_published = post.status == "published";
    let body = |p: &ghost_core::models::Post| {
        serde_json::json!({
            "id": p.id,
            "uuid": p.uuid,
            "title": p.title,
            "slug": p.slug,
            "status": p.status,
            "visibility": p.visibility,
            "published_at": p.published_at,
            "updated_at": p.updated_at,
            "excerpt": p.custom_excerpt,
            "url": format!("{}/{}/", state.site_url().trim_end_matches('/'), p.slug.clone().unwrap_or_default()),
        })
    };
    let key = if is_page { "page" } else { "post" };
    // Ghost sends the prior value of each field that changed, and nothing
    // for the fields that did not. A receiver diffing an edit needs that —
    // "the title changed, and here is what it was" is a different message
    // from "something about this post changed".
    let previous = match before {
        Some(old) => changed_fields(&body(old), &body(post)),
        None => serde_json::json!({}),
    };
    let payload = serde_json::json!({ key: { "current": body(post), "previous": previous } });

    let mut events: Vec<&'static str> = Vec::new();
    match change {
        "added" => events.push(if is_page { "page.added" } else { "post.added" }),
        "deleted" => events.push(if is_page { "page.deleted" } else { "post.deleted" }),
        _ => events.push(if is_page { "page.edited" } else { "post.edited" }),
    }
    if change != "deleted" {
        match (was_published, now_published) {
            (false, true) => events.push(if is_page { "page.published" } else { "post.published" }),
            (true, true) if change == "edited" => {
                events.push(if is_page { "page.published.edited" } else { "post.published.edited" })
            }
            (true, false) => {
                events.push(if is_page { "page.unpublished" } else { "post.unpublished" })
            }
            _ => {}
        }
        // Scheduling, as Ghost splits it. The old code raised a bare
        // `post.scheduled` for pages too, so a page listener heard nothing
        // and a post listener heard about pages — and it fired again on
        // every edit of an already-scheduled post, which is what
        // `rescheduled` is for.
        let was_scheduled = previous_status == "scheduled";
        let now_scheduled = post.status == "scheduled";
        match (was_scheduled, now_scheduled) {
            (false, true) if change != "added" => {
                events.push(if is_page { "page.scheduled" } else { "post.scheduled" })
            }
            (true, true) => {
                events.push(if is_page { "page.rescheduled" } else { "post.rescheduled" })
            }
            (true, false) => {
                events.push(if is_page { "page.unscheduled" } else { "post.unscheduled" })
            }
            _ => {}
        }
    }
    for event in events {
        ghost_api::webhooks::fire(state, event, payload.clone());
    }
}

/// The entries of `before` whose value differs in `after`.
///
/// Ghost's `previous` is exactly this: the old values of the fields that
/// moved, not a whole second copy of the resource. Sending the lot would
/// make every edit look like every field changed.
pub(crate) fn changed_fields(before: &serde_json::Value, after: &serde_json::Value) -> serde_json::Value {
    let (Some(b), Some(a)) = (before.as_object(), after.as_object()) else {
        return serde_json::json!({});
    };
    let mut out = serde_json::Map::new();
    for (k, old) in b {
        if a.get(k) != Some(old) {
            out.insert(k.clone(), old.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// `post.tag.attached` / `post.tag.detached`, and the page equivalents.
///
/// Ghost raises one event per tag that joined or left, carrying both the
/// entry and the tag — a listener wants to know *which* tag moved, which a
/// bare `post.edited` cannot tell it. These four were in the picker and had
/// no code behind them at all.
pub(crate) async fn fire_tag_link_webhooks(
    state: &AppState,
    post_type: &str,
    post: &ghost_core::models::Post,
    before: &[String],
    after: &[String],
) {
    let attached: Vec<&String> = after.iter().filter(|t| !before.contains(t)).collect();
    let detached: Vec<&String> = before.iter().filter(|t| !after.contains(t)).collect();
    if attached.is_empty() && detached.is_empty() {
        return;
    }
    let is_page = post_type == "page";
    let key = if is_page { "page" } else { "post" };
    for (tag_ids, event) in [
        (attached, if is_page { "page.tag.attached" } else { "post.tag.attached" }),
        (detached, if is_page { "page.tag.detached" } else { "post.tag.detached" }),
    ] {
        for tag_id in tag_ids {
            // A detached tag still exists, so it can be described in full.
            let Ok(Some(tag)) = repos::tags::find_by_id(&state.db, tag_id).await else {
                continue;
            };
            ghost_api::webhooks::fire(
                state,
                event,
                serde_json::json!({
                    key: { "current": { "id": post.id, "title": post.title, "slug": post.slug } },
                    "tag": { "current": { "id": tag.id, "name": tag.name, "slug": tag.slug } },
                }),
            );
        }
    }
}

/// `site.changed` — Ghost's whole-site invalidation event.
///
/// Ghost raises it whenever a response carries `X-Cache-Invalidate: /*`,
/// which is its way of saying the rendered site is now different. This port
/// has no such header, so it is raised from the saves that mean the same
/// thing: settings, navigation, code injection, the active theme and the
/// routing files. Cache purges and static rebuilds hang off this event, so a
/// site that never fires it serves stale pages until something else evicts
/// them.
pub(crate) fn fire_site_changed(state: &AppState) {
    ghost_api::webhooks::fire(state, "site.changed", serde_json::json!({}));
}

/// A settings save that changed the rendered site: flash as usual, and tell
/// the integrations.
pub(crate) fn settings_saved(state: &AppState, group: &str, message: &str) -> Response {
    fire_site_changed(state);
    settings_flash(group, "ok", message)
}

/// Tags, in Ghost's `{tag: {current, previous}}` shape.
///
/// The three tag events were in the picker from the start and never fired,
/// so a listener set up against one simply never heard anything.
pub(crate) fn fire_tag_webhook(state: &AppState, event: &'static str, tag: &ghost_core::models::Tag) {
    ghost_api::webhooks::fire(
        state,
        event,
        serde_json::json!({
            "tag": {
                "current": {
                    "id": tag.id,
                    "name": tag.name,
                    "slug": tag.slug,
                    "description": tag.description,
                    "visibility": tag.visibility,
                },
                "previous": {}
            }
        }),
    );
}

/// The members' equivalent, delegating to the shared builder so the admin's
/// form and Portal signup emit the identical shape.
pub(crate) fn fire_member_webhook(state: &AppState, event: &'static str, id: &str, email: &str, name: &str) {
    ghost_api::webhooks::fire_member(state, event, id, email, name, serde_json::json!({}));
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct WebhookForm {
    name: String,
    event: String,
    target_url: String,
    secret: String,
}

pub(crate) fn integration_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("/ghost/settings/integrations/custom/{id}/?{encoded}"))
        .into_response()
}

pub(crate) async fn webhook_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<WebhookForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // An event this build cannot fire would be registered and never heard
    // from, which is worse than refusing it.
    if !ghost_api::webhooks::is_known_event(form.event.trim()) {
        return integration_flash(&id, "error", "Choose an event from the list.");
    }
    let url = form.target_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return integration_flash(&id, "error", "The target URL must start with http:// or https://.");
    }
    match repos::webhooks::create(
        &state.db,
        &id,
        form.event.trim(),
        url,
        form.name.trim(),
        form.secret.trim(),
    )
    .await
    {
        Ok(_) => integration_flash(&id, "ok", "Webhook added."),
        Err(e) => integration_flash(&id, "error", &e.message()),
    }
}

pub(crate) async fn webhook_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, wid)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::webhooks::delete(&state.db, &wid).await {
        Ok(()) => integration_flash(&id, "ok", "Webhook deleted."),
        Err(e) => integration_flash(&id, "error", &e.message()),
    }
}

pub(crate) async fn custom_integration_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<IntegrationForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let sub = format!("custom/{id}/");
    match repos::integrations::update_custom(&state.db, &id, &form.name, Some(&form.description)).await {
        Ok(()) => integrations_flash(&sub, "ok", "Integration saved."),
        Err(e) => integrations_flash(&sub, "error", &e.message()),
    }
}

pub(crate) async fn custom_integration_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match repos::integrations::delete_custom(&state.db, &id).await {
        Ok(()) => {
            let _ = repos::actions::record(&state.db, "integrations", &id, &actor.user.id, "deleted").await;
            integrations_flash("", "ok", "Integration deleted.")
        }
        Err(e) => integrations_flash(&format!("custom/{id}/"), "error", &e.message()),
    }
}
