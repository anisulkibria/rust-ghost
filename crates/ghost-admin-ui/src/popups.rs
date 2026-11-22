//! Popups & lead generation: the admin screens.
//!
//! The rules are [`ghost_core::popups`] and the queries are
//! [`ghost_db::repos::popups`]; the visitor-facing half is
//! `ghost_api::popups`. What is here is the list, the editor, the leads and
//! their export.
//!
//! ## What this module is for
//!
//! A popup is the one piece of a site that interrupts somebody. Everything
//! on the editor exists to make the interruption answerable: *who* sees it,
//! *where*, *when*, and *how often*. A popup with no answer to those is an
//! advert pasted over the thing the reader came for, and the screen is laid
//! out in that order rather than by what is easy to store.
//!
//! ## The security contract
//!
//! Administrator and above, with an origin check on every write — the
//! module's `guard` takes a `writes` flag for that reason. The leads are
//! other people's personal data: the export runs every cell through
//! `csv_safe`, and the list is gated at the same level as Members.
//!
//! Nothing a visitor sent is ever rendered unescaped. A lead's address and
//! name arrive from the public endpoint already bounded by
//! `ghost_core::popups`, and still go out through `esc`.

use super::*;
use ghost_core::popups::{Audience, Frequency, Kind, Surface, Targeting, Trigger};
use ghost_db::repos::popups as repo;

const PATH: &str = "/ghost/popups/";
/// The module's site-wide settings: the master switch, phones, spacing and
/// the consent note. See [`popups_settings_page`].
const SETTINGS_PATH: &str = "/ghost/settings/popups/";
const SUB: &str = "Modals, slide-ins and bars that ask readers for an address.";
/// Enough to read, few enough to render. The export has everything.
const LEADS_SHOWN: i64 = 200;

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

// ---------------------------------------------------------------------------
// the list
// ---------------------------------------------------------------------------

pub(crate) async fn popups_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let popups = repo::list(&state.db).await.unwrap_or_default();
    let live = popups.iter().filter(|p| p.is_live()).count();
    let leads: i64 = popups.iter().map(|p| p.submissions).sum();

    let rows: String = popups
        .iter()
        .map(|p| {
            let rate = ghost_core::popups::conversion_rate(p.impressions, p.submissions)
                .map(|r| format!("{r:.1}% of {}", countable(p.impressions, "view")))
                // A popup nobody has seen has no rate, rather than 0%.
                .unwrap_or_else(|| "Not shown yet".into());
            format!(
                r#"<a class="row" href="{path}{id}/">
<span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name}</span>
<span class="detail">{kind} · {where_} · {rate}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span>{chev}</div></a>"#,
                path = PATH,
                id = esc(&p.id),
                tint = if p.is_live() { "green" } else { "slate" },
                ic = svg(match Kind::from_name(&p.kind) {
                    Kind::Bar => "flag",
                    Kind::SlideIn => "notes",
                    Kind::Modal => "panel-show",
                }),
                name = esc(&p.name),
                kind = esc(short_kind(Kind::from_name(&p.kind))),
                where_ = esc(&where_line(p)),
                rate = esc(&rate),
                chip = if p.is_live() { "success" } else { "" },
                status = if p.is_live() { "Live" } else { "Draft" },
                chev = svg("chevron"),
            )
        })
        .collect();

    let body = if popups.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:2;text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">No popups yet</p>
<p class="muted" style="margin:0 0 16px">One well-aimed popup on the pages that earn attention beats
one on every page.</p>
<a class="btn" href="/ghost/popups/new/">{add} Create a popup</a></div>"#,
            add = svg("add"),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:2">{rows}</div>"#)
    };

    // The module's three figures, as the admin's other reports draw them: a
    // grid of tiles rather than a row of bare numbers in one padded card.
    // Only the tiles that lead somewhere are links — see `.pu-tile` in the
    // stylesheet on why a `<div>` is used for the rest.
    let shown: i64 = popups.iter().map(|p| p.impressions).sum();
    let rate = ghost_core::popups::conversion_rate(shown, leads)
        .map(|r| format!("{r:.1}%"))
        .unwrap_or_else(|| "—".into());
    let summary = format!(
        r#"<div class="pu-tiles rise" style="--i:1">{live}{leads}{rate}</div>"#,
        live = tile(
            "Live now",
            &live.to_string(),
            &match (live, popups.len()) {
                (0, 0) => "Nothing created yet".to_string(),
                (0, n) => format!("{} waiting as a draft", countable(n as i64, "popup")),
                (_, total) => format!("of {total} {}", if total == 1 { "popup" } else { "popups" }),
            },
            None,
            "eye",
            if live > 0 { "green" } else { "slate" },
        ),
        leads = tile(
            "Leads captured",
            &leads.to_string(),
            "Addresses handed over, all time",
            (leads > 0).then_some("/ghost/popups/leads/"),
            "mail",
            "blue",
        ),
        rate = tile(
            "Conversion",
            &rate,
            // A rate nobody has been shown is not zero — it is unmeasured.
            &match shown {
                0 => "Nothing has been shown yet".to_string(),
                n => format!("of {}", countable(n, "view")),
            },
            None,
            "trending",
            "purple",
        ),
    );

    page_for_role(
        actor.role,
        "Popups & Lead Generation",
        "popups",
        Some(&actor.user),
        &format!(
            "{head}{banner}{summary}{body}{footer}",
            head = head(
                None,
                "Popups & Lead Generation",
                SUB,
                &format!(
                    r#"<a class="btn text" href="{PATH}leads/">{ic} All leads</a>
<a class="btn tonal" href="{SETTINGS_PATH}">{gear} Popup settings</a>
<a class="btn" href="{PATH}new/">{add} New popup</a>"#,
                    ic = svg("group"),
                    gear = svg("settings"),
                    add = svg("add"),
                ),
            ),
            banner = flash_banner(&q),
            summary = summary,
            body = body,
            footer = if popups.is_empty() {
                String::new()
            } else {
                r#"<p class="muted rise" style="--i:3;margin:14px 4px 0">A popup is the one thing on a site
that interrupts somebody. Each row says who sees it and where; the rate says whether the interruption was
worth it.</p>"#
                    .to_string()
            },
        ),
        true,
    )
    .into_response()
}

/// A figure, what it counts, and one line about it — the shape Newsletters,
/// Memberships and Security all draw their numbers in.
fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<span class="pu-tile-value">{value}</span>
<span class="pu-tile-label">{label}</span>
<span class="pu-tile-detail">{detail}</span>"#,
        tint = tint,
        icon = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(href) => format!(r#"<a class="card padded pu-tile" href="{href}">{body}</a>"#),
        None => format!(r#"<div class="card padded pu-tile">{body}</div>"#),
    }
}

/// The module's page header.
///
/// A module's main screen has no back arrow — it is reached from the rail and
/// the Modules hub, and "back" from it means nothing in particular — while a
/// screen inside it points at that main screen. Popups was a Tool until
/// 2026-11-20 and used `tool_head`, which looks its key up in `TOOLS`; a key
/// that is no longer there panics, so the module carries its own.
fn head(back: Option<&str>, title: &str, sub: &str, actions: &str) -> String {
    let back = back
        .map(|href| {
            format!(
                r#"<a class="icon-btn" style="transform:rotate(180deg)" href="{href}" title="Back to popups">{chev}</a>"#,
                href = esc(href),
                chev = svg("chevron"),
            )
        })
        .unwrap_or_default();
    format!(
        r#"<header class="page-head rise">{back}<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions">{actions}</div></header>"#,
        title = esc(title),
        sub = esc(sub),
    )
}

fn short_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Modal => "Modal",
        Kind::SlideIn => "Slide-in",
        Kind::Bar => "Bar",
    }
}

/// "Every page", or "Only 3 pages" — the list row's one-line answer to
/// *where*.
fn where_line(p: &repo::Popup) -> String {
    let count = p.path_rules().len();
    let addresses = match Targeting::from_name(&p.targeting) {
        Targeting::Everywhere => String::new(),
        Targeting::Only => format!(", only {}", countable(count as i64, "address")),
        Targeting::Except => format!(", not on {}", countable(count as i64, "address")),
    };
    let surface = match Surface::from_name(&p.surface) {
        Surface::Any => "Every page",
        Surface::Posts => "Posts",
        Surface::Pages => "Pages",
        Surface::Home => "The home page",
        Surface::Archives => "Archives",
    };
    format!("{surface}{addresses}")
}

// ---------------------------------------------------------------------------
// the editor
// ---------------------------------------------------------------------------

fn picker<T: Copy + PartialEq>(
    name: &str,
    label: &str,
    all: &[T],
    current: T,
    value: impl Fn(T) -> &'static str,
    text: impl Fn(T) -> &'static str,
) -> String {
    let options: String = all
        .iter()
        .map(|item| {
            format!(
                r#"<option value="{v}"{sel}>{t}</option>"#,
                v = value(*item),
                sel = if *item == current { " selected" } else { "" },
                t = esc(text(*item)),
            )
        })
        .collect();
    format!(
        r#"<label class="field fixed" style="margin:0"><select name="{name}">{options}</select><span>{label}</span>{car}</label>"#,
        name = esc(name),
        label = esc(label),
        car = SELECT_CARET,
    )
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing pu-control">{control}</div></div>"#,
        tint = tint,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

pub(crate) async fn popup_new_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&actor, None, &q).await
}

pub(crate) async fn popup_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) {
        return not_found_page(&actor.user, "That popup could not be found.");
    }
    match repo::find(&state.db, &id).await {
        Ok(Some(popup)) => editor(&actor, Some(popup), &q).await,
        _ => not_found_page(&actor.user, "That popup could not be found."),
    }
}

/// The editor is pure rendering: it takes the row it was handed and reads no
/// state of its own, which is why it takes no `&AppState`.
async fn editor(
    actor: &Actor,
    popup: Option<repo::Popup>,
    q: &HashMap<String, String>,
) -> Response {
    let new = popup.is_none();
    let p = popup.unwrap_or_else(blank);
    let action = if new { format!("{PATH}new/") } else { format!("{PATH}{}/", p.id) };

    let content = format!(
        r#"<div class="section">What it says</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="name" value="{name}" placeholder=" " required maxlength="{max_name}"><span>Name</span></label>
<p class="muted" style="margin:6px 0 0">For this screen only — readers never see it.</p></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="heading" value="{heading}" placeholder=" " maxlength="{max_heading}"><span>Heading</span></label></div></div>
<div class="row"><span class="ic tint-pink">{notes}</span>
<div class="body"><label class="field" style="margin:0"><textarea name="body" rows="3" placeholder=" " maxlength="{max_body}">{body}</textarea><span>Message</span></label></div></div>
</div>"#,
        title_ic = svg("title"),
        notes = svg("notes"),
        name = esc(&p.name),
        heading = esc(&p.heading),
        body = esc(&p.body),
        max_name = ghost_core::popups::MAX_NAME,
        max_heading = ghost_core::popups::MAX_HEADING,
        max_body = ghost_core::popups::MAX_BODY,
    );

    let ask = format!(
        r#"<div class="section">What it asks for</div>
<div class="card rows rise" style="--i:2">
{capture}
{capture_name}
<div class="row" style="align-items:center"><span class="ic tint-blue">{tap}</span>
<div class="body"><label class="field" style="margin:0"><input name="button_label" value="{button_label}" placeholder=" " maxlength="{max_label}"><span>Button</span></label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-blue">{link}</span>
<div class="body"><label class="field" style="margin:0"><input name="button_url" value="{button_url}" placeholder=" " maxlength="500"><span>Button link</span></label>
<p class="muted" style="margin:6px 0 0">Used only when the popup is not collecting addresses.</p></div></div>
<div class="row" style="align-items:center"><span class="ic tint-green">{check}</span>
<div class="body"><label class="field" style="margin:0"><input name="success_message" value="{success}" placeholder=" " maxlength="200"><span>After they subscribe</span></label></div></div>
</div>"#,
        capture = row(
            "mail",
            "blue",
            "Collect email addresses",
            "Everyone who subscribes becomes a free member, with the newsletters that subscribe on signup.",
            &checkbox("capture_email", p.captures_email()),
        ),
        capture_name = row(
            "person",
            "blue",
            "Ask for a name as well",
            "One more field is one more reason to close it. Worth it only if you will use the name.",
            &checkbox("capture_name", p.captures_name()),
        ),
        tap = svg("add"),
        link = svg("link"),
        check = svg("check"),
        button_label = esc(&p.button_label),
        button_url = esc(&p.button_url),
        success = esc(&p.success_message),
        max_label = ghost_core::popups::MAX_LABEL,
    );

    let behaviour = format!(
        r#"<div class="section">Who, where and when</div>
<div class="card rows rise" style="--i:3">
{kind}
{audience}
{surface}
{targeting}
<div class="row"><span class="ic tint-teal">{list_ic}</span>
<div class="body"><label class="field" style="margin:0"><textarea name="paths" rows="3" placeholder=" " spellcheck="false">{paths}</textarea><span>Pages</span></label>
<p class="muted" style="margin:6px 0 0">One address per line — <code>/about</code>, <code>/tag/news</code>.
A rule covers that page and everything under it; <code>/</code> on its own means the home page.</p></div></div>
{trigger}
{trigger_value}
{frequency}
{frequency_days}
<div class="row" style="align-items:center"><span class="ic tint-amber">{cal}</span>
<div class="body"><label class="field" style="margin:0"><input type="datetime-local" name="starts_at" value="{starts}" placeholder=" "><span>Start (optional)</span></label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-amber">{cal}</span>
<div class="body"><label class="field" style="margin:0"><input type="datetime-local" name="ends_at" value="{ends}" placeholder=" "><span>End (optional)</span></label></div></div>
</div>"#,
        kind = row(
            "panel-show",
            "purple",
            "How it appears",
            "A modal takes the most attention and gives the most offence.",
            &picker("kind", "Style", Kind::ALL, Kind::from_name(&p.kind), Kind::name, Kind::label),
        ),
        audience = row(
            "group",
            "purple",
            "Who sees it",
            "Asking a member for the address they signed up with is the clearest way to look inattentive.",
            &picker("audience", "Audience", Audience::ALL, Audience::from_name(&p.audience), Audience::name, Audience::label),
        ),
        surface = row(
            "pages",
            "teal",
            "What kind of page",
            "Posts, static pages, the home page or the archives. Ghost puts a post at /slug/, beside the pages, so this cannot be written as a path.",
            &picker("surface", "Show on", Surface::ALL, Surface::from_name(&p.surface), Surface::name, Surface::label),
        ),
        targeting = row(
            "globe",
            "teal",
            "Which addresses",
            "Narrow it further by URL. Applied on top of the choice above, not instead of it.",
            &picker("targeting", "Addresses", Targeting::ALL, Targeting::from_name(&p.targeting), Targeting::name, Targeting::label),
        ),
        list_ic = svg("menu"),
        paths = esc(&p.paths),
        trigger = row(
            "play",
            "green",
            "What makes it appear",
            "Exit intent falls back to a long delay on a phone, where there is no such gesture.",
            &picker("trigger_kind", "Trigger", Trigger::ALL, Trigger::from_name(&p.trigger_kind), Trigger::name, Trigger::label),
        ),
        trigger_value = row(
            "history",
            "green",
            "After how much",
            "Seconds for a delay, percent of the page for a scroll. Ignored for exit intent.",
            &format!(
                r#"<label class="field" style="margin:0;max-width:120px"><input type="number" name="trigger_value" value="{v}" min="1" max="120" placeholder=" "><span>Amount</span></label>"#,
                v = p.trigger_value,
            ),
        ),
        frequency = row(
            "repeat",
            "amber",
            "How often one person sees it",
            "Remembered in their browser. Someone who closed it has answered.",
            &picker("frequency", "Frequency", Frequency::ALL, Frequency::from_name(&p.frequency), Frequency::name, Frequency::label),
        ),
        frequency_days = row(
            "calendar",
            "amber",
            "Wait how long",
            "Used only with “Once, then wait a while”.",
            &format!(
                r#"<label class="field" style="margin:0;max-width:120px"><input type="number" name="frequency_days" value="{v}" min="1" max="365" placeholder=" "><span>Days</span></label>"#,
                v = if p.frequency_days > 0 { p.frequency_days } else { 30 },
            ),
        ),
        cal = svg("calendar"),
        starts = esc(&to_local_input(p.starts_at.as_deref())),
        ends = esc(&to_local_input(p.ends_at.as_deref())),
    );

    // Status is owned by the buttons in the header, not by a control down
    // here. It was a switch until 2026-11-20, and the switch flicked while
    // the words beside it — rendered from the *saved* row — went on saying
    // "Draft", which reads as a broken toggle rather than as an unsaved
    // change. A state you can only change by saving belongs on the button
    // that saves.
    let status = format!(
        r#"<div class="section">Status</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip {chip}">{title}</span></div></div>
</div>"#,
        tint = if p.is_live() { "green" } else { "slate" },
        ic = svg("eye"),
        chip = if p.is_live() { "success" } else { "" },
        title = if p.is_live() { "Live" } else { "Draft" },
        detail = if p.is_live() {
            "Showing to readers who match. Unpublish to take it down."
        } else if new {
            "Nobody sees it until you publish. Save as a draft to come back to it."
        } else {
            "Nobody sees it. Publish when it is ready."
        },
    );

    let stats = if new {
        String::new()
    } else {
        let rate = ghost_core::popups::conversion_rate(p.impressions, p.submissions)
            .map(|r| format!("{r:.1}%"))
            // A popup nobody has seen has no rate, rather than 0%.
            .unwrap_or_else(|| "—".into());
        let leads_href = format!("{PATH}{}/leads/", p.id);
        format!(
            r#"<div class="section">How it has done</div>
<div class="pu-tiles rise" style="--i:5">{shown}{subscribed}{converted}</div>"#,
            shown = tile(
                "Shown",
                &p.impressions.to_string(),
                "Times it has appeared",
                None,
                "eye",
                "slate",
            ),
            subscribed = tile(
                "Subscribed",
                &p.submissions.to_string(),
                if p.submissions == 1 { "One address captured" } else { "Addresses captured" },
                (p.submissions > 0).then_some(leads_href.as_str()),
                "mail",
                "blue",
            ),
            converted = tile(
                "Conversion",
                &rate,
                match p.impressions {
                    0 => "Nothing has been shown yet",
                    _ => "Of everyone who saw it",
                },
                None,
                "trending",
                "purple",
            ),
        )
    };

    let delete = if new {
        String::new()
    } else {
        format!(
            r#"<form method="post" action="{PATH}{id}/delete/" class="rise" style="--i:6;margin-top:18px"
 data-confirm="Delete this popup|“{name}” and the {leads} it captured are removed. Export them first if you want them.">
<button class="btn text danger" type="submit">{bin} Delete popup</button></form>"#,
            id = esc(&p.id),
            name = esc(&p.name),
            leads = countable(p.submissions, "lead"),
            bin = svg("delete"),
        )
    };

    page_for_role(
        actor.role,
        if new { "New popup" } else { "Edit popup" },
        "popups",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<form method="post" action="{action}" id="popup-form">
{content}{ask}{behaviour}{status}
</form>
{stats}{delete}"#,
            // Two verbs, as the post editor has: one that keeps it out of
            // sight and one that puts it up. A live popup gets Unpublish in
            // place of Publish, so taking something down is never hidden
            // behind "save with a switch off".
            // Back to the popups list, not the Tools hub: that list is where
            // this editor was opened from.
            head = head(
                Some(PATH),
                if new { "New popup" } else { p.name.as_str() },
                SUB,
                &actions(&p, new),
            ),
            banner = flash_banner(q),
            action = esc(&action),
        ),
        true,
    )
    .into_response()
}

/// The editor's two buttons.
///
/// `name="action"` on a submit button is only sent when *that* button is the
/// one pressed, which is what lets one form carry both verbs.
fn actions(p: &repo::Popup, new: bool) -> String {
    let save = format!(
        r#"<button class="btn text" type="submit" name="action" value="save" form="popup-form">{check} {label}</button>"#,
        check = svg("check"),
        label = if p.is_live() { "Save" } else { "Save draft" },
    );
    let publish = if p.is_live() {
        format!(
            r#"<button class="btn tonal" type="submit" name="action" value="unpublish" form="popup-form"
 data-confirm="Unpublish this popup|It stops appearing straight away. Everything it has captured is kept.">{hide} Unpublish</button>"#,
            hide = svg("hide"),
        )
    } else {
        format!(
            r#"<button class="btn" type="submit" name="action" value="publish" form="popup-form">{eye} {label}</button>"#,
            eye = svg("eye"),
            label = if new { "Publish" } else { "Publish" },
        )
    };
    format!("{save}{publish}")
}

fn checkbox(name: &str, on: bool) -> String {
    format!(
        r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{on}><span class="track"><span class="thumb"></span></span></label>"#,
        name = esc(name),
        on = if on { " checked" } else { "" },
    )
}

/// A fresh popup, with the defaults that make the first save safe: a draft,
/// shown to people who are not signed in, once each, after five seconds.
fn blank() -> repo::Popup {
    repo::Popup {
        id: String::new(),
        name: String::new(),
        status: "draft".into(),
        kind: "modal".into(),
        heading: "Subscribe to the newsletter".into(),
        body: "New posts in your inbox. No spam, unsubscribe any time.".into(),
        button_label: "Subscribe".into(),
        button_url: String::new(),
        success_message: "Thanks — check your inbox.".into(),
        capture_email: 1,
        capture_name: 0,
        trigger_kind: "delay".into(),
        trigger_value: 5,
        frequency: "once".into(),
        frequency_days: 30,
        audience: "logged_out".into(),
        surface: "any".into(),
        targeting: "everywhere".into(),
        paths: String::new(),
        starts_at: None,
        ends_at: None,
        impressions: 0,
        submissions: 0,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

/// `2026-11-20 14:30:00` → `2026-11-20T14:30`, which is what a
/// `datetime-local` input wants, and back again below.
fn to_local_input(stored: Option<&str>) -> String {
    let value = stored.unwrap_or("").trim();
    if value.len() < 16 {
        return String::new();
    }
    format!("{}T{}", &value[..10], &value[11..16])
}

fn from_local_input(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.len() < 16 {
        return None;
    }
    // The browser sends local time with no zone; the site stores UTC strings.
    // Treated as UTC rather than guessed at: a schedule that is an hour out
    // is a smaller surprise than one that silently moved by a timezone the
    // admin never chose, and the field says nothing about zones.
    Some(format!("{} {}:00", &value[..10], &value[11..16]))
}

// ---------------------------------------------------------------------------
// writes
// ---------------------------------------------------------------------------

/// What the pressed button means for the popup's status.
///
/// `current` is what the row says now, so a plain Save on a live popup keeps
/// it live. An action nobody offered — or none at all, which is what a form
/// submitted with the Enter key sends — falls back to the current status, so
/// pressing Enter in a text field can never publish something by accident.
fn status_after(action: &str, current: &str) -> String {
    match action {
        "publish" => "live".into(),
        "unpublish" | "draft" => "draft".into(),
        _ => current.to_string(),
    }
}

fn input_from(form: &HashMap<String, String>, current_status: &str) -> repo::PopupInput {
    use ghost_core::popups as rules;
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let on = |k: &str| form.get(k).is_some_and(|v| v == "1");

    let kind = Kind::from_name(get("kind"));
    let trigger = Trigger::from_name(get("trigger_kind"));
    let frequency = Frequency::from_name(get("frequency"));
    let targeting = Targeting::from_name(get("targeting"));
    let audience = Audience::from_name(get("audience"));

    repo::PopupInput {
        name: {
            let name = rules::clean(get("name"), rules::MAX_NAME);
            if name.is_empty() { "Untitled popup".into() } else { name }
        },
        status: status_after(get("action"), current_status),
        kind: kind.name().into(),
        heading: rules::clean(get("heading"), rules::MAX_HEADING),
        body: rules::clean(get("body"), rules::MAX_BODY),
        button_label: rules::clean(get("button_label"), rules::MAX_LABEL),
        // Only a link the admin may actually have meant. Anything else —
        // `javascript:`, a bare word — is dropped rather than rendered into
        // somebody else's page.
        button_url: {
            let url = rules::clean(get("button_url"), 500);
            let ok = url.starts_with('/')
                || url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("mailto:");
            if ok && !url.starts_with("//") { url } else { String::new() }
        },
        success_message: rules::clean(get("success_message"), 200),
        // 0/1 rather than a bool: see `repo::Popup::capture_email`.
        capture_email: on("capture_email") as i64,
        capture_name: on("capture_name") as i64,
        trigger_kind: trigger.name().into(),
        trigger_value: trigger.clamp(get("trigger_value").parse().unwrap_or(5)),
        frequency: frequency.name().into(),
        frequency_days: frequency.clamp_days(get("frequency_days").parse().unwrap_or(30)),
        audience: audience.name().into(),
        surface: Surface::from_name(get("surface")).name().into(),
        targeting: targeting.name().into(),
        // Rebuilt from the parser rather than stored as typed, so what is
        // saved is exactly what the matcher will read.
        paths: rules::parse_paths(get("paths")).join("\n"),
        starts_at: from_local_input(get("starts_at")),
        ends_at: from_local_input(get("ends_at")),
    }
}

pub(crate) async fn popup_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let input = input_from(&form, "draft");
    let published = input.status == "live";
    match repo::insert(&state.db, &input).await {
        Ok(id) => tool_saved(
            &state,
            &format!("{PATH}{id}/"),
            if published { "Popup published — it is live now." } else { "Saved as a draft." },
        ),
        Err(e) => {
            tracing::error!("popups: create failed: {}", e.message());
            tool_flash(&format!("{PATH}new/"), "error", "That could not be created.")
        }
    }
}

pub(crate) async fn popup_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return tool_flash(PATH, "error", "That popup could not be found.");
    }
    // Read the row first: a plain Save must not change whether it is live,
    // and only the row knows what it is now.
    let current = repo::find(&state.db, &id)
        .await
        .ok()
        .flatten()
        .map(|p| p.status)
        .unwrap_or_else(|| "draft".into());
    let input = input_from(&form, &current);
    let message = match (current.as_str(), input.status.as_str()) {
        ("draft", "live") => "Popup published — it is live now.",
        ("live", "draft") => "Popup unpublished. Nobody sees it now.",
        _ => "Popup saved.",
    };
    match repo::update(&state.db, &id, &input).await {
        Ok(()) => tool_saved(&state, &format!("{PATH}{id}/"), message),
        Err(e) => {
            tracing::error!("popups: saving {id} failed: {}", e.message());
            tool_flash(&format!("{PATH}{id}/"), "error", "That could not be saved.")
        }
    }
}

pub(crate) async fn popup_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return tool_flash(PATH, "error", "That popup could not be found.");
    }
    match repo::destroy(&state.db, &id).await {
        Ok(()) => tool_saved(&state, PATH, "Popup deleted."),
        Err(e) => {
            tracing::error!("popups: deleting {id} failed: {}", e.message());
            tool_flash(PATH, "error", "That could not be deleted.")
        }
    }
}

// ---------------------------------------------------------------------------
// Settings → Popups: what applies to every popup at once
// ---------------------------------------------------------------------------

fn site_settings(state: &AppState) -> ghost_core::popups::SiteSettings {
    ghost_core::popups::SiteSettings::read(|key| state.settings.get_str(key))
}

pub(crate) async fn popups_settings_page(
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
        .find(|g| g.slug == "popups")
        .expect("popups settings group");
    let site = site_settings(&state);
    let live = repo::list(&state.db).await.unwrap_or_default().iter().filter(|p| p.is_live()).count();

    page_for_role(
        actor.role,
        "Popups",
        "settings",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<form method="post" action="{SETTINGS_PATH}" id="popups-settings">
<div class="section">On or off</div>
<div class="card rows rise" style="--i:1">
{enabled}
</div>
<div class="section">On phones</div>
<div class="card rows rise" style="--i:2">
{mobile}
</div>
<div class="section">Space between popups</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:center"><span class="ic tint-amber">{clock}</span>
<div class="body"><span class="title">Hours between any two popups</span>
<span class="detail">Across every popup on the site, for one visitor. 0 leaves only each popup's own frequency.</span></div>
<div class="trailing pu-control"><label class="field" style="margin:0;max-width:140px"><input type="number" name="{cooldown_key}" value="{cooldown}" min="0" max="{max_cooldown}" placeholder=" "><span>Hours</span></label></div></div>
</div>
<div class="section">Consent note</div>
<div class="card rows rise" style="--i:4">
<div class="row"><span class="ic tint-blue">{notes}</span>
<div class="body"><label class="field" style="margin:0"><textarea name="{consent_key}" rows="2" maxlength="{max_consent}" placeholder=" ">{consent}</textarea><span>Shown under every address field</span></label>
<p class="muted" style="margin:6px 0 0">What will be sent, and how to stop it — for example <i>“One email a week. Unsubscribe in one click.”</i> Blank shows nothing.</p></div></div>
</div>
</form>
<p class="muted rise" style="--i:5;margin:14px 4px 0">The popups themselves — what they say, who sees them and where — are in
<a href="{PATH}">Tools → Popups &amp; Lead Generation</a>.</p>"#,
            head = crate::group_head(group, Some("popups-settings")),
            banner = flash_banner(&q),
            enabled = row(
                "eye",
                if site.enabled { "green" } else { "slate" },
                "Show popups on the site",
                &if site.enabled {
                    format!("{} showing now. Switch off to take every popup down at once without unpublishing them.", countable(live as i64, "popup"))
                } else {
                    "Off: no popup appears anywhere, and the visitor script is not added to any page.".to_string()
                },
                &checkbox(ghost_core::popups::SETTING_ENABLED, site.enabled),
            ),
            mobile = row(
                "layout",
                "purple",
                "Hide popups on phones",
                "On a small screen a modal covers the whole page. Slide-ins and bars are hidden too.",
                &checkbox(ghost_core::popups::SETTING_HIDE_ON_MOBILE, site.hide_on_mobile),
            ),
            clock = svg("history"),
            notes = svg("notes"),
            cooldown_key = ghost_core::popups::SETTING_COOLDOWN_HOURS,
            cooldown = site.cooldown_hours,
            max_cooldown = ghost_core::popups::MAX_COOLDOWN_HOURS,
            consent_key = ghost_core::popups::SETTING_CONSENT_TEXT,
            consent = esc(&site.consent_text),
            max_consent = ghost_core::popups::MAX_CONSENT,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn popups_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    use ghost_core::popups as rules;
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    // Normalised through the same reader the public endpoint uses, so what is
    // stored is exactly what will be applied: a checkbox is present or absent,
    // the hours are clamped, the note is bounded and stripped.
    let site = rules::SiteSettings::read(|key| match key {
        rules::SETTING_ENABLED | rules::SETTING_HIDE_ON_MOBILE => {
            Some(if form.get(key).is_some_and(|v| v == "1") { "true" } else { "false" }.to_string())
        }
        _ => form.get(key).cloned(),
    });
    let writes = [
        (rules::SETTING_ENABLED, serde_json::Value::Bool(site.enabled)),
        (rules::SETTING_HIDE_ON_MOBILE, serde_json::Value::Bool(site.hide_on_mobile)),
        (rules::SETTING_COOLDOWN_HOURS, serde_json::json!(site.cooldown_hours)),
        (rules::SETTING_CONSENT_TEXT, serde_json::json!(site.consent_text)),
    ];
    let mut ok = true;
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("popups: saving {key} failed: {}", e.message());
            ok = false;
        }
    }
    if ok {
        settings_saved(&state, "popups", "Popup settings saved.")
    } else {
        settings_flash("popups", "error", "Saving failed — check the logs.")
    }
}

// ---------------------------------------------------------------------------
// leads
// ---------------------------------------------------------------------------

pub(crate) async fn leads_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let leads = repo::all_leads(&state.db, LEADS_SHOWN).await.unwrap_or_default();
    let names: HashMap<String, String> = repo::list(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    leads_screen(&actor, leads, &names, None, &q).await
}

pub(crate) async fn popup_leads_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) {
        return not_found_page(&actor.user, "That popup could not be found.");
    }
    let Ok(Some(popup)) = repo::find(&state.db, &id).await else {
        return not_found_page(&actor.user, "That popup could not be found.");
    };
    let leads = repo::leads(&state.db, &id, LEADS_SHOWN).await.unwrap_or_default();
    let names: HashMap<String, String> = [(popup.id.clone(), popup.name.clone())].into_iter().collect();
    leads_screen(&actor, leads, &names, Some(popup), &q).await
}

async fn leads_screen(
    actor: &Actor,
    leads: Vec<repo::Lead>,
    names: &HashMap<String, String>,
    only: Option<repo::Popup>,
    q: &HashMap<String, String>,
) -> Response {
    let title = match &only {
        Some(p) => format!("Leads from “{}”", p.name),
        None => "All leads".to_string(),
    };
    let export = match &only {
        Some(p) => format!("{PATH}{}/leads.csv", p.id),
        None => format!("{PATH}leads.csv"),
    };
    let rows: String = leads
        .iter()
        .map(|lead| {
            let source = names.get(&lead.popup_id).map(String::as_str).unwrap_or("a deleted popup");
            format!(
                r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{email}</span>
<span class="detail">{name}{sep}{source} · {when}</span></div>
<div class="trailing">{member}</div></div>"#,
                ic = svg("mail"),
                email = esc(&lead.email),
                name = esc(&lead.name),
                sep = if lead.name.trim().is_empty() { "" } else { " · " },
                source = esc(source),
                when = esc(&lead.created_at),
                member = match &lead.member_id {
                    Some(id) if ghost_core::ids::is_objectid(id) => format!(
                        r#"<a class="btn text small" href="/ghost/members/{id}/">Member</a>"#,
                        id = esc(id),
                    ),
                    // A lead with no member is somebody whose signup did not
                    // complete. Worth saying so rather than showing nothing.
                    _ => r#"<span class="chip">Not a member</span>"#.to_string(),
                },
            )
        })
        .collect();

    let body = if leads.is_empty() {
        r#"<div class="card padded rise" style="--i:1;text-align:center">
<p class="muted" style="margin:10px 0">No addresses yet.</p></div>"#
            .to_string()
    } else {
        format!(r#"<div class="card rows rise" style="--i:1">{rows}</div>"#)
    };

    page_for_role(
        actor.role,
        &title,
        "popups",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="{back}" title="Back">{chev}</a>
<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions"><a class="btn tonal" href="{export}">{dl} Export CSV</a></div></header>
{banner}{body}"#,
            back = match &only {
                Some(p) => format!("{PATH}{}/", p.id),
                None => PATH.to_string(),
            },
            chev = svg("chevron"),
            title = esc(&title),
            sub = format!(
                "{} · newest first{}",
                countable(leads.len() as i64, "lead"),
                if leads.len() as i64 == LEADS_SHOWN { ", most recent shown" } else { "" },
            ),
            export = esc(&export),
            dl = svg("import"),
            banner = flash_banner(q),
            body = body,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn leads_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    let leads = repo::all_leads(&state.db, 1000).await.unwrap_or_default();
    let names: HashMap<String, String> = repo::list(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    csv_response(&leads, &names, "popup-leads.csv")
}

pub(crate) async fn popup_leads_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let leads = repo::leads(&state.db, &id, 1000).await.unwrap_or_default();
    let names: HashMap<String, String> = repo::find(&state.db, &id)
        .await
        .ok()
        .flatten()
        .map(|p| (p.id, p.name))
        .into_iter()
        .collect();
    csv_response(&leads, &names, "popup-leads.csv")
}

/// Every cell goes through `csv_safe`: a cell beginning `=`, `+`, `@` or `-`
/// is a formula in a spreadsheet, and these cells came from the internet.
fn csv_response(leads: &[repo::Lead], names: &HashMap<String, String>, filename: &str) -> Response {
    let mut out = String::from("email,name,popup,page,member_id,captured_at\n");
    for lead in leads {
        let source = names.get(&lead.popup_id).map(String::as_str).unwrap_or("");
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            ghost_core::forms::csv_safe(&lead.email),
            ghost_core::forms::csv_safe(&lead.name),
            ghost_core::forms::csv_safe(source),
            ghost_core::forms::csv_safe(&lead.source_url),
            ghost_core::forms::csv_safe(lead.member_id.as_deref().unwrap_or("")),
            ghost_core::forms::csv_safe(&lead.created_at),
        ));
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        out,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_verb_changes_whether_a_popup_is_live() {
        // Save keeps what it was, both ways round.
        assert_eq!(status_after("save", "live"), "live");
        assert_eq!(status_after("save", "draft"), "draft");
        // The two verbs that do change it.
        assert_eq!(status_after("publish", "draft"), "live");
        assert_eq!(status_after("unpublish", "live"), "draft");
        // Enter in a text field sends no action at all, and must not
        // publish something by accident.
        assert_eq!(status_after("", "draft"), "draft");
        assert_eq!(status_after("", "live"), "live");
        // Nor may a forged value invent a state.
        assert_eq!(status_after("go-viral", "draft"), "draft");
    }

    #[test]
    fn a_datetime_input_round_trips() {
        assert_eq!(to_local_input(Some("2026-11-20 14:30:00")), "2026-11-20T14:30");
        assert_eq!(to_local_input(None), "");
        assert_eq!(to_local_input(Some("")), "");
        assert_eq!(from_local_input("2026-11-20T14:30").as_deref(), Some("2026-11-20 14:30:00"));
        assert_eq!(from_local_input(""), None);
        assert_eq!(from_local_input("nonsense"), None);
    }

    #[test]
    fn a_hostile_button_link_is_dropped() {
        let mut form: HashMap<String, String> = HashMap::new();
        form.insert("button_url".into(), "javascript:alert(1)".into());
        assert_eq!(input_from(&form, "draft").button_url, "");
        // Protocol-relative is how a link leaves the site without saying so.
        form.insert("button_url".into(), "//evil.example.com".into());
        assert_eq!(input_from(&form, "draft").button_url, "");
        for good in ["/about/", "https://example.com/x", "mailto:hi@example.com"] {
            form.insert("button_url".into(), good.into());
            assert_eq!(input_from(&form, "draft").button_url, good);
        }
    }

    #[test]
    fn an_empty_form_still_makes_a_usable_popup() {
        // Every select arrives as a known value or its safe default, and an
        // unnamed popup is named rather than refused.
        let input = input_from(&HashMap::new(), "draft");
        assert_eq!(input.name, "Untitled popup");
        // No action pressed, so the status it came in with survives — and a
        // new popup comes in as a draft.
        assert_eq!(input.status, "draft", "a popup never goes live by accident");
        assert_eq!(input.kind, "modal");
        assert_eq!(input.audience, "logged_out");
        assert_eq!(input.targeting, "everywhere");
        assert_eq!(input.surface, "any");
        assert_eq!(input.trigger_kind, "delay");
        assert!(input.trigger_value >= 1);
        assert_eq!(input.capture_email, 0, "an unchecked box is off");
    }

    #[test]
    fn paths_are_stored_as_the_matcher_will_read_them() {
        let mut form: HashMap<String, String> = HashMap::new();
        form.insert("paths".into(), " tag/news/ \n\n/about?x=1\n# note\n".into());
        assert_eq!(input_from(&form, "draft").paths, "/tag/news\n/about");
    }

    #[test]
    fn a_forged_select_value_falls_back_rather_than_being_stored() {
        let mut form: HashMap<String, String> = HashMap::new();
        for (key, value) in [
            ("kind", "iframe"),
            ("audience", "everyone-including-admins"),
            ("targeting", "regex"),
            ("trigger_kind", "onload"),
            ("frequency", "hourly"),
            ("surface", "everything-everywhere"),
        ] {
            form.insert(key.into(), value.into());
        }
        let input = input_from(&form, "draft");
        assert_eq!(input.kind, "modal");
        assert_eq!(input.audience, "logged_out");
        assert_eq!(input.targeting, "everywhere");
        assert_eq!(input.trigger_kind, "delay");
        assert_eq!(input.frequency, "once");
        assert_eq!(input.surface, "any");
    }
}
