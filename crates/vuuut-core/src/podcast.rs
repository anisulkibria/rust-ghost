//! Podcast hosting & show websites: the rules.
//!
//! What Transistor, Buzzsprout, Captivate, Podbean, Libsyn and Castos give a
//! show, in one tool: shows with their artwork, category and the links to
//! every listening app; episodes with audio, seasons, numbers, trailers and
//! bonus episodes, show notes, chapters and transcripts; the hosts and the
//! guests; sponsors; members-only and paid episodes with a private feed per
//! member; download counts by day, episode, app and country; guest and
//! listener requests; and an RSS feed that Apple Podcasts, Spotify and the
//! Podcasting 2.0 apps accept.
//!
//! Pure. This module is the trust boundary: every form is rebuilt by
//! [`ShowInput`], [`EpisodeInput`], [`PersonInput`], [`SponsorInput`] or
//! [`RequestInput`], the feed is written by [`rss`] from plain structs with
//! every value escaped, and a feed from another host is read by
//! [`parse_feed`], which keeps only what it understands.

use crate::directory::{clean, clean_text, is_email, safe_image, safe_url, slugify};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

macro_rules! keyed {
    ($name:ident { $($variant:ident => $key:literal, $label:literal;)+ } default $def:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn key(self) -> &'static str { match self { $($name::$variant => $key),+ } }
            pub fn label(self) -> &'static str { match self { $($name::$variant => $label),+ } }
            pub fn from_key(k: &str) -> $name { Self::ALL.iter().copied().find(|x| x.key() == k.trim()).unwrap_or($name::$def) }
            pub fn options() -> Vec<(String, String)> { Self::ALL.iter().map(|x| (x.key().to_string(), x.label().to_string())).collect() }
        }
    };
}

keyed!(ShowType {
    Episodic => "episodic", "Episodic — newest episode first";
    Serial => "serial", "Serial — in order, by season";
} default Episodic);

keyed!(ShowStatus {
    Active => "active", "Live";
    Hidden => "hidden", "Hidden — by link and feed only";
    Draft => "draft", "Draft";
} default Draft);

keyed!(EpisodeType {
    Full => "full", "Full episode";
    Trailer => "trailer", "Trailer";
    Bonus => "bonus", "Bonus";
} default Full);

keyed!(Access {
    Public => "public", "Everyone";
    Members => "members", "Signed-in members";
    Paid => "paid", "Paying members";
} default Public);

keyed!(EpisodeStatus {
    Published => "published", "Published";
    Draft => "draft", "Draft";
} default Draft);

keyed!(Role {
    Host => "host", "Host";
    Cohost => "cohost", "Co-host";
    Guest => "guest", "Guest";
    Producer => "producer", "Producer";
    Editor => "editor", "Editor";
} default Guest);

keyed!(RequestKind {
    Guest => "guest", "I'd like to be a guest";
    Question => "question", "A question for the show";
    Feedback => "feedback", "Feedback";
    Sponsor => "sponsor", "Sponsorship";
} default Question);

keyed!(RequestStatus {
    New => "new", "New";
    Accepted => "accepted", "Accepted";
    Answered => "answered", "Answered";
    Declined => "declined", "Declined";
    Archived => "archived", "Archived";
} default New);

impl Access {
    /// Whether a visitor with this member status may listen.
    /// `None` = not signed in.
    pub fn allows(self, member_status: Option<&str>) -> bool {
        match self {
            Access::Public => true,
            Access::Members => member_status.is_some(),
            Access::Paid => matches!(member_status, Some(s) if s != "free"),
        }
    }
}

/// Apple Podcasts' categories: `(category, subcategories)`. Stored as
/// `"Arts"` or `"Arts > Design"`.
pub const CATEGORIES: &[(&str, &[&str])] = &[
    ("Arts", &["Books", "Design", "Fashion & Beauty", "Food", "Performing Arts", "Visual Arts"]),
    ("Business", &["Careers", "Entrepreneurship", "Investing", "Management", "Marketing", "Non-Profit"]),
    ("Comedy", &["Comedy Interviews", "Improv", "Stand-Up"]),
    ("Education", &["Courses", "How To", "Language Learning", "Self-Improvement"]),
    ("Fiction", &["Comedy Fiction", "Drama", "Science Fiction"]),
    ("Government", &[]),
    ("History", &[]),
    ("Health & Fitness", &["Alternative Health", "Fitness", "Medicine", "Mental Health", "Nutrition", "Sexuality"]),
    ("Kids & Family", &["Education for Kids", "Parenting", "Pets & Animals", "Stories for Kids"]),
    ("Leisure", &["Animation & Manga", "Automotive", "Aviation", "Crafts", "Games", "Hobbies", "Home & Garden", "Video Games"]),
    ("Music", &["Music Commentary", "Music History", "Music Interviews"]),
    ("News", &["Business News", "Daily News", "Entertainment News", "News Commentary", "Politics", "Sports News", "Tech News"]),
    ("Religion & Spirituality", &["Buddhism", "Christianity", "Hinduism", "Islam", "Judaism", "Religion", "Spirituality"]),
    ("Science", &["Astronomy", "Chemistry", "Earth Sciences", "Life Sciences", "Mathematics", "Natural Sciences", "Nature", "Physics", "Social Sciences"]),
    ("Society & Culture", &["Documentary", "Personal Journals", "Philosophy", "Places & Travel", "Relationships"]),
    ("Sports", &["Baseball", "Basketball", "Cricket", "Fantasy Sports", "Football", "Golf", "Hockey", "Rugby", "Running", "Soccer", "Swimming", "Tennis", "Volleyball", "Wilderness", "Wrestling"]),
    ("Technology", &[]),
    ("True Crime", &[]),
    ("TV & Film", &["After Shows", "Film History", "Film Interviews", "Film Reviews", "TV Reviews"]),
];

/// Every category as `(stored value, label)`.
pub fn category_options() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (top, subs) in CATEGORIES {
        out.push((top.to_string(), top.to_string()));
        for s in *subs {
            out.push((format!("{top} > {s}"), format!("{top} › {s}")));
        }
    }
    out
}

/// A stored category, or empty when it is not one of Apple's.
pub fn clean_category(raw: &str) -> String {
    let v = raw.trim();
    if category_options().iter().any(|(k, _)| k == v) { v.to_string() } else { String::new() }
}

pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"), ("en-us", "English (US)"), ("en-gb", "English (UK)"), ("es", "Spanish"), ("fr", "French"), ("de", "German"),
    ("it", "Italian"), ("pt", "Portuguese"), ("pt-br", "Portuguese (Brazil)"), ("nl", "Dutch"), ("sv", "Swedish"), ("da", "Danish"),
    ("no", "Norwegian"), ("fi", "Finnish"), ("pl", "Polish"), ("tr", "Turkish"), ("ru", "Russian"), ("ar", "Arabic"), ("hi", "Hindi"),
    ("bn", "Bengali"), ("ur", "Urdu"), ("id", "Indonesian"), ("ja", "Japanese"), ("ko", "Korean"), ("zh", "Chinese"),
];

/// The listening apps a show links to: `(key, label)`. Stored as JSON
/// `{key: url}` in `podcast_shows.links`.
pub const PLATFORMS: &[(&str, &str)] = &[
    ("apple", "Apple Podcasts"), ("spotify", "Spotify"), ("youtube", "YouTube"), ("amazon", "Amazon Music"),
    ("overcast", "Overcast"), ("pocketcasts", "Pocket Casts"), ("castbox", "Castbox"), ("iheart", "iHeartRadio"),
    ("deezer", "Deezer"), ("podcastaddict", "Podcast Addict"), ("castro", "Castro"), ("goodpods", "Goodpods"),
    ("fountain", "Fountain"), ("pandora", "Pandora"), ("tunein", "TuneIn"), ("patreon", "Patreon"),
];

/// How a show gets into each app: none of them take uploads — each is given
/// the RSS feed once and reads new episodes from it. `(key, submit page,
/// what to do)`; an empty submit page means the app lists shows it finds in
/// Apple's directory or the Podcast Index, so there is nothing to submit.
/// `podcastindex` is not a "Listen on" link, only a place to submit.
pub const DIRECTORIES: &[(&str, &str, &str)] = &[
    ("apple", "https://podcastsconnect.apple.com/", "Sign in with an Apple Account, choose + → New Show → Add a show with an RSS feed, paste the feed. Review usually takes a few days. Overcast, Pocket Casts, Castro and most other apps find shows through Apple's directory."),
    ("spotify", "https://creators.spotify.com/", "Choose Get started → find an existing podcast, paste the feed. Spotify emails a code to the owner email in the feed to prove the show is yours."),
    ("youtube", "https://studio.youtube.com/", "In YouTube Studio choose Create → New podcast → Submit an RSS feed. YouTube makes a video of each episode with the artwork; not offered in every country yet."),
    ("amazon", "https://podcasters.amazon.com/", "Add your podcast and paste the feed. Puts the show on Amazon Music and Audible."),
    ("podcastindex", "https://podcastindex.org/add", "Paste the feed. The open index behind Fountain, Podverse, Podcast Guru, Castamatic and other Podcasting 2.0 apps."),
    ("pocketcasts", "https://pocketcasts.com/submit/", "Paste the feed. Usually listed within an hour."),
    ("podcastaddict", "https://podcastaddict.com/submit", "Paste the feed."),
    ("deezer", "https://podcasters.deezer.com/", "Create an account and add the feed."),
    ("iheart", "https://www.iheart.com/content/submit-your-podcast/", "Fill in the form with the feed."),
    ("overcast", "", "Nothing to submit: appears once Apple Podcasts lists the show. Search for it in Overcast and paste its page."),
    ("castro", "", "Nothing to submit: appears once Apple Podcasts lists the show."),
    ("goodpods", "", "Nothing to submit: appears once Apple Podcasts lists the show."),
    ("castbox", "", "Usually appears on its own once Apple Podcasts lists the show; search for it and paste its page."),
    ("fountain", "", "Nothing to submit: appears once the Podcast Index lists the show."),
    ("pandora", "", "Paste its page once the show is there."),
    ("tunein", "", "Paste its page once the show is there."),
    ("patreon", "", "Your Patreon page, if listeners can support the show there."),
];

/// Whether directories on the internet can reach this address at all.
pub fn publicly_reachable(site_url: &str) -> bool {
    let host = site_url.split("://").nth(1).unwrap_or(site_url).split(['/', ':']).next().unwrap_or("").to_ascii_lowercase();
    !(host.is_empty() || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".test") || host.starts_with("127.") || host.starts_with("10.") || host.starts_with("192.168.") || host == "0.0.0.0" || host == "[::1]")
}

pub const TINTS: [&str; 8] = ["purple", "pink", "blue", "teal", "green", "amber", "red", "slate"];

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "podcast_enabled";
pub const SETTING_TITLE: &str = "podcast_title";
pub const SETTING_INTRO: &str = "podcast_intro";
pub const SETTING_BASE_PATH: &str = "podcast_base_path";
pub const SETTING_COUNT_DOWNLOADS: &str = "podcast_count_downloads";
pub const SETTING_PER_PAGE: &str = "podcast_per_page";
pub const SETTING_NOTIFY_EMAIL: &str = "podcast_notify_email";
pub const SETTING_REQUESTS: &str = "podcast_requests";

pub const DEFAULT_BASE: &str = "podcast";
pub const DEFAULT_TITLE: &str = "Podcasts";
/// The podcast pages' own addresses; never a show's slug.
pub const RESERVED_SLUGS: &[&str] = &["assets", "play", "people", "private", "feed", "search"];
/// Episode slugs that would collide with a show's own pages.
pub const RESERVED_EPISODE_SLUGS: &[&str] = &["feed", "contact", "private", "season", "assets", "feed-xml"];
/// A form filled faster than this is a bot.
pub const MIN_FILL_MS: i64 = 2500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub base: String,
}

impl Default for Paths {
    fn default() -> Self {
        Paths { base: DEFAULT_BASE.into() }
    }
}

impl Paths {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        Self::new(&get(SETTING_BASE_PATH).unwrap_or_default()).unwrap_or_default()
    }
    pub fn new(base: &str) -> Option<Self> {
        let b = base.trim().trim_matches('"').trim_matches('/');
        if b.is_empty() {
            return Some(Self::default());
        }
        let clean = crate::ecom::clean_address(b)?;
        if matches!(clean.split('/').next().unwrap_or_default(), "shop" | "directory" | "courses" | "book" | "portfolio" | "properties") {
            return None;
        }
        Some(Paths { base: clean })
    }
    pub fn public_path(&self, internal: &str) -> String {
        if self.base == DEFAULT_BASE {
            return internal.into();
        }
        match internal.strip_prefix("/podcast/") {
            Some(tail) => format!("/{}/{tail}", self.base),
            None if internal == "/podcast" => format!("/{}", self.base),
            None => internal.into(),
        }
    }
    pub fn internal_path(&self, public: &str) -> Option<String> {
        if self.base == DEFAULT_BASE {
            return None;
        }
        let rest = public.strip_prefix('/')?.strip_prefix(self.base.as_str())?;
        if rest.is_empty() {
            return Some("/podcast".into());
        }
        rest.strip_prefix('/').map(|t| format!("/podcast/{t}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub title: String,
    pub intro: String,
    /// Count each listen (deduplicated per listener, episode and day).
    pub count_downloads: bool,
    pub per_page: i64,
    /// Where guest and listener requests are announced.
    pub notify_email: String,
    /// Show the "be a guest / ask a question" form.
    pub requests: bool,
    pub paths: Paths,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let raw = |k: &str| get(k).map(|v| v.trim().trim_matches('"').to_string());
        let flag = |k: &str, d: bool| match raw(k).as_deref() {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => d,
        };
        let title = clean(&raw(SETTING_TITLE).unwrap_or_default(), 80);
        let notify = raw(SETTING_NOTIFY_EMAIL).unwrap_or_default();
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            title: if title.is_empty() { DEFAULT_TITLE.into() } else { title },
            intro: clean(&raw(SETTING_INTRO).unwrap_or_default(), 400),
            count_downloads: flag(SETTING_COUNT_DOWNLOADS, true),
            per_page: raw(SETTING_PER_PAGE).and_then(|v| v.parse::<i64>().ok()).map(|n| n.clamp(4, 100)).unwrap_or(12),
            notify_email: if is_email(&notify) { notify } else { String::new() },
            requests: flag(SETTING_REQUESTS, true),
            paths: Paths::read(&get),
        }
    }
}

// ---------------------------------------------------------------------------
// durations, sizes and labels
// ---------------------------------------------------------------------------

/// `"1:02:03"`, `"62:03"`, `"3723"`, `"62m"` → seconds. Anything else: `None`.
pub fn parse_duration(raw: &str) -> Option<i64> {
    let v = raw.trim();
    if v.is_empty() {
        return Some(0);
    }
    if let Some(m) = v.strip_suffix('m').or_else(|| v.strip_suffix("min")) {
        return m.trim().parse::<i64>().ok().filter(|n| (0..=6000).contains(n)).map(|n| n * 60);
    }
    let parts: Vec<&str> = v.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) || p.len() > 6) {
        // "123.45" seconds, as some feeds write it
        return v.parse::<f64>().ok().filter(|f| f.is_finite() && *f >= 0.0 && *f < 360_000.0).map(|f| f.round() as i64);
    }
    let mut total: i64 = 0;
    for (i, p) in parts.iter().enumerate() {
        let n: i64 = p.parse().ok()?;
        if i > 0 && n >= 60 {
            return None;
        }
        total = total * 60 + n;
    }
    (total < 360_000).then_some(total)
}

/// `3723` → `"1:02:03"`, `123` → `"2:03"`.
pub fn clock(secs: i64) -> String {
    let s = secs.max(0);
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// `3723` → `"1 hr 2 min"`, `95` → `"2 min"`, `30` → `"30 sec"`.
pub fn duration_label(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    if secs < 60 {
        return format!("{secs} sec");
    }
    let mins = (secs + 30) / 60;
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

/// `"S2 · E14"`, `"E14"`, `"Trailer"`, `"Bonus"` or empty.
pub fn episode_label(season: i64, number: i64, kind: &str) -> String {
    let mut parts = Vec::new();
    match EpisodeType::from_key(kind) {
        EpisodeType::Trailer => parts.push("Trailer".to_string()),
        EpisodeType::Bonus => parts.push("Bonus".to_string()),
        EpisodeType::Full => {}
    }
    if season > 0 {
        parts.push(format!("S{season}"));
    }
    if number > 0 {
        parts.push(format!("E{number}"));
    }
    parts.join(" · ")
}

/// `1_234_567` → `"1.2 MB"`.
pub fn size_label(bytes: i64) -> String {
    if bytes <= 0 {
        return String::new();
    }
    let b = bytes as f64;
    if b >= 1e9 { format!("{:.1} GB", b / 1e9) } else if b >= 1e6 { format!("{:.1} MB", b / 1e6) } else { format!("{:.0} KB", (b / 1e3).max(1.0)) }
}

/// The audio's type from its address. Unknown → MP3, which is what nearly
/// every podcast is and what every app plays.
pub fn audio_type(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    if path.ends_with(".m4a") || path.ends_with(".mp4") || path.ends_with(".aac") {
        "audio/x-m4a"
    } else if path.ends_with(".ogg") || path.ends_with(".oga") || path.ends_with(".opus") {
        "audio/ogg"
    } else if path.ends_with(".wav") {
        "audio/wav"
    } else if path.ends_with(".flac") {
        "audio/flac"
    } else {
        "audio/mpeg"
    }
}

/// The extension the download address ends in: `/podcast/play/<id>.mp3`.
pub fn audio_extension(url: &str) -> &'static str {
    match audio_type(url) {
        "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "audio/flac" => "flac",
        _ => "mp3",
    }
}

/// An audio address: http(s), or a file in this site's content folder.
pub fn safe_audio(raw: &str) -> Option<String> {
    let v = clean(raw, 1000);
    if v.is_empty() {
        return Some(v);
    }
    if (v.starts_with("/content/media/") || v.starts_with("/content/files/")) && !v.contains("..") && !v.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>' | '\\')) {
        return Some(v);
    }
    safe_url(&v)
}

/// An Apple Podcasts episode link (`…/id1200361736?i=1000803965110`) as
/// `(show id, episode id)`. Such a link is the episode's page, not its
/// audio; Apple's lookup turns it into the file (see [`apple_episode_audio`]).
pub fn apple_episode_ids(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix("https://").or_else(|| url.trim().strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !host.eq_ignore_ascii_case("podcasts.apple.com") && !host.eq_ignore_ascii_case("itunes.apple.com") {
        return None;
    }
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let digits = |s: &str| (!s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.to_string());
    let show = path.split('/').filter_map(|seg| seg.strip_prefix("id")).find_map(digits)?;
    let episode = query.split(['&', '#']).find_map(|kv| kv.strip_prefix("i=")).and_then(digits)?;
    Some((show, episode))
}

/// What Apple's lookup (`/lookup?id=SHOW&entity=podcastEpisode`) says about
/// one episode: `(audio address, length in seconds, artwork)`.
pub fn apple_episode_audio(lookup_json: &str, episode_id: &str) -> Option<(String, i64, String)> {
    let v: serde_json::Value = serde_json::from_str(lookup_json).ok()?;
    let hit = v.get("results")?.as_array()?.iter().find(|r| r.get("trackId").map(|t| t.to_string()) == Some(episode_id.to_string()))?;
    let audio = safe_url(hit.get("episodeUrl")?.as_str()?)?;
    let secs = hit.get("trackTimeMillis").and_then(|m| m.as_i64()).map(|m| m / 1000).unwrap_or(0);
    let art = hit.get("artworkUrl600").or_else(|| hit.get("artworkUrl160")).and_then(|a| a.as_str()).and_then(safe_url).unwrap_or_default();
    Some((audio, secs, art))
}

/// The audio on an Apple Podcasts episode page: the page carries the
/// episode's `streamUrl` and `duration`. Used only when the page names exactly
/// one stream, so a page listing several episodes can't hand over the wrong
/// one. Apple's lookup API is the fallback, but it is unreliable: asked for
/// 200 episodes it can answer with fewer and leave out the newest.
pub fn apple_page_audio(html: &str) -> Option<(String, i64)> {
    let mut streams: Vec<String> = Vec::new();
    for part in html.split("\"streamUrl\":\"").skip(1) {
        let raw = part.split('"').next().unwrap_or("");
        let url = raw.replace("\\u0026", "&").replace("\\/", "/");
        if !streams.contains(&url) {
            streams.push(url);
        }
    }
    if streams.len() != 1 {
        return None;
    }
    let audio = safe_url(&streams[0])?;
    // The first `"duration":` is schema.org's "PT27M40S"; the first plain
    // number of seconds is the one wanted.
    let secs = html.split("\"duration\":").skip(1).find_map(|r| r.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<i64>().ok()).filter(|d| *d > 0 && *d < 86_400 * 2).unwrap_or(0);
    Some((audio, secs))
}

/// A podcast app's page pasted where the audio file belongs. Those pages
/// play only inside their own app; there is no file behind them to give a
/// player (Apple episode links are the exception, handled above).
pub fn listening_page(url: &str) -> Option<&'static str> {
    let rest = url.trim().strip_prefix("https://").or_else(|| url.trim().strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    Some(match host {
        "podcasts.apple.com" | "itunes.apple.com" => "Apple Podcasts",
        "open.spotify.com" | "spotify.link" | "podcasters.spotify.com" | "creators.spotify.com" => "Spotify",
        "youtube.com" | "m.youtube.com" | "youtu.be" | "music.youtube.com" => "YouTube",
        "music.amazon.com" | "music.amazon.co.uk" | "audible.com" => "Amazon Music",
        "overcast.fm" => "Overcast",
        "pca.st" | "pocketcasts.com" | "play.pocketcasts.com" => "Pocket Casts",
        "castbox.fm" => "Castbox",
        "podcastaddict.com" => "Podcast Addict",
        "iheart.com" => "iHeartRadio",
        "deezer.com" => "Deezer",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// chapters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub start: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

pub const MAX_CHAPTERS: usize = 200;

/// Rows from the editor (`ch_time`, `ch_title`, `ch_url`, in order), or
/// lines of text — `00:00 Welcome`, `12:30 - The interview` — pasted from a
/// description. Sorted by start, duplicates of a start dropped.
pub fn chapters_from_rows(times: &[String], titles: &[String], urls: &[String]) -> Result<Vec<Chapter>, String> {
    let mut out: Vec<Chapter> = Vec::new();
    for (i, t) in times.iter().enumerate() {
        let title = clean(titles.get(i).map(String::as_str).unwrap_or(""), 200);
        if t.trim().is_empty() && title.is_empty() {
            continue;
        }
        let start = parse_duration(t).ok_or_else(|| format!("Chapter {}: “{}” is not a time like 12:30.", i + 1, clean(t, 20)))?;
        if title.is_empty() {
            return Err(format!("Chapter {} needs a title.", i + 1));
        }
        let url = safe_url(urls.get(i).map(String::as_str).unwrap_or("")).ok_or_else(|| format!("Chapter {}: the link must start with https://", i + 1))?;
        out.push(Chapter { start, title, url });
    }
    Ok(tidy_chapters(out))
}

pub fn chapters_from_text(text: &str) -> Vec<Chapter> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim().trim_start_matches(['(', '[']);
        let (time, rest) = match l.split_once(char::is_whitespace) {
            Some(x) => x,
            None => continue,
        };
        let time = time.trim_end_matches([')', ']', ':', '-']);
        if !time.contains(':') {
            continue;
        }
        let Some(start) = parse_duration(time) else { continue };
        let title = clean(rest.trim().trim_start_matches(['-', '–', '—', ')', ']', ':', '|']).trim(), 200);
        if !title.is_empty() {
            out.push(Chapter { start, title, url: String::new() });
        }
    }
    tidy_chapters(out)
}

fn tidy_chapters(mut v: Vec<Chapter>) -> Vec<Chapter> {
    v.sort_by_key(|c| c.start);
    v.dedup_by_key(|c| c.start);
    v.truncate(MAX_CHAPTERS);
    v
}

pub fn chapters_from_json(raw: &str) -> Vec<Chapter> {
    serde_json::from_str::<Vec<Chapter>>(raw).map(tidy_chapters).unwrap_or_default()
}

/// The Podcasting 2.0 JSON chapters file (`application/json+chapters`).
pub fn chapters_document(chapters: &[Chapter]) -> serde_json::Value {
    serde_json::json!({
        "version": "1.2.0",
        "chapters": chapters.iter().map(|c| {
            let mut j = serde_json::json!({ "startTime": c.start, "title": c.title });
            if !c.url.is_empty() { j["url"] = serde_json::json!(c.url); }
            j
        }).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// transcripts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    Vtt,
    Srt,
    Text,
}

impl TranscriptFormat {
    pub fn mime(self) -> &'static str {
        match self {
            TranscriptFormat::Vtt => "text/vtt",
            TranscriptFormat::Srt => "application/x-subrip",
            TranscriptFormat::Text => "text/plain",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            TranscriptFormat::Vtt => "vtt",
            TranscriptFormat::Srt => "srt",
            TranscriptFormat::Text => "txt",
        }
    }
}

pub const MAX_TRANSCRIPT: usize = 600_000;

pub fn transcript_format(t: &str) -> TranscriptFormat {
    let head = t.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        TranscriptFormat::Vtt
    } else if head.lines().take(4).any(|l| l.contains("-->") && l.contains(',')) {
        TranscriptFormat::Srt
    } else {
        TranscriptFormat::Text
    }
}

/// One cue of a transcript as a page shows it: when (if timed), who (if a
/// `Name:` or `<v Name>` speaker is given) and what was said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start: Option<i64>,
    pub speaker: String,
    pub text: String,
}

fn cue_time(s: &str) -> Option<i64> {
    // 00:01:02.345 or 00:01:02,345 or 01:02.345
    let t = s.trim().split([' ', '\t']).next()?.replace(',', ".");
    let whole = t.split('.').next()?;
    parse_duration(whole)
}

/// Turns a transcript into cues, merging consecutive cues from the same
/// speaker so a page reads as paragraphs rather than subtitle lines.
pub fn cues(t: &str) -> Vec<Cue> {
    let fmt = transcript_format(t);
    let mut out: Vec<Cue> = Vec::new();
    let push = |out: &mut Vec<Cue>, start: Option<i64>, raw: &str| {
        let mut text = raw.trim().to_string();
        let mut speaker = String::new();
        if let Some(rest) = text.strip_prefix("<v ") {
            if let Some((who, said)) = rest.split_once('>') {
                speaker = clean(who, 80);
                text = said.replace("</v>", "").trim().to_string();
            }
        } else if let Some((who, said)) = text.split_once(": ") {
            if !who.is_empty() && who.len() <= 40 && who.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '.' || c == '-' || c == '\'') && who.split_whitespace().count() <= 4 {
                speaker = who.trim().to_string();
                text = said.trim().to_string();
            }
        }
        // strip any other cue markup
        let text: String = {
            let mut s = String::new();
            let mut depth = 0;
            for c in text.chars() {
                match c {
                    '<' => depth += 1,
                    '>' if depth > 0 => depth -= 1,
                    _ if depth == 0 => s.push(c),
                    _ => {}
                }
            }
            s
        };
        if text.trim().is_empty() {
            return;
        }
        if let Some(last) = out.last_mut() {
            if (speaker.is_empty() || last.speaker == speaker) && last.text.len() < 900 {
                last.text.push(' ');
                last.text.push_str(text.trim());
                return;
            }
        }
        out.push(Cue { start, speaker, text: text.trim().to_string() });
    };
    match fmt {
        TranscriptFormat::Text => {
            for para in t.split("\n\n") {
                let p = para.trim();
                if p.is_empty() {
                    continue;
                }
                // "[00:12:03] Name: text" or "00:12 Name: text"
                let (start, rest) = match p.strip_prefix('[').and_then(|r| r.split_once(']')) {
                    Some((ts, rest)) if parse_duration(ts).is_some() => (parse_duration(ts), rest),
                    _ => (None, p),
                };
                let mut said = rest.replace('\n', " ");
                let mut speaker = String::new();
                if let Some((who, txt)) = said.clone().split_once(": ") {
                    if who.len() <= 40 && who.split_whitespace().count() <= 4 && !who.contains('.') {
                        speaker = who.trim().to_string();
                        said = txt.to_string();
                    }
                }
                out.push(Cue { start, speaker, text: clean_text(&said, 20_000) });
            }
        }
        _ => {
            let body = t.replace("\r\n", "\n");
            for block in body.split("\n\n") {
                let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                let Some(idx) = lines.iter().position(|l| l.contains("-->")) else { continue };
                let start = cue_time(lines[idx].split("-->").next().unwrap_or(""));
                let text = lines[idx + 1..].join(" ");
                push(&mut out, start, &text);
            }
        }
    }
    out
}

/// The transcript as plain text, for search engines and the `.txt` link.
pub fn transcript_text(t: &str) -> String {
    cues(t)
        .iter()
        .map(|c| if c.speaker.is_empty() { c.text.clone() } else { format!("{}: {}", c.speaker, c.text) })
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------------------------------------------------------------------
// inputs
// ---------------------------------------------------------------------------

fn get<'a>(form: &'a [(String, String)], k: &str) -> &'a str {
    form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or("")
}

fn all<'a>(form: &'a [(String, String)], k: &str) -> Vec<String> {
    form.iter().filter(|(n, _)| n == k).map(|(_, v)| v.clone()).collect()
}

fn flag(form: &[(String, String)], k: &str) -> bool {
    matches!(get(form, k), "1" | "on" | "true" | "yes")
}

fn num(form: &[(String, String)], k: &str, lo: i64, hi: i64) -> Result<i64, String> {
    let v = get(form, k).trim();
    if v.is_empty() {
        return Ok(0);
    }
    v.parse::<i64>().ok().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| format!("“{}” is not a number between {lo} and {hi}.", clean(v, 20)))
}

fn url_field(form: &[(String, String)], k: &str, what: &str) -> Result<String, String> {
    safe_url(get(form, k)).ok_or_else(|| format!("The {what} must start with https://"))
}

fn image_field(form: &[(String, String)], k: &str, what: &str) -> Result<String, String> {
    safe_image(get(form, k)).ok_or_else(|| format!("The {what} must be an uploaded picture or an https:// address."))
}

/// A show's links to the listening apps, keyed by [`PLATFORMS`].
pub fn links_from_form(form: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (key, label) in PLATFORMS {
        let v = get(form, &format!("link_{key}"));
        let u = safe_url(v).ok_or_else(|| format!("The {label} link must start with https://"))?;
        if !u.is_empty() {
            out.push((key.to_string(), u));
        }
    }
    Ok(out)
}

pub fn links_json(links: &[(String, String)]) -> String {
    if links.is_empty() {
        return String::new();
    }
    let map: serde_json::Map<String, serde_json::Value> = links.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect();
    serde_json::Value::Object(map).to_string()
}

/// Stored links, in [`PLATFORMS`] order, unknown keys and bad addresses dropped.
pub fn links_from_json(raw: &str) -> Vec<(String, String)> {
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(raw).unwrap_or_default();
    PLATFORMS
        .iter()
        .filter_map(|(k, _)| map.get(*k).and_then(|v| v.as_str()).and_then(|u| safe_url(u)).filter(|u| !u.is_empty()).map(|u| (k.to_string(), u)))
        .collect()
}

pub fn platform_label(key: &str) -> &'static str {
    PLATFORMS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or("Listen")
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShowInput {
    pub title: String,
    pub slug: String,
    pub subtitle: String,
    pub description: String,
    pub artwork: String,
    pub banner: String,
    pub author: String,
    pub owner_name: String,
    pub owner_email: String,
    pub category: String,
    pub category2: String,
    pub language: String,
    pub explicit: bool,
    pub show_type: String,
    pub copyright: String,
    pub links: Vec<(String, String)>,
    pub funding_url: String,
    pub funding_label: String,
    pub locked: bool,
    pub complete: bool,
    pub new_feed_url: String,
    pub tint: String,
    pub status: String,
    pub sort_order: i64,
    pub seo_title: String,
    pub seo_description: String,
    /// `(person id, role)` — the show's regular hosts.
    pub hosts: Vec<(String, String)>,
}

impl ShowInput {
    pub fn from_form(form: &[(String, String)], person_ok: impl Fn(&str) -> bool) -> Result<ShowInput, String> {
        let title = clean(get(form, "title"), 160);
        if title.is_empty() {
            return Err("Give the show a name.".into());
        }
        let owner_email = clean(get(form, "owner_email"), 191);
        if !owner_email.is_empty() && !is_email(&owner_email) {
            return Err("The owner's email does not look like an email address.".into());
        }
        let language = clean(get(form, "language"), 10).to_ascii_lowercase();
        let tint = clean(get(form, "tint"), 12);
        let mut hosts = Vec::new();
        for id in all(form, "host") {
            if person_ok(&id) && !hosts.iter().any(|(h, _): &(String, String)| h == &id) {
                let role = Role::from_key(get(form, &format!("host_role_{id}"))).key();
                hosts.push((id, if role == "guest" { "host".into() } else { role.to_string() }));
            }
        }
        Ok(ShowInput {
            slug: slugify(get(form, "slug")),
            subtitle: clean(get(form, "subtitle"), 255),
            description: clean_text(get(form, "description"), 4000),
            artwork: image_field(form, "artwork", "show artwork")?,
            banner: image_field(form, "banner", "banner")?,
            author: clean(get(form, "author"), 160),
            owner_name: clean(get(form, "owner_name"), 160),
            owner_email,
            category: clean_category(get(form, "category")),
            category2: clean_category(get(form, "category2")),
            language: if LANGUAGES.iter().any(|(k, _)| *k == language) { language } else { "en".into() },
            explicit: flag(form, "explicit"),
            show_type: ShowType::from_key(get(form, "show_type")).key().into(),
            copyright: clean(get(form, "copyright"), 200),
            links: links_from_form(form)?,
            funding_url: url_field(form, "funding_url", "support link")?,
            funding_label: clean(get(form, "funding_label"), 80),
            locked: flag(form, "locked"),
            complete: flag(form, "complete"),
            new_feed_url: url_field(form, "new_feed_url", "new feed address")?,
            tint: if TINTS.contains(&tint.as_str()) { tint } else { "purple".into() },
            status: ShowStatus::from_key(get(form, "status")).key().into(),
            sort_order: num(form, "sort_order", 0, 9999)?,
            seo_title: clean(get(form, "seo_title"), 70),
            seo_description: clean(get(form, "seo_description"), 320),
            hosts,
            title,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EpisodeInput {
    pub show_id: String,
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub notes: String,
    pub notes_lexical: String,
    pub notes_html: String,
    pub audio_url: String,
    pub audio_bytes: i64,
    pub duration: i64,
    pub video_url: String,
    pub artwork: String,
    pub season: i64,
    pub number: i64,
    pub episode_type: String,
    pub explicit: bool,
    pub access: String,
    pub status: String,
    /// UTC `YYYY-MM-DD HH:MM:SS`; empty = now, when published.
    pub published_at: String,
    pub transcript: String,
    pub chapters: Vec<Chapter>,
    pub featured: bool,
    pub seo_title: String,
    pub seo_description: String,
    /// `(person id, role)`.
    pub people: Vec<(String, String)>,
    pub sponsors: Vec<String>,
}

impl EpisodeInput {
    /// `published` arrives as the browser's `YYYY-MM-DDTHH:MM` in the site's
    /// time zone; `to_utc` turns it into the stored instant.
    pub fn from_form(
        form: &[(String, String)],
        ok: impl Fn(&str, &str) -> bool,
        to_utc: impl Fn(&str) -> Option<String>,
    ) -> Result<EpisodeInput, String> {
        let title = clean(get(form, "title"), 200);
        if title.is_empty() {
            return Err("Give the episode a title.".into());
        }
        let show_id = get(form, "show_id").trim().to_string();
        if !ok("show", &show_id) {
            return Err("Choose which show this episode belongs to.".into());
        }
        let audio_url = safe_audio(get(form, "audio_url")).ok_or("The audio must be an uploaded file or an https:// address.")?;
        let status = EpisodeStatus::from_key(get(form, "status")).key().to_string();
        if status == "published" && audio_url.is_empty() {
            return Err("Add the episode's audio before publishing it.".into());
        }
        let duration = parse_duration(get(form, "duration")).ok_or("The length should look like 42:10 or 1:02:03.")?;
        let video_url = url_field(form, "video_url", "video link")?;
        if !video_url.is_empty() && crate::lms::video_embed(&video_url).is_none() {
            return Err("The video link should be a YouTube, Vimeo, Loom or Wistia page, or an .mp4 file.".into());
        }
        let published_raw = get(form, "published_at").trim();
        let published_at = if published_raw.is_empty() { String::new() } else { to_utc(published_raw).ok_or("The publish date is not a date and time.")? };
        let chapters = {
            let rows = chapters_from_rows(&all(form, "ch_time"), &all(form, "ch_title"), &all(form, "ch_url"))?;
            if rows.is_empty() { chapters_from_text(get(form, "chapters_text")) } else { rows }
        };
        let transcript: String = get(form, "transcript").replace("\r\n", "\n").chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').take(MAX_TRANSCRIPT).collect();
        let mut people: Vec<(String, String)> = Vec::new();
        for id in all(form, "person") {
            if ok("person", &id) && !people.iter().any(|(p, _)| p == &id) {
                let role = Role::from_key(get(form, &format!("role_{id}"))).key().to_string();
                people.push((id, role));
            }
        }
        let mut sponsors: Vec<String> = all(form, "sponsor").into_iter().filter(|id| ok("sponsor", id)).collect();
        sponsors.dedup();
        let slug = slugify(get(form, "slug"));
        Ok(EpisodeInput {
            show_id,
            slug: if RESERVED_EPISODE_SLUGS.contains(&slug.as_str()) { format!("{slug}-episode") } else { slug },
            summary: clean(get(form, "summary"), 400),
            notes: clean_text(get(form, "notes"), 40_000),
            notes_lexical: String::new(),
            notes_html: String::new(),
            audio_bytes: num(form, "audio_bytes", 0, 4_000_000_000)?,
            duration,
            video_url,
            artwork: image_field(form, "artwork", "episode artwork")?,
            season: num(form, "season", 0, 999)?,
            number: num(form, "number", 0, 99_999)?,
            episode_type: EpisodeType::from_key(get(form, "episode_type")).key().into(),
            explicit: flag(form, "explicit"),
            access: Access::from_key(get(form, "access")).key().into(),
            status,
            published_at,
            transcript: transcript.trim().to_string(),
            chapters,
            featured: flag(form, "featured"),
            seo_title: clean(get(form, "seo_title"), 70),
            seo_description: clean(get(form, "seo_description"), 320),
            people,
            sponsors,
            audio_url,
            title,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PersonInput {
    pub name: String,
    pub slug: String,
    pub title: String,
    pub bio: String,
    pub photo: String,
    pub website: String,
    pub links: Vec<String>,
}

impl PersonInput {
    pub fn from_form(form: &[(String, String)]) -> Result<PersonInput, String> {
        let name = clean(get(form, "name"), 120);
        if name.is_empty() {
            return Err("Give the person a name.".into());
        }
        let mut links = Vec::new();
        for line in get(form, "links").lines().map(str::trim).filter(|l| !l.is_empty()).take(12) {
            links.push(safe_url(line).filter(|u| !u.is_empty()).ok_or_else(|| format!("“{}” is not an https:// link.", clean(line, 40)))?);
        }
        Ok(PersonInput {
            slug: slugify(get(form, "slug")),
            title: clean(get(form, "title"), 160),
            bio: clean_text(get(form, "bio"), 3000),
            photo: image_field(form, "photo", "photo")?,
            website: url_field(form, "website", "website")?,
            links,
            name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SponsorInput {
    pub name: String,
    pub tagline: String,
    pub url: String,
    pub logo: String,
    pub promo_code: String,
    pub active: bool,
}

impl SponsorInput {
    pub fn from_form(form: &[(String, String)]) -> Result<SponsorInput, String> {
        let name = clean(get(form, "name"), 120);
        if name.is_empty() {
            return Err("Give the sponsor a name.".into());
        }
        Ok(SponsorInput {
            tagline: clean(get(form, "tagline"), 240),
            url: url_field(form, "url", "sponsor's link")?,
            logo: image_field(form, "logo", "logo")?,
            promo_code: clean(get(form, "promo_code"), 40),
            // An unticked box sends nothing, so absent means inactive.
            active: flag(form, "active"),
            name,
        })
    }
}

/// A guest application or a listener's question, from the public form.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RequestInput {
    pub kind: String,
    pub name: String,
    pub email: String,
    pub topic: String,
    pub message: String,
    pub link: String,
}

impl RequestInput {
    pub fn from_form(form: &std::collections::HashMap<String, String>) -> Result<RequestInput, String> {
        let g = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let name = clean(g("name"), 120);
        let email = clean(g("email"), 191).to_ascii_lowercase();
        let message = clean_text(g("message"), 5000);
        if name.is_empty() {
            return Err("Please tell us your name.".into());
        }
        if !is_email(&email) {
            return Err("Please give an email address we can reply to.".into());
        }
        if message.chars().count() < 10 {
            return Err("Please write a little more in your message.".into());
        }
        Ok(RequestInput {
            kind: RequestKind::from_key(g("kind")).key().into(),
            topic: clean(g("topic"), 200),
            link: safe_url(g("link")).ok_or("The link must start with https://")?,
            name,
            email,
            message,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReviewInput {
    pub author: String,
    pub rating: i64,
    pub title: String,
    pub body: String,
    pub source: String,
    pub featured: bool,
}

pub const REVIEW_SOURCES: &[(&str, &str)] = &[("apple", "Apple Podcasts"), ("spotify", "Spotify"), ("email", "Email"), ("social", "Social media"), ("other", "Elsewhere")];

impl ReviewInput {
    pub fn from_form(form: &[(String, String)]) -> Result<ReviewInput, String> {
        let body = clean_text(get(form, "body"), 2000);
        if body.is_empty() {
            return Err("Paste the review.".into());
        }
        let source = clean(get(form, "source"), 20);
        Ok(ReviewInput {
            author: clean(get(form, "author"), 120),
            rating: num(form, "rating", 0, 5)?,
            title: clean(get(form, "title"), 160),
            source: if REVIEW_SOURCES.iter().any(|(k, _)| *k == source) { source } else { "other".into() },
            featured: flag(form, "featured"),
            body,
        })
    }
}

// ---------------------------------------------------------------------------
// downloads
// ---------------------------------------------------------------------------

/// The listening app, from a request's User-Agent. Order matters: several
/// apps mention another's engine in theirs.
pub fn app_from_user_agent(ua: &str) -> &'static str {
    let u = ua.to_ascii_lowercase();
    const APPS: &[(&str, &str)] = &[
        ("podcasts/", "Apple Podcasts"), ("applecoremedia", "Apple Podcasts"), ("itunes", "Apple Podcasts"), ("watchos", "Apple Podcasts"),
        ("spotify", "Spotify"), ("overcast", "Overcast"), ("pocket casts", "Pocket Casts"), ("pocketcasts", "Pocket Casts"),
        ("castro", "Castro"), ("podcast addict", "Podcast Addict"), ("podcastaddict", "Podcast Addict"), ("castbox", "Castbox"),
        ("amazon music", "Amazon Music"), ("amazonmusic", "Amazon Music"), ("alexa", "Amazon Music"), ("iheart", "iHeartRadio"),
        ("deezer", "Deezer"), ("podbean", "Podbean"), ("player fm", "Player FM"), ("playerfm", "Player FM"), ("fountain", "Fountain"),
        ("podverse", "Podverse"), ("antennapod", "AntennaPod"), ("goodpods", "Goodpods"), ("stitcher", "Stitcher"), ("tunein", "TuneIn"),
        ("youtube", "YouTube"), ("gpodder", "gPodder"), ("downcast", "Downcast"), ("podkicker", "Podkicker"), ("breaker", "Breaker"),
        ("edg/", "Web browser"), ("chrome/", "Web browser"), ("firefox/", "Web browser"), ("safari/", "Web browser"),
    ];
    APPS.iter().find(|(needle, _)| u.contains(needle)).map(|(_, app)| *app).unwrap_or("Other")
}

/// Crawlers and feed checkers, which are not listens.
pub fn is_bot(ua: &str) -> bool {
    let u = ua.to_ascii_lowercase();
    u.is_empty()
        || ["bot", "crawler", "spider", "curl/", "wget/", "python-", "go-http-client", "feedfetcher", "monitor", "headless", "preview", "facebookexternalhit", "slurp", "lighthouse"]
            .iter()
            .any(|n| u.contains(n))
}

/// A listener's daily fingerprint: the same address and app on the same day
/// is one download, however many range requests the app makes. Hashed, so
/// the table holds no addresses.
pub fn listener_key(ip: &str, ua: &str, day: &str, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b"|");
    h.update(day.as_bytes());
    h.update(b"|");
    h.update(ip.as_bytes());
    h.update(b"|");
    h.update(ua.as_bytes());
    hex::encode(&h.finalize()[..12])
}

// ---------------------------------------------------------------------------
// private feeds
// ---------------------------------------------------------------------------

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// A member's private-feed token: their uuid and a signature over it. It
/// names the member without a database lookup and cannot be forged for
/// somebody else; changing the secret revokes every private feed at once.
pub fn feed_token(secret: &str, member_uuid: &str) -> String {
    let sig = hmac_sha256(secret.as_bytes(), format!("podcast-feed:{member_uuid}").as_bytes());
    format!("{}.{}", member_uuid, hex::encode(&sig[..16]))
}

/// The member uuid a token was made for, if its signature holds.
pub fn feed_token_member(secret: &str, token: &str) -> Option<String> {
    let (uuid, sig) = token.rsplit_once('.')?;
    if uuid.is_empty() || uuid.len() > 64 || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') || secret.is_empty() {
        return None;
    }
    let want = feed_token(secret, uuid);
    let got = format!("{uuid}.{sig}");
    // constant-time compare
    (want.len() == got.len() && want.bytes().zip(got.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0).then(|| uuid.to_string())
}

// ---------------------------------------------------------------------------
// the RSS feed
// ---------------------------------------------------------------------------

/// XML text: the five entities, and control characters dropped (XML 1.0
/// cannot carry them at all, and Apple rejects a feed that does).
pub fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && !matches!(c, '\n' | '\t' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// HTML inside CDATA: the one sequence that ends a CDATA section is split.
fn cdata(s: &str) -> String {
    format!("<![CDATA[{}]]>", s.replace("]]>", "]]]]><![CDATA[>"))
}

#[derive(Debug, Clone, Default)]
pub struct FeedShow {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub link: String,
    pub feed_url: String,
    pub artwork: String,
    pub author: String,
    pub owner_name: String,
    pub owner_email: String,
    pub category: String,
    pub category2: String,
    pub language: String,
    pub explicit: bool,
    pub show_type: String,
    pub copyright: String,
    pub funding_url: String,
    pub funding_label: String,
    pub guid: String,
    pub locked: bool,
    pub complete: bool,
    pub new_feed_url: String,
    /// `(name, role, image, href)`
    pub people: Vec<(String, String, String, String)>,
    pub generator: String,
}

#[derive(Debug, Clone, Default)]
pub struct FeedEpisode {
    pub guid: String,
    pub title: String,
    pub link: String,
    pub summary: String,
    pub html: String,
    pub enclosure_url: String,
    pub enclosure_type: String,
    pub enclosure_bytes: i64,
    pub duration: i64,
    pub artwork: String,
    pub season: i64,
    pub number: i64,
    pub episode_type: String,
    pub explicit: bool,
    /// RFC 2822
    pub pub_date: String,
    pub transcript_url: String,
    pub transcript_type: String,
    pub chapters_url: String,
    pub people: Vec<(String, String, String, String)>,
}

fn category_xml(c: &str) -> String {
    if c.is_empty() {
        return String::new();
    }
    match c.split_once(" > ") {
        Some((top, sub)) => format!(r#"<itunes:category text="{}"><itunes:category text="{}"/></itunes:category>"#, xml(top), xml(sub)),
        None => format!(r#"<itunes:category text="{}"/>"#, xml(c)),
    }
}

fn person_xml(p: &(String, String, String, String)) -> String {
    let (name, role, img, href) = p;
    let mut attrs = format!(r#" role="{}""#, xml(role));
    if !img.is_empty() {
        attrs.push_str(&format!(r#" img="{}""#, xml(img)));
    }
    if !href.is_empty() {
        attrs.push_str(&format!(r#" href="{}""#, xml(href)));
    }
    format!("<podcast:person{attrs}>{}</podcast:person>", xml(name))
}

/// The show's RSS 2.0 feed with the iTunes and Podcasting 2.0 namespaces —
/// everything Apple Podcasts, Spotify, YouTube Music and the 2.0 apps read.
pub fn rss(show: &FeedShow, episodes: &[FeedEpisode]) -> String {
    let yes_no = |b: bool| if b { "true" } else { "false" };
    let mut out = String::with_capacity(4096 + episodes.len() * 2048);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str(r#"<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd" xmlns:podcast="https://podcastindex.org/namespace/1.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:content="http://purl.org/rss/1.0/modules/content/">"#);
    out.push_str("\n<channel>\n");
    out.push_str(&format!("<title>{}</title>\n", xml(&show.title)));
    out.push_str(&format!("<link>{}</link>\n", xml(&show.link)));
    out.push_str(&format!(r#"<atom:link href="{}" rel="self" type="application/rss+xml"/>"#, xml(&show.feed_url)));
    out.push('\n');
    out.push_str(&format!("<description>{}</description>\n", cdata(&show.description)));
    out.push_str(&format!("<language>{}</language>\n", xml(&show.language)));
    if !show.copyright.is_empty() {
        out.push_str(&format!("<copyright>{}</copyright>\n", xml(&show.copyright)));
    }
    if !show.generator.is_empty() {
        out.push_str(&format!("<generator>{}</generator>\n", xml(&show.generator)));
    }
    if !show.subtitle.is_empty() {
        out.push_str(&format!("<itunes:subtitle>{}</itunes:subtitle>\n", xml(&show.subtitle)));
    }
    out.push_str(&format!("<itunes:summary>{}</itunes:summary>\n", xml(&show.description)));
    out.push_str(&format!("<itunes:author>{}</itunes:author>\n", xml(if show.author.is_empty() { &show.title } else { &show.author })));
    if !show.artwork.is_empty() {
        out.push_str(&format!(r#"<itunes:image href="{}"/>"#, xml(&show.artwork)));
        out.push_str(&format!("\n<image><url>{}</url><title>{}</title><link>{}</link></image>\n", xml(&show.artwork), xml(&show.title), xml(&show.link)));
    }
    if !show.owner_email.is_empty() {
        out.push_str(&format!("<itunes:owner><itunes:name>{}</itunes:name><itunes:email>{}</itunes:email></itunes:owner>\n", xml(if show.owner_name.is_empty() { &show.author } else { &show.owner_name }), xml(&show.owner_email)));
    }
    out.push_str(&category_xml(&show.category));
    out.push_str(&category_xml(&show.category2));
    out.push('\n');
    out.push_str(&format!("<itunes:explicit>{}</itunes:explicit>\n", yes_no(show.explicit)));
    out.push_str(&format!("<itunes:type>{}</itunes:type>\n", if show.show_type == "serial" { "serial" } else { "episodic" }));
    if show.complete {
        out.push_str("<itunes:complete>Yes</itunes:complete>\n");
    }
    if !show.new_feed_url.is_empty() {
        out.push_str(&format!("<itunes:new-feed-url>{}</itunes:new-feed-url>\n", xml(&show.new_feed_url)));
    }
    if !show.guid.is_empty() {
        out.push_str(&format!("<podcast:guid>{}</podcast:guid>\n", xml(&show.guid)));
    }
    let owner = if show.owner_email.is_empty() { String::new() } else { format!(r#" owner="{}""#, xml(&show.owner_email)) };
    out.push_str(&format!("<podcast:locked{owner}>{}</podcast:locked>\n", if show.locked { "yes" } else { "no" }));
    if !show.funding_url.is_empty() {
        out.push_str(&format!(r#"<podcast:funding url="{}">{}</podcast:funding>"#, xml(&show.funding_url), xml(if show.funding_label.is_empty() { "Support the show" } else { &show.funding_label })));
        out.push('\n');
    }
    for p in &show.people {
        out.push_str(&person_xml(p));
        out.push('\n');
    }
    for e in episodes {
        out.push_str("<item>\n");
        out.push_str(&format!("<title>{}</title>\n", xml(&e.title)));
        out.push_str(&format!("<itunes:title>{}</itunes:title>\n", xml(&e.title)));
        out.push_str(&format!("<link>{}</link>\n", xml(&e.link)));
        out.push_str(&format!(r#"<guid isPermaLink="false">{}</guid>"#, xml(&e.guid)));
        out.push('\n');
        out.push_str(&format!("<pubDate>{}</pubDate>\n", xml(&e.pub_date)));
        let desc = if e.html.trim().is_empty() { format!("<p>{}</p>", xml(&e.summary)) } else { e.html.clone() };
        out.push_str(&format!("<description>{}</description>\n", cdata(&desc)));
        out.push_str(&format!("<content:encoded>{}</content:encoded>\n", cdata(&desc)));
        if !e.summary.is_empty() {
            out.push_str(&format!("<itunes:summary>{}</itunes:summary>\n", xml(&e.summary)));
        }
        out.push_str(&format!(r#"<enclosure url="{}" length="{}" type="{}"/>"#, xml(&e.enclosure_url), e.enclosure_bytes.max(0), xml(&e.enclosure_type)));
        out.push('\n');
        if e.duration > 0 {
            out.push_str(&format!("<itunes:duration>{}</itunes:duration>\n", e.duration));
        }
        if !e.artwork.is_empty() {
            out.push_str(&format!(r#"<itunes:image href="{}"/>"#, xml(&e.artwork)));
            out.push('\n');
        }
        if e.season > 0 {
            out.push_str(&format!("<itunes:season>{}</itunes:season>\n<podcast:season>{}</podcast:season>\n", e.season, e.season));
        }
        if e.number > 0 {
            out.push_str(&format!("<itunes:episode>{}</itunes:episode>\n<podcast:episode>{}</podcast:episode>\n", e.number, e.number));
        }
        out.push_str(&format!("<itunes:episodeType>{}</itunes:episodeType>\n", EpisodeType::from_key(&e.episode_type).key()));
        out.push_str(&format!("<itunes:explicit>{}</itunes:explicit>\n", yes_no(e.explicit)));
        if !e.transcript_url.is_empty() {
            out.push_str(&format!(r#"<podcast:transcript url="{}" type="{}"/>"#, xml(&e.transcript_url), xml(&e.transcript_type)));
            out.push('\n');
        }
        if !e.chapters_url.is_empty() {
            out.push_str(&format!(r#"<podcast:chapters url="{}" type="application/json+chapters"/>"#, xml(&e.chapters_url)));
            out.push('\n');
        }
        for p in &e.people {
            out.push_str(&person_xml(p));
            out.push('\n');
        }
        out.push_str("</item>\n");
    }
    out.push_str("</channel>\n</rss>\n");
    out
}

/// RFC 2822 for `pubDate`.
pub fn rfc2822(db_time: &str) -> String {
    crate::dates::from_db_string(db_time).map(|d| d.to_rfc2822()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// importing a feed from another host
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedEpisode {
    pub guid: String,
    pub title: String,
    pub summary: String,
    pub html: String,
    pub audio_url: String,
    pub audio_bytes: i64,
    pub duration: i64,
    pub artwork: String,
    pub season: i64,
    pub number: i64,
    pub episode_type: String,
    pub explicit: bool,
    pub published_at: String,
    pub link: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedShow {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub artwork: String,
    pub author: String,
    pub owner_name: String,
    pub owner_email: String,
    pub category: String,
    pub language: String,
    pub explicit: bool,
    pub show_type: String,
    pub copyright: String,
    pub guid: String,
    pub episodes: Vec<ImportedEpisode>,
}

fn unescape(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).map(str::to_string).unwrap_or_else(|| {
        t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
    });
    t.trim().to_string()
}

/// The text of the first `<tag …>…</tag>` in `block`, or `""`.
fn tag(block: &str, name: &str) -> String {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(i) = block[from..].find(&open) {
        let at = from + i;
        let after = &block[at + open.len()..];
        // `<title>` must not match `<titles>`
        if !after.starts_with(['>', ' ', '\t', '\n', '\r', '/']) {
            from = at + open.len();
            continue;
        }
        let Some(gt) = after.find('>') else { return String::new() };
        if after[..gt].ends_with('/') {
            return String::new();
        }
        let body = &after[gt + 1..];
        let close = format!("</{name}>");
        return body.find(&close).map(|j| unescape(&body[..j])).unwrap_or_default();
    }
    String::new()
}

/// An attribute of the first `<tag …>` in `block`.
fn attr(block: &str, name: &str, attribute: &str) -> String {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(i) = block[from..].find(&open) {
        let at = from + i;
        let after = &block[at + open.len()..];
        if !after.starts_with([' ', '\t', '\n', '\r', '/', '>']) {
            from = at + open.len();
            continue;
        }
        let head = &after[..after.find('>').unwrap_or(after.len())];
        for q in ['"', '\''] {
            let key = format!("{attribute}={q}");
            if let Some(k) = head.find(&key) {
                let v = &head[k + key.len()..];
                return v.find(q).map(|e| unescape(&v[..e])).unwrap_or_default();
            }
        }
        return String::new();
    }
    String::new()
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in html.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub const MAX_IMPORT_EPISODES: usize = 2000;

/// Reads a podcast RSS feed as far as it is understood. Episodes without an
/// audio enclosure are skipped. HTML in descriptions is kept only as the
/// caller sanitises it; this returns it raw.
pub fn parse_feed(xml_text: &str) -> Option<ImportedShow> {
    let channel_start = xml_text.find("<channel")?;
    let body = &xml_text[channel_start..];
    let head = &body[..body.find("<item").unwrap_or(body.len())];
    let title = clean(&tag(head, "title"), 160);
    if title.is_empty() {
        return None;
    }
    let category = {
        let top = attr(head, "itunes:category", "text");
        let all_cats: Vec<&str> = head.match_indices("<itunes:category").map(|(i, _)| &head[i..]).collect();
        let sub = all_cats.get(1).map(|b| attr(b, "itunes:category", "text")).unwrap_or_default();
        let joined = if sub.is_empty() { top.clone() } else { format!("{top} > {sub}") };
        let c = clean_category(&joined);
        if c.is_empty() { clean_category(&top) } else { c }
    };
    let explicit = |b: &str| matches!(tag(b, "itunes:explicit").to_ascii_lowercase().as_str(), "yes" | "true" | "explicit");
    let mut show = ImportedShow {
        subtitle: clean(&tag(head, "itunes:subtitle"), 255),
        description: clean_text(&strip_tags(&{ let d = tag(head, "description"); if d.is_empty() { tag(head, "itunes:summary") } else { d } }), 4000),
        artwork: safe_url(&{ let a = attr(head, "itunes:image", "href"); if a.is_empty() { tag(&tag(head, "image"), "url") } else { a } }).unwrap_or_default(),
        author: clean(&tag(head, "itunes:author"), 160),
        owner_name: clean(&tag(head, "itunes:name"), 160),
        owner_email: Some(clean(&tag(head, "itunes:email"), 191)).filter(|e| is_email(e)).unwrap_or_default(),
        category,
        language: { let l = clean(&tag(head, "language"), 10).to_ascii_lowercase(); if LANGUAGES.iter().any(|(k, _)| *k == l) { l } else if LANGUAGES.iter().any(|(k, _)| l.starts_with(k)) { l[..2].to_string() } else { "en".into() } },
        explicit: explicit(head),
        show_type: ShowType::from_key(&tag(head, "itunes:type")).key().into(),
        copyright: clean(&tag(head, "copyright"), 200),
        guid: clean(&tag(head, "podcast:guid"), 80),
        title,
        episodes: Vec::new(),
    };
    for item in body.split("<item").skip(1).take(MAX_IMPORT_EPISODES) {
        let item = &item[..item.find("</item>").unwrap_or(item.len())];
        let item = &format!("<item{item}");
        let audio = attr(item, "enclosure", "url");
        let Some(audio) = safe_audio(&audio).filter(|a| !a.is_empty()) else { continue };
        let html = { let c = tag(item, "content:encoded"); if c.is_empty() { tag(item, "description") } else { c } };
        let summary = { let s = tag(item, "itunes:summary"); clean(&strip_tags(if s.is_empty() { &html } else { &s }), 400) };
        let published_at = {
            let raw = tag(item, "pubDate");
            chrono::DateTime::parse_from_rfc2822(raw.trim()).map(|d| crate::dates::to_db_string(&d.with_timezone(&chrono::Utc))).unwrap_or_default()
        };
        let guid = { let g = clean(&tag(item, "guid"), 300); if g.is_empty() { audio.clone() } else { g } };
        show.episodes.push(ImportedEpisode {
            title: clean(&tag(item, "title"), 200),
            summary,
            html,
            audio_bytes: attr(item, "enclosure", "length").trim().parse::<i64>().unwrap_or(0).clamp(0, 4_000_000_000),
            duration: parse_duration(&tag(item, "itunes:duration")).unwrap_or(0),
            artwork: safe_url(&attr(item, "itunes:image", "href")).unwrap_or_default(),
            season: tag(item, "itunes:season").trim().parse::<i64>().unwrap_or(0).clamp(0, 999),
            number: tag(item, "itunes:episode").trim().parse::<i64>().unwrap_or(0).clamp(0, 99_999),
            episode_type: EpisodeType::from_key(&tag(item, "itunes:episodeType")).key().into(),
            explicit: explicit(item),
            published_at,
            link: safe_url(&tag(item, "link")).unwrap_or_default(),
            audio_url: audio,
            guid,
        });
    }
    Some(show)
}

// ---------------------------------------------------------------------------
// the feed check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Apple or Spotify will refuse the feed.
    Blocking,
    /// Accepted, but listeners or directories lose something.
    Warning,
    Tip,
}

#[derive(Debug, Clone, Default)]
pub struct ShowFacts {
    pub title: String,
    pub has_artwork: bool,
    pub has_description: bool,
    pub has_category: bool,
    pub has_owner_email: bool,
    pub has_author: bool,
    pub published: i64,
    pub missing_audio: i64,
    pub missing_duration: i64,
    pub has_links: bool,
    pub has_hosts: bool,
}

/// What stands between a show and the podcast directories, worst first.
pub fn check(f: &ShowFacts) -> Vec<(Level, String)> {
    let mut out = Vec::new();
    if !f.has_artwork {
        out.push((Level::Blocking, "Add show artwork — Apple Podcasts and Spotify need a square picture, 1400–3000 px.".to_string()));
    }
    if !f.has_category {
        out.push((Level::Blocking, "Choose a category; directories file the show under it.".to_string()));
    }
    if !f.has_description {
        out.push((Level::Blocking, "Write a show description.".to_string()));
    }
    if f.published == 0 {
        out.push((Level::Blocking, "Publish at least one episode — directories refuse an empty feed.".to_string()));
    }
    if !f.has_owner_email {
        out.push((Level::Warning, "Add the owner's email: Apple and Spotify send the ownership check there.".to_string()));
    }
    if f.missing_duration > 0 {
        out.push((Level::Warning, format!("{} published episode{} without a length.", f.missing_duration, if f.missing_duration == 1 { " is" } else { "s are" })));
    }
    if !f.has_author {
        out.push((Level::Tip, "Name the author or network — apps show it under the title.".to_string()));
    }
    if !f.has_links {
        out.push((Level::Tip, "Once the show is listed, add its Apple Podcasts and Spotify links so the site can show “Listen on”.".to_string()));
    }
    if !f.has_hosts {
        out.push((Level::Tip, "Add the hosts — they appear on the site and in the feed for Podcasting 2.0 apps.".to_string()));
    }
    out.sort_by_key(|(l, _)| *l);
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn directories_are_known_platforms_and_only_public_sites_can_submit() {
        for (k, submit, how) in DIRECTORIES {
            assert!(*k == "podcastindex" || PLATFORMS.iter().any(|(p, _)| p == k), "{k}");
            assert!(submit.is_empty() || submit.starts_with("https://"));
            assert!(!how.is_empty());
        }
        for u in ["http://localhost:1971", "http://127.0.0.1:2368/", "http://click.localhost", "http://my-mac.local/", "http://192.168.1.4"] {
            assert!(!publicly_reachable(u), "{u}");
        }
        assert!(publicly_reachable("https://pro.example.com/"));
        assert!(publicly_reachable("https://example.com:8443/blog/"));
    }

    use super::*;

    #[test]
    fn apple_episode_links_resolve_and_other_app_pages_are_named() {
        let u = "https://podcasts.apple.com/us/podcast/democrats-new-climate-strategy-deregulation/id1200361736?i=1000803965110";
        assert_eq!(apple_episode_ids(u), Some(("1200361736".into(), "1000803965110".into())));
        assert_eq!(apple_episode_ids("https://podcasts.apple.com/us/podcast/the-daily/id1200361736"), None);
        assert_eq!(apple_episode_ids("https://evil.example/id1?i=2"), None);
        let json = r#"{"results":[{"trackId":1200361736},{"trackId":1000803965110,"episodeUrl":"https://cdn.example/ep.mp3?x=1","trackTimeMillis":1660000,"artworkUrl600":"https://img.example/a.jpg"},{"trackId":5,"episodeUrl":"javascript:alert(1)"}]}"#;
        assert_eq!(apple_episode_audio(json, "1000803965110"), Some(("https://cdn.example/ep.mp3?x=1".into(), 1660, "https://img.example/a.jpg".into())));
        assert_eq!(apple_episode_audio(json, "5"), None);
        assert_eq!(apple_episode_audio(json, "9"), None);
        let page = r#"x{"duration":"PT27M40S"}{"streamUrl":"https://cdn.example/a.mp3?u=1\u0026v=2","duration":1660} y {"streamUrl":"https://cdn.example/a.mp3?u=1\u0026v=2"}"#;
        assert_eq!(apple_page_audio(page), Some(("https://cdn.example/a.mp3?u=1&v=2".into(), 1660)));
        assert_eq!(apple_page_audio(r#"{"streamUrl":"https://a/1.mp3"} {"streamUrl":"https://a/2.mp3"}"#), None);
        assert_eq!(apple_page_audio("no audio here"), None);
        assert_eq!(listening_page("https://open.spotify.com/episode/abc"), Some("Spotify"));
        assert_eq!(listening_page("https://www.youtube.com/watch?v=x"), Some("YouTube"));
        assert_eq!(listening_page("https://cdn.example/ep.mp3"), None);
    }

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn durations_read_every_way_feeds_write_them() {
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("62:03"), Some(3723));
        assert_eq!(parse_duration("3723"), Some(3723));
        assert_eq!(parse_duration("45m"), Some(2700));
        assert_eq!(parse_duration("123.6"), Some(124));
        assert_eq!(parse_duration(""), Some(0));
        assert_eq!(parse_duration("1:75"), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(clock(3723), "1:02:03");
        assert_eq!(clock(125), "2:05");
        assert_eq!(duration_label(3723), "1 hr 2 min");
        assert_eq!(duration_label(0), "");
    }

    #[test]
    fn episode_labels_name_season_number_and_kind() {
        assert_eq!(episode_label(2, 14, "full"), "S2 · E14");
        assert_eq!(episode_label(0, 3, "full"), "E3");
        assert_eq!(episode_label(1, 0, "trailer"), "Trailer · S1");
        assert_eq!(episode_label(0, 0, "bonus"), "Bonus");
    }

    #[test]
    fn chapters_come_from_rows_or_pasted_text_sorted() {
        let c = chapters_from_rows(&["12:30".into(), "0:00".into()], &["Interview".into(), "Welcome".into()], &["".into(), "".into()]).unwrap();
        assert_eq!(c.iter().map(|x| x.start).collect::<Vec<_>>(), vec![0, 750]);
        assert!(chapters_from_rows(&["nope".into()], &["X".into()], &[]).is_err());
        assert!(chapters_from_rows(&["1:00".into()], &["X".into()], &["javascript:alert(1)".into()]).is_err());
        let t = chapters_from_text("00:00 Welcome\n(05:10) - The news\nnot a chapter\n1:02:00 — Wrap up");
        assert_eq!(t.len(), 3);
        assert_eq!(t[1].title, "The news");
        assert_eq!(t[2].start, 3720);
        let doc = chapters_document(&t);
        assert_eq!(doc["version"], "1.2.0");
        assert_eq!(doc["chapters"][1]["startTime"], 310);
    }

    #[test]
    fn transcripts_are_recognised_and_read_as_paragraphs() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\n<v Ana>Hello and welcome.\n\n00:00:04.000 --> 00:00:06.000\n<v Ana>Today we talk rivers.\n\n00:00:06.500 --> 00:00:09.000\n<v Ben>Thanks for having me.";
        assert_eq!(transcript_format(vtt), TranscriptFormat::Vtt);
        let c = cues(vtt);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].speaker, "Ana");
        assert_eq!(c[0].text, "Hello and welcome. Today we talk rivers.");
        assert_eq!(c[1].start, Some(6));
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nAna: Hi\n\n2\n00:01:00,000 --> 00:01:02,000\nBen: Hello";
        assert_eq!(transcript_format(srt), TranscriptFormat::Srt);
        assert_eq!(cues(srt)[1].start, Some(60));
        let txt = "[00:00] Ana: Welcome in.\n\n[01:10] Ben: Glad to be here.";
        let c = cues(txt);
        assert_eq!(transcript_format(txt), TranscriptFormat::Text);
        assert_eq!((c[1].start, c[1].speaker.as_str()), (Some(70), "Ben"));
        assert!(transcript_text(vtt).starts_with("Ana: Hello"));
    }

    #[test]
    fn a_show_form_is_rebuilt_not_trusted() {
        let f = form(&[("title", " The <Long> Walk "), ("category", "Arts > Design"), ("category2", "Made Up"), ("language", "xx"), ("tint", "rainbow"),
            ("link_apple", "https://podcasts.apple.com/x"), ("link_bogus", "https://x"), ("host", "p1"), ("host", "nope"), ("host_role_p1", "cohost"), ("status", "active")]);
        let s = ShowInput::from_form(&f, |id| id == "p1").unwrap();
        assert_eq!(s.title, "The <Long> Walk");
        assert_eq!(s.category, "Arts > Design");
        assert_eq!(s.category2, "");
        assert_eq!(s.language, "en");
        assert_eq!(s.tint, "purple");
        assert_eq!(s.links, vec![("apple".to_string(), "https://podcasts.apple.com/x".to_string())]);
        assert_eq!(s.hosts, vec![("p1".to_string(), "cohost".to_string())]);
        assert!(ShowInput::from_form(&form(&[("title", "x"), ("link_spotify", "javascript:1")]), |_| true).is_err());
        assert!(ShowInput::from_form(&form(&[("title", "x"), ("owner_email", "nope")]), |_| true).is_err());
    }

    #[test]
    fn an_episode_needs_audio_to_be_published() {
        let ok = |k: &str, id: &str| (k == "show" && id == "s1") || (k == "person" && id == "p1");
        let base = [("title", "Ep"), ("show_id", "s1"), ("status", "published")];
        assert!(EpisodeInput::from_form(&form(&base), ok, |_| None).unwrap_err().contains("audio"));
        let mut f = form(&base);
        f.push(("audio_url".into(), "/content/media/2026/01/ep.mp3".into()));
        f.push(("duration".into(), "42:10".into()));
        f.push(("person".into(), "p1".into()));
        f.push(("role_p1".into(), "guest".into()));
        f.push(("person".into(), "p9".into()));
        f.push(("slug".into(), "feed".into()));
        let e = EpisodeInput::from_form(&f, ok, |_| None).unwrap();
        assert_eq!(e.duration, 2530);
        assert_eq!(e.people, vec![("p1".to_string(), "guest".to_string())]);
        assert_eq!(e.slug, "feed-episode");
        assert!(EpisodeInput::from_form(&form(&[("title", "Ep"), ("show_id", "zz")]), ok, |_| None).is_err());
        let mut bad = form(&base);
        bad.push(("audio_url".into(), "/content/media/../../etc/passwd".into()));
        assert!(EpisodeInput::from_form(&bad, ok, |_| None).is_err());
    }

    #[test]
    fn access_follows_member_status() {
        assert!(Access::Public.allows(None));
        assert!(!Access::Members.allows(None));
        assert!(Access::Members.allows(Some("free")));
        assert!(!Access::Paid.allows(Some("free")));
        assert!(Access::Paid.allows(Some("paid")) && Access::Paid.allows(Some("comped")));
    }

    #[test]
    fn private_feed_tokens_cannot_be_forged_or_moved_to_another_member() {
        let t = feed_token("secret", "abc-123");
        assert_eq!(feed_token_member("secret", &t).as_deref(), Some("abc-123"));
        assert_eq!(feed_token_member("other", &t), None);
        let moved = t.replace("abc-123", "abc-124");
        assert_eq!(feed_token_member("secret", &moved), None);
        assert_eq!(feed_token_member("secret", "abc"), None);
        assert_eq!(feed_token_member("", &t), None);
    }

    #[test]
    fn apps_and_bots_are_told_apart() {
        assert_eq!(app_from_user_agent("Podcasts/1650.20 CFNetwork/1333 Darwin/21.0.0"), "Apple Podcasts");
        assert_eq!(app_from_user_agent("Spotify/8.8 Android/31"), "Spotify");
        assert_eq!(app_from_user_agent("Overcast/3.0 (+http://overcast.fm/; iOS podcast app)"), "Overcast");
        assert_eq!(app_from_user_agent("Mozilla/5.0 ... Chrome/120 Safari/537"), "Web browser");
        assert!(is_bot("Googlebot/2.1"));
        assert!(is_bot(""));
        assert!(!is_bot("Pocket Casts"));
        assert_eq!(listener_key("1.2.3.4", "x", "2026-01-01", "s"), listener_key("1.2.3.4", "x", "2026-01-01", "s"));
        assert_ne!(listener_key("1.2.3.4", "x", "2026-01-01", "s"), listener_key("1.2.3.4", "x", "2026-01-02", "s"));
    }

    #[test]
    fn the_feed_escapes_everything_and_carries_the_podcast_tags() {
        let show = FeedShow {
            title: "Rivers & <Roads>".into(), description: "A show]]> about water".into(), link: "https://e.x/podcast/r/".into(),
            feed_url: "https://e.x/podcast/r/feed.xml".into(), artwork: "https://e.x/a.jpg".into(), owner_email: "o@e.x".into(),
            category: "Society & Culture > Documentary".into(), language: "en".into(), guid: "g-1".into(), funding_url: "https://e.x/support".into(),
            people: vec![("Ana \"A\"".into(), "host".into(), String::new(), String::new())], ..Default::default()
        };
        let ep = FeedEpisode { guid: "e1".into(), title: "One </title>".into(), enclosure_url: "https://e.x/podcast/play/1.mp3".into(), enclosure_type: "audio/mpeg".into(),
            duration: 61, season: 1, number: 2, episode_type: "full".into(), html: "<p>Hi]]><script></p>".into(), chapters_url: "https://e.x/c.json".into(), ..Default::default() };
        let x = rss(&show, &[ep]);
        assert!(x.contains("<title>Rivers &amp; &lt;Roads&gt;</title>"));
        assert!(x.contains(r#"<itunes:category text="Society &amp; Culture"><itunes:category text="Documentary"/></itunes:category>"#));
        assert!(x.contains("<title>One &lt;/title&gt;</title>"));
        assert!(!x.contains("Hi]]><script>"));
        assert!(x.contains("<itunes:season>1</itunes:season>") && x.contains("<podcast:episode>2</podcast:episode>"));
        assert!(x.contains(r#"<podcast:person role="host">Ana &quot;A&quot;</podcast:person>"#));
        assert!(x.contains(r#"<podcast:chapters url="https://e.x/c.json""#));
        assert!(x.contains("<podcast:locked owner=\"o@e.x\">no</podcast:locked>"));
    }

    #[test]
    fn a_feed_from_another_host_reads_back() {
        let feed = r#"<?xml version="1.0"?><rss xmlns:itunes="x"><channel><title>Night &amp; Day</title><language>en-us</language>
<description><![CDATA[<p>Stories after dark.</p>]]></description><itunes:author>Nora</itunes:author><itunes:image href="https://cdn.x/art.jpg"/>
<itunes:category text="Arts"><itunes:category text="Books"/></itunes:category><itunes:explicit>no</itunes:explicit><itunes:owner><itunes:name>Nora</itunes:name><itunes:email>nora@x.com</itunes:email></itunes:owner>
<item><title>Episode one</title><guid isPermaLink="false">abc-1</guid><pubDate>Tue, 03 Jun 2025 08:00:00 +0000</pubDate>
<enclosure url="https://cdn.x/1.mp3" length="12345" type="audio/mpeg"/><itunes:duration>00:31:10</itunes:duration><itunes:episode>1</itunes:episode><itunes:season>2</itunes:season>
<description><![CDATA[<p>The first one.</p>]]></description></item>
<item><title>No audio</title></item>
<item><title>Bad audio</title><enclosure url="javascript:alert(1)"/></item>
</channel></rss>"#;
        let s = parse_feed(feed).unwrap();
        assert_eq!(s.title, "Night & Day");
        assert_eq!(s.category, "Arts > Books");
        assert_eq!(s.language, "en-us");
        assert_eq!(s.description, "Stories after dark.");
        assert_eq!(s.owner_email, "nora@x.com");
        assert_eq!(s.episodes.len(), 1);
        let e = &s.episodes[0];
        assert_eq!((e.duration, e.season, e.number, e.audio_bytes), (1870, 2, 1, 12345));
        assert_eq!(e.guid, "abc-1");
        assert_eq!(e.published_at, "2025-06-03 08:00:00");
        assert!(parse_feed("<html>not a feed</html>").is_none());
    }

    #[test]
    fn the_feed_check_puts_what_blocks_listing_first() {
        let c = check(&ShowFacts { has_description: true, ..Default::default() });
        assert_eq!(c[0].0, Level::Blocking);
        assert!(c.iter().any(|(_, m)| m.contains("artwork")));
        let fine = check(&ShowFacts { has_artwork: true, has_description: true, has_category: true, has_owner_email: true, has_author: true, published: 3, has_links: true, has_hosts: true, ..Default::default() });
        assert!(fine.is_empty());
    }

    #[test]
    fn the_address_can_move_but_not_onto_another_tool() {
        assert_eq!(Paths::new("shows").unwrap().public_path("/podcast/x/"), "/shows/x/");
        assert_eq!(Paths::new("shows").unwrap().internal_path("/shows/x/").as_deref(), Some("/podcast/x/"));
        assert!(Paths::new("shop").is_none());
        assert_eq!(Paths::default().internal_path("/podcast/"), None);
    }
}
