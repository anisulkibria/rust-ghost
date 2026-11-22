//! The Memberships module: who reads this site, who pays for it, and what is
//! stopping the next person doing either.
//!
//! Three screens under `/ghost/memberships/`, plus the members list it
//! adopted and the settings it links to:
//!
//! * **Overview** — the funnel in six figures, growth over a window, every
//!   gap in the funnel worst-first, where the window's signups came from, the
//!   latest arrivals, and the newsletters they are subscribed to.
//! * **Members** — the list, at its own long-standing URL `/ghost/members/`.
//!   See "Where Members went", below.
//! * **Tiers** — every tier with its member count, its price and whether a
//!   visitor is ever shown it.
//! * **Offers** — every offer with its discount, its tier and how many people
//!   took it.
//!
//! The switches live under Settings → Membership, which existed long before
//! this module and is untouched by it.
//!
//! ## This module organises; it does not own
//!
//! Nothing here has a table, a setting or a migration of its own, and
//! **nothing here writes**. Members, tiers, offers, newsletters and
//! subscriptions are all already written by somebody else — the portal, the
//! Stripe webhook, the members list, the tier editor, the offer editor — and
//! every button on these screens either leads to the screen that owns the
//! thing or posts to the handler that already created it.
//!
//! That is the whole point of the module rather than a limitation of it. The
//! membership half of this admin was spread across six places that could not
//! see each other: the members list knew who had arrived, Settings →
//! Membership knew whether signups were open, Settings → Growth knew about
//! offers, Settings → Newsletters knew about reach, the dashboard knew the
//! MRR, and nothing at all knew whether those answers were consistent. A
//! visitor meets them as one path, and the commonest way for that path to be
//! broken is for two screens to each be individually correct — paid tiers
//! that exist, Stripe that is not connected. [`ghost_core::memberships`] is
//! where that judgement lives, with its own tests.
//!
//! Being read-only is also why [`guard`] takes no `writes` flag: there is no
//! state-changing handler in this file to origin-check. The two creation
//! buttons post to `settings::tier_create` and `settings::offer_new`, which
//! do their own role check, as they did before this module existed.
//!
//! ## Where Members went
//!
//! The members list used to be a rail item of its own. It is now a tab of
//! this module, and the rail item is gone — a publication's members are part
//! of its membership, not a sibling of it.
//!
//! What did **not** change is its URL. `/ghost/members/` and everything under
//! it stay exactly where they were, for the same reason Settings → Design and
//! Settings → Navigation keep their top-level pages: thirty-odd links inside
//! this admin point at them, as does every bookmark anybody has ever made,
//! and a module reorganisation is not a reason to break a URL. The list
//! carries a back-chevron to this overview instead of this module's tab bar,
//! because it already has a segmented bar of its own for the status filters
//! and two stacked bars read as a mistake.
//!
//! ## Who may see it
//!
//! Administrators, at the same level the members list itself is gated at, and
//! for the same reason: the overview names recent members with their email
//! addresses, which is personal data about readers. The figures beside them —
//! what the site earns, how fast it is growing — are commercially sensitive in
//! their own right, and neither is any of a Contributor's business.
//!
//! ## The security contract
//!
//! Everything interpolated into these pages goes through `esc`, including the
//! gap details (built from counts in `ghost-core`) and including member names
//! and email addresses, which readers chose and which are therefore the one
//! genuinely hostile input on the screen. Ids are checked with
//! `ghost_core::ids::is_objectid` before they are built into a link. The one
//! query parameter the pages accept, `?days=`, is reduced to one of three
//! offered windows by [`ghost_core::memberships::window_days`] before it is
//! used or linked.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ghost_api::AppState;
use ghost_core::memberships as rules;
use ghost_core::memberships::{Funnel, Gap, Severity};
use ghost_db::repos::memberships as repo;
use std::collections::HashMap;

use crate::{countable, esc, flash_banner, money, page_for_role, require_role, svg, Actor, Role};

/// The role check.
///
/// No `writes` flag, and no origin check, because this module has no
/// state-changing handler — see the module docs. A later handler that does
/// write must add both rather than reuse this, which is why this one says so
/// here instead of quietly taking a flag nobody passes.
async fn guard(state: &AppState, headers: &HeaderMap) -> Result<Actor, Response> {
    require_role(state, headers, Role::Administrator).await
}

fn server_error(e: &ghost_core::GhostError) -> Response {
    tracing::error!("memberships admin: {}", e.message());
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

/// The page header, shared by all three of this module's screens.
///
/// The primary action follows the tab, because "new" means a different thing
/// on each: a member, a tier, an offer. Two of the three are POSTs — creating
/// a tier or an offer inserts a draft row and opens its editor — and each
/// posts to the handler that has always owned that creation, under Settings.
/// One piece of code knows what a new tier looks like, and it is not this
/// module.
fn head(active: &str) -> String {
    let action = match active {
        "tiers" => create_button("/ghost/settings/memberships/tiers/new/", "New tier"),
        "offers" => create_button("/ghost/growth/offers/new/", "New offer"),
        _ => format!(
            r#"<a class="btn" href="/ghost/members/new/">{add} New member</a>"#,
            add = svg("add"),
        ),
    };
    format!(
        r#"<header class="page-head rise"><div><h1>Memberships</h1>
<p class="sub">Who reads this site, who pays for it, and what is stopping the next person doing either.</p></div>
<div class="actions"><a class="btn tonal" href="/ghost/settings/memberships/">{gear} Membership settings</a>
{action}</div></header>
{tabs}"#,
        gear = svg("settings"),
        action = action,
        tabs = tabs(active),
    )
}

/// A create button that posts to somebody else's handler.
///
/// `path` is always one of this file's literals. A button built out of
/// anything from a request would be a way to drive an arbitrary POST from the
/// admin's own chrome.
fn create_button(path: &str, label: &str) -> String {
    format!(
        r#"<form method="post" action="{path}" style="display:inline">
<button class="btn" type="submit">{add} {label}</button></form>"#,
        path = path,
        add = svg("add"),
        label = esc(label),
    )
}

/// The module's screens, as the shared segmented bar — the same `.seg-bar` /
/// `.seg` primitive the SEO report and the Security module use. Rule 6: reuse
/// the primitives rather than restyling them.
///
/// The Members tab leaves the module's own URL space, which is the one thing
/// about this bar worth knowing. It is a tab rather than a link in the header
/// because, to somebody using the admin, the list *is* one of this module's
/// screens; where its handler happens to be routed is not their problem.
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
        r#"<div class="seg-bar rise" style="--i:1">{overview}{members}{tiers}{offers}</div>"#,
        overview = tab("overview", "Overview", "/ghost/memberships/"),
        members = tab("members", "Members", "/ghost/members/"),
        tiers = tab("tiers", "Tiers", "/ghost/memberships/tiers/"),
        offers = tab("offers", "Offers", "/ghost/memberships/offers/"),
    )
}

/// A figure, what it counts, and one line about it. A link when there is
/// somewhere useful to go, a plain tile when there is not — a tile that looks
/// clickable and is not is worse than one that does not.
fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<span class="mb-tile-value">{value}</span>
<span class="mb-tile-label">{label}</span>
<span class="mb-tile-detail">{detail}</span>"#,
        tint = tint,
        icon = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(href) => format!(r#"<a class="card padded mb-tile" href="{href}">{body}</a>"#),
        None => format!(r#"<div class="card padded mb-tile">{body}</div>"#),
    }
}

/// A percentage, to one decimal place, or an em dash when there is none.
///
/// One place, because five tiles ask for it and `0.0%` and `—` mean very
/// different things — see [`rules::conversion_percent`] on why a rate with
/// nothing to divide by is absent rather than zero.
fn percent(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.1}%"),
        None => "—".into(),
    }
}

/// The trend line under a count: how this window compares with the one
/// before it.
fn trend(now: i64, before: i64, window: i64) -> String {
    match rules::growth_percent(now, before) {
        Some(pct) => format!(
            "{sign}{pct:.0}% against {before} in the previous {window} days",
            sign = if pct > 0.0 { "+" } else { "" },
            pct = pct,
            before = before,
            window = window,
        ),
        // No percentage from a standing start — see `growth_percent`.
        None if before == 0 && now > 0 => format!("none at all in the previous {window} days"),
        None => format!("nothing in the previous {window} days either"),
    }
}

/// One gap, as a row.
///
/// Carries `.mb-gap` as well as `.row` so the stylesheet can let a two-line
/// detail wrap — see the CSS block, which explains why the selector has to be
/// qualified `.rows .row.mb-gap` to win the cascade at all.
fn gap_row(g: &Gap) -> String {
    format!(
        r#"<div class="row mb-gap" id="gap-{code}"><span class="ic tint-{tint}">{icon}</span>
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
        // `.chip.tonal` is the quiet one. There is no `.chip.note`, and
        // adding one for a single row would be restyling a shared primitive
        // to suit this module.
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

/// The window picker, as a segmented bar of the three offered windows.
///
/// Every link is built from [`rules::WINDOWS`], so the set of links and the
/// set of values [`rules::window_days`] will accept are the same list.
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
// the growth chart
// ---------------------------------------------------------------------------

/// One day of the growth chart: the people who joined that day, split by what
/// they are now.
pub(crate) struct Day {
    pub(crate) day: String,
    pub(crate) free: i64,
    pub(crate) paying: i64,
}

/// Stacked columns of signups per day, free below and paying above.
///
/// Its own chart rather than `modules::views_chart`, for two reasons that are
/// both about honesty rather than taste. That chart labels every tooltip and
/// its `aria-label` "views", and a signup is not a view; and it draws one
/// series, where the useful question here is what proportion of a day's
/// arrivals ended up paying — which is a stack, not a column. The geometry,
/// the clean axis maximum and the `compact` ticks are shared with it.
fn growth_chart(days: &[Day]) -> String {
    if days.iter().all(|d| d.free + d.paying == 0) {
        return r#"<p class="muted" style="margin:8px 0 0">Nobody joined in this window.</p>"#.into();
    }
    let (w, h, pad_l, pad_b, pad_t) = (728usize, 180usize, 34usize, 22usize, 18usize);
    let plot_w = w - pad_l - 8;
    let plot_h = h - pad_b - pad_t;
    let slot = plot_w as f64 / days.len() as f64;
    let bar_w = slot.min(24.0) * 0.75;
    let axis = crate::clean_max(days.iter().map(|d| d.free + d.paying).max().unwrap_or(1));
    let y = |v: i64| pad_t as f64 + plot_h as f64 * (1.0 - v as f64 / axis as f64);

    let mut grid = String::new();
    for frac in [0.0, 0.5, 1.0] {
        let value = (axis as f64 * frac).round() as i64;
        let gy = y(value);
        grid.push_str(&format!(
            r#"<line x1="{pad_l}" x2="{right}" y1="{gy:.1}" y2="{gy:.1}" stroke="var(--outline-variant)" stroke-width="1"/>
<text x="{tick_x}" y="{ty:.1}" text-anchor="end" dominant-baseline="middle">{label}</text>"#,
            pad_l = pad_l,
            right = w - 8,
            gy = gy,
            tick_x = pad_l - 8,
            ty = gy + 3.0,
            label = crate::compact(value),
        ));
    }

    let mut bars = String::new();
    let mut labels = String::new();
    for (i, d) in days.iter().enumerate() {
        let cx = pad_l as f64 + slot * (i as f64 + 0.5);
        let show_label = days.len() <= 7 || i == 0 || i == days.len() - 1 || i == days.len() / 2;
        if show_label && d.day.len() >= 10 {
            labels.push_str(&format!(
                r#"<text x="{cx:.0}" y="{ly}" text-anchor="middle">{label}</text>"#,
                cx = cx,
                ly = h - 6,
                label = esc(&d.day[5..10]), // MM-DD
            ));
        }
        let total = d.free + d.paying;
        if total == 0 {
            continue;
        }
        // Drawn as two plain rectangles stacked, with the rounded cap only on
        // the top one: a cap on the free segment underneath would show as a
        // notch where the two meet.
        let x0 = cx - bar_w / 2.0;
        let base = (pad_t + plot_h) as f64;
        let y_total = y(total);
        let y_free = y(d.free);
        let title = format!(
            "{}: {} joined — {} free, {} paying",
            esc(&d.day),
            total,
            d.free,
            d.paying,
        );
        bars.push_str(&format!(r#"<g><title>{title}</title>"#));
        if d.free > 0 {
            bars.push_str(&format!(
                r#"<rect x="{x0:.1}" y="{y_free:.1}" width="{bw:.1}" height="{hh:.1}" fill="var(--primary)" opacity=".38"/>"#,
                x0 = x0,
                y_free = y_free,
                bw = bar_w,
                hh = (base - y_free).max(0.0),
            ));
        }
        if d.paying > 0 {
            let seg_h = (y_free - y_total).max(0.0);
            let r = 4.0_f64.min(seg_h);
            bars.push_str(&format!(
                r#"<rect x="{x0:.1}" y="{y_total:.1}" width="{bw:.1}" height="{hh:.1}" rx="{r:.1}" fill="var(--primary)"/>"#,
                x0 = x0,
                y_total = y_total,
                bw = bar_w,
                hh = seg_h,
                r = r,
            ));
        }
        bars.push_str("</g>");
    }

    format!(
        r#"<svg class="chart" viewBox="0 0 {w} {h}" role="img" aria-label="Signups per day, split by free and paying">{grid}{bars}{labels}</svg>
<div class="mb-legend"><span class="mb-key mb-key-paying"></span>Paying<span class="mb-key mb-key-free"></span>Free</div>"#,
        w = w,
        h = h,
        grid = grid,
        bars = bars,
        labels = labels,
    )
}

/// Fill the window's missing days with zeroes.
///
/// The repo returns only days somebody joined on — see
/// [`repo::signups_by_day`] — because only the screen knows how wide its
/// chart is. A chart drawn straight from those rows would space five bars
/// evenly across thirty days and read as steady growth.
fn fill_days(rows: &[(String, String, i64)], days: i64) -> Vec<Day> {
    let today = chrono::Utc::now().date_naive();
    (0..days)
        .rev()
        .map(|back| {
            let day = (today - chrono::Duration::days(back)).format("%Y-%m-%d").to_string();
            let on = |status: &str| {
                rows.iter()
                    .filter(|(d, s, _)| *d == day && s == status)
                    .map(|(_, _, n)| *n)
                    .sum::<i64>()
            };
            // Everything that is not `free` is somebody with a tier —
            // `paid`, `comped`, and any status an import invented. Counting
            // only `paid` would quietly drop comped members off the chart
            // while the tiles above still counted them.
            let free = on("free");
            let total: i64 = rows.iter().filter(|(d, _, _)| *d == day).map(|(_, _, n)| *n).sum();
            Day { day, free, paying: (total - free).max(0) }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

/// Assemble the [`Funnel`] the rules judge, from the settings and the counts.
///
/// In one place, so [`rules::audit`] stays a pure function of facts and the
/// screen cannot judge something it forgot to look up.
fn funnel_of(state: &AppState, counts: &repo::Counts, mrr_cents: i64) -> Funnel {
    let str_setting = |key: &str| state.settings.get_str(key).unwrap_or_default();
    let has = |key: &str| !str_setting(key).trim().is_empty();
    // `portal_plans` is stored as a JSON array. A malformed value reads as no
    // plans, which raises the "portal offers no plan" gap — the safe
    // direction, because it points at the screen that would show the problem.
    let portal_plans: Vec<String> = state
        .settings
        .get("portal_plans")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();
    // A suggested amount of zero is Ghost's "off": the portal shows no tip
    // option until somebody sets one.
    let donations_on = state
        .settings
        .get("donations_suggested_amount")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0)
        > 0;

    Funnel {
        members: counts.members,
        free: counts.free,
        paid: counts.paid,
        comped: counts.comped,
        email_disabled: counts.email_disabled,
        joined: counts.joined,
        joined_before: counts.joined_before,
        upgraded: counts.upgraded,
        cancelled: counts.cancelled,
        mrr_cents,
        currency: str_setting("stripe_currency"),
        paid_tiers: counts.paid_tiers,
        visible_paid_tiers: counts.visible_paid_tiers,
        active_offers: counts.active_offers,
        newsletters: counts.newsletters,
        signup_access: state
            .settings
            .get_str("members_signup_access")
            .unwrap_or_else(|| "all".into()),
        portal_plans,
        // Both ways of being connected, as the dashboard's checklist has it:
        // the OAuth flow leaves an account id, a pasted key leaves a secret.
        stripe_connected: has("stripe_connect_account_id") || has("stripe_secret_key"),
        comments_enabled: state.settings.get_str("comments_enabled").unwrap_or_else(|| "off".into()),
        donations_on,
        default_visibility: state
            .settings
            .get_str("default_content_visibility")
            .unwrap_or_else(|| "public".into()),
    }
}

/// Count the funnel and assemble the [`Funnel`] over a window of `days`.
///
/// Separate from the page so the admin home can ask this module what it thinks
/// rather than guessing at the same question with a different query — the home
/// screen reports *how many* blocking gaps there are and links here for the
/// list, and `mrr_cents` is already shared with it in the other direction.
///
/// Two round trips, not one: see [`repo::mrr_cents`] on why the `SUM` over a
/// `REAL` column stays out of the counts query.
pub(crate) async fn measure(state: &AppState, days: i64) -> ghost_core::Result<Funnel> {
    let now = chrono::Utc::now();
    let since = ghost_core::dates::to_db_string(&(now - chrono::Duration::days(days)));
    let before = ghost_core::dates::to_db_string(&(now - chrono::Duration::days(days * 2)));
    let counts = repo::funnel(&state.db, &since, &before).await?;
    let mrr = repo::mrr_cents(&state.db).await;
    Ok(funnel_of(state, &counts, mrr))
}

pub async fn memberships_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let days = rules::window_days(q.get("days").map(|s| s.as_str()));
    // The same window `measure` counts over, for the charts below it.
    let since = ghost_core::dates::to_db_string(
        &(chrono::Utc::now() - chrono::Duration::days(days)),
    );
    let funnel = match measure(&state, days).await {
        Ok(f) => f,
        Err(e) => return server_error(&e),
    };
    let mrr = funnel.mrr_cents;
    let gaps = rules::audit(&funnel);

    let currency = Some(funnel.currency.as_str()).filter(|c| !c.is_empty());
    let blocking = gaps.iter().filter(|g| g.severity == Severity::Blocking).count();

    // --- the hero sentence ---
    let conversion = rules::conversion_percent(funnel.supported(), funnel.members);
    let summary = if funnel.members == 0 {
        "Nobody has joined yet. Everything below is ready for the first signup — the list of \
         gaps says what a visitor would meet on the way in."
            .to_string()
    } else {
        format!(
            "{members}, {supported} of them paying or comped — {rate} of the list. \
             {money} a month in recurring revenue, {year} a year at this rate.",
            members = countable(funnel.members, "member"),
            supported = funnel.supported(),
            rate = percent(conversion),
            money = money(mrr, currency),
            year = money(rules::annual_run_rate_cents(mrr), currency),
        )
    };
    let verdict = if blocking > 0 {
        format!(
            "{} stopping somebody joining or paying right now.",
            countable(blocking as i64, "gap"),
        )
    } else if gaps.is_empty() {
        "Nothing is in the way of a signup. Every check this screen makes passes.".to_string()
    } else {
        format!(
            "Nothing is blocking a signup. {} worth looking at.",
            countable(gaps.len() as i64, "suggestion"),
        )
    };

    // --- the tiles ---
    let tiles = format!(
        r#"<div class="mb-tiles rise" style="--i:2">{members}{paying}{conv}{mrr}{joined}{churn}</div>"#,
        members = tile(
            "Members",
            &crate::compact(funnel.members),
            &format!("{} free · {} comped", funnel.free, funnel.comped),
            Some("/ghost/members/"),
            "group",
            "blue",
        ),
        paying = tile(
            "Paying",
            &crate::compact(funnel.paid),
            &match funnel.comped {
                0 => "On a paid subscription".to_string(),
                n => format!("Plus {} on a complimentary tier", n),
            },
            Some("/ghost/members/?filter=paid"),
            "star",
            "amber",
        ),
        conv = tile(
            "Conversion",
            &percent(conversion),
            "Members who pay or are comped",
            Some("/ghost/memberships/tiers/"),
            "trending",
            "green",
        ),
        mrr = tile(
            "Monthly revenue",
            &money(mrr, currency),
            &match rules::arpu_cents(mrr, funnel.paid) {
                Some(each) => format!("{} per paying member", money(each, currency)),
                None => "No paid subscriptions yet".to_string(),
            },
            Some("/ghost/settings/memberships/"),
            "chart",
            "purple",
        ),
        joined = tile(
            &format!("Joined in {days} days"),
            &crate::compact(funnel.joined),
            &trend(funnel.joined, funnel.joined_before, days),
            None,
            "add",
            "teal",
        ),
        churn = tile(
            "Cancellations",
            &crate::compact(funnel.cancelled),
            &format!("Against {} in the same window", countable(funnel.upgraded, "upgrade")),
            None,
            "logout",
            if funnel.cancelled > funnel.upgraded { "red" } else { "slate" },
        ),
    );

    // --- the chart ---
    let rows = repo::signups_by_day(&state.db, &since).await;
    let chart = format!(
        r#"<div class="section">Growth{bar}</div>
<div class="card padded rise" style="--i:3">{chart}</div>"#,
        bar = window_bar("/ghost/memberships/", days),
        chart = growth_chart(&fill_days(&rows, days)),
    );

    // --- the gaps ---
    let gap_card = if gaps.is_empty() {
        format!(
            r#"<div class="section">What is in the way</div>
<div class="card padded rise" style="--i:4"><p style="margin:0">{check} Nothing. Signups are open, there is something to buy, somebody can pay for it, and they will hear from you afterwards.</p></div>"#,
            check = svg("check"),
        )
    } else {
        format!(
            r#"<div class="section">What is in the way</div>
<div class="card rows rise" style="--i:4">{rows}</div>"#,
            rows = gaps.iter().map(gap_row).collect::<String>(),
        )
    };

    // --- where they came from, and who arrived ---
    let sources = sources_card(&state, &since, funnel.joined).await;
    let arrivals = arrivals_card(&state).await;
    let newsletters = newsletters_card(&state).await;

    page_for_role(
        actor.role,
        "Memberships",
        "memberships",
        Some(&actor.user),
        &format!(
            r#"{head}
{banner}
<div class="card padded rise mb-hero" style="--i:1">
<div class="mb-hero-body"><h2>{verdict}</h2><p class="muted">{summary}</p></div>
</div>
{tiles}
{chart}
{gaps}
<div class="mb-two">{sources}{arrivals}</div>
{newsletters}"#,
            head = head("overview"),
            banner = flash_banner(&q),
            verdict = esc(&verdict),
            summary = esc(&summary),
            tiles = tiles,
            chart = chart,
            gaps = gap_card,
            sources = sources,
            arrivals = arrivals,
            newsletters = newsletters,
        ),
        true,
    )
    .into_response()
}

/// Where the window's signups came from.
///
/// `joined` is the window's signup count, used only to say out loud when the
/// sources add up to less than it: a member with no `members_created_events`
/// row — an old import, a hand-inserted row — is genuinely unattributable,
/// and silently dropping them would make the card look like it had lost
/// people.
async fn sources_card(state: &AppState, since: &str, joined: i64) -> String {
    let rows = repo::sources(&state.db, since).await;
    let accounted: i64 = rows.iter().map(|(_, n)| *n).sum();
    let body = if rows.is_empty() {
        r#"<div class="row"><div class="body"><span class="title">Nothing yet</span>
<span class="detail">No signups in this window to attribute.</span></div></div>"#
            .to_string()
    } else {
        rows.iter()
            .map(|(source, n)| {
                let (label, detail, icon, tint) = match source.as_str() {
                    "member" => ("The site", "Signed up through the portal", "globe", "green"),
                    "admin" => ("Added by staff", "Created from the members list", "person", "blue"),
                    "api" => ("The Admin API", "Created by an integration", "integrations", "purple"),
                    "import" => ("Imported", "Arrived in a CSV", "import", "amber"),
                    "system" => ("The system", "Created by the site itself", "settings", "slate"),
                    _ => ("Other", "An unrecognised source", "notes", "slate"),
                };
                format!(
                    r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{label}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip tonal">{n}</span></div></div>"#,
                    tint = tint,
                    icon = svg(icon),
                    label = esc(label),
                    // `source` itself is never rendered: it is a column an
                    // integration can write, so it is matched against a
                    // known set and the label comes from this file.
                    detail = esc(detail),
                    n = n,
                )
            })
            .collect::<String>()
    };
    let note = if joined > accounted {
        format!(
            r#"<p class="muted" style="margin:10px 0 0 16px;font-size:12.5px">{} of the window's {} have no recorded source — an import or a hand-made row.</p>"#,
            joined - accounted,
            countable(joined, "signup"),
        )
    } else {
        String::new()
    };
    format!(
        r#"<div><div class="section">Where they came from</div>
<div class="card rows rise" style="--i:5">{body}</div>{note}</div>"#,
    )
}

/// The latest arrivals.
///
/// Names and email addresses, which is why this screen is Administrator-only.
/// Both are escaped: a reader chose them, which makes them the most hostile
/// input on the page.
async fn arrivals_card(state: &AppState) -> String {
    let arrivals = repo::recent_signups(&state.db, 6).await.unwrap_or_default();
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let body = if arrivals.is_empty() {
        r#"<div class="row"><div class="body"><span class="title">No members yet</span>
<span class="detail">The first signup appears here.</span></div></div>"#
            .to_string()
    } else {
        arrivals
            .iter()
            .map(|a| {
                let shown = a.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&a.email);
                let initial = shown.chars().next().unwrap_or('?').to_uppercase().to_string();
                let (chip, status) = match a.status.as_str() {
                    "paid" => ("published", "Paid"),
                    "comped" => ("tonal", "Comped"),
                    _ => ("draft", "Free"),
                };
                let when = ghost_core::dates::short_date_in_zone(&a.created_at, &timezone);
                // Rule 8: never build a link out of an id without checking
                // its shape first. A row with an unexpected id is shown
                // without a link rather than hidden.
                let row = format!(
                    r#"<span class="avatar">{initial}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{email}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span><span class="muted" style="margin-left:10px">{when}</span></div>"#,
                    initial = esc(&initial),
                    name = esc(shown),
                    email = esc(&a.email),
                    chip = chip,
                    status = status,
                    when = esc(&when),
                );
                if ghost_core::ids::is_objectid(&a.id) {
                    format!(r#"<a class="row" href="/ghost/members/{id}/">{row}</a>"#, id = esc(&a.id))
                } else {
                    format!(r#"<div class="row">{row}</div>"#)
                }
            })
            .collect::<String>()
    };
    format!(
        r#"<div><div class="section">Latest arrivals</div>
<div class="card rows rise" style="--i:5">{body}</div>
<p class="muted" style="margin:10px 0 0 16px;font-size:12.5px"><a href="/ghost/members/">Every member &rarr;</a></p></div>"#,
    )
}

/// Every newsletter with its reach.
///
/// Here rather than only under Settings → Newsletters because reach is a
/// membership figure: a thousand members and one newsletter nobody is
/// subscribed to is a specific and common failure, and neither screen on its
/// own shows it.
async fn newsletters_card(state: &AppState) -> String {
    let stats = repo::newsletter_stats(&state.db).await.unwrap_or_default();
    if stats.is_empty() {
        return format!(
            r#"<div class="section">Newsletters</div>
<div class="card padded rise" style="--i:6"><p style="margin:0 0 12px">There is no newsletter, so a new member hears nothing after the welcome email.</p>
<a class="btn tonal" href="/ghost/settings/newsletters/">{send} Set one up</a></div>"#,
            send = svg("send"),
        );
    }
    let rows: String = stats
        .iter()
        .map(|n| {
            let sending = n.status == "active";
            let detail = format!(
                "{} · {}",
                if sending { "Sending" } else { "Archived" },
                if n.visibility == "paid" { "paid members only" } else { "all members" },
            );
            let inner = format!(
                r#"<span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip tonal">{subs}</span></div>"#,
                tint = if sending { "purple" } else { "slate" },
                icon = svg("send"),
                name = esc(&n.name),
                detail = esc(&detail),
                subs = esc(&countable(n.subscribers, "subscriber")),
            );
            if ghost_core::ids::is_objectid(&n.id) {
                format!(
                    r#"<a class="row" href="/ghost/settings/newsletters/{id}/">{inner}</a>"#,
                    id = esc(&n.id),
                )
            } else {
                format!(r#"<div class="row">{inner}</div>"#)
            }
        })
        .collect();
    format!(
        r#"<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:6">{rows}</div>"#,
    )
}

mod offers;
mod tiers;

// Re-exported so the router keeps naming every handler
// `memberships::<handler>`, which is what rule 5 asks for and what makes the
// route block readable.
pub(crate) use offers::*;
pub(crate) use tiers::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: &str, free: i64, paying: i64) -> Day {
        Day { day: d.into(), free, paying }
    }

    #[test]
    fn a_member_chose_their_own_name_so_the_overview_escapes_it() {
        // The one genuinely hostile input on the screen. `esc` leaves the
        // text visible and inert, so the assertion is that the tag never
        // opens — not that the words are gone.
        let hostile = r#"<img src=x onerror=alert(1)>"#;
        let escaped = esc(hostile);
        assert!(!escaped.contains("<img src=x"));
        assert!(escaped.contains("&lt;img"));
    }

    #[test]
    fn a_rate_with_no_denominator_reads_as_a_dash_rather_than_zero() {
        assert_eq!(percent(None), "—");
        assert_eq!(percent(Some(12.34)), "12.3%");
        assert_eq!(percent(Some(0.0)), "0.0%");
    }

    #[test]
    fn the_trend_line_says_what_it_can_and_no_more() {
        // A rise from nothing is not a percentage, and inventing "+100%" for
        // it is the kind of wrong number somebody repeats in a meeting.
        assert!(trend(10, 0, 30).contains("none at all"));
        assert!(trend(0, 0, 30).contains("nothing in the previous"));
        assert!(trend(150, 100, 30).contains("+50%"));
        assert!(trend(50, 100, 30).contains("-50%"));
    }

    #[test]
    fn the_chart_fills_the_window_so_five_bars_do_not_look_like_thirty() {
        let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let rows = vec![(today.clone(), "free".to_string(), 3), (today.clone(), "paid".to_string(), 1)];
        let days = fill_days(&rows, 7);
        assert_eq!(days.len(), 7);
        // Ascending, ending today.
        assert_eq!(days[6].day, today);
        assert_eq!((days[6].free, days[6].paying), (3, 1));
        assert!(days[..6].iter().all(|d| d.free == 0 && d.paying == 0));
    }

    #[test]
    fn a_comped_member_is_counted_as_paying_on_the_chart() {
        // Otherwise the chart loses people the tiles above it still count,
        // and the two disagree on one screen.
        let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let rows = vec![
            (today.clone(), "free".to_string(), 1),
            (today.clone(), "comped".to_string(), 2),
            // An invented status, as an import can leave behind.
            (today.clone(), "legacy".to_string(), 1),
        ];
        let days = fill_days(&rows, 1);
        assert_eq!((days[0].free, days[0].paying), (1, 3));
    }

    #[test]
    fn an_empty_window_says_so_instead_of_drawing_an_empty_grid() {
        let out = growth_chart(&[day("2026-11-01", 0, 0), day("2026-11-02", 0, 0)]);
        assert!(out.contains("Nobody joined in this window"));
        assert!(!out.contains("<svg"));
    }

    #[test]
    fn the_chart_stacks_free_under_paying_and_labels_every_day() {
        let out = growth_chart(&[day("2026-11-01", 4, 2), day("2026-11-02", 0, 1)]);
        assert!(out.contains("<svg"));
        // One group per day that had anybody, each with a tooltip naming the
        // split — the chart's only accessible description of a single bar.
        assert_eq!(out.matches("<title>").count(), 2);
        assert!(out.contains("6 joined — 4 free, 2 paying"));
        // Free is drawn at reduced opacity, paying at full: one hue, two
        // weights, rather than a second colour nothing else on the page uses.
        assert!(out.contains(r#"opacity=".38""#));
        assert!(out.contains("mb-legend"));
    }

    #[test]
    fn the_window_bar_offers_only_windows_the_reducer_accepts() {
        // The two lists have to be the same list, or a tab leads to a page
        // that silently shows a different window than the one it names.
        for (key, days, _) in rules::WINDOWS {
            let bar = window_bar("/ghost/memberships/", *days);
            assert!(bar.contains(&format!(r#"href="/ghost/memberships/?days={key}""#)), "{bar}");
            assert_eq!(rules::window_days(Some(key)), *days);
        }
        // Exactly one tab is marked current.
        assert_eq!(window_bar("/ghost/memberships/", 30).matches("seg on").count(), 1);
    }

    #[test]
    fn a_forged_window_marks_the_default_tab_rather_than_none() {
        let days = rules::window_days(Some("4000"));
        assert_eq!(window_bar("/ghost/memberships/", days).matches("seg on").count(), 1);
    }

    #[test]
    fn the_tab_bar_marks_one_tab_and_sends_members_to_its_long_standing_url() {
        for active in ["overview", "members", "tiers", "offers"] {
            let bar = tabs(active);
            assert_eq!(bar.matches("seg on").count(), 1, "{active}");
        }
        assert!(tabs("overview").contains(r#"href="/ghost/members/""#));
    }

    #[test]
    fn a_gap_row_carries_its_code_as_an_anchor_and_its_fix_as_a_link() {
        let gap = Gap {
            code: "stripe-missing",
            severity: Severity::Blocking,
            title: "Paid tiers exist but Stripe is not connected",
            detail: "2 paid tiers on offer.".into(),
            fix: Some(("Connect Stripe", "/ghost/settings/memberships/#stripe")),
        };
        let row = gap_row(&gap);
        assert!(row.contains(r#"id="gap-stripe-missing""#));
        assert!(row.contains(r#"href="/ghost/settings/memberships/#stripe""#));
        assert!(row.contains("chip error"));
        assert!(row.contains("Blocking"));
    }

    #[test]
    fn a_gap_with_nothing_to_fix_draws_no_button() {
        let gap = Gap {
            code: "no-members",
            severity: Severity::Note,
            title: "No members yet",
            detail: "Nobody to count.".into(),
            fix: None,
        };
        let row = gap_row(&gap);
        assert!(!row.contains("<a"), "{row}");
        assert!(row.contains("chip tonal"));
    }

    #[test]
    fn a_tile_without_a_destination_is_not_drawn_as_a_link() {
        let linked = tile("Members", "12", "detail", Some("/ghost/members/"), "group", "blue");
        assert!(linked.starts_with("<a class=\"card padded mb-tile\""));
        let plain = tile("Cancellations", "0", "detail", None, "logout", "slate");
        assert!(plain.starts_with("<div class=\"card padded mb-tile\""));
        assert!(!plain.contains("href"));
    }
}
