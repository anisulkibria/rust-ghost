//! Real Estate & Property Listings: types, agents, properties and the
//! enquiries visitors send about them.
//!
//! Every value is bound; the few fragments written into SQL (sort orders,
//! the `IN (…)` marks) come from fixed lists here, never from a request.
//! Long text is MEDIUMTEXT on MySQL and read through COALESCE. Writes that
//! change what the public pages show call `content::touch()` so the page
//! cache empties. The rules for every column are `vuuut_core::properties`.

use super::dberr;
use crate::Db;
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_core::properties::{self as rules, AgentInput, EnquiryInput, Market, PropertyInput, Purpose, Settings};
use vuuut_core::{dates, ids, Result};

fn touch() {
    crate::content::touch();
}

fn public_url(s: &Settings, internal: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(internal))
}

/// A bound value of either kind, so numeric filters compare as numbers on
/// both engines.
#[derive(Debug, Clone)]
enum Bind {
    S(String),
    I(i64),
}

// ---------------------------------------------------------------------------
// types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct PropertyType {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub image: String,
    pub tint: String,
    pub sort_order: i64,
}

const TYPE_COLS: &str = "id, name, slug, description, image, tint, sort_order";

pub async fn types(pool: &Db) -> Vec<PropertyType> {
    sqlx::query_as::<_, PropertyType>(&format!("SELECT {TYPE_COLS} FROM property_types ORDER BY sort_order, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn type_by_slug(pool: &Db, slug: &str) -> Option<PropertyType> {
    sqlx::query_as::<_, PropertyType>(&format!("SELECT {TYPE_COLS} FROM property_types WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn type_slug_taken(pool: &Db, slug: &str, except: Option<&str>) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM property_types WHERE slug = ? AND id <> ?").bind(slug).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0) > 0
}

#[allow(clippy::too_many_arguments)]
pub async fn save_type(pool: &Db, id: Option<&str>, name: &str, slug: &str, description: &str, image: &str, tint: &str, sort: i64) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE property_types SET name = ?, slug = ?, description = ?, image = ?, tint = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(name).bind(slug).bind(description).bind(image).bind(tint).bind(sort).bind(&now).bind(id).execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO property_types (id, name, slug, description, image, tint, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(name).bind(slug).bind(description).bind(image).bind(tint).bind(sort).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_type(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE properties SET type_id = NULL WHERE type_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM property_types WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Properties per type: all of them, and those the public can see.
pub async fn type_counts(pool: &Db, public: bool) -> HashMap<String, i64> {
    let sql = if public {
        "SELECT type_id, COUNT(*) FROM properties WHERE status = 'published' AND type_id IS NOT NULL GROUP BY type_id"
    } else {
        "SELECT type_id, COUNT(*) FROM properties WHERE type_id IS NOT NULL GROUP BY type_id"
    };
    sqlx::query_as::<_, (String, i64)>(sql).fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

// ---------------------------------------------------------------------------
// agents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Agent {
    pub id: String,
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
    pub sort_order: i64,
}

impl Agent {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }
    pub fn initials(&self) -> String {
        self.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
    }
}

const AGENT_COLS: &str = "id, name, slug, title, agency, email, phone, mobile, photo, COALESCE(bio, '') AS bio, website, license, languages, status, sort_order";

pub async fn agents(pool: &Db, active_only: bool) -> Vec<Agent> {
    let filter = if active_only { " WHERE status = 'active'" } else { "" };
    sqlx::query_as::<_, Agent>(&format!("SELECT {AGENT_COLS} FROM property_agents{filter} ORDER BY sort_order, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_agent(pool: &Db, id: &str) -> Option<Agent> {
    sqlx::query_as::<_, Agent>(&format!("SELECT {AGENT_COLS} FROM property_agents WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn agent_by_slug(pool: &Db, slug: &str) -> Option<Agent> {
    sqlx::query_as::<_, Agent>(&format!("SELECT {AGENT_COLS} FROM property_agents WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

async fn unique_agent_slug(pool: &Db, wanted: &str, except: Option<&str>) -> String {
    let base = if wanted.is_empty() { "agent".to_string() } else { wanted.to_string() };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM property_agents WHERE slug = ? AND id <> ?").bind(&candidate).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

pub async fn save_agent(pool: &Db, id: Option<&str>, a: &AgentInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let slug = unique_agent_slug(pool, &a.slug, id).await;
    match id {
        Some(id) => {
            sqlx::query("UPDATE property_agents SET name = ?, slug = ?, title = ?, agency = ?, email = ?, phone = ?, mobile = ?, photo = ?, bio = ?, website = ?, license = ?, languages = ?, status = ?, updated_at = ? WHERE id = ?")
                .bind(&a.name).bind(&slug).bind(&a.title).bind(&a.agency).bind(&a.email).bind(&a.phone).bind(&a.mobile).bind(&a.photo).bind(&a.bio).bind(&a.website).bind(&a.license).bind(&a.languages).bind(&a.status).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            let sort: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM property_agents").fetch_one(pool).await.unwrap_or(0);
            sqlx::query("INSERT INTO property_agents (id, name, slug, title, agency, email, phone, mobile, photo, bio, website, license, languages, status, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(&a.name).bind(&slug).bind(&a.title).bind(&a.agency).bind(&a.email).bind(&a.phone).bind(&a.mobile).bind(&a.photo).bind(&a.bio).bind(&a.website).bind(&a.license).bind(&a.languages).bind(&a.status).bind(sort).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

/// The agent goes; their properties stay, unassigned.
pub async fn delete_agent(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE properties SET agent_id = NULL WHERE agent_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM property_agents WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Properties per agent, all of them.
pub async fn agent_counts(pool: &Db) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT agent_id, COUNT(*) FROM properties WHERE agent_id IS NOT NULL GROUP BY agent_id").fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

// ---------------------------------------------------------------------------
// properties
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Property {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub purpose: String,
    pub market: String,
    pub type_id: Option<String>,
    pub agent_id: Option<String>,
    pub reference: String,
    pub price_cents: i64,
    pub price_poa: i64,
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
    pub gallery: String,
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
    pub hide_address: i64,
    pub features: String,
    pub extra_features: String,
    pub floor_plans: String,
    pub open_houses: String,
    pub video_url: String,
    pub tour_url: String,
    pub brochure_url: String,
    pub featured: i64,
    pub keywords: String,
    pub seo_title: String,
    pub seo_description: String,
    pub views: i64,
    pub enquiries: i64,
    pub published_at: Option<String>,
    pub closed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Property {
    pub fn purpose(&self) -> Purpose {
        Purpose::from_key(&self.purpose)
    }
    pub fn market(&self) -> Market {
        Market::from_key(&self.market)
    }
    pub fn is_published(&self) -> bool {
        self.status == "published"
    }
    pub fn is_featured(&self) -> bool {
        self.featured != 0
    }
    pub fn gallery_list(&self) -> Vec<String> {
        self.gallery.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }
    /// The cover first, then the gallery.
    pub fn images(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.image.is_empty() {
            v.push(self.image.clone());
        }
        v.extend(self.gallery_list());
        v
    }
    pub fn price_label(&self, currency: &str) -> String {
        rules::price_label(self.price_cents, currency, self.purpose(), &self.rent_period, self.price_poa != 0, &self.price_prefix)
    }
    /// `2`, `2.5` (two full and one half).
    pub fn baths_label(&self) -> String {
        match (self.bathrooms, self.half_baths) {
            (0, 0) => String::new(),
            (b, 0) => b.to_string(),
            (b, 1) => format!("{b}.5"),
            (b, h) => format!("{b} + {h} half"),
        }
    }
    pub fn feature_keys(&self) -> Vec<&'static str> {
        rules::feature_keys(&self.features)
    }
    pub fn extra_list(&self) -> Vec<String> {
        self.extra_features.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }
    pub fn floor_plan_list(&self) -> Vec<rules::FloorPlan> {
        rules::floor_plans_from_json(&self.floor_plans)
    }
    pub fn open_house_list(&self) -> Vec<rules::OpenHouse> {
        rules::open_houses_from_json(&self.open_houses)
    }
    pub fn description_markup(&self) -> String {
        if self.description_html.trim().is_empty() { vuuut_core::directory::paragraphs(&self.description) } else { self.description_html.clone() }
    }
    /// The address the public sees: the street only when it is not hidden.
    pub fn public_address(&self) -> String {
        let street = if self.hide_address != 0 { "" } else { self.address.as_str() };
        [street, &self.neighborhood, &self.city, &self.region, &self.postal_code, &self.country]
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    }
    pub fn short_location(&self) -> String {
        [&self.neighborhood, &self.city].iter().filter(|p| !p.is_empty()).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
    }
    /// The pin the public sees; with the address hidden, about a kilometre out.
    pub fn public_coords(&self) -> Option<(f64, f64)> {
        let lat: f64 = self.latitude.parse().ok()?;
        let lng: f64 = self.longitude.parse().ok()?;
        if self.hide_address != 0 {
            Some(((lat * 100.0).round() / 100.0, (lng * 100.0).round() / 100.0))
        } else {
            Some((lat, lng))
        }
    }
}

const COLS: &str = "id, slug, title, status, purpose, market, type_id, agent_id, reference, price_cents, price_poa, price_prefix, rent_period, \
deposit_cents, service_charge_cents, tax_cents, summary, COALESCE(description, '') AS description, \
COALESCE(description_lexical, '') AS description_lexical, COALESCE(description_html, '') AS description_html, image, \
COALESCE(gallery, '') AS gallery, bedrooms, bathrooms, half_baths, rooms, parking, area, lot_area, year_built, floors, energy_rating, \
available_from, address, neighborhood, city, region, postal_code, country, latitude, longitude, hide_address, features, \
COALESCE(extra_features, '') AS extra_features, COALESCE(floor_plans, '') AS floor_plans, COALESCE(open_houses, '') AS open_houses, \
video_url, tour_url, brochure_url, featured, keywords, seo_title, seo_description, views, enquiries, published_at, closed_at, created_at, updated_at";

pub async fn find(pool: &Db, id: &str) -> Option<Property> {
    sqlx::query_as::<_, Property>(&format!("SELECT {COLS} FROM properties WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn by_slug(pool: &Db, slug: &str) -> Option<Property> {
    sqlx::query_as::<_, Property>(&format!("SELECT {COLS} FROM properties WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

/// A slug no other property has: `villa`, then `villa-2`, `villa-3`…
pub async fn unique_slug(pool: &Db, wanted: &str, except: Option<&str>) -> String {
    let base = match wanted {
        "" => "property".to_string(),
        w if rules::RESERVED_SLUGS.contains(&w) => format!("{w}-property"),
        w => w.to_string(),
    };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM properties WHERE slug = ? AND id <> ?").bind(&candidate).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

fn plans_json(i: &PropertyInput) -> String {
    serde_json::to_string(&i.floor_plans).unwrap_or_else(|_| "[]".into())
}

fn opens_json(i: &PropertyInput) -> String {
    serde_json::to_string(&i.open_houses).unwrap_or_else(|_| "[]".into())
}

/// Create a property. Returns its id.
pub async fn insert(pool: &Db, i: &PropertyInput) -> Result<String> {
    touch();
    let id = ids::new_id();
    let now = dates::now_db();
    let slug = unique_slug(pool, &i.slug, None).await;
    let published = (i.status == "published").then(|| now.clone());
    let closed = Market::from_key(&i.market).closed().then(|| now.clone());
    sqlx::query(
        "INSERT INTO properties (id, slug, title, status, purpose, market, type_id, agent_id, reference, price_cents, price_poa, price_prefix, rent_period, \
         deposit_cents, service_charge_cents, tax_cents, summary, description, description_lexical, description_html, image, gallery, bedrooms, bathrooms, \
         half_baths, rooms, parking, area, lot_area, year_built, floors, energy_rating, available_from, address, neighborhood, city, region, postal_code, \
         country, latitude, longitude, hide_address, features, extra_features, floor_plans, open_houses, video_url, tour_url, brochure_url, featured, \
         keywords, seo_title, seo_description, views, enquiries, published_at, closed_at, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,0,?,?,?,?)",
    )
    .bind(&id).bind(&slug).bind(&i.title).bind(&i.status).bind(&i.purpose).bind(&i.market).bind(i.type_id.clone()).bind(i.agent_id.clone())
    .bind(&i.reference).bind(i.price_cents).bind(i64::from(i.price_poa)).bind(&i.price_prefix).bind(&i.rent_period)
    .bind(i.deposit_cents).bind(i.service_charge_cents).bind(i.tax_cents).bind(&i.summary).bind(&i.description).bind(&i.description_lexical)
    .bind(&i.description_html).bind(&i.image).bind(i.gallery.join("\n")).bind(i.bedrooms).bind(i.bathrooms).bind(i.half_baths).bind(i.rooms)
    .bind(i.parking).bind(i.area).bind(i.lot_area).bind(i.year_built).bind(i.floors).bind(&i.energy_rating).bind(&i.available_from)
    .bind(&i.address).bind(&i.neighborhood).bind(&i.city).bind(&i.region).bind(&i.postal_code).bind(&i.country).bind(&i.latitude)
    .bind(&i.longitude).bind(i64::from(i.hide_address)).bind(i.features.join(",")).bind(&i.extra_features).bind(plans_json(i)).bind(opens_json(i))
    .bind(&i.video_url).bind(&i.tour_url).bind(&i.brochure_url).bind(i64::from(i.featured)).bind(&i.keywords).bind(&i.seo_title)
    .bind(&i.seo_description).bind(published).bind(closed).bind(&now).bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

/// Save a property. The first time it is published, `published_at` is set;
/// the first time it is sold or let, `closed_at`.
pub async fn update(pool: &Db, id: &str, i: &PropertyInput) -> Result<()> {
    touch();
    let now = dates::now_db();
    let slug = unique_slug(pool, &i.slug, Some(id)).await;
    let closed = Market::from_key(&i.market).closed();
    sqlx::query(
        "UPDATE properties SET slug = ?, title = ?, status = ?, purpose = ?, market = ?, type_id = ?, agent_id = ?, reference = ?, price_cents = ?, \
         price_poa = ?, price_prefix = ?, rent_period = ?, deposit_cents = ?, service_charge_cents = ?, tax_cents = ?, summary = ?, description = ?, \
         description_lexical = ?, description_html = ?, image = ?, gallery = ?, bedrooms = ?, bathrooms = ?, half_baths = ?, rooms = ?, parking = ?, \
         area = ?, lot_area = ?, year_built = ?, floors = ?, energy_rating = ?, available_from = ?, address = ?, neighborhood = ?, city = ?, region = ?, \
         postal_code = ?, country = ?, latitude = ?, longitude = ?, hide_address = ?, features = ?, extra_features = ?, floor_plans = ?, open_houses = ?, \
         video_url = ?, tour_url = ?, brochure_url = ?, featured = ?, keywords = ?, seo_title = ?, seo_description = ?, \
         published_at = CASE WHEN ? = 'published' AND published_at IS NULL THEN ? ELSE published_at END, \
         closed_at = CASE WHEN ? = 1 THEN COALESCE(closed_at, ?) ELSE NULL END, updated_at = ? WHERE id = ?",
    )
    .bind(&slug).bind(&i.title).bind(&i.status).bind(&i.purpose).bind(&i.market).bind(i.type_id.clone()).bind(i.agent_id.clone())
    .bind(&i.reference).bind(i.price_cents).bind(i64::from(i.price_poa)).bind(&i.price_prefix).bind(&i.rent_period)
    .bind(i.deposit_cents).bind(i.service_charge_cents).bind(i.tax_cents).bind(&i.summary).bind(&i.description).bind(&i.description_lexical)
    .bind(&i.description_html).bind(&i.image).bind(i.gallery.join("\n")).bind(i.bedrooms).bind(i.bathrooms).bind(i.half_baths).bind(i.rooms)
    .bind(i.parking).bind(i.area).bind(i.lot_area).bind(i.year_built).bind(i.floors).bind(&i.energy_rating).bind(&i.available_from)
    .bind(&i.address).bind(&i.neighborhood).bind(&i.city).bind(&i.region).bind(&i.postal_code).bind(&i.country).bind(&i.latitude)
    .bind(&i.longitude).bind(i64::from(i.hide_address)).bind(i.features.join(",")).bind(&i.extra_features).bind(plans_json(i)).bind(opens_json(i))
    .bind(&i.video_url).bind(&i.tour_url).bind(&i.brochure_url).bind(i64::from(i.featured)).bind(&i.keywords).bind(&i.seo_title)
    .bind(&i.seo_description).bind(&i.status).bind(&now).bind(i64::from(closed)).bind(&now).bind(&now).bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// The form a stored property would submit — for duplicating and for
/// one-field changes from the list.
pub fn input_of(p: &Property) -> PropertyInput {
    PropertyInput {
        title: p.title.clone(),
        slug: p.slug.clone(),
        status: p.status.clone(),
        purpose: p.purpose.clone(),
        market: p.market.clone(),
        type_id: p.type_id.clone(),
        agent_id: p.agent_id.clone(),
        reference: p.reference.clone(),
        price_cents: p.price_cents,
        price_poa: p.price_poa != 0,
        price_prefix: p.price_prefix.clone(),
        rent_period: p.rent_period.clone(),
        deposit_cents: p.deposit_cents,
        service_charge_cents: p.service_charge_cents,
        tax_cents: p.tax_cents,
        summary: p.summary.clone(),
        description: p.description.clone(),
        description_lexical: p.description_lexical.clone(),
        description_html: p.description_html.clone(),
        image: p.image.clone(),
        gallery: p.gallery_list(),
        bedrooms: p.bedrooms,
        bathrooms: p.bathrooms,
        half_baths: p.half_baths,
        rooms: p.rooms,
        parking: p.parking,
        area: p.area,
        lot_area: p.lot_area,
        year_built: p.year_built,
        floors: p.floors,
        energy_rating: p.energy_rating.clone(),
        available_from: p.available_from.clone(),
        address: p.address.clone(),
        neighborhood: p.neighborhood.clone(),
        city: p.city.clone(),
        region: p.region.clone(),
        postal_code: p.postal_code.clone(),
        country: p.country.clone(),
        latitude: p.latitude.clone(),
        longitude: p.longitude.clone(),
        hide_address: p.hide_address != 0,
        features: p.feature_keys().into_iter().map(str::to_string).collect(),
        extra_features: p.extra_features.clone(),
        floor_plans: p.floor_plan_list(),
        open_houses: p.open_house_list(),
        video_url: p.video_url.clone(),
        tour_url: p.tour_url.clone(),
        brochure_url: p.brochure_url.clone(),
        featured: p.featured != 0,
        keywords: p.keywords.clone(),
        seo_title: p.seo_title.clone(),
        seo_description: p.seo_description.clone(),
    }
}

/// A copy as a draft, "(copy)" in the title, no views or enquiries.
pub async fn duplicate(pool: &Db, id: &str) -> Result<Option<String>> {
    let Some(p) = find(pool, id).await else { return Ok(None) };
    let mut i = input_of(&p);
    i.title = format!("{} (copy)", p.title).chars().take(rules::MAX_TITLE).collect();
    i.slug = vuuut_core::directory::slugify(&i.title);
    i.status = "draft".into();
    i.featured = false;
    insert(pool, &i).await.map(Some)
}

/// Change only the publishing status and/or the market status.
pub async fn set_state(pool: &Db, id: &str, status: Option<&str>, market: Option<&str>) -> Result<()> {
    let Some(p) = find(pool, id).await else { return Ok(()) };
    let mut i = input_of(&p);
    if let Some(s) = status {
        i.status = rules::Status::from_key(s).key().into();
    }
    if let Some(m) = market {
        i.market = Market::from_key(m).key().into();
    }
    update(pool, id, &i).await
}

/// The property goes; its enquiries stay (they are somebody's message),
/// marked with no property.
pub async fn delete(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE property_enquiries SET property_id = NULL WHERE property_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM properties WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn add_view(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE properties SET views = views + 1 WHERE id = ?").bind(id).execute(pool).await;
}

/// A search over properties. `public` limits it to what visitors may see.
#[derive(Debug, Clone, Default)]
pub struct Browse<'a> {
    pub terms: &'a [String],
    pub status: Option<&'a str>,
    pub purpose: Option<&'a str>,
    pub market: Option<&'a str>,
    pub type_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub city: Option<&'a str>,
    pub min_price: i64,
    pub max_price: i64,
    pub beds: i64,
    pub baths: i64,
    pub min_area: i64,
    pub features: &'a [&'a str],
    pub featured_only: bool,
    /// Published only, and sold/let ones only when `show_sold`.
    pub public: bool,
    pub show_sold: bool,
    /// One of `vuuut_core::properties::SORTS`, or `title`/`updated` for staff.
    pub sort: &'a str,
    pub limit: i64,
    pub offset: i64,
}

fn where_clause(b: &Browse) -> (String, Vec<Bind>) {
    let mut sql = String::from(" WHERE 1 = 1");
    let mut binds: Vec<Bind> = Vec::new();
    for t in b.terms {
        let like = format!("%{}%", t.replace(['%', '_'], ""));
        sql.push_str(" AND (LOWER(title) LIKE ? OR LOWER(summary) LIKE ? OR LOWER(keywords) LIKE ? OR LOWER(city) LIKE ? OR LOWER(neighborhood) LIKE ? OR LOWER(reference) LIKE ? OR LOWER(postal_code) LIKE ?)");
        for _ in 0..7 {
            binds.push(Bind::S(like.clone()));
        }
    }
    if b.public {
        sql.push_str(" AND status = 'published'");
        if !b.show_sold {
            sql.push_str(" AND market NOT IN ('sold','let')");
        }
    } else if let Some(s) = b.status {
        sql.push_str(" AND status = ?");
        binds.push(Bind::S(s.into()));
    }
    for (col, v) in [("purpose", b.purpose), ("market", b.market), ("type_id", b.type_id), ("agent_id", b.agent_id)] {
        if let Some(v) = v {
            sql.push_str(&format!(" AND {col} = ?"));
            binds.push(Bind::S(v.into()));
        }
    }
    if let Some(c) = b.city {
        sql.push_str(" AND LOWER(city) = ?");
        binds.push(Bind::S(c.to_lowercase()));
    }
    if b.min_price > 0 {
        sql.push_str(" AND price_poa = 0 AND price_cents >= ?");
        binds.push(Bind::I(b.min_price));
    }
    if b.max_price > 0 {
        sql.push_str(" AND price_poa = 0 AND price_cents > 0 AND price_cents <= ?");
        binds.push(Bind::I(b.max_price));
    }
    for (col, v) in [("bedrooms", b.beds), ("bathrooms", b.baths), ("area", b.min_area)] {
        if v > 0 {
            sql.push_str(&format!(" AND {col} >= ?"));
            binds.push(Bind::I(v));
        }
    }
    for f in b.features {
        // Whole keys only, so `pool` never matches `pool_house`; spelt out
        // rather than concatenated because `||` is not MySQL's operator.
        sql.push_str(" AND (features = ? OR features LIKE ? OR features LIKE ? OR features LIKE ?)");
        for pattern in [f.to_string(), format!("{f},%"), format!("%,{f}"), format!("%,{f},%")] {
            binds.push(Bind::S(pattern));
        }
    }
    if b.featured_only {
        sql.push_str(" AND featured = 1");
    }
    (sql, binds)
}

fn order_by(sort: &str) -> &'static str {
    match sort {
        "newest" => "COALESCE(published_at, created_at) DESC, id DESC",
        "price_asc" => "price_poa ASC, price_cents ASC, id DESC",
        "price_desc" => "price_poa ASC, price_cents DESC, id DESC",
        "area_desc" => "area DESC, id DESC",
        "beds_desc" => "bedrooms DESC, id DESC",
        "title" => "title ASC, id ASC",
        "updated" => "updated_at DESC, id DESC",
        "views" => "views DESC, id DESC",
        // Available before under offer before sold, featured first, newest.
        _ => "CASE market WHEN 'available' THEN 0 WHEN 'under_offer' THEN 1 ELSE 2 END, featured DESC, COALESCE(published_at, created_at) DESC, id DESC",
    }
}

/// One page of matches and the total.
pub async fn browse(pool: &Db, b: &Browse<'_>) -> Result<(Vec<Property>, i64)> {
    let (clause, binds) = where_clause(b);
    let count_sql = format!("SELECT COUNT(*) FROM properties{clause}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql);
    for v in &binds {
        count = match v {
            Bind::S(s) => count.bind(s.clone()),
            Bind::I(n) => count.bind(*n),
        };
    }
    let total = count.fetch_one(pool).await.map_err(dberr)?;
    let sql = format!("SELECT {COLS} FROM properties{clause} ORDER BY {} LIMIT ? OFFSET ?", order_by(b.sort));
    let mut q = sqlx::query_as::<_, Property>(&sql);
    for v in &binds {
        q = match v {
            Bind::S(s) => q.bind(s.clone()),
            Bind::I(n) => q.bind(*n),
        };
    }
    let rows = q.bind(b.limit.clamp(1, 1000)).bind(b.offset.max(0)).fetch_all(pool).await.map_err(dberr)?;
    Ok((rows, total))
}

/// Published properties among `ids`, in the order asked — saved and compare.
pub async fn published_by_ids(pool: &Db, ids: &[String]) -> Vec<Property> {
    if ids.is_empty() {
        return Vec::new();
    }
    let marks = vec!["?"; ids.len()].join(",");
    let sql = format!("SELECT {COLS} FROM properties WHERE status = 'published' AND id IN ({marks})");
    let mut q = sqlx::query_as::<_, Property>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await.unwrap_or_default();
    ids.iter().filter_map(|id| rows.iter().find(|p| &p.id == id).cloned()).collect()
}

/// Cities with published properties, and how many.
pub async fn cities(pool: &Db) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT city, COUNT(*) FROM properties WHERE status = 'published' AND city <> '' GROUP BY city ORDER BY COUNT(*) DESC, city LIMIT 200")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// `status:…`, `purpose:…`, `market:…` → how many, for the overview.
pub async fn counts(pool: &Db) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for col in ["status", "purpose", "market"] {
        let sql = format!("SELECT {col}, COUNT(*) FROM properties GROUP BY {col}");
        for (k, n) in sqlx::query_as::<_, (String, i64)>(&sql).fetch_all(pool).await.unwrap_or_default() {
            out.insert(format!("{col}:{k}"), n);
        }
    }
    // What the public sees, by purpose.
    for (k, n) in sqlx::query_as::<_, (String, i64)>("SELECT purpose, COUNT(*) FROM properties WHERE status = 'published' AND market IN ('available','under_offer') GROUP BY purpose").fetch_all(pool).await.unwrap_or_default() {
        out.insert(format!("live:{k}"), n);
    }
    out
}

/// Sold or let since `since`, and their total value (sales only).
pub async fn closed_since(pool: &Db, since: &str) -> (i64, i64) {
    sqlx::query_as::<_, (i64, i64)>("SELECT COUNT(*), COALESCE(SUM(CASE WHEN purpose = 'sale' THEN price_cents ELSE 0 END), 0) FROM properties WHERE closed_at IS NOT NULL AND closed_at >= ?")
        .bind(since)
        .fetch_one(pool)
        .await
        .unwrap_or((0, 0))
}

pub async fn total_views(pool: &Db) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(views), 0) FROM properties").fetch_one(pool).await.unwrap_or(0)
}

/// The median asking price of available sales, for the overview.
pub async fn sale_prices(pool: &Db) -> Vec<i64> {
    sqlx::query_scalar::<_, i64>("SELECT price_cents FROM properties WHERE status = 'published' AND purpose = 'sale' AND market = 'available' AND price_poa = 0 AND price_cents > 0 ORDER BY price_cents")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// enquiries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Enquiry {
    pub id: String,
    pub property_id: Option<String>,
    pub agent_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub message: String,
    pub preferred_date: String,
    pub preferred_time: String,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
    /// The property's title, or empty when it has been deleted.
    pub property_title: String,
    pub property_slug: String,
}

const ENQUIRY_COLS: &str = "e.id, e.property_id, e.agent_id, e.kind, e.status, e.name, e.email, e.phone, COALESCE(e.message, '') AS message, \
e.preferred_date, e.preferred_time, COALESCE(e.note, '') AS note, e.created_at, e.updated_at, \
COALESCE(p.title, '') AS property_title, COALESCE(p.slug, '') AS property_slug";

/// Store an enquiry and count it on its property. Returns its id.
pub async fn insert_enquiry(pool: &Db, property: Option<&Property>, agent_id: Option<&str>, e: &EnquiryInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query("INSERT INTO property_enquiries (id, property_id, agent_id, kind, status, name, email, phone, message, preferred_date, preferred_time, note, created_at, updated_at) VALUES (?,?,?,?,'new',?,?,?,?,?,?,'',?,?)")
        .bind(&id).bind(property.map(|p| p.id.clone())).bind(agent_id.map(str::to_string)).bind(&e.kind).bind(&e.name).bind(&e.email).bind(&e.phone)
        .bind(&e.message).bind(&e.preferred_date).bind(&e.preferred_time).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    if let Some(p) = property {
        sqlx::query("UPDATE properties SET enquiries = enquiries + 1 WHERE id = ?").bind(&p.id).execute(pool).await.map_err(dberr)?;
    }
    Ok(id)
}

#[derive(Debug, Clone, Default)]
pub struct EnquiryQuery<'a> {
    pub status: Option<&'a str>,
    pub property_id: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn browse_enquiries(pool: &Db, q: &EnquiryQuery<'_>) -> (Vec<Enquiry>, i64) {
    let mut clause = String::from(" WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    match q.status {
        Some(s) => {
            clause.push_str(" AND e.status = ?");
            binds.push(s.into());
        }
        // Spam is out of sight unless asked for.
        None => clause.push_str(" AND e.status <> 'spam'"),
    }
    if let Some(p) = q.property_id {
        clause.push_str(" AND e.property_id = ?");
        binds.push(p.into());
    }
    if let Some(k) = q.kind {
        clause.push_str(" AND e.kind = ?");
        binds.push(k.into());
    }
    let count_sql = format!("SELECT COUNT(*) FROM property_enquiries e{clause}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &binds {
        count = count.bind(b.clone());
    }
    let total = count.fetch_one(pool).await.unwrap_or(0);
    let sql = format!("SELECT {ENQUIRY_COLS} FROM property_enquiries e LEFT JOIN properties p ON p.id = e.property_id{clause} ORDER BY e.created_at DESC, e.id DESC LIMIT ? OFFSET ?");
    let mut rows = sqlx::query_as::<_, Enquiry>(&sql);
    for b in &binds {
        rows = rows.bind(b.clone());
    }
    let rows = rows.bind(q.limit.clamp(1, 5000)).bind(q.offset.max(0)).fetch_all(pool).await.unwrap_or_default();
    (rows, total)
}

pub async fn find_enquiry(pool: &Db, id: &str) -> Option<Enquiry> {
    sqlx::query_as::<_, Enquiry>(&format!("SELECT {ENQUIRY_COLS} FROM property_enquiries e LEFT JOIN properties p ON p.id = e.property_id WHERE e.id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub async fn save_enquiry(pool: &Db, id: &str, status: &str, note: &str) -> Result<()> {
    sqlx::query("UPDATE property_enquiries SET status = ?, note = ?, updated_at = ? WHERE id = ?")
        .bind(rules::enquiry_status(status)).bind(note).bind(dates::now_db()).bind(id)
        .execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_enquiry(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM property_enquiries WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn enquiry_counts(pool: &Db) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM property_enquiries GROUP BY status").fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

/// Enquiries from one address since `since` — the per-address daily cap.
pub async fn enquiries_since(pool: &Db, email: &str, since: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM property_enquiries WHERE LOWER(email) = ? AND created_at >= ?")
        .bind(email.to_lowercase())
        .bind(since)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// Enquiries per day since `since` (`YYYY-MM-DD`, n).
pub async fn enquiries_per_day(pool: &Db, since: &str) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT SUBSTR(created_at, 1, 10) AS d, COUNT(*) FROM property_enquiries WHERE created_at >= ? AND status <> 'spam' GROUP BY SUBSTR(created_at, 1, 10)")
        .bind(since)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// as themes see them
// ---------------------------------------------------------------------------

pub fn type_json(s: &Settings, t: &PropertyType, count: i64) -> Value {
    json!({
        "id": t.id,
        "name": t.name,
        "slug": t.slug,
        "description": t.description,
        "feature_image": if t.image.is_empty() { Value::Null } else { json!(t.image) },
        "tint": t.tint,
        "count": count,
        "url": public_url(s, &format!("/properties/type/{}/", t.slug)),
    })
}

/// An agent as a theme sees them. The email address and phone numbers are
/// left out when the site has chosen not to show them.
pub fn agent_json(s: &Settings, a: &Agent, count: i64) -> Value {
    let contact = s.show_agent_contact;
    json!({
        "id": a.id,
        "name": a.name,
        "slug": a.slug,
        "title": a.title,
        "agency": a.agency,
        "initials": a.initials(),
        "photo": if a.photo.is_empty() { Value::Null } else { json!(a.photo) },
        "bio": a.bio,
        "website": if a.website.is_empty() { Value::Null } else { json!(a.website) },
        "license": a.license,
        "languages": a.languages,
        "email": if contact && !a.email.is_empty() { json!(a.email) } else { Value::Null },
        "phone": if contact && !a.phone.is_empty() { json!(a.phone) } else { Value::Null },
        "mobile": if contact && !a.mobile.is_empty() { json!(a.mobile) } else { Value::Null },
        "phone_url": if contact && !a.phone.is_empty() { json!(format!("tel:{}", a.phone.replace(' ', ""))) } else { Value::Null },
        "count": count,
        "url": public_url(s, &format!("/properties/agent/{}/", a.slug)),
    })
}

/// OpenStreetMap's embeddable map around a point.
pub fn map_embed(lat: f64, lng: f64, marker: bool) -> String {
    let m = if marker { format!("&marker={lat},{lng}") } else { String::new() };
    format!("https://www.openstreetmap.org/export/embed.html?bbox={},{},{},{}&layer=mapnik{m}", lng - 0.008, lat - 0.005, lng + 0.008, lat + 0.005)
}

/// A property as a theme sees it (`properties/*.hbs`, `{{#get "properties"}}`).
/// Every value is data, which Handlebars escapes, except `description`, the
/// HTML the editor rendered.
pub fn property_json(s: &Settings, p: &Property, types: &[PropertyType], agents: &[Agent]) -> Value {
    let t = p.type_id.as_deref().and_then(|id| types.iter().find(|t| t.id == id));
    let a = p.agent_id.as_deref().and_then(|id| agents.iter().find(|a| a.id == id && a.is_active()));
    let images = p.images();
    let unit = s.area_unit;
    let purpose = p.purpose();
    let market = p.market();
    let coords = p.public_coords();
    let feats: Vec<Value> = p.feature_keys().iter().filter_map(|k| rules::FEATURES.iter().find(|f| f.0 == *k)).map(|f| json!({ "key": f.0, "label": f.1, "group": f.2 })).collect();
    let opens: Vec<Value> = p
        .open_house_list()
        .iter()
        .filter(|o| o.date.as_str() >= dates::now_db().get(..10).unwrap_or(""))
        .map(|o| {
            let day = chrono::NaiveDate::parse_from_str(&o.date, "%Y-%m-%d").map(|d| d.format("%a %-d %b").to_string()).unwrap_or_default();
            let time = match (o.start.as_str(), o.end.as_str()) {
                ("", _) => String::new(),
                (a, "") => a.to_string(),
                (a, b) => format!("{a}–{b}"),
            };
            json!({ "date": o.date, "day": day, "start": o.start, "end": o.end, "time": time })
        })
        .collect();
    let plans: Vec<Value> = p.floor_plan_list().iter().map(|f| json!({ "title": f.title, "image": f.image, "size": f.size })).collect();
    let url = public_url(s, &format!("/properties/{}/", p.slug));
    let plain: String = if p.summary.is_empty() { p.description.chars().take(180).collect() } else { p.summary.clone() };
    let head = json!({
        "id": p.id,
        "slug": p.slug,
        "title": p.title,
        "url": url,
        "catalogue_url": public_url(s, "/properties/"),
        "purpose": purpose.key(),
        "purpose_label": purpose.label(),
        "for_sale": purpose == Purpose::Sale,
        "for_rent": purpose == Purpose::Rent,
        "market": market.key(),
        "market_label": market.label(),
        "badge": market.badge(purpose),
        "closed": market.closed(),
        "under_offer": market == Market::UnderOffer,
        "featured": p.is_featured(),
        "reference": p.reference,
        "price": p.price_label(&s.currency),
        "price_amount": if p.price_poa != 0 || p.price_cents <= 0 { Value::Null } else { json!(p.price_cents as f64 / 100.0) },
        "price_poa": p.price_poa != 0 || p.price_cents <= 0,
        "rent_period": p.rent_period,
        "currency": s.currency,
        "deposit": if p.deposit_cents > 0 { json!(rules::amount(p.deposit_cents, &s.currency)) } else { Value::Null },
        "service_charge": if p.service_charge_cents > 0 { json!(rules::amount(p.service_charge_cents, &s.currency)) } else { Value::Null },
        "tax": if p.tax_cents > 0 { json!(rules::amount(p.tax_cents, &s.currency)) } else { Value::Null },
        "type": t.map(|t| json!({ "name": t.name, "slug": t.slug, "url": public_url(s, &format!("/properties/type/{}/", t.slug)) })),
        "agent": a.map(|a| agent_json(s, a, 0)),
        "summary": p.summary,
        "custom_excerpt": p.summary,
        "excerpt": plain,
        "description": p.description_markup(),
        "feature_image": images.first(),
        "images": images,
        "gallery": images.iter().skip(1).collect::<Vec<_>>(),
        "photo_count": images.len(),
    });
    let more = json!({
        "bedrooms": p.bedrooms,
        "bathrooms": p.bathrooms,
        "half_baths": p.half_baths,
        "baths": p.baths_label(),
        "rooms": p.rooms,
        "parking": p.parking,
        "area": p.area,
        "area_label": rules::area_label(p.area, unit),
        "lot_area": p.lot_area,
        "lot_area_label": rules::area_label(p.lot_area, unit),
        "area_unit": unit.label(),
        "year_built": if p.year_built > 0 { json!(p.year_built) } else { Value::Null },
        "floors": p.floors,
        "energy_rating": p.energy_rating,
        "available_from": p.available_from,
    });
    let place = json!({
        "address": if p.hide_address != 0 { String::new() } else { p.address.clone() },
        "address_hidden": p.hide_address != 0,
        "neighborhood": p.neighborhood,
        "city": p.city,
        "city_url": if p.city.is_empty() { Value::Null } else { json!(format!("{}?city={}", public_url(s, "/properties/"), vuuut_core::social_media::encode(&p.city))) },
        "region": p.region,
        "postal_code": p.postal_code,
        "country": p.country,
        "location": p.short_location(),
        "full_address": p.public_address(),
        "latitude": coords.map(|c| c.0),
        "longitude": coords.map(|c| c.1),
        "map_embed_url": coords.map(|(lat, lng)| map_embed(lat, lng, p.hide_address == 0)),
        "directions_url": coords.map(|(lat, lng)| format!("https://www.google.com/maps/search/?api=1&query={lat},{lng}")),
        "features": feats,
        "extra_features": p.extra_list(),
        "floor_plans": plans,
        "open_houses": opens,
        "video_url": if p.video_url.is_empty() { Value::Null } else { json!(p.video_url) },
        "video_embed": vuuut_core::lms::video_embed(&p.video_url).map(|(u, frame)| json!({ "url": u, "iframe": frame })),
        "tour_url": if p.tour_url.is_empty() { Value::Null } else { json!(p.tour_url) },
        "tour_embed": rules::tour_embed(&p.tour_url),
        "brochure_url": if p.brochure_url.is_empty() { Value::Null } else { json!(vuuut_core::url::base_asset(&p.brochure_url)) },
        "views": p.views,
        "published_at": p.published_at,
        "updated_at": p.updated_at,
    });
    // Three literals because one is past `json!`'s recursion limit.
    let mut out = head;
    for part in [more, place] {
        if let (Some(o), Value::Object(m)) = (out.as_object_mut(), part) {
            o.extend(m);
        }
    }
    out
}
