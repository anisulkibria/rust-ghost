//! Settings -> Email: transactional sending, the SMTP and provider credentials, and a test send.

use super::*;

// --- email: the transactional transport that carries member magic links ---

/// Settings whose value is a credential. They are never rendered back into
/// the page (the field shows a "saved" placeholder instead), are masked by
/// the Admin API, and are absent from the theme/Content API allowlist — so a
/// password typed here cannot leak back out through any read path.
pub const SECRET_SETTINGS: &[&str] = &["mail_smtp_pass", "ses_smtp_user", "ses_smtp_pass", "mailgun_api_key", "mailgun_webhook_signing_key"];

/// A posted credential field. An empty box means "leave what is stored"
/// rather than "clear it", because the form never shows the current value —
/// without this, saving any other field on the page would wipe the password.
pub(crate) async fn save_secret(state: &AppState, key: &str, posted: Option<&String>) -> bool {
    match posted.map(|s| s.trim()) {
        None | Some("") => true,
        // an explicit sentinel is how you deliberately clear one
        Some("-") => state.settings.set(&state.db, key, &serde_json::json!("")).await.is_ok(),
        Some(v) => state.settings.set(&state.db, key, &serde_json::json!(v)).await.is_ok(),
    }
}

/// Whether a credential is stored, phrased for the helper text under the
/// field. Short enough not to clip the floating label it sits beside.
pub(crate) fn secret_state(state: &AppState, key: &str) -> &'static str {
    if state.settings.get_str(key).unwrap_or_default().trim().is_empty() {
        "Nothing stored yet."
    } else {
        "A value is stored. Leave the box empty to keep it, type to replace it, or enter a single “-” to clear it."
    }
}

pub(crate) fn mail_management_notice(state: &AppState) -> &'static str {
    if state.settings.mail_is_managed() {
        r#"<div class="banner info">Email delivery is managed by your hosting provider. Amazon SES and Mailgun settings are locked; no credentials are required from you.</div>"#
    } else { "" }
}

pub(crate) async fn email_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "email").unwrap();
    // Blank means Direct, the same fallback the mailer itself applies.
    let transport = state
        .settings
        .get_str("mail_transport")
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "Direct".into());
    let transport = match transport.to_ascii_lowercase().as_str() {
        "ses" => "SES", "smtp" => "SMTP", _ => "Direct",
    };
    let security = state
        .settings
        .get_str("mail_smtp_security")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "starttls".into());
    let site_url = state.site_url();
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.get_str("title").unwrap_or_else(|| "Ghost".into()),
        &site_url,
    );
    let status = if let Some(error) = cfg.configuration_error() {
        format!(r#"<div class="banner error">{}</div>"#, esc(&error))
    } else if cfg.uses_relay() {
        format!(
            r#"<div class="banner ok rise" style="--i:1">{check}<span class="msg">Sending as <code>{from}</code> over {transport_name}</span></div>"#,
            check = svg("check"),
            from = esc(&cfg.from_address),
            transport_name = cfg.transport_name(),
        )
    } else {
        format!(
            r#"<div class="banner info rise" style="--i:1">{bulb}<span class="msg">Using the <strong>Direct</strong> transport, sending as <code>{from}</code>. Direct delivers straight to each recipient's real mail server — it needs outbound port 25, which many networks block, and mail from an unauthenticated address is usually rejected or marked as spam. <strong>A local mail catcher will not see Direct mail</strong>: to test with MailHog or Mailpit, switch to SMTP below. Configure SMTP for anything real, too.</span></div>"#,
            bulb = svg("mail"),
            from = esc(&cfg.from_address),
        )
    };

    page_for_role(actor_role, 
        "Email",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}{status}{managed_notice}
<form method="post" action="/ghost/settings/email/" id="email-form">
<fieldset {managed_disabled} style="border:0;padding:0;margin:0;min-width:0">
<div class="section">Transactional email</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:center"><span class="ic tint-blue">{mail}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="mail_transport">{transport_opts}</select><span>Transport</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Carries member signup and signin magic links, and password resets. Ghost sets this as <code>mail.transport</code> in <code>config.production.json</code>; here it lives in Settings. <strong>Direct</strong> is Ghost’s default and needs no credentials.</p></div></div>
</div>
<div class="section">SMTP server</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{code}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="mail_smtp_host" value="{host}" placeholder=" " autocomplete="off"><span>Host · e.g. smtp.mailgun.org</span></label>
<label class="field" style="margin:0"><input name="mail_smtp_port" value="{port}" placeholder=" " inputmode="numeric" autocomplete="off"><span>Port · 587 STARTTLS, 465 TLS</span></label>
<label class="field fixed" style="margin:0"><select name="mail_smtp_security">{security_opts}</select><span>Encryption</span>{car}</label>
<label class="field" style="margin:0"><input name="mail_smtp_user" value="{user}" placeholder=" " autocomplete="off"><span>Username</span></label>
<label class="field" style="margin:0"><input name="mail_smtp_pass" type="password" value="" placeholder=" " autocomplete="new-password"><span>Password</span></label>
</div>
<p class="muted" style="margin:10px 0 0">The password is never shown again after it is saved. {pass_hint}</p>
<p class="muted" style="margin:6px 0 0">Testing locally? A mail catcher takes everything and delivers nothing: MailHog and Mailpit both listen on <code>127.0.0.1</code> port <code>1025</code> with encryption <strong>None</strong> and no username or password.</p>
</div></div>
</div>
<div class="section">Amazon SES</div>
<div class="card padded">
<div class="grid2" style="gap:12px">
<label class="field"><input name="ses_max_send_rate" type="number" min="1" max="10000" value="{ses_rate}" placeholder=" "><span>Newsletter messages per second</span></label>
<label class="field"><input name="ses_region" value="{ses_region}" placeholder=" "><span>AWS region · e.g. us-east-1</span></label>
<label class="field"><input name="ses_smtp_endpoint" value="{ses_endpoint}" placeholder=" "><span>SMTP endpoint · optional, from AWS CSV</span></label>
<label class="field"><input name="ses_smtp_user" type="password" value="" autocomplete="new-password" placeholder=" "><span>SES SMTP username</span></label>
<label class="field"><input name="ses_smtp_pass" type="password" value="" autocomplete="new-password" placeholder=" "><span>SES SMTP password</span></label>
</div>
<p class="muted">Use the SMTP endpoint and credentials from your AWS CSV, or leave the endpoint blank for regional SES. Configure a verified sender. AWS API secret keys cannot be used as SMTP passwords. SES uses STARTTLS on port 587. Leave credentials blank to keep them; enter “-” to clear.</p>
</div>
<div class="section">Sender identity</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{person}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="mail_from_address" value="{from_address}" placeholder=" "><span>From address</span></label>
<label class="field" style="margin:0"><input name="mail_from_name" value="{from_name}" placeholder=" "><span>From name</span></label>
<label class="field" style="margin:0"><input name="mail_reply_to" value="{reply_to}" placeholder=" "><span>Reply-to · optional</span></label>
</div>
<p class="muted" style="margin:10px 0 0">Defaults to <code>{default_from}</code>. Most providers reject a From address on a domain you have not verified.</p>
</div></div>
</div>
</fieldset></form>
<div class="section">Send a test</div>
<div class="card rows rise" style="--i:5">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{send}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Check the settings work</span>
<form method="post" action="/ghost/settings/email/test/" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<label class="field" style="margin:0;flex:1;min-width:220px"><input name="to" type="email" value="{owner_email}" placeholder=" " required><span>Send to</span></label>
<button class="btn tonal small" type="submit">Send test email</button>
</form>
<p class="muted" style="margin:10px 0 0">Save your changes first. Unlike magic links, this reports the transport error directly so you can debug credentials here.</p>
</div></div>
</div>"#,
            head = group_head(group, Some("email-form")),
            banner = flash_banner(&q),
            status = status,
            mail = svg("mail"),
            code = svg("code"),
            person = svg("ghost"),
            send = svg("send"),
            car = SELECT_CARET,
            transport_opts = select_options(&[("Direct", "Direct · no relay, straight to the recipient"), ("SMTP", "SMTP · send through a relay"), ("SES", "Amazon SES · regional SMTP")], &transport),
            security_opts = select_options(
                &[("starttls", "STARTTLS · port 587"), ("tls", "TLS/SSL · port 465"), ("none", "None · local relay only")],
                &security
            ),
            managed_notice = mail_management_notice(&state),
            managed_disabled = if state.settings.mail_is_managed() { "disabled" } else { "" },
            ses_region = s("ses_region"),
            ses_endpoint = s("ses_smtp_endpoint"),
            ses_rate = state.settings.get_str("ses_max_send_rate").and_then(|s| s.parse::<u32>().ok()).unwrap_or(1).clamp(1, 10_000),
            host = s("mail_smtp_host"),
            port = s("mail_smtp_port"),
            user = s("mail_smtp_user"),
            pass_hint = secret_state(&state, "mail_smtp_pass"),
            from_address = s("mail_from_address"),
            from_name = s("mail_from_name"),
            reply_to = s("mail_reply_to"),
            default_from = esc(&cfg.from_address),
            owner_email = esc(user.email.as_deref().unwrap_or("")),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn email_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    if state.settings.mail_is_managed() {
        return settings_flash("email", "error", "Email delivery is managed by your hosting provider.");
    }
    let rate = form.get("ses_max_send_rate").map(|value| value.parse::<u32>());
    if rate.as_ref().is_some_and(|value| !matches!(value, Ok(1..=10_000))) {
        return settings_flash("email", "error", "SES send rate must be between 1 and 10000 messages per second.");
    }
    let mut ok = true;
    if let Some(Ok(rate)) = rate {
        ok &= state.settings.set(&state.db, "ses_max_send_rate", &serde_json::json!(rate.to_string())).await.is_ok();
    }
    for key in [
        "ses_region", "ses_smtp_endpoint", "mail_transport", "mail_smtp_host", "mail_smtp_port", "mail_smtp_security",
        "mail_smtp_user", "mail_from_address", "mail_from_name", "mail_reply_to",
    ] {
        let value = form.get(key).cloned().unwrap_or_default().trim().to_string();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    ok &= save_secret(&state, "mail_smtp_pass", form.get("mail_smtp_pass")).await;
    ok &= save_secret(&state, "ses_smtp_user", form.get("ses_smtp_user")).await;
    ok &= save_secret(&state, "ses_smtp_pass", form.get("ses_smtp_pass")).await;
    if ok {
        settings_flash("email", "ok", "Email settings saved.")
    } else {
        settings_flash("email", "error", "Saving failed — check the logs.")
    }
}

/// Send a test message with the saved settings, reporting the transport's
/// own error. This is the one place a mail failure is surfaced to a human —
/// magic-link sends stay silent on purpose.
pub(crate) async fn email_test(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let to = form.get("to").cloned().unwrap_or_default().trim().to_string();
    if !to.contains('@') {
        return settings_flash("email", "error", "Enter a valid address to send the test to.");
    }
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let (subject, html, text) = ghost_mail::templates::test_email(&title, &site_url);
    let msg = ghost_mail::Message { to: to.clone(), subject, html, text };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => settings_flash("email", "ok", &format!("Test email sent to {to}.")),
        Err(e) => settings_flash("email", "error", &format!("Send failed: {}", e.message())),
    }
}
