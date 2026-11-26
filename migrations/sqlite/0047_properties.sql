-- ============================================================================
-- 0047_properties.sql — Tools → Real Estate & Property Listings
-- ============================================================================
--
-- What Estatik, Easy Property Listings and Essential Real Estate have in
-- common: properties for sale or to rent, the types they are grouped by,
-- the agents they are listed by, and the enquiries visitors send.
-- The rules for every column are `vuuut_core::properties`.
--
-- `property_types` — House, Apartment, Land… how the catalogue is browsed.
--
-- `property_agents` — the person (or branch, or landlord) a property is
--   listed by. status: active | hidden (kept, not shown on the site).
--
-- `properties` — one property.
--   status           draft | published | archived  (is it on the site)
--   purpose          sale | rent
--   market           available | under_offer | sold | let  (where it is in
--                    the market; a sold house usually stays on the site)
--   price_cents      hundredths of the site currency; 0 or price_poa = 1
--                    shows "Price on application"
--   price_prefix     "" | Offers over | Guide price | From | …
--   rent_period      month | week | year | night ("" for a sale)
--   deposit_cents, service_charge_cents (a month), tax_cents (a year)
--   gallery          newline-separated photo URLs after the cover `image`
--   area, lot_area   whole numbers in the site's unit (properties_area_unit)
--   half_baths       counted apart, as US listings show them (3.5 baths)
--   features         comma-separated keys from vuuut_core::properties::FEATURES
--   extra_features   free lines the list does not have
--   floor_plans      JSON [{"title","image","size"}]
--   open_houses      JSON [{"date":"YYYY-MM-DD","start":"HH:MM","end":"HH:MM"}]
--   hide_address     show the street to staff only; the city still shows
--   enquiries        how many enquiries this property has had
--
-- `property_enquiries` — what visitors send from a property page.
--   kind             info | viewing
--   status           new | contacted | viewing | closed | spam
--   preferred_date   YYYY-MM-DD for a viewing; preferred_time
--                    morning | afternoon | evening | any
--   This is personal data: staff (Editor and above) only, never public.
--
-- Ships switched off: no public catalogue until somebody turns it on.

CREATE TABLE IF NOT EXISTS property_types (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  image       TEXT NOT NULL DEFAULT '',
  tint        TEXT NOT NULL DEFAULT 'blue',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS property_agents (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL UNIQUE,
  title       TEXT NOT NULL DEFAULT '',
  agency      TEXT NOT NULL DEFAULT '',
  email       TEXT NOT NULL DEFAULT '',
  phone       TEXT NOT NULL DEFAULT '',
  mobile      TEXT NOT NULL DEFAULT '',
  photo       TEXT NOT NULL DEFAULT '',
  bio         TEXT,
  website     TEXT NOT NULL DEFAULT '',
  license     TEXT NOT NULL DEFAULT '',
  languages   TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'active',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS properties (
  id                   TEXT PRIMARY KEY,
  slug                 TEXT NOT NULL UNIQUE,
  title                TEXT NOT NULL,
  status               TEXT NOT NULL DEFAULT 'draft',
  purpose              TEXT NOT NULL DEFAULT 'sale',
  market               TEXT NOT NULL DEFAULT 'available',
  type_id              TEXT,
  agent_id             TEXT,
  reference            TEXT NOT NULL DEFAULT '',
  price_cents          INTEGER NOT NULL DEFAULT 0,
  price_poa            INTEGER NOT NULL DEFAULT 0,
  price_prefix         TEXT NOT NULL DEFAULT '',
  rent_period          TEXT NOT NULL DEFAULT '',
  deposit_cents        INTEGER NOT NULL DEFAULT 0,
  service_charge_cents INTEGER NOT NULL DEFAULT 0,
  tax_cents            INTEGER NOT NULL DEFAULT 0,
  summary              TEXT NOT NULL DEFAULT '',
  description          TEXT,
  description_lexical  TEXT,
  description_html     TEXT,
  image                TEXT NOT NULL DEFAULT '',
  gallery              TEXT,
  bedrooms             INTEGER NOT NULL DEFAULT 0,
  bathrooms            INTEGER NOT NULL DEFAULT 0,
  half_baths           INTEGER NOT NULL DEFAULT 0,
  rooms                INTEGER NOT NULL DEFAULT 0,
  parking              INTEGER NOT NULL DEFAULT 0,
  area                 INTEGER NOT NULL DEFAULT 0,
  lot_area             INTEGER NOT NULL DEFAULT 0,
  year_built           INTEGER NOT NULL DEFAULT 0,
  floors               INTEGER NOT NULL DEFAULT 0,
  energy_rating        TEXT NOT NULL DEFAULT '',
  available_from       TEXT NOT NULL DEFAULT '',
  address              TEXT NOT NULL DEFAULT '',
  neighborhood         TEXT NOT NULL DEFAULT '',
  city                 TEXT NOT NULL DEFAULT '',
  region               TEXT NOT NULL DEFAULT '',
  postal_code          TEXT NOT NULL DEFAULT '',
  country              TEXT NOT NULL DEFAULT '',
  latitude             TEXT NOT NULL DEFAULT '',
  longitude            TEXT NOT NULL DEFAULT '',
  hide_address         INTEGER NOT NULL DEFAULT 0,
  features             TEXT NOT NULL DEFAULT '',
  extra_features       TEXT,
  floor_plans          TEXT,
  open_houses          TEXT,
  video_url            TEXT NOT NULL DEFAULT '',
  tour_url             TEXT NOT NULL DEFAULT '',
  brochure_url         TEXT NOT NULL DEFAULT '',
  featured             INTEGER NOT NULL DEFAULT 0,
  keywords             TEXT NOT NULL DEFAULT '',
  seo_title            TEXT NOT NULL DEFAULT '',
  seo_description      TEXT NOT NULL DEFAULT '',
  views                INTEGER NOT NULL DEFAULT 0,
  enquiries            INTEGER NOT NULL DEFAULT 0,
  published_at         TEXT,
  closed_at            TEXT,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS properties_status_idx ON properties (status, purpose, market);
CREATE INDEX IF NOT EXISTS properties_type_idx ON properties (type_id);
CREATE INDEX IF NOT EXISTS properties_agent_idx ON properties (agent_id);
CREATE INDEX IF NOT EXISTS properties_city_idx ON properties (city);
CREATE INDEX IF NOT EXISTS properties_price_idx ON properties (price_cents);

CREATE TABLE IF NOT EXISTS property_enquiries (
  id             TEXT PRIMARY KEY,
  property_id    TEXT,
  agent_id       TEXT,
  kind           TEXT NOT NULL DEFAULT 'info',
  status         TEXT NOT NULL DEFAULT 'new',
  name           TEXT NOT NULL,
  email          TEXT NOT NULL,
  phone          TEXT NOT NULL DEFAULT '',
  message        TEXT,
  preferred_date TEXT NOT NULL DEFAULT '',
  preferred_time TEXT NOT NULL DEFAULT '',
  note           TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS property_enquiries_status_idx ON property_enquiries (status, created_at);
CREATE INDEX IF NOT EXISTS property_enquiries_property_idx ON property_enquiries (property_id);
CREATE INDEX IF NOT EXISTS property_enquiries_email_idx ON property_enquiries (email);

INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'properties','properties_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_title','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_intro','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_per_page','12','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_area_unit','sqft','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_enquiries','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_notify_email','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_show_agent_contact','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_mortgage','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_mortgage_rate','6.5','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_mortgage_term','25','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_mortgage_deposit','20','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_show_sold','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'properties','properties_base_path','properties','string',datetime('now'),datetime('now'));
