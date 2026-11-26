//! Podcast: what the admin and the public pages share.
//!
//! * [`settings`] — the tool's settings, read the one way.
//! * [`import_feed`] — bring a show over from another host by its RSS
//!   address. The fetch goes through [`crate::mentions::fetchable`], so an
//!   address on this server's own network is refused, redirects are not
//!   followed, and the body is capped. Show notes are passed through
//!   [`vuuut_core::comments::sanitize_rich_html`]: a foreign feed's HTML is
//!   somebody else's markup and is never stored as sent.
//! * [`notify_request`] — a guest application or listener question arrived.

use crate::AppState;
use vuuut_core::podcast::{self as rules, Settings};
use vuuut_db::repos::podcast as repo;

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The most a feed may be. Ten years of a daily show with long notes fits.
const MAX_FEED_BYTES: usize = 40 * 1024 * 1024;

async fn fetch(url: &str) -> Result<String, String> {
    let parsed = crate::mentions::fetchable(url).ok_or("That address cannot be fetched — use the feed's public https:// address.")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Vuuut podcast importer")
        .build()
        .map_err(|_| "The importer could not start.".to_string())?;
    let resp = client.get(parsed).send().await.map_err(|_| "The feed could not be reached.".to_string())?;
    if resp.status().is_redirection() {
        let to = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).unwrap_or("");
        return Err(format!("That address redirects to {to} — paste that address instead."));
    }
    if !resp.status().is_success() {
        return Err(format!("The feed answered {}.", resp.status().as_u16()));
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_FEED_BYTES) {
        return Err("That feed is too large to import.".into());
    }
    let bytes = resp.bytes().await.map_err(|_| "The feed could not be read.".to_string())?;
    if bytes.len() > MAX_FEED_BYTES {
        return Err("That feed is too large to import.".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub struct Imported {
    pub show_id: String,
    pub added: usize,
    pub skipped: usize,
}

/// The audio file behind what was pasted into an episode's audio box.
/// `Ok(None)`: it is already a file address, keep it. An Apple Podcasts
/// episode link becomes the file Apple lists for it, with its length and
/// artwork; any other podcast app's page is refused with a way forward,
/// because there is no file behind it for a player to play.
pub async fn resolve_audio(url: &str) -> Result<Option<(String, i64, String)>, String> {
    if let Some((show, episode)) = rules::apple_episode_ids(url) {
        // The episode's own page first: it names exactly this episode's audio.
        if let Ok(page) = fetch(url.trim()).await {
            if let Some((audio, secs)) = rules::apple_page_audio(&page) {
                return Ok(Some((audio, secs, String::new())));
            }
        }
        // Then Apple's lookup, small before large (a large ask can drop the
        // newest episodes).
        let mut reached = false;
        for limit in [25, 200] {
            let lookup = format!("https://itunes.apple.com/lookup?id={show}&entity=podcastEpisode&limit={limit}");
            if let Ok(body) = fetch(&lookup).await {
                reached = true;
                if let Some(found) = rules::apple_episode_audio(&body, &episode) {
                    return Ok(Some(found));
                }
            }
        }
        return Err(if reached {
            "Apple Podcasts doesn't give out that episode's audio file. Paste the MP3 address from your podcast host instead, or upload the file.".into()
        } else {
            "Apple Podcasts could not be reached to find that episode's audio file. Try again, or paste the MP3 address from your podcast host.".into()
        });
    }
    if let Some(app) = rules::listening_page(url) {
        return Err(format!("That is the episode's page on {app}, not its audio file — {app} plays it only inside its own app. Paste the MP3 address from your podcast host (on the episode's page there, often “Download” or “Copy audio URL”), or upload the file. Apple Podcasts episode links work too."));
    }
    Ok(None)
}

/// Creates the show (or adds to `into`, an existing show) from a feed.
/// Episodes keep their audio where it is hosted now and their guids, so
/// subscribers' apps see nothing new once the feed address is moved.
pub async fn import_feed(state: &AppState, url: &str, into: Option<&str>) -> Result<Imported, String> {
    let body = fetch(url).await?;
    let feed = rules::parse_feed(&body).ok_or("That address does not look like a podcast feed.")?;
    import_parsed(state, &feed, into).await
}

pub async fn import_parsed(state: &AppState, feed: &rules::ImportedShow, into: Option<&str>) -> Result<Imported, String> {
    let db = &state.db;
    let show_id = match into {
        Some(id) => repo::find_show(db, id).await.map(|s| s.id).ok_or("That show no longer exists.")?,
        None => {
            let input = rules::ShowInput {
                title: feed.title.clone(),
                subtitle: feed.subtitle.clone(),
                description: feed.description.clone(),
                artwork: feed.artwork.clone(),
                author: feed.author.clone(),
                owner_name: feed.owner_name.clone(),
                owner_email: feed.owner_email.clone(),
                category: feed.category.clone(),
                language: feed.language.clone(),
                explicit: feed.explicit,
                show_type: feed.show_type.clone(),
                copyright: feed.copyright.clone(),
                tint: "purple".into(),
                status: "active".into(),
                ..Default::default()
            };
            let id = repo::save_show(db, None, &input).await.map_err(|_| "The show could not be created.".to_string())?;
            if !feed.guid.is_empty() {
                let _ = sqlx::query("UPDATE podcast_shows SET guid = ? WHERE id = ?").bind(&feed.guid).bind(&id).execute(db).await;
            }
            id
        }
    };
    let mut added = 0;
    let mut skipped = 0;
    for e in &feed.episodes {
        let html = vuuut_core::comments::sanitize_rich_html(&e.html);
        let plain = vuuut_render::markdown::html_to_plaintext(&html);
        match repo::import_episode(db, &show_id, e, &html, &vuuut_core::directory::clean_text(&plain, 40_000)).await {
            Ok(true) => added += 1,
            Ok(false) => skipped += 1,
            Err(err) => {
                tracing::warn!("podcast import: an episode failed: {}", err.message());
                skipped += 1;
            }
        }
    }
    Ok(Imported { show_id, added, skipped })
}

/// Tells the team about a request from the site. Nothing is sent when no
/// address is set.
pub async fn notify_request(state: &AppState, r: &repo::Request, show_title: &str) {
    let s = settings(state);
    if s.notify_email.is_empty() {
        return;
    }
    let site = state.settings.get_str("title").unwrap_or_default();
    let admin = format!("{}/{}/podcast/requests/", state.site_url().trim_end_matches('/'), state.admin_slug());
    let kind = rules::RequestKind::from_key(&r.kind).label();
    let body = format!(
        r#"<h1 style="font-size:22px;margin:0 0 6px">{kind}</h1><p style="color:#55524b;margin:0 0 18px">{name} · <a href="mailto:{email}">{email}</a>{show}</p>
{topic}<p style="background:#f6f3ee;border-radius:10px;padding:12px 14px;margin:0 0 18px;white-space:pre-wrap">{msg}</p>{link}
<p style="margin:0"><a href="{admin}" style="display:inline-block;background:#3b2a6b;color:#fff;text-decoration:none;padding:12px 22px;border-radius:999px;font-weight:600">Open requests</a></p>"#,
        kind = esc(kind),
        name = esc(&r.name),
        email = esc(&r.email),
        show = if show_title.is_empty() { String::new() } else { format!(" · {}", esc(show_title)) },
        topic = if r.topic.is_empty() { String::new() } else { format!("<p style=\"margin:0 0 10px\"><b>{}</b></p>", esc(&r.topic)) },
        msg = esc(&r.message),
        link = if r.link.is_empty() { String::new() } else { format!(r#"<p style="margin:0 0 18px"><a href="{0}">{0}</a></p>"#, esc(&r.link)) },
        admin = esc(&admin),
    );
    let html = format!(
        r#"<!doctype html><html><body style="margin:0;background:#f4f2ee;font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1b1f1d"><div style="max-width:560px;margin:28px auto;background:#fff;border-radius:16px;padding:30px;border:1px solid #e7e2d9"><p style="margin:0 0 20px;font-weight:700">{}</p>{body}</div></body></html>"#,
        esc(&site)
    );
    let text = format!("{kind}\n\n{} <{}>\n{}\n\n{}\n\n{admin}", r.name, r.email, r.topic, r.message);
    let cfg = vuuut_mail::MailConfig::from_settings(&state.settings, &site, &state.site_url());
    let msg = vuuut_mail::Message { to: s.notify_email.clone(), subject: format!("{kind}: {}", r.name), html, text };
    tokio::spawn(async move {
        if let Err(e) = vuuut_mail::send(&cfg, &msg).await {
            tracing::warn!("podcast: request notice failed: {}", e.message());
        }
    });
}
