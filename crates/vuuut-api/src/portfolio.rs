//! Portfolio: what the admin and the public pages share.
//!
//! * [`settings`] — the tool's settings, read the one way.
//! * [`notify_enquiry`] — a new enquiry from the contact form is emailed to
//!   the address in Settings (or the portfolio's contact address), with the
//!   sender as Reply-To in the text so a reply goes straight to them.

use crate::AppState;
use vuuut_core::portfolio::Settings;
use vuuut_db::repos::portfolio as repo;

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub async fn notify_enquiry(state: &AppState, e: &repo::Enquiry) {
    let s = settings(state);
    let to = if !s.notify_email.is_empty() { s.notify_email.clone() } else { s.email.clone() };
    if to.is_empty() {
        return;
    }
    let site = state.settings.get_str("title").unwrap_or_default();
    let admin = format!("{}/{}/portfolio/enquiries/{}/", state.site_url().trim_end_matches('/'), state.admin_slug(), e.id);
    let rows: Vec<(&str, &str)> = [("Company", e.company.as_str()), ("Phone", e.phone.as_str()), ("Budget", e.budget.as_str()), ("Timeline", e.timeline.as_str()), ("Services", e.services.as_str()), ("About", e.project.as_str())]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let table: String = rows.iter().map(|(k, v)| format!(r#"<tr><td style="padding:8px 0;color:#77736b;width:96px;border-top:1px solid #efebe4">{k}</td><td style="padding:8px 0;border-top:1px solid #efebe4">{}</td></tr>"#, esc(v))).collect();
    let body = format!(
        r#"<h1 style="font-size:22px;margin:0 0 6px">New enquiry from {name}</h1><p style="color:#55524b;margin:0 0 18px"><a href="mailto:{email}">{email}</a></p>
<table style="width:100%;border-collapse:collapse;margin:0 0 18px;font-size:14px">{table}</table>
<p style="background:#f6f3ee;border-radius:10px;padding:12px 14px;margin:0 0 18px;white-space:pre-wrap">{msg}</p>
<p style="margin:0"><a href="{admin}" style="display:inline-block;background:#1b1f1d;color:#fff;text-decoration:none;padding:12px 22px;border-radius:999px;font-weight:600">Open in the admin</a></p>"#,
        name = esc(&e.name),
        email = esc(&e.email),
        msg = esc(&e.message),
        admin = esc(&admin),
    );
    let html = format!(
        r#"<!doctype html><html><body style="margin:0;background:#f4f2ee;font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1b1f1d"><div style="max-width:560px;margin:28px auto;background:#fff;border-radius:16px;padding:30px;border:1px solid #e7e2d9"><p style="margin:0 0 20px;font-weight:700">{}</p>{body}</div></body></html>"#,
        esc(&site)
    );
    let text = format!(
        "New enquiry from {} <{}>\n\n{}\n\n{}\n\n{admin}",
        e.name,
        e.email,
        rows.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"),
        e.message
    );
    let cfg = vuuut_mail::MailConfig::from_settings(&state.settings, &site, &state.site_url());
    let msg = vuuut_mail::Message { to, subject: format!("New enquiry: {}", e.name), html, text };
    tokio::spawn(async move {
        if let Err(err) = vuuut_mail::send(&cfg, &msg).await {
            tracing::warn!("portfolio: enquiry notice failed: {}", err.message());
        }
    });
}
