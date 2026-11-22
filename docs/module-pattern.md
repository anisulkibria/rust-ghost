# How to add an admin module

**Status: the rule for new work.** Forms is the reference implementation. If a
new module disagrees with this document, the module is wrong.

## Why this exists

`crates/ghost-admin-ui/src/lib.rs` was 23,600 lines. Every feature that was
added by typing into it made the next feature harder to add, review and test:
a merge touched the same file every time, the CSS and the handlers for
unrelated screens sat in each other's way, and nothing could be read in
isolation.

Forms was built the other way round — its own Rust module, its own assets, its
own tests, its own migration. On 2026-11-13 the rest of the admin was moved to
match it: the stylesheet to `assets/admin.css`, and every screen into a module
named for what it is. `lib.rs` is now **468 lines** — the crate's own wiring
and nothing else:

```
lib.rs          module declarations, the crate's public re-exports, the
                router, the base-path and settings middleware, the brand mark
```

That is the shape every module from here on takes. The file must not grow back.

A "module" here means a self-contained administrative feature with its own
screens under `/ghost/<name>/`: Forms, Comments, Automations, Analytics,
Media, Memberships, Newsletters, Security, SEO, Social web. Most of them
predate this
document.

**Media (2026-11-14) is the first module built to this document from
nothing**, so it is worth reading beside Forms. Two things in it are new and
are the pattern to copy, not deviations:

- Its rules module (`ghost-core/src/media.rs`) is shared with `ghost-api`
  rather than only with the admin — the upload endpoint and the library both
  ask it which types are allowed and what a path may look like. Rule 2 always
  implied this; Media is where it pays.
- It has **no table of its own contents**. The filesystem is the record and
  `media_meta` holds only what the filesystem cannot. Where the authority for
  a module's data already exists somewhere outside the database, say so in the
  migration's comment and read the authority.

**SEO (2026-11-14) is the second**, and takes that last point further: it has
no table at all. Its report is computed from `posts` during the request that
renders it, so there is nothing to invalidate and no way for the report to
disagree with the editor — see
[crates/ghost-db/src/repos/seo.rs](../crates/ghost-db/src/repos/seo.rs). Its
migration exists only to create the eight `settings` rows the module writes,
because `SettingsCache::set` refuses a key that has no row: **a module with
settings and no migration has a Save button that always fails.** Two more
things in it are worth copying:

- Its rules module (`ghost-core/src/seo.rs`) holds **the settings' defaults as
  well as the audit rules**, and every default reproduces the behaviour the
  site had before the module existed. Installing it changes nothing until
  somebody changes a switch.
- Four crates read those defaults through one function
  (`ghost_core::seo::flag`), so the renderer, the sitemap, the public
  robots.txt handler and the report cannot disagree about what is switched on.

**Dashboard (2026-11-15) is the third**, and is the one to read for a module
that is *not* behind a single role. Three things in it are new:

- **It is open to every signed-in role, and gates its sections instead.** It
  is where `/ghost/` lands, so a `require_role(Administrator)` at the top
  would mean a Contributor signs in to "Not allowed". Its `guard` is
  `require_actor` alone, and each section is rendered only for a role that may
  open everything it links to — rule 8's real intent, which is that no link
  ever leads to a refusal.
- **Its `ghost-core` module holds rankings, not validation.** Nothing on the
  page comes from a visitor, so `ghost_core::dashboard` is not a trust
  boundary; what it owns is the *order* queues are shown in and the thresholds
  at which a checklist item appears. Those are the parts that go quietly
  wrong, and they are unit-tested there without a database. Rule 2 is about
  "rules that can be wrong", and a ranking qualifies.
- **It is not a card on the Modules hub** (registration 3), deliberately: it
  is the home, not a tool filed under one. Registrations 5, 6, 8 and 9 are
  empty too — no settings, no scripts, no tables.

**Security (2026-11-16) is the fourth**, and is the one to read for a module
whose settings are *defences* rather than preferences. Four things in it
depart from the three above, each deliberately:

- **Two of its defaults change behaviour on upgrade.** Every module before it
  took the SEO line that installing it changes nothing until somebody moves a
  switch. That is right for editorial preferences and wrong here: a hardening
  switch that ships off is hardening nobody has. The line drawn instead is
  *can this break a working site* — `security_headers` and
  `security_private_fetch` ship on because no configuration exists in which
  they break the admin, while `security_hsts` and the idle timeout ship off
  because both are promises somebody has to choose. The migration states the
  reasoning per key, and the report names anything still weak.
- **It has one deliberately permissive default, and the report calls it
  Critical.** `security_trust_proxy` preserves the old behaviour of believing
  `X-Forwarded-For`, because turning it off on a proxied site gives the whole
  internet one shared rate-limit budget and nothing inside the process can
  tell whether there is a proxy. Where no default is safe, ship the one that
  cannot break the site and make the report say so out loud.
- **Its `ghost-core` module is a trust boundary twice over.** `trust_proxy`
  decides whose address a rate limit is counted against, and
  `private_fetch_blocked` decides whether the server will read its own
  network. Both are named functions rather than `flag` calls at the call site,
  so there is one place to change what they mean and one name to grep.
- **Its form extractors are `#[serde(default)]` on purpose.** An axum
  extractor runs *before* the handler, so a `String` field answers a stranger
  `422` from the extractor rather than `302 /ghost/signin/` from `guard()` —
  which makes rule 8's "every handler's first statement calls the guard"
  untrue. Be lenient in the extractor and strict in the handler.

Registrations 7 and 8 are empty: it needs no new glyph (`lock` already
existed) and ships no scripts.

**Its second half, the firewall (also 2026-11-16), is where it stops being a
report and starts refusing requests**, and it is worth reading for three
things the four modules before it had no reason to do:

- **It is the first module whose middleware runs before everything else.**
  `ghost_api::firewall::{public_guard, admin_guard}` are mounted outermost on
  both routers — outside the private-site gate, outside the analytics
  counter, outside the session read. An address that is barred is barred
  before a theme is loaded or a cookie is parsed, which is the whole point of
  a firewall: the work does not happen.
- **It is the first module that can lock an administrator out of the admin.**
  Three of its four actions can, so each simulates itself before it writes —
  running the real `ghost_core::firewall::decide` over the real current rules
  plus the change, against the author's own address read through the *same*
  function the middleware will use. A special-case "is this my IP" check
  would have been easier and would miss the case that actually catches people
  out: the first admin *allow* rule naming somebody else's address, which
  locks the author out by switching allowlist mode on rather than by matching
  them. Turning the firewall off is never checked — an undo that can refuse
  is not one.
- **Its defaults point the opposite way from the rest of the module's**, and
  so it has its own `BOOL_SETTINGS` table and its own reader
  (`firewall::flag_spam`, beside `security::flag`). A missing `security_*`
  row means the defence is **on**; a missing `spam_*` row means the filter is
  **off**. Two tables rather than one, because a key looked up in the wrong
  one reads as a plausible wrong default — and for a filter that refuses
  people, that is the failure hardest to notice.

**Memberships (2026-11-16) is the fifth**, and is the one to read for a module
that is built out of features the admin already had. Four things in it are new:

- **It owns nothing: no table, no setting, no migration, and no write.**
  Members, tiers, offers, newsletters and subscriptions are all written by
  somebody else, and every button on its screens either leads to the editor
  that owns the thing or posts to the handler that already created it. SEO had
  no table; this has no *data* at all. Its value is entirely in reading six
  existing things together, because the commonest way a membership funnel is
  broken is for every screen in it to be individually correct — paid tiers
  that exist, Stripe that is not connected, a portal with no plan selected.
  A fresh install is exactly that site, and nothing in the admin could say so
  before.
- **Its `guard` takes no `writes` flag.** There is no state-changing handler
  in the module, so a flag nobody passes would be decoration, and the next
  person to add a write would reuse a guard that never origin-checked
  anything. The absence is documented at the guard rather than implied.
- **It adopted another module's screen without moving it.** The members list
  was a rail item; it is now the Memberships module's Members tab, and
  `/ghost/members/` is unchanged — thirty-odd links inside the admin point at
  it, as does every bookmark anybody has made. Rule 5's nav mapping (4) is how
  the adoption is actually expressed: every members page now passes the nav key
  `"memberships"` and lights Modules. Settings → Design and Navigation set the
  precedent; this is the first time a *module* did it. The adopted screen gets
  a back-chevron rather than the module's tab bar, because it already has a
  segmented bar of its own and two stacked bars read as a mistake.
- **It does not reuse `views_chart`, and says why.** That chart labels every
  tooltip and its `aria-label` "views", and draws one series; a signup is not a
  view, and the useful question here is what proportion of a day's arrivals
  ended up paying, which is a stack. The geometry, the clean axis maximum and
  the `compact` ticks are still shared. Rule 6 is about not restyling shared
  primitives, not about using one that would have to lie.

Registration 9 is empty (no tables), and so are 5 and 6 — Settings →
Membership and its 24 `SETTINGS_INDEX` rows predate the module by a long way,
and the module links to them rather than adding a page beside them. It needed
registration 7: a `group` glyph, because the members rail item used `ghost`,
which is this admin's brand rather than a picture of a readership.

**Newsletters (2026-11-16) is the sixth**, built straight after Memberships
and to the same shape — no table, no setting, no migration, one adopted screen
(the newsletter list, left at `/ghost/settings/newsletters/` where its design
editor lives). Three things in it are new:

- **It owns exactly one verb, and that is why its `guard` has a `writes`
  flag.** Retrying a failed edition is the fix for the thing the screen
  reports, and a report that names a failure and cannot act on it sends people
  to curl. The work is still `crate::retry_email`, the same function the Admin
  API endpoint calls. Sending costs money per recipient and spends the site's
  sending reputation, so the retry is origin-checked — the clearest case in the
  admin for not resting a write on one browser behaviour.
- **A rate is `Option<f64>`, and the `None` is the whole point.** With
  `email_track_opens` off, every `opened_at` is NULL, so an open rate of "0%"
  would be a measurement nobody took, reported as a result. `Programme::
  open_rate` returns `None` when tracking is off and the screen renders a dash
  and says why. Where a figure can be *unmeasured* as well as zero, the type
  has to be able to say so.
- **It asks the resolver, not the settings.** `transport_of` calls
  `crate::resolve_bulk_transport` — the same function the send loop calls a
  moment before it sends — rather than reading `newsletter_transport` and
  reporting that. A screen that read the setting would say "Mailgun" for a site
  whose Mailgun key is missing, which is the exact failure it exists to catch.
  **When reporting whether something will work, call the thing that will do
  it.**

It also closed a real inconsistency rather than adding to it. "Subscribers" had
two definitions: Settings → Newsletters counted `email_disabled = 0` and
ignored the subscription status, while the Memberships card counted
`status = 'confirmed'` and ignored the disabled flag, so one newsletter could
show two different counts on two screens. Both halves are now one constant,
`repos::newsletters::SUBSCRIBED`, and the per-newsletter query moved into the
repo that owns the table — with Memberships re-exporting it under its own name.
Picking up a second module that reads the same tables is when that kind of
drift becomes visible; fix it then rather than matching it.

The firewall is also what finally gave the Security module registration 9 (its
`firewall_rules` table is in the wipe list) and pushed it past rule 1's
1,500-line limit, so it is `security.rs` plus `security/{firewall,settings}.rs`
— the second worked example of that split after `settings.rs`.

It also replaced five separate `Redirect::to("/ghost/posts/")` literals with
one `shell::ADMIN_HOME`. Those five are the ways into the admin and they have
to agree; before, `/ghost/` went to the posts list while the rail's brand link
went to Analytics, so "home" was two screens depending on how you arrived.

## The reference: what Forms is made of

| Layer | File | What lives there |
|---|---|---|
| Rules | [crates/ghost-core/src/forms.rs](../crates/ghost-core/src/forms.rs) | Pure domain rules — field types, bounds, normalization, validation. No DB, no HTTP. |
| Data | [crates/ghost-db/src/repos/forms.rs](../crates/ghost-db/src/repos/forms.rs) | Every query. Structs for rows. Nothing else talks to these tables. |
| Schema | [migrations/sqlite/0011_forms.sql](../migrations/sqlite/0011_forms.sql) | Tables, indexes, the one-row config table. |
| Public | [crates/ghost-api/src/forms.rs](../crates/ghost-api/src/forms.rs) | The internet-facing half: render into a page, accept a response, rate limits, tokens. |
| Public assets | `crates/ghost-api/assets/forms.{js,css}` | Shipped to visitors' browsers. |
| Admin | [crates/ghost-admin-ui/src/forms.rs](../crates/ghost-admin-ui/src/forms.rs) | Every admin screen, handler and its page HTML. 1,120 lines. |
| Admin assets | `crates/ghost-admin-ui/assets/forms-builder.js`, `koenig-forms.js` | The builder UI and the editor card. |
| Tests | `crates/ghost-admin-ui/tests/forms.rs`, `crates/ghost-db/tests/forms.rs`, `#[cfg(test)]` in each file above | End-to-end, repo-level, and unit. |

Not every module needs all eight rows. A module with no public surface skips
the `ghost-api` rows; a module with no new tables skips the migration. The
rows it does need go where the table says.

## The admin crate, module by module

Where to look, and where a new screen goes. Everything under
`crates/ghost-admin-ui/src/`:

| | |
|---|---|
| `lib.rs` | the router, the middleware, the crate's public re-exports |
| `shell.rs` | the page shell: `page_for_role`, `rail`, `esc`, `svg`, `icon`, the theme script, `countable` |
| `roles.rs` | `Role`, `Actor`, `require_role` |
| `assets.rs` | `admin_asset`: the stylesheet and the vendored editor runtime |
| `auth.rs` | setup, sign-in, the emailed code, sign-out |
| `modules.rs` | the Modules hub and its cards |
| `editor.rs` · `saving.rs` | the post/page editor, and everything saving one does |
| `posts.rs` · `tags.rs` · `members.rs` | the three big lists |
| `settings.rs` + `settings/` | the hub, then `general`, `email`, `newsletters`, `features`, `membership`, `growth`, `advanced` |
| `themes.rs` · `design.rs` · `navigation.rs` · `staff.rs` | the remaining settings groups with screens of their own |
| `integrations.rs` · `apps.rs` | API keys, custom integrations, and the built-in apps |
| `dashboard.rs` | the admin home at `/ghost/dashboard/`, where `/ghost/` and every sign-in land |
| `analytics.rs` · `comments.rs` · `automations.rs` · `forms.rs` · `media.rs` + `media/` · `memberships.rs` + `memberships/` · `newsletters.rs` + `newsletters/` · `security.rs` + `security/` · `seo.rs` · `social.rs` · `click.rs` · `popups.rs` · `cookies.rs` | the modules proper |
| `preview.rs` · `schedule.rs` · `rest.rs` | supporting machinery |
| `tests.rs` | tests for the shared chrome and for screens whose handlers sit elsewhere |

---

## The rules

### 1. One Rust file per module, named for the module

`crates/ghost-admin-ui/src/<module>.rs`, declared `pub mod <module>;` at the
top of `lib.rs`. Every handler, every page function, every helper that only
this module uses, lives there.

A module that grows past ~1,500 lines splits into `<module>.rs` plus a
`<module>/` directory beside it — never into `lib.rs`. `settings.rs` is the
worked example: the hub and its shared helpers stay in `settings.rs`, which
declares `mod general; … mod advanced;` and re-exports them with
`pub(crate) use general::*;` so the router keeps naming handlers
`settings::<handler>`. Each submodule opens with `use super::*;`, which
reaches both the settings helpers and, through them, the crate root.

Nothing a module needs gets added to `lib.rs`. If you find yourself writing a
helper in `lib.rs` for one module, it belongs in the module; if two modules
need it, it belongs in `shell.rs`.

### 2. Rules that can be wrong go in `ghost-core`, pure

Anything that decides what is valid — bounds, normalization, parsing of
user-supplied structures, "is this an email" — goes in
`crates/ghost-core/src/<module>.rs` with no database and no HTTP types, so it
can be unit-tested directly and called from both the admin and the public
side. `ghost_core::forms` is the trust boundary for Forms and says so in its
module doc; yours should do the same.

Rebuild untrusted structures from scratch rather than storing what was sent.
`normalize_fields` drops unknown field types, bounds every string, strips
control characters and regenerates keys that are not the shape it issues.

### 3. Every query goes in `ghost-db/src/repos/<module>.rs`

No `sqlx::query` in a `ghost-admin-ui` module except where there is genuinely
nothing to own it — Forms has exactly one (`owner_email`), and that is the
ceiling, not the budget. Repos take `&Db` and typed input structs
(`repo::FormInput`, `repo::Browse`), return domain structs, and bind every
value. Nothing from a request is ever formatted into SQL.

MySQL compatibility is a repo-layer concern: `MEDIUMTEXT`/JSON columns are
read through `ghost_db::types::Text`, never `String`; no `RETURNING`; no
`datetime('now', ?)` with a bound modifier — compute the instant in Rust.
See [the MySQL notes](../migrations/README.md) and the repo's existing
`dialect::sql` usage.

### 4. The public half, if there is one, is a `public_router()` in `ghost-api`

`crates/ghost-api/src/<module>.rs` exposes a `Router` that `ghost-front`
merges, plus whatever hooks the renderer needs (`expand_html` for Forms). It
owns the body limit, the origin check, the rate-limit budgets and the token
scheme, and states them in the module doc in the order a request meets them.
Admin code calls into it for shared primitives (`ghost_api::forms::
same_origin`) rather than reimplementing them.

### 5. Registrations only, and each one has an address

A module announces itself in ten places. Each is one line or one array entry,
and each has a fixed home — **only the first two are in `lib.rs`**. Anything
beyond this list means the module is leaking into shared files.

| | What | Where |
|---|---|---|
| 1 | `pub mod <module>;` | `lib.rs`, with the other declarations |
| 2 | The route block, under a comment | `lib.rs`, in `admin_ui_router` |
| 3 | A `MODULES` entry — `("Forms", "/ghost/forms/", "form", "amber", "…")` | `modules.rs` |
| 4 | The nav mapping: add the key to the `"analytics" \| "comments" \| … => "modules"` arm, so every page of the module lights the one Modules rail item | `shell.rs`, in `rail()` |
| 5 | A `SettingsGroup` — slug, icon, section names | `settings.rs` |
| 6 | `SETTINGS_INDEX` rows, one per searchable field | `settings.rs` |
| 7 | An `icon()` arm, if the glyph is new: Material Symbols path data, 24×24, filled, with a one-line comment saying what it depicts | `shell.rs` |
| 8 | `admin_asset` arms — one `include_str!` per script | `assets.rs` |
| 9 | Table names in the delete-site wipe list, children before parents | `settings/advanced.rs` |
| 10 | If the module audits itself: a count on the admin home — see below | `dashboard.rs` + `ghost-core/src/dashboard.rs` |

**10, in full.** A module that reports on itself — Security's findings,
Memberships' and Newsletters' gaps — and that has a severity meaning *somebody
is stopped* (`Critical`, `Blocking`) says so on the admin home. Three lines:

* the module exposes `pub(crate) async fn measure(&AppState, …)` returning the
  struct its own `audit()` judges, and its page is changed to call it, so home
  and the module cannot count different things;
* `dashboard::audits` calls it and keeps the count of that one severity;
* `ghost_core::dashboard::{Audits, Health, health}` gain a field and a variant,
  and `health` is where the new module is ranked against the others — with a
  test, because a ranking is exactly the kind of judgement that goes quietly
  wrong.

The home screen shows **a count and a link, never a copy of a finding**. Two
screens that have to keep agreeing about one sentence is how they stop
agreeing. `Opportunity` and `Note` levels stay on the module's own screen.

Routes: static segments go before `{id}` — `/forms/new/` and
`/forms/responses/` precede `/forms/{id}/`, or the literal is read as an id.

Handlers are named through their module in the router — `get(forms::forms_page)`,
`get(posts::posts_list)` — not imported loose. Two modules may own a
`filter_menu`; qualifying says which.

CSS is not on this list: it lives in `assets/admin.css` (rule 6).

### 6. CSS: one labelled block in `assets/admin.css`, one class prefix

The admin's stylesheet is [crates/ghost-admin-ui/assets/admin.css](../crates/ghost-admin-ui/assets/admin.css),
`include_str!`'d as `CSS` and linked from the page shell. It is not in
`lib.rs` and nothing should put it back there.

A module's rules go in a single contiguous block at the end of the file,
opened by a comment naming the module:

```css
/* ---- forms: the builder's field cards, and a response's answers ---- */
```

using class names under a prefix nobody else uses (`fb-` the builder, `fm-`
the responses, `vfc-` the editor card). Reuse the existing primitives —
`.card`, `.rows .row`, `.field`, `.chip`, `.seg-bar`, `.btn`, `.ic .tint-*` —
rather than restyling them. Never edit a shared rule to suit one module.

The stylesheet is served by `admin_asset` and cached `immutable`, so it is
linked with a content hash (`admin_css_hash`) exactly like the scripts in
rule 7. Editing it needs no version bump; the URL changes on its own.

### 7. Scripts are vendored files, served hashed, and are never the authority

- One file per script in `crates/ghost-admin-ui/assets/`, `include_str!`'d
  into an `admin_asset` arm. No build step, no bundler, no CDN. Plain ES5-era
  JS in an IIFE with `'use strict'`.
- **Cache-bust by content hash**, not by remembering to bump a version. Use
  the shared helper:

  ```rust
  fn builder_script_hash() -> &'static str {
      static HASH: std::sync::LazyLock<String> =
          std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/forms-builder.js")));
      &HASH
  }
  ```

  and request it as `{assets}/forms-builder.js?v={hash}`. Assets under
  `/ghost/assets/<version>/` are served `immutable` for a year; a script
  changed without a new URL stays stale in every open browser. The hash makes
  a changed script a new URL by construction. Use this for every new script.
  (`KOENIG_VERSION` still gates the path and still needs bumping for the
  editor bundle itself.)
- Hand the script its data through a `data-*` attribute holding escaped JSON
  (`data-builder="{…}"`), not through an inline `<script>` with values
  interpolated into it.
- The script is a convenience. The server re-derives and re-validates
  everything it posts. Say so in the file's header comment.
- Text from the server or from a user is placed with `textContent` and
  `.value` only. Never `innerHTML`.
- Give the no-JS case an answer — Forms shows a `<noscript>` banner.

### 8. The security contract every handler keeps

- **One `guard()` per module**, at the top of the file: the role check, plus
  an origin check for anything that changes state.

  ```rust
  async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
      let actor = require_role(state, headers, Role::Administrator).await?;
      if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
          return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
      }
      Ok(actor)
  }
  ```

  Every handler's first statement calls it. The session cookie is
  `SameSite=Lax` already; the explicit check is so a destructive POST does not
  rest on one browser behaviour.
- **Escape at the point of output.** `esc()` on every interpolated value,
  including values you believe are safe. Store plain text; never store markup.
- **Validate ids before they are used or linked**:
  `ghost_core::ids::is_objectid(&id)`.
- **Query parameters that come back as links are reduced to known values**
  first — see `filters()`, which maps anything unrecognised to a default, and
  its test asserting a forged status does not survive.
- **Never build a link out of user data** except through a strict check, and
  then only the narrow form (`mailto:` to an address that passed
  `is_email`; a `Referer` path only if it `starts_with('/')` and not `//`).
- **CSV exports run every cell through `csv_safe`** — a cell beginning `=`,
  `+`, `@` or `-` executes in a spreadsheet.
- **Errors**: log the detail with `tracing::error!`, return a short sentence.
  Never render a `GhostError` into the page.
- If the module handles other people's personal data, say so in the module
  doc and gate it at the same level as Members.

### 9. Settings live in the module, rendered with the shared chrome

A module with site-wide settings owns `settings_page` / `settings_save` in its
own file, routed at `/ghost/settings/<slug>/`, and renders with
`crate::group_head(group, Some("<slug>-settings"))` after looking its group up
in `SETTINGS_GROUPS`. Redirect with `crate::settings_flash(slug, "ok"|"error",
msg)`. Store under the module's own config table if any of it is secret —
`forms_config` holds the token-signing secret and is deliberately out of
`settings`, which the settings APIs serialize.

Accept only the values you offered: `retention_days` is parsed only if it
matches an entry in the `RETENTION` list.

### 10. Use the shared helpers; do not reinvent the chrome

From `crate::` in `ghost-admin-ui`:

| Helper | For |
|---|---|
| `page_for_role(role, title, nav_key, user, body, nav)` | The page shell. Always. |
| `require_role(state, headers, Role::…)` | Authorization. |
| `esc(s)` | Escaping. |
| `svg(name)`, `svg_stroke(name)` | Icons. |
| `countable(n, "field")` | "1 field" / "3 fields". |
| `select_options(&[(value, label)], current)`, `SELECT_CARET` | Selects. |
| `flash_banner(&q)` | The banner a redirect's `?ok=`/`?error=` renders. |
| `settings_flash(slug, kind, msg)` | Redirect back to a settings page. |
| `not_found_page_for(role, user, msg)` | A missing row. |
| `export_date_stamp()` | The stamp in an export filename. |
| `ASSETS` | The versioned asset base path. |

Add a local `flash_to(path, kind, message)` for in-module redirects, as Forms
does. **Post-redirect-get everywhere**: a POST never renders a page.

### 11. Migrations: new file, never an edit

`migrations/sqlite/00NN_<module>.sql`, next free number, with comments saying
what each column is for and what the enum-ish strings may be. An applied
migration can never be edited — sqlx checksums them; a change breaks boot with
"previously applied but has been modified". Add another file instead.

`migrations/translate.py` used to generate the MySQL twins, but it stops part
way (at `0004_email.sql`) and must not be run, so write `migrations/mysql/00NN_*.sql` by hand to match. Use
`VARCHAR(n)` with explicit lengths, `TEXT` for JSON blobs (read through
`types::Text`), `INTEGER` for booleans, `TEXT` for dates. Index what you
filter and sort on. `ON DELETE CASCADE` for children, and still list children
first in the wipe list.

### 12. Tests: three levels, and the negative ones matter most

1. **Unit tests** in `#[cfg(test)] mod tests` at the bottom of each file —
   the pure rules in `ghost-core`, the HTML builders and query-parameter
   reducers in the admin module. Forms tests that a hostile answer comes back
   escaped, that the export defuses formulas, that forged filters collapse to
   defaults, that the pager appears only when needed.
2. **Repo tests** in `crates/ghost-db/tests/<module>.rs` against
   `connect_test()`.
3. **An end-to-end test** in `crates/ghost-admin-ui/tests/<module>.rs` that
   drives the real router with `tower::ServiceExt::oneshot`: build the thing
   in the admin, exercise the public side the way a browser would, read it
   back, export it, delete it. Start with the refusals —
   `the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator` is
   the first test in the file for a reason.

Name tests as sentences describing the guarantee, not the function under test.

Run with `CARGO_INCREMENTAL=0` — the dev machine's disk is tight. Known
pre-existing failures that are not your regression:
`admin_branding_uses_the_current_site_name_and_escapes_html`, and 13 of 14 in
`tests/koenig-saving.cjs`.

### 13. Write the module doc before the code

Every file opens with a `//!` block that says what the module is, which halves
it has, who may see it, and what the security argument is — in prose, for
someone who has never seen the feature. The headers on
`ghost-admin-ui/src/forms.rs`, `ghost-api/src/forms.rs` and
`ghost-core/src/forms.rs` are the standard to match. Comments explain why a
choice was made, especially where the obvious alternative is wrong.

---

## Checklist for a new module

```
[ ] crates/ghost-admin-ui/src/<module>.rs         — all screens + handlers
[ ] crates/ghost-core/src/<module>.rs             — pure rules, if any
[ ] crates/ghost-db/src/repos/<module>.rs         — all queries
[ ] crates/ghost-api/src/<module>.rs              — public router, if any
[ ] migrations/sqlite/00NN_<module>.sql + mysql twin
[ ] crates/ghost-admin-ui/assets/<module>-*.js    — hashed URL, no build step
[ ] crates/ghost-admin-ui/assets/admin.css        — one labelled block, prefixed
[ ] crates/ghost-admin-ui/tests/<module>.rs       — end-to-end, refusals first
[ ] crates/ghost-db/tests/<module>.rs             — repo level
[ ] #[cfg(test)] unit tests in each file

registrations — one line each, in the file named:
[ ] lib.rs                pub mod <module>;
[ ] lib.rs                route block (static segments before {id})
[ ] modules.rs            MODULES entry
[ ] shell.rs              rail() nav mapping
[ ] shell.rs              icon() arm             (if it needs a new glyph)
[ ] assets.rs             admin_asset arms       (one per script)
[ ] settings.rs           SETTINGS_GROUPS entry  (if it has settings)
[ ] settings.rs           SETTINGS_INDEX rows    (if it has settings)
[ ] settings/advanced.rs  wipe-list tables, children first
[ ] dashboard.rs          audits() count + ghost-core health() ranking
                                                 (if the module audits itself)

Before calling it done:
[ ] guard() on every handler; writes also origin-checked
[ ] esc() on every interpolated value
[ ] ids validated before use or linking
[ ] query params reduced to known values before being linked
[ ] POST → redirect → GET everywhere
[ ] a <noscript> answer where a script drives the page
[ ] cargo build --release -p ghost && ./run.sh restart to see it on :1971
```

## Known deviations in Forms itself

Honest notes, so they are not copied as precedent:

- Its settings fields are missing from `SETTINGS_INDEX`, so they do not turn
  up in settings search.
- `owner_email` runs a query in the admin layer instead of in the repo.
- `responses_page` carries a small inline `<script>` for the form filter.
  Under rule 7 that belongs in an asset file; it stays inline only because it
  is nine lines with no data of its own.
- ~~Its CSS is in the shared `lib.rs` stylesheet.~~ Fixed 2026-11-13: the
  stylesheet moved to `assets/admin.css` whole, and took 130 KB off every
  admin page response along the way.

## What the 2026-11-13 split did, and what it left

`lib.rs` went 23,602 → 468 lines in two steps: the stylesheet to
`assets/admin.css`, then every screen into the modules listed above, including
`settings.rs` into a hub plus seven group modules. The moves were pure text —
verified line by line against the original, with four deliberate exceptions
(two routes qualified as `posts::`, one `uuid_v4` call fully qualified, one
duplicate import dropped). The crate's public API is unchanged; `cargo test`
gives the same 121 passed / 1 pre-existing failure it did before.

Three things the split surfaced rather than caused, each left as it was found:

- `audience_clause` existed twice, identically, in `lib.rs` and `analytics.rs`.
  Now once.
- Two different `filter_menu` functions exist — the posts list's six-argument
  one and the editor's five-argument NQL menu. They are told apart by
  qualifying the call, not by renaming either.
- `build_ghost_export` carries `#[cfg(test)]` in a file full of production
  export code. That is how it was written; worth a second look.

Still open, in rough order of value:

- The tests in `tests.rs` belong with the modules they exercise. Moving them
  means rewriting each `super::X` to name the module, so it was left for a
  pass of its own.
- `members.rs` (2,077), `analytics.rs` (1,907) and `saving.rs` (1,904) are
  over the 1,500-line line in rule 1, as are `settings/membership.rs` and
  `settings/advanced.rs`. Each has visible internal seams.
- Visibility was widened to `pub(crate)` mechanically, so some items are
  broader than they need to be. Narrowing them is safe, dull work the
  compiler checks.
