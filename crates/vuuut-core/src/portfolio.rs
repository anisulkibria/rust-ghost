//! Portfolio & case studies: the rules.
//!
//! One tool for both kinds of portfolio site. A **personal** portfolio — a
//! designer, developer, photographer, writer, architect — is projects plus
//! the person: a profile, availability for work, a résumé of experience,
//! education, awards and talks, skills, and a "hire me" form. A **studio**
//! portfolio — an agency, a practice, a production company — is projects
//! plus the business: services with starting prices, the team, clients,
//! awards and press, and a project-brief form that asks for budget and
//! timeline. Both share case studies with a written body, galleries, video,
//! results, credits, disciplines and testimonials, and projects that stay
//! behind a password when the work is under NDA.
//!
//! Pure. This module is the trust boundary: every form is rebuilt by one of
//! the `*Input` types and nothing from a request is stored as sent.

use crate::directory::{clean, clean_text, is_email, safe_image, safe_url, slugify};
use sha2::{Digest, Sha256};

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

keyed!(Mode {
    Personal => "personal", "Personal — one person's work";
    Studio => "studio", "Studio — an agency, practice or company";
} default Personal);

keyed!(ProjectStatus {
    Published => "published", "Published";
    Protected => "protected", "Password protected";
    Unlisted => "unlisted", "Unlisted — by link only";
    Draft => "draft", "Draft";
} default Draft);

keyed!(Layout {
    CaseStudy => "case_study", "Case study — story, then pictures";
    Gallery => "gallery", "Gallery — pictures first";
    Minimal => "minimal", "Minimal — one image, a few lines";
} default CaseStudy);

keyed!(Availability {
    Available => "available", "Available for new work";
    Limited => "limited", "Limited availability";
    Booked => "booked", "Booked up";
    Hidden => "hidden", "Don't say";
} default Hidden);

keyed!(EntryKind {
    Work => "work", "Experience";
    Education => "education", "Education";
    Award => "award", "Award";
    Press => "press", "Press";
    Talk => "talk", "Talk";
    Publication => "publication", "Publication";
    Certification => "certification", "Certification";
    Volunteer => "volunteer", "Volunteering";
} default Work);

keyed!(EnquiryStatus {
    New => "new", "New";
    Replied => "replied", "Replied";
    Proposal => "proposal", "Proposal sent";
    Won => "won", "Won";
    Lost => "lost", "Lost";
    Archived => "archived", "Archived";
    Spam => "spam", "Spam";
} default New);

keyed!(ServiceStatus {
    Active => "active", "Shown";
    Hidden => "hidden", "Hidden";
} default Active);

impl ProjectStatus {
    /// In the public grid (protected projects show with a lock).
    pub fn listed(self) -> bool {
        matches!(self, ProjectStatus::Published | ProjectStatus::Protected)
    }
    /// Reachable at its address at all.
    pub fn reachable(self) -> bool {
        !matches!(self, ProjectStatus::Draft)
    }
}

pub const TINTS: [&str; 8] = ["slate", "blue", "teal", "green", "amber", "purple", "pink", "red"];

/// Profiles a portfolio links to: `(key, label)`.
pub const SOCIALS: &[(&str, &str)] = &[
    ("linkedin", "LinkedIn"), ("github", "GitHub"), ("dribbble", "Dribbble"), ("behance", "Behance"), ("instagram", "Instagram"),
    ("x", "X"), ("bluesky", "Bluesky"), ("mastodon", "Mastodon"), ("youtube", "YouTube"), ("vimeo", "Vimeo"), ("medium", "Medium"),
    ("codepen", "CodePen"), ("artstation", "ArtStation"), ("figma", "Figma"), ("unsplash", "Unsplash"), ("website", "Website"),
];

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "portfolio_enabled";
pub const SETTING_BASE_PATH: &str = "portfolio_base_path";
pub const SETTING_MODE: &str = "portfolio_mode";
pub const SETTING_TITLE: &str = "portfolio_title";
pub const SETTING_INTRO: &str = "portfolio_intro";
pub const SETTING_NAME: &str = "portfolio_name";
pub const SETTING_HEADLINE: &str = "portfolio_headline";
pub const SETTING_ABOUT: &str = "portfolio_about";
pub const SETTING_PORTRAIT: &str = "portfolio_portrait";
pub const SETTING_LOCATION: &str = "portfolio_location";
pub const SETTING_AVAILABILITY: &str = "portfolio_availability";
pub const SETTING_AVAILABILITY_NOTE: &str = "portfolio_availability_note";
pub const SETTING_EMAIL: &str = "portfolio_email";
pub const SETTING_PHONE: &str = "portfolio_phone";
pub const SETTING_RESUME_URL: &str = "portfolio_resume_url";
pub const SETTING_SOCIALS: &str = "portfolio_socials";
pub const SETTING_FOUNDED: &str = "portfolio_founded";
pub const SETTING_ENQUIRIES: &str = "portfolio_enquiries";
pub const SETTING_BUDGETS: &str = "portfolio_budgets";
pub const SETTING_NOTIFY_EMAIL: &str = "portfolio_notify_email";
pub const SETTING_CURRENCY: &str = "portfolio_currency";
pub const SETTING_PER_PAGE: &str = "portfolio_per_page";

pub const DEFAULT_BASE: &str = "portfolio";
pub const DEFAULT_BUDGETS: &str = "Under $5k\n$5k – $15k\n$15k – $50k\n$50k+\nNot sure yet";
/// The portfolio's own pages; never a project's slug.
pub const RESERVED_SLUGS: &[&str] = &["assets", "category", "services", "about", "resume", "contact", "team", "clients", "unlock"];
pub const MIN_FILL_MS: i64 = 2500;
/// The cookie a correct project password sets, and how long it lasts.
pub const UNLOCK_COOKIE: &str = "vuuut-portfolio-unlock";
pub const UNLOCK_DAYS: i64 = 30;

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
        if matches!(clean.split('/').next().unwrap_or_default(), "shop" | "directory" | "courses" | "book" | "podcast" | "properties") {
            return None;
        }
        Some(Paths { base: clean })
    }
    pub fn public_path(&self, internal: &str) -> String {
        if self.base == DEFAULT_BASE {
            return internal.into();
        }
        match internal.strip_prefix("/portfolio/") {
            Some(tail) => format!("/{}/{tail}", self.base),
            None if internal == "/portfolio" => format!("/{}", self.base),
            None => internal.into(),
        }
    }
    pub fn internal_path(&self, public: &str) -> Option<String> {
        if self.base == DEFAULT_BASE {
            return None;
        }
        let rest = public.strip_prefix('/')?.strip_prefix(self.base.as_str())?;
        if rest.is_empty() {
            return Some("/portfolio".into());
        }
        rest.strip_prefix('/').map(|t| format!("/portfolio/{t}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub mode: Mode,
    pub title: String,
    pub intro: String,
    /// The person's or the studio's name; empty = the site's title.
    pub name: String,
    /// "Product designer in Lisbon", "An independent brand studio".
    pub headline: String,
    pub about: String,
    pub portrait: String,
    pub location: String,
    pub availability: Availability,
    pub availability_note: String,
    pub email: String,
    pub phone: String,
    pub resume_url: String,
    pub socials: Vec<(String, String)>,
    pub founded: i64,
    pub enquiries: bool,
    pub budgets: Vec<String>,
    pub notify_email: String,
    pub currency: String,
    pub per_page: i64,
    pub paths: Paths,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let raw = |k: &str| get(k).map(|v| v.trim().trim_matches('"').to_string()).unwrap_or_default();
        let flag = |k: &str, d: bool| match raw(k).as_str() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => d,
        };
        let mode = Mode::from_key(&raw(SETTING_MODE));
        let title = clean(&raw(SETTING_TITLE), 80);
        let email = raw(SETTING_EMAIL);
        let notify = raw(SETTING_NOTIFY_EMAIL);
        // Stored JSON strings arrive with their quotes escaped once more.
        let unjson = |k: &str| get(k).map(|v| serde_json::from_str::<String>(&v).unwrap_or(v)).unwrap_or_default();
        let budgets_raw = unjson(SETTING_BUDGETS);
        let budgets: Vec<String> = (if budgets_raw.trim().is_empty() { DEFAULT_BUDGETS.to_string() } else { budgets_raw })
            .lines().map(|l| clean(l, 60)).filter(|l| !l.is_empty()).take(12).collect();
        let currency = raw(SETTING_CURRENCY).to_uppercase();
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            title: if title.is_empty() { match mode { Mode::Personal => "Work".into(), Mode::Studio => "Our work".into() } } else { title },
            intro: clean_text(&unjson(SETTING_INTRO), 600),
            name: clean(&raw(SETTING_NAME), 120),
            headline: clean(&raw(SETTING_HEADLINE), 200),
            about: clean_text(&unjson(SETTING_ABOUT), 6000),
            portrait: safe_image(&raw(SETTING_PORTRAIT)).unwrap_or_default(),
            location: clean(&raw(SETTING_LOCATION), 120),
            availability: Availability::from_key(&raw(SETTING_AVAILABILITY)),
            availability_note: clean(&raw(SETTING_AVAILABILITY_NOTE), 160),
            email: if is_email(&email) { email } else { String::new() },
            phone: clean(&raw(SETTING_PHONE), 40),
            resume_url: safe_url(&raw(SETTING_RESUME_URL)).or_else(|| safe_image(&raw(SETTING_RESUME_URL))).unwrap_or_default(),
            socials: socials_from_json(&unjson(SETTING_SOCIALS)),
            founded: raw(SETTING_FOUNDED).parse::<i64>().ok().filter(|y| (1800..=2200).contains(y)).unwrap_or(0),
            enquiries: flag(SETTING_ENQUIRIES, true),
            budgets,
            notify_email: if is_email(&notify) { notify } else { String::new() },
            currency: if crate::directory::CURRENCIES.contains(&currency.as_str()) { currency } else { "USD".into() },
            per_page: raw(SETTING_PER_PAGE).parse::<i64>().ok().map(|n| n.clamp(4, 100)).unwrap_or(24),
            paths: Paths::read(&get),
            mode,
        }
    }
    pub fn studio(&self) -> bool {
        self.mode == Mode::Studio
    }
}

pub fn socials_from_form(form: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (key, label) in SOCIALS {
        let v = get(form, &format!("social_{key}"));
        let u = safe_url(v).ok_or_else(|| format!("The {label} link must start with https://"))?;
        if !u.is_empty() {
            out.push((key.to_string(), u));
        }
    }
    Ok(out)
}

pub fn socials_json(v: &[(String, String)]) -> String {
    let map: serde_json::Map<String, serde_json::Value> = v.iter().map(|(k, u)| (k.clone(), serde_json::json!(u))).collect();
    serde_json::Value::Object(map).to_string()
}

pub fn socials_from_json(raw: &str) -> Vec<(String, String)> {
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(raw).unwrap_or_default();
    SOCIALS
        .iter()
        .filter_map(|(k, _)| map.get(*k).and_then(|v| v.as_str()).and_then(|u| safe_url(u)).filter(|u| !u.is_empty()).map(|u| (k.to_string(), u)))
        .collect()
}

pub fn social_label(key: &str) -> &'static str {
    SOCIALS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or("Link")
}

// ---------------------------------------------------------------------------
// form helpers
// ---------------------------------------------------------------------------

fn get<'a>(form: &'a [(String, String)], k: &str) -> &'a str {
    form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or("")
}

fn all(form: &[(String, String)], k: &str) -> Vec<String> {
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

/// `#1a2b3c` or `#abc`, lower-cased; anything else is empty.
pub fn clean_colour(raw: &str) -> String {
    let v = raw.trim().to_ascii_lowercase();
    let ok = v.starts_with('#') && matches!(v.len(), 4 | 7) && v[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok { v } else { String::new() }
}

/// A comma- or line-separated list, each item cleaned, empties and repeats
/// dropped.
pub fn list(raw: &str, max_items: usize, max_len: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split([',', '\n']) {
        let v = clean(part, max_len);
        if !v.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
            out.push(v);
        }
        if out.len() >= max_items {
            break;
        }
    }
    out
}

/// `2024`, `2024-03` or empty — how dates on a résumé are written.
pub fn clean_month(raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() {
        return Some(String::new());
    }
    let (y, m) = v.split_once('-').map(|(y, m)| (y, Some(m))).unwrap_or((v, None));
    let year: i64 = y.parse().ok().filter(|y| (1900..=2200).contains(y))?;
    match m {
        None => Some(format!("{year}")),
        Some(m) => {
            let month: i64 = m.parse().ok().filter(|m| (1..=12).contains(m))?;
            Some(format!("{year}-{month:02}"))
        }
    }
}

/// `"2021-03"` → `"Mar 2021"`, `"2021"` → `"2021"`.
pub fn month_label(v: &str) -> String {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    match v.split_once('-') {
        Some((y, m)) => m.parse::<usize>().ok().filter(|m| (1..=12).contains(m)).map(|m| format!("{} {y}", M[m - 1])).unwrap_or_else(|| y.to_string()),
        None => v.to_string(),
    }
}

/// `"Mar 2021 – Present"`, `"2016 – 2019"`, `"2020"`.
pub fn span_label(start: &str, end: &str, current: bool) -> String {
    let s = month_label(start);
    let e = if current { "Present".to_string() } else { month_label(end) };
    match (s.is_empty(), e.is_empty()) {
        (true, true) => String::new(),
        (false, true) => s,
        (true, false) => e,
        (false, false) if s == e => s,
        _ => format!("{s} – {e}"),
    }
}

// ---------------------------------------------------------------------------
// projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct Metric {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct Credit {
    pub name: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

pub fn metrics_from_json(raw: &str) -> Vec<Metric> {
    serde_json::from_str(raw).unwrap_or_default()
}

pub fn credits_from_json(raw: &str) -> Vec<Credit> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Password hashes: salted SHA-256, `salt$hex`. A case study's password is a
/// share-with-a-client convenience, not an account; this keeps it out of
/// the database in the clear without pretending to be more.
pub fn hash_password(password: &str, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b"|");
    h.update(password.as_bytes());
    format!("{salt}${}", hex::encode(h.finalize()))
}

pub fn password_matches(stored: &str, given: &str) -> bool {
    let Some((salt, _)) = stored.split_once('$') else { return false };
    let want = hash_password(given, salt);
    want.len() == stored.len() && want.bytes().zip(stored.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

/// The unlock cookie's value for one project: a signature over the project
/// id and the stored hash, so changing the password locks everybody out
/// again and one project's cookie never opens another.
pub fn unlock_value(secret: &str, project_id: &str, stored_hash: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(b"|unlock|");
    h.update(project_id.as_bytes());
    h.update(b"|");
    h.update(stored_hash.as_bytes());
    format!("{project_id}.{}", hex::encode(&h.finalize()[..16]))
}

/// The unlocked project ids in a cookie value (`id.sig,id.sig`), checked.
pub fn unlocked(secret: &str, cookie: &str, hash_of: impl Fn(&str) -> Option<String>) -> Vec<String> {
    cookie
        .split(',')
        .filter_map(|part| {
            let (id, _) = part.split_once('.')?;
            let hash = hash_of(id)?;
            (unlock_value(secret, id, &hash) == part).then(|| id.to_string())
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectInput {
    pub title: String,
    pub slug: String,
    pub subtitle: String,
    pub summary: String,
    pub body: String,
    pub body_lexical: String,
    pub body_html: String,
    pub cover: String,
    pub gallery: Vec<String>,
    pub video_url: String,
    pub client_id: Option<String>,
    pub client_name: String,
    pub year: i64,
    pub duration: String,
    pub role: String,
    pub industry: String,
    pub location: String,
    pub tools: Vec<String>,
    pub live_url: String,
    pub repo_url: String,
    pub metrics: Vec<Metric>,
    pub credits: Vec<Credit>,
    pub testimonial_id: Option<String>,
    pub category_ids: Vec<String>,
    pub service_ids: Vec<String>,
    pub accent: String,
    pub layout: String,
    pub featured: bool,
    pub status: String,
    /// A new password to set; empty = keep the current one.
    pub password: String,
    pub sort_order: i64,
    pub seo_title: String,
    pub seo_description: String,
}

impl ProjectInput {
    /// `ok(kind, id)` says whether a client, category, service or
    /// testimonial id exists.
    pub fn from_form(form: &[(String, String)], ok: impl Fn(&str, &str) -> bool) -> Result<ProjectInput, String> {
        let title = clean(get(form, "title"), 200);
        if title.is_empty() {
            return Err("Give the project a title.".into());
        }
        // Pictures: the first `image` is the cover, the rest the gallery.
        let mut images: Vec<String> = Vec::new();
        for raw in all(form, "image") {
            let v = safe_image(&raw).filter(|v| !v.is_empty()).ok_or("A picture must be an upload or an https:// address.")?;
            if !images.contains(&v) {
                images.push(v);
            }
        }
        images.truncate(60);
        let cover = images.first().cloned().unwrap_or_default();
        let gallery = images.into_iter().skip(1).collect();
        let video_url = url_field(form, "video_url", "video link")?;
        if !video_url.is_empty() && crate::lms::video_embed(&video_url).is_none() {
            return Err("The video link should be a YouTube, Vimeo, Loom or Wistia page, or an .mp4 file.".into());
        }
        let mut metrics = Vec::new();
        let (mv, ml) = (all(form, "m_value"), all(form, "m_label"));
        for (i, v) in mv.iter().enumerate() {
            let value = clean(v, 30);
            let label = clean(ml.get(i).map(String::as_str).unwrap_or(""), 120);
            if !value.is_empty() || !label.is_empty() {
                if value.is_empty() || label.is_empty() {
                    return Err("Each result needs a figure and what it measures.".into());
                }
                metrics.push(Metric { value, label });
            }
        }
        metrics.truncate(8);
        let mut credits = Vec::new();
        let (cn, cr, cu) = (all(form, "c_name"), all(form, "c_role"), all(form, "c_url"));
        for (i, n) in cn.iter().enumerate() {
            let name = clean(n, 120);
            if name.is_empty() {
                continue;
            }
            let url = safe_url(cu.get(i).map(String::as_str).unwrap_or("")).ok_or("A credit's link must start with https://")?;
            credits.push(Credit { name, role: clean(cr.get(i).map(String::as_str).unwrap_or(""), 120), url });
        }
        credits.truncate(40);
        let client_id = Some(get(form, "client_id").trim().to_string()).filter(|c| !c.is_empty() && ok("client", c));
        let testimonial_id = Some(get(form, "testimonial_id").trim().to_string()).filter(|c| !c.is_empty() && ok("testimonial", c));
        let mut category_ids: Vec<String> = all(form, "category").into_iter().filter(|c| ok("category", c)).collect();
        category_ids.dedup();
        let mut service_ids: Vec<String> = all(form, "service").into_iter().filter(|c| ok("service", c)).collect();
        service_ids.dedup();
        let status = ProjectStatus::from_key(get(form, "status")).key().to_string();
        let password = get(form, "password").trim().to_string();
        if !password.is_empty() && password.chars().count() < 4 {
            return Err("Use a password of at least four characters.".into());
        }
        let slug = slugify(get(form, "slug"));
        Ok(ProjectInput {
            slug: if RESERVED_SLUGS.contains(&slug.as_str()) { format!("{slug}-project") } else { slug },
            subtitle: clean(get(form, "subtitle"), 240),
            summary: clean(get(form, "summary"), 400),
            body: clean_text(get(form, "body"), 60_000),
            body_lexical: String::new(),
            body_html: String::new(),
            cover,
            gallery,
            video_url,
            client_name: clean(get(form, "client_name"), 120),
            client_id,
            year: num(form, "year", 1900, 2200)?,
            duration: clean(get(form, "duration"), 60),
            role: clean(get(form, "role"), 160),
            industry: clean(get(form, "industry"), 80),
            location: clean(get(form, "location"), 120),
            tools: list(get(form, "tools"), 30, 40),
            live_url: url_field(form, "live_url", "live link")?,
            repo_url: url_field(form, "repo_url", "code link")?,
            metrics,
            credits,
            testimonial_id,
            category_ids,
            service_ids,
            accent: clean_colour(get(form, "accent")),
            layout: Layout::from_key(get(form, "layout")).key().into(),
            featured: flag(form, "featured"),
            status,
            password: password.chars().take(200).collect(),
            sort_order: num(form, "sort_order", 0, 9999)?,
            seo_title: clean(get(form, "seo_title"), 70),
            seo_description: clean(get(form, "seo_description"), 320),
            title,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServiceInput {
    pub name: String,
    pub slug: String,
    pub summary: String,
    pub description: String,
    pub image: String,
    pub icon: String,
    pub price_from: i64,
    pub price_note: String,
    pub deliverables: Vec<String>,
    pub status: String,
    pub featured: bool,
    pub sort_order: i64,
}

/// Glyphs a service can carry, named for what they show.
pub const SERVICE_ICONS: &[(&str, &str)] = &[
    ("brush", "Design"), ("code", "Development"), ("camera", "Photography"), ("video", "Film & motion"), ("pen", "Writing"),
    ("chart", "Strategy"), ("layers", "Branding"), ("phone", "Mobile"), ("megaphone", "Marketing"), ("cube", "3D & product"),
    ("home", "Architecture & interiors"), ("spark", "Consulting"),
];

impl ServiceInput {
    pub fn from_form(form: &[(String, String)]) -> Result<ServiceInput, String> {
        let name = clean(get(form, "name"), 120);
        if name.is_empty() {
            return Err("Give the service a name.".into());
        }
        let price_from = crate::directory::parse_price(get(form, "price_from")).ok_or("The starting price should be a number like 2500.")?;
        let icon = clean(get(form, "icon"), 20);
        let slug = slugify(get(form, "slug"));
        Ok(ServiceInput {
            slug,
            summary: clean(get(form, "summary"), 300),
            description: clean_text(get(form, "description"), 8000),
            image: image_field(form, "image", "picture")?,
            icon: if SERVICE_ICONS.iter().any(|(k, _)| *k == icon) { icon } else { "spark".into() },
            price_from,
            price_note: clean(get(form, "price_note"), 60),
            deliverables: list(get(form, "deliverables").replace(',', "\n").as_str(), 20, 160),
            status: ServiceStatus::from_key(get(form, "status")).key().into(),
            featured: flag(form, "featured"),
            sort_order: num(form, "sort_order", 0, 9999)?,
            name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientInput {
    pub name: String,
    pub logo: String,
    pub url: String,
    pub industry: String,
    pub featured: bool,
    pub sort_order: i64,
}

impl ClientInput {
    pub fn from_form(form: &[(String, String)]) -> Result<ClientInput, String> {
        let name = clean(get(form, "name"), 120);
        if name.is_empty() {
            return Err("Give the client a name.".into());
        }
        Ok(ClientInput {
            logo: image_field(form, "logo", "logo")?,
            url: url_field(form, "url", "client's website")?,
            industry: clean(get(form, "industry"), 80),
            featured: flag(form, "featured"),
            sort_order: num(form, "sort_order", 0, 9999)?,
            name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TestimonialInput {
    pub quote: String,
    pub author: String,
    pub role: String,
    pub company: String,
    pub photo: String,
    pub rating: i64,
    pub project_id: Option<String>,
    pub featured: bool,
    pub sort_order: i64,
}

impl TestimonialInput {
    pub fn from_form(form: &[(String, String)], project_ok: impl Fn(&str) -> bool) -> Result<TestimonialInput, String> {
        let quote = clean_text(get(form, "quote"), 2000);
        let author = clean(get(form, "author"), 120);
        if quote.is_empty() || author.is_empty() {
            return Err("A testimonial needs the words and who said them.".into());
        }
        Ok(TestimonialInput {
            role: clean(get(form, "role"), 120),
            company: clean(get(form, "company"), 120),
            photo: image_field(form, "photo", "photo")?,
            rating: num(form, "rating", 0, 5)?,
            project_id: Some(get(form, "project_id").trim().to_string()).filter(|p| !p.is_empty() && project_ok(p)),
            featured: flag(form, "featured"),
            sort_order: num(form, "sort_order", 0, 9999)?,
            quote,
            author,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MemberInput {
    pub name: String,
    pub slug: String,
    pub role: String,
    pub bio: String,
    pub photo: String,
    pub email: String,
    pub links: Vec<String>,
    pub status: String,
    pub sort_order: i64,
}

impl MemberInput {
    pub fn from_form(form: &[(String, String)]) -> Result<MemberInput, String> {
        let name = clean(get(form, "name"), 120);
        if name.is_empty() {
            return Err("Give the person a name.".into());
        }
        let email = clean(get(form, "email"), 191);
        if !email.is_empty() && !is_email(&email) {
            return Err("That email does not look right.".into());
        }
        let mut links = Vec::new();
        for line in get(form, "links").lines().map(str::trim).filter(|l| !l.is_empty()).take(10) {
            links.push(safe_url(line).filter(|u| !u.is_empty()).ok_or_else(|| format!("“{}” is not an https:// link.", clean(line, 40)))?);
        }
        Ok(MemberInput {
            slug: slugify(get(form, "slug")),
            role: clean(get(form, "role"), 120),
            bio: clean_text(get(form, "bio"), 3000),
            photo: image_field(form, "photo", "photo")?,
            status: if get(form, "status") == "hidden" { "hidden".into() } else { "active".into() },
            sort_order: num(form, "sort_order", 0, 9999)?,
            email,
            links,
            name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntryInput {
    pub kind: String,
    pub title: String,
    pub organisation: String,
    pub location: String,
    pub url: String,
    pub start: String,
    pub end: String,
    pub current: bool,
    pub description: String,
    pub sort_order: i64,
}

impl EntryInput {
    pub fn from_form(form: &[(String, String)]) -> Result<EntryInput, String> {
        let title = clean(get(form, "title"), 160);
        if title.is_empty() {
            return Err("Give the entry a title — a role, a degree, an award.".into());
        }
        let start = clean_month(get(form, "start")).ok_or("Write the start as a year or year and month, like 2021 or 2021-03.")?;
        let current = flag(form, "current");
        let end = if current { String::new() } else { clean_month(get(form, "end")).ok_or("Write the end as a year or year and month, like 2023 or 2023-09.")? };
        if !start.is_empty() && !end.is_empty() && end < start {
            return Err("The end comes before the start.".into());
        }
        Ok(EntryInput {
            kind: EntryKind::from_key(get(form, "kind")).key().into(),
            organisation: clean(get(form, "organisation"), 160),
            location: clean(get(form, "location"), 120),
            url: url_field(form, "url", "link")?,
            description: clean_text(get(form, "description"), 3000),
            sort_order: num(form, "sort_order", 0, 9999)?,
            start,
            end,
            current,
            title,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkillInput {
    pub name: String,
    pub group: String,
    pub level: i64,
    pub sort_order: i64,
}

impl SkillInput {
    pub fn from_form(form: &[(String, String)]) -> Result<SkillInput, String> {
        let name = clean(get(form, "name"), 80);
        if name.is_empty() {
            return Err("Name the skill.".into());
        }
        Ok(SkillInput { group: clean(get(form, "group"), 60), level: num(form, "level", 0, 100)?, sort_order: num(form, "sort_order", 0, 9999)?, name })
    }
}

/// "Hire me" / a project brief, from the public form.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnquiryInput {
    pub name: String,
    pub email: String,
    pub company: String,
    pub phone: String,
    pub budget: String,
    pub timeline: String,
    pub services: Vec<String>,
    pub message: String,
    pub project: String,
}

impl EnquiryInput {
    pub fn from_form(form: &std::collections::HashMap<String, String>, budgets: &[String], services: &[String]) -> Result<EnquiryInput, String> {
        let g = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let name = clean(g("name"), 120);
        let email = clean(g("email"), 191).to_ascii_lowercase();
        let message = clean_text(g("message"), 6000);
        if name.is_empty() {
            return Err("Please tell me your name.".into());
        }
        if !is_email(&email) {
            return Err("Please give an email address to reply to.".into());
        }
        if message.chars().count() < 10 {
            return Err("Please say a little about the project.".into());
        }
        let budget = clean(g("budget"), 60);
        let wanted: Vec<String> = g("services").split(',').map(|s| clean(s, 120)).filter(|s| services.contains(s)).collect();
        Ok(EnquiryInput {
            company: clean(g("company"), 120),
            phone: crate::directory::clean_phone(g("phone")).ok_or("That phone number does not look right.")?,
            budget: if budgets.contains(&budget) { budget } else { String::new() },
            timeline: clean(g("timeline"), 80),
            services: wanted,
            project: clean(g("project"), 200),
            name,
            email,
            message,
        })
    }
}

// ---------------------------------------------------------------------------
// the setup check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub studio: bool,
    pub enabled: bool,
    pub published: i64,
    pub without_cover: i64,
    pub has_name: bool,
    pub has_headline: bool,
    pub has_about: bool,
    pub has_contact: bool,
    pub services: i64,
    pub testimonials: i64,
    pub clients: i64,
    pub team: i64,
    pub entries: i64,
}

/// What a visitor would find missing, most important first. Each item is
/// `(done, label, where to fix it)`.
pub fn checklist(f: &Facts) -> Vec<(bool, &'static str, &'static str)> {
    let mut v = vec![
        (f.has_name && f.has_headline, "Say who you are: a name and a one-line headline", "settings"),
        (f.published >= 3, "Publish at least three projects", "projects"),
        (f.published > 0 && f.without_cover == 0, "Give every published project a cover picture", "projects"),
        (f.has_about, "Write the About text", "settings"),
        (f.has_contact, "Turn on enquiries or add a contact email", "settings"),
        (f.testimonials > 0, "Add a testimonial — the strongest proof there is", "testimonials"),
    ];
    if f.studio {
        v.push((f.services > 0, "List your services", "services"));
        v.push((f.clients >= 4, "Add the clients you've worked with", "clients"));
        v.push((f.team > 0, "Introduce the team", "team"));
    } else {
        v.push((f.entries > 0, "Fill in your résumé: experience and education", "resume"));
        v.push((f.services > 0, "Say what people can hire you for", "services"));
    }
    v.push((f.enabled, "Switch the portfolio on", "settings"));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_project_form_is_rebuilt_not_trusted() {
        let ok = |kind: &str, id: &str| matches!((kind, id), ("category", "c1") | ("service", "s1") | ("client", "k1"));
        let f = form(&[
            ("title", " Rebrand "), ("slug", "about"), ("image", "/content/images/a.jpg"), ("image", "https://x.com/b.jpg"), ("image", "/content/images/a.jpg"),
            ("category", "c1"), ("category", "evil"), ("service", "s1"), ("client_id", "k9"), ("accent", "#FFAA00"), ("layout", "weird"),
            ("m_value", "+42%"), ("m_label", "conversion"), ("m_value", ""), ("m_label", ""), ("c_name", "Ana"), ("c_role", "Lead"), ("c_url", ""),
            ("tools", "Figma, figma, React"), ("status", "protected"), ("password", "open sesame"),
        ]);
        let p = ProjectInput::from_form(&f, ok).unwrap();
        assert_eq!(p.title, "Rebrand");
        assert_eq!(p.slug, "about-project");
        assert_eq!(p.cover, "/content/images/a.jpg");
        assert_eq!(p.gallery, vec!["https://x.com/b.jpg".to_string()]);
        assert_eq!(p.category_ids, vec!["c1".to_string()]);
        assert_eq!(p.client_id, None);
        assert_eq!(p.accent, "#ffaa00");
        assert_eq!(p.layout, "case_study");
        assert_eq!(p.metrics.len(), 1);
        assert_eq!(p.tools, vec!["Figma".to_string(), "React".to_string()]);
        assert_eq!(p.status, "protected");
        assert!(ProjectInput::from_form(&form(&[("title", "x"), ("image", "javascript:alert(1)")]), ok).is_err());
        assert!(ProjectInput::from_form(&form(&[("title", "x"), ("m_value", "10")]), ok).is_err());
        assert!(ProjectInput::from_form(&form(&[("title", "x"), ("live_url", "data:text/html,hi")]), ok).is_err());
    }

    #[test]
    fn passwords_are_hashed_and_unlocks_are_per_project() {
        let h = hash_password("hunter22", "salt");
        assert!(password_matches(&h, "hunter22"));
        assert!(!password_matches(&h, "hunter23"));
        assert!(!password_matches("nonsense", "x"));
        let v = unlock_value("sec", "p1", &h);
        let opened = unlocked("sec", &format!("{v},p2.forged"), |id| (id == "p1" || id == "p2").then(|| h.clone()));
        assert_eq!(opened, vec!["p1".to_string()]);
        // A changed password closes it again.
        let h2 = hash_password("new", "salt2");
        assert!(unlocked("sec", &v, |_| Some(h2.clone())).is_empty());
    }

    #[test]
    fn resume_dates_are_years_or_months() {
        assert_eq!(clean_month("2021-3").as_deref(), Some("2021-03"));
        assert_eq!(clean_month("2021").as_deref(), Some("2021"));
        assert_eq!(clean_month("").as_deref(), Some(""));
        assert_eq!(clean_month("21"), None);
        assert_eq!(clean_month("2021-13"), None);
        assert_eq!(span_label("2021-03", "", true), "Mar 2021 – Present");
        assert_eq!(span_label("2016", "2019", false), "2016 – 2019");
        assert_eq!(span_label("2020", "2020", false), "2020");
        let e = EntryInput::from_form(&form(&[("title", "Designer"), ("start", "2022"), ("end", "2020")]));
        assert!(e.unwrap_err().contains("before"));
    }

    #[test]
    fn enquiries_keep_only_offered_budgets_and_services() {
        let mut f = std::collections::HashMap::new();
        for (k, v) in [("name", "Sam"), ("email", "Sam@Example.com"), ("message", "We need a new website soon."), ("budget", "$1M"), ("services", "Branding,Hacking")] {
            f.insert(k.to_string(), v.to_string());
        }
        let e = EnquiryInput::from_form(&f, &["Under $5k".into()], &["Branding".into()]).unwrap();
        assert_eq!(e.email, "sam@example.com");
        assert_eq!(e.budget, "");
        assert_eq!(e.services, vec!["Branding".to_string()]);
        f.insert("email".into(), "nope".into());
        assert!(EnquiryInput::from_form(&f, &[], &[]).is_err());
    }

    #[test]
    fn settings_default_to_a_closed_personal_portfolio() {
        let s = Settings::default();
        assert!(!s.enabled);
        assert_eq!(s.mode, Mode::Personal);
        assert_eq!(s.title, "Work");
        assert_eq!(s.budgets.len(), 5);
        let s = Settings::read(|k| match k { SETTING_MODE => Some("studio".into()), SETTING_SOCIALS => Some(r#"{"github":"https://github.com/x","evil":"javascript:1","x":"javascript:1"}"#.into()), _ => None });
        assert!(s.studio());
        assert_eq!(s.title, "Our work");
        assert_eq!(s.socials, vec![("github".to_string(), "https://github.com/x".to_string())]);
    }

    #[test]
    fn the_checklist_asks_studios_and_people_for_different_things() {
        let p = checklist(&Facts::default());
        assert!(p.iter().any(|(_, l, _)| l.contains("résumé")));
        let s = checklist(&Facts { studio: true, ..Default::default() });
        assert!(s.iter().any(|(_, l, _)| l.contains("team")));
        assert!(!s.iter().any(|(_, l, _)| l.contains("résumé")));
    }

    #[test]
    fn colours_and_lists_are_cleaned() {
        assert_eq!(clean_colour("#ABC"), "#abc");
        assert_eq!(clean_colour("red;background:url(x)"), "");
        assert_eq!(list("a, b,\nA, ,c", 10, 10), vec!["a", "b", "c"]);
    }

    #[test]
    fn the_address_can_move_but_not_onto_another_tool() {
        assert_eq!(Paths::new("work").unwrap().public_path("/portfolio/x/"), "/work/x/");
        assert_eq!(Paths::new("work").unwrap().internal_path("/work").as_deref(), Some("/portfolio"));
        assert!(Paths::new("podcast").is_none());
    }
}
