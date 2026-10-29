# Ghost 6.73.0 comparison — 29 October 2026

Compared the current Rust working tree, including staged changes, with the
installed official Ghost 6.73.0 source at
`/Users/anisulkibria/Documents/Ghost/Ghost/current`.
GitHub's latest-release URL also resolved to
[6.73.0, released 27 October 2026](https://github.com/TryGhost/Ghost/releases/tag/v6.73.0).

This is a source, route and data-flow audit. It is not an exhaustive browser
acceptance test or external-provider certification. “Missing” means no matching
implementation was found; “partial” means the workflow exists with specific gaps.
The older `ghost-feature-audit.md`, README deferred list, and some worklog
remaining-work entries predate implementations now present in this tree.

## Implementation update

The audit below is the original baseline. Its listed feature groups now have
implementations in the current working tree; see
[the implementation worklog](native-parity-worklog.md#follow-up-parity-fixes--29-october-2026)
for configuration and final verification (613 Rust tests and 9 editor tests
passed; 2 existing Rust tests ignored). In particular:

- Content transfer preserves supported table columns, Lexical, post metadata,
  relationships, tier assignments, dates, authors, settings and snippets in a
  transaction, with natural-key id remapping and rollback on invalid references.
- Automation tracking records opens/clicks against the actual send and immutable
  action revision. Gift expiry reminders have a persistent queue and retry state.
- Automated-email management, shared design/senders, verification, missing
  member/staff/subscription/bulk/CSV/key APIs and method aliases were added.
  Bulk link changes use the Ghost payload and require a valid selection.
- Mailgun reconciliation has persistent schedule/status/cancel workflows;
  individual theme downloads select the requested theme, and gift delivery flush
  processes due deliveries without advancing future gifts.
- AVIF and animated GIF variants, configured remote image-host routing,
  Transistor web/email rendering, all UTM fields, measured export metrics,
  optional local MMDB geolocation and the Tinybird read-token endpoint were added.
- Experimental app manifest review, digest checks, approval history, session
  rechecks, suspension and uninstall APIs are gated by the existing Apps flag.

These changes do not claim live provider certification, exact GScan diagnostics,
Ghost(Pro) hosting behavior, or a Tinybird data warehouse without its configured
external service. SQLite, direct Stripe keys and local/S3 storage remain the
native architecture. The table entries below describe the pre-fix code.

## Substantial remaining feature gaps

| Feature | Status | Current evidence and practical effect |
| --- | --- | --- |
| Faithful Ghost content import | Partial | `ghost-admin-ui/src/lib.rs:15079`, `run_import`, imports tags and posts but does not restore users, settings, or the normal `posts_tags`/`posts_authors` relation tables. It expects nested post tags instead. It clears author assignments and tier assignments, sets `published_at` to `None`, omits SEO/social/image metadata, and changes tier-restricted posts to drafts. A Ghost migration will require repairs. |
| Complete content export and round-trip backup | Partial | `ghost-admin-ui/src/lib.rs:14673`, `build_ghost_export`, exports a subset of post/tag fields. It omits Lexical source, authors, tag relations, tier relations, SEO/social metadata, snippets, and settings from the content JSON. The ZIP backup can include assets and themes, but its content JSON still uses this serializer. This is not a lossless database backup. |
| Automation email engagement tracking | Partial | Sending, wait steps, tier triggers, run history and counters exist. However, `ghost-api/src/automations_engine.rs:316` renders/personalizes then sends a plain message without an action-specific tracking pixel or click redirect registration. `ghost-db/src/repos/automations.rs:347` reads opened/clicked counters, but no runtime writer for those action-revision counters was found. Displaying these fields does not establish working open/click analytics. |
| Gift subscription expiry reminders | Missing | `ghost-api/src/admin_rest.rs:737`, `flush_gift_reminders`, only executes `UPDATE tokens SET data = data ...`, a no-op. The gift worker at `ghost-api/src/members.rs:3178` delivers purchase gifts, but no expiry-reminder sender/worker was found. Official Ghost has gift reminder scheduling, templates and sent-state tracking. Scheduled initial gift delivery is already implemented. |
| Complete responsive image support | Partial | `ghost-front/src/images.rs:136` returns original SVG/GIF/AVIF files rather than processing variants; AVIF is also absent from the accepted output-format switch. JPEG/PNG/WebP resizing exists. `ghost-api/src/storage.rs:172` publishes originals to S3, while the resize service reads local files; direct object-store asset URLs do not acquire native responsive variants. SVG passthrough itself is not a defect; AVIF/GIF transformation and remote variants are the gaps to assess. |
| Transistor Lexical card rendering | Partial | `ghost-render/src/lexical.rs:301` leaves the Transistor card out of server rendering. It is absent from the golden card fixtures too. Transistor account/private-feed configuration exists, but it does not implement this card renderer. Browser-saved HTML can still preserve existing markup, so this finding concerns rendering from Lexical source. |
| Full analytics compatibility | Partial | Local visitor, source, device, browser, conversion, country and realtime reporting exists. `ghost-front/src/analytics.rs:187` obtains country only from proxy headers, with no native IP geolocation. UTM term/content are not collected (`:160`). There is no Tinybird token/query contract, and the post analytics CSV omits clicks/signups/paid conversions (`ghost-admin-ui/src/lib.rs`, `POST_ANALYTICS_COLUMNS`). These are narrower gaps than “analytics missing.” |

Paths in tables are relative to `crates/` unless otherwise stated.

## Remaining Admin API compatibility gaps

Official reference: Ghost 6.73.0
`core/server/web/api/endpoints/admin/routes.js` and its endpoint handlers.
An HTML form or a differently named REST endpoint may implement the product
feature without satisfying an existing Ghost client's request.

| Area | Missing or incompatible behavior | Rust evidence |
| --- | --- | --- |
| Automated emails | No `/automated_emails` resource family, shared `/automated_emails/design`, independent sender editing/verification, or corresponding preview/test routes. No `/automations/:id/email_preview` or `/automations/:automation_id/actions/:action_id/links` route. The port's per-action designs and test-email route exist. | `ghost-api/src/automations.rs:35`; `ghost-api/src/automation_designs.rs:16`. |
| Members bulk actions and CSV | Ghost uses `PUT /members/bulk`, `DELETE /members`, and `GET/POST /members/upload`. Rust exposes `POST /members/bulk/` and `GET /members/export/`; CSV import is in the HTML admin. The native bulk/CSV features exist, but those Ghost requests have no equivalent method/path handler. | `ghost-api/src/admin_members.rs:16`; route inventory across `ghost-api` and `ghost-admin-ui/src/rest.rs`. |
| Staff-managed member subscriptions | No Admin API `POST /members/:id/subscriptions` or `PUT /members/:id/subscriptions/:subscription_id` route found. Member-facing Portal subscription endpoints are implemented separately. | `ghost-api/src/admin_members.rs:16`; `ghost-api/src/admin_rest.rs:27`; `ghost-api/src/member_subscriptions.rs`. |
| Posts/pages bulk deletion | No `DELETE /posts` or `DELETE /pages` collection handler. Native post bulk editing exists, but its allowed actions exclude deletion. | `ghost-api/src/admin.rs:32`; `ghost-api/src/publishing.rs:114`. |
| Optional CSV content importer | `csvContentImporter` can be toggled in Labs, but no `POST /posts/upload` CSV importer route was found. This is an optional upstream feature. | `ghost-admin-ui/src/lib.rs:9263`; API route inventory. |
| Staff and invitations | Missing `DELETE /users/:id`, `GET /invites/:id`, and `GET/POST /authentication/invitation`. Password change is registered as POST where Ghost uses PUT. Native staff deletion, invitation acceptance and password change through HTML screens are separate from these contracts. | `ghost-api/src/admin.rs:50`; `ghost-api/src/management.rs:29`; `ghost-admin-ui/src/rest.rs:26`. |
| Key/access-code operations | No `POST /integrations/:id/api_key/:keyid/refresh` or `POST /settings/access_code/regenerate` route found. | `ghost-api/src/admin.rs:70`; `ghost-api/src/admin_rest.rs:27`. |
| Newsletter sender verification | No `PUT /newsletters/verifications` route found. Newsletter CRUD and admin email configuration already exist. | `ghost-api/src/admin.rs:61`; route inventory. |
| Email previews and analytics resync | `GET /email_previews/posts/:id` exists, but its POST test-send method is absent; tests use `/posts/:id/test/`. No `GET/PUT /emails/:id/analytics` or `DELETE /emails/analytics` provider analytics-resync API. Status, batches, failures, retry, opens and clicks already exist. | `ghost-api/src/publishing.rs:24`; `ghost-api/src/newsletter_events.rs:14`; `ghost-admin-ui/src/lib.rs:272`. |
| Bulk link editing payload | Route exists, but Ghost takes a query `filter` and `bulk.action: "updateLink"`; Rust extracts filter from `bulk.meta.filter` and destination from `bulk.action.to`/`bulk.meta.to`. Standard Ghost payloads need adaptation. A missing/unrecognized nested filter also removes the post restriction in the current SQL. | `ghost-api/src/admin_rest.rs:514`, `links_bulk`; official `api/endpoints/links.js`. |
| Media thumbnail method | Rust registers `POST /media/thumbnail/upload`; Ghost registers PUT. Koenig's native upload may work, but a Ghost client using PUT will fail. | `ghost-api/src/images.rs:29`. |
| Individual theme download | `/themes/:name/download` points to the all-themes archive handler and does not select the requested theme. Full GScan diagnostic rule compatibility is also absent; native validation exists. | `ghost-admin-ui/src/rest.rs:39`, `:143`; `ghost-admin-ui/src/lib.rs`, `migration_export_themes`. |
| Gift delivery flush | `/gifts/flush_deliveries` updates a `status` column that does not exist in `scheduled_gift_deliveries`; the native worker instead selects by `sent_at` and attempts. The endpoint therefore cannot perform its intended flush with the current schema. | `ghost-api/src/admin_rest.rs:725`; `migrations/0019_publishing_workflows.sql:52`. |
| Experimental app installations | No `/apps/installations` preview/CRUD/session lifecycle was found. Ghost gates this family behind its private `apps` flag; this should be tracked separately from normal production features. | API composition in `ghost-api/src/admin.rs:13` and `ghost/src/main.rs:315`. |

There are further platform-specific upstream endpoints without a native match,
including Featurebase config/tokens, support requests, host-managed upgrades and
`/authentication/reset`. These should not be equated with ordinary publishing
features or a requirement to reproduce Ghost(Pro)'s hosting platform.

## Intentional architecture differences

- SQLite is the only working database backend; optional MySQL remains absent
  (`Cargo.toml:50`, `ghost-db/src/pool.rs`). SQLite is the project's documented
  default, so this is a platform difference rather than a request to replace it.
- Stripe Connect onboarding remains deferred. Direct-key Stripe setup, paid
  checkout and billing management exist. `/members/stripe_connect` reports
  direct-key status and a null onboarding URL (`ghost-api/src/admin_rest.rs:456`).
- Storage supports built-in local/S3 implementations, not arbitrary Ghost
  JavaScript adapter modules (`ghost-api/src/storage.rs:179`).
- The server-rendered admin is a native implementation. Exact Ghost Admin UI
  interaction/layout parity is not established by backend feature coverage.

## Present, but external interoperability remains unverified

Do not count social web, Explore, S3, checkout branding or machine payments as
entirely missing: substantive native implementations exist.

ActivityPub remote GETs are unsigned (`ghost-api/src/federation/network.rs:67`),
and inbox verification understands the legacy `Signature` header
(`ghost-api/src/federation/crypto.rs:73`). Compatibility with peers requiring
signed fetches or newer signature formats remains incomplete/unverified.
Live provider and federation acceptance testing was not performed in this audit.

## Features removed from the old “missing” list

Current implementations include revisions/restore, duplication, post bulk edits,
saved views, internal-link search, publisher gift links, member filtering and
impersonation, staff password recovery, Portal subscription updates/retention
offers, newsletter audience/paywall variants, click redirects, suppression,
scheduled gifts, checkout design, live theme design preview, JPEG/PNG/WebP
resizing, frontend staff toolbar, automation authoring/design, social web,
Explore sync, machine payments, local/S3 storage, and REST families for themes,
database import/export, notifications, mentions, recommendations and stats.

This list establishes implementation presence, not full upstream equivalence.

## Verification result

Ran `cargo test --workspace --offline --quiet` on the existing working tree.
It exited 101 before completing the suite:

```text
error[E0063]: missing field `host_settings` in initializer of `config::Config`
  --> crates/ghost-core/src/config.rs:520:9
```

The test helper `with_url` does not initialize the newly added field. No pass
count or current end-to-end readiness is claimed. Application source was not
changed during this audit; pre-existing staged work was preserved.

Suggested priorities: fix the test compilation blocker, make import/export
lossless for the supported data model, wire automation engagement tracking,
implement gift reminders, and close the Ghost API method/payload gaps needed by
real integrations. Follow with media variants and external acceptance tests.
