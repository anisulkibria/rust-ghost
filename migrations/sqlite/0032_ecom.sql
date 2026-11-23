-- ============================================================================
-- 0032_ecom.sql — Tools → E-commerce & Online Stores
-- ============================================================================
--
-- The store: a catalogue, the people who buy from it and what they bought.
--
-- `ecom_products` — one product. Its description is edited in Koenig, so the
--   Lexical document is kept (to reopen) beside the HTML it renders to (to
--   show). `options` is JSON: up to three `{name, values[]}`; `images` is a
--   JSON array of URLs, the first one the product's main image; `tags` is a
--   comma-separated list.
--   status   active | draft | archived — only active is on the storefront
--
-- `ecom_variants` — what is actually sold. Every product has at least one;
--   a product without options has exactly one, titled "Default Title".
--   Money is integer minor units (cents). `inventory` only means anything
--   when `track_inventory` is on.
--
-- `ecom_collections` + `ecom_collection_products` — groupings. A manual
--   collection lists its products; a smart one (`kind = 'smart'`) holds
--   every product matching `rules` (JSON), and the join table is unused.
--
-- `ecom_customers` — keyed on email. Linked to a member when one exists.
--   Totals are computed from orders, never stored, so they cannot drift.
--
-- `ecom_orders` + `ecom_order_items` + `ecom_order_events` — an order is a
--   frozen copy: titles, prices and addresses are copied in at checkout, so
--   editing or deleting a product never rewrites history. The events table
--   is the order's timeline.
--   financial_status   pending | paid | partially_refunded | refunded | voided
--   fulfillment_status unfulfilled | fulfilled
--   status             open | archived | cancelled
--
-- `ecom_discounts` — codes. kind percent (value = whole percent) | fixed
--   (value = cents) | free_shipping.
--
-- `ecom_shipping_rates` — what checkout offers, filtered by country and by
--   the order's subtotal.
--
-- `ecom_carts` — a visitor's cart, keyed by the random token in its cookie.
--   One with an email and no order after an hour is an abandoned checkout.
--
-- The settings ship with the storefront OFF: installing this adds no page
-- to the site.
CREATE TABLE IF NOT EXISTS ecom_products (
  id                  TEXT PRIMARY KEY,
  handle              TEXT NOT NULL UNIQUE,
  title               TEXT NOT NULL,
  status              TEXT NOT NULL DEFAULT 'draft',
  description_lexical TEXT NOT NULL DEFAULT '',
  description_html    TEXT NOT NULL DEFAULT '',
  excerpt             TEXT NOT NULL DEFAULT '',
  vendor              TEXT NOT NULL DEFAULT '',
  product_type        TEXT NOT NULL DEFAULT '',
  tags                TEXT NOT NULL DEFAULT '',
  images              TEXT NOT NULL DEFAULT '[]',
  options             TEXT NOT NULL DEFAULT '[]',
  seo_title           TEXT NOT NULL DEFAULT '',
  seo_description     TEXT NOT NULL DEFAULT '',
  requires_shipping   INTEGER NOT NULL DEFAULT 1,
  taxable             INTEGER NOT NULL DEFAULT 1,
  featured            INTEGER NOT NULL DEFAULT 0,
  published_at        TEXT,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_products_status_idx ON ecom_products (status, published_at);

CREATE TABLE IF NOT EXISTS ecom_variants (
  id               TEXT PRIMARY KEY,
  product_id       TEXT NOT NULL,
  title            TEXT NOT NULL DEFAULT 'Default Title',
  option1          TEXT NOT NULL DEFAULT '',
  option2          TEXT NOT NULL DEFAULT '',
  option3          TEXT NOT NULL DEFAULT '',
  sku              TEXT NOT NULL DEFAULT '',
  barcode          TEXT NOT NULL DEFAULT '',
  price_cents      INTEGER NOT NULL DEFAULT 0,
  compare_at_cents INTEGER NOT NULL DEFAULT 0,
  cost_cents       INTEGER NOT NULL DEFAULT 0,
  track_inventory  INTEGER NOT NULL DEFAULT 1,
  inventory        INTEGER NOT NULL DEFAULT 0,
  allow_backorder  INTEGER NOT NULL DEFAULT 0,
  weight_grams     INTEGER NOT NULL DEFAULT 0,
  image            TEXT NOT NULL DEFAULT '',
  position         INTEGER NOT NULL DEFAULT 0,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_variants_product_idx ON ecom_variants (product_id, position);

CREATE TABLE IF NOT EXISTS ecom_collections (
  id              TEXT PRIMARY KEY,
  handle          TEXT NOT NULL UNIQUE,
  title           TEXT NOT NULL,
  description     TEXT NOT NULL DEFAULT '',
  image           TEXT NOT NULL DEFAULT '',
  kind            TEXT NOT NULL DEFAULT 'manual',
  rules           TEXT NOT NULL DEFAULT '[]',
  match_all       INTEGER NOT NULL DEFAULT 1,
  sort            TEXT NOT NULL DEFAULT 'manual',
  published       INTEGER NOT NULL DEFAULT 1,
  seo_title       TEXT NOT NULL DEFAULT '',
  seo_description TEXT NOT NULL DEFAULT '',
  position        INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_collection_products (
  collection_id TEXT NOT NULL,
  product_id    TEXT NOT NULL,
  position      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (collection_id, product_id)
);
CREATE INDEX IF NOT EXISTS ecom_collection_products_product_idx ON ecom_collection_products (product_id);

CREATE TABLE IF NOT EXISTS ecom_customers (
  id                TEXT PRIMARY KEY,
  email             TEXT NOT NULL UNIQUE,
  first_name        TEXT NOT NULL DEFAULT '',
  last_name         TEXT NOT NULL DEFAULT '',
  phone             TEXT NOT NULL DEFAULT '',
  accepts_marketing INTEGER NOT NULL DEFAULT 0,
  note              TEXT NOT NULL DEFAULT '',
  tags              TEXT NOT NULL DEFAULT '',
  address           TEXT NOT NULL DEFAULT '{}',
  member_id         TEXT,
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_orders (
  id                 TEXT PRIMARY KEY,
  number             INTEGER NOT NULL UNIQUE,
  token              TEXT NOT NULL UNIQUE,
  customer_id        TEXT,
  email              TEXT NOT NULL DEFAULT '',
  phone              TEXT NOT NULL DEFAULT '',
  status             TEXT NOT NULL DEFAULT 'open',
  financial_status   TEXT NOT NULL DEFAULT 'pending',
  fulfillment_status TEXT NOT NULL DEFAULT 'unfulfilled',
  currency           TEXT NOT NULL DEFAULT 'USD',
  subtotal_cents     INTEGER NOT NULL DEFAULT 0,
  discount_cents     INTEGER NOT NULL DEFAULT 0,
  shipping_cents     INTEGER NOT NULL DEFAULT 0,
  tax_cents          INTEGER NOT NULL DEFAULT 0,
  total_cents        INTEGER NOT NULL DEFAULT 0,
  refunded_cents     INTEGER NOT NULL DEFAULT 0,
  tax_included       INTEGER NOT NULL DEFAULT 0,
  discount_code      TEXT NOT NULL DEFAULT '',
  shipping_method    TEXT NOT NULL DEFAULT '',
  shipping_address   TEXT NOT NULL DEFAULT '{}',
  payment_method     TEXT NOT NULL DEFAULT 'manual',
  payment_ref        TEXT NOT NULL DEFAULT '',
  note               TEXT NOT NULL DEFAULT '',
  staff_note         TEXT NOT NULL DEFAULT '',
  tracking_company   TEXT NOT NULL DEFAULT '',
  tracking_number    TEXT NOT NULL DEFAULT '',
  tracking_url       TEXT NOT NULL DEFAULT '',
  source             TEXT NOT NULL DEFAULT 'web',
  restocked          INTEGER NOT NULL DEFAULT 0,
  paid_at            TEXT,
  fulfilled_at       TEXT,
  cancelled_at       TEXT,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_orders_created_idx ON ecom_orders (created_at);
CREATE INDEX IF NOT EXISTS ecom_orders_customer_idx ON ecom_orders (customer_id);
CREATE INDEX IF NOT EXISTS ecom_orders_email_idx ON ecom_orders (email);

CREATE TABLE IF NOT EXISTS ecom_order_items (
  id            TEXT PRIMARY KEY,
  order_id      TEXT NOT NULL,
  product_id    TEXT,
  variant_id    TEXT,
  title         TEXT NOT NULL,
  variant_title TEXT NOT NULL DEFAULT '',
  sku           TEXT NOT NULL DEFAULT '',
  image         TEXT NOT NULL DEFAULT '',
  unit_cents    INTEGER NOT NULL DEFAULT 0,
  quantity      INTEGER NOT NULL DEFAULT 1,
  total_cents   INTEGER NOT NULL DEFAULT 0,
  requires_shipping INTEGER NOT NULL DEFAULT 1,
  position      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ecom_order_items_order_idx ON ecom_order_items (order_id, position);
CREATE INDEX IF NOT EXISTS ecom_order_items_product_idx ON ecom_order_items (product_id);

CREATE TABLE IF NOT EXISTS ecom_order_events (
  id         TEXT PRIMARY KEY,
  order_id   TEXT NOT NULL,
  kind       TEXT NOT NULL,
  message    TEXT NOT NULL,
  actor      TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_order_events_order_idx ON ecom_order_events (order_id, created_at);

CREATE TABLE IF NOT EXISTS ecom_discounts (
  id                 TEXT PRIMARY KEY,
  code               TEXT NOT NULL UNIQUE,
  kind               TEXT NOT NULL DEFAULT 'percent',
  value              INTEGER NOT NULL DEFAULT 0,
  min_subtotal_cents INTEGER NOT NULL DEFAULT 0,
  usage_limit        INTEGER NOT NULL DEFAULT 0,
  once_per_customer  INTEGER NOT NULL DEFAULT 0,
  used_count         INTEGER NOT NULL DEFAULT 0,
  active             INTEGER NOT NULL DEFAULT 1,
  starts_at          TEXT,
  ends_at            TEXT,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_shipping_rates (
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  price_cents        INTEGER NOT NULL DEFAULT 0,
  min_subtotal_cents INTEGER NOT NULL DEFAULT 0,
  max_subtotal_cents INTEGER NOT NULL DEFAULT 0,
  countries          TEXT NOT NULL DEFAULT '',
  position           INTEGER NOT NULL DEFAULT 0,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_carts (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL DEFAULT '',
  items         TEXT NOT NULL DEFAULT '[]',
  discount_code TEXT NOT NULL DEFAULT '',
  order_id      TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_carts_updated_idx ON ecom_carts (updated_at);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'ecom','ecom_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_store_name','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_intro','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_currency','USD','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_tax_rate','0','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_tax_included','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_tax_shipping','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_pay_manual','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_pay_bank','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_bank_details','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_pay_stripe','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_per_page','24','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_low_stock','5','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_notify_email','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_send_receipts','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_countries','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_require_phone','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_policy_refund','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_policy_shipping','','string',datetime('now'),datetime('now'));
