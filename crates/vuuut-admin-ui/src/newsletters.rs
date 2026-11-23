//! The Newsletters module: what goes out, who gets it, and what stopped it.
//!
//! Two screens under `/ghost/newsletters/`, plus the newsletter list it
//! adopted and the settings it links to:
//!
//! * **Overview** — reach, deliverability and engagement in six figures,
//!   every problem with the sending programme worst-first, what carries the
//!   mail and what is measured, and the last few editions.
//! * **Editions** — every post that was ever emailed, newest first, with what
//!   became of it and a retry for the ones that failed.
//! * **Newsletters** — the newsletters themselves, under Newsletters → Settings
//!   where their design editor has always lived. See "Where the list is",
//!   below.
//!
//! ## This module organises; it owns one verb
//!
//! Like [`crate::memberships`], it has no table, no setting and no migration
//! of its own: `newsletters`, `emails`, `email_recipients` and
//! `newsletter_clicks` are written by the send loop, the provider webhooks and
//! the design editor. What was missing was anywhere that read them together.
//!
//! A sending programme is broken in ways no single existing screen can see,
//! and all of them are one click apart. Newsletter settings determine whether
//! sending is on; Settings → Emails knows whether a relay is configured;
//! Analytics settings determine whether opens are tracked; the `emails` table
//! knows the last three sends failed. Each of those screens can be perfectly
//! correct while a post published today goes nowhere, and nothing said so.
//! [`vuuut_core::newsletters`] is where that judgement lives, with its tests.
//!
//! Unlike Memberships, this module does own one action: **retrying a failed
//! edition**. It is here rather than left to the Admin API's
//! `PUT /emails/{id}/retry` because a retry is the fix for the thing this
//! screen reports, and a report that names a failure and cannot do anything
//! about it sends people to curl. The work itself is still
//! [`crate::retry_email`] — the same function the API endpoint calls, with
//! the same guarantee: only the recipients a send could not reach are tried
//! again, so a retry cannot mail anybody twice.
//!
//! That one write is why [`guard`] takes a `writes` flag and origin-checks
//! it. Sending email costs money and reputation; a cross-site POST that
//! re-sent a newsletter would be somebody else spending both.
//!
//! ## Where the list is
//!
//! The newsletters themselves stay at `/ghost/newsletters/settings/`, which is
//! where their design editor lives and where every link in this admin points.
//! The module's third tab goes there, exactly as the Memberships module's
//! Members tab goes to `/ghost/members/`: to somebody using the admin it is
//! one of this module's screens, and where its handler happens to be routed
//! is not their problem.
//!
//! ## Who may see it
//!
//! Administrators. The screens name the failure reasons a provider returned,
//! which quote member email addresses, and the sending allowance is
//! commercially sensitive. The retry spends money.
//!
//! ## The security contract
//!
//! Everything interpolated goes through `esc`, including post titles, subject
//! lines and — most importantly — the `error` string a mail provider returned,
//! which is remote text this site did not write and the only genuinely
//! hostile input on the page. Ids are checked with
//! `vuuut_core::ids::is_objectid` before they are built into a link, and the
//! retry form's id is used as a bound parameter and looked up before anything
//! happens to it. `?days=` and `?page=` are reduced to known values before
//! they are used or linked.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use vuuut_api::AppState;
use vuuut_core::newsletters as rules;
use vuuut_core::newsletters::{Gap, Programme, Severity};
use vuuut_db::repos::newsletters as repo;
use std::collections::HashMap;

use crate::{countable, esc, flash_banner, page_for_role, require_role, svg, Actor, Role};

/// The role check, plus — for the retry — the origin check.
///
/// The session cookie is `SameSite=Lax` already; the explicit check is so
/// that re-sending a newsletter does not rest on one browser behaviour. This
/// is a write that costs money per recipient and spends the site's sending
/// reputation, which makes it the wrong one to leave resting on anything.
async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn server_error(e: &vuuut_core::GhostError) -> Response {
    tracing::error!("newsletters admin: {}", e.message());
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

/// Redirect back to one of this module's own screens with a flash.
///
/// `path` is always one of this file's literals, never anything from a
/// request: a redirect target built from user input is an open redirect even
/// when it looks like it cannot be.
pub(crate) fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

pub(crate) const EDITIONS_PATH: &str = "/ghost/newsletters/editions/";

/// The page header, shared by both of this module's screens.
fn head(active: &str) -> String {
    format!(
        r#"<header class="page-head rise"><div><h1>Newsletters</h1>
<p class="sub">What goes out, who gets it, and what stopped it.</p></div>
<div class="actions"><a class="btn tonal" href="/ghost/settings/emails/">{mail} Email settings</a>
<a class="btn" href="/ghost/newsletters/settings/">{gear} Newsletter settings</a></div></header>
{tabs}"#,
        mail = svg("mail"),
        gear = svg("settings"),
        tabs = tabs(active),
    )
}

/// The module's screens, as the shared segmented bar.
///
/// The third tab leaves the module's own URL space, the way the Memberships
/// module's Members tab does, and for the same reason: the newsletters
/// themselves are edited where their design editor lives, and a copy of that
/// list here would be a second place to keep in step.
fn tabs(active: &str) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            href = href,
            label = esc(label),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{overview}{editions}{list}</div>"#,
        overview = tab("overview", "Overview", "/ghost/newsletters/"),
        editions = tab("editions", "Editions", EDITIONS_PATH),
        list = tab("list", "Newsletters", "/ghost/newsletters/settings/"),
    )
}

/// A figure, what it counts, and one line about it.
fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<span class="nl-tile-value">{value}</span>
<span class="nl-tile-label">{label}</span>
<span class="nl-tile-detail">{detail}</span>"#,
        tint = tint,
        icon = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(href) => format!(r#"<a class="card padded nl-tile" href="{href}">{body}</a>"#),
        None => format!(r#"<div class="card padded nl-tile">{body}</div>"#),
    }
}

/// A rate to one decimal place, or a dash when there is none.
///
/// The dash is load-bearing on this screen more than anywhere else in the
/// admin: with `email_track_opens` off, the open rate is not zero, it is
/// unmeasured — see [`rules::Programme::open_rate`].
fn percent(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.1}%"),
        None => "—".into(),
    }
}

/// One problem, as a row.
fn gap_row(g: &Gap) -> String {
    format!(
        r#"<div class="row nl-gap" id="gap-{code}"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title} <span class="chip {chip}">{severity}</span></span>
<span class="detail">{detail}</span></div>
<div class="trailing">{fix}</div></div>"#,
        code = esc(g.code),
        tint = g.severity.tint(),
        icon = svg(match g.severity {
            Severity::Blocking => "warning",
            Severity::Opportunity => "trending",
            Severity::Note => "notes",
        }),
        title = esc(g.title),
        chip = match g.severity {
            Severity::Blocking => "error",
            Severity::Opportunity => "draft",
            Severity::Note => "tonal",
        },
        severity = esc(g.severity.label()),
        detail = esc(&g.detail),
        fix = g
            .fix
            .map(|(label, path)| format!(
                r#"<a class="btn text" href="{path}">{label} {chev}</a>"#,
                path = path,
                label = esc(label),
                chev = svg("chevron"),
            ))
            .unwrap_or_default(),
    )
}

/// The window picker, built from [`rules::WINDOWS`] so the links offered and
/// the values the reducer accepts are the same list.
fn window_bar(base: &str, days: i64) -> String {
    let segs: String = rules::WINDOWS
        .iter()
        .map(|(key, value, label)| {
            format!(
                r#"<a class="seg{on}" href="{base}?days={key}">{label}<span style="width:8px"></span></a>"#,
                on = if *value == days { " on" } else { "" },
                base = base,
                key = key,
                label = esc(label),
            )
        })
        .collect();
    format!(r#"<div class="seg-bar">{segs}</div>"#)
}

// ---------------------------------------------------------------------------
// measuring the programme
// ---------------------------------------------------------------------------

/// What carries the newsletters, or why nothing does.
///
/// Asks [`crate::resolve_bulk_transport`] — the same function the send loop
/// asks a moment before it sends — rather than reading the settings and
/// guessing. That is the whole point: a screen that inspected
/// `newsletter_transport` would say "Mailgun" for a site whose Mailgun key is
/// missing, which is the exact failure it exists to catch.
fn transport_of(state: &AppState) -> (Option<String>, Option<String>) {
    let title = state.settings.get_str("title").unwrap_or_else(|| vuuut_core::DEFAULT_SITE_TITLE.into());
    let mail = vuuut_mail::MailConfig::from_settings(&state.settings, &title, &state.site_url());
    match crate::resolve_bulk_transport(state, &mail) {
        Ok(crate::BulkTransport::Mailgun(_)) => (Some("Mailgun".into()), None),
        Ok(crate::BulkTransport::Ses(_)) => (Some("Amazon SES".into()), None),
        Ok(crate::BulkTransport::Smtp) => (Some(mail.transport_name().to_string()), None),
        Err(reason) => (None, Some(reason)),
    }
}

/// Assemble the [`Programme`] the rules judge, from the settings and the
/// counts, in one place — so [`rules::audit`] stays a pure function of facts
/// and the screen cannot judge something it forgot to look up.
pub(crate) async fn measure(state: &AppState, days: i64) -> vuuut_core::Result<(Programme, repo::Counts)> {
    let now = chrono::Utc::now();
    let since = vuuut_core::dates::to_db_string(&(now - chrono::Duration::days(days)));
    let counts = repo::programme(&state.db, &since).await?;
    let last_sent = repo::last_sent(&state.db).await;
    let (transport, transport_error) = transport_of(state);

    // Worked out here from this process's clock rather than asked of the
    // database, for the reason the whole project computes instants in Rust.
    let days_since_last = last_sent.as_deref().and_then(|stamp| {
        vuuut_core::dates::from_db_string(stamp).map(|then| (now - then).num_days().max(0))
    });

    // The host's sending allowance, when it enforces one. `report` is the
    // same figure the send loop checks before it accepts a send.
    let meter = vuuut_db::mail_usage::Meter::new(
        state.db.clone(),
        state.settings.get("hostSettings").unwrap_or(serde_json::Value::Null),
    );
    let (allowance, allowance_used) = match meter.report().await {
        Ok(report) => (
            report["emails"]["limit"].as_i64(),
            report["emails"]["used"].as_i64().unwrap_or(0),
        ),
        Err(_) => (None, 0),
    };

    let programme = Programme {
        newsletters: counts.newsletters,
        sending_newsletters: counts.sending_newsletters,
        subscribers: counts.subscribers,
        subscriptions: counts.subscriptions,
        members: counts.members,
        editions: counts.editions,
        editions_in_window: counts.editions_in_window,
        failed_editions: counts.failed_editions,
        pending_editions: counts.pending_editions,
        last_sent,
        days_since_last,
        attempted: counts.attempted,
        delivered: counts.delivered,
        opened: counts.opened,
        failed: counts.failed,
        clicked: counts.clicked,
        // Ghost encodes "sending is off" as this setting reading `disabled`,
        // rather than as a switch of its own.
        sending_on: state
            .settings
            .get_str("editor_default_email_recipients")
            .map(|v| v != "disabled")
            .unwrap_or(true),
        transport,
        transport_error,
        track_opens: state.settings.get_bool("email_track_opens"),
        track_clicks: state.settings.get_bool("email_track_clicks"),
        signups_closed: state.settings.get_str("members_signup_access").as_deref() == Some("none"),
        allowance,
        allowance_used,
    };
    Ok((programme, counts))
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub async fn newsletters_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let days = rules::window_days(q.get("days").map(|s| s.as_str()));
    let (p, _counts) = match measure(&state, days).await {
        Ok(m) => m,
        Err(e) => return server_error(&e),
    };
    let gaps = rules::audit(&p);
    let blocking = gaps.iter().filter(|g| g.severity == Severity::Blocking).count();

    let verdict = if blocking > 0 {
        format!(
            "{} stopping a post reaching anybody.",
            countable(blocking as i64, "problem"),
        )
    } else if gaps.is_empty() {
        "A post published now would reach its subscribers. Every check this screen makes passes."
            .to_string()
    } else {
        format!(
            "Nothing is stopping a send. {} worth looking at.",
            countable(gaps.len() as i64, "suggestion"),
        )
    };
    let summary = if !p.has_sent() {
        format!(
            "{} subscribed, and nothing emailed yet. The figures below fill in with the first \
             edition.",
            countable(p.subscribers, "member"),
        )
    } else {
        format!(
            "{} subscribed across {}. {} attempted in the last {} days, {} delivered{opened}.",
            countable(p.subscribers, "member"),
            countable(p.newsletters, "newsletter"),
            countable(p.attempted, "message"),
            days,
            p.delivered,
            opened = match p.open_rate() {
                Some(rate) => format!(", {rate:.0}% of them opened"),
                None => ", opens not measured".to_string(),
            },
        )
    };

    let tiles = format!(
        r#"<div class="nl-tiles rise" style="--i:2">{subs}{reach}{sent}{delivery}{opens}{clicks}</div>"#,
        subs = tile(
            "Subscribers",
            &crate::compact(p.subscribers),
            &format!("{} across every newsletter", countable(p.subscriptions, "subscription")),
            Some("/ghost/newsletters/settings/"),
            "send",
            "purple",
        ),
        reach = tile(
            "Reach",
            &percent(p.reach()),
            &format!("Of {}", countable(p.members, "member")),
            Some("/ghost/memberships/"),
            "group",
            "blue",
        ),
        sent = tile(
            &format!("Sent in {days} days"),
            &crate::compact(p.editions_in_window),
            &match p.days_since_last {
                Some(0) => "The last one went out today".to_string(),
                Some(1) => "The last one went out yesterday".to_string(),
                Some(n) => format!("The last one went out {n} days ago"),
                None => "Nothing has gone out yet".to_string(),
            },
            Some(EDITIONS_PATH),
            "mail",
            "teal",
        ),
        delivery = tile(
            "Delivered",
            &percent(p.delivery_rate()),
            &match p.failed {
                0 => format!("{} attempted, none failed", p.attempted),
                n => format!("{} attempted, {n} failed", p.attempted),
            },
            Some(EDITIONS_PATH),
            "check",
            if p.failure_rate().unwrap_or(0.0) > rules::FAILURE_RATE_LIMIT { "red" } else { "green" },
        ),
        opens = tile(
            "Opened",
            &percent(p.open_rate()),
            &if p.track_opens {
                format!("{} of {} delivered", p.opened, p.delivered)
            } else {
                "Open tracking is off".to_string()
            },
            Some("/ghost/analytics/settings/"),
            "eye",
            if p.track_opens { "amber" } else { "slate" },
        ),
        clicks = tile(
            "Clicked",
            &percent(p.click_rate()),
            &if p.track_clicks {
                format!("{} clicked a link", countable(p.clicked, "member"))
            } else {
                "Click tracking is off".to_string()
            },
            Some("/ghost/analytics/settings/"),
            "link",
            if p.track_clicks { "pink" } else { "slate" },
        ),
    );

    let gap_card = if gaps.is_empty() {
        format!(
            r#"<div class="section">What is in the way</div>
<div class="card padded rise" style="--i:3"><p style="margin:0">{check} Nothing. Sending is on, there is a transport behind it, somebody is subscribed, and the last send worked.</p></div>"#,
            check = svg("check"),
        )
    } else {
        format!(
            r#"<div class="section">What is in the way</div>
<div class="card rows rise" style="--i:3">{rows}</div>"#,
            rows = gaps.iter().map(gap_row).collect::<String>(),
        )
    };

    let delivery = delivery_card(&p);
    let recent = recent_editions_card(&state).await;
    let list = newsletter_list_card(&state).await;

    page_for_role(
        actor.role,
        "Newsletters",
        "newsletters",
        Some(&actor.user),
        &format!(
            r#"{head}
{banner}
<div class="card padded rise nl-hero" style="--i:1">
<div class="nl-hero-body"><h2>{verdict}</h2><p class="muted">{summary}</p></div>
{window}
</div>
{tiles}
{gaps}
<div class="nl-two">{delivery}{recent}</div>
{list}"#,
            head = head("overview"),
            banner = flash_banner(&q),
            verdict = esc(&verdict),
            summary = esc(&summary),
            window = window_bar("/ghost/newsletters/", days),
            tiles = tiles,
            gaps = gap_card,
            delivery = delivery,
            recent = recent,
            list = list,
        ),
        true,
    )
    .into_response()
}

/// What carries the mail, and what is being measured.
///
/// Four facts that each live on a different settings screen. Having them in
/// one card is most of the reason this module exists: the question "will a
/// post I publish now reach anybody, and will I be able to tell" cannot be
/// answered from any one of those screens.
fn delivery_card(p: &Programme) -> String {
    let row = |icon: &str, tint: &str, title: &str, detail: &str, chip: &str, state_label: &str, href: &str| {
        format!(
            r#"<a class="row" href="{href}"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip {chip}">{state_label}</span></div></a>"#,
            href = href,
            tint = tint,
            icon = svg(icon),
            title = esc(title),
            detail = esc(detail),
            chip = chip,
            state_label = esc(state_label),
        )
    };
    let allowance = match (p.allowance, p.allowance_used_percent()) {
        (Some(limit), Some(used)) => row(
            "chart",
            if used >= rules::ALLOWANCE_WARN_PERCENT { "red" } else { "slate" },
            "Sending allowance",
            &format!("{} of {limit} messages this period", p.allowance_used),
            if used >= rules::ALLOWANCE_WARN_PERCENT { "error" } else { "tonal" },
            &format!("{used:.0}%"),
            "/ghost/newsletters/settings/",
        ),
        // No allowance is the normal self-hosted case, and a row saying
        // "unlimited" would be a row about nothing.
        _ => String::new(),
    };
    format!(
        r#"<div><div class="section">Delivery &amp; measurement</div>
<div class="card rows rise" style="--i:4">{sending}{transport}{opens}{clicks}{allowance}</div></div>"#,
        sending = row(
            "send",
            if p.sending_on { "green" } else { "red" },
            "Newsletter sending",
            if p.sending_on {
                "Publishing a post can email it"
            } else {
                "Publishing a post emails nobody"
            },
            if p.sending_on { "published" } else { "error" },
            if p.sending_on { "On" } else { "Off" },
            "/ghost/newsletters/settings/",
        ),
        transport = match p.transport.as_deref() {
            Some(name) => row(
                "mail",
                "green",
                "Carried by",
                "The transport the send loop will actually use",
                "published",
                name,
                "/ghost/newsletters/settings/",
            ),
            None => row(
                "warning",
                "red",
                "Carried by",
                p.transport_error.as_deref().unwrap_or("Nothing is configured"),
                "error",
                "Nothing",
                "/ghost/newsletters/settings/",
            ),
        },
        opens = row(
            "eye",
            if p.track_opens { "amber" } else { "slate" },
            "Open tracking",
            if p.track_opens {
                "Every delivered message records whether it was opened"
            } else {
                "Open rates read as \"not measured\" rather than as a number"
            },
            if p.track_opens { "published" } else { "tonal" },
            if p.track_opens { "On" } else { "Off" },
            "/ghost/analytics/settings/",
        ),
        clicks = row(
            "link",
            if p.track_clicks { "pink" } else { "slate" },
            "Click tracking",
            if p.track_clicks {
                "Links in a newsletter are counted per member"
            } else {
                "No link in a newsletter is counted"
            },
            if p.track_clicks { "published" } else { "tonal" },
            if p.track_clicks { "On" } else { "Off" },
            "/ghost/analytics/settings/",
        ),
        allowance = allowance,
    )
}

/// The last few editions, as a card on the overview.
async fn recent_editions_card(state: &AppState) -> String {
    let editions = repo::editions(&state.db, 5, 0).await.unwrap_or_default();
    let body = if editions.is_empty() {
        r#"<div class="row"><div class="body"><span class="title">Nothing emailed yet</span>
<span class="detail">Publishing a post to a newsletter puts it here.</span></div></div>"#
            .to_string()
    } else {
        editions.iter().map(|e| edition_row(e, false)).collect::<String>()
    };
    format!(
        r#"<div><div class="section">Latest editions</div>
<div class="card rows rise" style="--i:4">{body}</div>
<p class="muted" style="margin:10px 0 0 16px;font-size:12.5px"><a href="{all}">Every edition &rarr;</a></p></div>"#,
        body = body,
        all = EDITIONS_PATH,
    )
}

/// Every newsletter with its reach, on the overview.
async fn newsletter_list_card(state: &AppState) -> String {
    let stats = repo::stats(&state.db).await.unwrap_or_default();
    if stats.is_empty() {
        return format!(
            r#"<div class="section">Newsletters</div>
<div class="card padded rise" style="--i:5"><p style="margin:0 0 12px">There is no newsletter, so a post has nowhere to be emailed to.</p>
<a class="btn tonal" href="/ghost/newsletters/settings/">{send} Set one up</a></div>"#,
            send = svg("send"),
        );
    }
    let rows: String = stats
        .iter()
        .map(|n| {
            let detail = format!(
                "{} · {}",
                if n.is_sending() { "Sending" } else { "Archived" },
                if n.visibility == "paid" { "paid members only" } else { "all members" },
            );
            let inner = format!(
                r#"<span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip tonal">{subs}</span></div>"#,
                tint = if n.is_sending() { "purple" } else { "slate" },
                icon = svg("send"),
                name = esc(&n.name),
                detail = esc(&detail),
                subs = esc(&countable(n.subscribers, "subscriber")),
            );
            if vuuut_core::ids::is_objectid(&n.id) {
                format!(
                    r#"<a class="row" href="/ghost/newsletters/settings/{id}/">{inner}</a>"#,
                    id = esc(&n.id),
                )
            } else {
                format!(r#"<div class="row">{inner}</div>"#)
            }
        })
        .collect();
    format!(
        r#"<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:5">{rows}</div>"#,
    )
}

mod editions;

// Re-exported so the router keeps naming every handler
// `newsletters::<handler>`, which is what rule 5 asks for and what makes the
// route block readable.
pub(crate) use editions::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn edition(status: &str) -> repo::Edition {
        repo::Edition {
            id: "0123456789abcdef01234567".into(),
            post_id: "0123456789abcdef01234568".into(),
            post_title: Some("The Friday letter".into()),
            post_slug: Some("the-friday-letter".into()),
            subject: Some("The Friday letter".into()),
            newsletter_name: Some("Weekly".into()),
            status: status.into(),
            error: None,
            created_at: "2026-11-14 09:00:00".into(),
            submitted_at: Some("2026-11-14 09:01:00".into()),
            attempted: 100,
            delivered: 98,
            opened: 40,
            failed: 2,
        }
    }

    fn programme() -> Programme {
        Programme {
            newsletters: 1,
            sending_newsletters: 1,
            subscribers: 180,
            members: 200,
            editions: 4,
            attempted: 400,
            delivered: 396,
            opened: 150,
            failed: 4,
            sending_on: true,
            transport: Some("Mailgun".into()),
            track_opens: true,
            track_clicks: true,
            ..Programme::default()
        }
    }

    #[test]
    fn an_unmeasured_rate_is_a_dash_and_not_a_zero() {
        // The distinction the whole screen turns on: open tracking off means
        // the number was never taken, not that nobody opened.
        assert_eq!(percent(None), "—");
        assert_eq!(percent(Some(12.34)), "12.3%");
        // And a real zero still reads as a zero: the dash is for "unmeasured",
        // not for "small".
        assert_eq!(percent(Some(0.0)), "0.0%");
    }

    #[test]
    fn the_delivery_card_says_what_actually_carries_the_mail() {
        let card = delivery_card(&programme());
        assert!(card.contains("Carried by"));
        assert!(card.contains(">Mailgun<"), "{card}");
        assert!(card.contains(r#"href="/ghost/analytics/settings/""#));
    }

    #[test]
    fn the_delivery_card_repeats_the_resolvers_reason_when_nothing_is_configured() {
        let mut p = programme();
        p.transport = None;
        p.transport_error = Some("Configure Mailgun in Settings.".into());
        let card = delivery_card(&p);
        assert!(card.contains("Configure Mailgun in Settings."), "{card}");
        assert!(card.contains(">Nothing<"));
    }

    #[test]
    fn the_delivery_card_omits_an_allowance_row_when_there_is_no_allowance() {
        // "Unlimited" would be a row about nothing, which is the normal
        // self-hosted case.
        let mut p = programme();
        assert!(!delivery_card(&p).contains("Sending allowance"));
        p.allowance = Some(1_000);
        p.allowance_used = 900;
        let card = delivery_card(&p);
        assert!(card.contains("Sending allowance"));
        assert!(card.contains("900 of 1000 messages"), "{card}");
        assert!(card.contains(">90%<"));
    }

    #[test]
    fn a_tile_without_a_destination_is_not_drawn_as_a_link() {
        let linked = tile("Subscribers", "12", "d", Some("/ghost/newsletters/"), "send", "purple");
        assert!(linked.starts_with("<a class=\"card padded nl-tile\""));
        let plain = tile("Subscribers", "12", "d", None, "send", "purple");
        assert!(plain.starts_with("<div class=\"card padded nl-tile\""));
        assert!(!plain.contains("href"));
    }

    #[test]
    fn a_gap_row_carries_its_code_as_an_anchor_and_its_fix_as_a_link() {
        let gap = Gap {
            code: "no-transport",
            severity: Severity::Blocking,
            title: "Nothing is configured to send newsletters",
            detail: "Set up Mailgun.".into(),
            fix: Some(("Newsletter settings", "/ghost/newsletters/settings/")),
        };
        let row = gap_row(&gap);
        assert!(row.contains(r#"id="gap-no-transport""#));
        assert!(row.contains(r#"href="/ghost/newsletters/settings/""#));
        assert!(row.contains("chip error"));
    }

    #[test]
    fn a_gap_with_nothing_to_fix_draws_no_button() {
        let gap = Gap {
            code: "never-sent",
            severity: Severity::Opportunity,
            title: "No edition has gone out yet",
            detail: "Nothing to report on.".into(),
            fix: None,
        };
        assert!(!gap_row(&gap).contains("<a"));
    }

    #[test]
    fn the_tab_bar_marks_one_tab_and_sends_the_list_to_its_long_standing_url() {
        for active in ["overview", "editions", "list"] {
            assert_eq!(tabs(active).matches("seg on").count(), 1, "{active}");
        }
        assert!(tabs("overview").contains(r#"href="/ghost/newsletters/settings/""#));
    }

    #[test]
    fn the_window_bar_offers_only_windows_the_reducer_accepts() {
        for (key, days, _) in rules::WINDOWS {
            let bar = window_bar("/ghost/newsletters/", *days);
            assert!(bar.contains(&format!(r#"href="/ghost/newsletters/?days={key}""#)), "{bar}");
            assert_eq!(rules::window_days(Some(key)), *days);
        }
        assert_eq!(window_bar("/ghost/newsletters/", 30).matches("seg on").count(), 1);
        // A forged window still marks exactly one tab, the default's.
        let forged = rules::window_days(Some("4000"));
        assert_eq!(window_bar("/ghost/newsletters/", forged).matches("seg on").count(), 1);
    }

    #[test]
    fn an_edition_with_no_post_and_no_subject_still_has_clickable_text() {
        let mut e = edition("submitted");
        e.post_title = None;
        assert_eq!(e.title(), "The Friday letter", "the subject should stand in");
        e.subject = Some("   ".into());
        assert_eq!(e.title(), "Untitled edition");
        e.subject = None;
        assert_eq!(e.title(), "Untitled edition");
    }

    #[test]
    fn a_flash_message_survives_punctuation_in_the_redirect() {
        // These messages hold apostrophes and em dashes; one interpolated
        // raw would truncate the redirect at the first `&`.
        let response = flash_to(EDITIONS_PATH, "ok", "Sent again — 3 of 4 reached.");
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(location.starts_with(EDITIONS_PATH));
        assert!(location.contains("ok=Sent+again"), "{location}");
        assert!(!location.contains('—'), "{location}");
    }
}
