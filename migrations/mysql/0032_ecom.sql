-- ============================================================================
-- 0032_ecom.sql — Tools → E-commerce & Online Stores
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0032_ecom.sql. VARCHAR, not TEXT, for
-- anything read back (sqlx `Any`); flags are SMALLINT; money is BIGINT.
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
  id                  VARCHAR(24) PRIMARY KEY,
  handle              VARCHAR(120) NOT NULL UNIQUE,
  title               VARCHAR(255) NOT NULL,
  status              VARCHAR(20) NOT NULL DEFAULT 'draft',
  description_lexical MEDIUMTEXT NOT NULL,
  description_html    MEDIUMTEXT NOT NULL,
  excerpt             VARCHAR(500) NOT NULL DEFAULT '',
  vendor              VARCHAR(120) NOT NULL DEFAULT '',
  product_type        VARCHAR(120) NOT NULL DEFAULT '',
  tags                VARCHAR(500) NOT NULL DEFAULT '',
  images              VARCHAR(4000) NOT NULL DEFAULT '[]',
  options             VARCHAR(3000) NOT NULL DEFAULT '[]',
  seo_title           VARCHAR(120) NOT NULL DEFAULT '',
  seo_description     VARCHAR(320) NOT NULL DEFAULT '',
  requires_shipping   SMALLINT NOT NULL DEFAULT 1,
  taxable             SMALLINT NOT NULL DEFAULT 1,
  featured            SMALLINT NOT NULL DEFAULT 0,
  published_at        VARCHAR(32) NULL,
  created_at          VARCHAR(32) NOT NULL,
  updated_at          VARCHAR(32) NOT NULL,
  INDEX ecom_products_status_idx (status, published_at)
);

CREATE TABLE IF NOT EXISTS ecom_variants (
  id               VARCHAR(24) PRIMARY KEY,
  product_id       VARCHAR(24) NOT NULL,
  title            VARCHAR(255) NOT NULL DEFAULT 'Default Title',
  option1          VARCHAR(80) NOT NULL DEFAULT '',
  option2          VARCHAR(80) NOT NULL DEFAULT '',
  option3          VARCHAR(80) NOT NULL DEFAULT '',
  sku              VARCHAR(80) NOT NULL DEFAULT '',
  barcode          VARCHAR(80) NOT NULL DEFAULT '',
  price_cents      BIGINT NOT NULL DEFAULT 0,
  compare_at_cents BIGINT NOT NULL DEFAULT 0,
  cost_cents       BIGINT NOT NULL DEFAULT 0,
  track_inventory  SMALLINT NOT NULL DEFAULT 1,
  inventory        BIGINT NOT NULL DEFAULT 0,
  allow_backorder  SMALLINT NOT NULL DEFAULT 0,
  weight_grams     BIGINT NOT NULL DEFAULT 0,
  image            VARCHAR(500) NOT NULL DEFAULT '',
  position         INT NOT NULL DEFAULT 0,
  created_at       VARCHAR(32) NOT NULL,
  updated_at       VARCHAR(32) NOT NULL,
  INDEX ecom_variants_product_idx (product_id, position)
);

CREATE TABLE IF NOT EXISTS ecom_collections (
  id              VARCHAR(24) PRIMARY KEY,
  handle          VARCHAR(120) NOT NULL UNIQUE,
  title           VARCHAR(255) NOT NULL,
  description     VARCHAR(2000) NOT NULL DEFAULT '',
  image           VARCHAR(500) NOT NULL DEFAULT '',
  kind            VARCHAR(20) NOT NULL DEFAULT 'manual',
  rules           VARCHAR(2000) NOT NULL DEFAULT '[]',
  match_all       SMALLINT NOT NULL DEFAULT 1,
  sort            VARCHAR(20) NOT NULL DEFAULT 'manual',
  published       SMALLINT NOT NULL DEFAULT 1,
  seo_title       VARCHAR(120) NOT NULL DEFAULT '',
  seo_description VARCHAR(320) NOT NULL DEFAULT '',
  position        INT NOT NULL DEFAULT 0,
  created_at      VARCHAR(32) NOT NULL,
  updated_at      VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_collection_products (
  collection_id VARCHAR(24) NOT NULL,
  product_id    VARCHAR(24) NOT NULL,
  position      INT NOT NULL DEFAULT 0,
  PRIMARY KEY (collection_id, product_id),
  INDEX ecom_collection_products_product_idx (product_id)
);

CREATE TABLE IF NOT EXISTS ecom_customers (
  id                VARCHAR(24) PRIMARY KEY,
  email             VARCHAR(191) NOT NULL UNIQUE,
  first_name        VARCHAR(80) NOT NULL DEFAULT '',
  last_name         VARCHAR(80) NOT NULL DEFAULT '',
  phone             VARCHAR(40) NOT NULL DEFAULT '',
  accepts_marketing SMALLINT NOT NULL DEFAULT 0,
  note              VARCHAR(2000) NOT NULL DEFAULT '',
  tags              VARCHAR(500) NOT NULL DEFAULT '',
  address           VARCHAR(1500) NOT NULL DEFAULT '{}',
  member_id         VARCHAR(24) NULL,
  created_at        VARCHAR(32) NOT NULL,
  updated_at        VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_orders (
  id                 VARCHAR(24) PRIMARY KEY,
  number             BIGINT NOT NULL UNIQUE,
  token              VARCHAR(64) NOT NULL UNIQUE,
  customer_id        VARCHAR(24) NULL,
  email              VARCHAR(191) NOT NULL DEFAULT '',
  phone              VARCHAR(40) NOT NULL DEFAULT '',
  status             VARCHAR(20) NOT NULL DEFAULT 'open',
  financial_status   VARCHAR(24) NOT NULL DEFAULT 'pending',
  fulfillment_status VARCHAR(24) NOT NULL DEFAULT 'unfulfilled',
  currency           VARCHAR(8) NOT NULL DEFAULT 'USD',
  subtotal_cents     BIGINT NOT NULL DEFAULT 0,
  discount_cents     BIGINT NOT NULL DEFAULT 0,
  shipping_cents     BIGINT NOT NULL DEFAULT 0,
  tax_cents          BIGINT NOT NULL DEFAULT 0,
  total_cents        BIGINT NOT NULL DEFAULT 0,
  refunded_cents     BIGINT NOT NULL DEFAULT 0,
  tax_included       SMALLINT NOT NULL DEFAULT 0,
  discount_code      VARCHAR(60) NOT NULL DEFAULT '',
  shipping_method    VARCHAR(120) NOT NULL DEFAULT '',
  shipping_address   VARCHAR(1500) NOT NULL DEFAULT '{}',
  payment_method     VARCHAR(20) NOT NULL DEFAULT 'manual',
  payment_ref        VARCHAR(255) NOT NULL DEFAULT '',
  note               VARCHAR(1000) NOT NULL DEFAULT '',
  staff_note         VARCHAR(2000) NOT NULL DEFAULT '',
  tracking_company   VARCHAR(80) NOT NULL DEFAULT '',
  tracking_number    VARCHAR(120) NOT NULL DEFAULT '',
  tracking_url       VARCHAR(500) NOT NULL DEFAULT '',
  source             VARCHAR(20) NOT NULL DEFAULT 'web',
  restocked          SMALLINT NOT NULL DEFAULT 0,
  paid_at            VARCHAR(32) NULL,
  fulfilled_at       VARCHAR(32) NULL,
  cancelled_at       VARCHAR(32) NULL,
  created_at         VARCHAR(32) NOT NULL,
  updated_at         VARCHAR(32) NOT NULL,
  INDEX ecom_orders_created_idx (created_at),
  INDEX ecom_orders_customer_idx (customer_id),
  INDEX ecom_orders_email_idx (email)
);

CREATE TABLE IF NOT EXISTS ecom_order_items (
  id            VARCHAR(24) PRIMARY KEY,
  order_id      VARCHAR(24) NOT NULL,
  product_id    VARCHAR(24) NULL,
  variant_id    VARCHAR(24) NULL,
  title         VARCHAR(255) NOT NULL,
  variant_title VARCHAR(255) NOT NULL DEFAULT '',
  sku           VARCHAR(80) NOT NULL DEFAULT '',
  image         VARCHAR(500) NOT NULL DEFAULT '',
  unit_cents    BIGINT NOT NULL DEFAULT 0,
  quantity      BIGINT NOT NULL DEFAULT 1,
  total_cents   BIGINT NOT NULL DEFAULT 0,
  requires_shipping SMALLINT NOT NULL DEFAULT 1,
  position      INT NOT NULL DEFAULT 0,
  INDEX ecom_order_items_order_idx (order_id, position),
  INDEX ecom_order_items_product_idx (product_id)
);

CREATE TABLE IF NOT EXISTS ecom_order_events (
  id         VARCHAR(24) PRIMARY KEY,
  order_id   VARCHAR(24) NOT NULL,
  kind       VARCHAR(30) NOT NULL,
  message    VARCHAR(1000) NOT NULL,
  actor      VARCHAR(120) NOT NULL DEFAULT '',
  created_at VARCHAR(32) NOT NULL,
  INDEX ecom_order_events_order_idx (order_id, created_at)
);

CREATE TABLE IF NOT EXISTS ecom_discounts (
  id                 VARCHAR(24) PRIMARY KEY,
  code               VARCHAR(60) NOT NULL UNIQUE,
  kind               VARCHAR(20) NOT NULL DEFAULT 'percent',
  value              BIGINT NOT NULL DEFAULT 0,
  min_subtotal_cents BIGINT NOT NULL DEFAULT 0,
  usage_limit        BIGINT NOT NULL DEFAULT 0,
  once_per_customer  SMALLINT NOT NULL DEFAULT 0,
  used_count         BIGINT NOT NULL DEFAULT 0,
  active             SMALLINT NOT NULL DEFAULT 1,
  starts_at          VARCHAR(32) NULL,
  ends_at            VARCHAR(32) NULL,
  created_at         VARCHAR(32) NOT NULL,
  updated_at         VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_shipping_rates (
  id                 VARCHAR(24) PRIMARY KEY,
  name               VARCHAR(120) NOT NULL,
  price_cents        BIGINT NOT NULL DEFAULT 0,
  min_subtotal_cents BIGINT NOT NULL DEFAULT 0,
  max_subtotal_cents BIGINT NOT NULL DEFAULT 0,
  countries          VARCHAR(1000) NOT NULL DEFAULT '',
  position           INT NOT NULL DEFAULT 0,
  created_at         VARCHAR(32) NOT NULL,
  updated_at         VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS ecom_carts (
  id            VARCHAR(64) PRIMARY KEY,
  email         VARCHAR(191) NOT NULL DEFAULT '',
  items         VARCHAR(8000) NOT NULL DEFAULT '[]',
  discount_code VARCHAR(60) NOT NULL DEFAULT '',
  order_id      VARCHAR(24) NULL,
  created_at    VARCHAR(32) NOT NULL,
  updated_at    VARCHAR(32) NOT NULL,
  INDEX ecom_carts_updated_idx (updated_at)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_store_name','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_intro','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_currency','USD','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_tax_rate','0','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_tax_included','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_tax_shipping','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_pay_manual','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_pay_bank','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_bank_details','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_pay_stripe','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_per_page','24','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_low_stock','5','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_notify_email','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_send_receipts','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_countries','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_require_phone','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_policy_refund','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_policy_shipping','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
