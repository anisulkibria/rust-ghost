-- ============================================================================
-- 0033_ecom_downloads.sql — Tools → E-commerce: digital downloads
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0033_ecom_downloads.sql (see there).
CREATE TABLE IF NOT EXISTS ecom_files (
  id           VARCHAR(24) PRIMARY KEY,
  product_id   VARCHAR(24) NOT NULL,
  name         VARCHAR(200) NOT NULL,
  filename     VARCHAR(200) NOT NULL,
  stored       VARCHAR(24) NOT NULL,
  size_bytes   BIGINT NOT NULL DEFAULT 0,
  content_type VARCHAR(120) NOT NULL DEFAULT 'application/octet-stream',
  position     INT NOT NULL DEFAULT 0,
  created_at   VARCHAR(32) NOT NULL,
  INDEX ecom_files_product_idx (product_id, position)
);

ALTER TABLE ecom_products ADD COLUMN download_limit BIGINT NOT NULL DEFAULT 0;
ALTER TABLE ecom_products ADD COLUMN download_days BIGINT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS ecom_downloads (
  id               VARCHAR(24) PRIMARY KEY,
  order_id         VARCHAR(24) NOT NULL,
  order_item_id    VARCHAR(24) NOT NULL UNIQUE,
  product_id       VARCHAR(24) NOT NULL,
  title            VARCHAR(255) NOT NULL,
  email            VARCHAR(191) NOT NULL,
  member_id        VARCHAR(24) NULL,
  download_count   BIGINT NOT NULL DEFAULT 0,
  download_limit   BIGINT NOT NULL DEFAULT 0,
  expires_at       VARCHAR(32) NULL,
  revoked          SMALLINT NOT NULL DEFAULT 0,
  last_download_at VARCHAR(32) NULL,
  created_at       VARCHAR(32) NOT NULL,
  INDEX ecom_downloads_email_idx (email, created_at),
  INDEX ecom_downloads_order_idx (order_id)
);

CREATE TABLE IF NOT EXISTS ecom_download_log (
  id         VARCHAR(24) PRIMARY KEY,
  grant_id   VARCHAR(24) NOT NULL,
  file_id    VARCHAR(24) NOT NULL,
  member_id  VARCHAR(24) NULL,
  ip         VARCHAR(64) NOT NULL DEFAULT '',
  user_agent VARCHAR(300) NOT NULL DEFAULT '',
  created_at VARCHAR(32) NOT NULL,
  INDEX ecom_download_log_grant_idx (grant_id, created_at)
);
