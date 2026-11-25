//! Directory & Classified Listings: the rules.
//!
//! One tool, two kinds of listing that share almost everything:
//!
//! * a **directory entry** — a business, a place, a person, a resource —
//!   that stays until somebody takes it down;
//! * a **classified** — for sale, wanted, a job, a service — with a price
//!   and a life: it expires after the site's chosen number of days.
//!
//! Pure. This is the trust boundary twice over — staff fill in listings in
//! the admin, and (when the site allows it) anybody fills one in on the
//! public submission form — so [`ListingInput::from_form`] rebuilds every
//! field: bounded, links http(s) only, an email that is an email, a price
//! that is a number, a kind and a price type from short lists. Nothing a
//! visitor typed is stored as markup; descriptions are plain text and the
//! renderer turns blank lines into paragraphs.
//!
//! Also here: [`Settings`] (everything off — no public directory, no
//! submissions — until somebody switches it on), [`visible`] (published
//! and not expired), [`format_price`], [`slugify`], and [`paragraphs`].

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    Classified,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Directory => "directory",
            Kind::Classified => "classified",
        }
    }
    pub fn from_key(k: &str) -> Kind {
        if k.trim() == "classified" { Kind::Classified } else { Kind::Directory }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Directory => "Directory entry",
            Kind::Classified => "Classified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Published,
    Rejected,
    Draft,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::Pending, Status::Published, Status::Draft, Status::Rejected];
    pub fn key(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Published => "published",
            Status::Rejected => "rejected",
            Status::Draft => "draft",
        }
    }
    pub fn from_key(k: &str) -> Status {
        match k.trim() {
            "published" => Status::Published,
            "rejected" => Status::Rejected,
            "draft" => Status::Draft,
            _ => Status::Pending,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "Waiting for review",
            Status::Published => "Published",
            Status::Rejected => "Rejected",
            Status::Draft => "Draft",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceType {
    None,
    Fixed,
    Negotiable,
    Free,
    OnRequest,
}

impl PriceType {
    pub const ALL: [PriceType; 5] = [PriceType::Fixed, PriceType::Negotiable, PriceType::Free, PriceType::OnRequest, PriceType::None];
    pub fn key(self) -> &'static str {
        match self {
            PriceType::None => "none",
            PriceType::Fixed => "fixed",
            PriceType::Negotiable => "negotiable",
            PriceType::Free => "free",
            PriceType::OnRequest => "on_request",
        }
    }
    pub fn from_key(k: &str) -> PriceType {
        PriceType::ALL.into_iter().find(|p| p.key() == k.trim()).unwrap_or(PriceType::None)
    }
    pub fn label(self) -> &'static str {
        match self {
            PriceType::None => "No price",
            PriceType::Fixed => "Fixed price",
            PriceType::Negotiable => "Or nearest offer",
            PriceType::Free => "Free",
            PriceType::OnRequest => "Price on request",
        }
    }
}

/// `2500` cents, `"USD"`, fixed → `$25.00`; free → `Free`. Currencies the
/// site is likely to use get their symbol; any other code is printed as is.
pub fn format_price(cents: i64, currency: &str, t: PriceType) -> Option<String> {
    let symbol = crate::ecom::symbol(currency);
    let amount = || {
        let whole = cents / 100;
        let frac = cents % 100;
        let mut digits = String::new();
        let s = whole.to_string();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                digits.push(',');
            }
            digits.push(c);
        }
        let n = if frac == 0 { digits } else { format!("{digits}.{frac:02}") };
        if symbol.is_empty() { format!("{n} {currency}") } else { format!("{symbol}{n}") }
    };
    match t {
        PriceType::None => None,
        PriceType::Free => Some("Free".into()),
        PriceType::OnRequest => Some("Price on request".into()),
        PriceType::Fixed => Some(amount()),
        PriceType::Negotiable => Some(format!("{} ONO", amount())),
    }
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "directory_enabled";
pub const SETTING_TITLE: &str = "directory_title";
pub const SETTING_INTRO: &str = "directory_intro";
pub const SETTING_SUBMISSIONS: &str = "directory_submissions";
pub const SETTING_MEMBERS_ONLY: &str = "directory_members_only";
pub const SETTING_MODERATE: &str = "directory_moderate";
pub const SETTING_CLASSIFIED_DAYS: &str = "directory_classified_days";
pub const SETTING_PER_PAGE: &str = "directory_per_page";
pub const SETTING_SHOW_CONTACT: &str = "directory_show_contact";
pub const SETTING_BASE_PATH: &str = "directory_base_path";
pub const SETTING_LISTING_PATH: &str = "directory_listing_path";
pub const DEFAULT_BASE: &str = "directory";
pub const RESERVED_SLUGS: &[&str] = &["assets", "category", "submit"];
pub const SETTING_CURRENCY: &str = "directory_currency";

pub const DAYS: [i64; 5] = [7, 14, 30, 60, 90];
pub const DEFAULT_PER_PAGE: i64 = 10;
pub const CURRENCIES: [&str; 11] = ["USD", "EUR", "GBP", "CAD", "AUD", "INR", "BDT", "JPY", "SGD", "NZD", "HKD"];
pub const DEFAULT_TITLE: &str = "Directory";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub base: String,
    /// Empty places listings directly under the directory.
    pub listing: String,
}

impl Default for Paths {
    fn default() -> Self {
        Self { base: DEFAULT_BASE.into(), listing: String::new() }
    }
}

impl Paths {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        Self::new(
            &get(SETTING_BASE_PATH).unwrap_or_default(),
            &get(SETTING_LISTING_PATH).unwrap_or_default(),
        ).unwrap_or_default()
    }

    pub fn new(base: &str, listing: &str) -> Option<Self> {
        let normalize = |raw: &str| {
            crate::ecom::clean_address(raw).filter(|path| path.split('/').next() != Some("shop"))
        };
        let base = if base.trim().is_empty() { DEFAULT_BASE.into() } else { normalize(base)? };
        let listing = if listing.trim().is_empty() { String::new() } else { normalize(listing)? };
        // Keep the default address available for redirects. Within the directory
        // the submission form, categories and assets own their subtrees.
        if (base != DEFAULT_BASE && path_tail(&base, DEFAULT_BASE).is_some())
            || (base != DEFAULT_BASE && path_tail(&listing, DEFAULT_BASE).is_some())
            || path_tail(&listing, &base).is_some_and(is_directory_route)
            || (!listing.is_empty() && listing != base && path_tail(&base, &listing).is_some())
        {
            return None;
        }
        Some(Self { base, listing })
    }

    fn listing_prefix(&self) -> &str {
        if self.listing.is_empty() { &self.base } else { &self.listing }
    }

    pub fn overlaps_store(&self, base: &str, product: &str) -> bool {
        [&*self.base, self.listing_prefix()].iter().any(|directory| {
            [base, product].iter().any(|store| {
                path_tail(directory, store).is_some() || path_tail(store, directory).is_some()
            })
        })
    }

    pub fn public_path(&self, internal: &str) -> String {
        let (path, suffix) = internal.split_at(internal.find(['?', '#']).unwrap_or(internal.len()));
        let Some(tail) = path_tail(path.trim_start_matches('/'), DEFAULT_BASE) else {
            return internal.into();
        };
        let prefix = if is_listing(tail) { self.listing_prefix() } else { &self.base };
        format!("/{prefix}/{tail}{suffix}")
    }

    pub fn internal_path(&self, public: &str) -> Option<String> {
        let public = public.strip_prefix('/')?;
        if let Some(tail) = path_tail(public, &self.base) {
            if tail.is_empty() || is_directory_route(tail) {
                return Some(format!("/{DEFAULT_BASE}/{tail}"));
            }
        }
        let tail = path_tail(public, self.listing_prefix())?;
        if !is_listing(tail) {
            return None;
        }
        Some(format!("/{DEFAULT_BASE}/{}/", tail.trim_end_matches('/')))
    }
}

fn path_tail<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let tail = path.strip_prefix(prefix)?;
    if tail.is_empty() { Some(tail) } else { tail.strip_prefix('/') }
}

fn is_directory_route(tail: &str) -> bool {
    tail.split('/').next().is_some_and(|part| RESERVED_SLUGS.contains(&part))
}

fn is_listing(tail: &str) -> bool {
    if tail.is_empty() || is_directory_route(tail) {
        return false;
    }
    let mut parts = tail.trim_end_matches('/').split('/');
    let slug = parts.next().unwrap_or_default();
    !slug.is_empty()
        && match parts.next() {
            None => true,
            Some("go") => parts.next().is_some_and(|target| !target.is_empty()) && parts.next().is_none(),
            _ => false,
        }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Whether the directory is publicly accessible.
    pub enabled: bool,
    pub title: String,
    pub intro: String,
    /// The public "Add a listing" form.
    pub submissions: bool,
    pub members_only: bool,
    /// Submissions wait for review before they are shown.
    pub moderate: bool,
    pub classified_days: i64,
    pub per_page: i64,
    /// Show email and phone on the listing page. Off shows the website only.
    pub show_contact: bool,
    pub currency: String,
    pub paths: Paths,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |k: &str, d: bool| match get(k).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => d,
        };
        let num = |k: &str| get(k).and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok());
        let title = clean(&get(SETTING_TITLE).unwrap_or_default(), 60);
        let currency = crate::currency::configured(&get).or_else(|| get(SETTING_CURRENCY).map(|c| c.trim().trim_matches('"').to_uppercase()).filter(|c| CURRENCIES.contains(&c.as_str()))).unwrap_or_else(|| "USD".into());
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            title: if title.is_empty() { DEFAULT_TITLE.into() } else { title },
            intro: clean(&get(SETTING_INTRO).unwrap_or_default(), 400),
            submissions: flag(SETTING_SUBMISSIONS, false),
            members_only: flag(SETTING_MEMBERS_ONLY, false),
            moderate: flag(SETTING_MODERATE, true),
            classified_days: num(SETTING_CLASSIFIED_DAYS).filter(|d| DAYS.contains(d)).unwrap_or(30),
            per_page: num(SETTING_PER_PAGE).filter(|n| *n > 0).unwrap_or(DEFAULT_PER_PAGE),
            show_contact: flag(SETTING_SHOW_CONTACT, true),
            currency,
            paths: Paths::read(get),
        }
    }
}

// ---------------------------------------------------------------------------
// input
// ---------------------------------------------------------------------------

pub const MAX_TITLE: usize = 120;
pub const MAX_SUMMARY: usize = 240;
pub const MAX_DESCRIPTION: usize = 5_000;
/// The editor's description: the HTML is what is shown, so it gets room.
pub const MAX_RICH: usize = 200_000;
pub const MAX_URL: usize = 500;
pub const MAX_SHORT: usize = 120;
pub const MAX_KEYWORDS: usize = 200;
pub const MAX_PRICE_CENTS: i64 = 100_000_000_000;

pub fn clean(raw: &str, max: usize) -> String {
    raw.trim().chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// Plain text with line breaks kept (descriptions).
pub fn clean_text(raw: &str, max: usize) -> String {
    raw.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// An http(s) address, or nothing.
pub fn safe_url(raw: &str) -> Option<String> {
    let v = clean(raw, MAX_URL);
    if v.is_empty() {
        return Some(String::new());
    }
    let lower = v.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://")) && !v.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>' | '\\')) {
        Some(v)
    } else {
        None
    }
}

/// An image: an http(s) address or a path in this site's content folder.
pub fn safe_image(raw: &str) -> Option<String> {
    let v = clean(raw, MAX_URL);
    if v.starts_with("/content/images/") && !v.contains("..") && !v.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>')) {
        return Some(v);
    }
    safe_url(&v)
}

pub fn is_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else { return false };
    !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && e.len() <= 191
        && !e.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | ',' | ';' | '(' | ')'))
}

/// Digits, spaces and the usual punctuation of a phone number, or nothing.
pub fn clean_phone(raw: &str) -> Option<String> {
    let v = clean(raw, 40);
    if v.is_empty() {
        return Some(v);
    }
    let ok = v.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '+' | '-' | '(' | ')' | '.'))
        && v.chars().filter(|c| c.is_ascii_digit()).count() >= 5;
    ok.then_some(v)
}

/// `"25"`, `"1,250.50"`, `"9.9"` → cents. Anything else is `None`.
pub fn parse_price(raw: &str) -> Option<i64> {
    let v: String = raw.trim().chars().filter(|c| !matches!(c, ',' | ' ' | '$' | '€' | '£')).collect();
    if v.is_empty() {
        return Some(0);
    }
    let (whole, frac) = v.split_once('.').unwrap_or((&v, ""));
    if whole.is_empty() && frac.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) || frac.len() > 2 {
        return None;
    }
    let w: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: i64 = if frac.is_empty() { 0 } else { format!("{frac:0<2}").parse().ok()? };
    let cents = w.checked_mul(100)?.checked_add(f)?;
    (cents <= MAX_PRICE_CENTS).then_some(cents)
}

/// A plain-ASCII address: accents folded (`Café` → `cafe`), anything else
/// that is not a letter or digit a dash. ASCII because a URL with `é` in it
/// is percent-encoded differently by different clients, and the site's own
/// canonicaliser redirects all but one spelling.
pub fn slugify(raw: &str) -> String {
    let s: String = crate::site_search::normalize(raw.trim())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let joined = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    joined.chars().take(80).collect::<String>().trim_matches('-').to_string()
}

/// What a save writes, rebuilt from a form.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListingInput {
    pub title: String,
    pub slug: String,
    pub kind: String,
    pub category_id: Option<String>,
    pub summary: String,
    pub description: String,
    pub image: String,
    pub website: String,
    pub email: String,
    pub phone: String,
    pub address: String,
    pub location: String,
    pub price_cents: i64,
    pub price_type: String,
    pub keywords: String,
    pub featured: bool,
    pub submitter_name: String,
    pub submitter_email: String,
    /// The editor's Lexical and the HTML rendered from it. Set by the admin
    /// (which owns the renderer); a public submission leaves both empty and
    /// its plain `description` is shown as paragraphs.
    pub description_lexical: String,
    pub description_html: String,
    pub details: Details,
}

/// Everything a full directory entry has beyond the basics: what Google
/// Business Profile, Yelp and the city guides ask for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Details {
    pub logo: String,
    /// More images; the listing's `image` is the main one.
    pub gallery: Vec<String>,
    pub region: String,
    pub postal_code: String,
    pub country: String,
    pub latitude: String,
    pub longitude: String,
    pub booking_url: String,
    pub video_url: String,
    pub hours: Hours,
    pub hours_note: String,
    /// 0 = not set, 1 … 4 = $ … $$$$.
    pub price_level: i64,
    pub amenities: Vec<String>,
    pub payments: Vec<String>,
    /// `(network key, profile address)`, in [`SOCIALS`] order.
    pub socials: Vec<(String, String)>,
    pub established: String,
    pub seo_title: String,
    pub seo_description: String,
}

// ---------------------------------------------------------------------------
// the lists a listing chooses from
// ---------------------------------------------------------------------------

/// `(key, label)`: the features the big directories filter on.
pub const AMENITIES: &[(&str, &str)] = &[
    ("wifi", "Free Wi-Fi"),
    ("parking", "Parking"),
    ("wheelchair", "Wheelchair accessible"),
    ("outdoor", "Outdoor seating"),
    ("reservations", "Takes reservations"),
    ("delivery", "Delivery"),
    ("takeout", "Takeout"),
    ("pets", "Pet friendly"),
    ("kids", "Good for kids"),
    ("groups", "Good for groups"),
    ("vegetarian", "Vegetarian options"),
    ("vegan", "Vegan options"),
    ("bar", "Full bar"),
    ("ac", "Air conditioning"),
    ("live-music", "Live music"),
    ("late", "Open late"),
    ("breakfast", "Breakfast"),
    ("pool", "Pool"),
    ("gym", "Gym"),
    ("ev", "EV charging"),
    ("lgbtq", "LGBTQ+ friendly"),
    ("women-owned", "Women-owned"),
];

/// `(key, label)`.
pub const PAYMENTS: &[(&str, &str)] = &[
    ("cash", "Cash"),
    ("credit", "Credit cards"),
    ("debit", "Debit cards"),
    ("contactless", "Contactless"),
    ("apple-pay", "Apple Pay"),
    ("google-pay", "Google Pay"),
    ("bank", "Bank transfer"),
    ("crypto", "Crypto"),
];

/// `(key, label, the address it must start with)`.
pub const SOCIALS: &[(&str, &str, &str)] = &[
    ("facebook", "Facebook", "https://"),
    ("instagram", "Instagram", "https://"),
    ("x", "X", "https://"),
    ("tiktok", "TikTok", "https://"),
    ("youtube", "YouTube", "https://"),
    ("linkedin", "LinkedIn", "https://"),
    ("tripadvisor", "Tripadvisor", "https://"),
    ("yelp", "Yelp", "https://"),
];

pub const PRICE_LEVELS: [(i64, &str, &str); 4] = [(1, "$", "Inexpensive"), (2, "$$", "Moderate"), (3, "$$$", "Pricey"), (4, "$$$$", "Very expensive")];

pub fn label_of<'a>(list: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    list.iter().find(|(k, _)| *k == key).map(|(_, l)| *l)
}

/// A comma list kept only where it names something on `list`, in `list`'s order.
pub fn pick_keys(list: &[(&str, &str)], chosen: &[&str]) -> Vec<String> {
    list.iter().filter(|(k, _)| chosen.contains(k)).map(|(k, _)| k.to_string()).collect()
}

pub fn split_keys(stored: &str) -> Vec<String> {
    stored.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

// ---------------------------------------------------------------------------
// opening hours
// ---------------------------------------------------------------------------

/// `(key, long name, short name)`, Monday first.
pub const DAYS_OF_WEEK: [(&str, &str, &str); 7] = [
    ("mon", "Monday", "Mon"),
    ("tue", "Tuesday", "Tue"),
    ("wed", "Wednesday", "Wed"),
    ("thu", "Thursday", "Thu"),
    ("fri", "Friday", "Fri"),
    ("sat", "Saturday", "Sat"),
    ("sun", "Sunday", "Sun"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Day {
    /// Nothing said about this day.
    Unknown,
    Closed,
    AllDay,
    /// `(open, close)` as minutes after midnight; a close at or before the
    /// open runs past midnight.
    Open(Vec<(u16, u16)>),
}

/// A week of opening hours, Monday first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hours(pub [Day; 7]);

impl Default for Hours {
    fn default() -> Self {
        Hours(std::array::from_fn(|_| Day::Unknown))
    }
}

/// `"09:00"`, `"9:30"`, `"21:15"` → minutes.
pub fn parse_clock(raw: &str) -> Option<u16> {
    let (h, m) = raw.trim().split_once(':').unwrap_or((raw.trim(), "0"));
    let h: u16 = h.trim().parse().ok()?;
    let m: u16 = m.trim().parse().ok()?;
    (h <= 24 && m < 60 && (h < 24 || m == 0)).then_some(h * 60 + m)
}

fn hhmm(m: u16) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// `9:00 am`, `12:30 pm`, `midnight` — the way the site shows a time.
pub fn clock_label(m: u16) -> String {
    let m = m % (24 * 60);
    let (h, min) = (m / 60, m % 60);
    let (h12, mer) = match h {
        0 => (12, "am"),
        1..=11 => (h, "am"),
        12 => (12, "pm"),
        _ => (h - 12, "pm"),
    };
    if min == 0 { format!("{h12} {mer}") } else { format!("{h12}:{min:02} {mer}") }
}

impl Day {
    /// The stored form: `closed`, `24h`, `09:00-17:00,18:00-22:00`, or empty.
    pub fn key(&self) -> String {
        match self {
            Day::Unknown => String::new(),
            Day::Closed => "closed".into(),
            Day::AllDay => "24h".into(),
            Day::Open(r) => r.iter().map(|(a, b)| format!("{}-{}", hhmm(*a), hhmm(*b))).collect::<Vec<_>>().join(","),
        }
    }

    pub fn parse(raw: &str) -> Day {
        let v = raw.trim().to_ascii_lowercase();
        match v.as_str() {
            "" => Day::Unknown,
            "closed" => Day::Closed,
            "24h" | "open 24 hours" => Day::AllDay,
            _ => {
                let ranges: Vec<(u16, u16)> = v
                    .split(',')
                    .filter_map(|r| {
                        let (a, b) = r.split_once('-')?;
                        Some((parse_clock(a)?, parse_clock(b)?))
                    })
                    .take(3)
                    .collect();
                if ranges.is_empty() { Day::Unknown } else { Day::Open(ranges) }
            }
        }
    }

    /// `09:00–17:00, 18:00–22:00`, `Closed`, `Open 24 hours` — the plain
    /// form themes parse for "Open now".
    pub fn plain(&self) -> String {
        match self {
            Day::Open(r) => r.iter().map(|(a, b)| format!("{}–{}", hhmm(*a), hhmm(*b % (24 * 60)))).collect::<Vec<_>>().join(", "),
            other => other.label(),
        }
    }

    /// `9 am – 5 pm`, `Closed`, `Open 24 hours`.
    pub fn label(&self) -> String {
        match self {
            Day::Unknown => String::new(),
            Day::Closed => "Closed".into(),
            Day::AllDay => "Open 24 hours".into(),
            Day::Open(r) => r.iter().map(|(a, b)| format!("{} – {}", clock_label(*a), clock_label(*b))).collect::<Vec<_>>().join(", "),
        }
    }
}

impl Hours {
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|d| *d == Day::Unknown)
    }

    /// From the stored JSON object; anything unreadable is "not said".
    pub fn from_json(raw: &str) -> Hours {
        let v: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
        let mut h = Hours::default();
        for (i, (k, _, _)) in DAYS_OF_WEEK.iter().enumerate() {
            if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
                h.0[i] = Day::parse(s);
            }
        }
        h
    }

    pub fn to_json(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut m = serde_json::Map::new();
        for (i, (k, _, _)) in DAYS_OF_WEEK.iter().enumerate() {
            let key = self.0[i].key();
            if !key.is_empty() {
                m.insert(k.to_string(), serde_json::Value::String(key));
            }
        }
        serde_json::Value::Object(m).to_string()
    }

    /// From the editor: `hours_<day>` = `open` | `closed` | `24h` | ``, and
    /// for an open day `hours_<day>_open` / `hours_<day>_close` (plus an
    /// optional second shift `_open2` / `_close2`).
    pub fn from_form(get: impl Fn(&str) -> String) -> Hours {
        let mut h = Hours::default();
        for (i, (k, _, _)) in DAYS_OF_WEEK.iter().enumerate() {
            h.0[i] = match get(&format!("hours_{k}")).trim() {
                "closed" => Day::Closed,
                "24h" => Day::AllDay,
                "open" => {
                    let shift = |a: &str, b: &str| Some((parse_clock(&get(&format!("hours_{k}_{a}")))?, parse_clock(&get(&format!("hours_{k}_{b}")))?));
                    let r: Vec<(u16, u16)> = [shift("open", "close"), shift("open2", "close2")].into_iter().flatten().collect();
                    if r.is_empty() { Day::Unknown } else { Day::Open(r) }
                }
                _ => Day::Unknown,
            };
        }
        h
    }
}

impl Details {
    /// `staff` reads everything; a visitor's form has no such fields, and
    /// anything sent anyway is still rebuilt, never trusted.
    pub fn from_form(form: &HashMap<String, String>, all: &[(String, String)]) -> Result<Details, String> {
        let get = |k: &str| form.get(k).cloned().unwrap_or_default();
        let many = |k: &str| all.iter().filter(|(x, _)| x == k).map(|(_, v)| v.as_str()).collect::<Vec<_>>();
        let url = |k: &str, what: &str| safe_url(&get(k)).ok_or(format!("The {what} must be an http(s) address."));
        let coord = |k: &str, max: f64, what: &str| -> Result<String, String> {
            let v = clean(&get(k), 24);
            if v.is_empty() {
                return Ok(v);
            }
            match v.parse::<f64>() {
                Ok(n) if n.is_finite() && n.abs() <= max => Ok(format!("{}", (n * 1e6).round() / 1e6)),
                _ => Err(format!("The {what} must be a number between -{max} and {max}.")),
            }
        };
        let mut socials = Vec::new();
        for (key, label, _) in SOCIALS {
            let v = safe_url(&get(&format!("social_{key}"))).ok_or(format!("The {label} link must be an http(s) address."))?;
            if !v.is_empty() {
                socials.push((key.to_string(), v));
            }
        }
        let established = clean(&get("established"), 4);
        if !established.is_empty() && !(established.len() == 4 && established.chars().all(|c| c.is_ascii_digit())) {
            return Err("The year established must be a year, like 1998.".into());
        }
        let logo = safe_image(&get("logo")).ok_or("The logo must be an image address.")?;
        let gallery = many("gallery").into_iter().filter_map(safe_image).filter(|u| !u.is_empty()).take(24).collect();
        Ok(Details {
            logo,
            gallery,
            region: clean(&get("region"), 80),
            postal_code: clean(&get("postal_code"), 20),
            country: clean(&get("country"), 80),
            latitude: coord("latitude", 90.0, "latitude")?,
            longitude: coord("longitude", 180.0, "longitude")?,
            booking_url: url("booking_url", "booking link")?,
            video_url: url("video_url", "video link")?,
            hours: Hours::from_form(get),
            hours_note: clean(&get_owned(form, "hours_note"), 160),
            price_level: get_owned(form, "price_level").trim().parse::<i64>().ok().filter(|n| (0..=4).contains(n)).unwrap_or(0),
            amenities: pick_keys(AMENITIES, &many("amenity")),
            payments: pick_keys(PAYMENTS, &many("payment")),
            socials,
            established,
            seo_title: clean(&get_owned(form, "seo_title"), 70),
            seo_description: clean(&get_owned(form, "seo_description"), 320),
        })
    }
}

fn get_owned(form: &HashMap<String, String>, k: &str) -> String {
    form.get(k).cloned().unwrap_or_default()
}

/// `https://www.example.com/menu` → `example.com`.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.trim_start_matches("www.").to_string()
}

impl ListingInput {
    /// `staff` relaxes nothing about shape, only which fields are read: a
    /// visitor cannot set `featured` or the slug.
    pub fn from_form(form: &HashMap<String, String>, staff: bool, category_exists: impl Fn(&str) -> bool) -> Result<ListingInput, String> {
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let title = clean(get("title"), MAX_TITLE);
        if title.chars().count() < 3 {
            return Err("Give the listing a title.".into());
        }
        let kind = Kind::from_key(get("kind"));
        let website = safe_url(get("website")).ok_or("The website must be an http(s) address.")?;
        let image = safe_image(get("image")).ok_or("The image must be an http(s) address.")?;
        let email = clean(get("email"), 191).to_lowercase();
        if !email.is_empty() && !is_email(&email) {
            return Err("That email address does not look right.".into());
        }
        let phone = clean_phone(get("phone")).ok_or("That phone number does not look right.")?;
        let price_type = if kind == Kind::Classified { PriceType::from_key(get("price_type")) } else { PriceType::None };
        let price_cents = match price_type {
            PriceType::Fixed | PriceType::Negotiable => {
                let p = parse_price(get("price")).ok_or("The price must be a number, like 25 or 1,250.50.")?;
                if p == 0 {
                    return Err("Add a price, or choose Free or Price on request.".into());
                }
                p
            }
            _ => 0,
        };
        let submitter_email = clean(get("submitter_email"), 191).to_lowercase();
        if !staff && !is_email(&submitter_email) {
            return Err("Add your email address, so we can tell you when it is live.".into());
        }
        let category_id = Some(get("category_id").trim().to_string()).filter(|c| !c.is_empty() && category_exists(c));
        if !staff && category_id.is_none() {
            return Err("Choose a category.".into());
        }
        let description = clean_text(get("description"), MAX_DESCRIPTION);
        if !staff && description.chars().count() < 20 {
            return Err("Describe it in a sentence or two.".into());
        }
        let slug = if staff { slugify(get("slug")) } else { String::new() };
        Ok(ListingInput {
            slug: if slug.is_empty() { slugify(&title) } else { slug },
            title,
            kind: kind.key().into(),
            category_id,
            summary: clean(get("summary"), MAX_SUMMARY),
            description,
            image,
            website,
            email,
            phone,
            address: clean(get("address"), MAX_SHORT),
            location: clean(get("location"), 80),
            price_cents,
            price_type: price_type.key().into(),
            keywords: clean(get("keywords"), MAX_KEYWORDS),
            featured: staff && get("featured") == "1",
            submitter_name: clean(get("submitter_name"), 80),
            submitter_email,
            description_lexical: String::new(),
            description_html: String::new(),
            details: Details::default(),
        })
    }
}

/// Whether a listing is shown publicly at `now`: published, and either not
/// a classified or not past its expiry.
pub fn visible(status: &str, expires_at: Option<&str>, now: &str) -> bool {
    status == "published" && expires_at.is_none_or(|e| e > now)
}

/// Plain text to paragraphs, escaped — the only way a description becomes
/// HTML. Single newlines are line breaks.
pub fn paragraphs(text: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", esc(p).replace('\n', "<br>")))
        .collect()
}

/// A search term: words, bounded, for a `LIKE` the repo binds.
pub fn search_terms(q: &str) -> Vec<String> {
    q.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || matches!(c, '-' | '\'' | '.')).take(40).collect::<String>().to_lowercase())
        .filter(|w| w.chars().count() >= 2)
        .take(5)
        .collect()
}

pub const MIN_FILL_MS: i64 = 3_000;

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn everything_ships_off_but_moderation_is_on() {
        let s = Settings::default();
        assert!(!s.enabled && !s.submissions && !s.members_only);
        assert!(s.moderate && s.show_contact);
        assert_eq!((s.classified_days, s.per_page, s.currency.as_str(), s.title.as_str()), (30, 10, "USD", "Directory"));
    }

    #[test]
    fn a_visitor_submission_is_rebuilt_and_cannot_feature_itself() {
        let f = form(&[
            ("title", "  Vintage <b>bike</b>  "), ("kind", "classified"), ("price_type", "negotiable"), ("price", "1,250.5"),
            ("description", "A lovely bike in good order, ridden twice."), ("category_id", "cat1"),
            ("submitter_email", "Me@Example.com"), ("featured", "1"), ("slug", "hijack"),
            ("website", "https://shop.example/x"), ("phone", "+1 (555) 010-2030"),
        ]);
        let l = ListingInput::from_form(&f, false, |c| c == "cat1").unwrap();
        assert_eq!(l.title, "Vintage <b>bike</b>", "stored as typed; escaped where shown");
        assert_eq!((l.price_cents, l.price_type.as_str()), (125_050, "negotiable"));
        assert!(!l.featured, "a visitor cannot feature their own listing");
        assert_eq!(l.slug, "vintage-b-bike-b", "a visitor cannot choose the address");
        assert_eq!(l.submitter_email, "me@example.com");
    }

    #[test]
    fn each_field_says_what_is_wrong() {
        let base = [("title", "A title"), ("description", "Long enough description here."), ("category_id", "c"), ("submitter_email", "a@b.co")];
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut v: Vec<(&str, &str)> = base.to_vec();
            v.extend_from_slice(extra);
            ListingInput::from_form(&form(&v), false, |_| true)
        };
        assert!(with(&[("website", "javascript:alert(1)")]).unwrap_err().contains("website"));
        assert!(with(&[("image", "data:image/png;base64,x")]).unwrap_err().contains("image"));
        assert!(with(&[("email", "nope")]).unwrap_err().contains("email"));
        assert!(with(&[("phone", "call me")]).unwrap_err().contains("phone"));
        assert!(with(&[("kind", "classified"), ("price_type", "fixed"), ("price", "abc")]).unwrap_err().contains("price"));
        assert!(with(&[("kind", "classified"), ("price_type", "fixed")]).unwrap_err().contains("Free"));
        assert!(ListingInput::from_form(&form(&[("title", "Hi")]), true, |_| true).unwrap_err().contains("title"));
        assert!(ListingInput::from_form(&form(&[("title", "A title")]), false, |_| true).unwrap_err().contains("email"));
        assert!(with(&[("image", "/content/images/2026/a.jpg")]).is_ok());
    }

    #[test]
    fn prices_are_parsed_and_printed_carefully() {
        assert_eq!(parse_price("25"), Some(2500));
        assert_eq!(parse_price("$1,234.5"), Some(123_450));
        assert_eq!(parse_price(".99"), Some(99));
        assert_eq!(parse_price("1.234"), None);
        assert_eq!(parse_price("-5"), None);
        assert_eq!(parse_price("1e9"), None);
        assert_eq!(format_price(123_450, "USD", PriceType::Fixed).as_deref(), Some("$1,234.50"));
        assert_eq!(format_price(2500, "EUR", PriceType::Negotiable).as_deref(), Some("€25 ONO"));
        assert_eq!(format_price(100, "CHF", PriceType::Fixed).as_deref(), Some("1 CHF"));
        assert_eq!(format_price(0, "USD", PriceType::None), None);
    }

    #[test]
    fn opening_hours_round_trip_and_read_the_editor() {
        let stored = r#"{"mon":"closed","tue":"12:00-15:00,18:00-22:30","fri":"17:00-01:00","sun":"24h","sat":"nonsense"}"#;
        let h = Hours::from_json(stored);
        assert_eq!(h.0[0], Day::Closed);
        assert_eq!(h.0[1], Day::Open(vec![(720, 900), (1080, 1350)]));
        assert_eq!(h.0[5], Day::Unknown, "unreadable is not said");
        assert_eq!(h.0[1].label(), "12 pm – 3 pm, 6 pm – 10:30 pm");
        assert_eq!(h.0[4].plain(), "17:00–01:00", "past midnight");
        assert_eq!(h.0[6].label(), "Open 24 hours");
        assert_eq!(Hours::from_json(&h.to_json()), h);
        assert_eq!(Hours::default().to_json(), "");

        let form = form(&[("hours_mon", "open"), ("hours_mon_open", "09:00"), ("hours_mon_close", "17:30"),
            ("hours_tue", "open"), ("hours_tue_open", "25:00"), ("hours_tue_close", "17:00"), ("hours_wed", "closed"), ("hours_thu", "24h")]);
        let h = Hours::from_form(|k| form.get(k).cloned().unwrap_or_default());
        assert_eq!(h.0[0], Day::Open(vec![(540, 1050)]));
        assert_eq!(h.0[1], Day::Unknown, "an impossible time is dropped, not stored");
        assert_eq!((h.0[2].clone(), h.0[3].clone()), (Day::Closed, Day::AllDay));
    }

    #[test]
    fn details_keep_only_known_choices_and_safe_links() {
        let pairs: Vec<(String, String)> = [("amenity", "wifi"), ("amenity", "<script>"), ("amenity", "parking"), ("payment", "cash"),
            ("gallery", "javascript:alert(1)"), ("gallery", "/content/images/a.jpg")]
            .iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let f = form(&[("latitude", "40.71283"), ("longitude", "-74.006"), ("price_level", "9"), ("social_instagram", "https://instagram.com/x")]);
        let d = Details::from_form(&f, &pairs).unwrap();
        assert_eq!(d.amenities, ["wifi", "parking"]);
        assert_eq!(d.payments, ["cash"]);
        assert_eq!(d.gallery, ["/content/images/a.jpg"]);
        assert_eq!(d.price_level, 0, "out of range is not set");
        assert_eq!(d.socials, [("instagram".to_string(), "https://instagram.com/x".to_string())]);
        assert!(Details::from_form(&form(&[("latitude", "123")]), &[]).unwrap_err().contains("latitude"));
        assert!(Details::from_form(&form(&[("social_x", "javascript:x")]), &[]).unwrap_err().contains("X"));
        assert!(Details::from_form(&form(&[("established", "19x8")]), &[]).unwrap_err().contains("year"));
        assert!(Details::from_form(&form(&[("booking_url", "ftp://x")]), &[]).unwrap_err().contains("booking"));
        assert_eq!(host_of("https://www.Example.com/menu?x"), "Example.com");
    }

    #[test]
    fn descriptions_become_escaped_paragraphs() {
        assert_eq!(paragraphs("One\nline two\n\n<script>x</script>"), "<p>One<br>line two</p><p>&lt;script&gt;x&lt;/script&gt;</p>");
    }

    #[test]
    fn expiry_hides_a_classified_and_slugs_are_tidy() {
        assert!(visible("published", None, "2026-11-21 00:00:00"));
        assert!(!visible("published", Some("2026-11-20 00:00:00"), "2026-11-21 00:00:00"));
        assert!(!visible("pending", None, "2026-11-21 00:00:00"));
        assert_eq!(slugify("  Café & Bar — Main St. "), "cafe-bar-main-st");
        assert_eq!(search_terms("  Bike %' \"red\" x "), ["bike", "red"]);
    }
}
