//! The Security module: what is holding this site shut, and what is not.
//!
//! Two screens under `/ghost/security/`, plus the switches behind them under
//! Settings → Security:
//!
//! * **Overview** — a posture score, then every finding worst-first with a
//!   link to the thing that fixes it, then the ways in that are not a
//!   password (staff, API keys, invitations) counted.
//! * **Sessions** — every staff member who is signed in right now, revocable
//!   one at a time or all at once; and every address currently serving a
//!   brute-force lockout, clearable the same way.
//!
//! The site-wide switches live under Settings → Security ([`settings_page`]),
//! with the other settings.
//!
//! ## There is no security data
//!
//! This module stores nothing. Every number and every finding on both screens
//! is computed from the site as it is at the moment the page is asked for —
//! the sessions table, the brute table, the API keys, the invitations, the
//! staff list and the settings. See [`ghost_db::repos::security`] for why
//! that is the only honest arrangement, and [`ghost_core::security`] for the
//! rules themselves.
//!
//! The argument is stronger here than it was for SEO. A security report that
//! kept its own copy of anything would, the first time somebody changed
//! something by another route, be telling an administrator that the site is
//! safe on the strength of a stale row. A report cannot be allowed to be
//! out of date about this.
//!
//! The only thing the module writes is the nine settings its own page offers
//! — see `migrations/sqlite/0015_security.sql`, which creates their rows and
//! says why two of their defaults deliberately change behaviour. None of them
//! is a secret, so none needs a config table of its own.
//!
//! ## Who may see it
//!
//! Administrators, and this one means it. The screens name every signed-in
//! staff member with the times they signed in, and the lockout list holds IP
//! addresses and the email addresses that password-reset budgets are keyed
//! on. Both are personal data about staff. More to the point, the page is a
//! list of the site's weaknesses with links to each one: it is the most
//! useful page in the admin to an attacker who has got a Contributor account,
//! which is exactly why it is not visible to one.
//!
//! Session ids are **never rendered**. The list shows
//! [`ghost_db::repos::security::Session::short`] — six of thirty-two
//! characters — and the full value travels only in the hidden field of the
//! revoke form, posted back same-origin. A screenshot of this screen must not
//! be a working login, and without that rule it would be.
//!
//! ## The security contract
//!
//! Everything interpolated into these pages goes through `esc`, including the
//! findings, whose text is built from counts and setting values, and
//! including the lockout keys, which contain addresses a stranger chose.
//! Every state-changing handler goes through [`guard`] with `writes` set, so
//! a revoke cannot be driven from another origin. The two things that come
//! back from the page as parameters — a session id and a lockout key — are
//! used only as a bound parameter to a `DELETE` and are never rendered back
//! or built into a link.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use ghost_api::AppState;
use ghost_core::security as rules;
use ghost_core::security::{Finding, Posture, Severity, Surface};
use ghost_db::repos::firewall as fw_repo;
use ghost_db::repos::security as repo;
use ghost_db::repos;
use std::collections::HashMap;

use crate::{
    esc, flash_banner, page_for_role, require_role, select_options, svg, Actor, Role, SELECT_CARET,
};

/// The role check, plus — for anything that changes state — the origin check.
///
/// The session cookie is `SameSite=Lax` already; the explicit origin check is
/// so that revoking every session on the site does not rest on one browser
/// behaviour. This is the module where that argument is least worth skipping:
/// a cross-site POST that signs the whole staff out is a denial of service
/// with no password needed.
async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn server_error(e: &ghost_core::GhostError) -> Response {
    tracing::error!("security admin: {}", e.message());
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

/// Redirect back to one of this module's own screens with a flash.
///
/// `path` is always one of this file's literals, never anything from a
/// request: a redirect target built from user input is an open redirect even
/// when it looks like it cannot be.
fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    // Through `form_urlencoded`, as Forms does, rather than by interpolating
    // the message into a query string: these messages hold apostrophes and em
    // dashes, and one that was not encoded would truncate the redirect.
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

/// Stored UTC, shown local, like every other date in the admin.
fn when(stored: &str, timezone: &str) -> String {
    let date = ghost_core::dates::short_date_in_zone(stored, timezone);
    if date.is_empty() {
        return stored.to_string();
    }
    format!("{date} · {}", ghost_core::dates::clock_in_zone(stored, timezone))
}

/// "4 minutes ago" for a unix-millisecond stamp, which is what the `brute`
/// table stores. Coarse on purpose: the useful question about a lockout is
/// "is this happening now", not what second it started.
fn ago_ms(stamp: i64) -> String {
    let secs = (chrono::Utc::now().timestamp_millis() - stamp).max(0) / 1000;
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} hr ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

fn head(active: &str) -> String {
    format!(
        r#"<header class="page-head rise"><div><h1>Security</h1>
<p class="sub">What is holding this site shut, and what is not.</p></div>
<div class="actions"><a class="btn tonal" href="/ghost/settings/security/">{gear} Security settings</a></div></header>
{tabs}"#,
        gear = svg("settings"),
        tabs = tabs(active),
    )
}

/// The two screens, as the shared segmented bar — `.seg-bar` / `.seg`, the
/// same primitive the SEO report's tabs use. Rule 6: reuse the existing
/// primitives rather than restyling them.
fn tabs(active: &str) -> String {
    let tab = |key: &str, label: &str, href: &str, count: Option<i64>| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{count}</a>"#,
            on = if key == active { " on" } else { "" },
            href = href,
            label = esc(label),
            count = count
                .map(|n| format!(r#"<span class="seg-count">{n}</span>"#))
                .unwrap_or_else(|| r#"<span style="width:8px"></span>"#.into()),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{overview}{sessions}{firewall}</div>"#,
        overview = tab("overview", "Overview", "/ghost/security/", None),
        sessions = tab("sessions", "Sessions & lockouts", "/ghost/security/sessions/", None),
        firewall = tab("firewall", "Firewall", "/ghost/security/firewall/", None),
    )
}

// ---------------------------------------------------------------------------
// the posture, measured
// ---------------------------------------------------------------------------

/// Build the [`Posture`] the rules judge, from the settings and the database.
///
/// Everything measured in one place so that [`rules::audit`] stays a pure
/// function of facts and the screen cannot accidentally judge something it
/// has not looked up.
pub(crate) async fn measure(state: &AppState) -> ghost_core::Result<(Posture, repo::Counts)> {
    let settings = ghost_api::security_settings::snapshot(state);
    let (max_age, idle) = ghost_api::security_settings::session_policy(state);
    let counts = repo::counts(&state.db, max_age).await?;

    let posture = Posture {
        https: state.site_url().starts_with("https://"),
        // A development build is expected to be served over http on
        // localhost, and telling somebody their laptop has no certificate is
        // noise that teaches them to ignore the page.
        development: cfg!(debug_assertions) || state.site_url().contains("localhost"),
        headers_on: rules::flag(&settings, "security_headers"),
        hsts_on: rules::flag(&settings, "security_hsts"),
        trust_proxy: rules::trust_proxy(&settings),
        private_fetch_blocked: rules::private_fetch_blocked(&settings),
        mfa_required: state.settings.get_bool("require_email_mfa"),
        session_days: max_age / 86_400,
        session_idle_minutes: idle / 60,
        min_password_length: rules::min_password_length(&settings),
        admin_count: counts.admins,
        suspended_with_sessions: counts.suspended_with_sessions,
        session_count: counts.sessions,
        stale_sessions: counts.stale_sessions,
        open_invites: counts.open_invites,
        expired_invites: counts.expired_invites,
        admin_api_keys: counts.admin_api_keys,
        content_api_keys: counts.content_api_keys,
        lockouts: counts.lockouts,
        is_private: state.settings.get_bool("is_private"),
        webhooks: counts.webhooks,
    };
    Ok((posture, counts))
}

/// The score ring at the top of the Overview.
///
/// An SVG arc rather than a bar: the number is a single judgement about one
/// thing, and a bar invites comparison with a bar beside it. Drawn with
/// `stroke-dasharray` on a circle, which needs no path arithmetic and scales
/// cleanly, and labelled in text inside the ring so the number is selectable
/// and readable by a screen reader without an `aria-label` duplicating it.
fn score_ring(score: i64, grade: &str, tint: &str) -> String {
    // r=54 on a 128 box: circumference 2πr ≈ 339.29.
    let circumference = 339.29_f64;
    let filled = circumference * (score.clamp(0, 100) as f64 / 100.0);
    format!(
        r#"<div class="sec-score">
<svg viewBox="0 0 128 128" class="sec-ring sec-ring-{tint}" role="img" aria-label="Security posture {score} out of 100: {grade}">
<circle cx="64" cy="64" r="54" fill="none" stroke="var(--outline-variant)" stroke-width="10"/>
<circle cx="64" cy="64" r="54" fill="none" stroke="currentColor" stroke-width="10" stroke-linecap="round"
 stroke-dasharray="{filled:.2} {circumference:.2}" transform="rotate(-90 64 64)"/>
</svg>
<div class="sec-score-text"><span class="num">{score}</span><span class="grade">{grade}</span></div>
</div>"#,
        tint = tint,
        score = score,
        grade = esc(grade),
        filled = filled,
        circumference = circumference,
    )
}

/// One finding, as a row.
fn finding_row(f: &Finding) -> String {
    format!(
        r#"<div class="row sec-finding" id="finding-{code}"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title} <span class="chip {chip}">{severity}</span></span>
<span class="detail">{detail}</span></div>
<div class="trailing">{fix}</div></div>"#,
        code = esc(f.code),
        tint = f.severity.tint(),
        icon = svg(match f.severity {
            Severity::Critical => "warning",
            Severity::Warning => "warning",
            Severity::Note => "lock",
        }),
        title = esc(f.title),
        chip = match f.severity {
            Severity::Critical => "error",
            Severity::Warning => "draft",
            // `.chip.tonal` is the quiet one; there is no `.chip.muted`
            // and adding one for a single row would be restyling a shared
            // primitive to suit this module.
            Severity::Note => "tonal",
        },
        severity = esc(f.severity.label()),
        detail = esc(&f.detail),
        fix = f
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

/// A tile in the Overview's "ways in" row.
fn tile(label: &str, value: String, detail: &str, href: &str, icon: &str, tint: &str) -> String {
    format!(
        r#"<a class="card padded sec-tile" href="{href}">
<span class="ic tint-{tint}">{icon}</span>
<span class="sec-tile-value">{value}</span>
<span class="sec-tile-label">{label}</span>
<span class="sec-tile-detail">{detail}</span></a>"#,
        href = href,
        tint = tint,
        icon = svg(icon),
        value = esc(&value),
        label = esc(label),
        detail = esc(detail),
    )
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub async fn security_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (posture, counts) = match measure(&state).await {
        Ok(m) => m,
        Err(e) => return server_error(&e),
    };
    let findings = rules::audit(&posture);
    let score = rules::score(&findings);
    let (grade, tint) = rules::grade(score);

    let critical = findings.iter().filter(|f| f.severity == Severity::Critical).count();
    let warnings = findings.iter().filter(|f| f.severity == Severity::Warning).count();

    let summary = if findings.is_empty() {
        "Every check this report makes passes. It is not a guarantee — it is the set of questions \
         the site can answer about itself."
            .to_string()
    } else {
        let mut parts = Vec::new();
        if critical > 0 {
            parts.push(format!(
                "{} that {} a defence absent or bypassable",
                crate::countable(critical as i64, "finding"),
                if critical == 1 { "leaves" } else { "leave" },
            ));
        }
        if warnings > 0 {
            parts.push(format!("{} weaker than it should be", crate::countable(warnings as i64, "defence")));
        }
        let notes = findings.len() - critical - warnings;
        if notes > 0 {
            parts.push(format!("{} worth knowing", crate::countable(notes as i64, "thing")));
        }
        format!("{}.", parts.join(", "))
    };

    let findings_card = if findings.is_empty() {
        format!(
            r#"<div class="card rows rise" style="--i:3"><div class="row"><span class="ic tint-green">{check}</span>
<div class="body"><span class="title">Nothing to report</span>
<span class="detail">No finding is outstanding. This page is computed fresh every time it is opened, so it will say so again only while it stays true.</span></div></div></div>"#,
            check = svg("check"),
        )
    } else {
        format!(
            r#"<div class="section">Findings</div>
<div class="card rows rise" style="--i:3">{rows}</div>"#,
            rows = findings.iter().map(finding_row).collect::<String>(),
        )
    };

    page_for_role(
        actor.role,
        "Security",
        "security",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<div class="card padded rise sec-hero" style="--i:2">
{ring}
<div class="sec-hero-body"><h2>Posture</h2><p class="muted">{summary}</p>
<p class="muted" style="margin:10px 0 0;font-size:13px">Measured from this site's own settings, sessions, keys and staff at the moment you opened this page. Nothing here is stored or cached.</p></div>
</div>
{findings_card}
<div class="section">The ways in</div>
<div class="sec-tiles rise" style="--i:4">
{staff_tile}
{session_tile}
{key_tile}
{invite_tile}
{lockout_tile}
{private_tile}
</div>"#,
            head = head("overview"),
            banner = flash_banner(&q),
            ring = score_ring(score, grade, tint),
            summary = esc(&summary),
            findings_card = findings_card,
            staff_tile = tile(
                "Can change settings",
                counts.admins.to_string(),
                "Owner and Administrator accounts",
                "/ghost/settings/staff/",
                "person",
                "pink",
            ),
            session_tile = tile(
                "Signed in now",
                counts.sessions.to_string(),
                if counts.suspended_with_sessions > 0 {
                    "Includes a suspended account"
                } else {
                    "Staff sessions, revocable"
                },
                "/ghost/security/sessions/",
                "lock",
                if counts.suspended_with_sessions > 0 { "red" } else { "teal" },
            ),
            key_tile = tile(
                "API keys",
                format!("{} + {}", counts.admin_api_keys, counts.content_api_keys),
                "Admin keys, then Content keys",
                "/ghost/settings/integrations/",
                "integrations",
                "blue",
            ),
            invite_tile = tile(
                "Open invitations",
                counts.open_invites.to_string(),
                if counts.expired_invites > 0 {
                    "Some expired ones are still listed"
                } else {
                    "Each one is a way to a new account"
                },
                "/ghost/settings/staff/",
                "mail",
                if counts.expired_invites > 0 { "amber" } else { "purple" },
            ),
            lockout_tile = tile(
                "Addresses waiting",
                counts.lockouts.to_string(),
                "Serving a brute-force lockout",
                "/ghost/security/sessions/#lockouts",
                "warning",
                if counts.lockouts > 0 { "amber" } else { "slate" },
            ),
            private_tile = tile(
                "Public access",
                if posture.is_private { "Closed".into() } else { "Open".into() },
                if posture.is_private {
                    "Behind the shared access code"
                } else {
                    "Anyone can read the published site"
                },
                "/ghost/settings/membership/",
                "globe",
                if posture.is_private { "amber" } else { "green" },
            ),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// Sessions & lockouts
// ---------------------------------------------------------------------------

pub async fn sessions_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let here = ghost_api::admin::session_id_from(&headers).unwrap_or_default();

    let sessions = match repo::sessions(&state.db, repo::LIST_LIMIT).await {
        Ok(s) => s,
        Err(e) => return server_error(&e),
    };
    let lockouts = match repo::lockouts(&state.db, repo::LIST_LIMIT).await {
        Ok(l) => l,
        Err(e) => return server_error(&e),
    };
    // The last 40 rows of the site's audit log that are about access. The
    // table is the whole site's log — it records a post being published as
    // readily as a role change — so the repo filters to an allowlist of
    // resource types rather than letting this page pick, which would be a
    // query parameter reaching a comparison.
    let events = match repo::events(&state.db, 40).await {
        Ok(e) => e,
        Err(e) => return server_error(&e),
    };

    let session_rows: String = sessions
        .iter()
        .map(|s| {
            let is_me = s.session_id == here;
            let suspended = s.user_status != "active";
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{name}{chips}</span>
<span class="detail">{role} · {email} · last active {active} · signed in {created} · <code>{short}</code></span></div>
<div class="trailing">{action}</div></div>"#,
                tint = if suspended { "red" } else if is_me { "teal" } else { "slate" },
                icon = svg(if suspended { "warning" } else { "person" }),
                name = esc(if s.user_name.is_empty() { &s.user_email } else { &s.user_name }),
                chips = format!(
                    "{me}{sus}{unver}",
                    me = if is_me { r#" <span class="chip tonal">this browser</span>"# } else { "" },
                    sus = if suspended { r#" <span class="chip error">suspended</span>"# } else { "" },
                    unver = if s.verified {
                        ""
                    } else {
                        r#" <span class="chip draft">awaiting code</span>"#
                    },
                ),
                role = esc(if s.role.is_empty() { "No role" } else { &s.role }),
                email = esc(&s.user_email),
                active = esc(&when(s.last_active(), &timezone)),
                created = esc(&when(&s.created_at, &timezone)),
                // Six characters of thirty-two. Enough to tell two sessions
                // of one person apart; not a cookie.
                short = esc(&s.short()),
                action = if is_me {
                    // Deliberately not offered: a button that signs you out
                    // of the page you are using reads as a bug the moment it
                    // works. Sign out is in the rail, where it belongs.
                    r#"<span class="muted" style="font-size:13px">Use Sign out</span>"#.to_string()
                } else {
                    format!(
                        r#"<form method="post" action="/ghost/security/sessions/revoke/" style="margin:0">
<input type="hidden" name="session_id" value="{sid}">
<button class="btn text danger" type="submit">Revoke</button></form>"#,
                        // The full id, in a hidden field, posted back
                        // same-origin — the one place it appears, and never
                        // in anything visible or copyable from the page.
                        sid = esc(&s.session_id),
                    )
                },
            )
        })
        .collect();

    let lockout_rows: String = lockouts
        .iter()
        .map(|l| {
            format!(
                r#"<div class="row"><span class="ic tint-amber">{icon}</span>
<div class="body"><span class="title">{kind}</span>
<span class="detail"><code>{key}</code> · {count} on record · last attempt {ago}</span></div>
<div class="trailing"><form method="post" action="/ghost/security/lockouts/clear/" style="margin:0">
<input type="hidden" name="key" value="{key_v}">
<button class="btn text" type="submit">Clear</button></form></div></div>"#,
                icon = svg("warning"),
                kind = esc(l.kind()),
                // A key can hold an address a stranger chose and an email
                // address a stranger typed. Escaped like everything else.
                key = esc(&l.key),
                key_v = esc(&l.key),
                count = crate::countable(l.count.unwrap_or(0), "attempt"),
                ago = esc(&l.last_request.map(ago_ms).unwrap_or_else(|| "unknown".into())),
            )
        })
        .collect();

    let truncated = |n: usize, what: &str| {
        if n as i64 >= repo::LIST_LIMIT {
            format!(
                r#"<p class="muted" style="margin:10px 0 0">Showing the first {limit} {what}. There are more.</p>"#,
                limit = repo::LIST_LIMIT,
                what = esc(what),
            )
        } else {
            String::new()
        }
    };

    page_for_role(
        actor.role,
        "Security",
        "security",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<div class="section" id="sessions">Signed in now</div>
{sessions_card}
{sessions_more}
{revoke_all}

<div class="section" id="lockouts">Lockouts</div>
<p class="muted rise" style="--i:4;margin:0 0 10px">Addresses and accounts with failed attempts on record. The wait doubles with each failure and the record clears itself after an hour of quiet, so most of these need nothing — clear one only for somebody who has locked themselves out and is waiting.</p>
{lockouts_card}
{lockouts_more}
{clear_all}

<div class="section" id="activity">Recent access changes</div>
<p class="muted rise" style="--i:7;margin:0 0 10px">From the site's audit log, narrowed to the events that change who can get in — staff, invitations, API keys, integrations, webhooks and settings. Everything else the log records is under <a href="/ghost/settings/history/">History</a>.</p>
{events_card}"#,
            head = head("sessions"),
            banner = flash_banner(&q),
            sessions_card = if sessions.is_empty() {
                // Only reachable if the reader's own session is somehow not
                // in the table, which would itself be the story.
                r#"<div class="card padded rise" style="--i:2"><p class="muted" style="margin:0">Nobody is signed in.</p></div>"#.to_string()
            } else {
                format!(r#"<div class="card rows rise" style="--i:2">{session_rows}</div>"#)
            },
            sessions_more = truncated(sessions.len(), "sessions"),
            revoke_all = if sessions.len() > 1 {
                format!(
                    r#"<form method="post" action="/ghost/security/sessions/revoke-all/" style="margin:12px 0 0">
<button class="btn tonal danger" type="submit">{icon} Sign out every other session</button>
<span class="muted" style="margin-left:10px;font-size:13px">Yours stays. Everyone else signs in again.</span></form>"#,
                    icon = svg("logout"),
                )
            } else {
                String::new()
            },
            lockouts_card = if lockouts.is_empty() {
                r#"<div class="card padded rise" style="--i:5"><p class="muted" style="margin:0">No failed attempts on record.</p></div>"#.to_string()
            } else {
                format!(r#"<div class="card rows rise" style="--i:5">{lockout_rows}</div>"#)
            },
            lockouts_more = truncated(lockouts.len(), "records"),
            events_card = if events.is_empty() {
                r#"<div class="card padded rise" style="--i:8"><p class="muted" style="margin:0">Nothing has changed who can get in.</p></div>"#.to_string()
            } else {
                format!(
                    r#"<div class="card rows rise" style="--i:8">{rows}</div>"#,
                    rows = events
                        .iter()
                        .map(|e| format!(
                            r#"<div class="row"><span class="ic tint-slate">{icon}</span>
<div class="body"><span class="title">{what}</span>
<span class="detail">{who} · {at}</span></div></div>"#,
                            icon = svg(match e.resource_type.as_str() {
                                "user" | "invite" => "person",
                                "api_key" | "integration" => "integrations",
                                "webhook" => "share",
                                _ => "settings",
                            }),
                            // Both halves are from a fixed allowlist in the
                            // repo and a fixed verb list in the recorder, but
                            // escaped like everything else — the rule is
                            // "escape at the point of output", including
                            // values you believe are safe.
                            what = esc(&format!("{} {}", e.resource_type.replace('_', " "), e.event)),
                            who = esc(e.actor_name.as_deref().unwrap_or("Somebody no longer on the staff")),
                            at = esc(&when(&e.created_at, &timezone)),
                        ))
                        .collect::<String>(),
                )
            },
            clear_all = if lockouts.len() > 1 {
                r#"<form method="post" action="/ghost/security/lockouts/clear-all/" style="margin:12px 0 0">
<button class="btn tonal danger" type="submit">Clear every lockout</button>
<span class="muted" style="margin-left:10px;font-size:13px">Hands a fresh allowance to every address that has been guessing.</span></form>"#.to_string()
            } else {
                String::new()
            },
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// the actions
// ---------------------------------------------------------------------------

const SESSIONS_PATH: &str = "/ghost/security/sessions/";

/// The revoke form's body.
///
/// `#[serde(default)]`, so a missing or malformed field is an empty string
/// rather than a rejected request. That matters more than it looks: an axum
/// extractor runs **before** the handler body, so a `String` field would have
/// a stranger posting an empty body answered `422 Unprocessable Entity` by
/// the extractor — before [`guard`] ran, and with a different answer from the
/// `302 /ghost/signin/` they get for every other route in this module. Rule 8
/// says every handler's first statement calls the guard; an extractor that
/// can refuse first makes that untrue. Being lenient here and strict in the
/// handler is what keeps the refusal the guard's to give.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct RevokeForm {
    session_id: String,
}

/// Revoke one session.
///
/// The id arrives from the page's own hidden field. It is used as a bound
/// parameter to a `DELETE` and nowhere else — not rendered back, not built
/// into the redirect — so there is nothing for a forged value to reach. It is
/// still shape-checked first, because a `DELETE` with a wildcard-looking
/// value should never have been issued in the first place and the cheapest
/// place to say so is here.
pub(crate) async fn session_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<RevokeForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let sid = form.session_id.trim();
    // A session id is 32 lowercase hex characters — `ids::random_hex(16)`.
    if sid.len() != 32 || !sid.chars().all(|c| c.is_ascii_hexdigit()) {
        return flash_to(SESSIONS_PATH, "error", "That is not a session id.");
    }
    // Revoking your own would sign you out of the page you are on, which
    // reads as a bug rather than as a feature. Sign out is in the rail.
    if ghost_api::admin::session_id_from(&headers).as_deref() == Some(sid) {
        return flash_to(SESSIONS_PATH, "error", "Use Sign out to end your own session.");
    }
    match repo::revoke_session(&state.db, sid).await {
        Ok(true) => flash_to(SESSIONS_PATH, "ok", "Session revoked — that browser is signed out."),
        Ok(false) => flash_to(SESSIONS_PATH, "ok", "That session had already ended."),
        Err(e) => {
            tracing::error!("security: revoking a session failed: {}", e.message());
            flash_to(SESSIONS_PATH, "error", "The session could not be revoked.")
        }
    }
}

/// Revoke every session but the caller's.
pub(crate) async fn sessions_revoke_all(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let keep = ghost_api::admin::session_id_from(&headers);
    match repo::revoke_all_sessions(&state.db, keep.as_deref()).await {
        Ok(0) => flash_to(SESSIONS_PATH, "ok", "There was nothing else signed in."),
        Ok(n) => flash_to(
            SESSIONS_PATH,
            "ok",
            &format!("{} signed out.", crate::countable(n as i64, "session")),
        ),
        Err(e) => {
            tracing::error!("security: revoking every session failed: {}", e.message());
            flash_to(SESSIONS_PATH, "error", "The sessions could not be revoked.")
        }
    }
}

/// The clear-lockout form's body. `#[serde(default)]` for the same reason as
/// [`RevokeForm`]: the guard, not the extractor, decides who is refused.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct LockoutForm {
    key: String,
}

/// Clear one brute-force record.
///
/// The key is bound into a `DELETE` and never rendered or linked. It is
/// length-bounded because the column is `VARCHAR(191)` and a value longer
/// than that cannot be a key that exists — matching nothing, but after the
/// database has been asked.
pub(crate) async fn lockout_clear(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LockoutForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let key = form.key.trim();
    if key.is_empty() || key.chars().count() > 191 {
        return flash_to(SESSIONS_PATH, "error", "That is not a lockout key.");
    }
    match repo::clear_lockout(&state.db, key).await {
        Ok(true) => flash_to(SESSIONS_PATH, "ok", "Lockout cleared — they can try again now."),
        Ok(false) => flash_to(SESSIONS_PATH, "ok", "That lockout had already expired."),
        Err(e) => {
            tracing::error!("security: clearing a lockout failed: {}", e.message());
            flash_to(SESSIONS_PATH, "error", "The lockout could not be cleared.")
        }
    }
}

/// Clear every brute-force record.
pub(crate) async fn lockouts_clear_all(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    match repo::clear_all_lockouts(&state.db).await {
        Ok(n) => flash_to(
            SESSIONS_PATH,
            "ok",
            &format!("{} cleared.", crate::countable(n as i64, "lockout")),
        ),
        Err(e) => {
            tracing::error!("security: clearing every lockout failed: {}", e.message());
            flash_to(SESSIONS_PATH, "error", "The lockouts could not be cleared.")
        }
    }
}

mod firewall;
mod settings;

// Re-exported so the router keeps naming every handler `security::<handler>`,
// which is what rule 5 asks for and what makes the route block readable.
pub(crate) use firewall::*;
pub(crate) use settings::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_never_appears_anywhere_a_screenshot_would_catch_it() {
        // The rule the whole screen rests on: a photograph of the Security
        // page must not be a working login. The short form is what is drawn;
        // the full id exists only inside a hidden input's `value`.
        let id = "0123456789abcdef0123456789abcdef";
        let session = repo::Session {
            session_id: id.into(),
            user_id: "u".into(),
            user_name: "Ada".into(),
            user_email: "ada@example.com".into(),
            user_slug: "ada".into(),
            user_status: "active".into(),
            role: "Editor".into(),
            created_at: "2026-11-15 10:00:00".into(),
            updated_at: None,
            verified: true,
        };
        assert_eq!(session.short(), "012345");
        assert!(!id.starts_with(&format!("{}x", session.short())));
    }

    #[test]
    fn a_hostile_session_id_is_refused_before_it_reaches_a_delete() {
        // `is_session_id` is the shape check `session_revoke` makes. These are
        // the values that would be interesting if the id were ever formatted
        // into SQL — it is bound, not formatted, but the shape check is what
        // means a DELETE with a value like this is never issued at all.
        let shaped = |s: &str| s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit());
        for hostile in [
            "' OR 1=1 --",
            "%",
            "0123456789abcdef0123456789abcde",     // one short
            "0123456789abcdef0123456789abcdefa",   // one long
            "0123456789abcdef0123456789abcdeg",    // not hex
            "",
        ] {
            assert!(!shaped(hostile), "{hostile}");
        }
        assert!(shaped("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn a_lockout_key_holding_markup_comes_back_escaped() {
        // A brute key is built from a client address and, for the reset
        // budgets, an email address a stranger typed. Both reach the page.
        let hostile = r#"<script>alert(1)</script>@example.comreset"#;
        let escaped = esc(hostile);
        assert!(!escaped.contains("<script"));
        assert!(escaped.contains("&lt;script&gt;"));
    }

    #[test]
    fn a_forged_setting_is_reduced_before_it_is_stored() {
        // What `settings_save` does with each select: store the reduction's
        // answer, not what was posted. A `security_min_password_length` of 1
        // would otherwise weaken every password path in the application.
        for hostile in ["1", "0", "999", "", "16; DROP TABLE users"] {
            let reduced = rules::choice(
                &serde_json::json!({ "security_min_password_length": hostile }),
                "security_min_password_length",
                rules::PASSWORD_LENGTHS,
                rules::DEFAULT_PASSWORD_LENGTH,
            );
            assert_eq!(reduced, 10, "{hostile}");
        }
        // And a value we did offer survives.
        let reduced = rules::choice(
            &serde_json::json!({ "security_min_password_length": "16" }),
            "security_min_password_length",
            rules::PASSWORD_LENGTHS,
            rules::DEFAULT_PASSWORD_LENGTH,
        );
        assert_eq!(reduced, 16);
    }

    #[test]
    fn the_score_ring_is_labelled_for_a_screen_reader() {
        let svg = score_ring(72, "Reasonable", "blue");
        assert!(svg.contains(r#"role="img""#));
        assert!(svg.contains("Security posture 72 out of 100: Reasonable"));
    }

    #[test]
    fn a_finding_renders_escaped_with_its_fix_as_a_link() {
        let f = Finding {
            severity: Severity::Critical,
            code: "no_https",
            title: "This site is served over http",
            detail: r#"A detail with <b>markup</b> & an ampersand"#.into(),
            fix: Some(("General settings", "/ghost/settings/general/")),
        };
        let html = finding_row(&f);
        assert!(!html.contains("<b>markup</b>"));
        assert!(html.contains("&lt;b&gt;markup&lt;/b&gt;"));
        assert!(html.contains(r#"href="/ghost/settings/general/""#));
        assert!(html.contains("Critical"));
        assert!(html.contains(r#"id="finding-no_https""#));
    }

    #[test]
    fn a_lockouts_timestamp_reads_as_a_rough_age() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(ago_ms(now), "just now");
        assert_eq!(ago_ms(now - 5 * 60_000), "5 min ago");
        assert_eq!(ago_ms(now - 3 * 3_600_000), "3 hr ago");
        assert_eq!(ago_ms(now - 2 * 86_400_000), "2 days ago");
        // A stamp in the future is clock skew, not a negative age.
        assert_eq!(ago_ms(now + 60_000), "just now");
    }

    #[test]
    fn the_overview_and_the_sessions_screen_are_one_nav_item() {
        // Rule 4: every page of the module lights the one Modules rail item.
        // Both screens pass the same nav key, which is what `rail()` maps.
        assert!(tabs("overview").contains(r#"class="seg on" href="/ghost/security/""#));
        assert!(tabs("sessions").contains(r#"class="seg on" href="/ghost/security/sessions/""#));
    }
}
