//! Tools → Social → **Social media**: the accounts this site posts to on
//! other platforms, what it shares to them, and the share buttons readers
//! see under a post.
//!
//! The Social tool has two halves and this is the second. The first, the
//! **social web** (`crate::social`), makes the publication itself an account
//! on the fediverse that people follow directly. This half is different in
//! kind: the site holds accounts elsewhere — X, Bluesky, Mastodon, Threads,
//! LinkedIn, a Facebook Page, a Telegram channel, a Discord channel — and
//! posts links to its own posts there. [`section_switch`] draws the two side
//! by side at the top of every screen of both, saying in one line each what
//! they are, so nobody mistakes one for the other.
//!
//! Screens (all under `/ghost/social/media/`):
//!
//! * **Accounts** — connect, test, auto-share on or off, a per-account
//!   template, disconnect.
//! * **Compose** — share a published post now or at a set time, with each
//!   account's text pre-written from its template and checked against the
//!   network's limit.
//! * **History** — every share: sent (with a link to it), scheduled, failed
//!   (with the platform's reason and Retry).
//! * **Settings** — the default template, campaign tags, hashtags, and the
//!   share buttons under posts.
//!
//! The rules are [`vuuut_core::social_media`]; the platform calls are
//! `vuuut_api::social_media`; the records are
//! [`vuuut_db::repos::social_media`].
//!
//! ## The security contract
//!
//! Connecting, changing or disconnecting an account is Administrator and
//! above, because the credentials can post as the publication. Composing and
//! reading history is Editor and above, like the social web. Every write is
//! origin-checked. Stored secrets are never rendered back — an edit form
//! shows that a secret is set and a blank field keeps it. Remote links in
//! the history are shown only when they are http(s).

use super::*;
use vuuut_core::social_media::{self as rules, Network, ShareButton, ShareSettings};
use vuuut_db::repos::social_media as repo;

const BASE: &str = "/ghost/social/media/";

async fn guard(state: &AppState, headers: &HeaderMap, role: Role, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, role).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

/// A redirect with a flash. The composer's address already carries `?post=`,
/// so the flash is joined with `&` there — `tool_flash` assumes a bare path.
fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

fn share_settings(state: &AppState) -> ShareSettings {
    ShareSettings::read(|k| state.settings.get_str(k))
}

// ---------------------------------------------------------------------------
// the two halves of Social, side by side
// ---------------------------------------------------------------------------

/// The header both halves of the Social tool share: what each is, and which
/// one you are in. `active` is `"web"` or `"media"`.
pub(crate) fn section_switch(active: &str) -> String {
    let card = |key: &str, href: &str, icon: &str, tint: &str, title: &str, what: &str, how: &str| {
        let on = key == active;
        format!(
            r#"<a class="sm-section{on}" href="{href}"{current}>
<span class="ic tint-{tint}">{ic}</span>
<span class="sm-section-text"><b>{title}</b><span>{what}</span><small>{how}</small></span>
{mark}</a>"#,
            on = if on { " on" } else { "" },
            current = if on { r#" aria-current="page""# } else { "" },
            ic = svg(icon),
            title = esc(title),
            what = esc(what),
            how = esc(how),
            mark = if on { r#"<span class="sm-section-mark">You are here</span>"# } else { "" },
        )
    };
    format!(
        r#"<header class="page-head rise"><div><h1>Social</h1>
<p class="sub">Two different ways your publication reaches people on social platforms.</p></div></header>
<nav class="sm-sections rise" aria-label="Social sections">
{web}
{media}
</nav>"#,
        web = card(
            "web",
            "/ghost/social/",
            "globe",
            "teal",
            "Social web",
            "Your publication is its own account on the fediverse.",
            "People on Mastodon, Threads and other ActivityPub apps follow the site itself. Nothing to connect.",
        ),
        media = card(
            "media",
            BASE,
            "share",
            "pink",
            "Social media",
            "Your accounts on X, Bluesky, LinkedIn, Facebook and more.",
            "Connect them once; new posts are shared to them, and readers get share buttons.",
        ),
    )
}

fn tabs(active: &str, admin: bool) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}{d}</div>"#,
        a = tab("accounts", "Accounts", BASE),
        b = tab("compose", "Share a post", "/ghost/social/media/compose/"),
        c = tab("history", "History", "/ghost/social/media/history/"),
        d = if admin { tab("settings", "Settings & share buttons", "/ghost/social/media/settings/") } else { String::new() },
    )
}

fn badge(network: Option<Network>) -> String {
    match network {
        Some(n) => format!(r#"<span class="sm-badge sm-{key} tint-{tint}" aria-hidden="true">{mark}</span>"#, key = n.key(), tint = n.tint(), mark = esc(n.mark())),
        None => r#"<span class="sm-badge tint-slate" aria-hidden="true">?</span>"#.to_string(),
    }
}

fn status_chip(status: &str) -> (&'static str, &'static str) {
    match status {
        "sent" => ("published", "Sent"),
        "failed" => ("error", "Failed"),
        "scheduled" => ("scheduled", "Scheduled"),
        "sending" | "queued" => ("tonal", "Sending"),
        "ok" => ("published", "Working"),
        "error" => ("error", "Needs attention"),
        _ => ("tonal", "Not tested"),
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

pub(crate) async fn accounts_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let admin = actor.role.is_admin();
    let accounts = repo::accounts(&state.db).await.unwrap_or_default();
    let counts: HashMap<String, i64> = repo::counts(&state.db).await.into_iter().collect();

    let rows: String = accounts
        .iter()
        .map(|a| {
            let network = Network::from_key(&a.network);
            let (chip, label) = status_chip(&a.status);
            let controls = if admin {
                format!(
                    r#"<form method="post" action="{BASE}accounts/{id}/auto/" class="sm-auto"><input type="hidden" name="on" value="{next}">
<button type="submit" class="sm-toggle{on}" aria-pressed="{pressed}" title="Share new posts automatically">{dot}<span>Auto-share {state_word}</span></button></form>
<form method="post" action="{BASE}accounts/{id}/test/"><button class="btn text small" type="submit">{bolt} Test</button></form>
<a class="btn text small" href="{BASE}accounts/{id}/">{edit} Edit</a>"#,
                    id = esc(&a.id),
                    next = if a.shares_automatically() { "0" } else { "1" },
                    on = if a.shares_automatically() { " on" } else { "" },
                    pressed = a.shares_automatically(),
                    dot = r#"<i></i>"#,
                    state_word = if a.shares_automatically() { "on" } else { "off" },
                    bolt = svg("check"),
                    edit = svg("edit"),
                )
            } else {
                format!(r#"<span class="chip tonal">Auto-share {}</span>"#, if a.shares_automatically() { "on" } else { "off" })
            };
            format!(
                r#"<div class="row sm-account">{badge}
<div class="body"><span class="title">{name} <span class="chip {chip}">{label}</span></span>
<span class="detail">{network}{err}</span></div>
<div class="trailing sm-controls">{controls}</div></div>"#,
                badge = badge(network),
                name = esc(if a.name.is_empty() { "Account" } else { &a.name }),
                network = esc(network.map(Network::label).unwrap_or("Unknown network")),
                err = if a.status == "error" && !a.last_error.is_empty() { format!(" — {}", esc(&a.last_error)) } else { String::new() },
            )
        })
        .collect();
    let list = if accounts.is_empty() {
        r#"<div class="card padded sm-empty"><b>No accounts connected yet.</b><p class="muted">Connect one below and new posts can be shared to it automatically, or by hand from “Share a post”.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };

    let connect = if admin {
        let tiles: String = Network::ALL
            .iter()
            .map(|n| {
                let fields: String = n
                    .fields()
                    .iter()
                    .map(|(name, label, secret, hint)| {
                        format!(
                            r#"<label class="field"><input name="{name}" type="{kind}" autocomplete="off" spellcheck="false" placeholder=" "{value}><span>{label}</span></label>{hint}"#,
                            kind = if *secret { "password" } else { "text" },
                            value = if *name == "pds" { r#" value="https://bsky.social""# } else { "" },
                            label = esc(label),
                            hint = if hint.is_empty() { String::new() } else { format!(r#"<p class="sm-hint">{}</p>"#, esc(hint)) },
                        )
                    })
                    .collect();
                format!(
                    r#"<details class="sm-connect"><summary>{badge}<b>{label}</b><span class="sm-limit">{limit} characters</span>{chev}</summary>
<form method="post" action="{BASE}accounts/" class="sm-connect-form"><input type="hidden" name="network" value="{key}">
{fields}
<label class="sm-check"><input type="checkbox" name="auto_share" value="1" checked> Share new posts automatically</label>
<div class="sm-connect-actions"><button class="btn" type="submit">{plus} Connect and test</button></div></form></details>"#,
                    badge = badge(Some(*n)),
                    label = esc(n.label()),
                    limit = n.limit(),
                    chev = svg("chevron"),
                    key = n.key(),
                    plus = svg("add"),
                )
            })
            .collect();
        format!(
            r#"<div class="section">Connect an account</div>
<p class="muted sm-lead">Each platform gives you a key or token for your own account. Paste it once; it is stored on this server only, never shown again, and used only to post your links.</p>
<div class="sm-connects rise" style="--i:3">{tiles}</div>"#
        )
    } else {
        r#"<p class="muted sm-lead">An administrator connects and manages accounts.</p>"#.to_string()
    };

    let n = |k: &str| counts.get(k).copied().unwrap_or(0);
    page_for_role(
        actor.role,
        "Social media",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{switch}{tabs}{banner}
<div class="sm-tiles rise" style="--i:2">
<div class="card padded sm-tile"><span class="sm-tile-value">{accounts}</span><span class="sm-tile-label">Connected</span></div>
<div class="card padded sm-tile"><span class="sm-tile-value">{auto}</span><span class="sm-tile-label">Sharing automatically</span></div>
<div class="card padded sm-tile"><span class="sm-tile-value">{sent}</span><span class="sm-tile-label">Shares sent</span></div>
<a class="card padded sm-tile" href="{BASE}history/?status=failed"><span class="sm-tile-value{fail_cls}">{failed}</span><span class="sm-tile-label">Failed</span></a>
<a class="card padded sm-tile" href="{BASE}history/?status=scheduled"><span class="sm-tile-value">{scheduled}</span><span class="sm-tile-label">Scheduled</span></a>
</div>
<div class="section">Your accounts</div>
<div class="rise" style="--i:2">{list}</div>
{connect}"#,
            switch = section_switch("media"),
            tabs = tabs("accounts", admin),
            banner = flash_banner(&q),
            accounts = accounts.len(),
            auto = accounts.iter().filter(|a| a.shares_automatically()).count(),
            sent = n("sent"),
            failed = n("failed"),
            fail_cls = if n("failed") > 0 { " sm-bad" } else { "" },
            scheduled = n("scheduled"),
        ),
        true,
    )
    .into_response()
}

/// A form's values, for `rules::credentials`.
fn lookup(form: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
    move |k| form.get(k).cloned()
}

pub(crate) async fn account_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let Some(network) = form.get("network").and_then(|n| Network::from_key(n)) else {
        return flash_to(BASE, "error", "Choose a network to connect.");
    };
    let creds = match rules::credentials(network, &lookup(&form), None) {
        Ok(c) => c,
        Err(m) => return flash_to(BASE, "error", &m),
    };
    let name = rules::display_name(network, &creds);
    let auto = form.get("auto_share").is_some_and(|v| v == "1");
    let id = match repo::add_account(&state.db, network.key(), &name, &creds, auto).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("social media: connecting {} failed: {}", network.key(), e.message());
            return flash_to(BASE, "error", "The account could not be saved.");
        }
    };
    // Tested straight away, so a mistyped token is found now rather than on
    // the next publish.
    match vuuut_api::social_media::test(&state, network, &creds).await {
        Ok(who) => {
            repo::set_health(&state.db, &id, true, "", Some(&who)).await;
            flash_to(BASE, "ok", &format!("{} connected as {who}.", network.label()))
        }
        Err(e) => {
            repo::set_health(&state.db, &id, false, &e, None).await;
            flash_to(BASE, "error", &format!("Saved, but the test failed: {e}"))
        }
    }
}

async fn load_account(state: &AppState, id: &str) -> Option<(repo::Account, Network)> {
    if !vuuut_core::ids::is_objectid(id) {
        return None;
    }
    let a = repo::account(&state.db, id).await.ok().flatten()?;
    let n = Network::from_key(&a.network)?;
    Some((a, n))
}

pub(crate) async fn account_test(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let Some((a, network)) = load_account(&state, &id).await else {
        return flash_to(BASE, "error", "That account could not be found.");
    };
    match vuuut_api::social_media::test(&state, network, &a.creds()).await {
        Ok(who) => {
            repo::set_health(&state.db, &id, true, "", Some(&who)).await;
            flash_to(BASE, "ok", &format!("{} works — signed in as {who}.", network.label()))
        }
        Err(e) => {
            repo::set_health(&state.db, &id, false, &e, None).await;
            flash_to(BASE, "error", &e)
        }
    }
}

pub(crate) async fn account_auto(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let Some((a, _)) = load_account(&state, &id).await else {
        return flash_to(BASE, "error", "That account could not be found.");
    };
    let on = form.get("on").is_some_and(|v| v == "1");
    match repo::update_account(&state.db, &id, &a.name, &a.creds(), on, &a.template).await {
        Ok(()) => flash_to(BASE, "ok", if on { "New posts will be shared automatically." } else { "Automatic sharing is off for that account." }),
        Err(_) => flash_to(BASE, "error", "That could not be saved."),
    }
}

pub(crate) async fn account_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some((a, network)) = load_account(&state, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That account could not be found.");
    };
    let creds = a.creds();
    let fields: String = network
        .fields()
        .iter()
        .map(|(name, label, secret, hint)| {
            let shown = if *secret { String::new() } else { creds.get(*name).and_then(|v| v.as_str()).unwrap_or("").to_string() };
            format!(
                r#"<label class="field"><input name="{name}" type="{kind}" autocomplete="off" spellcheck="false" placeholder="{ph}" value="{v}"><span>{label}</span></label>{hint}"#,
                kind = if *secret { "password" } else { "text" },
                // A stored secret is never sent back; the placeholder says
                // one is set, and a blank field keeps it.
                ph = if *secret { "•••••••• saved — leave blank to keep" } else { " " },
                v = esc(&shown),
                label = esc(label),
                hint = if hint.is_empty() { String::new() } else { format!(r#"<p class="sm-hint">{}</p>"#, esc(hint)) },
            )
        })
        .collect();
    let defaults = share_settings(&state);
    page_for_role(
        actor.role,
        &a.name,
        "tools",
        Some(&actor.user),
        &format!(
            r#"{switch}{banner}
<div class="sm-account-head rise" style="--i:1"><a class="icon-btn" style="transform:rotate(180deg)" href="{BASE}" title="Back to accounts">{back}</a>{badge}<div><h2>{name}</h2><p class="muted">{network}</p></div></div>
<form method="post" action="{BASE}accounts/{id}/" class="card padded rise sm-edit" style="--i:2">
<div class="section" style="margin-top:0">Connection</div>
{fields}
<div class="section">What it posts</div>
<label class="field"><textarea name="template" rows="4" placeholder="{default_t}">{template}</textarea><span>This account's template</span></label>
<p class="sm-hint">Blank uses the site's template. You can use {{title}}, {{excerpt}}, {{url}}, {{hashtags}} and {{site}}. Shortened to {limit} characters if needed — the link is never cut.</p>
<label class="sm-check"><input type="checkbox" name="auto_share" value="1"{auto}> Share new posts automatically</label>
<div class="sm-edit-actions"><button class="btn tonal small danger" type="submit" form="sm-delete">{del} Disconnect</button><button class="btn" type="submit">{check} Save and test</button></div>
</form>
<form method="post" action="{BASE}accounts/{id}/delete/" id="sm-delete" data-confirm="Disconnect {name}|Its credentials and its share history are deleted here. Nothing already posted on {network} is touched."></form>"#,
            switch = section_switch("media"),
            banner = flash_banner(&q),
            back = svg("chevron"),
            badge = badge(Some(network)),
            name = esc(&a.name),
            network = esc(network.label()),
            id = esc(&a.id),
            default_t = esc(&defaults.template.replace('\n', " ⏎ ")),
            template = esc(&a.template),
            limit = network.limit(),
            auto = if a.shares_automatically() { " checked" } else { "" },
            del = svg("delete"),
            check = svg("check"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn account_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let back = format!("{BASE}accounts/{id}/");
    let Some((a, network)) = load_account(&state, &id).await else {
        return flash_to(BASE, "error", "That account could not be found.");
    };
    let old = a.creds();
    let creds = match rules::credentials(network, &lookup(&form), Some(&old)) {
        Ok(c) => c,
        Err(m) => return flash_to(&back, "error", &m),
    };
    let template: String = form
        .get("template")
        .map(|t| t.chars().filter(|c| !c.is_control() || *c == '\n').take(rules::MAX_TEMPLATE).collect::<String>().trim().to_string())
        .unwrap_or_default();
    let auto = form.get("auto_share").is_some_and(|v| v == "1");
    if repo::update_account(&state.db, &id, &a.name, &creds, auto, &template).await.is_err() {
        return flash_to(&back, "error", "That could not be saved.");
    }
    match vuuut_api::social_media::test(&state, network, &creds).await {
        Ok(who) => {
            repo::set_health(&state.db, &id, true, "", Some(&who)).await;
            flash_to(&back, "ok", &format!("Saved. Signed in as {who}."))
        }
        Err(e) => {
            repo::set_health(&state.db, &id, false, &e, None).await;
            flash_to(&back, "error", &format!("Saved, but the test failed: {e}"))
        }
    }
}

pub(crate) async fn account_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if load_account(&state, &id).await.is_none() {
        return flash_to(BASE, "error", "That account could not be found.");
    }
    match repo::delete_account(&state.db, &id).await {
        Ok(()) => flash_to(BASE, "ok", "Account disconnected."),
        Err(_) => flash_to(BASE, "error", "That could not be disconnected."),
    }
}

// ---------------------------------------------------------------------------
// composing a share
// ---------------------------------------------------------------------------

/// The text an account would post for this post, from its template.
pub(crate) async fn text_for(state: &AppState, account: &repo::Account, post: &vuuut_core::models::Post, campaign: &str) -> String {
    let network = Network::from_key(&account.network).unwrap_or(Network::Mastodon);
    let s = share_settings(state);
    let url = crate::languages::public_url(state, post).await;
    let url = if s.utm { rules::with_utm(&url, network, campaign) } else { url };
    let tags = if s.hashtags { rules::hashtags(&repo::post_tag_names(&state.db, &post.id).await) } else { String::new() };
    let vars = rules::Vars {
        title: post.title.clone().unwrap_or_default(),
        excerpt: post
            .custom_excerpt
            .clone()
            .or_else(|| vuuut_core::models::excerpt_from_plaintext(post.plaintext.as_deref(), 40))
            .unwrap_or_default(),
        url,
        site: state.settings.get_str("title").unwrap_or_default(),
        hashtags: tags,
    };
    let template = if account.template.trim().is_empty() { s.template.as_str() } else { account.template.as_str() };
    rules::compose(template, &vars, network)
}

pub(crate) async fn compose_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let accounts = repo::accounts(&state.db).await.unwrap_or_default();
    let recent = repo::recent_posts(&state.db, 60).await;
    let chosen = q.get("post").filter(|p| vuuut_core::ids::is_objectid(p)).cloned().or_else(|| recent.first().map(|r| r.0.clone()));
    let post = match &chosen {
        Some(id) => vuuut_db::repos::posts::find_by_id(&state.db, id).await.ok().flatten().filter(|p| matches!(p.status.as_str(), "published" | "sent")),
        None => None,
    };
    let picker: String = recent
        .iter()
        .map(|(id, title, at)| {
            format!(
                r#"<option value="{id}"{sel}>{t} — {d}</option>"#,
                id = esc(id),
                sel = if chosen.as_deref() == Some(id.as_str()) { " selected" } else { "" },
                t = esc(title.as_deref().unwrap_or("(Untitled)")),
                d = esc(at.as_deref().and_then(|d| d.get(..10)).unwrap_or("")),
            )
        })
        .collect();

    let body = match (&post, accounts.is_empty()) {
        (_, true) => format!(r#"<div class="card padded sm-empty"><b>No accounts to share to.</b><p class="muted">Connect one on <a href="{BASE}">Accounts</a> first.</p></div>"#),
        (None, _) => r#"<div class="card padded sm-empty"><b>Nothing published yet.</b><p class="muted">Shares link to a published post. Publish one, then come back.</p></div>"#.to_string(),
        (Some(p), false) => {
            let earlier = repo::of_post(&state.db, &p.id).await;
            let mut rows = String::new();
            for a in &accounts {
                let network = Network::from_key(&a.network);
                let text = text_for(&state, a, p, "share").await;
                let done = earlier.iter().filter(|s| s.account_id == a.id && s.status == "sent").count();
                rows.push_str(&format!(
                    r#"<div class="sm-draft" data-limit="{limit}" data-link="{link}">
<label class="sm-draft-head"><input type="checkbox" name="account" value="{id}"{checked}>{badge}<b>{name}</b><span class="muted">{network}</span>{done}</label>
<label class="field" style="margin:0"><textarea name="text_{id}" rows="4" maxlength="5000" placeholder=" ">{text}</textarea><span>What it says</span></label>
<div class="sm-count"><span data-count>{weight}</span> / {limit}</div></div>"#,
                    limit = network.map(Network::limit).unwrap_or(500),
                    link = network.and_then(Network::link_weight).unwrap_or(0),
                    id = esc(&a.id),
                    checked = if a.status != "error" { " checked" } else { "" },
                    badge = badge(network),
                    name = esc(&a.name),
                    network = esc(network.map(Network::label).unwrap_or("")),
                    done = if done > 0 { format!(r#"<span class="chip published">Shared {done}×</span>"#) } else { String::new() },
                    text = esc(&text),
                    weight = network.map(|n| rules::weight(&text, n)).unwrap_or(0),
                ));
            }
            format!(
                r#"<form method="post" action="{BASE}compose/" class="rise" style="--i:3"><input type="hidden" name="post" value="{post}">
<div class="sm-drafts">{rows}</div>
<div class="card padded sm-send">
<label class="field" style="margin:0;max-width:280px"><input type="datetime-local" name="send_at" placeholder=" "><span>Send later (UTC) — blank sends now</span></label>
<button class="btn" type="submit">{send} Share</button></div>
</form>"#,
                post = esc(&p.id),
                send = svg("send"),
            )
        }
    };

    page_for_role(
        actor.role,
        "Share a post",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{switch}{tabs}{banner}
<form method="get" action="{BASE}compose/" class="sm-pick rise" style="--i:2">
<label class="field fixed" style="margin:0;flex:1"><select name="post" onchange="this.form.submit()">{picker}</select><span>Post to share</span>{caret}</label>
<noscript><button class="btn tonal" type="submit">Choose</button></noscript></form>
{body}
<script src="{assets}/social-compose.js?v={hash}" defer></script>"#,
            switch = section_switch("media"),
            tabs = tabs("compose", actor.role.is_admin()),
            banner = flash_banner(&q),
            caret = SELECT_CARET,
            assets = ASSETS,
            hash = compose_script_hash(),
        ),
        true,
    )
    .into_response()
}

fn compose_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/social-compose.js")));
    &HASH
}

pub(crate) async fn compose_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    // Parsed by hand: the account checkboxes repeat one name.
    let mut form: HashMap<String, String> = HashMap::new();
    let mut chosen: Vec<String> = Vec::new();
    for (k, v) in form_urlencoded::parse(body.as_bytes()) {
        if k == "account" {
            chosen.push(v.into_owned());
        } else {
            form.insert(k.into_owned(), v.into_owned());
        }
    }
    let post_id = form.get("post").cloned().unwrap_or_default();
    let back = format!("{BASE}compose/?post={}", if vuuut_core::ids::is_objectid(&post_id) { post_id.as_str() } else { "" });
    if !vuuut_core::ids::is_objectid(&post_id) {
        return flash_to(&back, "error", "Choose a post to share.");
    }
    let Ok(Some(post)) = vuuut_db::repos::posts::find_by_id(&state.db, &post_id).await else {
        return flash_to(&back, "error", "That post could not be found.");
    };
    if !matches!(post.status.as_str(), "published" | "sent") {
        return flash_to(&back, "error", "Only a published post can be shared.");
    }
    if chosen.is_empty() {
        return flash_to(&back, "error", "Tick at least one account.");
    }
    let send_at = form.get("send_at").map(|v| v.trim()).filter(|v| !v.is_empty()).map(|v| {
        chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M").map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
    });
    let send_at = match send_at {
        None => None,
        Some(Ok(at)) if at > vuuut_core::dates::now_db() => Some(at),
        Some(Ok(_)) => return flash_to(&back, "error", "That time has passed. Leave it blank to send now."),
        Some(Err(_)) => return flash_to(&back, "error", "That date and time could not be read."),
    };
    let accounts = repo::accounts(&state.db).await.unwrap_or_default();
    let mut queued = 0;
    for id in chosen.iter().filter(|id| vuuut_core::ids::is_objectid(id)) {
        let Some(a) = accounts.iter().find(|a| &a.id == id) else { continue };
        let Some(network) = Network::from_key(&a.network) else { continue };
        let text: String = form.get(&format!("text_{id}")).map(|t| t.trim().to_string()).unwrap_or_default();
        if text.is_empty() {
            return flash_to(&back, "error", &format!("The text for {} is empty.", a.name));
        }
        if rules::weight(&text, network) > network.limit() {
            return flash_to(&back, "error", &format!("The text for {} is over {}'s {} characters.", a.name, network.label(), network.limit()));
        }
        match repo::add_share(&state.db, &a.id, &post.id, &text, "manual", send_at.as_deref(), &actor.user.id).await {
            Ok(share_id) => {
                if send_at.is_none() {
                    vuuut_api::social_media::spawn_send(&state, share_id);
                }
                queued += 1;
            }
            Err(e) => tracing::error!("social media: queueing a share failed: {}", e.message()),
        }
    }
    let message = match (send_at, queued) {
        (_, 0) => return flash_to(&back, "error", "Nothing was shared."),
        (Some(at), n) => format!("{} scheduled for {at} UTC.", countable(n, "share")),
        (None, n) => format!("Sharing to {} now. Results appear in History.", countable(n, "account")),
    };
    flash_to(&format!("{BASE}history/"), "ok", &message)
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

pub(crate) async fn history_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let status = match q.get("status").map(String::as_str) {
        Some(s @ ("sent" | "failed" | "scheduled")) => Some(s),
        _ => None,
    };
    let shares = repo::history(&state.db, status, 200).await.unwrap_or_default();
    let accounts: HashMap<String, repo::Account> = repo::accounts(&state.db).await.unwrap_or_default().into_iter().map(|a| (a.id.clone(), a)).collect();
    let tz = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let mut rows = String::new();
    for s in &shares {
        let account = accounts.get(&s.account_id);
        let network = account.and_then(|a| Network::from_key(&a.network));
        let title = vuuut_db::repos::posts::find_by_id(&state.db, &s.post_id).await.ok().flatten().and_then(|p| p.title).unwrap_or_else(|| "A deleted post".into());
        let (chip, label) = status_chip(&s.status);
        let when = s.sent_at.as_deref().or(s.send_at.as_deref()).unwrap_or(&s.created_at);
        let remote = if s.remote_url.starts_with("https://") || s.remote_url.starts_with("http://") {
            format!(r#"<a class="btn text small" href="{u}" target="_blank" rel="noopener noreferrer">{ic} View</a>"#, u = esc(&s.remote_url), ic = svg("open"))
        } else {
            String::new()
        };
        let actions = match s.status.as_str() {
            "failed" => format!(
                r#"<form method="post" action="{BASE}shares/{id}/retry/"><button class="btn tonal small" type="submit">{ic} Retry</button></form>
<form method="post" action="{BASE}shares/{id}/cancel/"><button class="btn text small" type="submit">Dismiss</button></form>"#,
                id = esc(&s.id),
                ic = svg("repeat"),
            ),
            "scheduled" => format!(
                r#"<form method="post" action="{BASE}shares/{id}/cancel/" data-confirm="Cancel this share|It will not be posted."><button class="btn text small" type="submit">Cancel</button></form>"#,
                id = esc(&s.id),
            ),
            _ => String::new(),
        };
        rows.push_str(&format!(
            r#"<div class="row sm-share">{badge}
<div class="body"><span class="title">{title} <span class="chip {chip}">{label}</span>{auto}</span>
<span class="detail">{account} · {when}</span>
<span class="sm-share-text">{text}</span>{err}</div>
<div class="trailing sm-controls">{remote}{actions}</div></div>"#,
            badge = badge(network),
            title = esc(&title),
            auto = if s.trigger_kind == "auto" { r#" <span class="chip tonal">Automatic</span>"# } else { "" },
            account = esc(account.map(|a| a.name.as_str()).unwrap_or("A disconnected account")),
            when = esc(&format!("{} {}", vuuut_core::dates::short_date_in_zone(when, &tz), vuuut_core::dates::clock_in_zone(when, &tz))),
            text = esc(&s.body),
            err = if s.status == "failed" && !s.error.is_empty() { format!(r#"<span class="sm-err">{}</span>"#, esc(&s.error)) } else { String::new() },
        ));
    }
    let filter = |key: Option<&str>, label: &str| {
        format!(
            r#"<a class="wf-chip{on}" href="{BASE}history/{q}">{label}</a>"#,
            on = if status == key { " on" } else { "" },
            q = key.map(|k| format!("?status={k}")).unwrap_or_default(),
            label = esc(label),
        )
    };
    let list = if shares.is_empty() {
        r#"<div class="card padded sm-empty"><p class="muted" style="margin:0">Nothing here yet.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };
    page_for_role(
        actor.role,
        "Share history",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{switch}{tabs}{banner}
<div class="wf-chips sm-filters rise" style="--i:2">{all}{sent}{scheduled}{failed}</div>
<div class="rise" style="--i:3">{list}</div>"#,
            switch = section_switch("media"),
            tabs = tabs("history", actor.role.is_admin()),
            banner = flash_banner(&q),
            all = filter(None, "Everything"),
            sent = filter(Some("sent"), "Sent"),
            scheduled = filter(Some("scheduled"), "Scheduled"),
            failed = filter(Some("failed"), "Failed"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn share_retry(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{BASE}history/");
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(&back, "error", "That share could not be found.");
    }
    match repo::share(&state.db, &id).await {
        Ok(Some(s)) if s.status == "failed" => {
            vuuut_api::social_media::spawn_send(&state, id);
            flash_to(&back, "ok", "Trying again.")
        }
        _ => flash_to(&back, "error", "Only a failed share can be retried."),
    }
}

pub(crate) async fn share_cancel(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{BASE}history/");
    if vuuut_core::ids::is_objectid(&id) && repo::cancel(&state.db, &id).await {
        flash_to(&back, "ok", "Removed.")
    } else {
        flash_to(&back, "error", "That share has already gone out.")
    }
}

// ---------------------------------------------------------------------------
// Settings & share buttons
// ---------------------------------------------------------------------------

pub(crate) async fn sm_settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = share_settings(&state);
    let switch = |name: &str, on: bool| {
        format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
    };
    let boxes: String = ShareButton::ALL
        .iter()
        .map(|b| format!(
            r#"<label class="ss-tag sm-btn-pick"><input type="checkbox" name="button" value="{k}"{c}><span>{l}</span></label>"#,
            k = b.key(),
            c = if s.list.contains(b) { " checked" } else { "" },
            l = esc(b.label()),
        ))
        .collect();
    let positions = [("bottom", "End of every post"), ("top", "Top of every post"), ("both", "Top and end")];
    let pos_opts: String = positions
        .iter()
        .map(|(k, l)| format!(r#"<option value="{k}"{sel}>{l}</option>"#, sel = if s.position == *k { " selected" } else { "" }))
        .collect();
    let preview = vuuut_api::social_media::share_bar(&s, "https://example.com/a-post/", "A post");
    page_for_role(
        actor.role,
        "Social media settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{switch_html}{tabs}{banner}
<form method="post" action="{BASE}settings/" id="sm-settings">
<div class="section">What shares say</div>
<div class="card padded rise" style="--i:2">
<label class="field"><textarea name="{t_key}" rows="4" placeholder=" ">{template}</textarea><span>Default template</span></label>
<p class="sm-hint">Used by every account without its own. {{title}}, {{excerpt}}, {{url}}, {{hashtags}} and {{site}}. Each network's text is shortened to fit — title and excerpt first, never the link.</p>
</div>
<div class="card rows rise" style="--i:3;margin-top:12px">
<div class="row" style="align-items:center"><span class="ic tint-green">{chart}</span><div class="body"><span class="title">Campaign tags on links</span>
<span class="detail">Adds utm_source=bluesky (and so on) so Analytics shows which network sent each reader.</span></div><div class="trailing">{utm}</div></div>
<div class="row" style="align-items:center"><span class="ic tint-blue">{tags_ic}</span><div class="body"><span class="title">Tags as hashtags</span>
<span class="detail">A post tagged “Product design” gets #ProductDesign in {{hashtags}}. Up to five.</span></div><div class="trailing">{hash}</div></div>
</div>
<div class="section">Share buttons under posts</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:center"><span class="ic tint-pink">{share_ic}</span><div class="body"><span class="title">Show share buttons</span>
<span class="detail">Plain links to each network's own share page. Nothing loads from any network and no cookie is set until a reader chooses one.</span></div><div class="trailing">{buttons_on}</div></div>
<div class="row"><div class="body"><span class="title">Buttons</span><div class="ss-tags">{boxes}</div></div></div>
<div class="row" style="align-items:center"><div class="body"><span class="title">Where</span></div>
<div class="trailing sm-control"><label class="field fixed" style="margin:0"><select name="{pos_key}">{pos_opts}</select><span>Position</span>{caret}</label></div></div>
<div class="row" style="align-items:center"><div class="body"><span class="title">Label</span></div>
<div class="trailing sm-control"><label class="field" style="margin:0"><input name="{label_key}" value="{label}" maxlength="60" placeholder="{label_default}"><span>Shown above the buttons</span></label></div></div>
</div>
<div class="section">Preview</div>
<div class="card padded sm-preview rise" style="--i:5">{preview}</div>
<div class="sm-save"><button class="btn" type="submit">{check} Save</button></div>
</form>"#,
            check = svg("check"),
            switch_html = section_switch("media"),
            tabs = tabs("settings", true),
            banner = flash_banner(&q),
            t_key = rules::SETTING_TEMPLATE,
            template = esc(&s.template),
            chart = svg("chart"),
            tags_ic = svg("tags"),
            share_ic = svg("share"),
            utm = switch(rules::SETTING_UTM, s.utm),
            hash = switch(rules::SETTING_HASHTAGS, s.hashtags),
            buttons_on = switch(rules::SETTING_BUTTONS, s.buttons),
            pos_key = rules::SETTING_BUTTON_POSITION,
            caret = SELECT_CARET,
            label_key = rules::SETTING_BUTTON_LABEL,
            label = esc(&state.settings.get_str(rules::SETTING_BUTTON_LABEL).unwrap_or_default()),
            label_default = esc(rules::DEFAULT_LABEL),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn sm_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let mut form: HashMap<String, String> = HashMap::new();
    let mut buttons: Vec<String> = Vec::new();
    for (k, v) in form_urlencoded::parse(body.as_bytes()) {
        if k == "button" {
            buttons.push(v.into_owned());
        } else {
            form.insert(k.into_owned(), v.into_owned());
        }
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let s = ShareSettings::read(|key| match key {
        rules::SETTING_BUTTONS | rules::SETTING_UTM | rules::SETTING_HASHTAGS => Some(flag(key).to_string()),
        rules::SETTING_BUTTON_LIST => Some(buttons.join(",")),
        _ => form.get(key).cloned(),
    });
    let label: String = form.get(rules::SETTING_BUTTON_LABEL).map(|l| l.trim().chars().filter(|c| !c.is_control()).take(60).collect()).unwrap_or_default();
    let template = if form.get(rules::SETTING_TEMPLATE).is_some_and(|t| t.trim().is_empty()) { String::new() } else { s.template.clone() };
    let writes = [
        (rules::SETTING_TEMPLATE, serde_json::json!(template)),
        (rules::SETTING_UTM, serde_json::json!(s.utm)),
        (rules::SETTING_HASHTAGS, serde_json::json!(s.hashtags)),
        (rules::SETTING_BUTTONS, serde_json::json!(s.buttons)),
        (rules::SETTING_BUTTON_LIST, serde_json::json!(s.list.iter().map(|b| b.key()).collect::<Vec<_>>().join(","))),
        (rules::SETTING_BUTTON_POSITION, serde_json::json!(s.position)),
        (rules::SETTING_BUTTON_LABEL, serde_json::json!(label)),
    ];
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("social media: saving {key} failed: {}", e.message());
            return flash_to(&format!("{BASE}settings/"), "error", "Saving failed — check the logs.");
        }
    }
    crate::tools::tool_saved(&state, &format!("{BASE}settings/"), "Social media settings saved.")
}

// ---------------------------------------------------------------------------
// sharing on publish
// ---------------------------------------------------------------------------

/// Share a post that has just gone live to every account set to share
/// automatically. Called beside the Slack notification, from the editor and
/// from the scheduler. Posts only (not pages), each account once per post
/// however often it is unpublished and published again. Never fails the
/// publish: the sends run in the background and record their own outcome.
pub(crate) async fn auto_share(state: &AppState, post: &vuuut_core::models::Post) {
    if post.post_type != "post" || post.status != "published" {
        return;
    }
    let accounts = repo::accounts(&state.db).await.unwrap_or_default();
    for a in accounts.iter().filter(|a| a.shares_automatically()) {
        if repo::already_auto_shared(&state.db, &a.id, &post.id).await {
            continue;
        }
        let text = text_for(state, a, post, "auto").await;
        if let Ok(id) = repo::add_share(&state.db, &a.id, &post.id, &text, "auto", None, post.published_by.as_deref().unwrap_or("")).await {
            vuuut_api::social_media::spawn_send(state, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_names_both_halves_and_marks_the_current_one() {
        let html = section_switch("media");
        assert!(html.contains("Social web") && html.contains("Social media"));
        assert_eq!(html.matches(r#"aria-current="page""#).count(), 1);
        assert!(html.contains(r#"class="sm-section on" href="/ghost/social/media/""#));
    }
}
