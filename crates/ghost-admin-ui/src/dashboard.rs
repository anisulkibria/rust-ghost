//! The admin home screen at `/ghost/dashboard/`.
//!
//! This is the first thing a signed-in staff member sees, and it answers one
//! question: **what should I do next?** Everything on it is either work
//! waiting, something broken, or a way to start. It is deliberately not a
//! second Analytics screen — `/ghost/analytics/` owns the reporting, and the
//! figures here are a glance with a link to it.
//!
//! The order down the page is the order the questions matter in:
//!
//! 1. the greeting, and the two or three things you came here to start;
//! 2. what is still undone about setting the site up, while it is undone;
//! 3. the queues waiting on a person, faults before chores;
//! 4. the drafts and scheduled posts to pick up;
//! 5. the headline figures, with a link to the full report;
//! 6. what went out lately, and how it has been read.
//!
//! ## Who sees what
//!
//! Every signed-in role may open this page — it is where `/ghost/` lands, so
//! refusing it below Administrator would mean a Contributor signs in to a
//! "Not allowed" screen. It is the *sections* that are gated, by the same
//! capabilities the handlers they link to enforce: a card whose only
//! destination is Administrator-only is not rendered for anybody else, because
//! a link that leads to a refusal is worse than no link.
//!
//! Sections 4 and 6 are narrowed to the viewer's own entries for an Author or
//! a Contributor, exactly as the posts list is (`Role::can_edit_any_post`).
//! A writer's home screen shows a writer's work.
//!
//! ## The security contract
//!
//! Read-only: there is no POST route in this module and nothing on the page
//! changes state, so there is no origin check to make — [`guard`] is the role
//! check alone, and every handler's first statement is a call to it.
//!
//! Nothing here is interpolated unescaped. Post titles, member names and the
//! site's own name all pass through `esc`, and ids are checked with
//! `ghost_core::ids::is_objectid` before they are used to build an editor
//! link. The rankings and thresholds are
//! [`ghost_core::dashboard`]'s, unit-tested there without a database; the
//! counts are [`ghost_db::repos::dashboard`]'s. This file turns those into
//! rows and says nothing about either.

use super::*;

use ghost_core::dashboard::{Pending, Setup};
use ghost_db::repos::dashboard as repo;

/// The window the headline figures and the sparkline cover.
///
/// Seven days rather than thirty: the home screen is somewhere you look most
/// days, and a 30-day window barely moves between two of them. The full range
/// picker lives on Analytics, which this page links to.
const WINDOW_DAYS: i64 = 7;

/// How many entries the work queue and the recently-published card each list.
///
/// Five is about what fits above a scroll on a laptop. The point of both cards
/// is to be read at a glance and then left — anything longer belongs on the
/// posts list, which both cards link to.
const LIST_LIMIT: i64 = 5;

/// The role check. Any signed-in staff member may see this page; see the
/// module docs on why, and on how the sections are gated instead.
///
/// No `writes` parameter, unlike the other modules' guards: this module has no
/// route that changes anything, so there is no state-changing request for an
/// origin check to protect. Adding one means adding the check with it.
async fn guard(state: &AppState, headers: &HeaderMap) -> Result<Actor, Response> {
    require_actor(state, headers).await
}

// ---------------------------------------------------------------------------
// the page
// ---------------------------------------------------------------------------

pub async fn dashboard_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match guard(&state, &headers).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let role = actor.role;
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let now = chrono::Utc::now();

    // An Author or Contributor sees their own entries, the same narrowing the
    // posts list applies. `None` means every entry.
    let mine = (!role.can_edit_any_post()).then_some(actor.user.id.as_str());

    let (window, previous) = analytics::Window::pair(now, Some(WINDOW_DAYS));
    let since_30 = (now - chrono::Duration::days(30)).format("%Y-%m-%d %H:%M:%S").to_string();
    let stamp_now = now.format("%Y-%m-%d %H:%M:%S").to_string();

    // The counts are one read, and a failure costs the figures rather than the
    // page: a home screen that renders the work queue and apologises for the
    // numbers is far more useful than an error page.
    let snapshot = match repo::snapshot(&state.db, &stamp_now, &since_30).await {
        Ok(snapshot) => snapshot,
        Err(e) => {
            tracing::error!("dashboard snapshot failed: {e}");
            repo::Snapshot::default()
        }
    };

    let queue = repo::work_queue(&state.db, mine, LIST_LIMIT).await.unwrap_or_default();
    let recent = repo::recent_published(&state.db, mine, LIST_LIMIT).await.unwrap_or_default();

    let head = header(&actor, &timezone, now);
    let checklist = if role.is_admin() { setup_card(&state, &snapshot).await } else { String::new() };
    let attention = if role.is_admin() { pending_card(&snapshot) } else { String::new() };
    let work = work_card(&queue, &timezone, role);
    let figures = if role.is_admin() {
        figures_section(&state, &snapshot, &window, &previous).await
    } else {
        String::new()
    };
    let published = published_card(&state, &recent, &timezone, &since_30, role).await;

    // `--i` staggers the rise animation down the page; the numbers only have
    // to increase, and a section that is not rendered simply leaves a gap in
    // the sequence rather than shifting everything after it.
    page_for_role(role, "Dashboard", "dashboard", Some(&actor.user), &format!(
        "{head}{checklist}{attention}{work}{figures}{published}"
    ), true)
    .into_response()
}

// ---------------------------------------------------------------------------
// the header: who, when, and what to start
// ---------------------------------------------------------------------------

/// Greeting, today's date, and the actions worth one click.
///
/// The actions are the ones every role may actually take. Media is
/// Administrator-only, so it appears only there; "View site" is a link to the
/// public site and is for everybody.
fn header(actor: &Actor, timezone: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let role = actor.role;
    let hour = ghost_core::dashboard::hour_in_zone(now, timezone);
    // The first word of the name, not the whole thing: "Good morning, Ada"
    // reads like a person talking; "Good morning, Ada Lovelace-Byron" reads
    // like a letter from a bank.
    let first = actor
        .user
        .name
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    let greeting = match first.is_empty() {
        true => ghost_core::dashboard::greeting(hour).to_string(),
        false => format!("{}, {}", ghost_core::dashboard::greeting(hour), esc(&first)),
    };
    let today = ghost_core::dashboard::today_in_zone(now, timezone);

    let mut actions = String::new();
    if role.is_admin() {
        actions.push_str(&format!(
            r#"<a class="btn text" href="/ghost/media/">{ic} Media</a>"#,
            ic = svg("library"),
        ));
    }
    actions.push_str(&format!(
        r#"<a class="btn text" href="/" target="_blank" rel="noopener">{ic} View site</a>
<a class="btn tonal" href="/ghost/pages/new/">New page</a>
<a class="btn" href="/ghost/posts/new/">{add} New post</a>"#,
        ic = svg("open"),
        add = svg("add"),
    ));

    format!(
        r#"<header class="page-head rise"><div><h1>{greeting}</h1>
<p class="sub">{today} · {site}</p></div>
<div class="actions">{actions}</div></header>"#,
        greeting = greeting,
        today = esc(&today),
        site = esc(&admin_site_name()),
        actions = actions,
    )
}

// ---------------------------------------------------------------------------
// section 2: what is still undone about the site
// ---------------------------------------------------------------------------

/// The setup checklist, or nothing at all once there is nothing left on it.
///
/// Deliberately disappears rather than showing ticks: a completed checklist is
/// a trophy and this is a work surface.
///
/// Administrator-only because four of its five steps are settings screens that
/// nobody below Administrator may open, and because the fifth ("publish
/// something") is a judgement about the site as a whole rather than about the
/// viewer's own work.
async fn setup_card(state: &AppState, snapshot: &repo::Snapshot) -> String {
    let has = |key: &str| {
        state
            .settings
            .get_str(key)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    };
    let signup = state.settings.get_str("members_signup_access").unwrap_or_else(|| "all".into());
    let site = ghost_core::dashboard::Site {
        has_published: snapshot.published > 0,
        has_description: has("description"),
        has_icon: has("icon"),
        // Both halves: a site with paid signups switched off, or with no
        // visible paid tier to buy, is not missing Stripe. `paid_tiers`
        // counts only tiers a visitor can reach — see its doc comment for
        // why the seeded one does not count.
        sells_memberships: signup != "none" && snapshot.paid_tiers > 0,
        stripe_connected: has("stripe_connect_account_id") || has("stripe_secret_key"),
        analytics_on: state.settings.get_bool("web_analytics"),
    };

    let steps = ghost_core::dashboard::setup(&site);
    if steps.is_empty() {
        return String::new();
    }

    let rows: String = steps.iter().map(|step| setup_row(*step)).collect();
    format!(
        r#"<div class="section">Finish setting up</div>
<div class="card rows dash-rows rise" style="--i:1">{rows}</div>"#,
    )
}

/// One checklist row: what is missing, why it matters, and where to fix it.
fn setup_row(step: Setup) -> String {
    let (icon_name, tint, title, detail, href, cta) = match step {
        Setup::FirstPost => (
            "posts",
            "blue",
            "Publish your first post",
            "The site is live but there is nothing on it yet.",
            "/ghost/posts/new/",
            "Write one",
        ),
        Setup::Describe => (
            "excerpt",
            "amber",
            "Say what this site is about",
            "The description is what search results and most themes show under the title.",
            "/ghost/settings/general/",
            "Add one",
        ),
        Setup::Icon => (
            "image",
            "purple",
            "Add a site icon",
            "Browser tabs and bookmarks have nothing to show without one.",
            "/ghost/settings/general/",
            "Upload",
        ),
        Setup::ConnectStripe => (
            "lock",
            "green",
            "Connect Stripe",
            "Paid tiers are on offer, but nobody can pay for one yet.",
            "/ghost/settings/membership/",
            "Connect",
        ),
        Setup::TurnOnAnalytics => (
            "chart",
            "teal",
            "Turn on web analytics",
            "No visits are being recorded, so every figure here stays at zero.",
            "/ghost/settings/analytics/",
            "Switch on",
        ),
    };
    format!(
        r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><a class="btn text" href="{href}">{cta}</a></div></div>"#,
        tint = tint,
        ic = svg(icon_name),
        title = esc(title),
        detail = esc(detail),
        href = href,
        cta = esc(cta),
    )
}

// ---------------------------------------------------------------------------
// section 3: the queues waiting on a person
// ---------------------------------------------------------------------------

/// "Needs you" — the non-empty queues, faults first, or nothing.
///
/// Administrator-only. Two of the four destinations are themselves
/// Administrator-only (comment moderation, form responses), and all four
/// counts are facts about the whole site: how many readers reported a comment,
/// whether a newsletter reached anybody. A writer has no business with those
/// even where the screen behind the link would let them in.
fn pending_card(snapshot: &repo::Snapshot) -> String {
    let queues = ghost_core::dashboard::Queues {
        reported_comments: snapshot.reported_comments,
        new_responses: snapshot.new_responses,
        failed_emails: snapshot.failed_emails,
        overdue_scheduled: snapshot.overdue_scheduled,
    };
    let items = ghost_core::dashboard::pending(&queues);
    if items.is_empty() {
        return String::new();
    }
    let rows: String = items.iter().map(|item| pending_row(*item)).collect();
    format!(
        r#"<div class="section">Needs you</div>
<div class="card rows dash-rows rise" style="--i:2">{rows}</div>"#,
    )
}

/// One queue as a row. A fault is tinted red and chipped `error`; a chore is
/// tinted and chipped tonally. The words say which it is either way — the
/// colour only agrees with them.
fn pending_row(item: Pending) -> String {
    let count = item.count();
    let (icon_name, tint, title, detail, href) = match item {
        Pending::OverdueScheduled(n) => (
            "warning",
            "red",
            countable(n, "scheduled post"),
            "Past the time it should have gone out. The publishing scheduler may have stopped.".to_string(),
            "/ghost/posts/?filter=scheduled",
        ),
        Pending::FailedEmails(n) => (
            "mail",
            "red",
            format!("{} that failed to send", countable(n, "newsletter")),
            "The post published, but the email did not reach its recipients.".to_string(),
            "/ghost/posts/?filter=sent",
        ),
        Pending::ReportedComments(n) => (
            "flag",
            "amber",
            countable(n, "reported comment"),
            "A reader flagged these for moderation.".to_string(),
            "/ghost/comments/?filter=reported",
        ),
        Pending::NewResponses(n) => (
            "form",
            "blue",
            countable(n, "new form response"),
            "Somebody wrote in through a form and nobody has read it.".to_string(),
            "/ghost/forms/responses/?status=new",
        ),
    };
    format!(
        r#"<a class="row" href="{href}"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip {chip}">{count}</span>{chev}</div></a>"#,
        href = href,
        tint = tint,
        ic = svg(icon_name),
        title = esc(&title),
        detail = esc(&detail),
        chip = if item.is_fault() { "error" } else { "tonal" },
        count = count,
        chev = svg("chevron"),
    )
}

// ---------------------------------------------------------------------------
// section 4: the work in progress
// ---------------------------------------------------------------------------

/// "Pick up where you left off" — scheduled entries then drafts.
///
/// Always rendered, for every role: on a quiet site the empty state is the
/// most useful thing on the page, because it is the one that offers to start
/// something.
fn work_card(entries: &[repo::Entry], timezone: &str, role: Role) -> String {
    let rows: String = entries.iter().map(|entry| work_row(entry, timezone)).collect();
    let body = if rows.is_empty() {
        let what = if role.can_edit_any_post() { "No drafts" } else { "You have no drafts" };
        format!(
            r#"<div class="row"><span class="ic tint-green">{ic}</span>
<div class="body"><span class="title">All caught up</span>
<span class="detail">{what} or scheduled posts waiting.</span></div>
<div class="trailing"><a class="btn text" href="/ghost/posts/new/">Start a post</a></div></div>"#,
            ic = svg("check"),
            what = esc(what),
        )
    } else {
        rows
    };
    format!(
        r#"<div class="section dash-head">Pick up where you left off<a class="dash-more" href="/ghost/posts/?filter=draft">All drafts {chev}</a></div>
<div class="card rows dash-rows rise" style="--i:3">{body}</div>"#,
        chev = svg("chevron"),
    )
}

/// One draft or scheduled entry, linking into the editor.
///
/// The detail line is the thing a writer actually wants: when a draft was last
/// touched, or when a scheduled post is due. Both are shown in the site's
/// timezone, because a publish time in UTC is a time the reader has to convert
/// in their head.
fn work_row(entry: &repo::Entry, timezone: &str) -> String {
    let title = entry.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let title = title.unwrap_or("(Untitled)");
    let is_page = entry.kind == "page";
    let scheduled = entry.status == "scheduled";

    let detail = if scheduled {
        match entry.published_at.as_deref() {
            Some(at) => format!(
                "Goes out {} at {}",
                esc(&ghost_core::dates::short_date_in_zone(at, timezone)),
                esc(&ghost_core::dates::clock_in_zone(at, timezone)),
            ),
            None => "Scheduled with no time set".to_string(),
        }
    } else {
        match entry.updated_at.as_deref() {
            Some(at) => format!("Edited {}", esc(&when(at))),
            None => "Not edited yet".to_string(),
        }
    };

    // An id that is not an id never becomes a link. The editor would refuse it
    // anyway, but a link is markup and this is the cheap place to stop it.
    let body = if ghost_core::ids::is_objectid(&entry.id) {
        format!(
            r#"<a class="body" href="{base}{id}/"><span class="title">{title}</span><span class="detail">{detail}</span></a>"#,
            base = if is_page { "/ghost/pages/" } else { "/ghost/posts/" },
            id = esc(&entry.id),
            title = esc(title),
            detail = detail,
        )
    } else {
        format!(
            r#"<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>"#,
            title = esc(title),
            detail = detail,
        )
    };

    format!(
        r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>{body}
<div class="trailing"><span class="chip {chip}">{label}</span></div></div>"#,
        tint = if scheduled { "blue" } else { "amber" },
        ic = svg(if scheduled { "calendar" } else if is_page { "pages" } else { "posts" }),
        body = body,
        chip = if scheduled { "scheduled" } else { "draft" },
        label = if scheduled { "Scheduled" } else { "Draft" },
    )
}

// ---------------------------------------------------------------------------
// section 5: the headline figures
// ---------------------------------------------------------------------------

/// The four tiles and the sparkline, with a link to the full report.
///
/// On a site connected to Vuuut Click this shows a pointer to Click's report
/// and no local figures at all. That is the same rule the Analytics screen
/// keeps: a connected site has **one** reporting source, and quietly
/// substituting local `page_views` counts here would put two different numbers
/// for the same week on two screens of the same admin.
async fn figures_section(
    state: &AppState,
    snapshot: &repo::Snapshot,
    window: &analytics::Window,
    previous: &Option<analytics::Window>,
) -> String {
    let members_tile = analytics::tile(
        "Members",
        &compact(snapshot.members),
        analytics::Change { now: snapshot.members, before: Some(snapshot.members_before) },
        "previous 30 days",
    );
    let mrr = repo::mrr_minor(&state.db).await;
    // MRR and the content count have no previous window to compare against —
    // one is a snapshot of what is live and the other a running total — so
    // both are written out rather than passed through `tile`, which would
    // print "nothing to compare" under a figure where a comparison is not a
    // missing fact but a meaningless one.
    let mrr_tile = format!(
        r#"<div class="tile"><div class="label">MRR</div><div class="value">{mrr}</div>
<div class="delta">from {paid}</div></div>"#,
        mrr = esc(&money_minor(mrr)),
        paid = esc(&countable(snapshot.paid_members, "paid member")),
    );
    let content_tile = format!(
        r#"<div class="tile"><div class="label">Published</div><div class="value">{posts}</div>
<div class="delta">{pages} · {drafts}</div></div>"#,
        posts = esc(&compact(snapshot.published)),
        pages = esc(&countable(snapshot.pages, "page")),
        drafts = esc(&countable(snapshot.drafts, "draft")),
    );
    // The absolute figure behind the Members tile's percentage, and the way to
    // the list. `.live-note` is the Analytics overview's line under its own
    // tiles — the same kind of statement in the same place, so it is reused
    // rather than restyled.
    let members_note = format!(
        r#"<p class="live-note rise" style="--i:4">{joined} in the last 30 days · <a href="/ghost/members/">Manage members</a></p>"#,
        // "new member" rather than "member joined": `countable` pluralises by
        // adding an `s` to what it is given, which would make "3 member joineds".
        joined = esc(&countable(snapshot.new_members, "new member")),
    );

    match ghost_api::click::connection(state).await {
        // Connected: Click is the authority, and it is the only thing offered.
        Ok(Some(_)) => format!(
            r#"<div class="section">Your publication</div>
<div class="tiles rise" style="--i:4">{members_tile}{mrr_tile}{content_tile}</div>
{members_note}
<div class="card padded rise" style="--i:5"><p class="muted" style="margin:0">
Web traffic for this site is reported by Vuuut Click.
<a href="/ghost/analytics/">Open the report</a></p></div>"#,
        ),
        // Not connected, or Click could not be reached. Either way the local
        // `page_views` table is this site's own record and is what is shown;
        // an unreachable connector is reported on the Analytics screen, which
        // owns that conversation.
        _ => {
            let visitors = analytics::traffic(&state.db, window, "all").await.0;
            let before = match previous {
                Some(w) => Some(analytics::traffic(&state.db, w, "all").await.0),
                None => None,
            };
            let visitors_tile = analytics::tile(
                "Unique visitors",
                &compact(visitors),
                analytics::Change { now: visitors, before },
                &format!("previous {WINDOW_DAYS} days"),
            );
            let chart = sparkline(&state.db, &window.from).await;
            format!(
                r#"<div class="section dash-head">Your publication<a class="dash-more" href="/ghost/analytics/">Full report {chev}</a></div>
<div class="tiles rise" style="--i:4">{visitors_tile}{members_tile}{mrr_tile}{content_tile}</div>
{members_note}
<div class="card padded rise" style="--i:5">{chart}</div>"#,
                chev = svg("chevron"),
            )
        }
    }
}

/// Unique visitors per day across the window, as the shared column chart.
///
/// Gaps are filled with zeroes here rather than in the query, because only
/// this side knows how many days the chart is meant to be wide: a day with no
/// visits is a column of height zero, not a missing column that would squeeze
/// the rest.
async fn sparkline(db: &ghost_db::Db, from: &str) -> String {
    let counted: std::collections::HashMap<String, i64> =
        repo::daily_visitors(db, from).await.into_iter().collect();
    let days: Vec<DayViews> = (0..WINDOW_DAYS)
        .rev()
        .map(|back| {
            let day = (chrono::Utc::now() - chrono::Duration::days(back))
                .format("%Y-%m-%d")
                .to_string();
            DayViews { views: counted.get(&day).copied().unwrap_or(0), day }
        })
        .collect();
    format!(
        r#"<p class="dash-chart-title">Unique visitors per day</p>{}"#,
        views_chart(&days)
    )
}

// ---------------------------------------------------------------------------
// section 6: what went out lately
// ---------------------------------------------------------------------------

/// "Recently published", with how each entry has been read.
///
/// The view counts are asked for by path, bound as data, in one query for the
/// whole card — never by concatenating a slug into SQL, which is both a
/// correctness problem on MySQL and the wrong way to build a query.
///
/// An Author and above get the per-post analytics screen, which is the role
/// that screen itself asks for; a Contributor gets the live post.
async fn published_card(
    state: &AppState,
    entries: &[repo::Entry],
    timezone: &str,
    since: &str,
    role: Role,
) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let paths: Vec<String> = entries.iter().map(repo::Entry::path).collect();
    let views: std::collections::HashMap<String, i64> = repo::views_for_paths(&state.db, &paths, since)
        .await
        .into_iter()
        .map(|(path, visitors, _views)| (path, visitors))
        .collect();

    let rows: String = entries
        .iter()
        .map(|entry| {
            let title = entry
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or("(Untitled)");
            let on = entry
                .published_at
                .as_deref()
                .map(|at| ghost_core::dates::short_date_in_zone(at, timezone))
                .filter(|s| !s.is_empty())
                .map(|s| format!("Published {}", esc(&s)))
                .unwrap_or_else(|| "Published".to_string());
            let detail = match entry.status.as_str() {
                // An emailed post went to members as well as to the web, and
                // that is worth saying where the figure beside it counts only
                // the web.
                "sent" => format!("{on} · sent by email"),
                _ => on,
            };
            // `post_analytics_page` asks for Author, not Administrator, so the
            // link is offered from Author up — the gate here matches the gate
            // there rather than being stricter than it. A Contributor gets the
            // live post, which is all they could open anyway (and in practice
            // has nothing in this card, since they cannot publish).
            //
            // A link that leaves the admin opens in a new tab, as every other
            // link to the live site in this admin does — losing the admin to a
            // back button is the one thing a row like this must not do.
            let (href, target) = match role.can_publish() && ghost_core::ids::is_objectid(&entry.id) {
                true => (format!("/ghost/posts/{}/analytics/", esc(&entry.id)), ""),
                false => (format!("/{}/", esc(&entry.slug)), r#" target="_blank" rel="noopener""#),
            };
            let visitors = views.get(&entry.path()).copied().unwrap_or(0);
            format!(
                r#"<a class="row" href="{href}"{target}><span class="ic tint-green">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip tonal">{figure}</span>{chev}</div></a>"#,
                href = href,
                target = target,
                ic = svg("posts"),
                title = esc(title),
                detail = detail,
                figure = esc(&countable(visitors, "reader")),
                chev = svg("chevron"),
            )
        })
        .collect();

    format!(
        r#"<div class="section dash-head">Recently published<span class="dash-note">readers in the last 30 days</span></div>
<div class="card rows dash-rows rise" style="--i:6">{rows}</div>"#,
    )
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// "just now" / "5m ago" / "3d ago", and a plain date past a week.
///
/// Past about a week a relative figure stops meaning anything — "47d ago" is
/// something you have to count back from, where a date is something you read.
/// The same shape `social.rs`, `history.rs` and `media.rs` each use for their
/// own timestamps; each is private to its module because each measures a
/// different kind of thing.
fn when(stamp: &str) -> String {
    let Some(then) = ghost_core::dates::from_db_string(stamp) else {
        return esc(stamp);
    };
    let minutes = (chrono::Utc::now() - then).num_minutes();
    match minutes {
        m if m < 1 => "just now".to_string(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m if m < 60 * 24 * 7 => format!("{}d ago", m / (60 * 24)),
        _ => then.format("%-d %b %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(status: &str, title: Option<&str>) -> repo::Entry {
        repo::Entry {
            id: "6553f7a1b2c3d4e5f6a7b8c9".into(),
            title: title.map(str::to_string),
            slug: "a-post".into(),
            kind: "post".into(),
            status: status.into(),
            updated_at: Some("2026-11-15 09:00:00".into()),
            published_at: Some("2026-11-20 09:00:00".into()),
        }
    }

    /// A hostile title is escaped wherever it lands, in both of the cards that
    /// render one. This is the whole of this module's output risk: the rest is
    /// numbers and fixed prose.
    #[test]
    fn a_title_carrying_markup_comes_back_escaped() {
        let evil = entry("draft", Some("<script>alert(1)</script>"));
        let html = work_row(&evil, "Etc/UTC");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    /// A draft with no title still reads as something, rather than as a blank
    /// row that looks like a rendering bug.
    #[test]
    fn an_untitled_draft_is_labelled_rather_than_blank() {
        assert!(work_row(&entry("draft", None), "Etc/UTC").contains("(Untitled)"));
        assert!(work_row(&entry("draft", Some("   ")), "Etc/UTC").contains("(Untitled)"));
    }

    /// A scheduled entry says when it goes out; a draft says when it was
    /// touched. Getting these two the wrong way round is the mistake this
    /// pins, because both are dates on the same row.
    #[test]
    fn a_scheduled_entry_shows_its_publish_time_and_a_draft_its_edit_time() {
        let html = work_row(&entry("scheduled", Some("Launch")), "Etc/UTC");
        assert!(html.contains("Goes out 20 Nov 2026 at 09:00"), "{html}");
        assert!(html.contains("chip scheduled"), "{html}");

        let draft = work_row(&entry("draft", Some("Notes")), "Etc/UTC");
        assert!(draft.contains("Edited"), "{draft}");
        assert!(!draft.contains("Goes out"), "{draft}");
        assert!(draft.contains("chip draft"), "{draft}");
    }

    /// The publish time follows the site's timezone, not UTC.
    #[test]
    fn a_publish_time_is_shown_in_the_sites_zone() {
        let html = work_row(&entry("scheduled", Some("Launch")), "Asia/Dhaka");
        // 09:00 UTC is 15:00 in Dhaka, same day.
        assert!(html.contains("at 15:00"), "{html}");
    }

    /// An id that is not an ObjectID never becomes an editor link.
    #[test]
    fn a_malformed_id_is_not_linked() {
        let mut bad = entry("draft", Some("Notes"));
        bad.id = "../../etc/passwd".into();
        let html = work_row(&bad, "Etc/UTC");
        assert!(!html.contains("href"), "{html}");
        assert!(html.contains("Notes"), "{html}");
    }

    /// A page in the queue links into the pages editor, not the posts one —
    /// the editor is reached by a different path for each and the row has to
    /// pick the right one.
    #[test]
    fn a_page_links_to_the_pages_editor() {
        let mut page = entry("draft", Some("About"));
        page.kind = "page".into();
        let html = work_row(&page, "Etc/UTC");
        assert!(html.contains("/ghost/pages/6553f7a1b2c3d4e5f6a7b8c9/"), "{html}");
        assert!(!html.contains("/ghost/posts/6553"), "{html}");
    }

    /// An empty queue offers to start something instead of showing a bare
    /// card, and says whose drafts it was looking at.
    #[test]
    fn an_empty_queue_offers_a_way_to_start() {
        let mine = work_card(&[], "Etc/UTC", Role::Author);
        assert!(mine.contains("You have no drafts"), "{mine}");
        assert!(mine.contains("/ghost/posts/new/"), "{mine}");

        let all = work_card(&[], "Etc/UTC", Role::Editor);
        assert!(all.contains("No drafts"), "{all}");
        assert!(!all.contains("You have no drafts"), "{all}");
    }

    /// Nothing waiting means no "Needs you" card at all — not a card full of
    /// zeroes, which is what makes the real thing invisible when it appears.
    #[test]
    fn a_quiet_site_shows_no_attention_card() {
        assert!(pending_card(&repo::Snapshot::default()).is_empty());
    }

    /// The card carries one row per non-empty queue, the fault chipped as an
    /// error and the chore chipped tonally.
    #[test]
    fn the_attention_card_distinguishes_a_fault_from_a_chore() {
        let html = pending_card(&repo::Snapshot {
            overdue_scheduled: 2,
            new_responses: 3,
            ..repo::Snapshot::default()
        });
        assert!(html.contains("chip error"), "{html}");
        assert!(html.contains("chip tonal"), "{html}");
        assert!(html.contains("2 scheduled posts"), "{html}");
        assert!(html.contains("3 new form responses"), "{html}");
        // And in that order: the fault leads.
        let fault_at = html.find("scheduled posts").unwrap();
        let chore_at = html.find("new form responses").unwrap();
        assert!(fault_at < chore_at, "{html}");
    }

    /// Every link the attention card can produce points at a real admin
    /// route, with a filter the destination will accept.
    #[test]
    fn every_attention_row_links_somewhere_that_exists() {
        for item in [
            Pending::OverdueScheduled(1),
            Pending::FailedEmails(1),
            Pending::ReportedComments(1),
            Pending::NewResponses(1),
        ] {
            let html = pending_row(item);
            assert!(html.contains(r#"href="/ghost/"#), "{item:?} -> {html}");
        }
        assert!(pending_row(Pending::ReportedComments(1)).contains("filter=reported"));
        assert!(pending_row(Pending::NewResponses(1)).contains("status=new"));
        assert!(pending_row(Pending::OverdueScheduled(1)).contains("filter=scheduled"));
        assert!(pending_row(Pending::FailedEmails(1)).contains("filter=sent"));
    }

    /// Counts agree with their nouns, which is the detail that makes a row
    /// read like a sentence rather than a readout.
    #[test]
    fn a_single_item_is_described_in_the_singular() {
        assert!(pending_row(Pending::ReportedComments(1)).contains("1 reported comment<"));
        assert!(pending_row(Pending::ReportedComments(4)).contains("4 reported comments"));
    }

    /// Each checklist step offers somewhere to go and something to click.
    #[test]
    fn every_setup_step_offers_a_way_to_finish_it() {
        for step in [
            Setup::FirstPost,
            Setup::Describe,
            Setup::Icon,
            Setup::ConnectStripe,
            Setup::TurnOnAnalytics,
        ] {
            let html = setup_row(step);
            assert!(html.contains(r#"href="/ghost/"#), "{step:?} -> {html}");
            assert!(html.contains("btn text"), "{step:?} -> {html}");
        }
    }

    /// The greeting uses a first name and nothing more, and a user with no
    /// name set still gets a greeting rather than a dangling comma.
    #[test]
    fn the_greeting_uses_a_first_name_when_there_is_one() {
        let user = |name: Option<&str>| ghost_core::models::User {
            name: name.map(str::to_string),
            ..blank_user()
        };
        let noon = chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let actor = Actor { user: user(Some("Ada Lovelace")), role: Role::Owner };
        let html = header(&actor, "Etc/UTC", noon);
        assert!(html.contains("Good afternoon, Ada</h1>"), "{html}");
        assert!(!html.contains("Lovelace"), "{html}");
        // Monday 16 November, in the site's zone.
        assert!(html.contains("Monday 16 November"), "{html}");

        let anonymous = Actor { user: user(None), role: Role::Owner };
        let html = header(&anonymous, "Etc/UTC", noon);
        assert!(html.contains("Good afternoon</h1>"), "{html}");
        assert!(!html.contains("Good afternoon,"), "{html}");
    }

    /// A name carrying markup is escaped in the greeting too.
    #[test]
    fn a_hostile_name_does_not_survive_the_greeting() {
        let actor = Actor {
            user: ghost_core::models::User {
                name: Some("<img src=x onerror=alert(1)>".into()),
                ..blank_user()
            },
            role: Role::Owner,
        };
        let html = header(&actor, "Etc/UTC", chrono::Utc::now());
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }

    /// Media is Administrator-only, so the shortcut to it is not offered to
    /// anybody who would be refused — while the actions every role may take
    /// are offered to every role.
    #[test]
    fn the_header_only_offers_actions_the_viewer_may_take() {
        let actor = Actor { user: blank_user(), role: Role::Contributor };
        let html = header(&actor, "Etc/UTC", chrono::Utc::now());
        assert!(!html.contains("/ghost/media/"), "{html}");
        assert!(html.contains("/ghost/posts/new/"), "{html}");
        assert!(html.contains("/ghost/pages/new/"), "{html}");

        let admin = Actor { user: blank_user(), role: Role::Administrator };
        let html = header(&admin, "Etc/UTC", chrono::Utc::now());
        assert!(html.contains("/ghost/media/"), "{html}");
    }

    /// Relative while that is useful, absolute once it is not.
    #[test]
    fn edit_times_are_relative_only_while_that_means_something() {
        let now = chrono::Utc::now();
        let ago = |minutes: i64| {
            when(&(now - chrono::Duration::minutes(minutes)).format("%Y-%m-%d %H:%M:%S").to_string())
        };
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(5), "5m ago");
        assert_eq!(ago(120), "2h ago");
        assert_eq!(ago(60 * 24 * 3), "3d ago");
        // Past a week, a date rather than a number to count back from.
        assert!(!ago(60 * 24 * 40).contains("ago"), "{}", ago(60 * 24 * 40));
    }

    fn blank_user() -> ghost_core::models::User {
        ghost_core::models::User {
            id: "6553f7a1b2c3d4e5f6a7b8c9".into(),
            name: None,
            slug: None,
            email: None,
            ..Default::default()
        }
    }
}
