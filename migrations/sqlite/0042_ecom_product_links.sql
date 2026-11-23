-- ============================================================================
-- 0042_ecom_product_links.sql — Tools → E-commerce: a product's outside links
-- ============================================================================
--
-- `ecom_products.demo_url` — where a buyer can try a digital product before
--   paying (a theme's demo site, a template preview). The storefront shows a
--   "Live demo" button; themes read it as `demo_url`.
--
-- `ecom_products.purchase_url` — a direct checkout somewhere else (a Polar or
--   Gumroad product link). When set, the storefront's Buy now goes there in
--   place of the cart; themes read it as `purchase_url`.
--
-- Both are an http(s) URL or empty.
ALTER TABLE ecom_products ADD COLUMN demo_url TEXT NOT NULL DEFAULT '';
ALTER TABLE ecom_products ADD COLUMN purchase_url TEXT NOT NULL DEFAULT '';
