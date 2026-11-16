//! Settings → Security: every switch the module owns, on one screen.
//!
//! Four groups, in the order somebody thinks about them: how people sign in,
//! how long a session lasts, what this server tells a browser, and what this
//! server believes and fetches. The rules and their defaults are
//! [`ghost_core::security`] and [`ghost_core::firewall`]; this is the screen.
//!
//! Every select is reduced to a value the page offered before it is stored —
//! see [`settings_save`]. That is not belt and braces: `security_min_password_
//! length` reaches every password path in the application, and a forged post
//! setting it to 1 would weaken all of them at once.

use super::*;

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub async fn settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let group = crate::SETTINGS_GROUPS
        .iter()
        .find(|g| g.slug == "security")
        .expect("security settings group");
    let settings = ghost_api::security_settings::snapshot(&state);
    let spam = ghost_api::security_settings::spam_snapshot(&state);
    let https = state.site_url().starts_with("https://");

    let switch = |name: &str, icon: &str, tint: &str, title: &str, detail: &str, on: bool| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="{name}" value="1"{ck} form="security-settings"><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            icon = svg(icon),
            title = esc(title),
            detail = esc(detail),
            name = esc(name),
            ck = if on { " checked" } else { "" },
        )
    };
    let flag = |key: &str| rules::flag(&settings, key);
    let pick = |name: &str, icon: &str, tint: &str, title: &str, detail: &str, options: String| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><div class="selbox"><select name="{name}" aria-label="{title}" form="security-settings">{options}</select>{car}</div></div></div>"#,
            icon = svg(icon),
            title = esc(title),
            detail = esc(detail),
            name = esc(name),
            options = options,
            car = SELECT_CARET,
        )
    };

    let session_days = rules::session_max_age_secs(&settings) / 86_400;
    let idle_minutes = rules::session_idle_secs(&settings) / 60;
    let min_length = rules::min_password_length(&settings);

    // What the headers switch is actually sending right now, listed rather
    // than described: "security headers" means nothing to somebody deciding
    // whether to turn it off, and the list is two lines.
    let sending = rules::response_headers(&settings, Surface::Admin, https);
    let sending_note = if sending.is_empty() {
        "Admin responses carry no security headers.".to_string()
    } else {
        format!(
            "Admin responses are carrying: {}.",
            sending.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "),
        )
    };

    page_for_role(
        actor.role,
        "Security",
        "settings",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<p class="rise" style="--i:1"><a class="btn tonal" href="/ghost/security/">{lock} Open the Security report</a></p>
<form method="post" action="/ghost/settings/security/" id="security-settings">
<!-- `.settings` is what undoes the shared one-line truncation on a row's
     detail (`.settings .rows .row .detail` in admin.css). Every switch on
     this page is a defence whose *explanation* is the reason somebody can
     decide about it — "Believe forwarded client addresses" means nothing
     without the sentence after it — so the sentences have to be readable
     rather than ellipsed at the card's edge. Newsletters uses the same
     wrapper for the same reason. -->
<div class="settings">

<div class="section">Sign-in</div>
<div class="card rows rise" style="--i:2">
{mfa}
{password_len}
</div>

<div class="section">Sessions</div>
<div class="card rows rise" style="--i:3">
{session_days_row}
{idle_row}
{revoke_row}
</div>
<p class="muted" style="margin:8px 0 0">A session is refused as soon as it passes either limit, on the request that would have used it — not when a background job next runs. Shortening these takes effect immediately and may sign people out.</p>

<div class="section">Response headers</div>
<div class="card rows rise" style="--i:4">
{headers_row}
{referrer_row}
{hsts_row}
</div>
<p class="muted" style="margin:8px 0 0">{sending_note}{hsts_note}</p>

<div class="section">Requests this server makes and trusts</div>
<div class="card rows rise" style="--i:5">
{proxy_row}
{fetch_row}
</div>
<p class="muted" style="margin:8px 0 0">Forwarded addresses are the setting to get right. Every rate limit in the application counts attempts per client address; if the address can be chosen by the caller, none of those limits applies. Turn it on only when there is a reverse proxy, load balancer or CDN in front of this server — and then it is required, or the whole internet shares one allowance.</p>

<div class="section">Spam</div>
<div class="card rows rise" style="--i:6">
{disposable_row}
{signup_origin_row}
{user_agent_row}
</div>
<div class="card padded rise" style="--i:7">
<label class="field" style="margin:0 0 8px"><textarea name="spam_blocked_user_agents" rows="5" placeholder=" " maxlength="{ua_max}" spellcheck="false" style="font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px">{user_agents}</textarea><span>Blocked user agents</span></label>
<p class="muted" style="margin:0 0 6px">One fragment per line, matched anywhere in the header and ignoring case — <code>SemrushBot</code> catches <code>Mozilla/5.0 (compatible; SemrushBot/7~bl)</code>. Lines starting <code>#</code> are comments. Fragments, not patterns: a regular expression run against a header on every request is a denial of service you can write into your own settings by accident.</p>
<p class="muted" style="margin:0">A request with <b>no</b> User-Agent is never blocked — feed readers, monitoring and plenty of ordinary clients send none, and a rule that refuses them refuses the wrong people silently.</p>
</div>
<p class="muted" style="margin:8px 0 0">Member signups are also checked against <a href="/ghost/settings/membership/">Settings → Membership → Spam filters</a>, which is the site&rsquo;s own list of blocked domains and applies whether or not the switch above is on.</p>

<div class="section">Firewall</div>
<div class="card padded rise" style="--i:8">
<p class="muted" style="margin:0 0 10px">Barring an address or a range outright — from the admin, or from the whole site — is on its own screen, because a rule is a decision about a person rather than a setting.</p>
<a class="btn tonal" href="/ghost/security/firewall/">{lock2} Open the firewall</a>
</div>
</div>
</form>"#,
            head = crate::group_head(group, Some("security-settings")),
            banner = flash_banner(&q),
            lock = svg("lock"),
            mfa = switch(
                "require_email_mfa",
                "mail",
                "green",
                "Require a code emailed at sign-in",
                "A correct password buys an unverified session until a six-digit code comes back. A first-ever sign-in skips it — that person only has a password because they followed an emailed link.",
                state.settings.get_bool("require_email_mfa"),
            ),
            password_len = pick(
                "security_min_password_length",
                "lock",
                "blue",
                "Shortest staff password",
                "Applies to setup, a redeemed invitation, the staff screen, a reset link and the Admin API alike. Existing passwords are not affected until they are changed.",
                select_options(rules::PASSWORD_LENGTHS, &min_length.to_string()),
            ),
            session_days_row = pick(
                "security_session_days",
                "calendar",
                "teal",
                "Sign staff out after",
                "However much the session is used. The session cookie's own lifetime is seven days, so anything longer than that only affects an id taken from the database.",
                select_options(rules::SESSION_DAYS, &session_days.to_string()),
            ),
            idle_row = pick(
                "security_session_idle_minutes",
                "history",
                "amber",
                "Sign staff out when idle for",
                "Counted from the last time the session was used. Off by default: it signs somebody out of a tab they left open, which is the point and is also somebody's decision.",
                select_options(rules::SESSION_IDLE_MINUTES, &idle_minutes.to_string()),
            ),
            revoke_row = switch(
                "security_revoke_on_password_change",
                "logout",
                "purple",
                "Sign out other sessions when a password changes",
                "A cookie copied before the change stops working. The browser making the change keeps its own session.",
                flag("security_revoke_on_password_change"),
            ),
            headers_row = switch(
                "security_headers",
                "lock",
                "green",
                "Send security headers",
                "Stops the admin being framed by another site, stops a response's declared type being second-guessed, and keeps admin pages out of shared caches.",
                flag("security_headers"),
            ),
            referrer_row = pick(
                "security_admin_referrer_policy",
                "share",
                "slate",
                "Referrer policy for admin pages",
                "What another site learns when a link is followed out of the admin. An admin URL can carry a post or member id in its path, so only values that never send one off-origin are offered.",
                select_options(rules::ADMIN_REFERRER_POLICIES, rules::admin_referrer_policy(&settings)),
            ),
            hsts_row = switch(
                "security_hsts",
                "globe",
                if https { "blue" } else { "slate" },
                "Strict transport security (HSTS)",
                "Tells a browser never to try this site over http again, for two years. It is a promise that cannot be withdrawn quickly: a site that later loses its certificate becomes unreachable rather than merely insecure.",
                flag("security_hsts"),
            ),
            sending_note = esc(&sending_note),
            hsts_note = if https {
                ""
            } else {
                " This site's URL is http, so HSTS is never sent regardless of the switch — a browser that honoured it would lock itself out of the site."
            },
            proxy_row = switch(
                "security_trust_proxy",
                "share",
                "red",
                "Believe forwarded client addresses",
                "Read the client's address from X-Forwarded-For and X-Real-IP instead of from the connection.",
                flag("security_trust_proxy"),
            ),
            ua_max = ghost_core::firewall::USER_AGENTS_MAX,
            user_agents = esc(
                &state.settings.get_str("spam_blocked_user_agents").unwrap_or_default()
            ),
            lock2 = svg("lock"),
            disposable_row = switch(
                "spam_block_disposable_email",
                "mail",
                "amber",
                "Refuse signups from disposable addresses",
                "A short built-in list of services whose whole purpose is a ten-minute address. Deliberately short: the failure mode of an email blocklist is silent and lands on a reader who did nothing.",
                ghost_core::firewall::flag_spam(&spam, "spam_block_disposable_email"),
            ),
            signup_origin_row = switch(
                "spam_signup_same_origin",
                "share",
                "blue",
                "Signups must come from this site",
                "Refuse a member signup that does not carry this site's origin, as the forms endpoint already does. Leave off if you embed the signup form on another domain.",
                ghost_core::firewall::flag_spam(&spam, "spam_signup_same_origin"),
            ),
            user_agent_row = switch(
                "spam_block_user_agents",
                "hide",
                "slate",
                "Block these user agents",
                "Refuse any request whose User-Agent matches a line below. Applies to the published site and the admin alike.",
                ghost_core::firewall::flag_spam(&spam, "spam_block_user_agents"),
            ),
            fetch_row = switch(
                "security_private_fetch",
                "globe",
                "amber",
                "Refuse to fetch private addresses",
                "Bookmark and embed cards have this server read a URL an author typed. With this on, a URL resolving to loopback, a private range or a cloud metadata service is refused — including after a redirect.",
                flag("security_private_fetch"),
            ),
        ),
        true,
    )
    .into_response()
}

pub async fn settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let mut ok = true;

    // A checkbox that is off is absent from the post, so every switch is
    // written on every save rather than only the ones that arrived.
    for (key, _) in rules::BOOL_SETTINGS {
        let value = posted.get(*key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(value)).await.is_ok();
    }
    // `require_email_mfa` predates this module and lives in its own group; it
    // is on this screen because this is where somebody deciding about it is
    // looking, and it is written the same way as the rest.
    let mfa = posted.get("require_email_mfa").map(|v| v == "1").unwrap_or(false);
    ok &= state.settings.set(&state.db, "require_email_mfa", &serde_json::Value::Bool(mfa)).await.is_ok();

    // Only values we offered. A session lifetime or a password floor that
    // came from somewhere else has never been thought about, and the second
    // one reaches every password path in the application — so each is run
    // through the same reduction the rules use to read it back, and what is
    // stored is the reduction's answer rather than what was posted.
    for (key, offered, default) in [
        ("security_session_days", rules::SESSION_DAYS, rules::DEFAULT_SESSION_DAYS),
        ("security_session_idle_minutes", rules::SESSION_IDLE_MINUTES, 0),
        ("security_min_password_length", rules::PASSWORD_LENGTHS, rules::DEFAULT_PASSWORD_LENGTH),
    ] {
        let posted_value = posted.get(key).map(String::as_str).unwrap_or("");
        let reduced = rules::choice(&serde_json::json!({ key: posted_value }), key, offered, default);
        ok &= state
            .settings
            .set(&state.db, key, &serde_json::json!(reduced.to_string()))
            .await
            .is_ok();
    }

    // The spam and firewall switches. Same shape as the `security_*` ones and
    // written on every save for the same reason: an unchecked checkbox is
    // absent from the post, not present-and-false.
    //
    // `firewall_enabled` is in this list, so turning the firewall off is
    // possible from here as well as from its own screen. Turning it *on* from
    // here is not guarded by the lockout simulation the firewall screen runs
    // — so it is deliberately not offered on this page, and a post that names
    // it is honoured only in the "off" direction.
    for (key, _) in ghost_core::firewall::BOOL_SETTINGS {
        if *key == "firewall_enabled" {
            continue;
        }
        let value = posted.get(*key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(value)).await.is_ok();
    }

    // Rebuilt rather than stored as sent: parsed into the fragment list the
    // matcher will actually use, then written back as that list's own text.
    // What is stored is therefore always what is enforced — a line that was
    // dropped for being blank, a comment, or past the limit does not sit in
    // the textarea looking as though it is doing something.
    let agents = ghost_core::firewall::parse_user_agents(
        posted.get("spam_blocked_user_agents").map(String::as_str).unwrap_or(""),
    );
    ok &= state
        .settings
        .set(&state.db, "spam_blocked_user_agents", &serde_json::json!(agents.join("\n")))
        .await
        .is_ok();

    let policy = posted.get("security_admin_referrer_policy").map(String::as_str).unwrap_or("");
    let policy =
        rules::admin_referrer_policy(&serde_json::json!({ "security_admin_referrer_policy": policy }));
    ok &= state
        .settings
        .set(&state.db, "security_admin_referrer_policy", &serde_json::json!(policy))
        .await
        .is_ok();

    if ok {
        crate::settings_flash("security", "ok", "Security settings saved.")
    } else {
        crate::settings_flash("security", "error", "Saving failed — check the logs.")
    }
}
