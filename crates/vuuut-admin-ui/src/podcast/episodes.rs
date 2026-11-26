//! Episodes: the list, and the episode editor — the same two-column editor
//! as a product or a course. Down the middle: title and summary, the audio
//! (uploaded through the editor's own media endpoint, or a link to where it
//! is hosted), show notes in Koenig, video, chapters, transcript, guests and
//! the search listing. Down the side: status and publish time, show,
//! season/number/type, who may listen, sponsors.

use super::*;

pub(crate) fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

pub(crate) fn card(title: &str, icon: &str, tint: &str, body: &str, id: &str) -> String {
    format!(r#"<section class="card padded ec-card" id="{id}"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title))
}

pub(crate) fn check(name: &str, value: &str, label: &str, on: bool) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="{v}"{c}> {l}</label>"#, v = esc(value), c = if on { " checked" } else { "" }, l = esc(label))
}

pub(crate) fn select(name: &str, label: &str, opts: &[(String, String)], current: &str) -> String {
    let o: String = opts.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), s = if v == current { " selected" } else { "" }, l = esc(l))).collect();
    format!(r#"<label class="field fixed"><select name="{name}">{o}</select><span>{l}</span>{SELECT_CARET}</label>"#, l = esc(label))
}

pub(crate) fn textarea(name: &str, label: &str, value: &str, rows: u32, extra: &str) -> String {
    format!(r#"<label class="field"><textarea name="{name}" rows="{rows}" placeholder=" "{extra}>{v}</textarea><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

/// The Koenig editor's config for a description field.
pub(crate) fn koenig_config(state: &AppState, lexical: &str, legacy_html: &str, placeholder: &str) -> String {
    let initial = if !lexical.trim().is_empty() { Some(lexical.to_string()) } else { vuuut_render::lexical::wrap_existing(Some(""), Some(legacy_html)) };
    let initial = initial.map(|x| vuuut_core::url::base_assets(&vuuut_core::url::unbase_assets(&x)));
    serde_json::json!({
        "lexical": initial,
        "placeholder": placeholder,
        "siteUrl": state.site_url(),
        "siteTitle": state.settings.get_str("title").unwrap_or_default(),
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false, "stripeEnabled": false,
        "basePath": vuuut_core::url::subdir(),
        "unsplash": serde_json::Value::Null, "pinturaConfig": serde_json::Value::Null,
    })
    .to_string()
}

/// `(lexical, html, plain)` from the Koenig field, or None when it was not
/// mounted (no script) and the plain-text fallback is what was sent.
pub(crate) fn read_lexical(raw: &str, max_plain: usize) -> Result<Option<(String, String, String)>, String> {
    if !raw.trim().starts_with('{') {
        return Ok(None);
    }
    let lexical = vuuut_core::url::unbase_assets(raw);
    let html = vuuut_render::lexical::to_html(&lexical);
    if html.len() > 600_000 {
        return Err("The text is too long.".into());
    }
    let plain = vuuut_render::markdown::html_to_plaintext(&html);
    let empty = plain.trim().is_empty() && !html.contains("<img") && !html.contains("<iframe") && !html.contains("<figure");
    Ok(Some(if empty { (String::new(), String::new(), String::new()) } else { (lexical, html, vuuut_core::directory::clean_text(&plain, max_plain)) }))
}

pub(crate) fn media_box(image: &str, label: &str) -> String {
    format!(
        r#"<div class="ec-media lm-cover" id="ec-media">{items}<label class="ec-media-add" title="Upload a picture"><input type="file" accept="image/*" id="ec-upload">{plus}<span>{label}</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>"#,
        items = if image.is_empty() { String::new() } else { crate::ecom::media_item(image) },
        plus = svg("add"),
        label = esc(label),
    )
}

// ---------------------------------------------------------------------------
// the list
// ---------------------------------------------------------------------------

pub(crate) async fn episodes_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let tz = zone(&state);
    let shows = repo::shows(&state.db, false).await;
    let show = q.get("show").filter(|v| shows.iter().any(|s| &s.id == *v)).cloned();
    let status = q.get("status").map(String::as_str).filter(|v| matches!(*v, "draft" | "published" | "scheduled"));
    let search = q.get("q").map(|v| vuuut_core::directory::clean(v, 80)).unwrap_or_default();
    let page = q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).clamp(1, 10_000);
    let per = 30;
    let (rows, total) = repo::episodes(&state.db, &repo::EpisodeQuery {
        show_id: show.as_deref(), status, search: (!search.is_empty()).then_some(search.as_str()), limit: per, offset: (page - 1) * per, ..Default::default()
    }).await;
    let pages = (total + per - 1) / per;
    let keep = |p: i64| {
        let mut parts = vec![format!("page={p}")];
        if let Some(sh) = &show { parts.push(format!("show={sh}")); }
        if let Some(st) = status { parts.push(format!("status={st}")); }
        if !search.is_empty() { parts.push(format!("q={}", vuuut_core::social_media::encode(&search))); }
        format!("{EPISODES}?{}", parts.join("&"))
    };
    let show_opts: Vec<(String, String)> = std::iter::once((String::new(), "All shows".to_string())).chain(shows.iter().map(|s| (s.id.clone(), s.title.clone()))).collect();
    let status_opts: Vec<(String, String)> = [("", "Any status"), ("published", "Published"), ("scheduled", "Scheduled"), ("draft", "Drafts")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let body = if rows.is_empty() {
        format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">{}</p><a class="btn" href="{EPISODES}new/">{} New episode</a></div>"#, if total == 0 && search.is_empty() && show.is_none() && status.is_none() { "No episodes yet." } else { "Nothing matches." }, svg("add"))
    } else {
        format!(r#"<div class="card rows">{}</div>"#, rows.iter().map(|e| episode_line(e, &shows, tz)).collect::<String>())
    };
    page_for_role(
        actor.role,
        "Episodes",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="pc-filters rise" style="--i:2" method="get">{show_sel}{status_sel}<label class="field"><input name="q" value="{qv}" placeholder=" "><span>Search</span></label><button class="btn tonal" type="submit">Filter</button></form>
<p class="muted dr-lead">{count}</p><div class="rise" style="--i:3">{body}</div>{pager}"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, &format!(r#"<a class="btn" href="{EPISODES}new/{qs}">{} New episode</a>"#, svg("add"), qs = show.as_ref().map(|s| format!("?show={s}")).unwrap_or_default())),
            tabs = tabs_for(&state, "episodes", &actor).await,
            banner = flash_banner(&q),
            show_sel = select("show", "Show", &show_opts, show.as_deref().unwrap_or("")),
            status_sel = select("status", "Status", &status_opts, status.unwrap_or("")),
            qv = esc(&search),
            count = esc(&countable(total, "episode")),
            pager = if pages > 1 { format!(r#"<nav class="pc-pager">{}<span class="muted">Page {page} of {pages}</span>{}</nav>"#,
                if page > 1 { format!(r#"<a class="btn text" href="{}">← Newer</a>"#, esc(&keep(page - 1))) } else { "<span></span>".into() },
                if page < pages { format!(r#"<a class="btn text" href="{}">Older →</a>"#, esc(&keep(page + 1))) } else { "<span></span>".into() }) } else { String::new() },
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// the editor
// ---------------------------------------------------------------------------

pub(crate) async fn episode_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let shows = repo::shows(&state.db, false).await;
    if shows.is_empty() {
        return flash_to(&format!("{SHOWS}new/"), "error", "Create the show first — every episode belongs to one.");
    }
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn episode_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(e) = repo::find_episode(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That episode could not be found.") };
    editor(&state, &actor, Some(e), &q).await
}

fn chapter_row(time: &str, title: &str, url: &str) -> String {
    format!(
        r#"<div class="pc-chapter"><input class="ec-in" name="ch_time" value="{t}" placeholder="0:00" maxlength="9" aria-label="Start"><input class="ec-in" name="ch_title" value="{ti}" placeholder="Chapter title" maxlength="200" aria-label="Title"><input class="ec-in" name="ch_url" value="{u}" placeholder="Link (optional)" maxlength="500" aria-label="Link"><button class="icon-btn" type="button" data-pc-remove title="Remove">×</button></div>"#,
        t = esc(time), ti = esc(title), u = esc(url),
    )
}

async fn editor(state: &AppState, actor: &Actor, e: Option<repo::Episode>, q: &HashMap<String, String>) -> Response {
    let s = podcast_settings(state);
    let tz = zone(state);
    let shows = repo::shows(&state.db, false).await;
    let get = |f: fn(&repo::Episode) -> &str| e.as_ref().map(f).unwrap_or("").to_string();
    let num = |f: fn(&repo::Episode) -> i64, d: i64| e.as_ref().map(f).unwrap_or(d);
    let show_id = e.as_ref().map(|x| x.show_id.clone()).or_else(|| q.get("show").filter(|v| shows.iter().any(|s| &s.id == *v)).cloned()).unwrap_or_else(|| shows.first().map(|s| s.id.clone()).unwrap_or_default());
    let show = shows.iter().find(|x| x.id == show_id);
    let action = match &e { Some(x) => format!("{EPISODES}{}/", esc(&x.id)), None => format!("{EPISODES}new/") };
    let config = koenig_config(state, &get(|x| &x.notes_lexical), &e.as_ref().map(|x| x.notes_markup()).unwrap_or_default(), "Show notes — what's in the episode, links you mentioned, where to find your guest…");
    let audio = get(|x| &x.audio_url);
    let suggested_number = if e.is_none() { repo::next_number(&state.db, &show_id).await } else { num(|x| x.number, 0) };
    let audio_card = format!(
        r#"<div class="pc-audio" id="pc-audio" data-upload="{upload}">
<div class="pc-audio-drop"><label class="btn tonal"><input type="file" accept="audio/*,.mp3,.m4a,.ogg,.wav" id="pc-audio-file" hidden>{up} Upload audio</label><span class="muted" id="pc-audio-status">{status}</span></div>
{url}<div class="ec-grid2">{dur}{bytes}</div>
<audio controls preload="none" id="pc-audio-preview" src="{src}"{hide}></audio></div>
<p class="muted ec-hint">MP3 is what every app plays. Upload here, or paste the address of the MP3 where it is hosted now (Transistor, Buzzsprout, S3…). Already on Apple Podcasts? Paste the episode's Apple Podcasts link and the audio file behind it is filled in when you save. The length and size fill in from the file.</p>"#,
        upload = esc(&vuuut_core::url::base("/ghost/api/admin/media/upload/")),
        up = svg("upload"),
        status = if audio.is_empty() { "No audio yet" } else { "Audio attached" },
        url = field("audio_url", "Audio file address", &audio, "text", r#" maxlength="1000" id="pc-audio-url" inputmode="url""#),
        dur = field("duration", "Length (h:mm:ss)", &{ let d = num(|x| x.duration, 0); if d > 0 { rules::clock(d) } else { String::new() } }, "text", r#" maxlength="9" id="pc-duration" pattern="[0-9:\.]*""#),
        bytes = field("audio_bytes", "File size (bytes)", &{ let b = num(|x| x.audio_bytes, 0); if b > 0 { b.to_string() } else { String::new() } }, "number", r#" min="0" id="pc-bytes""#),
        src = esc(&if audio.starts_with('/') { vuuut_core::url::base(&audio) } else { audio.clone() }),
        hide = if audio.is_empty() { " hidden" } else { "" },
    );
    let chapters = e.as_ref().map(|x| x.chapters()).unwrap_or_default();
    let chapters_card = format!(
        r#"<div id="pc-chapters">{rows}</div><button class="btn text small" type="button" id="pc-chapter-add">{add} Add chapter</button>
<template id="pc-chapter-tpl">{tpl}</template>
<details class="ec-more"><summary>Paste chapters instead</summary>{paste}<p class="muted ec-hint">One per line — <code>00:00 Welcome</code>, <code>12:30 The interview</code>. Used when the rows above are empty.</p></details>
<p class="muted ec-hint">Chapters show on the episode page, jump the player, and go in the feed as Podcasting 2.0 chapters for apps that show them.</p>"#,
        rows = chapters.iter().map(|c| chapter_row(&rules::clock(c.start), &c.title, &c.url)).collect::<String>(),
        add = svg("add"),
        tpl = chapter_row("", "", ""),
        paste = textarea("chapters_text", "Chapters as text", "", 4, r#" maxlength="20000""#),
    );
    let transcript = get(|x| &x.transcript);
    let transcript_card = format!(
        r#"{ta}<label class="btn text small"><input type="file" accept=".vtt,.srt,.txt,text/vtt,text/plain" id="pc-transcript-file" hidden>{up} Load a .vtt, .srt or .txt file</label>
<p class="muted ec-hint">WebVTT or SRT keeps the timings (each line becomes a jump-to link), plain text works too — <code>Name: words</code> marks the speaker. Shown on the page, searchable, and linked in the feed.</p>"#,
        ta = textarea("transcript", "Transcript", &transcript, 8, r#" maxlength="600000" id="pc-transcript""#),
        up = svg("upload"),
    );
    let people = repo::people(&state.db).await;
    let credited: Vec<(String, String)> = match &e {
        Some(x) => repo::credits(&state.db, "episode", &x.id).await.into_iter().map(|(p, r)| (p.id, r)).collect(),
        None => Vec::new(),
    };
    let role_opts: Vec<(String, String)> = rules::Role::options();
    let people_card = if people.is_empty() {
        format!(r#"<p class="muted" style="margin:0">No hosts or guests yet. <a href="/ghost/podcast/people/new/">Add someone</a> — they get a page listing every episode they're on.</p>"#)
    } else {
        format!(
            r#"<div class="pc-people">{}</div><p class="muted ec-hint">Ticked people appear on the episode and in the feed (podcast:person). <a href="/ghost/podcast/people/new/">Add someone new</a></p>"#,
            people.iter().map(|p| {
                let cur = credited.iter().find(|(id, _)| id == &p.id);
                let opts: String = role_opts.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), l = esc(l), s = if cur.map(|c| c.1.as_str()).unwrap_or("guest") == v { " selected" } else { "" })).collect();
                format!(
                    r#"<div class="pc-person"><label class="ec-check"><input type="checkbox" name="person" value="{id}"{c}> {img}<b>{n}</b></label><select name="role_{id}" class="ec-in pc-role">{opts}</select></div>"#,
                    id = esc(&p.id), c = if cur.is_some() { " checked" } else { "" },
                    img = if p.photo.is_empty() { format!(r#"<i class="pc-face">{}</i>"#, esc(&p.initials())) } else { format!(r#"<img class="pc-face" src="{}" alt="">"#, esc(&p.photo)) },
                    n = esc(&p.name),
                )
            }).collect::<String>()
        )
    };
    let sponsors = repo::sponsors(&state.db).await;
    let sponsored = match &e { Some(x) => repo::episode_sponsor_ids(&state.db, &x.id).await, None => Vec::new() };
    let sponsor_card = if sponsors.is_empty() {
        r#"<p class="muted" style="margin:0">No sponsors yet. <a href="/ghost/podcast/sponsors/">Add one</a></p>"#.to_string()
    } else {
        format!(r#"<div class="dr-checks">{}</div>"#, sponsors.iter().filter(|x| x.active != 0 || sponsored.contains(&x.id)).map(|x| check("sponsor", &x.id, &x.name, sponsored.contains(&x.id))).collect::<String>())
    };
    let video_card = format!(
        r#"{f}<p class="muted ec-hint">A YouTube, Vimeo or Loom link shows the video version above the notes.</p>"#,
        f = field("video_url", "Video link (optional)", &get(|x| &x.video_url), "url", r#" maxlength="500" placeholder="https://youtube.com/watch?v=…""#),
    );
    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}{prefix}<b>{slug}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>{t}{d}{h}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        prefix = esc(&s.paths.public_path(&format!("/podcast/{}/", show.map(|x| x.slug.as_str()).unwrap_or("show")))),
        slug = esc(&get(|x| &x.slug)),
        st = esc(&{ let t = get(|x| &x.seo_title); if t.is_empty() { get(|x| &x.title) } else { t } }),
        sd = esc(&{ let d = get(|x| &x.seo_description); if d.is_empty() { get(|x| &x.summary) } else { d } }),
        t = field("seo_title", "Page title", &get(|x| &x.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = textarea("seo_description", "Meta description", &get(|x| &x.seo_description), 2, r#" maxlength="320" id="ec-seo-desc""#),
        h = field("slug", "Address on the site", &get(|x| &x.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*" id="ec-handle""#),
    );
    let published_local = e.as_ref().and_then(|x| x.published_at.as_deref()).and_then(vuuut_core::dates::from_db_string).map(|d| d.with_timezone(&tz).format("%Y-%m-%dT%H:%M").to_string()).unwrap_or_default();
    let status_card = format!(
        r#"{status}{when}<p class="muted ec-hint">Times are in {tz}. A future time schedules the episode: it appears on the site and in the feed then.</p>{view}"#,
        status = select("status", "Status", &rules::EpisodeStatus::options(), e.as_ref().map(|x| x.status.as_str()).unwrap_or("draft")),
        when = field("published_at", "Publish date", &published_local, "datetime-local", ""),
        tz = esc(tz.name()),
        view = e.as_ref().filter(|x| x.is_live() && s.enabled).and_then(|x| show.map(|sh| format!(r#"<a class="btn text small" href="{}" target="_blank" rel="noopener">{} View on the site</a>"#, public_href(&s, &format!("/podcast/{}/{}/", sh.slug, x.slug)), svg("open")))).unwrap_or_default(),
    );
    let show_opts: Vec<(String, String)> = shows.iter().map(|x| (x.id.clone(), x.title.clone())).collect();
    let numbering = format!(
        r#"{show}<div class="ec-grid2">{season}{number}</div>{kind}{explicit}{featured}"#,
        show = select("show_id", "Show", &show_opts, &show_id),
        season = field("season", "Season", &{ let n = num(|x| x.season, 0); if n > 0 { n.to_string() } else { String::new() } }, "number", r#" min="0" max="999""#),
        number = field("number", "Episode no.", &if suggested_number > 0 { suggested_number.to_string() } else { String::new() }, "number", r#" min="0" max="99999""#),
        kind = select("episode_type", "Type", &rules::EpisodeType::options(), e.as_ref().map(|x| x.episode_type.as_str()).unwrap_or("full")),
        explicit = check("explicit", "1", "Explicit content", num(|x| x.explicit, show.map(|x| x.explicit).unwrap_or(0)) != 0),
        featured = check("featured", "1", "Feature on the site", num(|x| x.featured, 0) != 0),
    );
    let access_card = format!(
        r#"{sel}<p class="muted ec-hint">Members-only episodes stay out of the public feed. Members hear them on the site and in their own private feed.</p>"#,
        sel = select("access", "Who can listen", &rules::Access::options(), e.as_ref().map(|x| x.access.as_str()).unwrap_or("public")),
    );
    let art = media_box(&get(|x| &x.artwork), "Artwork");
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{summary}
<div class="ec-sublabel">Show notes</div><div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="notes_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback">{plain}</div></section>
{audio}{people}{chapters}{transcript}{video}{seo}"#,
        title = field("title", "Episode title", &get(|x| &x.title), "text", r#" required maxlength="200" id="ec-title""#),
        summary = field("summary", "One-line summary (lists and podcast apps)", &get(|x| &x.summary), "text", r#" maxlength="400""#),
        config = esc(&config),
        plain = textarea("notes", "Show notes (plain text)", &get(|x| &x.notes), 6, r#" maxlength="40000""#),
        audio = card("Audio", "mic", "purple", &audio_card, "pc-audio-card"),
        people = card("Hosts & guests", "group", "pink", &people_card, "pc-people-card"),
        chapters = card("Chapters", "list", "blue", &chapters_card, "pc-chapters-card"),
        transcript = card("Transcript", "notes", "teal", &transcript_card, "pc-transcript-card"),
        video = card("Video", "play", "red", &video_card, "pc-video-card"),
        seo = card("Search engine listing", "search", "slate", &seo, "pc-seo"),
    );
    let side = format!(
        "{st}{num}{acc}{art}{sp}{del}",
        st = card("Publishing", "calendar", "green", &status_card, "pc-status"),
        num = card("Show & numbering", "mic", "purple", &numbering, "pc-num"),
        acc = card("Access", "lock", "amber", &access_card, "pc-access"),
        art = card("Episode artwork", "image", "pink", &format!(r#"{art}<p class="muted ec-hint">Optional — the show's artwork is used when empty. Square, 1400–3000 px.</p>"#), "pc-art"),
        sp = card("Sponsors", "campaign", "teal", &sponsor_card, "pc-sponsors"),
        del = if e.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="pc-delete">{} Delete episode</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    let heading = e.as_ref().map(|x| x.title.clone()).unwrap_or_else(|| "New episode".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{EPISODES}" title="Back to episodes">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pc-episode">{check} {save}</button></div></header>
{banner}{del}{import}
<form id="pc-episode" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script><script src="{ASSETS}/react-jsx.js" defer></script><script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script><script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script><script src="{ASSETS}/podcast-editor.js?v={hash}" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&match &e { Some(x) => format!("{}{}{}", show.map(|s| s.title.as_str()).unwrap_or(""), if x.label().is_empty() { String::new() } else { format!(" · {}", x.label()) }, if x.downloads > 0 { format!(" · {} downloads", x.downloads) } else { String::new() }), None => "Audio, notes and everything a podcast app shows.".into() }),
        check = svg("check"),
        save = if e.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        del = e.as_ref().map(|x| format!(r#"<form method="post" action="{EPISODES}{}/delete/" id="pc-delete" data-confirm="Delete “{}”|It disappears from the site and the feed, with its download history.">"#, esc(&x.id), esc(&x.title)) + "</form>").unwrap_or_default(),
        hash = editor_script_hash(),
        import = if e.is_none() { import_box(show) } else { String::new() },
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

/// On a new episode: episodes that already live on another host come across
/// from its RSS feed rather than one by one. Importing into an existing show
/// skips the episodes it already has, so running it again picks up only the
/// new ones.
fn import_box(show: Option<&repo::Show>) -> String {
    format!(
        r#"<details class="card padded rise pc-import"><summary><b>Already published on another host?</b> <span class="muted">Bring the episodes over from its RSS feed instead of adding them one by one.</span></summary>
<form method="post" action="{SHOWS}import/"><p class="muted" style="margin-top:0">Paste the show's RSS feed address from Transistor, Buzzsprout, Spotify for Creators (Anchor), Libsyn, Podbean, Captivate, Simplecast, RSS.com or any other host. Each host shows it on the show's settings or distribution page, usually labelled “RSS feed”. Titles, notes, artwork, seasons, numbers and dates come across. The audio stays where it is hosted. Episodes already here are skipped, so importing again later brings in only the new ones.</p>
<div class="ec-row">{url}{into}<button class="btn" type="submit">{imp} Import episodes</button></div></form></details>"#,
        url = field("url", "Feed address", "", "url", r#" required maxlength="1000" placeholder="https://feeds.example.com/my-show" style="flex:1""#),
        into = match show { Some(sh) => format!(r#"<input type="hidden" name="into" value="{}"><p class="muted ec-hint" style="margin:0">Into “{}”</p>"#, esc(&sh.id), esc(&sh.title)), None => String::new() },
        imp = svg("import"),
    )
}

pub(super) fn editor_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../../assets/podcast-editor.js")));
    &HASH
}

async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::EpisodeInput, String> {
    let shows = repo::shows(&state.db, false).await;
    let people = repo::people(&state.db).await;
    let sponsors = repo::sponsors(&state.db).await;
    let tz = zone(state);
    // The media box posts `image`; an episode calls it artwork.
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image" && k != "artwork").cloned().collect();
    all.push(("artwork".into(), pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default()));
    let mut input = rules::EpisodeInput::from_form(
        &all,
        |kind, id| match kind {
            "show" => shows.iter().any(|x| x.id == id),
            "person" => people.iter().any(|x| x.id == id),
            "sponsor" => sponsors.iter().any(|x| x.id == id),
            _ => false,
        },
        |local| {
            let naive = chrono::NaiveDateTime::parse_from_str(local, "%Y-%m-%dT%H:%M").ok()?;
            let at = chrono::TimeZone::from_local_datetime(&tz, &naive).earliest()?;
            Some(vuuut_core::dates::to_db_string(&at.with_timezone(&Utc)))
        },
    )?;
    // A podcast app's page pasted in place of the audio file: an Apple
    // episode link becomes its file, anything else is refused (see
    // `resolve_audio`). A plain file address is kept without any lookup.
    if input.audio_url.starts_with("http") {
        if let Some((audio, secs, art)) = vuuut_api::podcast::resolve_audio(&input.audio_url).await? {
            input.audio_url = audio;
            if input.duration == 0 {
                input.duration = secs;
            }
            if input.artwork.is_empty() {
                input.artwork = art;
            }
        }
    }
    let raw = pairs.iter().find(|(k, _)| k == "notes_lexical").map(|(_, v)| v.as_str()).unwrap_or("");
    if let Some((lexical, html, plain)) = read_lexical(raw, 40_000)? {
        input.notes_lexical = lexical;
        input.notes_html = html;
        input.notes = plain;
    }
    Ok(input)
}

pub(crate) async fn episode_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{EPISODES}new/"), "error", &m),
    };
    match repo::save_episode(&state.db, None, &input).await {
        Ok(id) => flash_to(&format!("{EPISODES}{id}/"), "ok", if input.status == "published" { "Episode published." } else { "Episode saved as a draft." }),
        Err(e) => {
            tracing::error!("podcast: creating an episode failed: {}", e.message());
            flash_to(&format!("{EPISODES}new/"), "error", "The episode could not be saved.")
        }
    }
}

pub(crate) async fn episode_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{EPISODES}{id}/");
    if repo::find_episode(&state.db, &id).await.is_none() {
        return flash_to(EPISODES, "error", "That episode could not be found.");
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_episode(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(e) => {
            tracing::error!("podcast: saving an episode failed: {}", e.message());
            flash_to(&back, "error", "The episode could not be saved.")
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct StatusForm {
    status: String,
}

pub(crate) async fn episode_status(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<StatusForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let status = if f.status == "published" { "published" } else { "draft" };
    match repo::set_episode_status(&state.db, &id, status).await {
        Ok(()) => flash_to(&format!("{EPISODES}{id}/"), "ok", if status == "published" { "Published." } else { "Unpublished." }),
        Err(_) => flash_to(EPISODES, "error", "That could not be changed."),
    }
}

pub(crate) async fn episode_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_episode(&state.db, &id).await {
        Ok(()) => flash_to(EPISODES, "ok", "Episode deleted."),
        Err(_) => flash_to(EPISODES, "error", "That could not be deleted."),
    }
}
