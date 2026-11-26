//! Real Estate & Property Listings: the email an enquiry sends.
//!
//! When a visitor asks about a property or requests a viewing, the
//! property's agent and the site's notification address (Settings →
//! Properties) each get one message, with `Reply-To`-free plain facts: the
//! visitor's own address is in the body, so replying is the agent's choice
//! and nothing the visitor typed becomes a mail header. Every value is
//! escaped into the HTML part. Sending happens on a spawned task, so a slow
//! mail server never holds up the visitor's page.
//!
//! The public form itself (`vuuut_front::properties`) owns the defences —
//! same-origin, honeypot, fill time, the per-address budget and the daily
//! cap per email — and only calls this once the enquiry is stored.

use crate::AppState;
use vuuut_core::properties::{EnquiryInput, EnquiryKind, Settings};
use vuuut_db::repos::properties as repo;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn send(state: &AppState, to: &str, subject: String, html: String, text: String) {
    if to.is_empty() {
        return;
    }
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let cfg = vuuut_mail::MailConfig::from_settings(&state.settings, &site_title, &state.site_url());
    let to = to.to_string();
    tokio::spawn(async move {
        let msg = vuuut_mail::Message { to: to.clone(), subject, html, text };
        if let Err(e) = vuuut_mail::send(&cfg, &msg).await {
            tracing::warn!("properties: enquiry mail to {to} failed: {}", e.message());
        }
    });
}

/// The rows of facts in an enquiry email.
pub fn facts(s: &Settings, property: Option<&repo::Property>, e: &EnquiryInput) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(p) = property {
        rows.push(("Property", if p.reference.is_empty() { p.title.clone() } else { format!("{} (ref. {})", p.title, p.reference) }));
        rows.push(("Price", p.price_label(&s.currency)));
    }
    rows.push(("From", e.name.clone()));
    rows.push(("Email", e.email.clone()));
    if !e.phone.is_empty() {
        rows.push(("Phone", e.phone.clone()));
    }
    if EnquiryKind::from_key(&e.kind) == EnquiryKind::Viewing {
        let when = match (e.preferred_date.as_str(), e.preferred_time.as_str()) {
            ("", "any" | "") => "Any time".to_string(),
            ("", t) => format!("Any day, {t}"),
            (d, "any" | "") => d.to_string(),
            (d, t) => format!("{d}, {t}"),
        };
        rows.push(("Viewing", when));
    }
    rows
}

/// Tell the agent and the site's notification address.
pub async fn notify_enquiry(state: &AppState, s: &Settings, property: Option<&repo::Property>, agent: Option<&repo::Agent>, e: &EnquiryInput) {
    let kind = EnquiryKind::from_key(&e.kind);
    let rows = facts(s, property, e);
    let what = property.map(|p| p.title.clone()).unwrap_or_else(|| "a property".into());
    let subject = match kind {
        EnquiryKind::Viewing => format!("Viewing request: {what}"),
        EnquiryKind::Info => format!("Enquiry: {what}"),
    };
    let admin = format!("{}/ghost/properties/enquiries/", state.site_url().trim_end_matches('/'));
    let table: String = rows
        .iter()
        .map(|(k, v)| format!(r#"<tr><td style="padding:9px 0;color:#77736b;width:96px;vertical-align:top;border-top:1px solid #efebe4">{k}</td><td style="padding:9px 0;border-top:1px solid #efebe4">{}</td></tr>"#, esc(v)))
        .collect();
    let message = if e.message.is_empty() {
        String::new()
    } else {
        format!(r#"<p style="margin:0 0 20px;white-space:pre-wrap;padding:14px 16px;background:#f7f5f1;border-radius:12px">{}</p>"#, esc(&e.message))
    };
    let site = state.settings.get_str("title").unwrap_or_default();
    let html = format!(
        r#"<!doctype html><html><body style="margin:0;background:#f4f2ee;font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1b1f1d">
<div style="max-width:560px;margin:28px auto;background:#fff;border-radius:16px;padding:30px 30px 26px;border:1px solid #e7e2d9">
<p style="margin:0 0 6px;font-weight:700;font-size:17px">{site}</p>
<p style="margin:0 0 18px;color:#5b5850">{lead}</p>
<table style="width:100%;border-collapse:collapse;margin:0 0 18px;font-size:15px">{table}</table>{message}
<p style="margin:0"><a href="{admin}" style="display:inline-block;background:#1f3d5c;color:#fff;text-decoration:none;padding:12px 22px;border-radius:999px;font-weight:600">Open the enquiries</a></p></div></body></html>"#,
        site = esc(&site),
        lead = match kind {
            EnquiryKind::Viewing => "Somebody would like to view this property.",
            EnquiryKind::Info => "Somebody has a question about this property.",
        },
        admin = esc(&admin),
    );
    let text = format!(
        "{}\n\n{}\n\n{}\n\nOpen the enquiries: {admin}\n",
        subject,
        rows.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"),
        e.message
    );
    let mut sent: Vec<String> = Vec::new();
    for to in [agent.filter(|a| a.is_active()).map(|a| a.email.clone()).unwrap_or_default(), s.notify_email.clone()] {
        let key = to.to_lowercase();
        if !to.is_empty() && !sent.contains(&key) {
            send(state, &to, subject.clone(), html.clone(), text.clone());
            sent.push(key);
        }
    }
}
