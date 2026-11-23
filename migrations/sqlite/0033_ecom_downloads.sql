-- ============================================================================
-- 0033_ecom_downloads.sql — Tools → E-commerce: digital downloads
-- ============================================================================
--
-- `ecom_files` — a product's downloadable files. The bytes live outside
--   every public route, at `content/private/ecom/<stored>`; `stored` is the
--   row's own id, never anything a person typed. `filename` is what the
--   buyer's browser saves it as.
--
-- `ecom_products.download_limit` / `download_days` — per purchase: how many
--   downloads (0 = unlimited) and for how many days after payment (0 = no
--   end).
--
-- `ecom_downloads` — one grant per paid order line of a product with files.
--   It names the buyer by email; a download needs a signed-in member with
--   that email. The limit and end date are copied in at grant time, so
--   changing the product later does not rewrite what was sold.
--
-- `ecom_download_log` — every file served: who, when, from where.
CREATE TABLE IF NOT EXISTS ecom_files (
  id           TEXT PRIMARY KEY,
  product_id   TEXT NOT NULL,
  name         TEXT NOT NULL,
  filename     TEXT NOT NULL,
  stored       TEXT NOT NULL,
  size_bytes   INTEGER NOT NULL DEFAULT 0,
  content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
  position     INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_files_product_idx ON ecom_files (product_id, position);

ALTER TABLE ecom_products ADD COLUMN download_limit INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ecom_products ADD COLUMN download_days INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS ecom_downloads (
  id               TEXT PRIMARY KEY,
  order_id         TEXT NOT NULL,
  order_item_id    TEXT NOT NULL UNIQUE,
  product_id       TEXT NOT NULL,
  title            TEXT NOT NULL,
  email            TEXT NOT NULL,
  member_id        TEXT,
  download_count   INTEGER NOT NULL DEFAULT 0,
  download_limit   INTEGER NOT NULL DEFAULT 0,
  expires_at       TEXT,
  revoked          INTEGER NOT NULL DEFAULT 0,
  last_download_at TEXT,
  created_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_downloads_email_idx ON ecom_downloads (email, created_at);
CREATE INDEX IF NOT EXISTS ecom_downloads_order_idx ON ecom_downloads (order_id);

CREATE TABLE IF NOT EXISTS ecom_download_log (
  id         TEXT PRIMARY KEY,
  grant_id   TEXT NOT NULL,
  file_id    TEXT NOT NULL,
  member_id  TEXT,
  ip         TEXT NOT NULL DEFAULT '',
  user_agent TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ecom_download_log_grant_idx ON ecom_download_log (grant_id, created_at);
