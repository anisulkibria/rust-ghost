//! Shows: the list (with each show's feed address and feed check), the show
//! editor, and importing a show from another host by its RSS address.

use super::episodes::{card, check, field, media_box, select, textarea};
use super::*;

pub(crate) async fn shows_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = podcast_settings(&state);
    let shows = repo::shows(&state.db, false).await;
    let counts = repo::episode_counts(&state.db).await;
    let mut rows = String::new();
    for sh in &shows {
        let (live, drafts, sched) = counts.get(&sh.id).copied().unwrap_or((0, 0, 0));
        let findings = rules::check(&facts(&state, sh).await);
        let blocking = findings.iter().filter(|(l, _)| *l == rules::Level::Blocking).count();
        let feed = vuuut_core::url::absolute(&state.site_url(), &public_href(&s, &format!("/podcast/{}/feed.xml", sh.slug)));
        let (chip, label) = match sh.status.as_str() { "active" => ("published", "Live"), "hidden" => ("tonal", "Hidden"), _ => ("draft", "Draft") };
        rows.push_str(&format!(
            r#"<div class="row dr-listing">{thumb}<div class="body"><span class="title"><a href="{SHOWS}{id}/">{t}</a> <span class="chip {chip}">{label}</span>{warn}</span>
<span class="detail">{live} published · {drafts} draft{dpl} · {sched} scheduled{cat}</span><span class="detail pc-feed"><code>{feed}</code> <button class="btn text small" type="button" data-pc-copy="{feed}">Copy feed</button></span></div>
<div class="trailing dr-actions"><a class="btn tonal small" href="{EPISODES}new/?show={id}">{add} Episode</a><a class="icon-btn" href="{SHOWS}{id}/" title="Edit">{edit}</a></div></div>"#,
            thumb = if sh.artwork.is_empty() { format!(r#"<span class="ic tint-{}">{}</span>"#, esc(&sh.tint), svg("mic")) } else { format!(r#"<img class="dr-thumb" src="{}" alt="" loading="lazy">"#, esc(&sh.artwork)) },
            id = esc(&sh.id),
            t = esc(&sh.title),
            warn = if blocking > 0 { format!(r#" <span class="chip error">{} to fix</span>"#, blocking) } else { String::new() },
            dpl = if drafts == 1 { "" } else { "s" },
            cat = if sh.category.is_empty() { String::new() } else { format!(" · {}", esc(&sh.category.replace(" > ", " › "))) },
            feed = esc(&feed),
            add = svg("add"),
            edit = svg("edit"),
        ));
    }
    page_for_role(
        actor.role,
        "Shows",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<div class="rise" style="--i:3">{body}</div>
<div class="section" id="import">Move a show here from another host</div>
<form class="card padded rise" style="--i:4" method="post" action="{SHOWS}import/"><p class="muted" style="margin-top:0">Paste the show's RSS feed address from Transistor, Buzzsprout, Anchor/Spotify for Creators, Libsyn, Podbean, Captivate, Simplecast — anywhere. Episodes, notes, artwork, seasons and numbers come across; the audio stays where it is hosted, and every episode keeps its ID, so listeners' apps don't download anything twice.</p>
<div class="ec-row">{url}{into}<button class="btn" type="submit">{imp} Import</button></div>
<p class="muted ec-hint">After importing, set a redirect at your old host (most call it “redirect feed” or “move podcast”) to this show's feed address so subscribers follow.</p></form>
<script src="{ASSETS}/podcast-editor.js?v={hash}" defer></script>"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, &format!(r#"<a class="btn" href="{SHOWS}new/">{} New show</a>"#, svg("add"))),
            tabs = tabs_for(&state, "shows", &actor).await,
            banner = flash_banner(&q),
            body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No shows yet.</p><a class="btn" href="{SHOWS}new/">{} New show</a></div>"#, svg("add")) } else { format!(r#"<div class="card rows">{rows}</div>"#) },
            url = field("url", "Feed address", "", "url", r#" required maxlength="1000" placeholder="https://feeds.example.com/my-show" style="flex:1""#),
            into = if shows.is_empty() { String::new() } else { select("into", "Into", &std::iter::once((String::new(), "A new show".to_string())).chain(shows.iter().map(|x| (x.id.clone(), x.title.clone()))).collect::<Vec<_>>(), "") },
            imp = svg("import"),
            hash = super::episodes::editor_script_hash(),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn show_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn show_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(sh) = repo::find_show(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That show could not be found.") };
    editor(&state, &actor, Some(sh), &q).await
}

/// The way into Apple Podcasts, Spotify and the rest: none of them take
/// uploads, each is given the feed once. One row per directory with where
/// to submit, what to do there, and the field for the show's page once it is
/// listed (that page becomes a "Listen on" button on the site).
fn get_listed(site_url: &str, feed: Option<&str>, links: &[(String, String)]) -> String {
    let row = |(k, submit, how): &(&str, &str, &str)| -> String {
        let index = *k == "podcastindex";
        let label = if index { "Podcast Index" } else { rules::platform_label(k) };
        let cur = links.iter().find(|(x, _)| x == k).map(|(_, u)| u.as_str()).unwrap_or("");
        let status = if index { String::new() } else if cur.is_empty() { r#"<span class="pc-dir-st">Not listed yet</span>"#.to_string() } else { format!(r#"<a class="pc-dir-st on" href="{}" target="_blank" rel="noopener">Listed</a>"#, esc(cur)) };
        let go = if submit.is_empty() || feed.is_none() { String::new() } else { format!(r#"<a class="btn tonal small" href="{}" target="_blank" rel="noopener">Submit the feed ↗</a>"#, esc(submit)) };
        let input = if index { String::new() } else { format!(r#"<input class="ec-in" type="url" name="link_{k}" value="{v}" maxlength="500" placeholder="The show's page on {l}, once it is listed" aria-label="{l} page">"#, v = esc(cur), l = esc(label)) };
        format!(r#"<div class="pc-dir"><div class="pc-dir-head"><b>{l}</b>{status}{go}</div><p class="muted">{h}</p>{input}</div>"#, l = esc(label), h = esc(how))
    };
    let (main, more): (Vec<_>, Vec<_>) = rules::DIRECTORIES.iter().partition(|(k, _, _)| matches!(*k, "apple" | "spotify" | "youtube" | "amazon" | "podcastindex"));
    let listed = rules::PLATFORMS.iter().filter(|(k, _)| links.iter().any(|(x, u)| x == k && !u.is_empty())).count();
    let top = match feed {
        Some(f) => format!(
            r#"<ol class="pc-steps"><li>Copy the show's RSS feed. <span class="pc-feed"><code>{f}</code><button class="btn text small" type="button" data-pc-copy="{f}">Copy</button></span></li><li>Submit it <b>once</b> to each directory below. Podcast apps don't take uploads — they read every new episode from the feed by themselves.</li><li>When a directory accepts the show, paste the show's page there into its box and save. The site then shows a “Listen on” button for it.</li></ol>{warn}<p class="muted ec-hint">{listed} of {total} apps linked. Before submitting, clear the problems in the feed check (in the sidebar). Apple and Spotify need artwork, a category and an owner email.</p>"#,
            f = esc(f),
            warn = if rules::publicly_reachable(site_url) { String::new() } else { format!(r#"<p class="pc-warn"><b>Directories can't read this feed yet.</b> The site's address is <code>{}</code>, which only works on this computer. Apple, Spotify and the others fetch the feed over the internet, so submit it once the site is live on its own domain.</p>"#, esc(site_url)) },
            total = rules::PLATFORMS.len(),
        ),
        None => r#"<p class="muted ec-hint" style="margin-top:0">Save the show first. Its RSS feed address appears here, ready to submit to Apple Podcasts, Spotify, YouTube and the rest.</p>"#.to_string(),
    };
    format!(
        r#"{top}<div class="pc-dirs">{m}</div><details class="pc-more"><summary>More apps ({n})</summary><div class="pc-dirs">{o}</div></details>"#,
        m = main.iter().map(|d| row(d)).collect::<String>(),
        n = more.len(),
        o = more.iter().map(|d| row(d)).collect::<String>(),
    )
}

async fn editor(state: &AppState, actor: &Actor, sh: Option<repo::Show>, q: &HashMap<String, String>) -> Response {
    let s = podcast_settings(state);
    let get = |f: fn(&repo::Show) -> &str| sh.as_ref().map(f).unwrap_or("").to_string();
    let num = |f: fn(&repo::Show) -> i64, d: i64| sh.as_ref().map(f).unwrap_or(d);
    let action = match &sh { Some(x) => format!("{SHOWS}{}/", esc(&x.id)), None => format!("{SHOWS}new/") };
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let cats: Vec<(String, String)> = std::iter::once((String::new(), "Choose…".to_string())).chain(rules::category_options()).collect();
    let langs: Vec<(String, String)> = rules::LANGUAGES.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
    let links = sh.as_ref().map(|x| x.links()).unwrap_or_default();
    let people = repo::people(&state.db).await;
    let hosts: Vec<(String, String)> = match &sh { Some(x) => repo::credits(&state.db, "show", &x.id).await.into_iter().map(|(p, r)| (p.id, r)).collect(), None => Vec::new() };
    let host_roles: Vec<(String, String)> = rules::Role::options().into_iter().filter(|(k, _)| k != "guest").collect();
    let hosts_card = if people.is_empty() {
        r#"<p class="muted" style="margin:0">Add the hosts under <a href="/ghost/podcast/people/new/">Hosts &amp; guests</a> first, then tick them here.</p>"#.to_string()
    } else {
        format!(r#"<div class="pc-people">{}</div>"#, people.iter().map(|p| {
            let cur = hosts.iter().find(|(id, _)| id == &p.id);
            let opts: String = host_roles.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), l = esc(l), s = if cur.map(|c| c.1.as_str()).unwrap_or("host") == v { " selected" } else { "" })).collect();
            format!(r#"<div class="pc-person"><label class="ec-check"><input type="checkbox" name="host" value="{id}"{c}> {img}<b>{n}</b></label><select name="host_role_{id}" class="ec-in pc-role">{opts}</select></div>"#,
                id = esc(&p.id), c = if cur.is_some() { " checked" } else { "" },
                img = if p.photo.is_empty() { format!(r#"<i class="pc-face">{}</i>"#, esc(&p.initials())) } else { format!(r#"<img class="pc-face" src="{}" alt="">"#, esc(&p.photo)) }, n = esc(&p.name))
        }).collect::<String>())
    };
    let feed = sh.as_ref().map(|x| vuuut_core::url::absolute(&state.site_url(), &public_href(&s, &format!("/podcast/{}/feed.xml", x.slug))));
    let tints: Vec<(String, String)> = rules::TINTS.iter().map(|t| (t.to_string(), t[..1].to_uppercase() + &t[1..])).collect();
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{subtitle}{desc}</section>
{directory}{owner}{listen}{advanced}{seo}"#,
        title = field("title", "Show name", &get(|x| &x.title), "text", r#" required maxlength="160" id="ec-title""#),
        subtitle = field("subtitle", "Tagline", &get(|x| &x.subtitle), "text", r#" maxlength="255""#),
        desc = textarea("description", "Description — what the show is about, who it's for, how often it comes out", &get(|x| &x.description), 6, r#" maxlength="4000""#),
        directory = card("Podcast directories", "library", "blue", &format!(
            r#"<div class="ec-grid2">{c1}{c2}</div><div class="ec-grid2">{lang}{kind}</div>{explicit}
<p class="muted ec-hint">Apple Podcasts and Spotify file the show by these. Serial shows are listened to in order (a story, a course); episodic ones newest first.</p>"#,
            c1 = select("category", "Category", &cats, &get(|x| &x.category)),
            c2 = select("category2", "Second category (optional)", &cats, &get(|x| &x.category2)),
            lang = select("language", "Language", &langs, sh.as_ref().map(|x| x.language.as_str()).unwrap_or("en")),
            kind = select("show_type", "Type", &rules::ShowType::options(), sh.as_ref().map(|x| x.show_type.as_str()).unwrap_or("episodic")),
            explicit = check("explicit", "1", "Contains explicit content", num(|x| x.explicit, 0) != 0),
        ), "pc-dir"),
        owner = card("Author & owner", "person", "purple", &format!(
            r#"{author}<div class="ec-grid2">{on}{oe}</div>{copy}<p class="muted ec-hint">The owner's email is in the feed so directories can confirm the show is yours; it isn't shown on the site.</p>"#,
            author = field("author", "Author or network (shown under the title in apps)", &{ let a = get(|x| &x.author); if a.is_empty() && sh.is_none() { site_title.clone() } else { a } }, "text", r#" maxlength="160""#),
            on = field("owner_name", "Owner name", &get(|x| &x.owner_name), "text", r#" maxlength="160""#),
            oe = field("owner_email", "Owner email", &get(|x| &x.owner_email), "email", r#" maxlength="191""#),
            copy = field("copyright", "Copyright line", &get(|x| &x.copyright), "text", r#" maxlength="200" placeholder="© 2026 …""#),
        ), "pc-owner"),
        listen = card("Get listed in podcast apps", "link", "green", &get_listed(&state.site_url(), feed.as_deref(), &links), "pc-links"),
        advanced = card("Support & Podcasting 2.0", "heart", "red", &format!(
            r#"<div class="ec-grid2">{fu}{fl}</div>{locked}{complete}{nf}<p class="muted ec-hint">The support link appears on the site and as podcast:funding in apps like Fountain and Podverse. Locking stops other hosts importing your feed. Set a new feed address only when moving the show away from this site.</p>{guid}"#,
            fu = field("funding_url", "Support link (Patreon, Ko-fi, membership…)", &get(|x| &x.funding_url), "url", r#" maxlength="500""#),
            fl = field("funding_label", "Button text", &get(|x| &x.funding_label), "text", r#" maxlength="80" placeholder="Support the show""#),
            locked = check("locked", "1", "Lock the feed (podcast:locked)", num(|x| x.locked, 0) != 0),
            complete = check("complete", "1", "The show has ended — no more episodes", num(|x| x.complete, 0) != 0),
            nf = field("new_feed_url", "Moved to a new feed address", &get(|x| &x.new_feed_url), "url", r#" maxlength="500""#),
            guid = sh.as_ref().map(|x| format!(r#"<p class="muted ec-hint">Podcast GUID: <code>{}</code></p>"#, esc(&x.guid))).unwrap_or_default(),
        ), "pc-adv"),
        seo = card("Search engine listing", "search", "slate", &format!(
            "{}{}{}",
            field("seo_title", "Page title", &get(|x| &x.seo_title), "text", r#" maxlength="70""#),
            textarea("seo_description", "Meta description", &get(|x| &x.seo_description), 2, r#" maxlength="320""#),
            field("slug", "Address on the site", &get(|x| &x.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*""#),
        ), "pc-seo"),
    );
    let findings = match &sh { Some(x) => Some(rules::check(&facts(state, x).await)), None => None };
    let side = format!(
        "{status}{art}{hosts}{feed}{del}",
        status = card("Status", "eye", "green", &format!("{}{}{}",
            select("status", "Status", &rules::ShowStatus::options(), sh.as_ref().map(|x| x.status.as_str()).unwrap_or("active")),
            select("tint", "Colour", &tints, sh.as_ref().map(|x| x.tint.as_str()).unwrap_or("purple")),
            field("sort_order", "Order on the podcast page", &num(|x| x.sort_order, 0).to_string(), "number", r#" min="0" max="9999""#)), "pc-status"),
        art = card("Artwork", "image", "pink", &format!(r#"{}<p class="muted ec-hint">Square JPG or PNG, 1400–3000 px. Apps show it everywhere the show appears.</p>"#, media_box(&get(|x| &x.artwork), "Artwork")), "pc-art"),
        hosts = card("Hosts", "group", "purple", &hosts_card, "pc-hosts"),
        feed = match (&feed, &findings) {
            (Some(f), Some(list)) => card("RSS feed", "link", "amber", &format!(r#"<p class="pc-feed"><code>{f}</code></p><button class="btn tonal small" type="button" data-pc-copy="{f}">Copy feed address</button>{check}<p class="muted ec-hint">Where to submit it: <a href='#pc-links'>Get listed in podcast apps</a>.</p>"#, f = esc(f), check = check_html(list)), "pc-feed"),
            _ => String::new(),
        },
        del = if sh.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="pc-delete">{} Delete show</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    let heading = sh.as_ref().map(|x| x.title.clone()).unwrap_or_else(|| "New show".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{SHOWS}" title="Back to shows">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pc-show">{check} {save}</button></div></header>
{banner}{del}
<form id="pc-show" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/ecom-editor.js" defer></script><script src="{ASSETS}/podcast-editor.js?v={hash}" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = if sh.is_some() { "The show's page, its feed and what apps show about it.".to_string() } else { "A podcast with its own page and RSS feed.".to_string() },
        check = svg("check"),
        save = if sh.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        del = sh.as_ref().map(|x| format!(r#"<form method="post" action="{SHOWS}{}/delete/" id="pc-delete" data-confirm="Delete “{}” and all its episodes?|The page and the feed go too. Subscribers stop getting episodes. This cannot be undone.">"#, esc(&x.id), esc(&x.title)) + "</form>").unwrap_or_default(),
        hash = super::episodes::editor_script_hash(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::ShowInput, String> {
    let people = repo::people(&state.db).await;
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image" && k != "artwork").cloned().collect();
    all.push(("artwork".into(), pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default()));
    rules::ShowInput::from_form(&all, |id| people.iter().any(|p| p.id == id))
}

pub(crate) async fn show_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{SHOWS}new/"), "error", &m),
    };
    match repo::save_show(&state.db, None, &input).await {
        Ok(id) => flash_to(&format!("{SHOWS}{id}/"), "ok", "Show created. Add its first episode next."),
        Err(_) => flash_to(&format!("{SHOWS}new/"), "error", "The show could not be saved."),
    }
}

pub(crate) async fn show_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{SHOWS}{id}/");
    if repo::find_show(&state.db, &id).await.is_none() {
        return flash_to(SHOWS, "error", "That show could not be found.");
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_show(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "The show could not be saved."),
    }
}

pub(crate) async fn show_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    match repo::delete_show(&state.db, &id).await {
        Ok(()) => flash_to(SHOWS, "ok", "Show deleted."),
        Err(_) => flash_to(SHOWS, "error", "That could not be deleted."),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ImportForm {
    url: String,
    into: String,
}

pub(crate) async fn show_import(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<ImportForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let url = vuuut_core::directory::clean(&f.url, 1000);
    if url.is_empty() {
        return flash_to(&format!("{SHOWS}#import"), "error", "Paste the show's feed address.");
    }
    let into = (!f.into.is_empty() && vuuut_core::ids::is_objectid(&f.into)).then_some(f.into.as_str());
    match vuuut_api::podcast::import_feed(&state, &url, into).await {
        Ok(done) => flash_to(&format!("{SHOWS}{}/", done.show_id), "ok", &format!("Imported {} episode{}{}.", done.added, if done.added == 1 { "" } else { "s" }, if done.skipped > 0 { format!(" ({} already here or without audio)", done.skipped) } else { String::new() })),
        Err(m) => flash_to(SHOWS, "error", &m),
    }
}
