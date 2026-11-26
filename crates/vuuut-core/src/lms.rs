//! LMS & Online Courses: the rules.
//!
//! What LearnDash, LifterLMS, Tutor LMS, Sensei and LearnPress share, in one
//! tool: courses built from sections and lessons, lessons that are text,
//! video, a quiz or an assignment, enrolment that is open, free, for
//! members, for chosen tiers or bought in the store, progress and drip,
//! reviews, and certificates anybody can verify.
//!
//! Pure. Staff write courses in the admin; students answer quizzes, hand in
//! assignments and write reviews on the site. Every one of those forms is
//! rebuilt here — bounded text, http(s) links only, numbers in range, kinds
//! from short lists — and nothing a student typed is stored as markup.
//!
//! The pieces: [`Settings`] (everything off until switched on), [`Paths`]
//! (the catalogue's address), [`Access`] and friends, [`CourseInput`],
//! [`LessonInput`], [`Question`] with [`grade`], [`video_embed`],
//! [`unlocks_at`], [`progress`], [`duration_label`].

use crate::directory::{clean, clean_text, safe_image, safe_url, slugify};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

/// Who can take a course.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Every lesson is public; no account, no enrolment, no progress.
    Open,
    /// Free; enrolling needs a member account (made on the spot).
    Free,
    /// Any signed-in member (or only paid members, see `tier_ids = paid`).
    Members,
    /// Members of chosen tiers.
    Tiers,
    /// Bought: a store product grants it when its order is paid.
    Paid,
}

impl Access {
    pub const ALL: [Access; 5] = [Access::Free, Access::Paid, Access::Members, Access::Tiers, Access::Open];
    pub fn key(self) -> &'static str {
        match self {
            Access::Open => "open",
            Access::Free => "free",
            Access::Members => "members",
            Access::Tiers => "tiers",
            Access::Paid => "paid",
        }
    }
    pub fn from_key(k: &str) -> Access {
        Access::ALL.into_iter().find(|a| a.key() == k.trim()).unwrap_or(Access::Free)
    }
    pub fn label(self) -> &'static str {
        match self {
            Access::Open => "Open — no sign-up",
            Access::Free => "Free — enrol with an account",
            Access::Members => "Members",
            Access::Tiers => "Chosen tiers",
            Access::Paid => "Paid — buy in the store",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Access::Open => "Anyone can read every lesson. Nothing is tracked.",
            Access::Free => "Students enrol in one click; a member account is made for them if they have none.",
            Access::Members => "Signed-in members enrol in one click. Choose “paid members only” to keep it for paying members.",
            Access::Tiers => "Only members of the tiers ticked below can enrol.",
            Access::Paid => "Students buy the linked store product. Paying for it enrols them, whoever marks it paid.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    All,
    Beginner,
    Intermediate,
    Advanced,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::All, Level::Beginner, Level::Intermediate, Level::Advanced];
    pub fn key(self) -> &'static str {
        match self {
            Level::All => "all",
            Level::Beginner => "beginner",
            Level::Intermediate => "intermediate",
            Level::Advanced => "advanced",
        }
    }
    pub fn from_key(k: &str) -> Level {
        Level::ALL.into_iter().find(|l| l.key() == k.trim()).unwrap_or(Level::All)
    }
    pub fn label(self) -> &'static str {
        match self {
            Level::All => "All levels",
            Level::Beginner => "Beginner",
            Level::Intermediate => "Intermediate",
            Level::Advanced => "Advanced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CourseStatus {
    Draft,
    Published,
    Archived,
}

impl CourseStatus {
    pub const ALL: [CourseStatus; 3] = [CourseStatus::Draft, CourseStatus::Published, CourseStatus::Archived];
    pub fn key(self) -> &'static str {
        match self {
            CourseStatus::Draft => "draft",
            CourseStatus::Published => "published",
            CourseStatus::Archived => "archived",
        }
    }
    pub fn from_key(k: &str) -> CourseStatus {
        CourseStatus::ALL.into_iter().find(|s| s.key() == k.trim()).unwrap_or(CourseStatus::Draft)
    }
    pub fn label(self) -> &'static str {
        match self {
            CourseStatus::Draft => "Draft",
            CourseStatus::Published => "Published",
            CourseStatus::Archived => "Archived — enrolled students keep access",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LessonKind {
    Lesson,
    Quiz,
    Assignment,
}

impl LessonKind {
    pub const ALL: [LessonKind; 3] = [LessonKind::Lesson, LessonKind::Quiz, LessonKind::Assignment];
    pub fn key(self) -> &'static str {
        match self {
            LessonKind::Lesson => "lesson",
            LessonKind::Quiz => "quiz",
            LessonKind::Assignment => "assignment",
        }
    }
    pub fn from_key(k: &str) -> LessonKind {
        LessonKind::ALL.into_iter().find(|s| s.key() == k.trim()).unwrap_or(LessonKind::Lesson)
    }
    pub fn label(self) -> &'static str {
        match self {
            LessonKind::Lesson => "Lesson",
            LessonKind::Quiz => "Quiz",
            LessonKind::Assignment => "Assignment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    Single,
    Multiple,
    TrueFalse,
    Text,
}

impl QuestionKind {
    pub const ALL: [QuestionKind; 4] = [QuestionKind::Single, QuestionKind::Multiple, QuestionKind::TrueFalse, QuestionKind::Text];
    pub fn key(self) -> &'static str {
        match self {
            QuestionKind::Single => "single",
            QuestionKind::Multiple => "multiple",
            QuestionKind::TrueFalse => "truefalse",
            QuestionKind::Text => "text",
        }
    }
    pub fn from_key(k: &str) -> QuestionKind {
        QuestionKind::ALL.into_iter().find(|s| s.key() == k.trim()).unwrap_or(QuestionKind::Single)
    }
    pub fn label(self) -> &'static str {
        match self {
            QuestionKind::Single => "One answer",
            QuestionKind::Multiple => "Several answers",
            QuestionKind::TrueFalse => "True or false",
            QuestionKind::Text => "Typed answer",
        }
    }
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "lms_enabled";
pub const SETTING_TITLE: &str = "lms_title";
pub const SETTING_INTRO: &str = "lms_intro";
pub const SETTING_BASE_PATH: &str = "lms_base_path";
pub const SETTING_PER_PAGE: &str = "lms_per_page";
pub const SETTING_CURRENCY: &str = "lms_currency";
pub const SETTING_REVIEWS: &str = "lms_reviews";
pub const SETTING_MODERATE_REVIEWS: &str = "lms_moderate_reviews";
pub const SETTING_CERTIFICATES: &str = "lms_certificates";
pub const SETTING_SIGNER: &str = "lms_certificate_signer";
pub const SETTING_SHOW_STUDENTS: &str = "lms_show_students";

pub const DEFAULT_BASE: &str = "courses";
pub const DEFAULT_TITLE: &str = "Courses";
pub const DEFAULT_PER_PAGE: i64 = 12;
/// The catalogue's own addresses; never a course's slug.
pub const RESERVED_SLUGS: &[&str] = &["assets", "category", "dashboard", "certificate", "learn", "search"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Where the catalogue lives, `courses` unless changed.
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

    /// One to three lowercase segments, outside the store, the directory
    /// and the system's own roots.
    pub fn new(base: &str) -> Option<Self> {
        if base.trim().trim_matches('/').is_empty() {
            return Some(Self::default());
        }
        let clean = crate::ecom::clean_address(base)?;
        let first = clean.split('/').next().unwrap_or_default();
        if matches!(first, "shop" | "directory" | "book" | "podcast" | "portfolio") {
            return None;
        }
        Some(Paths { base: clean })
    }

    /// `/courses/x/` → the address the visitor sees.
    pub fn public_path(&self, internal: &str) -> String {
        if self.base == DEFAULT_BASE {
            return internal.into();
        }
        match internal.strip_prefix("/courses/") {
            Some(tail) => format!("/{}/{tail}", self.base),
            None if internal == "/courses" => format!("/{}", self.base),
            None => internal.into(),
        }
    }

    /// A public address → the internal one, when it is inside the catalogue.
    pub fn internal_path(&self, public: &str) -> Option<String> {
        if self.base == DEFAULT_BASE {
            return None;
        }
        let rest = public.strip_prefix('/')?.strip_prefix(self.base.as_str())?;
        if rest.is_empty() {
            return Some("/courses".into());
        }
        rest.strip_prefix('/').map(|tail| format!("/courses/{tail}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub title: String,
    pub intro: String,
    pub per_page: i64,
    pub currency: String,
    pub reviews: bool,
    pub moderate_reviews: bool,
    pub certificates: bool,
    /// The name printed under the signature line of a certificate.
    pub signer: String,
    /// Enrolment counts on cards and course pages.
    pub show_students: bool,
    pub paths: Paths,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |k: &str, d: bool| match get(k).as_deref().map(|v| v.trim().trim_matches('"')) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => d,
        };
        let num = |k: &str| get(k).and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok());
        let title = clean(get(SETTING_TITLE).unwrap_or_default().trim_matches('"'), 60);
        let currency = crate::currency::configured(&get).or_else(|| get(SETTING_CURRENCY)
            .map(|c| c.trim().trim_matches('"').to_uppercase())
            .filter(|c| crate::directory::CURRENCIES.contains(&c.as_str()))
        ).unwrap_or_else(|| "USD".into());
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            title: if title.is_empty() { DEFAULT_TITLE.into() } else { title },
            intro: clean(get(SETTING_INTRO).unwrap_or_default().trim_matches('"'), 400),
            per_page: num(SETTING_PER_PAGE).filter(|n| (1..=96).contains(n)).unwrap_or(DEFAULT_PER_PAGE),
            currency,
            reviews: flag(SETTING_REVIEWS, true),
            moderate_reviews: flag(SETTING_MODERATE_REVIEWS, false),
            certificates: flag(SETTING_CERTIFICATES, true),
            signer: clean(get(SETTING_SIGNER).unwrap_or_default().trim_matches('"'), 80),
            show_students: flag(SETTING_SHOW_STUDENTS, true),
            paths: Paths::read(get),
        }
    }
}

/// `$49`, `€19.50`; a zero price is `Free`.
pub fn format_price(cents: i64, currency: &str) -> String {
    if cents <= 0 {
        return "Free".into();
    }
    crate::directory::format_price(cents, currency, crate::directory::PriceType::Fixed).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/// Non-empty trimmed lines, bounded.
pub fn lines(text: &str) -> Vec<String> {
    text.lines().map(|l| clean(l.trim_start_matches(['-', '*', '•']), 200)).filter(|l| !l.is_empty()).take(30).collect()
}

/// `90` → `1h 30m`; `45` → `45m`; `0` → empty.
pub fn duration_label(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, 0) => String::new(),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// `(label, address)` pairs from the stored `label|address` lines; lines
/// whose address is not http(s) or an uploaded file are dropped.
pub fn attachments(stored: &str) -> Vec<(String, String)> {
    stored
        .lines()
        .filter_map(|l| {
            let (label, url) = l.split_once('|').unwrap_or(("", l));
            let url = url.trim();
            let ok = (url.starts_with("/content/") && !url.contains("..")) || safe_url(url).is_some_and(|u| !u.is_empty());
            if !ok || url.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>')) {
                return None;
            }
            let label = clean(label, 120);
            let label = if label.is_empty() { url.rsplit('/').next().unwrap_or(url).to_string() } else { label };
            Some((label, url.to_string()))
        })
        .take(20)
        .collect()
}

/// A YouTube, Vimeo, Loom or Wistia link → the player address to put in an
/// iframe; a direct `.mp4`/`.webm` address → itself (for a `<video>`).
/// Anything else is no video. Only the id is carried across, so a link
/// cannot smuggle parameters into the player.
pub fn video_embed(url: &str) -> Option<(String, bool)> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || (u.starts_with("/content/") && !u.contains(".."))) {
        return None;
    }
    let id_ok = |s: &str| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let after = |marker: &str| lower.find(marker).map(|i| &u[i + marker.len()..]);
    let first = |s: &str| s.split(['?', '&', '#', '/']).next().unwrap_or("").to_string();
    if lower.contains("youtube.com/watch") {
        let q = u.split_once('?').map(|x| x.1).unwrap_or("");
        let id = q.split('&').find_map(|p| p.strip_prefix("v=")).map(first).unwrap_or_default();
        return id_ok(&id).then(|| (format!("https://www.youtube-nocookie.com/embed/{id}?rel=0"), true));
    }
    for marker in ["youtu.be/", "youtube.com/embed/", "youtube.com/shorts/", "youtube-nocookie.com/embed/"] {
        if let Some(rest) = after(marker) {
            let id = first(rest);
            return id_ok(&id).then(|| (format!("https://www.youtube-nocookie.com/embed/{id}?rel=0"), true));
        }
    }
    for marker in ["player.vimeo.com/video/", "vimeo.com/"] {
        if let Some(rest) = after(marker) {
            let id = first(rest);
            return (id_ok(&id) && id.chars().all(|c| c.is_ascii_digit())).then(|| (format!("https://player.vimeo.com/video/{id}?dnt=1"), true));
        }
    }
    for marker in ["loom.com/share/", "loom.com/embed/"] {
        if let Some(rest) = after(marker) {
            let id = first(rest);
            return id_ok(&id).then(|| (format!("https://www.loom.com/embed/{id}"), true));
        }
    }
    if let Some(rest) = after("wistia.com/medias/") {
        let id = first(rest);
        return id_ok(&id).then(|| (format!("https://fast.wistia.net/embed/iframe/{id}"), true));
    }
    let path = lower.split(['?', '#']).next().unwrap_or("");
    if [".mp4", ".webm", ".m4v", ".mov"].iter().any(|e| path.ends_with(e)) && !u.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>')) {
        return Some((u.to_string(), false));
    }
    None
}

/// When a dripped lesson opens: `drip_days` after enrolment.
pub fn unlocks_at(enrolled_at: &str, drip_days: i64) -> Option<String> {
    if drip_days <= 0 {
        return None;
    }
    let at = crate::dates::from_db_string(enrolled_at)? + chrono::Duration::days(drip_days);
    Some(crate::dates::to_db_string(&at))
}

/// Whole-number percentage, never above 100.
pub fn progress(done: usize, total: usize) -> i64 {
    if total == 0 {
        return 0;
    }
    ((done.min(total) * 100) / total) as i64
}

/// A certificate's public code: 12 characters, no look-alikes.
pub fn certificate_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..12)
        .map(|i| {
            let c = ALPHABET[crate::ids::random_below(ALPHABET.len() as u32) as usize] as char;
            if i == 4 || i == 8 { format!("-{c}") } else { c.to_string() }
        })
        .collect()
}

/// Whether a stored code could be one (for the verification address).
pub fn is_certificate_code(s: &str) -> bool {
    s.len() == 14 && s.chars().enumerate().all(|(i, c)| if i == 4 || i == 9 { c == '-' } else { c.is_ascii_uppercase() || c.is_ascii_digit() })
}

// ---------------------------------------------------------------------------
// course input
// ---------------------------------------------------------------------------

pub const MAX_TITLE: usize = 160;
pub const MAX_SUBTITLE: usize = 240;
pub const MAX_PLAIN: usize = 20_000;
pub const MAX_RICH: usize = 400_000;
pub const MAX_PRICE_CENTS: i64 = 100_000_000;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CourseInput {
    pub title: String,
    pub slug: String,
    pub subtitle: String,
    pub description: String,
    pub description_lexical: String,
    pub description_html: String,
    pub image: String,
    pub intro_video: String,
    pub category_id: Option<String>,
    pub instructor_id: Option<String>,
    pub level: String,
    pub language: String,
    pub duration_minutes: i64,
    pub status: String,
    pub access: String,
    pub price_cents: i64,
    pub compare_cents: i64,
    pub product_id: Option<String>,
    pub tier_ids: Vec<String>,
    pub outcomes: Vec<String>,
    pub requirements: Vec<String>,
    pub audience: Vec<String>,
    pub keywords: String,
    pub featured: bool,
    pub sequential: bool,
    pub certificate: bool,
    pub capacity: i64,
    pub access_days: i64,
    pub opens_at: Option<String>,
    pub closes_at: Option<String>,
    pub seo_title: String,
    pub seo_description: String,
}

/// `2026-12-01` or `2026-12-01T09:30` → a stored date-time, or nothing.
pub fn parse_when(raw: &str) -> Result<Option<String>, ()> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok(None);
    }
    let fmt = if v.len() == 10 { format!("{v}T00:00") } else { v.replace(' ', "T") };
    chrono::NaiveDateTime::parse_from_str(fmt.get(..16).unwrap_or(&fmt), "%Y-%m-%dT%H:%M")
        .map(|d| Some(d.format("%Y-%m-%d %H:%M:%S").to_string()))
        .map_err(|_| ())
}

impl CourseInput {
    /// The editor's form. `pairs` keeps repeated fields (`tier`).
    /// `exists` answers whether a category / staff user / product / tier
    /// id is real: `("category" | "instructor" | "product" | "tier", id)`.
    pub fn from_form(pairs: &[(String, String)], exists: impl Fn(&str, &str) -> bool) -> Result<CourseInput, String> {
        let form: HashMap<&str, &str> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let get = |k: &str| form.get(k).copied().unwrap_or("");
        let title = clean(get("title"), MAX_TITLE);
        if title.chars().count() < 3 {
            return Err("Give the course a title.".into());
        }
        let money = |k: &str, what: &str| -> Result<i64, String> {
            let p = crate::directory::parse_price(get(k)).ok_or(format!("The {what} must be a number, like 49 or 19.50."))?;
            if p > MAX_PRICE_CENTS { Err(format!("The {what} is too high.")) } else { Ok(p) }
        };
        let num = |k: &str, max: i64| get(k).trim().parse::<i64>().ok().map(|n| n.clamp(0, max)).unwrap_or(0);
        let access = Access::from_key(get("access"));
        let price_cents = money("price", "price")?;
        let compare_cents = money("compare_price", "regular price")?;
        let product_id = Some(get("product_id").trim().to_string()).filter(|p| !p.is_empty() && exists("product", p));
        let tier_ids: Vec<String> = pairs
            .iter()
            .filter(|(k, _)| k == "tier")
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| v == "paid" || exists("tier", v))
            .take(20)
            .collect();
        if access == Access::Tiers && tier_ids.iter().all(|t| t == "paid") {
            return Err("Tick at least one tier, or choose another kind of access.".into());
        }
        let intro_video = safe_url(get("intro_video")).ok_or("The introduction video must be an http(s) link.")?;
        if !intro_video.is_empty() && video_embed(&intro_video).is_none() {
            return Err("The introduction video must be a YouTube, Vimeo, Loom or Wistia link, or a video file.".into());
        }
        let opens_at = parse_when(get("opens_at")).map_err(|_| "The enrolment opening date is not a date.")?;
        let closes_at = parse_when(get("closes_at")).map_err(|_| "The enrolment closing date is not a date.")?;
        if let (Some(a), Some(b)) = (&opens_at, &closes_at) {
            if b <= a {
                return Err("Enrolment must close after it opens.".into());
            }
        }
        let slug = slugify(get("slug"));
        Ok(CourseInput {
            slug: if slug.is_empty() { slugify(&title) } else { slug },
            title,
            subtitle: clean(get("subtitle"), MAX_SUBTITLE),
            description: clean_text(get("description"), MAX_PLAIN),
            description_lexical: String::new(),
            description_html: String::new(),
            image: safe_image(get("image")).ok_or("The cover image must be an image address.")?,
            intro_video,
            category_id: Some(get("category_id").trim().to_string()).filter(|c| !c.is_empty() && exists("category", c)),
            instructor_id: Some(get("instructor_id").trim().to_string()).filter(|c| !c.is_empty() && exists("instructor", c)),
            level: Level::from_key(get("level")).key().into(),
            language: clean(get("language"), 40),
            duration_minutes: num("duration_minutes", 100_000),
            status: CourseStatus::from_key(get("status")).key().into(),
            access: access.key().into(),
            price_cents,
            compare_cents: if compare_cents > price_cents { compare_cents } else { 0 },
            product_id,
            tier_ids,
            outcomes: lines(get("outcomes")),
            requirements: lines(get("requirements")),
            audience: lines(get("audience")),
            keywords: clean(get("keywords"), 200),
            featured: get("featured") == "1",
            sequential: get("sequential") == "1",
            certificate: get("certificate") == "1",
            capacity: num("capacity", 1_000_000),
            access_days: num("access_days", 36_500),
            opens_at,
            closes_at,
            seo_title: clean(get("seo_title"), 70),
            seo_description: clean(get("seo_description"), 320),
        })
    }
}

// ---------------------------------------------------------------------------
// lessons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LessonInput {
    pub title: String,
    pub slug: String,
    pub kind: String,
    pub status: String,
    pub section_id: Option<String>,
    pub summary: String,
    pub content_lexical: String,
    pub content_html: String,
    pub video_url: String,
    pub duration_minutes: i64,
    pub preview: bool,
    pub drip_days: i64,
    pub attachments: Vec<(String, String)>,
    pub pass_mark: i64,
    pub attempts: i64,
    pub time_limit: i64,
    pub shuffle: bool,
    pub reveal: bool,
    pub points: i64,
}

impl LessonInput {
    /// The lesson editor's form. Attachments come as repeated
    /// `att_label` / `att_url` pairs, in order.
    pub fn from_form(pairs: &[(String, String)], section_exists: impl Fn(&str) -> bool) -> Result<LessonInput, String> {
        let form: HashMap<&str, &str> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let get = |k: &str| form.get(k).copied().unwrap_or("");
        let num = |k: &str, d: i64, lo: i64, hi: i64| get(k).trim().parse::<i64>().ok().map(|n| n.clamp(lo, hi)).unwrap_or(d);
        let title = clean(get("title"), MAX_TITLE);
        if title.chars().count() < 2 {
            return Err("Give the lesson a title.".into());
        }
        let video_url = safe_url(get("video_url")).ok_or("The video must be an http(s) link.")?;
        let video_url = if video_url.is_empty() { clean(get("video_url"), 500).starts_with("/content/").then(|| clean(get("video_url"), 500)).unwrap_or_default() } else { video_url };
        if !video_url.is_empty() && video_embed(&video_url).is_none() {
            return Err("The video must be a YouTube, Vimeo, Loom or Wistia link, or a video file.".into());
        }
        let labels: Vec<&str> = pairs.iter().filter(|(k, _)| k == "att_label").map(|(_, v)| v.as_str()).collect();
        let urls: Vec<&str> = pairs.iter().filter(|(k, _)| k == "att_url").map(|(_, v)| v.as_str()).collect();
        let stored: String = urls
            .iter()
            .enumerate()
            .filter(|(_, u)| !u.trim().is_empty())
            .map(|(i, u)| format!("{}|{}", clean(labels.get(i).copied().unwrap_or(""), 120).replace('|', "/"), u.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        let attachments = attachments(&stored);
        if attachments.len() < urls.iter().filter(|u| !u.trim().is_empty()).count() {
            return Err("Each attachment needs an http(s) address or an uploaded file.".into());
        }
        let slug = slugify(get("slug"));
        Ok(LessonInput {
            slug: if slug.is_empty() { slugify(&title) } else { slug },
            title,
            kind: LessonKind::from_key(get("kind")).key().into(),
            status: if get("status") == "draft" { "draft".into() } else { "published".into() },
            section_id: Some(get("section_id").trim().to_string()).filter(|s| !s.is_empty() && section_exists(s)),
            summary: clean(get("summary"), 300),
            content_lexical: String::new(),
            content_html: String::new(),
            video_url,
            duration_minutes: num("duration_minutes", 0, 0, 10_000),
            preview: get("preview") == "1",
            drip_days: num("drip_days", 0, 0, 3650),
            attachments,
            pass_mark: num("pass_mark", 70, 0, 100),
            attempts: num("attempts", 0, 0, 100),
            time_limit: num("time_limit", 0, 0, 600),
            shuffle: get("shuffle") == "1",
            reveal: get("reveal") == "1",
            points: num("points", 100, 1, 1000),
        })
    }
}

// ---------------------------------------------------------------------------
// quizzes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Choice {
    /// The answer's text.
    pub t: String,
    /// Whether it is (one of) the right answer(s).
    #[serde(default)]
    pub c: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Question {
    pub id: String,
    pub kind: String,
    pub prompt: String,
    pub options: Vec<Choice>,
    /// Accepted answers to a typed question.
    pub answers: Vec<String>,
    pub explanation: String,
    pub points: i64,
}

impl Question {
    pub fn kind(&self) -> QuestionKind {
        QuestionKind::from_key(&self.kind)
    }

    /// The editor's JSON — `[{id?, kind, prompt, options:[{t,c}], answer,
    /// explanation, points}]` — rebuilt. A question that cannot be marked
    /// (no right answer) is an error, so a quiz never silently fails people.
    pub fn list_from_json(raw: &str) -> Result<Vec<Question>, String> {
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        let v: serde_json::Value = serde_json::from_str(raw).map_err(|_| "The questions could not be read. Reload the page and try again.")?;
        let arr = v.as_array().ok_or("The questions could not be read.")?;
        let mut out = Vec::new();
        for (i, q) in arr.iter().take(200).enumerate() {
            let s = |k: &str| q.get(k).and_then(|x| x.as_str()).unwrap_or("");
            let prompt = clean(s("prompt"), 1000);
            if prompt.is_empty() {
                continue;
            }
            let kind = QuestionKind::from_key(s("kind"));
            let mut options: Vec<Choice> = q
                .get("options")
                .and_then(|o| o.as_array())
                .map(|a| a.iter().filter_map(|c| {
                    let t = clean(c.get("t").and_then(|x| x.as_str()).unwrap_or(""), 300);
                    (!t.is_empty()).then(|| Choice { t, c: c.get("c").and_then(|x| x.as_bool()).unwrap_or(false) })
                }).take(12).collect())
                .unwrap_or_default();
            let answers: Vec<String> = s("answer").split('|').map(|a| clean(a, 100)).filter(|a| !a.is_empty()).take(10).collect();
            match kind {
                QuestionKind::TrueFalse => {
                    let truth = options.iter().find(|o| o.c).map(|o| o.t.eq_ignore_ascii_case("true")).unwrap_or(true);
                    options = vec![Choice { t: "True".into(), c: truth }, Choice { t: "False".into(), c: !truth }];
                }
                QuestionKind::Single | QuestionKind::Multiple => {
                    if options.len() < 2 {
                        return Err(format!("Question {} needs at least two answers to choose from.", i + 1));
                    }
                    let right = options.iter().filter(|o| o.c).count();
                    if right == 0 {
                        return Err(format!("Question {} has no right answer marked.", i + 1));
                    }
                    if kind == QuestionKind::Single && right > 1 {
                        return Err(format!("Question {} allows one answer but has {right} marked right. Switch it to “Several answers”.", i + 1));
                    }
                }
                QuestionKind::Text => {
                    if answers.is_empty() {
                        return Err(format!("Question {} needs at least one accepted answer.", i + 1));
                    }
                    options.clear();
                }
            }
            let id = s("id");
            out.push(Question {
                id: if crate::ids::is_objectid(id) { id.to_string() } else { String::new() },
                kind: kind.key().into(),
                prompt,
                options,
                answers,
                explanation: clean(s("explanation"), 1000),
                points: q.get("points").and_then(|p| p.as_i64().or_else(|| p.as_str().and_then(|s| s.trim().parse().ok()))).unwrap_or(1).clamp(1, 100),
            });
        }
        Ok(out)
    }
}

/// How one question went.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Marked {
    pub id: String,
    pub correct: bool,
    pub points: i64,
    /// What the student chose or typed, as given back to them.
    pub given: Vec<String>,
}

fn fold(s: &str) -> String {
    crate::site_search::normalize(s.trim()).split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Mark an attempt. `answer(question_id)` is every value the student sent
/// for that question (option indexes as text, or the typed answer).
/// Returns `(points, max_points, per question)`. All or nothing per
/// question: a multiple-answer question scores only when exactly the right
/// set is chosen.
pub fn grade(questions: &[Question], answer: impl Fn(&str) -> Vec<String>) -> (i64, i64, Vec<Marked>) {
    let mut points = 0;
    let mut max = 0;
    let mut out = Vec::new();
    for q in questions {
        max += q.points;
        let given = answer(&q.id);
        let correct = match q.kind() {
            QuestionKind::Text => given.first().is_some_and(|g| !g.trim().is_empty() && q.answers.iter().any(|a| fold(a) == fold(g))),
            _ => {
                let mut chosen: Vec<usize> = given.iter().filter_map(|g| g.trim().parse::<usize>().ok()).filter(|i| *i < q.options.len()).collect();
                chosen.sort_unstable();
                chosen.dedup();
                let right: Vec<usize> = q.options.iter().enumerate().filter(|(_, o)| o.c).map(|(i, _)| i).collect();
                !chosen.is_empty() && chosen == right
            }
        };
        if correct {
            points += q.points;
        }
        let shown = match q.kind() {
            QuestionKind::Text => given.iter().take(1).map(|g| clean(g, 100)).collect(),
            _ => given.iter().filter_map(|g| g.trim().parse::<usize>().ok()).filter_map(|i| q.options.get(i).map(|o| o.t.clone())).collect(),
        };
        out.push(Marked { id: q.id.clone(), correct, points: if correct { q.points } else { 0 }, given: shown });
    }
    (points, max, out)
}

/// A review from the site: 1–5 stars and words.
pub fn review_from_form(form: &HashMap<String, String>) -> Result<(i64, String, String), String> {
    let rating = form.get("rating").and_then(|r| r.trim().parse::<i64>().ok()).filter(|r| (1..=5).contains(r)).ok_or("Choose from one to five stars.")?;
    let body = clean_text(form.get("body").map(String::as_str).unwrap_or(""), 2000);
    Ok((rating, clean(form.get("title").map(String::as_str).unwrap_or(""), 120), body))
}

/// An assignment handed in: words and/or a link.
pub fn submission_from_form(form: &HashMap<String, String>) -> Result<(String, String), String> {
    let body = clean_text(form.get("body").map(String::as_str).unwrap_or(""), MAX_PLAIN);
    let link = safe_url(form.get("link").map(String::as_str).unwrap_or("")).ok_or("The link must be an http(s) address.")?;
    if body.chars().count() < 2 && link.is_empty() {
        return Err("Write your answer, or add a link to your work.".into());
    }
    Ok((body, link))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn ships_off_with_sensible_defaults() {
        let s = Settings::default();
        assert!(!s.enabled);
        assert!(s.reviews && s.certificates && !s.moderate_reviews && s.show_students);
        assert_eq!((s.title.as_str(), s.per_page, s.currency.as_str(), s.paths.base.as_str()), ("Courses", 12, "USD", "courses"));
    }

    #[test]
    fn paths_move_the_catalogue() {
        let p = Paths::new("learn/academy").unwrap();
        assert_eq!(p.public_path("/courses/x/"), "/learn/academy/x/");
        assert_eq!(p.internal_path("/learn/academy/x/").as_deref(), Some("/courses/x/"));
        assert_eq!(p.internal_path("/learn/other/"), None);
        assert!(Paths::new("shop/courses").is_none());
        assert_eq!(Paths::new("").unwrap(), Paths::default());
        assert_eq!(Paths::default().public_path("/courses/a/"), "/courses/a/");
    }

    #[test]
    fn a_course_form_is_rebuilt() {
        let p = pairs(&[
            ("title", "  Watercolour <i>basics</i> "), ("access", "tiers"), ("tier", "t1"), ("tier", "bogus"),
            ("price", "49"), ("compare_price", "30"), ("level", "advanced"), ("outcomes", "- One\n\n* Two\n"),
            ("intro_video", "https://youtu.be/dQw4w9WgXcQ"), ("opens_at", "2026-12-01"), ("closes_at", "2026-12-31T18:00"),
        ]);
        let c = CourseInput::from_form(&p, |k, id| k == "tier" && id == "t1").unwrap();
        assert_eq!(c.slug, "watercolour-i-basics-i");
        assert_eq!(c.tier_ids, ["t1"]);
        assert_eq!((c.price_cents, c.compare_cents), (4900, 0), "a lower regular price is dropped");
        assert_eq!(c.outcomes, ["One", "Two"]);
        assert_eq!(c.opens_at.as_deref(), Some("2026-12-01 00:00:00"));
        assert!(CourseInput::from_form(&pairs(&[("title", "Abc"), ("access", "tiers")]), |_, _| true).unwrap_err().contains("tier"));
        assert!(CourseInput::from_form(&pairs(&[("title", "Abc"), ("intro_video", "https://example.com/x")]), |_, _| true).unwrap_err().contains("YouTube"));
        assert!(CourseInput::from_form(&pairs(&[("title", "Abc"), ("opens_at", "2026-12-02"), ("closes_at", "2026-12-01")]), |_, _| true).unwrap_err().contains("close"));
    }

    #[test]
    fn videos_become_safe_players() {
        assert_eq!(video_embed("https://www.youtube.com/watch?v=abc_123&t=9").unwrap().0, "https://www.youtube-nocookie.com/embed/abc_123?rel=0");
        assert_eq!(video_embed("https://vimeo.com/76979871").unwrap().0, "https://player.vimeo.com/video/76979871?dnt=1");
        assert_eq!(video_embed("https://cdn.example.com/a.mp4"), Some(("https://cdn.example.com/a.mp4".into(), false)));
        assert!(video_embed("javascript:alert(1)").is_none());
        assert!(video_embed("https://youtu.be/\"><script>").is_none());
    }

    #[test]
    fn quizzes_are_checked_and_marked() {
        let raw = r#"[{"kind":"single","prompt":"2+2?","options":[{"t":"3"},{"t":"4","c":true}],"points":2},
            {"kind":"multiple","prompt":"Primes","options":[{"t":"2","c":true},{"t":"4"},{"t":"5","c":true}]},
            {"kind":"text","prompt":"Capital of France","answer":"Paris|Paree"},
            {"kind":"truefalse","prompt":"Sky is blue","options":[{"t":"True","c":true}]}]"#;
        let mut qs = Question::list_from_json(raw).unwrap();
        for (i, q) in qs.iter_mut().enumerate() {
            q.id = format!("q{i}");
        }
        let given = |id: &str| -> Vec<String> {
            match id {
                "q0" => vec!["1".into()],
                "q1" => vec!["0".into(), "2".into()],
                "q2" => vec!["  paris ".into()],
                _ => vec!["1".into()],
            }
        };
        let (p, max, marks) = grade(&qs, given);
        assert_eq!((p, max), (4, 5));
        assert!(marks[2].correct && !marks[3].correct);
        assert_eq!(marks[0].given, ["4"]);
        assert!(Question::list_from_json(r#"[{"kind":"single","prompt":"x","options":[{"t":"a"},{"t":"b"}]}]"#).unwrap_err().contains("no right answer"));
        assert!(Question::list_from_json(r#"[{"kind":"single","prompt":"x","options":[{"t":"a","c":true},{"t":"b","c":true}]}]"#).unwrap_err().contains("Several"));
        assert!(Question::list_from_json(r#"[{"kind":"text","prompt":"x"}]"#).unwrap_err().contains("accepted"));
    }

    #[test]
    fn small_helpers() {
        assert_eq!(duration_label(95), "1h 35m");
        assert_eq!(duration_label(120), "2h");
        assert_eq!(progress(3, 4), 75);
        assert_eq!(progress(0, 0), 0);
        let c = certificate_code();
        assert!(is_certificate_code(&c), "{c}");
        assert_eq!(attachments("Slides|https://x.io/a.pdf\nbad|javascript:x\n|/content/files/b.zip"), vec![("Slides".into(), "https://x.io/a.pdf".into()), ("b.zip".into(), "/content/files/b.zip".into())]);
        assert_eq!(unlocks_at("2026-01-01 10:00:00", 3).as_deref(), Some("2026-01-04 10:00:00"));
        assert_eq!(format_price(0, "USD"), "Free");
        assert_eq!(format_price(4900, "USD"), "$49");
    }
}
