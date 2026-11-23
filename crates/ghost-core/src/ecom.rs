//! Tools → E-commerce & Online Stores: the rules.
//!
//! This is the store's trust boundary. Everything that decides what a price
//! is, what a cart costs, whether a discount code applies, which products a
//! smart collection holds, or what a submitted address may contain is here,
//! pure — no database, no HTTP — so the admin, the storefront and the tests
//! all ask the same function and get the same answer.
//!
//! Money is always integer minor units (cents). Nothing here ever holds a
//! float, so a total can never be off by a rounding error that a person
//! would notice on a receipt.
//!
//! Untrusted structures (options, collection rules, addresses, cart lines)
//! are rebuilt from scratch with bounds rather than stored as sent.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const CURRENCIES: &[&str] = &[
    "USD", "EUR", "GBP", "CAD", "AUD", "NZD", "JPY", "INR", "BDT", "SGD", "HKD", "CHF", "SEK", "NOK", "DKK", "PLN", "BRL", "MXN", "ZAR", "AED",
];

/// Currencies with no minor unit: ¥100 is stored as 100, not 10000.
pub fn zero_decimal(currency: &str) -> bool {
    matches!(currency, "JPY")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub store_name: String,
    pub intro: String,
    pub currency: String,
    /// Basis points: 825 is 8.25%.
    pub tax_rate_bp: i64,
    pub tax_included: bool,
    pub tax_shipping: bool,
    pub pay_manual: bool,
    pub pay_bank: bool,
    pub bank_details: String,
    pub pay_stripe: bool,
    pub per_page: i64,
    pub low_stock: i64,
    pub notify_email: String,
    pub send_receipts: bool,
    /// ISO 3166 alpha-2 codes checkout ships to; empty means anywhere.
    pub countries: Vec<String>,
    pub require_phone: bool,
    pub policy_refund: String,
    pub policy_shipping: String,
    /// Where the store lives, without slashes: `shop` → `/shop/`.
    pub base_path: String,
    /// What comes before a product's handle: `shop/products` →
    /// `/shop/products/<handle>/`. May equal `base_path`.
    pub product_path: String,
}

pub const PER_PAGE: &[i64] = &[12, 24, 36, 48];

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |k: &str, d: bool| match get(k).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => d,
        };
        let num = |k: &str| get(k).and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok());
        let text = |k: &str, max: usize| clean(&get(k).unwrap_or_default(), max);
        let currency = get("ecom_currency")
            .map(|c| c.trim().trim_matches('"').to_uppercase())
            .filter(|c| CURRENCIES.contains(&c.as_str()))
            .unwrap_or_else(|| "USD".into());
        Settings {
            enabled: flag("ecom_enabled", false),
            store_name: text("ecom_store_name", 80),
            intro: text("ecom_intro", 400),
            currency,
            tax_rate_bp: num("ecom_tax_rate").unwrap_or(0).clamp(0, 5_000),
            tax_included: flag("ecom_tax_included", false),
            tax_shipping: flag("ecom_tax_shipping", false),
            pay_manual: flag("ecom_pay_manual", true),
            pay_bank: flag("ecom_pay_bank", false),
            bank_details: clean_multiline(&get("ecom_bank_details").unwrap_or_default(), 600),
            pay_stripe: flag("ecom_pay_stripe", false),
            per_page: num("ecom_per_page").filter(|n| PER_PAGE.contains(n)).unwrap_or(24),
            low_stock: num("ecom_low_stock").unwrap_or(5).clamp(0, 10_000),
            notify_email: get("ecom_notify_email").map(|e| e.trim().to_lowercase()).filter(|e| is_email(e)).unwrap_or_default(),
            send_receipts: flag("ecom_send_receipts", true),
            countries: parse_countries(&get("ecom_countries").unwrap_or_default()),
            require_phone: flag("ecom_require_phone", false),
            policy_refund: clean_multiline(&get("ecom_policy_refund").unwrap_or_default(), 4_000),
            policy_shipping: clean_multiline(&get("ecom_policy_shipping").unwrap_or_default(), 4_000),
            base_path: get("ecom_base_path").and_then(|p| clean_path(&p)).unwrap_or_else(|| DEFAULT_BASE.into()),
            product_path: get("ecom_product_path").and_then(|p| clean_path(&p)).unwrap_or_else(|| DEFAULT_PRODUCT_PATH.into()),
        }
    }

    /// The store's public name: its own, else the site's title.
    pub fn name<'a>(&'a self, site_title: &'a str) -> &'a str {
        if self.store_name.is_empty() { site_title } else { &self.store_name }
    }

    pub fn ships_to(&self, country: &str) -> bool {
        self.countries.is_empty() || self.countries.iter().any(|c| c == country)
    }

    /// The payment methods checkout offers, in order. Stripe only when the
    /// site actually has keys (`stripe_ready`).
    pub fn payment_methods(&self, stripe_ready: bool) -> Vec<PaymentMethod> {
        let mut out = Vec::new();
        if self.pay_stripe && stripe_ready {
            out.push(PaymentMethod::Stripe);
        }
        if self.pay_bank {
            out.push(PaymentMethod::Bank);
        }
        if self.pay_manual {
            out.push(PaymentMethod::Manual);
        }
        out
    }
}

/// Upper-cased two-letter codes from free text: `us, ca gb` → `US CA GB`.
pub fn parse_countries(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        let p = part.trim().to_uppercase();
        if p.len() == 2 && p.chars().all(|c| c.is_ascii_uppercase()) && !out.contains(&p) {
            out.push(p);
        }
    }
    out.truncate(250);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentMethod {
    Stripe,
    Bank,
    Manual,
}

impl PaymentMethod {
    pub fn key(self) -> &'static str {
        match self {
            PaymentMethod::Stripe => "stripe",
            PaymentMethod::Bank => "bank",
            PaymentMethod::Manual => "manual",
        }
    }
    pub fn from_key(k: &str) -> Option<PaymentMethod> {
        match k {
            "stripe" => Some(PaymentMethod::Stripe),
            "bank" => Some(PaymentMethod::Bank),
            "manual" => Some(PaymentMethod::Manual),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaymentMethod::Stripe => "Card (Stripe)",
            PaymentMethod::Bank => "Bank transfer",
            PaymentMethod::Manual => "Cash on delivery",
        }
    }
}

pub fn payment_label(key: &str) -> &'static str {
    match key {
        "stripe" => "Card (Stripe)",
        "bank" => "Bank transfer",
        "manual" => "Cash on delivery",
        "admin" => "Recorded by staff",
        "free" => "Free — nothing to pay",
        _ => "Other",
    }
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// One line of text: control characters gone, whitespace collapsed, bounded.
pub fn clean(raw: &str, max: usize) -> String {
    let s: String = raw.trim().trim_matches('"').chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Several lines: each line cleaned, at most one blank line in a row.
pub fn clean_multiline(raw: &str, max: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in raw.replace("\r\n", "\n").split('\n') {
        let l = clean(line, max);
        if l.is_empty() && out.last().is_some_and(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    out.join("\n").trim().chars().take(max).collect()
}

pub fn is_email(e: &str) -> bool {
    crate::forms::is_email(e)
}

/// A URL-safe handle: `Blue T-Shirt!` → `blue-t-shirt`.
pub fn handleize(raw: &str) -> String {
    crate::directory::slugify(raw)
}

/// Comma-separated tags, trimmed, de-duplicated case-insensitively, bounded.
pub fn parse_tags(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw.split(',') {
        let t = clean(t, 40);
        if !t.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
        if out.len() >= 30 {
            break;
        }
    }
    out
}

pub fn search_terms(q: &str) -> Vec<String> {
    crate::directory::search_terms(q)
}

/// Make a CSV cell safe to open in a spreadsheet.
pub fn csv_cell(v: &str) -> String {
    crate::forms::csv_safe(v)
}

/// Plain text → escaped paragraphs.
pub fn paragraphs(text: &str) -> String {
    crate::directory::paragraphs(text)
}

// ---------------------------------------------------------------------------
// money
// ---------------------------------------------------------------------------

/// `"12.5"` → 1250, `"1,299.99"` → 129999, `"7"` → 700. Negative, blank,
/// or absurd input is `None`. For a zero-decimal currency the number is
/// already in minor units.
pub fn parse_money(raw: &str, currency: &str) -> Option<i64> {
    let s: String = raw.trim().chars().filter(|c| !matches!(c, ',' | ' ' | '$' | '€' | '£' | '¥' | '₹' | '৳')).collect();
    if s.is_empty() || s.starts_with('-') {
        return None;
    }
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) || whole.len() > 10 {
        return None;
    }
    let w: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    if zero_decimal(currency) {
        return Some(w);
    }
    let f: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        _ => frac[..2].parse().ok()?,
    };
    Some(w * 100 + f)
}

/// `0` for blank, otherwise [`parse_money`]; bad input is also 0.
pub fn money_or_zero(raw: &str, currency: &str) -> i64 {
    parse_money(raw, currency).unwrap_or(0)
}

/// The value an `<input>` shows: 1250 → `12.50`, 0 → empty.
pub fn money_input(cents: i64, currency: &str) -> String {
    if cents == 0 {
        return String::new();
    }
    if zero_decimal(currency) {
        return cents.to_string();
    }
    format!("{}.{:02}", cents / 100, (cents % 100).abs())
}

pub fn symbol(currency: &str) -> &'static str {
    match currency {
        "USD" | "CAD" | "AUD" | "NZD" | "SGD" | "HKD" | "MXN" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        "INR" => "₹",
        "BDT" => "৳",
        "BRL" => "R$",
        "ZAR" => "R",
        "PLN" => "zł",
        _ => "",
    }
}

/// 129999 USD → `$1,299.99`; 0 → `$0.00`; −500 → `−$5.00`.
pub fn format_money(cents: i64, currency: &str) -> String {
    let neg = cents < 0;
    let c = cents.abs();
    let (whole, frac) = if zero_decimal(currency) { (c, None) } else { (c / 100, Some(c % 100)) };
    let s = whole.to_string();
    let mut digits = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            digits.push(',');
        }
        digits.push(ch);
    }
    let n = match frac {
        Some(f) => format!("{digits}.{f:02}"),
        None => digits,
    };
    let sym = symbol(currency);
    let body = if sym.is_empty() { format!("{n} {currency}") } else { format!("{sym}{n}") };
    if neg { format!("−{body}") } else { body }
}

/// The amount as Stripe and schema.org want it: `12.50`.
pub fn decimal(cents: i64, currency: &str) -> String {
    if zero_decimal(currency) { cents.to_string() } else { format!("{}.{:02}", cents / 100, cents % 100) }
}

/// `12.5%` from basis points.
pub fn format_rate(bp: i64) -> String {
    let whole = bp / 100;
    let frac = bp % 100;
    if frac == 0 { format!("{whole}%") } else if frac % 10 == 0 { format!("{whole}.{}%", frac / 10) } else { format!("{whole}.{frac:02}%") }
}

/// `"8.25"` → 825 basis points.
pub fn parse_rate(raw: &str) -> Option<i64> {
    parse_money(raw.trim_end_matches('%'), "USD").filter(|bp| *bp <= 5_000)
}

// ---------------------------------------------------------------------------
// products
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductStatus {
    Active,
    Draft,
    Archived,
}

impl ProductStatus {
    pub const ALL: [ProductStatus; 3] = [ProductStatus::Active, ProductStatus::Draft, ProductStatus::Archived];
    pub fn from_key(k: &str) -> ProductStatus {
        match k {
            "active" => ProductStatus::Active,
            "archived" => ProductStatus::Archived,
            _ => ProductStatus::Draft,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            ProductStatus::Active => "active",
            ProductStatus::Draft => "draft",
            ProductStatus::Archived => "archived",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ProductStatus::Active => "Active",
            ProductStatus::Draft => "Draft",
            ProductStatus::Archived => "Archived",
        }
    }
}

pub const MAX_OPTIONS: usize = 3;
pub const MAX_OPTION_VALUES: usize = 50;
pub const MAX_VARIANTS: usize = 250;
pub const MAX_IMAGES: usize = 30;
pub const DEFAULT_VARIANT: &str = "Default Title";

/// One of a product's options: `Size` with `S, M, L`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOption {
    pub name: String,
    pub values: Vec<String>,
}

/// Rebuild options from `(name, comma-separated values)` pairs. Empty names
/// or names without values are dropped; duplicates are folded.
pub fn build_options(pairs: &[(String, String)]) -> Vec<ProductOption> {
    let mut out: Vec<ProductOption> = Vec::new();
    for (name, values) in pairs {
        let name = clean(name, 40);
        if name.is_empty() || out.iter().any(|o| o.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let mut vals: Vec<String> = Vec::new();
        for v in values.split(',') {
            let v = clean(v, 60);
            if !v.is_empty() && !vals.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
                vals.push(v);
            }
            if vals.len() >= MAX_OPTION_VALUES {
                break;
            }
        }
        if vals.is_empty() {
            continue;
        }
        out.push(ProductOption { name, values: vals });
        if out.len() >= MAX_OPTIONS {
            break;
        }
    }
    out
}

/// Options as stored, read back leniently: anything malformed is no options.
pub fn read_options(json: &str) -> Vec<ProductOption> {
    let parsed: Vec<ProductOption> = serde_json::from_str(json).unwrap_or_default();
    let pairs: Vec<(String, String)> = parsed.into_iter().map(|o| (o.name, o.values.join(","))).collect();
    build_options(&pairs)
}

/// Every combination of option values, in Shopify's order (the first
/// option varies slowest). No options → one empty combination, the
/// product's default variant. Capped at [`MAX_VARIANTS`].
pub fn combinations(options: &[ProductOption]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![Vec::new()];
    for o in options {
        let mut next = Vec::new();
        for prefix in &out {
            for v in &o.values {
                let mut c = prefix.clone();
                c.push(v.clone());
                next.push(c);
                if next.len() >= MAX_VARIANTS {
                    break;
                }
            }
            if next.len() >= MAX_VARIANTS {
                break;
            }
        }
        out = next;
    }
    out
}

pub fn variant_title(values: &[String]) -> String {
    let parts: Vec<&str> = values.iter().map(String::as_str).filter(|v| !v.is_empty()).collect();
    if parts.is_empty() { DEFAULT_VARIANT.to_string() } else { parts.join(" / ") }
}

/// Image URLs as stored (a JSON array), bounded and de-duplicated.
pub fn read_images(json: &str) -> Vec<String> {
    let parsed: Vec<String> = serde_json::from_str(json).unwrap_or_default();
    clean_images(parsed.iter().map(String::as_str))
}

/// Accept only same-site paths and http(s) URLs, nothing that could run.
pub fn clean_images<'a>(urls: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in urls {
        let u = u.trim();
        if u.is_empty() || u.len() > 500 || u.chars().any(|c| c.is_control() || c == '"' || c == '<' || c == '>') {
            continue;
        }
        let ok = u.starts_with('/') && !u.starts_with("//") || u.starts_with("https://") || u.starts_with("http://");
        if ok && !out.iter().any(|x| x == u) {
            out.push(u.to_string());
        }
        if out.len() >= MAX_IMAGES {
            break;
        }
    }
    out
}

/// Whether a variant can be bought in `qty`.
pub fn purchasable(track: bool, inventory: i64, backorder: bool, qty: i64) -> bool {
    !track || backorder || inventory >= qty
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stock {
    InStock,
    Low,
    Out,
    Untracked,
}

pub fn stock(track: bool, inventory: i64, low: i64) -> Stock {
    if !track {
        Stock::Untracked
    } else if inventory <= 0 {
        Stock::Out
    } else if inventory <= low {
        Stock::Low
    } else {
        Stock::InStock
    }
}

// ---------------------------------------------------------------------------
// collections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleField {
    Tag,
    Type,
    Vendor,
    Title,
    Price,
    Inventory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOp {
    Equals,
    NotEquals,
    Contains,
    Gt,
    Lt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub field: RuleField,
    pub op: RuleOp,
    pub value: String,
}

impl RuleField {
    pub const ALL: [RuleField; 6] = [RuleField::Tag, RuleField::Type, RuleField::Vendor, RuleField::Title, RuleField::Price, RuleField::Inventory];
    pub fn key(self) -> &'static str {
        match self {
            RuleField::Tag => "tag",
            RuleField::Type => "type",
            RuleField::Vendor => "vendor",
            RuleField::Title => "title",
            RuleField::Price => "price",
            RuleField::Inventory => "inventory",
        }
    }
    pub fn from_key(k: &str) -> Option<RuleField> {
        RuleField::ALL.into_iter().find(|f| f.key() == k)
    }
    pub fn label(self) -> &'static str {
        match self {
            RuleField::Tag => "Product tag",
            RuleField::Type => "Product type",
            RuleField::Vendor => "Vendor",
            RuleField::Title => "Title",
            RuleField::Price => "Price",
            RuleField::Inventory => "Inventory stock",
        }
    }
    fn numeric(self) -> bool {
        matches!(self, RuleField::Price | RuleField::Inventory)
    }
}

impl RuleOp {
    pub const ALL: [RuleOp; 5] = [RuleOp::Equals, RuleOp::NotEquals, RuleOp::Contains, RuleOp::Gt, RuleOp::Lt];
    pub fn key(self) -> &'static str {
        match self {
            RuleOp::Equals => "equals",
            RuleOp::NotEquals => "not_equals",
            RuleOp::Contains => "contains",
            RuleOp::Gt => "gt",
            RuleOp::Lt => "lt",
        }
    }
    pub fn from_key(k: &str) -> Option<RuleOp> {
        RuleOp::ALL.into_iter().find(|o| o.key() == k)
    }
    pub fn label(self) -> &'static str {
        match self {
            RuleOp::Equals => "is equal to",
            RuleOp::NotEquals => "is not equal to",
            RuleOp::Contains => "contains",
            RuleOp::Gt => "is greater than",
            RuleOp::Lt => "is less than",
        }
    }
}

pub const MAX_RULES: usize = 20;

/// Rebuild rules from `(field, op, value)` triples; nonsense is dropped (a
/// "greater than" on a tag, a price that is not a number).
pub fn build_rules(triples: &[(String, String, String)], currency: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    for (f, o, v) in triples {
        let (Some(field), Some(op)) = (RuleField::from_key(f), RuleOp::from_key(o)) else { continue };
        let value = clean(v, 80);
        if value.is_empty() {
            continue;
        }
        let value = if field.numeric() {
            if op == RuleOp::Contains {
                continue;
            }
            let n = if field == RuleField::Price { parse_money(&value, currency) } else { value.parse::<i64>().ok() };
            match n {
                Some(n) => n.to_string(),
                None => continue,
            }
        } else {
            if matches!(op, RuleOp::Gt | RuleOp::Lt) {
                continue;
            }
            value
        };
        out.push(Rule { field, op, value });
        if out.len() >= MAX_RULES {
            break;
        }
    }
    out
}

pub fn read_rules(json: &str) -> Vec<Rule> {
    let parsed: Vec<Rule> = serde_json::from_str(json).unwrap_or_default();
    parsed.into_iter().take(MAX_RULES).collect()
}

/// What a rule can look at on a product.
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    pub title: &'a str,
    pub product_type: &'a str,
    pub vendor: &'a str,
    pub tags: Vec<String>,
    /// Lowest variant price, cents.
    pub price: i64,
    /// Total tracked inventory.
    pub inventory: i64,
}

pub fn rule_matches(r: &Rule, f: &Facts<'_>) -> bool {
    let text = |hay: &str| {
        let h = hay.to_lowercase();
        let v = r.value.to_lowercase();
        match r.op {
            RuleOp::Equals => h == v,
            RuleOp::NotEquals => h != v,
            RuleOp::Contains => h.contains(&v),
            _ => false,
        }
    };
    let num = |n: i64| {
        let v: i64 = r.value.parse().unwrap_or(0);
        match r.op {
            RuleOp::Equals => n == v,
            RuleOp::NotEquals => n != v,
            RuleOp::Gt => n > v,
            RuleOp::Lt => n < v,
            RuleOp::Contains => false,
        }
    };
    match r.field {
        RuleField::Tag => match r.op {
            RuleOp::NotEquals => !f.tags.iter().any(|t| t.eq_ignore_ascii_case(&r.value)),
            RuleOp::Contains => f.tags.iter().any(|t| t.to_lowercase().contains(&r.value.to_lowercase())),
            _ => f.tags.iter().any(|t| t.eq_ignore_ascii_case(&r.value)),
        },
        RuleField::Type => text(f.product_type),
        RuleField::Vendor => text(f.vendor),
        RuleField::Title => text(f.title),
        RuleField::Price => num(f.price),
        RuleField::Inventory => num(f.inventory),
    }
}

/// A smart collection's test. No rules matches nothing, not everything — an
/// empty smart collection must not quietly become "all products".
pub fn collection_matches(rules: &[Rule], match_all: bool, f: &Facts<'_>) -> bool {
    if rules.is_empty() {
        return false;
    }
    if match_all { rules.iter().all(|r| rule_matches(r, f)) } else { rules.iter().any(|r| rule_matches(r, f)) }
}

pub const SORTS: &[(&str, &str)] = &[
    ("manual", "Manually"),
    ("best", "Best selling"),
    ("title", "Product title A–Z"),
    ("price_asc", "Lowest price"),
    ("price_desc", "Highest price"),
    ("newest", "Newest"),
];

pub fn sort_key(raw: &str) -> &'static str {
    SORTS.iter().find(|(k, _)| *k == raw).map(|(k, _)| *k).unwrap_or("manual")
}

// ---------------------------------------------------------------------------
// discounts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountKind {
    Percent,
    Fixed,
    FreeShipping,
}

impl DiscountKind {
    pub const ALL: [DiscountKind; 3] = [DiscountKind::Percent, DiscountKind::Fixed, DiscountKind::FreeShipping];
    pub fn from_key(k: &str) -> DiscountKind {
        match k {
            "fixed" => DiscountKind::Fixed,
            "free_shipping" => DiscountKind::FreeShipping,
            _ => DiscountKind::Percent,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            DiscountKind::Percent => "percent",
            DiscountKind::Fixed => "fixed",
            DiscountKind::FreeShipping => "free_shipping",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DiscountKind::Percent => "Percentage off",
            DiscountKind::Fixed => "Fixed amount off",
            DiscountKind::FreeShipping => "Free shipping",
        }
    }
}

/// A code as the buyer typed it, normalised: `  summer-10 ` → `SUMMER-10`.
pub fn normalise_code(raw: &str) -> String {
    raw.trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(40).collect::<String>().to_uppercase()
}

/// What the code needs to be checked against.
#[derive(Debug, Clone)]
pub struct DiscountView<'a> {
    pub kind: DiscountKind,
    pub value: i64,
    pub min_subtotal: i64,
    pub usage_limit: i64,
    pub used: i64,
    pub active: bool,
    pub starts_at: Option<&'a str>,
    pub ends_at: Option<&'a str>,
    /// For once-per-customer codes: whether this buyer has used it.
    pub used_by_buyer: bool,
}

/// Why a code does not apply, in words for the buyer — or `Ok`.
pub fn discount_applies(d: &DiscountView<'_>, subtotal: i64, now: &str, currency: &str) -> Result<(), String> {
    if !d.active {
        return Err("That code is not active.".into());
    }
    if d.starts_at.is_some_and(|s| s > now) {
        return Err("That code is not active yet.".into());
    }
    if d.ends_at.is_some_and(|e| e <= now) {
        return Err("That code has expired.".into());
    }
    if d.usage_limit > 0 && d.used >= d.usage_limit {
        return Err("That code has been used up.".into());
    }
    if d.used_by_buyer {
        return Err("That code has already been used with this email.".into());
    }
    if subtotal < d.min_subtotal {
        return Err(format!("That code needs an order of at least {}.", format_money(d.min_subtotal, currency)));
    }
    Ok(())
}

/// `(off the items, off shipping)`. Never more than what it is off.
pub fn discount_amount(kind: DiscountKind, value: i64, subtotal: i64, shipping: i64) -> (i64, i64) {
    match kind {
        DiscountKind::Percent => ((subtotal * value.clamp(0, 100) + 50) / 100, 0),
        DiscountKind::Fixed => (value.clamp(0, subtotal), 0),
        DiscountKind::FreeShipping => (0, shipping),
    }
}

pub fn describe_discount(kind: DiscountKind, value: i64, currency: &str) -> String {
    match kind {
        DiscountKind::Percent => format!("{value}% off"),
        DiscountKind::Fixed => format!("{} off", format_money(value, currency)),
        DiscountKind::FreeShipping => "Free shipping".into(),
    }
}

// ---------------------------------------------------------------------------
// totals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub subtotal: i64,
    /// Total discount, items and shipping together.
    pub discount: i64,
    /// Shipping after any free-shipping discount.
    pub shipping: i64,
    pub tax: i64,
    pub total: i64,
}

/// The arithmetic of a checkout.
///
/// * `taxable` is the part of `subtotal` on taxable products.
/// * Prices that include tax: tax is the share already inside them, and the
///   total is not raised. Prices without tax: tax is added on top.
/// * An item discount is spread over taxable and untaxed lines in
///   proportion, so a discount lowers the tax as it would on a till.
#[allow(clippy::too_many_arguments)]
pub fn totals(subtotal: i64, taxable: i64, item_discount: i64, shipping: i64, shipping_discount: i64, rate_bp: i64, included: bool, tax_shipping: bool) -> Totals {
    let item_discount = item_discount.clamp(0, subtotal);
    let shipping_after = (shipping - shipping_discount).max(0);
    let taxable_after = if subtotal > 0 { taxable - (item_discount * taxable + subtotal / 2) / subtotal } else { 0 }.max(0);
    let base = taxable_after + if tax_shipping { shipping_after } else { 0 };
    let tax = if rate_bp <= 0 {
        0
    } else if included {
        (base * rate_bp + (10_000 + rate_bp) / 2) / (10_000 + rate_bp)
    } else {
        (base * rate_bp + 5_000) / 10_000
    };
    let total = subtotal - item_discount + shipping_after + if included { 0 } else { tax };
    Totals { subtotal, discount: item_discount + (shipping - shipping_after), shipping: shipping_after, tax, total }
}

// ---------------------------------------------------------------------------
// carts
// ---------------------------------------------------------------------------

pub const MAX_CART_LINES: usize = 50;
pub const MAX_LINE_QTY: i64 = 999;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartLine {
    pub variant_id: String,
    pub qty: i64,
}

/// A cart as stored, rebuilt: valid ids, positive bounded quantities,
/// duplicate lines merged.
pub fn read_cart(json: &str) -> Vec<CartLine> {
    let parsed: Vec<CartLine> = serde_json::from_str(json).unwrap_or_default();
    normalise_cart(parsed)
}

pub fn normalise_cart(lines: Vec<CartLine>) -> Vec<CartLine> {
    let mut out: Vec<CartLine> = Vec::new();
    for l in lines {
        if !crate::ids::is_objectid(&l.variant_id) || l.qty <= 0 {
            continue;
        }
        if let Some(e) = out.iter_mut().find(|e| e.variant_id == l.variant_id) {
            e.qty = (e.qty + l.qty).min(MAX_LINE_QTY);
        } else if out.len() < MAX_CART_LINES {
            out.push(CartLine { variant_id: l.variant_id, qty: l.qty.min(MAX_LINE_QTY) });
        }
    }
    out
}

/// A cart token from a cookie: 32 lowercase hex characters, nothing else.
pub fn is_cart_token(t: &str) -> bool {
    t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub const CART_COOKIE: &str = "vuuut-cart";

// ---------------------------------------------------------------------------
// addresses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Address {
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub line1: String,
    #[serde(default)]
    pub line2: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub postal: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub phone: String,
}

impl Address {
    /// Rebuilt from form fields (`prefix` + name).
    pub fn from_form(get: impl Fn(&str) -> Option<String>) -> Address {
        let f = |k: &str, max: usize| clean(&get(k).unwrap_or_default(), max);
        let country = f("country", 2).to_uppercase();
        Address {
            first_name: f("first_name", 60),
            last_name: f("last_name", 60),
            company: f("company", 80),
            line1: f("line1", 120),
            line2: f("line2", 120),
            city: f("city", 80),
            region: f("region", 80),
            postal: f("postal", 20),
            country: if country.len() == 2 && country.chars().all(|c| c.is_ascii_uppercase()) { country } else { String::new() },
            phone: f("phone", 30),
        }
    }

    pub fn read(json: &str) -> Address {
        let a: Address = serde_json::from_str(json).unwrap_or_default();
        // Stored values went through from_form, but rebuild anyway so a row
        // written by hand cannot smuggle in something longer.
        Address::from_form(|k| {
            Some(match k {
                "first_name" => a.first_name.clone(),
                "last_name" => a.last_name.clone(),
                "company" => a.company.clone(),
                "line1" => a.line1.clone(),
                "line2" => a.line2.clone(),
                "city" => a.city.clone(),
                "region" => a.region.clone(),
                "postal" => a.postal.clone(),
                "country" => a.country.clone(),
                "phone" => a.phone.clone(),
                _ => String::new(),
            })
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn is_empty(&self) -> bool {
        self.line1.is_empty() && self.city.is_empty() && self.first_name.is_empty() && self.last_name.is_empty()
    }

    pub fn name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name).trim().to_string()
    }

    /// What is missing for shipping, as field names.
    pub fn missing(&self, require_phone: bool) -> Vec<&'static str> {
        let mut m = Vec::new();
        if self.last_name.is_empty() && self.first_name.is_empty() {
            m.push("name");
        }
        if self.line1.is_empty() {
            m.push("address");
        }
        if self.city.is_empty() {
            m.push("city");
        }
        if self.country.is_empty() {
            m.push("country");
        }
        if require_phone && self.phone.is_empty() {
            m.push("phone");
        }
        m
    }

    /// The address as lines for a label or a receipt.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for l in [self.name(), self.company.clone(), self.line1.clone(), self.line2.clone()] {
            if !l.is_empty() {
                out.push(l);
            }
        }
        let city = [self.city.as_str(), self.region.as_str(), self.postal.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
        if !city.is_empty() {
            out.push(city);
        }
        if !self.country.is_empty() {
            out.push(country_name(&self.country).to_string());
        }
        if !self.phone.is_empty() {
            out.push(self.phone.clone());
        }
        out
    }
}

/// The common countries by code, for the checkout picker. Anything else is
/// shown by its code.
pub const COUNTRIES: &[(&str, &str)] = &[
    ("AE", "United Arab Emirates"), ("AR", "Argentina"), ("AT", "Austria"), ("AU", "Australia"), ("BD", "Bangladesh"), ("BE", "Belgium"),
    ("BR", "Brazil"), ("CA", "Canada"), ("CH", "Switzerland"), ("CL", "Chile"), ("CN", "China"), ("CO", "Colombia"), ("CZ", "Czechia"),
    ("DE", "Germany"), ("DK", "Denmark"), ("EG", "Egypt"), ("ES", "Spain"), ("FI", "Finland"), ("FR", "France"), ("GB", "United Kingdom"),
    ("GR", "Greece"), ("HK", "Hong Kong"), ("HU", "Hungary"), ("ID", "Indonesia"), ("IE", "Ireland"), ("IL", "Israel"), ("IN", "India"),
    ("IT", "Italy"), ("JP", "Japan"), ("KE", "Kenya"), ("KR", "South Korea"), ("LK", "Sri Lanka"), ("MX", "Mexico"), ("MY", "Malaysia"),
    ("NG", "Nigeria"), ("NL", "Netherlands"), ("NO", "Norway"), ("NP", "Nepal"), ("NZ", "New Zealand"), ("PH", "Philippines"),
    ("PK", "Pakistan"), ("PL", "Poland"), ("PT", "Portugal"), ("RO", "Romania"), ("SA", "Saudi Arabia"), ("SE", "Sweden"),
    ("SG", "Singapore"), ("TH", "Thailand"), ("TR", "Türkiye"), ("TW", "Taiwan"), ("UA", "Ukraine"), ("US", "United States"),
    ("VN", "Vietnam"), ("ZA", "South Africa"),
];

pub fn country_name(code: &str) -> &str {
    COUNTRIES.iter().find(|(c, _)| *c == code).map(|(_, n)| *n).unwrap_or(code)
}

// ---------------------------------------------------------------------------
// orders
// ---------------------------------------------------------------------------

pub fn financial_label(k: &str) -> &'static str {
    match k {
        "paid" => "Paid",
        "partially_refunded" => "Partially refunded",
        "refunded" => "Refunded",
        "voided" => "Voided",
        _ => "Payment pending",
    }
}

pub fn fulfillment_label(k: &str) -> &'static str {
    match k {
        "fulfilled" => "Fulfilled",
        _ => "Unfulfilled",
    }
}

/// The financial status after refunding `refunded` of `total`.
pub fn refund_status(total: i64, refunded: i64) -> &'static str {
    if refunded <= 0 {
        "paid"
    } else if refunded >= total {
        "refunded"
    } else {
        "partially_refunded"
    }
}

/// A public order token: 32 hex characters.
pub fn is_order_token(t: &str) -> bool {
    is_cart_token(t)
}

/// Carriers whose tracking page can be linked from the number alone.
pub fn tracking_link(company: &str, number: &str) -> Option<String> {
    let n: String = number.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if n.is_empty() {
        return None;
    }
    let url = match company.to_lowercase().as_str() {
        "ups" => format!("https://www.ups.com/track?tracknum={n}"),
        "usps" => format!("https://tools.usps.com/go/TrackConfirmAction?tLabels={n}"),
        "fedex" => format!("https://www.fedex.com/fedextrack/?trknbr={n}"),
        "dhl" => format!("https://www.dhl.com/en/express/tracking.html?AWB={n}"),
        "royal mail" => format!("https://www.royalmail.com/track-your-item#/tracking-results/{n}"),
        "canada post" => format!("https://www.canadapost-postescanada.ca/track-reperage/en#/details/{n}"),
        _ => return None,
    };
    Some(url)
}

pub const CARRIERS: &[&str] = &["UPS", "USPS", "FedEx", "DHL", "Royal Mail", "Canada Post", "Other"];



// ---------------------------------------------------------------------------
// the store's addresses
// ---------------------------------------------------------------------------
//
// The storefront's routes are mounted at `/shop/…` and stay there. What a
// visitor sees is configurable: the store at any `/<base>/`, and products
// at any `/<product path>/<handle>/` (the same as the base gives
// `/themes/<handle>/`). Incoming public addresses are translated to the
// internal ones before routing ([`Paths::internal_path`]); every link the
// store writes goes the other way ([`Paths::public_path`]). The addresses
// travel with each request (see `ghost_front::shop::addresses`) rather than
// in a process-wide variable, so two requests can never write each other's.

pub const DEFAULT_BASE: &str = "shop";
pub const DEFAULT_PRODUCT_PATH: &str = "shop/products";

/// The store's own second segments. A product handle may not be one of
/// these, because with products directly under the store
/// (`/themes/<handle>/`) it would shadow the cart or the checkout.
pub const RESERVED_HANDLES: &[&str] = &["cart", "checkout", "collections", "orders", "account", "downloads", "dashboard", "policies", "assets", "products"];

/// First segments the site already owns; the store may not take them.
const RESERVED_ROOTS: &[&str] = &[
    "ghost", "members", "content", "assets", "public", "p", "private", "tag", "author", "rss", "sitemap.xml", "page", "email", "webmentions",
    "unsubscribe", "r", ".well-known", ".ghost", "directory", "search", "forms", "cookies", "ads", "popups", "llms.txt", "robots.txt", "favicon.ico",
];

/// A store path as typed (`/Themes/`, `shop/products`, `digital goods`)
/// cleaned to segments of `a-z 0-9 -`, one to three of them, or `None`
/// when it cannot be one.
pub fn clean_path(raw: &str) -> Option<String> {
    let segs: Vec<String> = raw.trim().trim_matches('/').split('/').map(|s| handleize(s)).collect();
    if segs.is_empty() || segs.len() > 3 || segs.iter().any(|s| s.is_empty()) {
        return None;
    }
    if RESERVED_ROOTS.contains(&segs[0].as_str()) {
        return None;
    }
    Some(segs.join("/"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub base: String,
    pub product: String,
}

impl Default for Paths {
    fn default() -> Self {
        Paths { base: DEFAULT_BASE.into(), product: DEFAULT_PRODUCT_PATH.into() }
    }
}

impl Paths {
    pub fn from_settings(s: &Settings) -> Paths {
        Paths { base: s.base_path.clone(), product: s.product_path.clone() }
    }

    pub fn is_default(&self) -> bool {
        *self == Paths::default()
    }

    /// `/shop/products/hat/` → `/themes/hat/`; `/shop/cart/` → `/themes/cart/`.
    pub fn public_path(&self, internal: &str) -> String {
        let (path, rest) = match internal.find(['?', '#']) {
            Some(i) => internal.split_at(i),
            None => (internal, ""),
        };
        let mapped = if let Some(tail) = path.strip_prefix("/shop/products/") {
            format!("/{}/{tail}", self.product)
        } else if let Some(tail) = path.strip_prefix("/shop/") {
            format!("/{}/{tail}", self.base)
        } else if path == "/shop" {
            format!("/{}/", self.base)
        } else {
            path.to_string()
        };
        format!("{mapped}{rest}")
    }

    /// The internal route for a public path, or `None` when it is not the
    /// store's. Products are tried first, for a single handle segment that
    /// is not one of the store's own words.
    pub fn internal_path(&self, public: &str) -> Option<String> {
        let prefix = format!("/{}/", self.product);
        if let Some(tail) = public.strip_prefix(&prefix) {
            let mut parts = tail.splitn(2, '/');
            let handle = parts.next().unwrap_or("");
            let after = parts.next();
            if !handle.is_empty() && !RESERVED_HANDLES.contains(&handle) && after.is_none_or(|a| a.is_empty()) {
                return Some(format!("/shop/products/{tail}"));
            }
        }
        let base = format!("/{}/", self.base);
        if let Some(tail) = public.strip_prefix(&base) {
            return Some(format!("/shop/{tail}"));
        }
        if public == format!("/{}", self.base) {
            return Some("/shop/".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// digital downloads
// ---------------------------------------------------------------------------

/// Largest file a product may carry.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 20;

/// What a buyer's browser saves the file as: the last path segment only,
/// no control characters, quotes or separators, at most 180 characters.
pub fn safe_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '<' | '>' | ':' | '|' | '?' | '*' | ';'))
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let out: String = cleaned.chars().take(180).collect();
    if out.is_empty() { "download".into() } else { out }
}

/// A `Content-Disposition: attachment` value that every browser reads the
/// same way: an ASCII fallback plus the RFC 5987 UTF-8 name.
pub fn content_disposition(filename: &str) -> String {
    let name = safe_filename(filename);
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '_' }).collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// A content type from the extension, for the admin list only. Files are
/// always served as attachments, whatever this says.
pub fn content_type_for(filename: &str) -> &'static str {
    match filename.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("epub") => "application/epub+zip",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        _ => "application/octet-stream",
    }
}

/// `2.4 MB`.
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else if v >= 10.0 { format!("{v:.0} {}", UNITS[i]) } else { format!("{v:.1} {}", UNITS[i]) }
}

/// Whether a grant may serve one more download, or why not (for the buyer).
pub fn download_allowed(count: i64, limit: i64, expires_at: Option<&str>, revoked: bool, now: &str) -> Result<(), &'static str> {
    if revoked {
        return Err("This download is no longer available.");
    }
    if expires_at.is_some_and(|e| e <= now) {
        return Err("This download has expired.");
    }
    if limit > 0 && count >= limit {
        return Err("You have used all the downloads for this purchase.");
    }
    Ok(())
}

/// Whether an order's payment state lets its files be downloaded.
pub fn order_unlocks_downloads(status: &str, financial: &str) -> bool {
    status != "cancelled" && matches!(financial, "paid" | "partially_refunded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_round_trips_without_floats() {
        assert_eq!(parse_money("12.5", "USD"), Some(1250));
        assert_eq!(parse_money("1,299.99", "USD"), Some(129999));
        assert_eq!(parse_money("$7", "USD"), Some(700));
        assert_eq!(parse_money(".5", "USD"), Some(50));
        assert_eq!(parse_money("-3", "USD"), None);
        assert_eq!(parse_money("abc", "USD"), None);
        assert_eq!(parse_money("", "USD"), None);
        assert_eq!(parse_money("1500", "JPY"), Some(1500));
        assert_eq!(format_money(129999, "USD"), "$1,299.99");
        assert_eq!(format_money(0, "EUR"), "€0.00");
        assert_eq!(format_money(1500, "JPY"), "¥1,500");
        assert_eq!(format_money(-500, "GBP"), "−£5.00");
        assert_eq!(format_money(500, "CHF"), "5.00 CHF");
        assert_eq!(money_input(1250, "USD"), "12.50");
        assert_eq!(money_input(0, "USD"), "");
    }

    #[test]
    fn rates_are_basis_points() {
        assert_eq!(parse_rate("8.25"), Some(825));
        assert_eq!(parse_rate("20%"), Some(2000));
        assert_eq!(parse_rate("99"), None);
        assert_eq!(format_rate(825), "8.25%");
        assert_eq!(format_rate(2000), "20%");
        assert_eq!(format_rate(750), "7.5%");
    }

    #[test]
    fn options_are_rebuilt_and_bounded() {
        let o = build_options(&[
            ("Size".into(), "S, M, m, L,".into()),
            ("size".into(), "XL".into()),
            ("".into(), "x".into()),
            ("Color".into(), "".into()),
            ("Colour".into(), "Red,Blue".into()),
            ("Material".into(), "Cotton".into()),
            ("Fourth".into(), "Nope".into()),
        ]);
        assert_eq!(o.len(), 3);
        assert_eq!(o[0].values, vec!["S", "M", "L"]);
        assert_eq!(o[1].name, "Colour");
    }

    #[test]
    fn combinations_follow_shopify_order() {
        let o = build_options(&[("Size".into(), "S,M".into()), ("Color".into(), "Red,Blue".into())]);
        let c = combinations(&o);
        assert_eq!(c, vec![vec!["S", "Red"], vec!["S", "Blue"], vec!["M", "Red"], vec!["M", "Blue"]]);
        assert_eq!(combinations(&[]), vec![Vec::<String>::new()]);
        assert_eq!(variant_title(&[]), DEFAULT_VARIANT);
        assert_eq!(variant_title(&["S".into(), "Red".into()]), "S / Red");
        let big = build_options(&[("A".into(), (0..50).map(|i| i.to_string()).collect::<Vec<_>>().join(",")), ("B".into(), (0..50).map(|i| i.to_string()).collect::<Vec<_>>().join(","))]);
        assert_eq!(combinations(&big).len(), MAX_VARIANTS);
    }

    #[test]
    fn images_refuse_scripts_and_protocol_relative_urls() {
        let i = clean_images(["/content/images/a.jpg", "javascript:alert(1)", "//evil.example/x.png", "https://cdn.example/b.png", "/content/images/a.jpg", "/x\"onerror=1"].into_iter());
        assert_eq!(i, vec!["/content/images/a.jpg", "https://cdn.example/b.png"]);
    }

    #[test]
    fn smart_collections_match_on_facts() {
        let rules = build_rules(
            &[
                ("tag".into(), "equals".into(), "Summer".into()),
                ("price".into(), "lt".into(), "50".into()),
                ("tag".into(), "gt".into(), "nonsense".into()),
                ("price".into(), "contains".into(), "1".into()),
            ],
            "USD",
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1].value, "5000");
        let f = Facts { title: "Hat", tags: vec!["summer".into()], price: 2500, ..Default::default() };
        assert!(collection_matches(&rules, true, &f));
        let pricey = Facts { price: 9000, ..f.clone() };
        assert!(!collection_matches(&rules, true, &pricey));
        assert!(collection_matches(&rules, false, &pricey));
        assert!(!collection_matches(&[], false, &f), "no rules is an empty collection, not everything");
    }

    #[test]
    fn discount_codes_check_every_condition() {
        let d = DiscountView { kind: DiscountKind::Percent, value: 10, min_subtotal: 5000, usage_limit: 2, used: 1, active: true, starts_at: None, ends_at: Some("2030-01-01 00:00:00"), used_by_buyer: false };
        let now = "2026-11-22 10:00:00";
        assert!(discount_applies(&d, 5000, now, "USD").is_ok());
        assert!(discount_applies(&d, 4999, now, "USD").unwrap_err().contains("$50.00"));
        assert!(discount_applies(&DiscountView { used: 2, ..d.clone() }, 9000, now, "USD").is_err());
        assert!(discount_applies(&DiscountView { ends_at: Some("2020-01-01 00:00:00"), ..d.clone() }, 9000, now, "USD").is_err());
        assert!(discount_applies(&DiscountView { starts_at: Some("2027-01-01 00:00:00"), ..d.clone() }, 9000, now, "USD").is_err());
        assert!(discount_applies(&DiscountView { active: false, ..d.clone() }, 9000, now, "USD").is_err());
        assert!(discount_applies(&DiscountView { used_by_buyer: true, ..d }, 9000, now, "USD").is_err());
        assert_eq!(discount_amount(DiscountKind::Percent, 15, 1000, 500), (150, 0));
        assert_eq!(discount_amount(DiscountKind::Fixed, 5000, 1000, 500), (1000, 0));
        assert_eq!(discount_amount(DiscountKind::FreeShipping, 0, 1000, 500), (0, 500));
        assert_eq!(normalise_code("  summer-10 "), "SUMMER-10");
    }

    #[test]
    fn totals_add_tax_or_take_it_out_of_the_price() {
        // $100 of goods, 10% tax on top, $10 shipping untaxed.
        let t = totals(10_000, 10_000, 0, 1_000, 0, 1_000, false, false);
        assert_eq!(t, Totals { subtotal: 10_000, discount: 0, shipping: 1_000, tax: 1_000, total: 12_000 });
        // Same, 20% VAT already inside the price: the total does not move.
        let t = totals(12_000, 12_000, 0, 0, 0, 2_000, true, false);
        assert_eq!((t.tax, t.total), (2_000, 12_000));
        // A 10% discount lowers the tax with it.
        let t = totals(10_000, 10_000, 1_000, 0, 0, 1_000, false, false);
        assert_eq!((t.tax, t.total, t.discount), (900, 9_900, 1_000));
        // Free shipping shows as a discount and zero shipping.
        let t = totals(5_000, 0, 0, 800, 800, 1_000, false, true);
        assert_eq!((t.shipping, t.discount, t.tax, t.total), (0, 800, 0, 5_000));
        // Untaxed products pay no tax.
        assert_eq!(totals(5_000, 0, 0, 0, 0, 1_000, false, false).tax, 0);
    }

    #[test]
    fn carts_are_rebuilt_from_untrusted_json() {
        let id = "0123456789abcdef01234567";
        let c = read_cart(&format!(r#"[{{"variant_id":"{id}","qty":2}},{{"variant_id":"{id}","qty":3}},{{"variant_id":"x","qty":1}},{{"variant_id":"{id}0","qty":-1}}]"#));
        assert_eq!(c, vec![CartLine { variant_id: id.into(), qty: 5 }]);
        assert!(read_cart("not json").is_empty());
        assert!(is_cart_token("0123456789abcdef0123456789abcdef"));
        assert!(!is_cart_token("0123456789ABCDEF0123456789abcdef"));
        assert!(!is_cart_token("../../etc/passwd"));
    }

    #[test]
    fn addresses_are_cleaned_and_checked() {
        let a = Address::from_form(|k| Some(match k {
            "first_name" => "  Ada\u{0}  ".into(),
            "line1" => "1 Main St".into(),
            "city" => "Springfield".into(),
            "country" => "us".into(),
            _ => String::new(),
        }));
        assert_eq!(a.first_name, "Ada");
        assert_eq!(a.country, "US");
        assert!(a.missing(false).is_empty());
        assert_eq!(a.missing(true), vec!["phone"]);
        assert_eq!(Address::read(&a.to_json()), a);
        assert!(a.lines().contains(&"United States".to_string()));
        assert_eq!(parse_countries("us, ca  gb,xx1,US"), vec!["US", "CA", "GB"]);
    }

    #[test]
    fn stock_and_purchase_rules() {
        assert!(purchasable(true, 3, false, 3));
        assert!(!purchasable(true, 2, false, 3));
        assert!(purchasable(true, 0, true, 3));
        assert!(purchasable(false, 0, false, 3));
        assert_eq!(stock(true, 0, 5), Stock::Out);
        assert_eq!(stock(true, 4, 5), Stock::Low);
        assert_eq!(stock(true, 40, 5), Stock::InStock);
        assert_eq!(refund_status(1000, 0), "paid");
        assert_eq!(refund_status(1000, 400), "partially_refunded");
        assert_eq!(refund_status(1000, 1000), "refunded");
        assert_eq!(tracking_link("UPS", "1Z 999"), Some("https://www.ups.com/track?tracknum=1Z999".into()));
        assert_eq!(tracking_link("Other", "1"), None);
    }

    #[test]
    fn download_names_and_rules() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("C:\\x\\Guide \"v2\".pdf"), "Guide v2.pdf");
        assert_eq!(safe_filename("\u{0}..."), "download");
        let cd = content_disposition("Résumé kit.zip");
        assert!(cd.starts_with("attachment; filename=\"R_sum_ kit.zip\""), "{cd}");
        assert!(cd.contains("filename*=UTF-8''R%C3%A9sum%C3%A9%20kit.zip"), "{cd}");
        assert!(!content_disposition("a\"b\r\nX-Evil: 1").contains('\n'));
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2_500_000), "2.4 MB");
        assert_eq!(format_bytes(50 * 1024 * 1024), "50 MB");
        let now = "2026-11-23 10:00:00";
        assert!(download_allowed(0, 0, None, false, now).is_ok());
        assert!(download_allowed(3, 3, None, false, now).is_err());
        assert!(download_allowed(0, 0, Some("2026-11-01 00:00:00"), false, now).is_err());
        assert!(download_allowed(0, 0, None, true, now).is_err());
        assert!(order_unlocks_downloads("open", "paid"));
        assert!(order_unlocks_downloads("archived", "partially_refunded"));
        assert!(!order_unlocks_downloads("open", "pending"));
        assert!(!order_unlocks_downloads("open", "refunded"));
        assert!(!order_unlocks_downloads("cancelled", "paid"));
    }

    #[test]
    fn store_addresses_map_both_ways() {
        assert_eq!(clean_path("/Themes/"), Some("themes".into()));
        assert_eq!(clean_path("digital goods/items"), Some("digital-goods/items".into()));
        assert_eq!(clean_path("ghost"), None);
        assert_eq!(clean_path("a/b/c/d"), None);
        assert_eq!(clean_path("//"), None);
        let d = Paths::default();
        assert_eq!(d.public_path("/shop/products/hat/?variant=1"), "/shop/products/hat/?variant=1");
        let t = Paths { base: "themes".into(), product: "themes".into() };
        assert_eq!(t.public_path("/shop/"), "/themes/");
        assert_eq!(t.public_path("/shop/products/asahi/"), "/themes/asahi/");
        assert_eq!(t.public_path("/shop/cart/"), "/themes/cart/");
        assert_eq!(t.public_path("/about/"), "/about/");
        assert_eq!(t.internal_path("/themes/"), Some("/shop/".into()));
        assert_eq!(t.internal_path("/themes"), Some("/shop/".into()));
        assert_eq!(t.internal_path("/themes/asahi/"), Some("/shop/products/asahi/".into()));
        assert_eq!(t.internal_path("/themes/cart/"), Some("/shop/cart/".into()), "the store's own words are not products");
        assert_eq!(t.internal_path("/themes/collections/new/"), Some("/shop/collections/new/".into()));
        assert_eq!(t.internal_path("/themes/downloads/a/b/"), Some("/shop/downloads/a/b/".into()));
        assert_eq!(t.internal_path("/about/"), None);
        let split = Paths { base: "store".into(), product: "item".into() };
        assert_eq!(split.public_path("/shop/products/hat/"), "/item/hat/");
        assert_eq!(split.internal_path("/item/hat/"), Some("/shop/products/hat/".into()));
        assert_eq!(split.internal_path("/item/hat/extra/"), None);
        for internal in ["/shop/", "/shop/products/x/", "/shop/cart/", "/shop/downloads/g/f/", "/shop/collections/c/", "/shop/orders/t/send-link/"] {
            assert_eq!(t.internal_path(&t.public_path(internal)).as_deref(), Some(internal), "{internal} round-trips");
            assert_eq!(split.internal_path(&split.public_path(internal)).as_deref(), Some(internal), "{internal} round-trips");
        }
    }
}
