//! Tools: the screens that shape the public site itself — its look, its
//! menus, the code and banner on every page, how its URLs are built and
//! where old ones go — plus what connects to it and who can sign in to
//! change any of it.
//!
//! Most of these were Settings groups. They moved because they are not
//! configuration you set once: they are things you work on. Each keeps its
//! own top-level URL, and the old Settings URLs redirect here.
//!
//! Redirects and Routes used to be a file upload and nothing else, so what
//! was in force could only be found by downloading the file again. Both are
//! editors now: every rule on screen, in the order it is tried, with a tester
//! that says which rule (or route) an address hits and why. The file is still
//! there for anyone who keeps it in version control.

use super::*;
use vuuut_routing::routes_yaml::{CollectionEntry, RouteEntry, RouteSettings};
use vuuut_routing::{RouteMatch, RouterTree};

pub(crate) struct Tool {
    pub(crate) key: &'static str,
    pub(crate) title: &'static str,
    pub(crate) href: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) tint: &'static str,
    pub(crate) description: &'static str,
}

/// A–Z by title, like `SETTINGS_GROUPS`, so a name is looked up rather than
/// hunted for. Keep it sorted when adding a tool.
///
/// Since 2026-11-21 this is the only hub: the ten cards that were Modules
/// (Analytics, Cookie Consent, Forms, Languages, Media, Memberships,
/// Newsletters, Popups, Security, SEO) moved here and `/ghost/modules/`
/// redirects. Their permission grants were carried across by
/// `vuuut_core::permissions::FORMER_MODULES`.
pub(crate) const TOOLS: &[Tool] = &[
    Tool { key: "ads", title: "Advertising & Ad Management", href: "/ghost/ads/", icon: "campaign", tint: "amber",
        description: "Sell and run ads: banners, sponsors and network code, with reports for each advertiser." },
    Tool { key: "analytics", title: "Analytics", href: "/ghost/analytics/", icon: "chart", tint: "green",
        description: "Track visitors, content performance, and member growth." },
    Tool { key: "announcement", title: "Announcement", href: "/ghost/announcement/", icon: "flag", tint: "amber",
        description: "A bar across the top of the site, for the audiences you pick." },
    Tool { key: "automations", title: "Automations", href: "/ghost/automations/", icon: "flow", tint: "blue",
        description: "Manage automated workflows and their activity." },
    Tool { key: "backups", title: "Backups", href: "/ghost/backups/", icon: "history", tint: "green",
        description: "Automatic and manual backups of the whole site, ready to restore — plus exports." },
    Tool { key: "caching", title: "Caching & Performance", href: "/ghost/caching/", icon: "trending", tint: "green",
        description: "A page cache, browser and CDN headers, compression and faster pages — measured on this server." },
    Tool { key: "code-injection", title: "Code injection", href: "/ghost/code-injection/", icon: "code", tint: "slate",
        description: "Scripts and styles added to the head and foot of every page." },
    Tool { key: "comments", title: "Comments", href: "/ghost/comments/", icon: "comment", tint: "purple",
        description: "Review discussions and manage comments across your posts." },
    Tool { key: "cookies", title: "Cookie Consent & Privacy", href: "/ghost/cookies/", icon: "cookie", tint: "amber",
        description: "Ask before tracking, hold back what nobody agreed to, and keep the proof." },
    Tool { key: "directories", title: "Directory & Classified Listings", href: "/ghost/directories/", icon: "library", tint: "teal",
        description: "A public directory and classifieds in your theme: categories, visitor submissions, review and expiry." },
    Tool { key: "dummies", title: "Dummy content", href: "/ghost/dummies/", icon: "box", tint: "purple",
        description: "Fill the site with demonstration posts, pages, people and products, and remove them in one click." },
    Tool { key: "ecom", title: "E-commerce & Online Stores", href: "/ghost/ecom/", icon: "store", tint: "green",
        description: "Sell from your site: products and variants, collections, orders, customers, discounts, shipping and taxes." },
    Tool { key: "forms", title: "Forms", href: "/ghost/forms/", icon: "form", tint: "amber",
        description: "Build contact forms and read the responses visitors send." },
    Tool { key: "growth", title: "Growth", href: "/ghost/growth/", icon: "globe", tint: "slate",
        description: "Recommendations, signup forms, offers, Ghost Explore and the social web." },
    Tool { key: "history", title: "History", href: "/ghost/history/", icon: "history", tint: "slate",
        description: "Everything that changed on the site, who changed it and when." },
    Tool { key: "integrations", title: "Integrations", href: "/ghost/integrations/", icon: "integrations", tint: "pink",
        description: "Built-in apps, API keys for your own scripts, and webhooks." },
    Tool { key: "languages", title: "Languages", href: "/ghost/languages/", icon: "globe", tint: "teal",
        description: "Publish this site in more than one language, and keep the translations in step." },
    Tool { key: "lms", title: "LMS & Online Courses", href: "/ghost/lms/", icon: "school", tint: "purple",
        description: "Sell and teach courses: lessons, video, quizzes, assignments, drip, progress, reviews and certificates." },
    Tool { key: "machine-payments", title: "Machine payments", href: "/ghost/machine-payments/", icon: "integrations", tint: "amber",
        description: "Let readers and AI agents pay per article, by card or stablecoin." },
    Tool { key: "media", title: "Media", href: "/ghost/media/", icon: "library", tint: "pink",
        description: "Upload and manage every image, video and file on your site." },
    Tool { key: "memberships", title: "Memberships", href: "/ghost/memberships/", icon: "group", tint: "purple",
        description: "Members, tiers, offers and what is stopping the next person joining or paying." },
    Tool { key: "migrations", title: "Migrations", href: "/ghost/migrations/", icon: "import", tint: "blue",
        description: "Bring posts, pages, tags and members in from Ghost, WordPress, Substack and more." },
    Tool { key: "navigations", title: "Navigations", href: "/ghost/navigations/", icon: "menu", tint: "red",
        description: "The primary and secondary menus your theme shows." },
    Tool { key: "newsletters", title: "Newsletters", href: "/ghost/newsletters/", icon: "send", tint: "amber",
        description: "What goes out, who gets it, how it was opened, and what stopped it." },
    Tool { key: "booking", title: "Booking & Appointment Scheduling", href: "/ghost/booking/", icon: "calendar", tint: "teal",
        description: "Take appointments and class bookings online: services, staff hours, time off, payments, reminders and a calendar." },
    Tool { key: "builder", title: "Page Builder & Visual Editor", href: "/ghost/builder/", icon: "layout", tint: "pink",
        description: "Design any page visually: drag-and-drop sections, columns and widgets, inside your theme." },
    // Owner only, and never grantable — see `crate::permissions::UNGRANTABLE`.
    Tool { key: "permissions", title: "Permissions", href: "/ghost/permissions/", icon: "person", tint: "slate",
        description: "What each staff role may reach: tools, settings and content." },
    Tool { key: "podcast", title: "Podcast", href: "/ghost/podcast/", icon: "mic", tint: "purple",
        description: "Host a podcast: shows, episodes, an Apple- and Spotify-ready feed, player, chapters, transcripts, members-only episodes and download stats." },
    Tool { key: "popups", title: "Popups & Lead Generation", href: "/ghost/popups/", icon: "panel-show", tint: "pink",
        description: "Modals, slide-ins and bars that ask readers for an address." },
    Tool { key: "portfolio", title: "Portfolio", href: "/ghost/portfolio/", icon: "work", tint: "blue",
        description: "Show your work: case studies, galleries, services, clients, testimonials, a résumé or a team, and a project-brief form." },
    Tool { key: "redirects", title: "Redirects", href: "/ghost/redirects/", icon: "link", tint: "blue",
        description: "Send old addresses to new ones, rule by rule, and test any URL." },
    Tool { key: "routes", title: "Routes", href: "/ghost/routes/", icon: "layout", tint: "green",
        description: "How post, tag and author URLs are built, and what each address renders." },
    Tool { key: "search", title: "Search and Filtering", href: "/ghost/search/", icon: "search", tint: "pink",
        description: "Choose the site's search engine, what it can find, and how results are ranked." },
    Tool { key: "security", title: "Security", href: "/ghost/security/", icon: "lock", tint: "red",
        description: "See what is holding this site shut, who is signed in, and what is not protected." },
    Tool { key: "seo", title: "SEO", href: "/ghost/seo/", icon: "trending", tint: "blue",
        description: "See what search engines read on this site, and what is stopping them." },
    // Two halves under one card since 2026-11-21: the social web (the site as
    // its own fediverse account) and social media (accounts it posts to).
    Tool { key: "social", title: "Social", href: "/ghost/social/", icon: "share", tint: "teal",
        description: "The fediverse, and your accounts on X, Bluesky, LinkedIn and more: sharing and share buttons." },
    Tool { key: "staff", title: "Staffs", href: "/ghost/staffs/", icon: "person", tint: "teal",
        description: "The people who can sign in, their roles and invitations." },
    Tool { key: "workflows", title: "Workflow & Editorial Management", href: "/ghost/workflows/", icon: "flow", tint: "teal",
        description: "An editorial board for drafts: stages, assignments, due dates, notes and approval before publishing." },
];

pub(crate) fn tool(key: &str) -> &'static Tool {
    TOOLS.iter().find(|t| t.key == key).expect("known tool")
}

/// A tool page's header: back to Tools, the title with its one-line purpose,
/// any extra actions, and Save when the page has a form.
pub(crate) fn tool_head(key: &str, form_id: Option<&str>, extra: &str) -> String {
    tool_head_back(key, "/ghost/tools/", form_id, extra)
}

/// `tool_head` for a screen one level inside a tool — a popup's editor, say —
/// whose back arrow belongs to the tool's own list rather than to the Tools
/// hub. Jumping two levels up from an editor loses the list the person came
/// from, which is the one place they almost certainly want to go back to.
pub(crate) fn tool_head_back(key: &str, back_href: &str, form_id: Option<&str>, extra: &str) -> String {
    let t = tool(key);
    let save = form_id
        .map(|id| format!(r#"<button class="btn" type="submit" form="{id}">{check} Save</button>"#, id = esc(id), check = svg("check")))
        .unwrap_or_default();
    format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="{back_href}" title="{back_title}">{back}</a>
<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions">{extra}{save}</div></header>"#,
        back_href = esc(back_href),
        back_title = if back_href == "/ghost/tools/" { "Back to tools".to_string() } else { format!("Back to {}", t.title) },
        back = svg("chevron"),
        title = esc(t.title),
        sub = esc(t.description),
    )
}

pub(crate) fn tool_flash(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("{path}?{encoded}")).into_response()
}

pub(crate) fn tool_saved(state: &AppState, path: &str, message: &str) -> Response {
    fire_site_changed(state);
    tool_flash(path, "ok", message)
}

/// The upload handlers also serve the Admin API, which wants JSON rather
/// than a redirect to an HTML page.
pub(crate) fn tool_flash_or_json(headers: &HeaderMap, path: &str, kind: &str, message: &str) -> Response {
    if !wants_json(headers) {
        return tool_flash(path, kind, message);
    }
    if kind == "error" {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
        )
            .into_response()
    } else {
        axum::Json(serde_json::json!({"meta":{"message":message}})).into_response()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// the hub
// ---------------------------------------------------------------------------

/// One line of live state per tool, so the hub says what is set up before
/// anything is opened.
async fn tool_states(state: &AppState) -> HashMap<&'static str, (bool, String)> {
    let mut out = HashMap::new();

    let primary = nav_items_from_setting(state.settings.get("navigation")).len();
    let secondary = nav_items_from_setting(state.settings.get("secondary_navigation")).len();
    out.insert("navigations", (primary + secondary > 0, format!("{primary} primary · {secondary} secondary")));

    let head = !state.settings.get_str("codeinjection_head").unwrap_or_default().trim().is_empty();
    let foot = !state.settings.get_str("codeinjection_foot").unwrap_or_default().trim().is_empty();
    let ci = match (head, foot) {
        (true, true) => "Head and foot",
        (true, false) => "Head only",
        (false, true) => "Foot only",
        (false, false) => "Nothing injected",
    };
    out.insert("code-injection", (head || foot, ci.to_string()));

    let content = state.settings.get_str("announcement_content").unwrap_or_default();
    let audiences: Vec<&str> = announcement_visibility_set(state.settings.get("announcement_visibility"))
        .iter()
        .filter_map(|v| ANNOUNCEMENT_VISIBILITIES.iter().find(|(k, _)| k == v).map(|(_, l)| *l))
        .collect();
    let ann = if content.trim().is_empty() {
        (false, "Off".to_string())
    } else if audiences.is_empty() {
        (false, "Written, but shown to nobody".to_string())
    } else {
        (true, format!("Showing to {}", audiences.join(", ").to_lowercase()))
    };
    out.insert("announcement", ann);

    let rules = state.redirects.read().await.rules().len();
    out.insert("redirects", (rules > 0, if rules == 0 { "No rules yet".into() } else { format!("{} in force", plural(rules, "rule", "rules")) }));

    let routes = load_route_settings(state).await.0;
    out.insert("routes", (true, match &routes {
        Ok(s) => format!("{} · {}", plural(s.collections.len(), "collection", "collections"), plural(s.routes.len(), "custom route", "custom routes")),
        Err(_) => "routes.yaml cannot be read".into(),
    }));

    let apps = [
        vuuut_api::slack::SlackConfig::from_settings(&state.settings).is_some(),
        state.settings.get_bool("unsplash"),
    ]
    .iter()
    .filter(|on| **on)
    .count();
    let custom = repos::integrations::list_custom(&state.db).await.unwrap_or_default().len();
    out.insert("integrations", (apps + custom > 0, format!("{} · {}", plural(custom, "custom integration", "custom integrations"), plural(apps, "app on", "apps on"))));

    let kept = vuuut_api::backups::list(state).await;
    let schedule = vuuut_api::backups::Schedule::from_settings(state);
    let latest = kept.first().map(|b| format!(" · last {}", vuuut_api::backups::local_time(state, b.taken_at).to_lowercase())).unwrap_or_default();
    out.insert("backups", (
        schedule != vuuut_api::backups::Schedule::Off,
        match schedule {
            vuuut_api::backups::Schedule::Off if kept.is_empty() => "No backups yet · automatic off".to_string(),
            vuuut_api::backups::Schedule::Off => format!("{} of {} kept · automatic off{latest}", kept.len(), vuuut_api::backups::MAX_BACKUPS),
            s => format!("{} · {} of {} kept{latest}", s.as_str()[..1].to_uppercase() + &s.as_str()[1..], kept.len(), vuuut_api::backups::MAX_BACKUPS),
        },
    ));
    out.insert("migrations", (false, "Imports add content; nothing is replaced".into()));

    // The tool's own two countable things. Recommendations is the switch the
    // page leads with, so it decides the dot; offers are counted beside it
    // because an offer is the other thing this screen creates.
    let recommendations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recommendations")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let offers = repos::offers::list(&state.db).await.unwrap_or_default().len();
    let rec_on = state.settings.get_bool("recommendations_enabled");
    out.insert("growth", (
        rec_on,
        format!(
            "{} · {}",
            if rec_on { plural(recommendations as usize, "recommendation", "recommendations") } else { "Recommendations off".into() },
            plural(offers, "offer", "offers"),
        ),
    ));

    let staff = repos::staff::list_staff(&state.db).await.unwrap_or_default();
    let active = staff.iter().filter(|s| s.status.as_deref() != Some("inactive")).count();
    out.insert("staff", (true, plural(active, "person", "people")));

    let mp_on = state.settings.get_bool("machine_payments_enabled");
    out.insert("machine-payments", (mp_on, if mp_on { "Taking payments".into() } else { "Off".into() }));

    let comments = state.settings.get_str("comments_enabled").unwrap_or_else(|| "off".into());
    out.insert("comments", match comments.as_str() {
        "all" => (true, "Open to everyone".to_string()),
        "paid" => (true, "Paid members only".to_string()),
        _ => (false, "Off".to_string()),
    });

    // The system switch, not a count: an automation that exists while the
    // system is off is not running, and the card should say the thing that
    // decides whether anything happens.
    let automations_on = state.settings.get_str("automations_enabled").as_deref() != Some("false");
    out.insert("automations", (automations_on, if automations_on { "Running".into() } else { "Paused".into() }));

    // How many roles the Owner has moved off the defaults. "Default access"
    // is the honest reading of an untouched matrix.
    let changed = vuuut_core::permissions::read(state.settings.get(vuuut_core::permissions::SETTING_KEY).as_ref())
        .iter()
        .filter(|(role, grants)| **grants != vuuut_core::permissions::defaults(role))
        .count();
    out.insert("permissions", match changed {
        0 => (false, "Default access".to_string()),
        n => (true, format!("{} changed", plural(n, "role", "roles"))),
    });

    let social_on = state.settings.get_bool("social_web_enabled");
    out.insert("social", (social_on, if social_on { "Connected".into() } else { "Off".into() }));

    let week_ago = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(7)));
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM actions WHERE created_at >= ?")
        .bind(&week_ago)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    out.insert("history", (events > 0, format!("{} in the last 7 days", plural(events as usize, "change", "changes"))));
    out
}

pub(crate) async fn tools_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let mut states = tool_states(&state).await;
    states.extend(crate::tool_finder::former_module_states(&state).await);
    let inside = crate::tool_finder::things_inside(&state).await;
    // Only the tools this viewer may open — the middleware refuses the rest,
    // and a card that answers "Not allowed" is worse than no card.
    let grants = crate::permissions::grants_for(&state, actor.role);
    let visible: Vec<&Tool> = TOOLS
        .iter()
        .filter(|t| {
            // Permissions is Owner-only and deliberately outside the matrix,
            // so it is not offered to anybody else — see
            // `crate::permissions::UNGRANTABLE`.
            if crate::permissions::UNGRANTABLE.contains(&t.key) {
                return actor.role == Role::Owner;
            }
            grants.tools.access(t.key).may_read()
        })
        .collect();
    let cards: String = visible
        .iter()
        .map(|t| {
            let (on, line) = states.get(t.key).cloned().unwrap_or((false, String::new()));
            // What the search box matches inside this tool: its own headings
            // and fields, then what staff made in it. Escaped JSON in an
            // attribute, read by `tools-finder.js` and placed with
            // textContent.
            let mut phrases: Vec<String> = crate::tool_finder::content_of(t.key).iter().map(|p| p.to_string()).collect();
            phrases.extend(
                crate::settings::settings_entries(false, Some(t.key))
                    .into_iter()
                    .filter(|entry| grants.settings.access(entry.group).may_read())
                    .map(|entry| entry.title),
            );
            if let Some(made) = inside.get(t.key) {
                phrases.extend(made.iter().cloned());
            }
            format!(
                r#"<a class="tool-card" href="{href}" data-tool="{key}" data-find="{find}">
<div class="tool-top"><span class="ic tint-{tint}">{icon}</span><span class="tool-go" aria-hidden="true">{arrow}</span></div>
<div class="tool-text"><h2>{title}</h2><p>{description}</p></div>
<span class="tool-found" hidden></span>
<span class="tool-state{cls}"><i></i>{line}</span>
</a>"#,
                href = t.href,
                key = esc(t.key),
                find = esc(&serde_json::Value::from(phrases).to_string()),
                tint = t.tint,
                icon = svg(t.icon),
                arrow = svg("chevron"),
                title = esc(t.title),
                description = esc(t.description),
                cls = if on { " on" } else { "" },
                line = esc(&line),
            )
        })
        .collect();
    page_for_role(actor.role, "Tools", "tools", Some(&actor.user), &format!(
        r#"<header class="page-head rise"><div><h1>Tools</h1>
<p class="sub">Everything that runs this publication — its audience, its pages, its reports and the people behind it.</p></div></header>
<div class="tool-finder rise" style="--i:1" role="search">
<span class="tool-finder-ic" aria-hidden="true">{search}</span>
<input type="search" id="tool-find" placeholder="Search {n} tools and everything in them" aria-label="Search tools" autocomplete="off" spellcheck="false" aria-controls="tool-grid">
<span class="tool-finder-count" id="tool-find-count" role="status" aria-live="polite"></span>
</div>
<div class="tool-grid rise" style="--i:2" id="tool-grid">{cards}</div>
<div class="card padded tool-none rise" id="tool-none" hidden>
<p><b>No tool matches “<span id="tool-none-q"></span>”.</b></p>
<p class="muted">The search box in the sidebar also looks through posts, pages, members and every setting.</p>
<button class="btn tonal" type="button" data-omni-open>{search} Search everything</button>
</div>
<noscript><style>.tool-finder{{display:none}}</style></noscript>
<script src="{assets}/tools-finder.js?v={hash}" defer></script>"#,
        search = svg("search"),
        n = visible.len(),
        assets = ASSETS,
        hash = finder_script_hash(),
    ), true).into_response()
}

fn finder_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/tools-finder.js")));
    &HASH
}

// ---------------------------------------------------------------------------
// redirects
// ---------------------------------------------------------------------------

/// One rule as the editor holds it: old address, new address, 301 or not.
type Rule = (String, String, bool);

/// Characters that make a `from` a pattern rather than a plain path. A dot
/// is left out: `/feed.xml` is a path to anyone reading it.
fn is_pattern(from: &str) -> bool {
    from.chars().any(|c| "^$*+?()[]{}|\\".contains(c))
}

fn redirect_row(i: Option<usize>, rule: &Rule, hit: bool) -> String {
    let (from, to, permanent) = rule;
    let pattern = is_pattern(from);
    format!(
        r#"<div class="rx-row{hit}" data-row>
<span class="rx-num" data-num>{num}</span>
<label class="rx-cell rx-status-cell"><span class="rx-lbl">Type</span><select name="status" class="rx-in" aria-label="Type">
<option value="301"{p}>301 · Permanent</option><option value="302"{t}>302 · Temporary</option></select></label>
<label class="rx-cell"><span class="rx-lbl">When someone visits</span><input class="rx-in mono" name="from" value="{from}" aria-label="When someone visits" placeholder="/old-post/" data-from spellcheck="false" autocomplete="off"></label>
<span class="rx-arrow" aria-hidden="true">{arrow}</span>
<label class="rx-cell"><span class="rx-lbl">Send them to</span><input class="rx-in mono" name="to" value="{to}" aria-label="Send them to" placeholder="/new-post/ or https://…" spellcheck="false" autocomplete="off"></label>
<span class="chip rx-kind{pat}" data-kind title="Exact matches one address. A pattern is a regular expression and can match many; use $1, $2 in the destination for what it captured.">{kind}</span>
<div class="rx-tools">{tools}</div>
</div>"#,
        hit = if hit { " rx-hit" } else { "" },
        num = i.map(|i| (i + 1).to_string()).unwrap_or_default(),
        p = if *permanent { " selected" } else { "" },
        t = if *permanent { "" } else { " selected" },
        from = esc(from),
        to = esc(to),
        arrow = svg("chevron"),
        pat = if pattern { " pat" } else { "" },
        kind = if pattern { "Pattern" } else { "Exact" },
        tools = row_tools("rule"),
    )
}

fn row_tools(what: &str) -> String {
    format!(
        r#"<button class="icon-btn" type="button" data-move="up" title="Move up" aria-label="Move {what} up">{up}</button>
<button class="icon-btn" type="button" data-move="down" title="Move down" aria-label="Move {what} down">{down}</button>
<button class="icon-btn danger" type="button" data-remove-row title="Remove" aria-label="Remove {what}">{del}</button>"#,
        up = svg("up"),
        down = svg("down"),
        del = svg("delete"),
    )
}

async fn redirects_file_text(state: &AppState) -> Option<String> {
    tokio::fs::read_to_string(state.config.content_path().join("redirects.yaml")).await.ok()
}

fn redirects_html(rules: &[Rule], banner: &str, test: &str, file: Option<&str>, hit: Option<usize>) -> String {
    let rows: String = rules.iter().enumerate().map(|(i, r)| redirect_row(Some(i), r, hit == Some(i))).collect();
    let template = redirect_row(None, &(String::new(), String::new(), true), false);
    let file_view = match file {
        Some(text) if !text.trim().is_empty() => format!(
            r#"<details class="file-view"><summary>{eye} View redirects.yaml as it is on disk</summary><pre>{text}</pre></details>"#,
            eye = svg("eye"),
            text = esc(text),
        ),
        _ => r#"<p class="muted" style="margin:10px 0 0">There is no redirects.yaml yet; saving rules here writes one.</p>"#.into(),
    };
    format!(
        r#"{head}{banner}
<div class="section">Test an address</div>
<div class="card padded rise" style="--i:1">
<form method="get" action="/ghost/redirects/" class="inline-form">
<label class="field" style="flex:1;min-width:220px"><input name="test" value="{test_value}" placeholder="/an-old-post/" spellcheck="false" class="code"><span>Address on this site</span></label>
<button class="btn tonal" type="submit">{search} Test</button></form>
{test}
<p class="muted" style="margin:10px 0 0">Tests the saved rules. Redirects run before anything else on the site, so a rule here wins over a post at the same address.</p>
</div>

<div class="section">Rules · tried top to bottom, first match wins</div>
<form method="post" action="/ghost/redirects/" id="redirects-form" class="rise" style="--i:2">
<div class="card rx-card">
<div class="rx-list" id="rx-list">{rows}
<div class="rx-empty"><span class="ic tint-blue">{link}</span><div><b>No redirects yet</b><span>Add a rule below, or upload a redirects file.</span></div></div>
</div>
<div class="rx-foot"><button class="btn tonal small" type="button" data-add-row="rx-template" data-target="rx-list">{add} Add rule</button>
<span class="muted">Leave a row empty to drop it. Nothing changes on the site until you save.</span></div>
</div>
</form>
<template id="rx-template">{template}</template>

<div class="section">File</div>
<div class="card padded rise" style="--i:3">
<div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<a class="btn tonal small" href="/ghost/redirects/download/">{download} Download redirects.yaml</a>
<form method="post" action="/ghost/redirects/upload/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:0" data-confirm="Replace every rule|The uploaded file replaces all the rules above.">
<input type="file" name="file" accept=".yaml,.yml,.json" required class="filefield">
<button class="btn tonal small" type="submit">{upload} Upload &amp; replace</button></form></div>
<p class="muted" style="margin:10px 0 0">Ghost's format — <code>301:</code> and <code>302:</code> groups, or a <code>redirects.json</code> list. The file is checked before it replaces anything.</p>
{file_view}
</div>
<script src="{assets}/tools-editor.js" defer></script>"#,
        head = tool_head("redirects", Some("redirects-form"), ""),
        banner = banner,
        test_value = "",
        search = svg("search"),
        test = test,
        rows = rows,
        link = svg("link"),
        add = svg("add"),
        template = template,
        download = svg("import"),
        upload = svg("upload"),
        file_view = file_view,
        assets = ASSETS,
    )
}

/// The tester's verdict for one address against the saved rules.
fn redirect_test_html(redirects: &vuuut_routing::redirects::Redirects, raw: &str) -> (String, Option<usize>) {
    let (path, query) = split_test_address(raw);
    match redirects.match_rule(&path, query.as_deref()) {
        Some((i, target, permanent)) => (
            format!(
                r#"<div class="test-result hit"><span class="ic tint-green">{check}</span><div><b>Rule {n} catches <code>{path}</code></b>
<span>Visitors are sent to <code>{target}</code> with a {code} ({kind}). The rule is highlighted below.</span></div></div>"#,
                check = svg("check"),
                n = i + 1,
                path = esc(&path),
                target = esc(&target),
                code = if permanent { 301 } else { 302 },
                kind = if permanent { "permanent — browsers and search engines remember it" } else { "temporary — nothing is remembered" },
            ),
            Some(i),
        ),
        None => (
            format!(
                r#"<div class="test-result"><span class="ic tint-slate">{flag}</span><div><b>No rule catches <code>{path}</code></b>
<span>The site serves it as usual. <a href="/ghost/routes/?test={q}">See what it renders in Routes</a>.</span></div></div>"#,
                flag = svg("flag"),
                path = esc(&path),
                q = esc(&form_urlencoded::byte_serialize(path.as_bytes()).collect::<String>()),
            ),
            None,
        ),
    }
}

/// Accept a full URL as readily as a path: the part after the host is what
/// the rules see.
fn split_test_address(raw: &str) -> (String, Option<String>) {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("http://").or_else(|| s.strip_prefix("https://")) {
        s = rest.find('/').map(|i| &rest[i..]).unwrap_or("/");
    }
    let (path, query) = match s.split_once('?') {
        Some((p, q)) => (p, Some(q.to_string())),
        None => (s, None),
    };
    let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    (path, query)
}

fn test_value_input(html: String, value: &str) -> String {
    html.replacen(
        r#"<input name="test" value="""#,
        &format!(r#"<input name="test" value="{}""#, esc(value)),
        1,
    )
}

pub(crate) async fn redirects_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let redirects = state.redirects.read().await.clone();
    let rules: Vec<Rule> = redirects.rules().iter().map(|r| (r.from_raw.clone(), r.to.clone(), r.permanent)).collect();
    let tested = q.get("test").map(|t| t.trim()).filter(|t| !t.is_empty());
    let (test, hit) = match tested {
        Some(t) => redirect_test_html(&redirects, t),
        None => (String::new(), None),
    };
    let file = redirects_file_text(&state).await;
    let html = redirects_html(&rules, &flash_banner(&q), &test, file.as_deref(), hit);
    let html = test_value_input(html, tested.unwrap_or(""));
    page_for_role(actor.role, "Redirects", "tools", Some(&actor.user), &html, true).into_response()
}

/// Parse, write and swap in a redirects file. Nothing is written unless it parses.
async fn apply_redirects(state: &AppState, text: &str) -> Result<usize, String> {
    let parsed = vuuut_routing::redirects::parse_redirects_yaml(text).map_err(|e| format!("Not a valid redirects file: {e}"))?;
    let path = state.config.content_path().join("redirects.yaml");
    tokio::fs::write(&path, text).await.map_err(|e| format!("Could not write redirects.yaml: {e}"))?;
    let count = parsed.rules().len();
    *state.redirects.write().await = Arc::new(parsed);
    fire_site_changed(state);
    Ok(count)
}

pub(crate) async fn redirects_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    // Each row posts status, from, to — in that order — so a status starts a row.
    let mut rows: Vec<Rule> = Vec::new();
    for (key, value) in pairs {
        match key.as_str() {
            "status" => rows.push((String::new(), String::new(), value != "302")),
            "from" => if let Some(r) = rows.last_mut() { r.0 = value.trim().to_string() },
            "to" => if let Some(r) = rows.last_mut() { r.1 = value.trim().to_string() },
            _ => {}
        }
    }
    rows.retain(|(from, to, _)| !(from.is_empty() && to.is_empty()));

    let mut errors = Vec::new();
    for (i, (from, to, _)) in rows.iter().enumerate() {
        if let Err(e) = vuuut_routing::redirects::validate_from(from) {
            errors.push(format!("Rule {}: {e}", i + 1));
        }
        if to.is_empty() {
            errors.push(format!("Rule {}: where should “{from}” go?", i + 1));
        }
    }
    let mut seen = HashSet::new();
    for (i, (from, _, _)) in rows.iter().enumerate() {
        if !from.is_empty() && !seen.insert(from.clone()) {
            errors.push(format!("Rule {}: “{from}” is already handled by an earlier rule, so this one would never run.", i + 1));
        }
    }

    let result = if errors.is_empty() {
        apply_redirects(&state, &vuuut_routing::redirects::to_yaml(&rows)).await.map_err(|e| vec![e])
    } else {
        Err(errors)
    };
    match result {
        Ok(n) => tool_flash("/ghost/redirects/", "ok", &format!("Redirects saved — {} in force.", plural(n, "rule", "rules"))),
        Err(errors) => {
            // Re-render what was posted, so a typo does not cost the other rows.
            let banner = format!(
                r#"<div class="banner error rise" style="--i:1">{warn}<div class="msg"><b>Nothing was saved.</b><ul style="margin:6px 0 0;padding-left:18px">{items}</ul></div></div>"#,
                warn = svg("flag"),
                items = errors.iter().map(|e| format!("<li>{}</li>", esc(e))).collect::<String>(),
            );
            let file = redirects_file_text(&state).await;
            let html = test_value_input(redirects_html(&rows, &banner, "", file.as_deref(), None), "");
            (StatusCode::UNPROCESSABLE_ENTITY, page_for_role(actor.role, "Redirects", "tools", Some(&actor.user), &html, true)).into_response()
        }
    }
}

/// The current redirects.yaml. A site that has never had any gets the
/// neutral document rather than an error.
pub(crate) async fn redirects_download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let body = redirects_file_text(&state).await.unwrap_or_else(|| "# No redirects configured yet.\n".to_string());
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"redirects.yaml\""),
        ],
        body,
    )
        .into_response()
}

pub(crate) async fn redirects_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let back = "/ghost/redirects/";
    while let Ok(Some(field)) = multipart.next_field().await {
        if !matches!(field.name(), Some("file" | "redirects")) {
            continue;
        }
        let text = match field.text().await {
            Ok(t) => t,
            Err(e) => return tool_flash_or_json(&headers, back, "error", &format!("Upload failed: {e}")),
        };
        return match apply_redirects(&state, &text).await {
            Ok(n) => tool_flash_or_json(&headers, back, "ok", &format!("Redirects uploaded — {} in force.", plural(n, "rule", "rules"))),
            Err(e) => tool_flash_or_json(&headers, back, "error", &e),
        };
    }
    tool_flash_or_json(&headers, back, "error", "No file was uploaded.")
}

// ---------------------------------------------------------------------------
// routes
// ---------------------------------------------------------------------------

/// The routes file as text (the default when there is none), and its parse.
async fn load_route_settings(state: &AppState) -> (Result<RouteSettings, String>, String, bool) {
    let (text, on_disk) = match tokio::fs::read_to_string(state.config.content_path().join("routes.yaml")).await {
        Ok(t) => (t, true),
        Err(_) => (vuuut_routing::DEFAULT_ROUTES_YAML.to_string(), false),
    };
    (vuuut_routing::routes_yaml::parse_routes_yaml(&text).map_err(|e| e.message()), text, on_disk)
}

/// A YAML value as one line an input can hold: strings bare, anything
/// structured as JSON (which YAML reads back).
fn yaml_inline(v: &Option<serde_yaml::Value>) -> String {
    match v {
        None | Some(serde_yaml::Value::Null) => String::new(),
        Some(serde_yaml::Value::String(s)) => s.clone(),
        Some(serde_yaml::Value::Number(n)) => n.to_string(),
        Some(serde_yaml::Value::Bool(b)) => b.to_string(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn yaml_from_input(s: &str) -> Option<serde_yaml::Value> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Some(serde_yaml::from_str(s).unwrap_or_else(|_| serde_yaml::Value::String(s.to_string())))
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn split_templates(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

fn rt_field(name: &str, label: &str, value: &str, placeholder: &str, extra: &str) -> String {
    format!(
        r#"<label class="field"><input name="{name}" value="{value}" placeholder="{placeholder}" spellcheck="false" autocomplete="off" class="code"{extra}><span>{label}</span></label>"#,
        name = name,
        value = esc(value),
        placeholder = esc(placeholder),
        label = esc(label),
        extra = extra,
    )
}

/// Where the published posts land under the current routes: per
/// collection, how many and a couple of real addresses — and how many land
/// nowhere at all.
struct Landing {
    by_collection: HashMap<String, (usize, Vec<String>)>,
    orphans: usize,
    total: usize,
}

async fn post_landing(state: &AppState, tree: &RouterTree) -> Landing {
    let posts: Vec<_> = repos::posts::all_published(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.post_type == "post")
        .take(2000)
        .collect();
    let rel = repos::posts::load_relations(&state.db, &posts).await.unwrap_or_default();
    let mut landing = Landing { by_collection: HashMap::new(), orphans: 0, total: posts.len() };
    for post in &posts {
        let data = vuuut_core::models::post_to_json(post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
        match tree.permalink_for(&data) {
            Some((name, permalink)) => {
                let entry = landing.by_collection.entry(name).or_default();
                entry.0 += 1;
                if entry.1.len() < 2 {
                    entry.1.push(vuuut_routing::tree::build_permalink(&permalink, &data));
                }
            }
            None => landing.orphans += 1,
        }
    }
    landing
}

fn collection_card(i: Option<usize>, c: &CollectionEntry, landing: Option<&Landing>) -> String {
    let live = match (i, landing) {
        (Some(_), Some(l)) => {
            let name = vuuut_routing::tree::route_name(&c.path);
            match l.by_collection.get(&name) {
                Some((n, examples)) => format!(
                    r#"<p class="rt-live on"><i></i>{count} land here{eg}</p>"#,
                    count = plural(*n, "published post", "published posts"),
                    eg = if examples.is_empty() {
                        String::new()
                    } else {
                        format!(" · e.g. {}", examples.iter().map(|e| format!("<code>{}</code>", esc(e))).collect::<Vec<_>>().join(", "))
                    },
                ),
                None => r#"<p class="rt-live"><i></i>No published post lands here — an earlier collection takes them, or none match the filter.</p>"#.into(),
            }
        }
        _ => r#"<p class="rt-live"><i></i>New — save to see which posts land here.</p>"#.into(),
    };
    format!(
        r#"<div class="rt-card" data-row>
<input type="hidden" name="c_start" value="1">
<div class="rt-head"><span class="rt-num" data-num>{num}</span><span class="chip tonal">Collection</span>
<span class="rt-title">{title}</span><div class="rx-tools">{tools}</div></div>
<div class="rt-grid">
{path}{permalink}{filter}{templates}{order}{limit}
<label class="field fixed"><select name="c_rss"><option value="on"{rss_on}>On</option><option value="off"{rss_off}>Off</option></select><span>RSS feed</span>{car}</label>
{data}
</div>
{live}
</div>"#,
        num = i.map(|i| (i + 1).to_string()).unwrap_or_default(),
        title = if c.path.is_empty() {
            "Lists posts at an address, and gives each post its own".to_string()
        } else {
            format!("Lists at <code>{}</code> · each post at <code>{}</code>", esc(&c.path), esc(&c.permalink))
        },
        tools = row_tools("collection"),
        path = rt_field("c_path", "Listed at", &c.path, "/blog/", " required"),
        permalink = rt_field("c_permalink", "Each post's address", &c.permalink, "/blog/{slug}/", " required"),
        filter = rt_field("c_filter", "Which posts (filter)", c.filter.as_deref().unwrap_or(""), "tag:blog — empty takes the rest", ""),
        templates = rt_field("c_templates", "Template", &c.templates.join(", "), "index — theme default when empty", ""),
        order = rt_field("c_order", "Order", c.order.as_deref().unwrap_or(""), "published_at desc", ""),
        limit = rt_field("c_limit", "Posts per page", &yaml_inline(&c.limit), "theme default", ""),
        rss_on = if c.rss != Some(false) { " selected" } else { "" },
        rss_off = if c.rss == Some(false) { " selected" } else { "" },
        car = SELECT_CARET,
        data = rt_field("c_data", "Data", &yaml_inline(&c.data), "page.blog", ""),
        live = live,
    )
}

fn route_card(i: Option<usize>, r: &RouteEntry) -> String {
    let channel = r.filter.is_some();
    format!(
        r#"<div class="rt-card" data-row>
<input type="hidden" name="r_start" value="1">
<div class="rt-head"><span class="rt-num" data-num>{num}</span><span class="chip {kind_cls}" data-route-kind>{kind}</span>
<span class="rt-title">{title}</span><div class="rx-tools">{tools}</div></div>
<div class="rt-grid">
{path}{templates}{filter}{order}{limit}{data}{ctype}
</div>
<p class="rt-live"><i></i>{explain}</p>
</div>"#,
        num = i.map(|i| (i + 1).to_string()).unwrap_or_default(),
        kind_cls = if channel { "published" } else { "tonal" },
        kind = if channel { "Channel" } else { "Template page" },
        title = if r.path.is_empty() { "A fixed address".to_string() } else { format!("<code>{}</code>", esc(&r.path)) },
        tools = row_tools("route"),
        path = rt_field("r_path", "Address", &r.path, "/about/team/", " required"),
        templates = rt_field("r_templates", "Template", &r.templates.join(", "), "team", ""),
        filter = rt_field("r_filter", "Filter — makes it a channel", r.filter.as_deref().unwrap_or(""), "tag:podcast", " data-route-filter"),
        order = rt_field("r_order", "Order", r.order.as_deref().unwrap_or(""), "published_at desc", ""),
        limit = rt_field("r_limit", "Posts per page", &yaml_inline(&r.limit), "theme default", ""),
        data = rt_field("r_data", "Data", &yaml_inline(&r.data), "page.team", ""),
        ctype = rt_field("r_content_type", "Content type", r.content_type.as_deref().unwrap_or(""), "text/html", ""),
        explain = if channel {
            "A channel lists the posts matching its filter, paginated — without taking them out of their collection."
        } else {
            "Renders the template at exactly this address. Give it Data to hand the template a post, page or tag."
        },
    )
}

fn describe_templates(t: &[String]) -> String {
    if t.is_empty() {
        "the theme's default template".into()
    } else {
        format!("template <code>{}</code>", t.iter().map(|x| esc(x)).collect::<Vec<_>>().join("</code>, then <code>"))
    }
}

/// What the router does with one address, in words.
async fn route_test_html(state: &AppState, tree: &RouterTree, raw: &str) -> String {
    let (path, query) = split_test_address(raw);
    let redirects = state.redirects.read().await.clone();
    if let Some((i, target, _)) = redirects.match_rule(&path, query.as_deref()) {
        return format!(
            r#"<div class="test-result"><span class="ic tint-blue">{link}</span><div><b>Redirected before routing</b>
<span><a href="/ghost/redirects/?test={q}">Redirect rule {n}</a> sends <code>{path}</code> to <code>{target}</code>, so no route is consulted.</span></div></div>"#,
            link = svg("link"),
            q = esc(&form_urlencoded::byte_serialize(path.as_bytes()).collect::<String>()),
            n = i + 1,
            path = esc(&path),
            target = esc(&target),
        );
    }
    let normalised = if path.ends_with('/') || path.contains('.') { path.clone() } else { format!("{path}/") };
    let found = |kind: &str, detail: String| {
        format!(
            r#"<div class="test-result hit"><span class="ic tint-green">{check}</span><div><b>{kind}</b><span>{detail}</span></div></div>"#,
            check = svg("check"),
        )
    };
    match tree.match_path(&normalised) {
        Some(RouteMatch::Preview { .. }) => found("Post preview", "A preview link for a draft — not a public address.".into()),
        Some(RouteMatch::Channel { path: p, templates, page, .. }) => found(
            "Channel",
            format!("Custom route <code>{}</code> lists the posts matching its filter (page {page}), rendered with {}.", esc(&p), describe_templates(&templates)),
        ),
        Some(RouteMatch::Custom { templates, .. }) => found(
            "Template page",
            format!("A custom route renders {} at exactly this address.", describe_templates(&templates)),
        ),
        Some(RouteMatch::CollectionIndex { path: p, templates, page, .. }) => found(
            "Collection index",
            format!("The list of posts in collection <code>{}</code>, page {page}, rendered with {}.", esc(&p), describe_templates(&templates)),
        ),
        Some(RouteMatch::CollectionEntry { collection, permalink, slug }) => {
            let post = repos::posts::find_by_slug(&state.db, &slug, "post", true).await.ok().flatten();
            let what = match post {
                Some(p) => format!("the post “{}”", esc(p.title.as_deref().unwrap_or("Untitled"))),
                None => "a post — but no published post has that slug, so visitors get a 404".into(),
            };
            found(
                "Post",
                format!("Matches <code>{}</code> in collection <b>{}</b> with slug <code>{}</code>: {what}.", esc(&permalink), esc(&collection), esc(&slug)),
            )
        }
        Some(RouteMatch::Rss { base, .. }) => found("RSS feed", format!("The feed for <code>{}</code>.", esc(&base))),
        Some(RouteMatch::StaticPage { slug }) => {
            let page = repos::posts::find_by_slug(&state.db, &slug, "page", true).await.ok().flatten();
            match page {
                Some(p) => found("Page", format!("The page “{}”. Pages always live at <code>/{{slug}}/</code>.", esc(p.title.as_deref().unwrap_or("Untitled")))),
                None => format!(
                    r#"<div class="test-result"><span class="ic tint-amber">{flag}</span><div><b>Not found</b><span>This is where a page with slug <code>{}</code> would live, but there is none — visitors get a 404.</span></div></div>"#,
                    esc(&slug),
                    flag = svg("flag"),
                ),
            }
        }
        Some(RouteMatch::Taxonomy { kind, slug, permalink, page }) => found(
            match kind {
                vuuut_routing::tree::TaxonomyKind::Tag => "Tag archive",
                vuuut_routing::tree::TaxonomyKind::Author => "Author archive",
            },
            format!("Posts with slug <code>{}</code>, page {page}, from <code>{}</code>.", esc(&slug), esc(&permalink)),
        ),
        None => format!(
            r#"<div class="test-result"><span class="ic tint-amber">{flag}</span><div><b>Nothing routes <code>{}</code></b><span>Visitors get a 404.</span></div></div>"#,
            esc(&path),
            flag = svg("flag"),
        ),
    }
}

struct RoutesView<'a> {
    settings: &'a RouteSettings,
    banner: &'a str,
    test_value: &'a str,
    test: &'a str,
    file: &'a str,
    on_disk: bool,
    landing: Option<&'a Landing>,
}

fn routes_html(v: RoutesView) -> String {
    let s = v.settings;
    let collections: String = s.collections.iter().enumerate().map(|(i, c)| collection_card(Some(i), c, v.landing)).collect();
    let routes: String = s.routes.iter().enumerate().map(|(i, r)| route_card(Some(i), r)).collect();
    let blank_c = CollectionEntry { path: String::new(), permalink: String::new(), template: None, templates: vec![], filter: None, order: None, limit: None, data: None, rss: None };
    let blank_r = RouteEntry { path: String::new(), template: None, templates: vec![], filter: None, order: None, limit: None, data: None, content_type: None };
    let summary = match v.landing {
        Some(l) => {
            let orphans = if l.orphans > 0 {
                format!(r#" <span class="chip error">{} with no address</span>"#, plural(l.orphans, "post", "posts"))
            } else {
                String::new()
            };
            format!(r#"<p class="muted rise" style="--i:1;margin:0 0 4px">{} published.{orphans} Pages always live at <code>/{{slug}}/</code>; everything else follows the rules below.</p>"#, plural(l.total, "post", "posts"))
        }
        None => String::new(),
    };
    let tax = |name: &str, label: &str, value: &Option<String>, default: &str, ex: &str| -> String {
        format!(
            r#"<div class="rt-tax"><label class="switch-line"><input type="checkbox" name="{name}_on" value="1"{on}><span><b>{label}</b><span class="muted">{ex}</span></span></label>
{field}</div>"#,
            on = if value.is_some() { " checked" } else { "" },
            field = rt_field(name, "Archive address", value.as_deref().unwrap_or(default), default, ""),
        )
    };
    format!(
        r#"{head}{banner}{summary}
<div class="section">Test an address</div>
<div class="card padded rise" style="--i:1">
<form method="get" action="/ghost/routes/" class="inline-form">
<label class="field" style="flex:1;min-width:220px"><input name="test" value="{test_value}" placeholder="/blog/hello-world/" spellcheck="false" class="code"><span>Address on this site</span></label>
<button class="btn tonal" type="submit">{search} Test</button></form>
{test}
<p class="muted" style="margin:10px 0 0">Answers from the saved routes. Order of precedence: redirects, then custom routes, then collections, then pages, then tag and author archives.</p>
</div>

<form method="post" action="/ghost/routes/" id="routes-form">
<div class="section">Collections · a post belongs to the first one whose filter it matches</div>
<div class="rt-list rise" style="--i:2" id="rt-collections">{collections}
<div class="rx-empty"><span class="ic tint-amber">{flag}</span><div><b>No collections</b><span>Without one, posts have no address. Add one — <code>/</code> with <code>/{{slug}}/</code> is Ghost's default.</span></div></div>
</div>
<div class="rt-add"><button class="btn tonal small" type="button" data-add-row="rt-c-template" data-target="rt-collections">{add} Add collection</button></div>

<div class="section">Custom routes · checked before collections</div>
<div class="rt-list rise" style="--i:3" id="rt-routes">{routes}
<div class="rx-empty"><span class="ic tint-green">{layout}</span><div><b>No custom routes</b><span>Add one to render a template at a fixed address, or — with a filter — a channel of posts.</span></div></div>
</div>
<div class="rt-add"><button class="btn tonal small" type="button" data-add-row="rt-r-template" data-target="rt-routes">{add} Add route</button></div>

<div class="section">Taxonomies</div>
<div class="card padded rise" style="--i:4">
{tag}{author}
<p class="muted" style="margin:6px 0 0">Switch one off and its archives stop existing. The address must contain <code>{{slug}}</code>.</p>
</div>
</form>
<template id="rt-c-template">{blank_c}</template>
<template id="rt-r-template">{blank_r}</template>

<div class="section">File</div>
<div class="card padded rise" style="--i:5">
<div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<a class="btn tonal small" href="/ghost/routes/download/">{download} Download routes.yaml</a>
<form method="post" action="/ghost/routes/upload/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:0" data-confirm="Replace all routes|The uploaded file replaces everything above, and the theme is rebuilt against it.">
<input type="file" name="file" accept=".yaml,.yml" required class="filefield">
<button class="btn tonal small" type="submit">{upload} Upload &amp; replace</button></form></div>
<p class="muted" style="margin:10px 0 0">Saving here or uploading checks the routes first, then rebuilds routing and the active theme at once.</p>
<details class="file-view"><summary>{eye} View routes.yaml{disk}</summary><pre>{file}</pre></details>
</div>
<script src="{assets}/tools-editor.js" defer></script>"#,
        head = tool_head("routes", Some("routes-form"), ""),
        banner = v.banner,
        summary = summary,
        test_value = esc(v.test_value),
        search = svg("search"),
        test = v.test,
        collections = collections,
        routes = routes,
        flag = svg("flag"),
        layout = svg("layout"),
        add = svg("add"),
        tag = tax("tag", "Tag archives", &s.taxonomies.tag, "/tag/{slug}/", "A page per tag listing its posts"),
        author = tax("author", "Author archives", &s.taxonomies.author, "/author/{slug}/", "A page per author listing their posts"),
        blank_c = collection_card(None, &blank_c, None),
        blank_r = route_card(None, &blank_r),
        download = svg("import"),
        upload = svg("upload"),
        eye = svg("eye"),
        disk = if v.on_disk { "" } else { " (the built-in default — no file on disk yet)" },
        file = esc(v.file),
        assets = ASSETS,
    )
}

pub(crate) async fn routes_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (parsed, text, on_disk) = load_route_settings(&state).await;
    let tree = state.router.read().await.clone();
    let tested = q.get("test").map(|t| t.trim()).filter(|t| !t.is_empty());
    let test = match tested {
        Some(t) => route_test_html(&state, &tree, t).await,
        None => String::new(),
    };
    let mut banner = flash_banner(&q);
    let settings = match parsed {
        Ok(s) => s,
        Err(e) => {
            banner.push_str(&format!(
                r#"<div class="banner error rise" style="--i:1">{warn}routes.yaml on disk cannot be read ({}). The editor shows nothing from it; upload a corrected file below, or build the routes here and save.</div>"#,
                esc(&e),
                warn = svg("flag"),
            ));
            RouteSettings::default()
        }
    };
    let landing = post_landing(&state, &tree).await;
    let html = routes_html(RoutesView {
        settings: &settings,
        banner: &banner,
        test_value: tested.unwrap_or(""),
        test: &test,
        file: &text,
        on_disk,
        landing: Some(&landing),
    });
    page_for_role(actor.role, "Routes", "tools", Some(&actor.user), &html, true).into_response()
}

/// Check, write and swap in a routes file, then rebuild the theme against
/// it — a theme's resolvers and template choices come from routes. `Ok`
/// carries a warning when routing changed but the theme could not follow.
async fn apply_routes(state: &AppState, text: &str) -> Result<Option<String>, String> {
    let settings = vuuut_routing::routes_yaml::parse_routes_yaml(text).map_err(|e| format!("Not a valid routes file: {}", e.message()))?;
    let tree = RouterTree::build(&settings).map_err(|e| format!("These routes cannot be routed: {}", e.message()))?;
    let path = state.config.content_path().join("routes.yaml");
    tokio::fs::write(&path, text).await.map_err(|e| format!("Could not write routes.yaml: {e}"))?;
    *state.router.write().await = Arc::new(tree);
    let theme = vuuut_api::themes::active_theme_name(state);
    let warning = vuuut_api::themes::activate(state, &theme)
        .await
        .err()
        .map(|e| format!("Routes are in force, but the theme could not be rebuilt against them: {}", e.message()));
    fire_site_changed(state);
    Ok(warning)
}

/// Read the editor's form back into settings. Every card starts with a
/// `*_start` marker and its fields follow, so order — which decides which
/// collection a post belongs to — is kept exactly as posted.
fn routes_from_form(pairs: &[(String, String)]) -> RouteSettings {
    let mut s = RouteSettings::default();
    let (mut tag_on, mut author_on) = (false, false);
    let (mut tag, mut author) = (String::new(), String::new());
    for (key, value) in pairs {
        let v = value.as_str();
        match key.as_str() {
            "c_start" => s.collections.push(CollectionEntry { path: String::new(), permalink: String::new(), template: None, templates: vec![], filter: None, order: None, limit: None, data: None, rss: None }),
            "r_start" => s.routes.push(RouteEntry { path: String::new(), template: None, templates: vec![], filter: None, order: None, limit: None, data: None, content_type: None }),
            "tag_on" => tag_on = true,
            "author_on" => author_on = true,
            "tag" => tag = v.trim().to_string(),
            "author" => author = v.trim().to_string(),
            k if k.starts_with("c_") => {
                let Some(c) = s.collections.last_mut() else { continue };
                match k {
                    "c_path" => c.path = v.trim().to_string(),
                    "c_permalink" => c.permalink = v.trim().to_string(),
                    "c_filter" => c.filter = opt(v),
                    "c_templates" => c.templates = split_templates(v),
                    "c_order" => c.order = opt(v),
                    "c_limit" => c.limit = yaml_from_input(v),
                    "c_rss" => c.rss = (v == "off").then_some(false),
                    "c_data" => c.data = yaml_from_input(v),
                    _ => {}
                }
            }
            k if k.starts_with("r_") => {
                let Some(r) = s.routes.last_mut() else { continue };
                match k {
                    "r_path" => r.path = v.trim().to_string(),
                    "r_templates" => r.templates = split_templates(v),
                    "r_filter" => r.filter = opt(v),
                    "r_order" => r.order = opt(v),
                    "r_limit" => r.limit = yaml_from_input(v),
                    "r_data" => r.data = yaml_from_input(v),
                    "r_content_type" => r.content_type = opt(v),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // A card left completely empty is a row the editor added and never used.
    s.collections.retain(|c| !(c.path.is_empty() && c.permalink.is_empty()));
    s.routes.retain(|r| !r.path.is_empty() || !r.templates.is_empty() || r.filter.is_some());
    for c in &mut s.collections {
        c.template = c.templates.first().cloned();
    }
    for r in &mut s.routes {
        r.template = r.templates.first().cloned();
    }
    s.taxonomies.tag = (tag_on && !tag.is_empty()).then_some(tag);
    s.taxonomies.author = (author_on && !author.is_empty()).then_some(author);
    s
}

fn route_problems(s: &RouteSettings) -> Vec<String> {
    let mut errors = Vec::new();
    let slashed = |p: &str| p.starts_with('/') && p.ends_with('/');
    for (i, c) in s.collections.iter().enumerate() {
        let n = i + 1;
        if !slashed(&c.path) {
            errors.push(format!("Collection {n}: “{}” must start and end with a slash, like /blog/.", c.path));
        }
        if !slashed(&c.permalink) {
            errors.push(format!("Collection {n}: the post address must start and end with a slash, like /blog/{{slug}}/."));
        } else if !["{slug}", "{id}", "{uuid}"].iter().any(|p| c.permalink.contains(p)) {
            errors.push(format!("Collection {n}: the post address needs {{slug}} (or {{id}}) in it, or every post would share one address."));
        }
    }
    for (i, r) in s.routes.iter().enumerate() {
        if !slashed(&r.path) {
            errors.push(format!("Route {}: “{}” must start and end with a slash, like /about/team/.", i + 1, r.path));
        }
    }
    let mut seen = HashSet::new();
    for p in s.collections.iter().map(|c| &c.path).chain(s.routes.iter().map(|r| &r.path)) {
        if !p.is_empty() && !seen.insert(p.clone()) {
            errors.push(format!("“{p}” is used twice; only the first would ever be reached."));
        }
    }
    for (label, v) in [("Tag", &s.taxonomies.tag), ("Author", &s.taxonomies.author)] {
        if let Some(p) = v {
            if !p.contains("{slug}") || !slashed(p) {
                errors.push(format!("{label} archives: the address must start and end with a slash and contain {{slug}}."));
            }
        }
    }
    errors
}

pub(crate) async fn routes_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let settings = routes_from_form(&pairs);
    let mut errors = route_problems(&settings);
    let yaml = vuuut_routing::routes_yaml::to_yaml(&settings);
    let result = if errors.is_empty() {
        apply_routes(&state, &yaml).await
    } else {
        Err(String::new())
    };
    match result {
        Ok(None) => tool_flash("/ghost/routes/", "ok", "Routes saved — routing and the theme have been rebuilt."),
        Ok(Some(warning)) => tool_flash("/ghost/routes/", "error", &warning),
        Err(e) => {
            if !e.is_empty() {
                errors.push(e);
            }
            let banner = format!(
                r#"<div class="banner error rise" style="--i:1">{warn}<div class="msg"><b>Nothing was saved — the site is still on its previous routes.</b><ul style="margin:6px 0 0;padding-left:18px">{items}</ul></div></div>"#,
                warn = svg("flag"),
                items = errors.iter().map(|e| format!("<li>{}</li>", esc(e))).collect::<String>(),
            );
            let (_, text, on_disk) = load_route_settings(&state).await;
            let html = routes_html(RoutesView {
                settings: &settings,
                banner: &banner,
                test_value: "",
                test: "",
                file: &text,
                on_disk,
                landing: None,
            });
            (StatusCode::UNPROCESSABLE_ENTITY, page_for_role(actor.role, "Routes", "tools", Some(&actor.user), &html, true)).into_response()
        }
    }
}

pub(crate) async fn routes_download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let (_, body, _) = load_route_settings(&state).await;
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"routes.yaml\""),
        ],
        body,
    )
        .into_response()
}

/// Upload a routes.yaml. Routing is live configuration, so the file is
/// parsed *and* built into a router tree before anything is written.
pub(crate) async fn routes_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let back = "/ghost/routes/";
    while let Ok(Some(field)) = multipart.next_field().await {
        if !matches!(field.name(), Some("file" | "routes")) {
            continue;
        }
        let text = match field.text().await {
            Ok(t) => t,
            Err(e) => return tool_flash_or_json(&headers, back, "error", &format!("Upload failed: {e}")),
        };
        return match apply_routes(&state, &text).await {
            Ok(None) => tool_flash_or_json(&headers, back, "ok", "Routes uploaded — routing and the theme have been rebuilt."),
            Ok(Some(warning)) | Err(warning) => tool_flash_or_json(&headers, back, "error", &warning),
        };
    }
    tool_flash_or_json(&headers, back, "error", "No file was uploaded.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn the_routes_form_keeps_collection_order_and_fields() {
        let s = routes_from_form(&pairs(&[
            ("c_start", "1"), ("c_path", "/blog/"), ("c_permalink", "/blog/{slug}/"), ("c_filter", "tag:blog"),
            ("c_templates", "blog, index"), ("c_order", ""), ("c_limit", "5"), ("c_rss", "off"), ("c_data", "page.blog"),
            ("c_start", "1"), ("c_path", "/"), ("c_permalink", "/{slug}/"), ("c_filter", ""), ("c_templates", "index"),
            ("c_order", ""), ("c_limit", ""), ("c_rss", "on"), ("c_data", ""),
            ("r_start", "1"), ("r_path", "/podcast/"), ("r_templates", ""), ("r_filter", "tag:podcast"),
            ("r_start", "1"), ("r_path", ""), ("r_templates", ""), ("r_filter", ""),
            ("tag_on", "1"), ("tag", "/topic/{slug}/"), ("author", "/author/{slug}/"),
        ]));
        assert_eq!(s.collections.len(), 2);
        assert_eq!(s.collections[0].path, "/blog/");
        assert_eq!(s.collections[0].templates, vec!["blog", "index"]);
        assert_eq!(s.collections[0].rss, Some(false));
        assert_eq!(s.collections[0].limit.as_ref().and_then(|v| v.as_u64()), Some(5));
        assert_eq!(s.collections[1].rss, None);
        assert_eq!(s.routes.len(), 1, "the empty route card is dropped");
        assert_eq!(s.taxonomies.tag.as_deref(), Some("/topic/{slug}/"));
        assert!(s.taxonomies.author.is_none(), "unchecked taxonomy is switched off");
        assert!(route_problems(&s).is_empty());
        // and what is written reads back as the same thing
        let back = vuuut_routing::routes_yaml::parse_routes_yaml(&vuuut_routing::routes_yaml::to_yaml(&s)).unwrap();
        assert_eq!(back.collections[0].filter.as_deref(), Some("tag:blog"));
        assert!(RouterTree::build(&back).is_ok());
    }

    #[test]
    fn a_post_address_without_a_slug_is_refused() {
        let s = routes_from_form(&pairs(&[("c_start", "1"), ("c_path", "/"), ("c_permalink", "/post/")]));
        assert_eq!(route_problems(&s).len(), 1);
    }

    #[test]
    fn a_full_url_is_tested_by_its_path() {
        assert_eq!(split_test_address("https://site.test/old/?a=1"), ("/old/".into(), Some("a=1".into())));
        assert_eq!(split_test_address("old"), ("/old".into(), None));
    }

    #[test]
    fn patterns_are_told_apart_from_paths() {
        assert!(!is_pattern("/feed.xml"));
        assert!(is_pattern("/post/(\\d+)/"));
    }
}
