//! Projects: the grid, and the case-study editor — the same two-column
//! editor as a product. Down the middle: title, subtitle and summary, the
//! story in Koenig, pictures (the first is the cover), video, the facts
//! (client, year, role, timeline, industry, tools), links, results and
//! credits, and the search listing. Down the side: status and password,
//! disciplines, services, the testimonial, featured/order, layout and the
//! page's accent colour.

use super::*;
use crate::podcast::{card, check, field, koenig_config, read_lexical, select, textarea};

pub(crate) async fn projects_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = portfolio_settings(&state);
    let x = repo::Lookup::load(&state.db).await;
    let status = q.get("status").map(String::as_str).filter(|v| rules::ProjectStatus::ALL.iter().any(|x| x.key() == *v));
    let cat = q.get("category").filter(|v| x.categories.iter().any(|c| &c.id == *v)).cloned();
    let search = q.get("q").map(|v| vuuut_core::directory::clean(v, 80)).unwrap_or_default();
    let (rows, total) = repo::projects(&state.db, &repo::ProjectQuery { status, category_id: cat.as_deref(), search: (!search.is_empty()).then_some(search.as_str()), limit: 500, ..Default::default() }).await;
    let cards: String = rows.iter().map(|p| {
        let (chip, label) = status_chip(&p.status);
        let cats: Vec<&str> = x.project_categories.get(&p.id).map(|ids| x.categories.iter().filter(|c| ids.contains(&c.id)).map(|c| c.name.as_str()).collect()).unwrap_or_default();
        format!(
            r#"<a class="pf-card" href="{PROJECTS}{id}/"><span class="pf-card-art">{img}<span class="chip {chip}">{label}</span>{feat}</span><span class="pf-card-body"><b>{t}</b><span>{meta}</span><small>{views} views</small></span></a>"#,
            id = esc(&p.id),
            img = if p.cover.is_empty() { format!(r#"<i>{}</i>"#, svg("image")) } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&p.cover)) },
            feat = if p.featured != 0 { format!(r#"<span class="pf-star" title="Featured">{}</span>"#, svg("star")) } else { String::new() },
            t = esc(&p.title),
            meta = esc(&[x.client_name(p), cats.join(", "), if p.year > 0 { p.year.to_string() } else { String::new() }].iter().filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(" · ")),
            views = p.views,
        )
    }).collect();
    let status_opts: Vec<(String, String)> = std::iter::once((String::new(), "Any status".to_string())).chain(rules::ProjectStatus::options()).collect();
    let cat_opts: Vec<(String, String)> = std::iter::once((String::new(), "All disciplines".to_string())).chain(x.categories.iter().map(|c| (c.id.clone(), c.name.clone()))).collect();
    page_for_role(
        actor.role,
        "Projects",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="pc-filters rise" style="--i:2" method="get">{st}{cat}<label class="field"><input name="q" value="{qv}" placeholder=" "><span>Search</span></label><button class="btn tonal" type="submit">Filter</button></form>
<p class="muted dr-lead">{count}. Featured projects come first on the site, then by their order number.{view}</p>
<div class="pf-grid rise" style="--i:3">{cards}</div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, &format!(r#"<a class="btn" href="{PROJECTS}new/">{} New project</a>"#, svg("add"))),
            tabs = tabs_for(&state, "projects", &actor).await,
            banner = flash_banner(&q),
            st = select("status", "Status", &status_opts, status.unwrap_or("")),
            cat = select("category", "Discipline", &cat_opts, cat.as_deref().unwrap_or("")),
            qv = esc(&search),
            count = esc(&countable(total, "project")),
            view = if s.enabled { format!(r#" <a href="{}" target="_blank" rel="noopener">View the portfolio ↗</a>"#, public_href(&s, "/portfolio/")) } else { String::new() },
            cards = if cards.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No projects{}.</p><a class="btn" href="{PROJECTS}new/">{} New project</a></div>"#, if total == 0 && search.is_empty() { " yet" } else { " match" }, svg("add")) } else { cards },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn project_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn project_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(p) = repo::find_project(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That project could not be found.") };
    editor(&state, &actor, Some(p), &q).await
}

fn metric_row(v: &str, l: &str) -> String {
    format!(r#"<div class="pf-row2"><input class="ec-in" name="m_value" value="{}" placeholder="+38%" maxlength="30" aria-label="Figure"><input class="ec-in" name="m_label" value="{}" placeholder="increase in sign-ups" maxlength="120" aria-label="What it measures"><button class="icon-btn" type="button" data-pf-remove title="Remove">×</button></div>"#, esc(v), esc(l))
}

fn credit_row(n: &str, r: &str, u: &str) -> String {
    format!(r#"<div class="pf-row3"><input class="ec-in" name="c_name" value="{}" placeholder="Name" maxlength="120" aria-label="Name"><input class="ec-in" name="c_role" value="{}" placeholder="Role" maxlength="120" aria-label="Role"><input class="ec-in" name="c_url" value="{}" placeholder="Link (optional)" maxlength="500" aria-label="Link"><button class="icon-btn" type="button" data-pf-remove title="Remove">×</button></div>"#, esc(n), esc(r), esc(u))
}

pub(super) fn editor_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../../assets/portfolio-editor.js")));
    &HASH
}

async fn editor(state: &AppState, actor: &Actor, p: Option<repo::Project>, q: &HashMap<String, String>) -> Response {
    let s = portfolio_settings(state);
    let x = repo::Lookup::load(&state.db).await;
    let get = |f: fn(&repo::Project) -> &str| p.as_ref().map(f).unwrap_or("").to_string();
    let num = |f: fn(&repo::Project) -> i64, d: i64| p.as_ref().map(f).unwrap_or(d);
    let action = match &p { Some(v) => format!("{PROJECTS}{}/", esc(&v.id)), None => format!("{PROJECTS}new/") };
    let config = koenig_config(state, &get(|v| &v.body_lexical), &p.as_ref().map(|v| v.body_markup()).unwrap_or_default(), "Tell the story: the brief, the problem, what you tried, what you made, and what changed.");
    let images = p.as_ref().map(|v| v.images()).unwrap_or_default();
    let media = format!(
        r#"<div class="ec-media" id="ec-media">{items}<label class="ec-media-add" title="Upload pictures"><input type="file" accept="image/*" id="ec-upload" multiple>{plus}<span>Add</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>
<p class="muted ec-hint">The first picture is the cover in the grid and at the top of the page; the rest make the gallery. Drag to reorder.</p>"#,
        items = images.iter().map(|u| crate::ecom::media_item(u)).collect::<String>(),
        plus = svg("add"),
    );
    let mut client_opts = vec![(String::new(), "Not in the client list".to_string())];
    client_opts.extend(x.clients.iter().map(|c| (c.id.clone(), c.name.clone())));
    let facts = format!(
        r#"<div class="ec-grid2">{client}{client_name}</div><div class="ec-grid3">{year}{duration}{role}</div><div class="ec-grid2">{industry}{location}</div>{tools}"#,
        client = select("client_id", "Client", &client_opts, p.as_ref().and_then(|v| v.client_id.as_deref()).unwrap_or("")),
        client_name = field("client_name", "…or the client's name", &get(|v| &v.client_name), "text", r#" maxlength="120""#),
        year = field("year", "Year", &{ let y = num(|v| v.year, 0); if y > 0 { y.to_string() } else { String::new() } }, "number", r#" min="1900" max="2200""#),
        duration = field("duration", "Timeline", &get(|v| &v.duration), "text", r#" maxlength="60" placeholder="12 weeks""#),
        role = field("role", if s.studio() { "Our role" } else { "My role" }, &get(|v| &v.role), "text", r#" maxlength="160" placeholder="Lead designer""#),
        industry = field("industry", "Industry", &get(|v| &v.industry), "text", r#" maxlength="80""#),
        location = field("location", "Location", &get(|v| &v.location), "text", r#" maxlength="120""#),
        tools = field("tools", "Tools and technologies, comma-separated", &p.as_ref().map(|v| v.tools().join(", ")).unwrap_or_default(), "text", r#" maxlength="1500" placeholder="Figma, React, Blender""#),
    );
    let links = format!(
        r#"<div class="ec-grid2">{live}{repo}</div>{video}<p class="muted ec-hint">A YouTube, Vimeo or Loom link (or an .mp4) plays under the cover.</p>"#,
        live = field("live_url", "Live site or app", &get(|v| &v.live_url), "url", r#" maxlength="500""#),
        repo = field("repo_url", "Source code (GitHub…)", &get(|v| &v.repo_url), "url", r#" maxlength="500""#),
        video = field("video_url", "Video", &get(|v| &v.video_url), "url", r#" maxlength="500""#),
    );
    let metrics = p.as_ref().map(|v| v.metrics()).unwrap_or_default();
    let results = format!(
        r#"<div id="pf-metrics">{rows}</div><button class="btn text small" type="button" data-pf-add="pf-metrics" data-pf-tpl="pf-metric-tpl">{add} Add a result</button><template id="pf-metric-tpl">{tpl}</template>
<p class="muted ec-hint">Big numbers under the story: <i>+38%</i> sign-ups, <i>2.1M</i> users, <i>4.9★</i> rating.</p>"#,
        rows = if metrics.is_empty() { metric_row("", "") } else { metrics.iter().map(|m| metric_row(&m.value, &m.label)).collect() },
        add = svg("add"),
        tpl = metric_row("", ""),
    );
    let credits = p.as_ref().map(|v| v.credits()).unwrap_or_default();
    let credits_card = format!(
        r#"<div id="pf-credits">{rows}</div><button class="btn text small" type="button" data-pf-add="pf-credits" data-pf-tpl="pf-credit-tpl">{add} Add a credit</button><template id="pf-credit-tpl">{tpl}</template>
<p class="muted ec-hint">Who worked on it. A team member credited by name gets this project on their page.</p>"#,
        rows = if credits.is_empty() { credit_row("", "", "") } else { credits.iter().map(|c| credit_row(&c.name, &c.role, &c.url)).collect() },
        add = svg("add"),
        tpl = credit_row("", "", ""),
    );
    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}{prefix}<b>{slug}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>{t}{d}{h}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        prefix = esc(&s.paths.public_path("/portfolio/")),
        slug = esc(&get(|v| &v.slug)),
        st = esc(&{ let t = get(|v| &v.seo_title); if t.is_empty() { get(|v| &v.title) } else { t } }),
        sd = esc(&{ let d = get(|v| &v.seo_description); if d.is_empty() { get(|v| &v.summary) } else { d } }),
        t = field("seo_title", "Page title", &get(|v| &v.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = textarea("seo_description", "Meta description", &get(|v| &v.seo_description), 2, r#" maxlength="320" id="ec-seo-desc""#),
        h = field("slug", "Address on the site", &get(|v| &v.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*" id="ec-handle""#),
    );
    let has_password = p.as_ref().is_some_and(|v| !v.password_hash.is_empty());
    let status_card = format!(
        r#"{sel}<div class="pf-password">{pw}<p class="muted ec-hint">{pw_hint}</p>{clear}</div>{view}"#,
        sel = select("status", "Status", &rules::ProjectStatus::options(), p.as_ref().map(|v| v.status.as_str()).unwrap_or("draft")),
        pw = field("password", if has_password { "New password (blank keeps the current one)" } else { "Password" }, "", "text", r#" maxlength="200" autocomplete="off""#),
        pw_hint = "For “Password protected”: the project shows in the grid with a lock, and opens for anyone with the password — for work under NDA.",
        clear = if has_password { r#"<button class="btn text small" type="submit" form="pf-clear-password">Remove the password</button>"#.to_string() } else { String::new() },
        view = p.as_ref().filter(|v| v.status != "draft" && s.enabled).map(|v| format!(r#"<a class="btn text small" href="{}" target="_blank" rel="noopener">{} View on the site</a>"#, public_href(&s, &format!("/portfolio/{}/", v.slug)), svg("open"))).unwrap_or_default(),
    );
    let linked_cats = match &p { Some(v) => repo::project_category_ids(&state.db, &v.id).await, None => Vec::new() };
    let linked_svcs = match &p { Some(v) => repo::project_service_ids(&state.db, &v.id).await, None => Vec::new() };
    let org = format!(
        r#"<div class="ec-sublabel">Disciplines</div><div class="dr-checks">{cats}</div>{svc_head}<div class="dr-checks">{svcs}</div>{feat}{order}"#,
        cats = if x.categories.is_empty() { r#"<p class="muted" style="margin:0">None yet — <a href="/ghost/portfolio/disciplines/">add Branding, Web, Photography…</a></p>"#.to_string() } else { x.categories.iter().map(|c| check("category", &c.id, &c.name, linked_cats.contains(&c.id))).collect() },
        svc_head = if x.services.is_empty() { String::new() } else { r#"<div class="ec-sublabel">Services</div>"#.into() },
        svcs = x.services.iter().map(|v| check("service", &v.id, &v.name, linked_svcs.contains(&v.id))).collect::<String>(),
        feat = check("featured", "1", "Featured — first in the grid", num(|v| v.featured, 0) != 0),
        order = field("sort_order", "Order (lower first)", &num(|v| v.sort_order, 0).to_string(), "number", r#" min="0" max="9999""#),
    );
    let mut t_opts = vec![(String::new(), "None".to_string())];
    t_opts.extend(x.testimonials.iter().map(|t| (t.id.clone(), format!("{} — “{}…”", t.author, t.quote.chars().take(40).collect::<String>()))));
    let look = format!(
        r#"{layout}<label class="field fixed"><input type="color" name="accent_pick" value="{pick}" data-pf-accent><span>Accent colour</span></label><input type="hidden" name="accent" value="{accent}" id="pf-accent"><label class="ec-check"><input type="checkbox" id="pf-accent-none"{none}> No accent — use the theme's</label>"#,
        layout = select("layout", "Layout", &rules::Layout::options(), p.as_ref().map(|v| v.layout.as_str()).unwrap_or("case_study")),
        pick = esc(&{ let a = get(|v| &v.accent); if a.len() == 7 { a } else { "#111111".into() } }),
        accent = esc(&get(|v| &v.accent)),
        none = if get(|v| &v.accent).is_empty() { " checked" } else { "" },
    );
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{subtitle}{summary}
<div class="ec-sublabel">The story</div><div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="body_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback">{plain}</div></section>
{media}{facts}{links}{results}{credits}{seo}"#,
        title = field("title", "Project title", &get(|v| &v.title), "text", r#" required maxlength="200" id="ec-title""#),
        subtitle = field("subtitle", "Subtitle — one line under the title", &get(|v| &v.subtitle), "text", r#" maxlength="240""#),
        summary = textarea("summary", "Summary — for the grid and search engines", &get(|v| &v.summary), 2, r#" maxlength="400""#),
        config = esc(&config),
        plain = textarea("body", "The story (plain text)", &get(|v| &v.body), 8, r#" maxlength="60000""#),
        media = card("Pictures", "image", "pink", &media, "pf-media"),
        facts = card("Facts", "notes", "blue", &facts, "pf-facts"),
        links = card("Links & video", "link", "teal", &links, "pf-links"),
        results = card("Results", "trending", "green", &results, "pf-results"),
        credits = card("Credits", "group", "purple", &credits_card, "pf-credits-card"),
        seo = card("Search engine listing", "search", "slate", &seo, "pf-seo"),
    );
    let side = format!(
        "{st}{org}{testimonial}{look}{actions}",
        st = card("Status", "eye", "green", &status_card, "pf-status"),
        org = card("Organisation", "folder", "purple", &org, "pf-org"),
        testimonial = card("Testimonial", "comment", "amber", &format!(r#"{}<p class="muted ec-hint"><a href="/ghost/portfolio/testimonials/new/">Add a testimonial</a></p>"#, select("testimonial_id", "Quote shown with this project", &t_opts, p.as_ref().and_then(|v| v.testimonial_id.as_deref()).unwrap_or(""))), "pf-testimonial"),
        look = card("Look", "themes", "pink", &look, "pf-look"),
        actions = if p.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text" type="submit" form="pf-duplicate">{} Duplicate</button><button class="btn text danger" type="submit" form="pf-delete">{} Delete project</button></div></section>"#, svg("copy"), svg("delete")) } else { String::new() },
    );
    let heading = p.as_ref().map(|v| v.title.clone()).unwrap_or_else(|| "New project".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{PROJECTS}" title="Back to projects">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pf-project">{check} {save}</button></div></header>
{banner}{extra_forms}
<form id="pf-project" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script><script src="{ASSETS}/react-jsx.js" defer></script><script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script><script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script><script src="{ASSETS}/portfolio-editor.js?v={hash}" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&match &p { Some(v) => format!("{} · {} views", status_chip(&v.status).1, v.views), None => "A project or case study.".into() }),
        check = svg("check"),
        save = if p.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        extra_forms = p.as_ref().map(|v| format!(
            r#"<form method="post" action="{PROJECTS}{id}/delete/" id="pf-delete" data-confirm="Delete “{t}”|Its page goes. Testimonials tied to it stay."></form><form method="post" action="{PROJECTS}{id}/duplicate/" id="pf-duplicate"></form><form method="post" action="{PROJECTS}{id}/password/clear/" id="pf-clear-password" data-confirm="Remove the password?|The project becomes public if it was password protected."></form>"#,
            id = esc(&v.id), t = esc(&v.title))).unwrap_or_default(),
        hash = editor_script_hash(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::ProjectInput, String> {
    let x = repo::Lookup::load(&state.db).await;
    let mut input = rules::ProjectInput::from_form(pairs, |kind, id| match kind {
        "client" => x.clients.iter().any(|c| c.id == id),
        "category" => x.categories.iter().any(|c| c.id == id),
        "service" => x.services.iter().any(|c| c.id == id),
        "testimonial" => x.testimonials.iter().any(|c| c.id == id),
        _ => false,
    })?;
    let raw = pairs.iter().find(|(k, _)| k == "body_lexical").map(|(_, v)| v.as_str()).unwrap_or("");
    if let Some((lexical, html, plain)) = read_lexical(raw, 60_000)? {
        input.body_lexical = lexical;
        input.body_html = html;
        input.body = plain;
    }
    Ok(input)
}

pub(crate) async fn project_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{PROJECTS}new/"), "error", &m),
    };
    if input.status == "protected" && input.password.is_empty() {
        return flash_to(&format!("{PROJECTS}new/"), "error", "Set a password for a password-protected project.");
    }
    match repo::save_project(&state.db, None, &input).await {
        Ok(id) => flash_to(&format!("{PROJECTS}{id}/"), "ok", "Project created."),
        Err(e) => {
            tracing::error!("portfolio: creating a project failed: {}", e.message());
            flash_to(&format!("{PROJECTS}new/"), "error", "The project could not be saved.")
        }
    }
}

pub(crate) async fn project_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{PROJECTS}{id}/");
    let Some(existing) = repo::find_project(&state.db, &id).await else { return flash_to(PROJECTS, "error", "That project could not be found.") };
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    if input.status == "protected" && input.password.is_empty() && existing.password_hash.is_empty() {
        return flash_to(&back, "error", "Set a password for a password-protected project.");
    }
    match repo::save_project(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", if input.password.is_empty() { "Saved." } else { "Saved. The new password is in force; anyone who opened it before must enter it again." }),
        Err(e) => {
            tracing::error!("portfolio: saving a project failed: {}", e.message());
            flash_to(&back, "error", "The project could not be saved.")
        }
    }
}

pub(crate) async fn project_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_project(&state.db, &id).await {
        Ok(()) => flash_to(PROJECTS, "ok", "Project deleted."),
        Err(_) => flash_to(PROJECTS, "error", "That could not be deleted."),
    }
}

pub(crate) async fn project_duplicate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::duplicate_project(&state.db, &id).await {
        Ok(Some(new_id)) => flash_to(&format!("{PROJECTS}{new_id}/"), "ok", "Duplicated as a draft."),
        _ => flash_to(PROJECTS, "error", "That could not be duplicated."),
    }
}

pub(crate) async fn project_clear_password(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::clear_password(&state.db, &id).await {
        Ok(()) => flash_to(&format!("{PROJECTS}{id}/"), "ok", "Password removed."),
        Err(_) => flash_to(PROJECTS, "error", "That could not be changed."),
    }
}
