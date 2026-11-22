//! Tools → Search and Filtering: how the public site's search works.
//!
//! The rules are [`ghost_core::site_search`]; the built-in engine, its index
//! and the overlay visitors use are `ghost_api::site_search`; Sodo Search's
//! three index feeds read the same settings in `ghost_api::content`. What is
//! here is the three screens that drive them:
//!
//! * **Configure** (`/ghost/search/`) — which engine answers the theme's
//!   search button, what either engine may find, how the built-in engine
//!   ranks, and what the box looks like.
//! * **Try it** (`/ghost/search/try/`) — a search run through the real
//!   ranking with the saved settings, showing each result's score and the
//!   reasons for it, so a weight or a synonym can be tuned against a query
//!   rather than guessed at.
//! * **Insights** (`/ghost/search/insights/`) — what visitors searched for
//!   over the last 30 days, and what found nothing: the content a site's
//!   readers are asking for and not getting.
//!
//! Not to be confused with `search.rs`, the staff palette behind the rail's
//! box — which lived at `/ghost/search/` until this tool took the address
//! and now answers at `/ghost/omnisearch/`.
//!
//! ## The security contract
//!
//! Administrator and above, origin-checked on every write, through `guard`.
//! Insights are text visitors typed, so every query is escaped on the way
//! out, and the tester's query is bounded by `terms()` before it reaches
//! anything. Nothing here can make a members-only body searchable: that rule
//! lives in `Doc::new`, below every setting.

use super::*;
use ghost_core::site_search::{self as rules, Engine, Kind, Settings};
use ghost_db::repos::site_search as repo;

const PATH: &str = "/ghost/search/";

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn tabs(active: &str) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}</div>"#,
        a = tab("configure", "Configure", PATH),
        b = tab("try", "Try it", "/ghost/search/try/"),
        c = tab("insights", "Insights", "/ghost/search/insights/"),
    )
}

fn checkbox(name: &str, on: bool) -> String {
    format!(
        r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{on}><span class="track"><span class="thumb"></span></span></label>"#,
        on = if on { " checked" } else { "" },
    )
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing ss-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

fn picker(name: &str, label: &str, options: &[(String, String)], current: &str) -> String {
    let opts: String = options
        .iter()
        .map(|(v, t)| format!(r#"<option value="{v}"{sel}>{t}</option>"#, v = esc(v), sel = if v == current { " selected" } else { "" }, t = esc(t)))
        .collect();
    format!(
        r#"<label class="field fixed" style="margin:0"><select name="{name}">{opts}</select><span>{label}</span>{caret}</label>"#,
        label = esc(label),
        caret = SELECT_CARET,
    )
}

/// A 0–10 weight: a range for the hand and the number beside it for the eye.
/// The number is an `<output>` the range updates — no script beyond the one
/// `oninput`, and without it the range still submits.
fn weight(name: &str, label: &str, detail: &str, value: i64) -> String {
    format!(
        r#"<div class="ss-weight"><div><span class="title">{label}</span><span class="detail">{detail}</span></div>
<input type="range" name="{name}" min="0" max="{max}" step="1" value="{value}" aria-label="{label} weight" oninput="this.nextElementSibling.value=this.value">
<output>{value}</output></div>"#,
        label = esc(label),
        detail = esc(detail),
        max = rules::MAX_WEIGHT,
    )
}

// ---------------------------------------------------------------------------
// Configure
// ---------------------------------------------------------------------------

pub(crate) async fn search_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::site_search::settings(&state);
    let docs = ghost_api::site_search::documents(&state).await;
    let tags = repo::public_tags(&state.db).await.unwrap_or_default();

    let engine_card = |e: Engine, icon: &str, tint: &str, line: &str, points: &[&str]| {
        let list: String = points.iter().map(|p| format!("<li>{}</li>", esc(p))).collect();
        format!(
            r#"<label class="card ss-engine{on}"><input type="radio" name="{key}" value="{value}"{checked}>
<span class="ss-engine-head"><span class="ic tint-{tint}">{ic}</span><span class="ss-radio" aria-hidden="true"></span></span>
<b>{label}</b><span class="ss-engine-line">{line}</span><ul>{list}</ul></label>"#,
            on = if s.engine == e { " on" } else { "" },
            key = rules::SETTING_ENGINE,
            value = e.key(),
            checked = if s.engine == e { " checked" } else { "" },
            ic = svg(icon),
            label = esc(e.label()),
            line = esc(line),
        )
    };
    let engines = format!(
        r#"<div class="ss-engines rise" style="--i:2">{a}{b}{c}</div>"#,
        a = engine_card(Engine::Vuuut, "search", "pink", "Built in. Ranked on this server.", &[
            "Searches whole posts, not just titles",
            "Synonyms, field weights and filters",
            "Counts what readers search for",
        ]),
        b = engine_card(Engine::Sodo, "ghost", "blue", "Ghost's Sodo overlay, loaded from a CDN.", &[
            "What every Ghost theme expects",
            "Titles and excerpts, in the browser",
            "Follows the “What can be found” rules",
        ]),
        c = engine_card(Engine::Off, "hide", "slate", "Neither script is added to pages.", &[
            "The theme's search button does nothing",
            "Use when the theme has its own search",
        ]),
    );

    let tag_boxes: String = if tags.is_empty() {
        r#"<p class="muted" style="margin:0">No public tags yet.</p>"#.to_string()
    } else {
        tags.iter()
            .map(|(name, slug)| {
                format!(
                    r#"<label class="ss-tag"><input type="checkbox" name="exclude_tag" value="{slug}"{on}><span>{name}</span></label>"#,
                    slug = esc(slug),
                    on = if s.exclude_tags.contains(slug) { " checked" } else { "" },
                    name = esc(name),
                )
            })
            .collect()
    };

    let orders = vec![("relevance".to_string(), "Best match first".to_string()), ("newest".to_string(), "Newest first".to_string())];
    let limits: Vec<(String, String)> = rules::LIMITS.iter().map(|n| (n.to_string(), format!("{n} results"))).collect();
    let posts = docs.iter().filter(|d| d.kind == Kind::Post).count();
    let pages = docs.len() - posts;
    let members = docs.iter().filter(|d| d.visibility != "public").count();

    page_for_role(
        actor.role,
        "Search and Filtering",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{PATH}" id="search-settings">
<div class="section">Engine</div>
<p class="muted ss-lead">Which search opens when a reader presses the theme's search button or a <code>#/search</code> link.</p>
{engines}
<div class="section">What can be found</div>
<p class="muted ss-lead">Applies to both engines. {count}.</p>
<div class="card rows rise" style="--i:3">
{pages_row}
{members_row}
{full_row}
{authors_row}
{tags_row}
<div class="row"><span class="ic tint-red">{hide}</span><div class="body"><span class="title">Leave out posts tagged</span>
<span class="detail">Posts with any of these tags never appear in search.</span>
<div class="ss-tags">{tag_boxes}</div></div></div>
</div>
<div class="section">Ranking</div>
<p class="muted ss-lead">Vuuut search only. A match in a field scores its weight — twice for a whole word. 0 stops searching that field.</p>
<div class="card padded ss-weights rise" style="--i:4">
{w_title}{w_excerpt}{w_tags}{w_body}
</div>
<div class="card rows rise" style="--i:5;margin-top:12px">
{order_row}
{limit_row}
<div class="row"><span class="ic tint-purple">{repeat}</span><div class="body"><span class="title">Synonyms</span>
<span class="detail">One group per line, words separated by commas. Any word in a group finds the others.</span>
<label class="field" style="margin:10px 0 0"><textarea name="{syn_key}" rows="5" placeholder="js, javascript&#10;seo, search engine optimisation&#10;colour, color">{synonyms}</textarea><span>Groups of words that mean the same here</span></label></div></div>
</div>
<div class="section">The search box</div>
<div class="card rows rise" style="--i:6">
<div class="row"><span class="ic tint-blue">{title_ic}</span><div class="body"><span class="title">Placeholder</span><span class="detail">What the empty box says.</span>
<label class="field" style="margin:10px 0 0"><input name="{ph_key}" value="{placeholder}" maxlength="{ph_max}" placeholder="{ph_default}"><span>Placeholder</span></label></div></div>
{shortcut_row}
{log_row}
</div>
</form>"#,
            head = crate::tools::tool_head("search", Some("search-settings"), &format!(
                r#"<a class="btn text" href="/" target="_blank" rel="noopener">{open} View site</a>"#,
                open = svg("open"),
            )),
            tabs = tabs("configure"),
            banner = flash_banner(&q),
            count = esc(&format!(
                "Right now search covers {}{}{}",
                countable(posts as i64, "post"),
                if s.include_pages { format!(" and {}", countable(pages as i64, "page")) } else { String::new() },
                if members > 0 { format!(", {members} of them for members (title and excerpt only)") } else { String::new() },
            )),
            pages_row = row("pages", "green", "Include pages", "About, contact and other pages appear beside posts.", &checkbox(rules::SETTING_INCLUDE_PAGES, s.include_pages)),
            members_row = row("lock", "amber", "Include members-only posts", "Listed by title and excerpt so readers know they exist. Their body is never searched or shown.", &checkbox(rules::SETTING_INCLUDE_MEMBERS, s.include_members)),
            full_row = row("notes", "blue", "Search the whole post", "Match words anywhere in a public post, not just its title and excerpt.", &checkbox(rules::SETTING_FULL_TEXT, s.full_text)),
            authors_row = row("person", "teal", "Show matching authors", "Staff whose name matches, above the posts.", &checkbox(rules::SETTING_SHOW_AUTHORS, s.show_authors)),
            tags_row = row("tags", "purple", "Show matching tags", "Tags whose name matches, above the posts.", &checkbox(rules::SETTING_SHOW_TAGS, s.show_tags)),
            hide = svg("hide"),
            w_title = weight(rules::SETTING_WEIGHT_TITLE, "Title", "The strongest signal there is.", s.weight_title),
            w_excerpt = weight(rules::SETTING_WEIGHT_EXCERPT, "Excerpt", "The summary under the title.", s.weight_excerpt),
            w_tags = weight(rules::SETTING_WEIGHT_TAGS, "Tags", "A post filed under the word.", s.weight_tags),
            w_body = weight(rules::SETTING_WEIGHT_BODY, "Body", "Only with “Search the whole post” on.", s.weight_body),
            order_row = row("trending", "green", "Order", "Best match puts strong matches first; newest ignores the score.", &picker(rules::SETTING_ORDER, "Order", &orders, s.order.key())),
            limit_row = row("menu", "slate", "Results per search", "How many posts one search shows.", &picker(rules::SETTING_LIMIT, "Show", &limits, &s.limit.to_string())),
            repeat = svg("repeat"),
            syn_key = rules::SETTING_SYNONYMS,
            synonyms = esc(&rules::synonyms_text(&s.synonyms)),
            title_ic = svg("title"),
            ph_key = rules::SETTING_PLACEHOLDER,
            placeholder = esc(&state.settings.get_str(rules::SETTING_PLACEHOLDER).unwrap_or_default()),
            ph_max = rules::MAX_PLACEHOLDER,
            ph_default = esc(rules::DEFAULT_PLACEHOLDER),
            shortcut_row = row("integrations", "pink", "Keyboard shortcut", "“/” or ⌘K opens search from anywhere on the site.", &checkbox(rules::SETTING_SHORTCUT, s.shortcut)),
            log_row = row("chart", "amber", "Count searches", "Keeps the words readers search for — no names, cookies or addresses — for 90 days, for Insights.", &checkbox(rules::SETTING_LOG, s.log)),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn search_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    // Parsed by hand rather than through `Form<HashMap>`, because the tag
    // checkboxes repeat one name and a map keeps only one of them.
    let mut form: HashMap<String, String> = HashMap::new();
    let mut excluded: Vec<String> = Vec::new();
    for (k, v) in form_urlencoded::parse(body.as_bytes()) {
        if k == "exclude_tag" {
            excluded.push(v.into_owned());
        } else {
            form.entry(k.into_owned()).or_insert_with(|| v.into_owned());
        }
    }
    let flag = |key: &str| form.get(key).is_some_and(|v| v == "1");
    // Through the same reader the site uses, so what is stored is what runs:
    // an unknown engine is Sodo, weights are clamped, a limit not offered is
    // the default.
    let s = Settings::read(|key| match key {
        rules::SETTING_INCLUDE_PAGES | rules::SETTING_INCLUDE_MEMBERS | rules::SETTING_FULL_TEXT
        | rules::SETTING_SHOW_AUTHORS | rules::SETTING_SHOW_TAGS | rules::SETTING_SHORTCUT | rules::SETTING_LOG => {
            Some(flag(key).to_string())
        }
        rules::SETTING_EXCLUDE_TAGS => Some(excluded.join(",")),
        _ => form.get(key).cloned(),
    });
    let placeholder = rules::clean(form.get(rules::SETTING_PLACEHOLDER).map(String::as_str).unwrap_or(""), rules::MAX_PLACEHOLDER);
    let writes = [
        (rules::SETTING_ENGINE, serde_json::json!(s.engine.key())),
        (rules::SETTING_INCLUDE_PAGES, serde_json::json!(s.include_pages)),
        (rules::SETTING_INCLUDE_MEMBERS, serde_json::json!(s.include_members)),
        (rules::SETTING_EXCLUDE_TAGS, serde_json::json!(s.exclude_tags.join(","))),
        (rules::SETTING_FULL_TEXT, serde_json::json!(s.full_text)),
        (rules::SETTING_SHOW_AUTHORS, serde_json::json!(s.show_authors)),
        (rules::SETTING_SHOW_TAGS, serde_json::json!(s.show_tags)),
        (rules::SETTING_WEIGHT_TITLE, serde_json::json!(s.weight_title)),
        (rules::SETTING_WEIGHT_EXCERPT, serde_json::json!(s.weight_excerpt)),
        (rules::SETTING_WEIGHT_TAGS, serde_json::json!(s.weight_tags)),
        (rules::SETTING_WEIGHT_BODY, serde_json::json!(s.weight_body)),
        (rules::SETTING_ORDER, serde_json::json!(s.order.key())),
        (rules::SETTING_LIMIT, serde_json::json!(s.limit)),
        (rules::SETTING_SYNONYMS, serde_json::json!(rules::synonyms_text(&s.synonyms))),
        (rules::SETTING_PLACEHOLDER, serde_json::json!(placeholder)),
        (rules::SETTING_SHORTCUT, serde_json::json!(s.shortcut)),
        (rules::SETTING_LOG, serde_json::json!(s.log)),
    ];
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("search: saving {key} failed: {}", e.message());
            return crate::tools::tool_flash(PATH, "error", "Saving failed — check the logs.");
        }
    }
    // The next search — on either engine — sees the change at once.
    ghost_api::site_search::invalidate(&state);
    crate::tools::tool_saved(&state, PATH, "Search settings saved.")
}

// ---------------------------------------------------------------------------
// Try it
// ---------------------------------------------------------------------------

pub(crate) async fn try_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::site_search::settings(&state);
    // Bounded the way the public box bounds it, then shown back escaped.
    let query: String = q.get("q").map(|v| v.chars().take(rules::MAX_QUERY).collect()).unwrap_or_default();
    let docs = ghost_api::site_search::documents(&state).await;
    let ranked = rules::rank(&docs, &query, &s, None, None);
    let groups = rules::expand(&rules::terms(&query), &s.synonyms);
    let expanded: Vec<String> = groups.iter().filter(|g| g.len() > 1).map(|g| g.join(" = ")).collect();

    let rows: String = ranked
        .iter()
        .take(s.limit as usize)
        .enumerate()
        .map(|(i, r)| {
            format!(
                r#"<div class="row ss-hit"><span class="ss-rank">{n}</span>
<div class="body"><span class="title">{title}{kind}{members}</span>
<span class="detail">{why}</span></div>
<div class="trailing"><span class="ss-score">{score}</span><a class="icon-btn" href="{url}" target="_blank" rel="noopener" title="Open">{open}</a></div></div>"#,
                n = i + 1,
                title = esc(&r.doc.title),
                kind = if r.doc.kind == Kind::Page { r#" <span class="chip tonal">Page</span>"# } else { "" },
                members = if r.doc.visibility != "public" { r#" <span class="chip draft">Members</span>"# } else { "" },
                why = esc(&r.why.join(" · ")),
                score = format_args!("{:.1}", r.score),
                url = esc(&r.doc.url),
                open = svg("open"),
            )
        })
        .collect();

    let result = if query.trim().is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3"><p class="muted" style="margin:0">Type what a reader would. Each result shows its score and what earned it, using the saved settings — change a weight or a synonym on Configure, then search again here. Searching {}.</p></div>"#,
            esc(&countable(docs.len() as i64, "post or page")),
        )
    } else if ranked.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3;text-align:center"><p style="margin:6px 0;font-weight:600">Nothing matches “{q}”.</p>
<p class="muted" style="margin:0">Every word has to match somewhere. Try “Search the whole post”, or add a synonym.</p></div>"#,
            q = esc(&query),
        )
    } else {
        format!(
            r#"<p class="muted ss-lead">{n}{shown}{syn}</p><div class="card rows rise" style="--i:3">{rows}</div>"#,
            n = esc(&countable(ranked.len() as i64, "result")),
            shown = if ranked.len() as i64 > s.limit { format!(", the first {} shown as readers see them", s.limit) } else { String::new() },
            syn = if expanded.is_empty() { String::new() } else { format!(" · synonyms used: {}", esc(&expanded.join("; "))) },
        )
    };

    let note = match s.engine {
        Engine::Vuuut => String::new(),
        Engine::Sodo => r#"<div class="card padded ss-note rise" style="--i:2">This is how <b>Vuuut search</b> would rank it. The site is using Sodo search, which matches titles and excerpts in the reader's browser and does not use weights or synonyms.</div>"#.to_string(),
        Engine::Off => r#"<div class="card padded ss-note rise" style="--i:2">Search is switched off on the site. This is how Vuuut search would answer if it were on.</div>"#.to_string(),
    };

    page_for_role(
        actor.role,
        "Try search",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{note}
<form class="ss-try rise" style="--i:2" method="get" action="{PATH}try/">
<label class="field" style="margin:0;flex:1"><input type="search" name="q" value="{q}" placeholder=" " maxlength="{max}" autofocus autocomplete="off"><span>Search as a reader would</span></label>
<button class="btn" type="submit">{ic} Search</button></form>
{result}"#,
            head = crate::tools::tool_head("search", None, ""),
            tabs = tabs("try"),
            q = esc(&query),
            max = rules::MAX_QUERY,
            ic = svg("search"),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// Insights
// ---------------------------------------------------------------------------

pub(crate) async fn insights_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::site_search::settings(&state);
    let since = ghost_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(30)));
    let (total, zero) = repo::totals(&state.db, &since).await;
    let top = repo::top(&state.db, &since, false, 20).await.unwrap_or_default();
    let missing = repo::top(&state.db, &since, true, 20).await.unwrap_or_default();
    let max = top.first().map(|t| t.1).unwrap_or(1).max(1);

    let list = |rows: &[(String, i64, i64)], empty: &str, zero_list: bool| -> String {
        if rows.is_empty() {
            return format!(r#"<div class="card padded"><p class="muted" style="margin:0">{}</p></div>"#, esc(empty));
        }
        let items: String = rows
            .iter()
            .map(|(query, n, results)| {
                let try_href = format!(
                    "/ghost/search/try/?{}",
                    form_urlencoded::Serializer::new(String::new()).append_pair("q", query).finish()
                );
                format!(
                    r#"<a class="row ss-query" href="{href}"><div class="body"><span class="title">{query}</span>
<span class="ss-bar"><span style="width:{w:.0}%"></span></span></div>
<div class="trailing"><span class="ss-count">{n}</span><span class="chip {chip}">{res}</span></div></a>"#,
                    href = esc(&try_href),
                    query = esc(query),
                    w = (*n as f64) * 100.0 / max as f64,
                    chip = if *results == 0 { "error" } else { "tonal" },
                    res = if zero_list || *results == 0 { "no results".to_string() } else { countable(*results, "result") },
                )
            })
            .collect();
        format!(r#"<div class="card rows">{items}</div>"#)
    };

    let tile = |label: &str, value: String, detail: &str, icon: &str, tint: &str| {
        format!(
            r#"<div class="card padded ss-tile"><span class="ic tint-{tint}">{ic}</span><span class="ss-tile-value">{value}</span>
<span class="ss-tile-label">{label}</span><span class="ss-tile-detail">{detail}</span></div>"#,
            ic = svg(icon),
            value = esc(&value),
            label = esc(label),
            detail = esc(detail),
        )
    };
    let rate = if total > 0 { format!("{:.0}%", zero as f64 * 100.0 / total as f64) } else { "—".into() };

    let off = if s.engine != Engine::Vuuut {
        r#"<div class="card padded ss-note rise" style="--i:2">Only <b>Vuuut search</b> counts searches — Sodo search runs in the reader's browser and never tells the site what was typed. Switch engines on Configure to start collecting.</div>"#
    } else if !s.log {
        r#"<div class="card padded ss-note rise" style="--i:2">Counting searches is switched off on Configure.</div>"#
    } else {
        ""
    };

    page_for_role(
        actor.role,
        "Search insights",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{off}
<div class="ss-tiles rise" style="--i:3">{a}{b}{c}</div>
<div class="ss-split">
<div><div class="section">Most searched</div>{top}</div>
<div><div class="section">Found nothing</div>{missing}
<p class="muted" style="margin:10px 4px 0;font-size:12.5px">Readers asked for these and the site had no answer. Write it, tag it, or add a synonym that leads to what you have.</p></div>
</div>"#,
            head = crate::tools::tool_head("search", None, &format!(
                r#"<form method="post" action="{PATH}insights/clear/"><button class="btn text" type="submit"
 data-confirm="Clear search insights|Every counted search is deleted. It cannot be undone.">{del} Clear</button></form>"#,
                del = svg("delete"),
            )),
            tabs = tabs("insights"),
            banner = flash_banner(&q),
            a = tile("Searches", total.to_string(), "Last 30 days", "search", "pink"),
            b = tile("Found nothing", zero.to_string(), "Searches with no result", "warning", "amber"),
            c = tile("Miss rate", rate, "Share of searches that found nothing", "trending", "purple"),
            top = list(&top, "No searches counted yet.", false),
            missing = list(&missing, "Every search found something.", true),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn insights_clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    match repo::clear(&state.db).await {
        Ok(n) => crate::tools::tool_flash("/ghost/search/insights/", "ok", &format!("{} deleted.", countable(n as i64, "search"))),
        Err(e) => {
            tracing::error!("search: clearing insights failed: {}", e.message());
            crate::tools::tool_flash("/ghost/search/insights/", "error", "That could not be done — check the logs.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weight_control_escapes_its_label_and_carries_its_value() {
        let html = weight("w", "<b>", "x", 7);
        assert!(html.contains("&lt;b&gt;") && html.contains(r#"value="7""#) && html.contains("<output>7</output>"));
    }

    #[test]
    fn order_round_trips_through_its_key() {
        use ghost_core::site_search::Order;
        assert_eq!(Order::from_key(Order::Newest.key()), Order::Newest);
    }
}
