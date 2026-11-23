//! Admin-wide search: one box that finds anything in the admin.
//!
//! The trigger sits in the rail, under the brand and above the first nav item,
//! so it is in the same place on every screen. Clicking it (or pressing `/`,
//! `⌘K` or `Ctrl-K`) opens a modal whose results come from here.
//!
//! ## Why the server renders the rows
//!
//! The obvious shape is a JSON endpoint and a client-side template. This
//! returns an **HTML fragment** instead, for two reasons:
//!
//! - Every title on this screen is user content — a post title, a member's
//!   name, a tag. Rendering here means it passes through `esc` on the way out,
//!   next to every other piece of admin HTML, rather than relying on a
//!   hand-written JS template to escape correctly forever.
//! - The static half of the index (modules, tools, settings) is already Rust
//!   data. Shipping it to the browser would mean serialising ~250 rows into
//!   every admin page, or a second endpoint and a cache; matching it here
//!   costs a string compare per row on a list that is a few hundred long.
//!
//! ## Who sees what
//!
//! The handler is open to any signed-in role, and each *source* is gated by
//! the role its own page requires, so a result never leads to a refusal:
//!
//! | source | needs |
//! |---|---|
//! | modules, tools, settings | Administrator |
//! | members | Administrator |
//! | tags | Editor |
//! | posts, pages | any role, narrowed to the viewer's own below Editor |
//!
//! That narrowing is `Role::can_edit_any_post`, the same test the posts list
//! and the dashboard use.

use super::*;

/// How many of each kind of result to show. Small on purpose: this is a
/// jump-to box, not a report. Someone who wants the twelfth matching post
/// wants the posts list, and the footer row says so.
const PER_KIND: usize = 5;

/// A matched row, ready to render.
struct Hit {
    url: String,
    title: String,
    detail: String,
    icon: &'static str,
    tint: &'static str,
}

/// Case-insensitive substring match, which is what a jump-to box needs: the
/// query is a few characters of a name somebody already knows.
fn matches(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

fn row(h: &Hit, kind: &str) -> String {
    format!(
        r#"<a class="omni-hit" href="{url}" data-kind="{kind}">
<span class="ic tint-{tint}">{ic}</span>
<span class="omni-text"><span class="omni-title">{title}</span><span class="omni-detail">{detail}</span></span>
<span class="omni-go">{chev}</span></a>"#,
        url = esc(&h.url),
        kind = esc(kind),
        tint = h.tint,
        ic = svg(h.icon),
        title = esc(&h.title),
        detail = esc(&h.detail),
        chev = svg("chevron"),
    )
}

fn section(label: &str, hits: &[Hit], more: Option<(&str, &str)>) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let rows: String = hits.iter().map(|h| row(h, label)).collect();
    // "See all 42 posts" — the way out of a deliberately short list.
    let tail = more
        .map(|(text, url)| {
            format!(
                r#"<a class="omni-more" href="{url}">{text}</a>"#,
                url = esc(url),
                text = esc(text),
            )
        })
        .unwrap_or_default();
    format!(
        r#"<div class="omni-group"><p class="omni-label">{label}</p>{rows}{tail}</div>"#,
        label = esc(label),
    )
}

/// `GET /ghost/omnisearch/?q=…` — the modal's result list, as HTML.
///
/// It was `/ghost/search/` until 2026-11-21, when that address went to
/// Tools → Search and Filtering (the public site's search). The palette is
/// staff-only chrome, so it moved rather than the tool.
pub(crate) async fn search_results(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    // Any signed-in staff member may search; the sources are gated below,
    // each by the role its own page requires. Read-only, so there is no
    // state-changing request for an origin check to protect.
    let actor = match require_actor(&state, &headers).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let role = actor.role;
    let term = q.get("q").map(|s| s.trim()).unwrap_or("");
    // One character matches most of the admin, which is noise rather than an
    // answer. The modal says what to do instead.
    if term.chars().count() < 2 {
        return html_fragment(String::new());
    }
    let needle = term.to_lowercase();

    let mut out = String::new();

    // ---- the static half: tools and settings ----
    // All three hubs are Administrator-only, so nobody below that is offered a
    // door they cannot open.
    // Only what this viewer may actually open, as the Permissions module has
    // it. A search result is a door: offering one that answers "Not allowed"
    // is the same mistake as a rail item that does.
    let grants = crate::permissions::grants_for(&state, role);
    {
        let mut tools: Vec<Hit> = crate::tools::TOOLS
            .iter()
            .filter(|t| {
                // Owner-only and outside the matrix — see
                // `crate::permissions::UNGRANTABLE`.
                if crate::permissions::UNGRANTABLE.contains(&t.key) {
                    return role == Role::Owner;
                }
                grants.tools.access(t.key).may_read()
            })
            // Also by what is on the tool's screens — the same phrases the
            // Tools hub's own box searches (`crate::tool_finder`).
            .filter(|t| {
                matches(t.title, &needle)
                    || matches(t.description, &needle)
                    || crate::tool_finder::content_of(t.key).iter().any(|p| matches(p, &needle))
            })
            .take(PER_KIND)
            .map(|t| Hit {
                url: t.href.to_string(),
                title: t.title.to_string(),
                detail: t.description.to_string(),
                icon: t.icon,
                tint: t.tint,
            })
            .collect();
        tools.extend(
            crate::settings::SETTINGS_GROUPS.iter()
                .filter(|g| g.tool.is_some() && grants.settings.access(g.slug).may_read())
                .filter(|g| {
                    matches(&format!("{} settings", g.title), &needle)
                        || crate::settings::settings_entries(false, g.tool)
                            .iter()
                            .any(|entry| matches(&entry.haystack, &needle))
                })
                .map(|g| Hit {
                    url: g.path(),
                    title: format!("{} settings", g.title),
                    detail: g.items.join(" · "),
                    icon: g.icon,
                    tint: g.tint,
                }),
        );
        out.push_str(&section("Tools", &tools, None));

        // The settings index, matched on the same haystack the settings hub's
        // own box matches — see `settings::settings_entries`.
        let entries = crate::settings::settings_entries(state.config.env == "development", None);
        let matched: Vec<&crate::settings::SearchEntry> = entries
            .iter()
            .filter(|e| grants.settings.access(e.group).may_read())
            .filter(|e| matches(&e.haystack, &needle))
            .collect();
        let settings: Vec<Hit> = matched
            .iter()
            .take(PER_KIND)
            .map(|e| Hit {
                url: e.url.clone(),
                title: e.title.clone(),
                detail: e.detail.clone(),
                icon: e.icon,
                tint: e.tint,
            })
            .collect();
        // Only when there is actually more to see: a "see all" under three
        // results is an invitation to a page that shows the same three.
        let more = (matched.len() > settings.len())
            .then_some(("Search all settings", "/ghost/settings/"));
        out.push_str(&section("Settings", &settings, more));
    }

    // ---- the content half ----
    // An Author or Contributor sees their own entries, the same narrowing the
    // posts list and the dashboard apply.
    for (post_type, label, list_url) in
        [("post", "Posts", "/ghost/posts/"), ("page", "Pages", "/ghost/pages/")]
    {
        let content = if post_type == "page" { grants.pages } else { grants.posts };
        if !content.may_read() {
            continue;
        }
        let mine = (!content.any_author()).then_some(actor.user.id.clone());
        let query = repos::posts::AdminBrowsePosts {
            filter: None,
            post_type: post_type.to_string(),
            author_id: mine.clone(),
            search: term.to_string(),
            status: None,
            sort: String::new(),
            page: 1,
            limit: PER_KIND as i64,
        };
        // A failed read costs that section, not the search: the rest of the
        // results are still worth showing.
        let Ok(found) = repos::posts::browse_admin(&state.db, &query).await else {
            continue;
        };
        let total: i64 = found.counts.values().sum();
        let hits: Vec<Hit> = found
            .listing
            .posts
            .iter()
            .map(|p| Hit {
                url: format!("/ghost/{post_type}s/{}/", p.id),
                // An entry with no title is a real draft somebody started and
                // has to be able to find again, so it gets a stand-in rather
                // than being dropped from the list.
                title: match p.title.as_deref().map(str::trim) {
                    Some(t) if !t.is_empty() => t.to_string(),
                    _ => "(untitled)".to_string(),
                },
                detail: format!("{} · {}", label.trim_end_matches('s'), p.status),
                icon: if post_type == "post" { "posts" } else { "pages" },
                tint: if post_type == "post" { "blue" } else { "green" },
            })
            .collect();
        let more = (total > hits.len() as i64)
            .then(|| (format!("See all {total} {}", label.to_lowercase()), list_url));
        out.push_str(&section(
            label,
            &hits,
            more.as_ref().map(|(t, u)| (t.as_str(), *u)),
        ));
    }

    // Tags are an Editor's screen and there are rarely many, so they are read
    // whole and filtered here rather than in SQL.
    if grants.tags.may_read() {
        let tags = repos::tags::all(&state.db).await.unwrap_or_default();
        let hits: Vec<Hit> = tags
            .iter()
            .filter(|t| {
                matches(t.name.as_deref().unwrap_or(""), &needle)
                    || matches(t.slug.as_deref().unwrap_or(""), &needle)
            })
            .take(PER_KIND)
            .map(|t| Hit {
                url: format!("/ghost/tags/{}/", t.id),
                title: match t.name.as_deref().map(str::trim) {
                    Some(n) if !n.is_empty() => n.to_string(),
                    _ => t.slug.clone().unwrap_or_else(|| "(untitled)".into()),
                },
                detail: match t.visibility.as_str() {
                    "internal" => "Tag · internal".to_string(),
                    _ => "Tag".to_string(),
                },
                icon: "tags",
                tint: "amber",
            })
            .collect();
        out.push_str(&section("Tags", &hits, None));
    }

    if role.is_admin() {
        let filter = crate::members::members_nql("", term, "");
        let (members, total) = repos::members::browse(&state.db, filter.as_deref(), PER_KIND as i64, 1)
            .await
            .unwrap_or_default();
        let hits: Vec<Hit> = members
            .iter()
            .map(|m| Hit {
                url: format!("/ghost/members/{}/", m.id),
                title: match m.name.as_deref().map(str::trim) {
                    Some(name) if !name.is_empty() => name.to_string(),
                    _ => m.email.clone(),
                },
                detail: m.email.clone(),
                icon: "group",
                tint: "purple",
            })
            .collect();
        let more = (total > hits.len() as i64)
            .then(|| (format!("See all {total} members"), "/ghost/members/"));
        out.push_str(&section(
            "Members",
            &hits,
            more.as_ref().map(|(t, u)| (t.as_str(), *u)),
        ));
    }

    if out.is_empty() {
        out = format!(
            r#"<p class="omni-empty">Nothing matches <b>{}</b>.</p>"#,
            esc(term)
        );
    }
    html_fragment(out)
}

/// A fragment, not a document: no shell, and never cached — a result list is
/// about content that changes under it.
fn html_fragment(body: String) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

/// The rail's trigger: a button shaped like a search field, between the brand
/// and the first nav item. A button rather than an `<input>` because the real
/// input is the one inside the modal — two focusable boxes that look alike,
/// one of which silently does nothing, is the worse version of this.
pub(crate) fn rail_trigger() -> String {
    format!(
        r#"<button type="button" class="rail-search" data-omni-open aria-label="Search the admin">
{ic}<span class="rail-search-label">Search</span><kbd class="rail-search-key">/</kbd>
</button>"#,
        ic = svg("search"),
    )
}

/// The modal itself, rendered once per page into the shell.
///
/// Empty until it is opened: the rows arrive from `search_results`. The
/// `<dialog>` is the platform's own — it brings the top layer, the backdrop,
/// focus trapping and Escape without any of it being written here.
pub(crate) fn modal() -> String {
    format!(
        r#"<dialog class="omni" id="omni" aria-label="Search the admin">
<div class="omni-box">
<label class="omni-field">{ic}
<input type="search" id="omni-input" autocomplete="off" spellcheck="false"
 placeholder="Search posts, pages, tags, settings…" aria-label="Search the admin">
<kbd class="omni-esc">esc</kbd></label>
<div class="omni-results" id="omni-results" role="listbox" aria-label="Results">
<p class="omni-empty">Type to search posts, pages, tags, members, modules, tools and settings.</p>
</div>
</div>
</dialog>"#,
        ic = svg("search"),
    )
}
