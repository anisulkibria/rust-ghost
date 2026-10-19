# Ghost-rust

A Rust port of the [Ghost](https://ghost.org) publishing platform, converted
from the Node.js source in `Ghost-main`. It runs real Ghost themes (verified
against the `flux` theme), exposes the Content and Admin REST APIs, and ships
a built-in server-rendered admin UI.

## Quick start

```bash
cargo run
```

- Site: http://localhost:2368
- Admin UI: http://localhost:2368/ghost → setup wizard on first boot
- Content API: `GET /ghost/api/content/posts/?key=<content key>` (the seeded
  key is printed to the boot log and shown in Admin → Integrations)

First boot migrates and seeds the SQLite database (`content/data/ghost-dev.db`),
activates the theme from the `active_theme` setting (falls back to the first
theme in `content/themes/`), and creates fixture content ("Coming soon" post +
"About" page + News tag). Complete setup at `/ghost` to activate the Owner
account.

## What's implemented

| Area | Status |
|---|---|
| Theme engine (Handlebars) | ✅ layouts (`{{!< default}}`), theme partials override builtins, `@site`/`@config` data, `{{^helper}}` inverted sections, ~40 helpers incl. `foreach`, `content`, `excerpt`, `img_url`, `date`, `is`, `has`, `pagination`, `ghost_head`/`ghost_foot`, `asset` fingerprinting, `tags`/`authors`, `navigation`, `{{#get}}` with NQL filters + `{{path}}` interpolation, `prev_post`/`next_post`, `match`, `plural`, `t` |
| Dynamic routing | ✅ `content/routes.yaml` (routes/collections/taxonomies, permalinks, `/page/N/`, RSS mounts), `content/redirects.yaml` (301/302 + regex + `$n` captures) |
| Content API | ✅ `/ghost/api/content/{posts,pages,tags,authors,settings,tiers,newsletters}` browse + read by id/slug, `?key=` auth (anonymous = public only), NQL `filter=`/`include=`/`order=`/`limit=`/`fields=`, Ghost envelope shapes |
| Admin API | ✅ setup flow, bcrypt session login (DB-backed, `ghost-admin-api-session` cookie), posts/pages/tags/users/settings/integrations CRUD, actions audit log, image/media upload |
| Admin UI | ✅ server-rendered: setup wizard, sign in/out, posts/pages list, markdown editor (tags, featured, status), tags, full Ghost-style settings hub — general, meta data, social accounts, analytics, site (announcement bar w/ server-rendered bar), membership (access, tiers, signup portal, newsletters), growth (recommendations, signup forms; Network & Explore deferred), advanced (integrations, Ghost-format import/export, code injection, labs), history (audit log), danger zone — plus navigation editor (primary + secondary menus) |
| Markdown | ✅ comrak (GFM + raw HTML) → `html`/`plaintext`/`reading_time`; markdown source round-trips in a `markdown` column |
| Sitemap / RSS | ✅ `/sitemap.xml` + per-resource maps, `/rss/` per collection/taxonomy, `/feed/` → 301 |
| Errors | ✅ theme `error.hbs` hierarchy (`error-<code>` → `error-<N>xx` → `error`) with builtin fallback |
| Database | ✅ SQLite (WAL, FK on), embedded migrations, Ghost-compatible column names (import-friendly), Ghost-style seeding (roles/permissions matrix, ~130 default settings with dynamic secrets, fixture content, builtin integrations) |

Deferred (extension points exist): members/newsletters/emails, comments,
Stripe offers, oEmbed, Koenig/Lexical editor format (admin uses markdown),
image resizing (`/content/images/size/WxH/…` URLs serve the original).

## Architecture

```
crates/
  ghost-core      config (Ghost-style layered JSON + GHOST_* env), errors,
                  24-hex ObjectIds, Ghost slug rules, dates, domain models
                  + Content-API JSON serializers
  ghost-nql       Ghost's NQL filter language: parser → parameterized SQL
                  (virtual keys: primary_tag/author(s)/tag(s)), in-memory
                  evaluation for routing ownership
  ghost-db        SQLite pool + migrations + seeder, repositories
                  (posts/tags/users/settings/api-keys/sessions/…)
  ghost-routing   routes.yaml/redirects.yaml → router tree with Ghost's
                  dominance order, permalink build/parse, template resolution
  ghost-render    theme loading/validation, Handlebars registry, preprocessing
                  (@site → @root rewrite, layout extraction, inverted
                  sections), helpers, comrak markdown pipeline
  ghost-api       AppState, Content API, Admin API, image upload
  ghost-admin-ui  built-in server-rendered admin area
  ghost-front     public site: routing → fetch → render, sitemap, RSS,
                  theme assets, /content/* static files
  ghost           the binary: boot sequence + HTTP composition
migrations/       embedded SQL migrations (schema mirrors Ghost's columns)
content/          themes/, data/, images/, routes.yaml, redirects.yaml
```

## Compatibility notes

- **Themes**: `content/themes/flux` runs unchanged (its own `routes.yaml` is
  copied to `content/routes.yaml`). handlebars-rust vs express-hbs gaps are
  bridged by preprocessing (`@site` → `@root.site`, `{{^helper}}` →
  `{{#helper}}…{{else}}`) and a `blockHelperMissing` implementation.
- **Database**: same table/column names as Ghost's SQLite schema (plus a
  `posts.markdown` column), so a future `INSERT INTO … SELECT` import from a
  real Ghost database works. Passwords are bcrypt like Ghost.
- **Config**: `config.development.json` / `config.production.json` in the
  working directory, `GHOST_…__…` env overrides, defaults like Ghost
  (port 2368, `content/` path).

## Tests

```bash
cargo test --workspace
```

42 unit tests: NQL parser/SQL compilation (incl. injection attempts), slug
rules, routes.yaml parsing + router matching, redirects, template resolution,
preprocessor rewrites, markdown pipeline, handlebars helper mechanics.
