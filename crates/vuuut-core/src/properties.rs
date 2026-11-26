//! Tools → Real Estate & Property Listings: the rules.
//!
//! What Estatik, Easy Property Listings and Essential Real Estate have in
//! common, reduced to the parts that can be wrong: a property is for sale or
//! to rent, at a price (per month, week… for a rent) or "on application", with
//! bedrooms, bathrooms, parking, floor and plot areas, a year built, an
//! energy rating, an address and a pin, features from a fixed list, floor
//! plans, open-house times, a video, a virtual tour and a brochure. It has a
//! market status beside its publishing status — Available, Under offer,
//! Sold, Let — because a sold house usually stays on the site as proof.
//!
//! This module is the trust boundary for all of it. [`PropertyInput`],
//! [`AgentInput`] and [`EnquiryInput`] are rebuilt from a submitted form
//! field by field: unknown keys are dropped, every string is bounded and
//! stripped of control characters, URLs must be http(s) or a path in this
//! site's content folder, numbers are clamped, and the JSON lists (floor
//! plans, open houses) are rebuilt from their own rows rather than stored as
//! sent. [`Filters`] reduces a public search's query string to known values,
//! so nothing a visitor types is ever written into a link or a query
//! unchecked. [`monthly_payment`] is the mortgage calculator's arithmetic,
//! shared with the page that shows its first answer before any script runs.
//!
//! No database and no HTTP here: the admin (`vuuut_admin_ui::properties`),
//! the public pages (`vuuut_front::properties`) and the theme helpers all
//! call these functions, so they cannot disagree.

use crate::directory::{clean, clean_phone, clean_text, is_email, parse_price, safe_image, safe_url, slugify};

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "properties_enabled";
pub const SETTING_TITLE: &str = "properties_title";
pub const SETTING_INTRO: &str = "properties_intro";
pub const SETTING_PER_PAGE: &str = "properties_per_page";
pub const SETTING_AREA_UNIT: &str = "properties_area_unit";
pub const SETTING_ENQUIRIES: &str = "properties_enquiries";
pub const SETTING_NOTIFY_EMAIL: &str = "properties_notify_email";
pub const SETTING_SHOW_AGENT_CONTACT: &str = "properties_show_agent_contact";
pub const SETTING_MORTGAGE: &str = "properties_mortgage";
pub const SETTING_RATE: &str = "properties_mortgage_rate";
pub const SETTING_TERM: &str = "properties_mortgage_term";
pub const SETTING_DEPOSIT: &str = "properties_mortgage_deposit";
pub const SETTING_SHOW_SOLD: &str = "properties_show_sold";
pub const SETTING_BASE_PATH: &str = "properties_base_path";

pub const DEFAULT_TITLE: &str = "Properties";
pub const DEFAULT_BASE: &str = "properties";
pub const DEFAULT_PER_PAGE: i64 = 12;
/// The catalogue's own addresses; never a property's slug.
pub const RESERVED_SLUGS: &[&str] = &["assets", "type", "agent", "agents", "saved", "compare", "search", "map", "city", "enquire"];

pub const MAX_TITLE: usize = 160;
pub const MAX_SUMMARY: usize = 300;
pub const MAX_PLAIN: usize = 20_000;
pub const MAX_RICH: usize = 200_000;
pub const MAX_IMAGES: usize = 40;
pub const MAX_PRICE_CENTS: i64 = 100_000_000_000_00;
/// A person cannot fill the enquiry form faster than this.
pub const MIN_FILL_MS: i64 = 3_000;

/// Where the catalogue lives on the site; `properties` unless changed.
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
        Self::new(get(SETTING_BASE_PATH).unwrap_or_default().trim_matches('"')).unwrap_or_default()
    }

    /// One to three lowercase segments, outside the other tools' roots.
    pub fn new(base: &str) -> Option<Self> {
        if base.trim().trim_matches('/').is_empty() {
            return Some(Self::default());
        }
        let clean = crate::ecom::clean_address(base)?;
        let first = clean.split('/').next().unwrap_or_default();
        if matches!(first, "shop" | "directory" | "courses" | "book") {
            return None;
        }
        Some(Paths { base: clean })
    }

    /// `/properties/x/` → the address the visitor sees.
    pub fn public_path(&self, internal: &str) -> String {
        if self.base == DEFAULT_BASE {
            return internal.into();
        }
        match internal.strip_prefix("/properties/") {
            Some(tail) => format!("/{}/{tail}", self.base),
            None if internal == "/properties" => format!("/{}", self.base),
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
            return Some("/properties".into());
        }
        rest.strip_prefix('/').map(|tail| format!("/properties/{tail}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaUnit {
    SqFt,
    SqM,
}

impl AreaUnit {
    pub fn key(self) -> &'static str {
        match self {
            AreaUnit::SqFt => "sqft",
            AreaUnit::SqM => "m2",
        }
    }
    pub fn from_key(k: &str) -> AreaUnit {
        if k.trim().trim_matches('"') == "sqft" { AreaUnit::SqFt } else { AreaUnit::SqM }
    }
    pub fn label(self) -> &'static str {
        match self {
            AreaUnit::SqFt => "sq ft",
            AreaUnit::SqM => "m²",
        }
    }
    /// The schema.org / UN/CEFACT unit code.
    pub fn code(self) -> &'static str {
        match self {
            AreaUnit::SqFt => "FTK",
            AreaUnit::SqM => "MTK",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub title: String,
    pub intro: String,
    pub per_page: i64,
    pub currency: String,
    pub area_unit: AreaUnit,
    /// The contact-the-agent and request-a-viewing forms.
    pub enquiries: bool,
    /// Where enquiry notices go besides the property's agent.
    pub notify_email: String,
    /// Agents' email addresses and phone numbers on public pages.
    pub show_agent_contact: bool,
    /// The mortgage calculator on properties for sale.
    pub mortgage: bool,
    /// The calculator's starting interest rate, in hundredths of a percent.
    pub rate_bp: i64,
    pub term_years: i64,
    pub deposit_pct: i64,
    /// Sold and let properties stay in the catalogue, marked.
    pub show_sold: bool,
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
        let num = |k: &str, d: i64, lo: i64, hi: i64| raw(k).and_then(|v| v.parse::<i64>().ok()).map(|n| n.clamp(lo, hi)).unwrap_or(d);
        let title = clean(&raw(SETTING_TITLE).unwrap_or_default(), 60);
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            title: if title.is_empty() { DEFAULT_TITLE.into() } else { title },
            intro: clean(&raw(SETTING_INTRO).unwrap_or_default(), 400),
            per_page: num(SETTING_PER_PAGE, DEFAULT_PER_PAGE, 1, 96),
            currency: crate::currency::preferred(&get),
            area_unit: AreaUnit::from_key(&raw(SETTING_AREA_UNIT).unwrap_or_else(|| "sqft".into())),
            enquiries: flag(SETTING_ENQUIRIES, true),
            notify_email: raw(SETTING_NOTIFY_EMAIL).filter(|e| is_email(e)).unwrap_or_default(),
            show_agent_contact: flag(SETTING_SHOW_AGENT_CONTACT, true),
            mortgage: flag(SETTING_MORTGAGE, true),
            rate_bp: raw(SETTING_RATE).and_then(|v| parse_rate(&v)).unwrap_or(650),
            term_years: num(SETTING_TERM, 25, 1, 50),
            deposit_pct: num(SETTING_DEPOSIT, 20, 0, 100),
            show_sold: flag(SETTING_SHOW_SOLD, true),
            paths: Paths::read(&get),
        }
    }
}

/// `"6.5"`, `"6.25%"` → 650, 625 hundredths of a percent; 0 … 30%.
pub fn parse_rate(raw: &str) -> Option<i64> {
    let v = raw.trim().trim_end_matches('%').trim();
    let (whole, frac) = v.split_once('.').unwrap_or((v, ""));
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) || frac.len() > 2 || whole.len() > 3 {
        return None;
    }
    let bp = whole.parse::<i64>().ok()? * 100 + if frac.is_empty() { 0 } else { format!("{frac:0<2}").parse::<i64>().ok()? };
    (0..=3000).contains(&bp).then_some(bp)
}

pub fn rate_label(bp: i64) -> String {
    if bp % 100 == 0 { format!("{}", bp / 100) } else if bp % 10 == 0 { format!("{}.{}", bp / 100, (bp % 100) / 10) } else { format!("{}.{:02}", bp / 100, bp % 100) }
}

// ---------------------------------------------------------------------------
// the kinds of thing a property is
// ---------------------------------------------------------------------------

/// For sale or to rent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Sale,
    Rent,
}

impl Purpose {
    pub const ALL: [Purpose; 2] = [Purpose::Sale, Purpose::Rent];
    pub fn key(self) -> &'static str {
        match self {
            Purpose::Sale => "sale",
            Purpose::Rent => "rent",
        }
    }
    pub fn from_key(k: &str) -> Purpose {
        if k.trim() == "rent" { Purpose::Rent } else { Purpose::Sale }
    }
    pub fn label(self) -> &'static str {
        match self {
            Purpose::Sale => "For sale",
            Purpose::Rent => "To rent",
        }
    }
}

/// Where the property is in the market — separate from whether the listing
/// is published, because a sold house usually stays on the site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    Available,
    UnderOffer,
    Sold,
    Let,
}

impl Market {
    pub const ALL: [Market; 4] = [Market::Available, Market::UnderOffer, Market::Sold, Market::Let];
    pub fn key(self) -> &'static str {
        match self {
            Market::Available => "available",
            Market::UnderOffer => "under_offer",
            Market::Sold => "sold",
            Market::Let => "let",
        }
    }
    pub fn from_key(k: &str) -> Market {
        Market::ALL.into_iter().find(|m| m.key() == k.trim()).unwrap_or(Market::Available)
    }
    pub fn label(self) -> &'static str {
        match self {
            Market::Available => "Available",
            Market::UnderOffer => "Under offer",
            Market::Sold => "Sold",
            Market::Let => "Let",
        }
    }
    /// The word a card's ribbon shows, which depends on the purpose.
    pub fn badge(self, purpose: Purpose) -> &'static str {
        match (self, purpose) {
            (Market::Available, Purpose::Sale) => "For sale",
            (Market::Available, Purpose::Rent) => "To rent",
            (Market::UnderOffer, Purpose::Sale) => "Under offer",
            (Market::UnderOffer, Purpose::Rent) => "Let agreed",
            (Market::Sold, _) => "Sold",
            (Market::Let, _) => "Let",
        }
    }
    /// No longer on the market.
    pub fn closed(self) -> bool {
        matches!(self, Market::Sold | Market::Let)
    }
    /// The schema.org availability of the offer.
    pub fn availability(self) -> &'static str {
        match self {
            Market::Available => "https://schema.org/InStock",
            Market::UnderOffer => "https://schema.org/LimitedAvailability",
            Market::Sold | Market::Let => "https://schema.org/SoldOut",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Draft,
    Published,
    Archived,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Draft, Status::Published, Status::Archived];
    pub fn key(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Published => "published",
            Status::Archived => "archived",
        }
    }
    pub fn from_key(k: &str) -> Status {
        Status::ALL.into_iter().find(|s| s.key() == k.trim()).unwrap_or(Status::Draft)
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Draft => "Draft — only staff can see it",
            Status::Published => "Published — on the site",
            Status::Archived => "Archived — off the site, kept here",
        }
    }
}

/// A rent's period. A sale has none.
pub const RENT_PERIODS: &[(&str, &str, &str)] = &[
    ("month", "per month", "/mo"),
    ("week", "per week", "/wk"),
    ("year", "per year", "/yr"),
    ("night", "per night", "/night"),
];

pub fn period_suffix(key: &str) -> &'static str {
    RENT_PERIODS.iter().find(|p| p.0 == key).map(|p| p.2).unwrap_or("/mo")
}

/// Energy ratings, as EU and UK certificates print them.
pub const ENERGY_RATINGS: &[&str] = &["A+", "A", "B", "C", "D", "E", "F", "G"];

/// Features: the checklist every listing plugin ships, grouped as visitors
/// filter them. Keys are what is stored; labels are what is shown.
pub const FEATURES: &[(&str, &str, &str)] = &[
    ("air_conditioning", "Air conditioning", "Inside"),
    ("central_heating", "Central heating", "Inside"),
    ("fireplace", "Fireplace", "Inside"),
    ("furnished", "Furnished", "Inside"),
    ("hardwood_floors", "Hardwood floors", "Inside"),
    ("open_plan", "Open-plan kitchen", "Inside"),
    ("dishwasher", "Dishwasher", "Inside"),
    ("laundry", "Laundry room", "Inside"),
    ("walk_in_closet", "Walk-in wardrobe", "Inside"),
    ("home_office", "Home office", "Inside"),
    ("storage", "Storage", "Inside"),
    ("elevator", "Lift / elevator", "Building"),
    ("concierge", "Concierge", "Building"),
    ("gym", "Gym", "Building"),
    ("security", "Security / alarm", "Building"),
    ("wheelchair", "Step-free access", "Building"),
    ("pets", "Pets allowed", "Building"),
    ("smart_home", "Smart home", "Building"),
    ("solar", "Solar panels", "Building"),
    ("ev_charging", "EV charging", "Building"),
    ("garden", "Garden", "Outside"),
    ("balcony", "Balcony", "Outside"),
    ("terrace", "Roof terrace", "Outside"),
    ("pool", "Swimming pool", "Outside"),
    ("garage", "Garage", "Outside"),
    ("driveway", "Driveway", "Outside"),
    ("bbq", "Barbecue area", "Outside"),
    ("sea_view", "Sea view", "Views"),
    ("city_view", "City view", "Views"),
    ("waterfront", "Waterfront", "Views"),
    ("mountain_view", "Mountain view", "Views"),
];

pub fn feature_label(key: &str) -> Option<&'static str> {
    FEATURES.iter().find(|f| f.0 == key).map(|f| f.1)
}

/// `"pool,garden, nonsense"` → the known keys, in the list's order.
pub fn feature_keys(stored: &str) -> Vec<&'static str> {
    let wanted: Vec<&str> = stored.split(',').map(str::trim).collect();
    FEATURES.iter().filter(|f| wanted.contains(&f.0)).map(|f| f.0).collect()
}

// ---------------------------------------------------------------------------
// money and sizes
// ---------------------------------------------------------------------------

/// Thousands separated; hundredths only when there are any.
pub fn amount(cents: i64, currency: &str) -> String {
    let symbol = crate::ecom::symbol(currency);
    let whole = (cents / 100).to_string();
    let mut digits = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            digits.push(',');
        }
        digits.push(c);
    }
    let frac = cents % 100;
    let n = if frac == 0 { digits } else { format!("{digits}.{frac:02}") };
    if symbol.is_empty() { format!("{n} {currency}") } else { format!("{symbol}{n}") }
}

/// The price line a card shows: `$450,000`, `Offers over $450,000`,
/// `$2,400 /mo`, or `Price on application`.
pub fn price_label(cents: i64, currency: &str, purpose: Purpose, period: &str, poa: bool, prefix: &str) -> String {
    if poa || cents <= 0 {
        return "Price on application".into();
    }
    let mut out = amount(cents, currency);
    if !prefix.is_empty() {
        out = format!("{prefix} {out}");
    }
    if purpose == Purpose::Rent {
        out.push(' ');
        out.push_str(period_suffix(period));
    }
    out
}

/// `1850` in square feet → `1,850 sq ft`.
pub fn area_label(n: i64, unit: AreaUnit) -> String {
    if n <= 0 {
        return String::new();
    }
    let s = n.to_string();
    let mut digits = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            digits.push(',');
        }
        digits.push(c);
    }
    format!("{digits} {}", unit.label())
}

/// The repayment on a loan of `principal` (any minor unit) at `rate_bp`
/// hundredths of a percent a year over `years`, monthly, in the same unit.
/// The standard annuity formula; a 0% loan is the principal split evenly.
pub fn monthly_payment(principal: i64, rate_bp: i64, years: i64) -> i64 {
    if principal <= 0 || years <= 0 {
        return 0;
    }
    let n = (years * 12) as f64;
    let r = rate_bp as f64 / 10_000.0 / 12.0;
    let p = principal as f64;
    let m = if r == 0.0 { p / n } else { p * r / (1.0 - (1.0 + r).powf(-n)) };
    m.round() as i64
}

// ---------------------------------------------------------------------------
// what a save writes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct FloorPlan {
    pub title: String,
    pub image: String,
    pub size: String,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct OpenHouse {
    /// `YYYY-MM-DD`
    pub date: String,
    /// `HH:MM`
    pub start: String,
    pub end: String,
}

pub fn floor_plans_from_json(raw: &str) -> Vec<FloorPlan> {
    serde_json::from_str::<Vec<FloorPlan>>(raw).unwrap_or_default().into_iter().filter(|p| safe_image(&p.image).is_some_and(|i| !i.is_empty())).take(20).collect()
}

pub fn open_houses_from_json(raw: &str) -> Vec<OpenHouse> {
    serde_json::from_str::<Vec<OpenHouse>>(raw).unwrap_or_default().into_iter().filter(|o| valid_date(&o.date)).take(20).collect()
}

/// `YYYY-MM-DD` that is a real day.
pub fn valid_date(d: &str) -> bool {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok()
}

fn valid_time(t: &str) -> bool {
    chrono::NaiveTime::parse_from_str(t, "%H:%M").is_ok()
}

/// A virtual tour that may be shown in a frame: Matterport, Kuula,
/// Momento360, CloudPano, a YouTube 360 video. Anything else is a link.
pub fn tour_embed(url: &str) -> Option<String> {
    let u = safe_url(url)?;
    let lower = u.to_ascii_lowercase();
    let host = lower.trim_start_matches("https://").trim_start_matches("http://").split(['/', '?', '#']).next().unwrap_or("").to_string();
    let ok = ["my.matterport.com", "kuula.co", "momento360.com", "app.cloudpano.com", "tour.giraffe360.com"].iter().any(|h| host == *h || host.ends_with(&format!(".{h}")));
    if ok && lower.starts_with("https://") {
        return Some(u);
    }
    crate::lms::video_embed(&u).filter(|(_, frame)| *frame).map(|(e, _)| e)
}

/// The property form, rebuilt. `known(kind, id)` answers whether an id the
/// form names (`type`, `agent`) exists.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PropertyInput {
    pub title: String,
    pub slug: String,
    pub status: String,
    pub purpose: String,
    pub market: String,
    pub type_id: Option<String>,
    pub agent_id: Option<String>,
    pub reference: String,
    pub price_cents: i64,
    pub price_poa: bool,
    pub price_prefix: String,
    pub rent_period: String,
    pub deposit_cents: i64,
    pub service_charge_cents: i64,
    pub tax_cents: i64,
    pub summary: String,
    pub description: String,
    pub description_lexical: String,
    pub description_html: String,
    pub image: String,
    pub gallery: Vec<String>,
    pub bedrooms: i64,
    pub bathrooms: i64,
    pub half_baths: i64,
    pub rooms: i64,
    pub parking: i64,
    pub area: i64,
    pub lot_area: i64,
    pub year_built: i64,
    pub floors: i64,
    pub energy_rating: String,
    pub available_from: String,
    pub address: String,
    pub neighborhood: String,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    pub country: String,
    pub latitude: String,
    pub longitude: String,
    pub hide_address: bool,
    pub features: Vec<String>,
    pub extra_features: String,
    pub floor_plans: Vec<FloorPlan>,
    pub open_houses: Vec<OpenHouse>,
    pub video_url: String,
    pub tour_url: String,
    pub brochure_url: String,
    pub featured: bool,
    pub keywords: String,
    pub seo_title: String,
    pub seo_description: String,
}

fn coordinate(raw: &str, limit: f64) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() {
        return Some(String::new());
    }
    let n: f64 = v.parse().ok()?;
    (n.is_finite() && n.abs() <= limit).then(|| format!("{:.6}", n).trim_end_matches('0').trim_end_matches('.').to_string())
}

impl PropertyInput {
    pub fn from_form(pairs: &[(String, String)], known: impl Fn(&str, &str) -> bool) -> Result<PropertyInput, String> {
        let get = |k: &str| pairs.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str()).unwrap_or("");
        let all = |k: &str| pairs.iter().filter(|(x, _)| x == k).map(|(_, v)| v.as_str()).collect::<Vec<_>>();
        let on = |k: &str| matches!(get(k), "1" | "on" | "true");
        let count = |k: &str, hi: i64| get(k).trim().parse::<i64>().ok().map(|n| n.clamp(0, hi)).unwrap_or(0);

        let title = clean(get("title"), MAX_TITLE);
        if title.chars().count() < 3 {
            return Err("Give the property a title of at least three characters.".into());
        }
        let slug = {
            let s = slugify(get("slug"));
            if s.is_empty() { slugify(&title) } else { s }
        };
        let purpose = Purpose::from_key(get("purpose"));
        let price = |k: &str, what: &str| -> Result<i64, String> {
            let c = parse_price(get(k)).ok_or_else(|| format!("The {what} is not a number."))?;
            if c > MAX_PRICE_CENTS { Err(format!("The {what} is too large.")) } else { Ok(c) }
        };
        let id_of = |k: &str, kind: &str| Some(get(k).trim().to_string()).filter(|v| !v.is_empty() && crate::ids::is_objectid(v) && known(kind, v));

        let mut images: Vec<String> = Vec::new();
        for raw in all("image") {
            match safe_image(raw) {
                Some(u) if !u.is_empty() && !images.contains(&u) => images.push(u),
                Some(_) => {}
                None => return Err("A photo address must start with https:// or /content/images/.".into()),
            }
        }
        images.truncate(MAX_IMAGES);
        let image = if images.is_empty() { String::new() } else { images.remove(0) };

        let url = |k: &str, what: &str| safe_url(get(k)).ok_or_else(|| format!("The {what} must be an https:// address."));
        let video_url = url("video_url", "video")?;
        if !video_url.is_empty() && crate::lms::video_embed(&video_url).is_none() {
            return Err("The video must be a YouTube, Vimeo, Loom or Wistia link, or an .mp4 file.".into());
        }
        let tour_url = url("tour_url", "virtual tour")?;
        let brochure_url = {
            let v = clean(get("brochure_url"), 500);
            if v.starts_with("/content/") && !v.contains("..") && !v.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>')) {
                v
            } else {
                url("brochure_url", "brochure")?
            }
        };

        // Floor plans: rows of fp_title / fp_image / fp_size, kept in order,
        // dropped when they have no picture.
        let (fp_t, fp_i, fp_s) = (all("fp_title"), all("fp_image"), all("fp_size"));
        let mut floor_plans = Vec::new();
        for (i, img) in fp_i.iter().enumerate() {
            let Some(img) = safe_image(img) else { return Err("A floor plan picture must be an https:// or /content/images/ address.".into()) };
            if img.is_empty() {
                continue;
            }
            floor_plans.push(FloorPlan {
                title: clean(fp_t.get(i).copied().unwrap_or(""), 80),
                image: img,
                size: clean(fp_s.get(i).copied().unwrap_or(""), 40),
            });
        }
        floor_plans.truncate(20);

        // Open houses: oh_date / oh_start / oh_end; a row without a valid
        // date is an empty row, not an error.
        let (oh_d, oh_s, oh_e) = (all("oh_date"), all("oh_start"), all("oh_end"));
        let mut open_houses = Vec::new();
        for (i, d) in oh_d.iter().enumerate() {
            let d = d.trim();
            if d.is_empty() {
                continue;
            }
            if !valid_date(d) {
                return Err("An open-house date is not a date.".into());
            }
            let start = oh_s.get(i).map(|t| t.trim()).filter(|t| valid_time(t)).unwrap_or("").to_string();
            let end = oh_e.get(i).map(|t| t.trim()).filter(|t| valid_time(t)).unwrap_or("").to_string();
            if !start.is_empty() && !end.is_empty() && end <= start {
                return Err("An open house must end after it starts.".into());
            }
            open_houses.push(OpenHouse { date: d.into(), start, end });
        }
        open_houses.sort_by(|a, b| (&a.date, &a.start).cmp(&(&b.date, &b.start)));
        open_houses.truncate(20);

        let wanted: Vec<&str> = all("feature");
        let features: Vec<String> = FEATURES.iter().filter(|f| wanted.contains(&f.0)).map(|f| f.0.to_string()).collect();

        let energy = get("energy_rating").trim();
        let available_from = get("available_from").trim();
        let year = count("year_built", 2100);
        let phone_free = |s: &str| clean(s, 120);
        Ok(PropertyInput {
            title,
            slug,
            status: Status::from_key(get("status")).key().into(),
            purpose: purpose.key().into(),
            market: Market::from_key(get("market")).key().into(),
            type_id: id_of("type_id", "type"),
            agent_id: id_of("agent_id", "agent"),
            reference: clean(get("reference"), 40),
            price_cents: price("price", "price")?,
            price_poa: on("price_poa"),
            price_prefix: match get("price_prefix").trim() {
                p @ ("Offers over" | "Offers in the region of" | "Guide price" | "From" | "Fixed price" | "Starting at") => p.to_string(),
                _ => String::new(),
            },
            rent_period: if purpose == Purpose::Rent { RENT_PERIODS.iter().find(|p| p.0 == get("rent_period").trim()).map(|p| p.0).unwrap_or("month").to_string() } else { String::new() },
            deposit_cents: if purpose == Purpose::Rent { price("deposit", "deposit")? } else { 0 },
            service_charge_cents: price("service_charge", "service charge")?,
            tax_cents: price("tax", "property tax")?,
            summary: clean(get("summary"), MAX_SUMMARY),
            description: clean_text(get("description"), MAX_PLAIN),
            description_lexical: String::new(),
            description_html: String::new(),
            image,
            gallery: images,
            bedrooms: count("bedrooms", 99),
            bathrooms: count("bathrooms", 99),
            half_baths: count("half_baths", 20),
            rooms: count("rooms", 999),
            parking: count("parking", 999),
            area: count("area", 100_000_000),
            lot_area: count("lot_area", 1_000_000_000),
            year_built: if (1000..=2100).contains(&year) { year } else { 0 },
            floors: count("floors", 200),
            energy_rating: ENERGY_RATINGS.iter().find(|r| **r == energy).map(|r| r.to_string()).unwrap_or_default(),
            available_from: if valid_date(available_from) { available_from.into() } else { String::new() },
            address: phone_free(get("address")),
            neighborhood: clean(get("neighborhood"), 80),
            city: clean(get("city"), 80),
            region: clean(get("region"), 80),
            postal_code: clean(get("postal_code"), 20),
            country: clean(get("country"), 60),
            latitude: coordinate(get("latitude"), 90.0).ok_or("The latitude must be a number between -90 and 90.")?,
            longitude: coordinate(get("longitude"), 180.0).ok_or("The longitude must be a number between -180 and 180.")?,
            hide_address: on("hide_address"),
            features,
            extra_features: clean_text(get("extra_features"), 2000).lines().map(|l| clean(l, 80)).filter(|l| !l.is_empty()).take(30).collect::<Vec<_>>().join("\n"),
            floor_plans,
            open_houses,
            video_url,
            tour_url,
            brochure_url,
            featured: on("featured"),
            keywords: clean(get("keywords"), 200),
            seo_title: clean(get("seo_title"), 70),
            seo_description: clean(get("seo_description"), 320),
        })
    }
}

/// An agent (or a branch, or a landlord) a property is listed by.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentInput {
    pub name: String,
    pub slug: String,
    pub title: String,
    pub agency: String,
    pub email: String,
    pub phone: String,
    pub mobile: String,
    pub photo: String,
    pub bio: String,
    pub website: String,
    pub license: String,
    pub languages: String,
    pub status: String,
}

impl AgentInput {
    pub fn from_form(pairs: &[(String, String)]) -> Result<AgentInput, String> {
        let get = |k: &str| pairs.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str()).unwrap_or("");
        let name = clean(get("name"), 120);
        if name.chars().count() < 2 {
            return Err("Give the agent a name.".into());
        }
        let email = clean(get("email"), 191);
        if !email.is_empty() && !is_email(&email) {
            return Err("That email address does not look right.".into());
        }
        let phone = clean_phone(get("phone")).ok_or("That phone number does not look right.")?;
        let mobile = clean_phone(get("mobile")).ok_or("That mobile number does not look right.")?;
        let photo = safe_image(get("photo")).ok_or("The photo must be an https:// or /content/images/ address.")?;
        let website = safe_url(get("website")).ok_or("The website must be an https:// address.")?;
        let slug = {
            let s = slugify(get("slug"));
            if s.is_empty() { slugify(&name) } else { s }
        };
        Ok(AgentInput {
            name,
            slug,
            title: clean(get("title"), 120),
            agency: clean(get("agency"), 120),
            email,
            phone,
            mobile,
            photo,
            bio: clean_text(get("bio"), 4000),
            website,
            license: clean(get("license"), 60),
            languages: clean(get("languages"), 120),
            status: if get("status") == "hidden" { "hidden".into() } else { "active".into() },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnquiryKind {
    Info,
    Viewing,
}

impl EnquiryKind {
    pub fn key(self) -> &'static str {
        match self {
            EnquiryKind::Info => "info",
            EnquiryKind::Viewing => "viewing",
        }
    }
    pub fn from_key(k: &str) -> EnquiryKind {
        if k.trim() == "viewing" { EnquiryKind::Viewing } else { EnquiryKind::Info }
    }
    pub fn label(self) -> &'static str {
        match self {
            EnquiryKind::Info => "Question",
            EnquiryKind::Viewing => "Viewing request",
        }
    }
}

pub const ENQUIRY_STATUSES: &[(&str, &str)] = &[("new", "New"), ("contacted", "Contacted"), ("viewing", "Viewing booked"), ("closed", "Closed"), ("spam", "Spam")];

pub fn enquiry_status(k: &str) -> &'static str {
    ENQUIRY_STATUSES.iter().find(|s| s.0 == k.trim()).map(|s| s.0).unwrap_or("new")
}

/// What a visitor sends from a property page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnquiryInput {
    pub kind: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub message: String,
    /// `YYYY-MM-DD` for a viewing request.
    pub preferred_date: String,
    /// morning | afternoon | evening | any
    pub preferred_time: String,
}

impl EnquiryInput {
    pub fn from_form(get: impl Fn(&str) -> Option<String>, today: &str) -> Result<EnquiryInput, String> {
        let g = |k: &str| get(k).unwrap_or_default();
        let kind = EnquiryKind::from_key(&g("kind"));
        let name = clean(&g("name"), 120);
        if name.chars().count() < 2 {
            return Err("Please tell us your name.".into());
        }
        let email = clean(&g("email"), 191);
        if !is_email(&email) {
            return Err("Please give an email address we can reply to.".into());
        }
        let phone = clean_phone(&g("phone")).ok_or("That phone number does not look right.")?;
        let message = clean_text(&g("message"), 4000);
        let date = g("preferred_date").trim().to_string();
        let preferred_date = if kind == EnquiryKind::Viewing && valid_date(&date) {
            if date.as_str() < today {
                return Err("Choose a viewing date from today on.".into());
            }
            date
        } else {
            String::new()
        };
        if kind == EnquiryKind::Info && message.chars().count() < 5 {
            return Err("Please write a short message.".into());
        }
        Ok(EnquiryInput {
            kind: kind.key().into(),
            name,
            email,
            phone,
            message,
            preferred_date,
            preferred_time: match g("preferred_time").trim() {
                t @ ("morning" | "afternoon" | "evening") => t.into(),
                _ => if kind == EnquiryKind::Viewing { "any".into() } else { String::new() },
            },
        })
    }
}

// ---------------------------------------------------------------------------
// public search
// ---------------------------------------------------------------------------

pub const SORTS: &[(&str, &str)] = &[("", "Featured first"), ("newest", "Newest"), ("price_asc", "Price: low to high"), ("price_desc", "Price: high to low"), ("area_desc", "Largest first"), ("beds_desc", "Most bedrooms")];

/// A public search, reduced to values the catalogue knows. Each field is
/// checked here, so a link rebuilt from it carries nothing a visitor typed
/// except the free-text query, which is percent-encoded where it is used.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Filters {
    pub q: String,
    pub purpose: Option<&'static str>,
    /// A type's slug; checked against the stored types by the caller.
    pub type_slug: String,
    /// A city the catalogue has; checked by the caller.
    pub city: String,
    pub min_price: i64,
    pub max_price: i64,
    pub beds: i64,
    pub baths: i64,
    pub min_area: i64,
    pub features: Vec<&'static str>,
    pub sort: &'static str,
    pub page: i64,
}

impl Filters {
    pub fn from_query(get: impl Fn(&str) -> Option<String>) -> Filters {
        let money = |k: &str| get(k).and_then(|v| parse_price(&v)).filter(|c| *c > 0 && *c <= MAX_PRICE_CENTS).unwrap_or(0);
        let small = |k: &str, hi: i64| get(k).and_then(|v| v.trim().trim_end_matches('+').parse::<i64>().ok()).map(|n| n.clamp(0, hi)).unwrap_or(0);
        let feats = get("features").unwrap_or_default();
        Filters {
            q: get("q").map(|v| clean(&v, 100)).unwrap_or_default(),
            purpose: match get("purpose").as_deref().map(str::trim) {
                Some("sale") | Some("buy") => Some("sale"),
                Some("rent") => Some("rent"),
                _ => None,
            },
            type_slug: get("type").map(|v| slugify(&v)).unwrap_or_default(),
            city: get("city").map(|v| clean(&v, 80)).unwrap_or_default(),
            min_price: money("min_price"),
            max_price: money("max_price"),
            beds: small("beds", 10),
            baths: small("baths", 10),
            min_area: small("min_area", 100_000_000),
            features: feature_keys(&feats.replace(' ', ",")),
            sort: SORTS.iter().find(|s| Some(s.0) == get("sort").as_deref().map(str::trim)).map(|s| s.0).unwrap_or(""),
            page: get("page").and_then(|p| p.parse().ok()).unwrap_or(1).clamp(1, 10_000),
        }
    }

    /// The query string for this search on another page (without `page`).
    /// `city` and `type_slug` must already have been checked by the caller.
    pub fn query_string(&self, page: i64) -> String {
        let mut p: Vec<String> = Vec::new();
        let enc = crate::social_media::encode;
        if !self.q.is_empty() {
            p.push(format!("q={}", enc(&self.q)));
        }
        if let Some(x) = self.purpose {
            p.push(format!("purpose={x}"));
        }
        if !self.type_slug.is_empty() {
            p.push(format!("type={}", enc(&self.type_slug)));
        }
        if !self.city.is_empty() {
            p.push(format!("city={}", enc(&self.city)));
        }
        let whole = |c: i64| (c / 100).to_string();
        if self.min_price > 0 {
            p.push(format!("min_price={}", whole(self.min_price)));
        }
        if self.max_price > 0 {
            p.push(format!("max_price={}", whole(self.max_price)));
        }
        if self.beds > 0 {
            p.push(format!("beds={}", self.beds));
        }
        if self.baths > 0 {
            p.push(format!("baths={}", self.baths));
        }
        if self.min_area > 0 {
            p.push(format!("min_area={}", self.min_area));
        }
        if !self.features.is_empty() {
            p.push(format!("features={}", self.features.join(",")));
        }
        if !self.sort.is_empty() {
            p.push(format!("sort={}", self.sort));
        }
        if page > 1 {
            p.push(format!("page={page}"));
        }
        p.join("&")
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty() && self.purpose.is_none() && self.type_slug.is_empty() && self.city.is_empty() && self.min_price == 0 && self.max_price == 0 && self.beds == 0 && self.baths == 0 && self.min_area == 0 && self.features.is_empty()
    }
}

/// Up to `max` distinct object ids from `a,b,c` — the saved and compare lists.
pub fn id_list(raw: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in raw.split(',').map(str::trim) {
        if crate::ids::is_objectid(id) && !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_property_form_is_rebuilt_not_stored_as_sent() {
        let p = PropertyInput::from_form(
            &pairs(&[
                ("title", "  Light-filled\u{0007} family home "),
                ("purpose", "rent"),
                ("rent_period", "fortnight"),
                ("price", "2,400"),
                ("market", "nonsense"),
                ("status", "published"),
                ("bedrooms", "400"),
                ("feature", "pool"),
                ("feature", "<script>"),
                ("feature", "garden"),
                ("energy_rating", "Z"),
                ("image", "/content/images/a.jpg"),
                ("image", "https://x.test/b.jpg"),
                ("image", "/content/images/a.jpg"),
                ("type_id", "000000000000000000000001"),
                ("agent_id", "not-an-id"),
                ("fp_title", "Ground"),
                ("fp_image", ""),
                ("fp_title", "First"),
                ("fp_image", "/content/images/plan.png"),
                ("oh_date", "2026-12-05"),
                ("oh_start", "10:00"),
                ("oh_end", "11:30"),
                ("oh_date", ""),
                ("latitude", "51.5"),
            ]),
            |kind, _| kind == "type",
        )
        .unwrap();
        assert_eq!(p.title, "Light-filled family home");
        assert_eq!(p.slug, "light-filled-family-home");
        assert_eq!((p.purpose.as_str(), p.rent_period.as_str()), ("rent", "month"));
        assert_eq!(p.price_cents, 240_000);
        assert_eq!(p.market, "available");
        assert_eq!(p.bedrooms, 99);
        assert_eq!(p.features, vec!["garden", "pool"]);
        assert_eq!(p.energy_rating, "");
        assert_eq!(p.image, "/content/images/a.jpg");
        assert_eq!(p.gallery, vec!["https://x.test/b.jpg"]);
        assert_eq!(p.type_id.as_deref(), Some("000000000000000000000001"));
        assert_eq!(p.agent_id, None);
        assert_eq!(p.floor_plans.len(), 1);
        assert_eq!(p.floor_plans[0].title, "First");
        assert_eq!(p.open_houses, vec![OpenHouse { date: "2026-12-05".into(), start: "10:00".into(), end: "11:30".into() }]);
        assert_eq!(p.latitude, "51.5");
    }

    #[test]
    fn hostile_urls_and_bad_numbers_are_refused() {
        let base = [("title", "A home")];
        let with = |extra: (&str, &str)| {
            let mut v = pairs(&base);
            v.push((extra.0.into(), extra.1.into()));
            PropertyInput::from_form(&v, |_, _| true)
        };
        assert!(with(("image", "javascript:alert(1)")).is_err());
        assert!(with(("video_url", "https://evil.test/x")).is_err());
        assert!(with(("tour_url", "javascript:alert(1)")).is_err());
        assert!(with(("brochure_url", "/content/../etc/passwd")).is_err());
        assert!(with(("price", "1e9")).is_err());
        assert!(with(("latitude", "91")).is_err());
        assert!(with(("oh_date", "2026-02-30")).is_err());
        assert!(PropertyInput::from_form(&pairs(&[("title", "ab")]), |_, _| true).is_err());
        assert!(with(("brochure_url", "/content/files/brochure.pdf")).is_ok());
    }

    #[test]
    fn prices_read_the_way_agents_write_them() {
        assert_eq!(price_label(45_000_000, "USD", Purpose::Sale, "", false, ""), "$450,000");
        assert_eq!(price_label(45_000_000, "GBP", Purpose::Sale, "", false, "Offers over"), "Offers over £450,000");
        assert_eq!(price_label(240_000, "EUR", Purpose::Rent, "week", false, ""), "€2,400 /wk");
        assert_eq!(price_label(240_000, "USD", Purpose::Sale, "", true, ""), "Price on application");
        assert_eq!(price_label(0, "USD", Purpose::Sale, "", false, ""), "Price on application");
        assert_eq!(area_label(1850, AreaUnit::SqFt), "1,850 sq ft");
        assert_eq!(area_label(0, AreaUnit::SqM), "");
    }

    #[test]
    fn the_mortgage_arithmetic_matches_a_bank_calculator() {
        // 300,000 over 25 years at 6.5%: 2,025.62 a month.
        assert_eq!(monthly_payment(30_000_000, 650, 25), 202_562);
        assert_eq!(monthly_payment(1_200_00, 0, 1), 10_000);
        assert_eq!(monthly_payment(0, 650, 25), 0);
        assert_eq!(parse_rate("6.5%"), Some(650));
        assert_eq!(parse_rate("6.25"), Some(625));
        assert_eq!(parse_rate("31"), None);
        assert_eq!(parse_rate("x"), None);
        assert_eq!(rate_label(650), "6.5");
        assert_eq!(rate_label(625), "6.25");
    }

    #[test]
    fn a_forged_search_collapses_to_known_values() {
        let q = |list: &[(&str, &str)]| {
            let m: std::collections::HashMap<String, String> = list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            Filters::from_query(|k| m.get(k).cloned())
        };
        let f = q(&[("purpose", "<x>"), ("sort", "id; DROP"), ("beds", "3+"), ("features", "pool,<b>,garden"), ("min_price", "abc"), ("max_price", "500000"), ("type", "Town House!")]);
        assert_eq!(f.purpose, None);
        assert_eq!(f.sort, "");
        assert_eq!(f.beds, 3);
        assert_eq!(f.features, vec!["garden", "pool"]);
        assert_eq!((f.min_price, f.max_price), (0, 50_000_000));
        assert_eq!(f.type_slug, "town-house");
        assert_eq!(f.query_string(2), "type=town-house&max_price=500000&beds=3&features=garden,pool&page=2");
        assert!(q(&[]).is_empty());
    }

    #[test]
    fn enquiries_need_a_way_to_reply() {
        let form = |list: &[(&str, &str)]| {
            let m: std::collections::HashMap<String, String> = list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            EnquiryInput::from_form(|k| m.get(k).cloned(), "2026-11-25")
        };
        assert!(form(&[("name", "Ann"), ("email", "nope"), ("message", "Hello there")]).is_err());
        assert!(form(&[("name", "Ann"), ("email", "ann@x.test"), ("message", "Hi")]).is_err());
        assert!(form(&[("name", "Ann"), ("email", "ann@x.test"), ("kind", "viewing"), ("preferred_date", "2026-11-01")]).is_err());
        let v = form(&[("name", "Ann"), ("email", "ann@x.test"), ("kind", "viewing"), ("preferred_date", "2026-12-01"), ("preferred_time", "dawn")]).unwrap();
        assert_eq!((v.kind.as_str(), v.preferred_date.as_str(), v.preferred_time.as_str()), ("viewing", "2026-12-01", "any"));
    }

    #[test]
    fn only_known_tour_hosts_are_framed() {
        assert!(tour_embed("https://my.matterport.com/show/?m=abc").is_some());
        assert!(tour_embed("https://evil.test/my.matterport.com").is_none());
        assert!(tour_embed("https://www.youtube.com/watch?v=dQw4w9WgXcQ").is_some());
        assert!(tour_embed("http://my.matterport.com/show/?m=abc").is_none());
    }

    #[test]
    fn the_catalogue_can_move_but_not_onto_another_tool() {
        assert_eq!(Paths::new("homes").unwrap().public_path("/properties/a/"), "/homes/a/");
        assert_eq!(Paths::new("homes").unwrap().internal_path("/homes/a/").as_deref(), Some("/properties/a/"));
        assert!(Paths::new("shop").is_none());
        assert_eq!(Paths::new("").unwrap(), Paths::default());
        assert_eq!(id_list("000000000000000000000001,x,000000000000000000000001,000000000000000000000002", 4).len(), 2);
    }
}
