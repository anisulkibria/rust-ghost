# Ghost 6.73.0 versus the Rust port

Audit date: 30 October 2026, Asia/Dhaka.

The latest stable release verified through GitHub is [Ghost 6.73.0, released
27 October 2026](https://github.com/TryGhost/Ghost/releases/tag/v6.73.0).
The reference installation is `/Users/anisulkibria/Documents/Ghost/Ghost/current`
(`versions/6.73.0`). Its Admin route registry and `collection` helper were also
compared byte-for-byte with the official `v6.73.0` GitHub tag and matched.
The nearby `Ghost-main` checkout reports `6.72.0-rc.0`; it was not used as the
latest-release baseline.

This audit includes staged, unstaged and untracked application code in the
current Rust working tree. It supersedes the **remaining-work conclusions** of
the older audits, not their historical findings. Many of those findings have
since been implemented.

## Reading the results

Implementation updates are recorded per finding below and in
[implementation progress](parity-implementation-progress.md). Unresolved findings
remain the audit baseline; passing tests for one area do not establish full parity.

- **Missing:** no equivalent implementation was found.
- **Partial:** a feature exists, but the stated behavior or contract is absent.
- **Different architecture/UI:** deliberately implemented another way.
- **Unverified:** implementation exists; production interoperability is not established.

This is a source and contract audit, not an exhaustive execution of Ghost's
acceptance suite. Registration of a route or helper alone is not evidence of
behavioral compatibility. Conversely, a different table name is not evidence
that a feature is missing.

## Confirmed product and data gaps

| Area | Status | Remaining gap and consequence | Port evidence |
| --- | --- | --- | --- |
| Posts/pages list pagination | Implemented | Database-side search, tag/status/NQL filtering, author scoping and global sorting precede pagination. Both lists show 25 records per page, accurate totals and state-preserving Previous/Next links. Verified with 170 fixture records and live browser checks. | [posts_list_impl](../crates/ghost-admin-ui/src/lib.rs), [list sorting](../crates/ghost-admin-ui/src/posts.rs) |
| Preview of unsaved content | Implemented | The dialog submits the live editor state — the bridge serializes Koenig rather than reading the hidden inputs — and the server renders a transient post that is never persisted. The social tab shows Facebook and X as separate cards reading their own meta fields, the email tab names the newsletter it rendered as, device widths cover desktop/tablet/mobile, and a saved post also offers its `/p/<uuid>/` link labelled as the saved version. | [preview.rs](../crates/ghost-admin-ui/src/preview.rs), [publishing-workflows.js](../crates/ghost-admin-ui/assets/publishing-workflows.js), [social cards](../crates/ghost-api/src/publishing.rs) |
| Legacy Mobiledoc conversion | Implemented | `@tryghost/kg-converters` ported: markup/list/card sections, the markup format bitmask, links with `rel`, `soft-return` linebreaks, renamed cards, the `embed` type rename and the callout colour fallback. Applied on import, on Admin API writes (including migrating a stored Mobiledoc on its next save), on editor open so the post is editable as real Lexical, and once at startup for posts nobody edits. Existing rendered HTML is preserved. | [mobiledoc.rs](../crates/ghost-render/src/mobiledoc.rs), [import preparation](../crates/ghost-api/src/publishing.rs), [Admin post writes](../crates/ghost-api/src/admin.rs) |
| Secret redaction in content exports | Implemented | Content exports exclude core settings, Ghost's credential blocklist and native provider credentials. Imports also preserve protected destination settings, including when an incoming row spoofs their group. Content exports still contain private publication content and staff data, as in Ghost; they are not public documents. Regression tests cover redaction and destination-key preservation. | [transfer::export](../crates/ghost-db/src/transfer.rs), [build_content_export](../crates/ghost-admin-ui/src/lib.rs); upstream `core/server/data/exporter/exporter.js` and `table-lists.js` |
| Latest self-service site-export API | Implemented | `GET /exports/download?components=` builds a selected-component ZIP over upstream's five synchronous components, refusing an empty selection and unknown names with upstream's messages. `POST /exports` validates upstream's component object and answers 202, then builds the archive — media included — in the background and emails the requester a link; `GET /exports` reports its state. Entry names are upstream's, so an archive restores into Ghost. | [export routes](../crates/ghost-admin-ui/src/rest.rs), migration 0032 |
| Audio/video in media archives | Implemented | Media export and restore include images, files, audio and video under `content/media`. Archives are assembled through a file on disk and streamed back in chunks, and directory entries are copied rather than read whole, so peak memory no longer scales with site size; the queued export path builds in the background. | `build_export_archive_to_file` in [admin lib.rs](../crates/ghost-admin-ui/src/lib.rs) |
| Extended backup/restore and offer history | Implemented | `export_with`/`include` reproduces upstream's `options.include` over a `BACKUP_TABLES` list — members and their full event history, emails, comments, automations, subscriptions, gifts, app installations, sessions and keys — plus this port's own operational tables, and the import reads them back. Opt-in, surfaced in the admin as "Members & full history", because the default export is the portable one; only listed tables can be named. `offer_redemptions` is projected into and out of Ghost's shape from `members.offer_id` with an id stable across exports. | [transfer BACKUP_TABLES](../crates/ghost-db/src/transfer.rs) |
| Article gift-link lifecycle | Implemented | GET reads without mutation; PUT atomically ensures/reuses a token; POST resets the single live link and retains historical rows. Tokens use Ghost's 192-bit base64url format; revocation preserves history and reports the standard count. Audit events identify staff/integration actors. Correction to the initial audit: Ghost also permits only one live link per post, with multiple historical records. Legacy native tokens remain valid on reads, but their hashes cannot be recovered as readable tokens; the first explicit ensure/reset replaces them. | [publishing router and gift handler](../crates/ghost-api/src/publishing.rs); upstream `core/server/api/endpoints/gift-links.js` |
| Excluding email link-checker HEAD requests | Implemented | Newsletter and automation redirect/pixel handlers record engagement only for GET. HEAD still returns the normal status/redirect with no body. Newsletter redirects also honor the click-tracking setting. Actual router regression tests cover HEAD and disabled tracking. | [newsletter redirects](../crates/ghost-api/src/newsletter_events.rs), [automation redirects](../crates/ghost-api/src/automation_tracking.rs); upstream `core/server/services/link-redirection/link-redirects-service.js` |
| Concurrent edit collision protection | Implemented | A stale `updated_at` plus a real change is refused with 409 and code `UPDATE_COLLISION`, through one rule shared by the Admin API and the native editor save. Upstream's two escape hatches are kept: a request without `updated_at` is unchecked, and a stale request that changes nothing is allowed; server-owned columns do not count as edits. The check runs before any write, where upstream rolls back. The editor adopts each save's new stamp and stops autosaving on a collision. | [collision.rs](../crates/ghost-api/src/collision.rs), [editor_collision](../crates/ghost-admin-ui/src/lib.rs), [save bridge](../crates/ghost-admin-ui/assets/koenig-bridge.js) |
| Complete authentication throttling policy | Implemented | Ghost's `express-brute` policy ported over the existing `brute` table, with the budgets from upstream's `config/defaults.json` and its doubling/lifetime semantics. Covers password login (API and HTML form, sharing one key), password-reset requests per IP and per address, sending and submitting verification codes, member magic links per address, and the Content API key with upstream's reset-on-success. A correct credential clears its count; a blocked caller cannot extend their own block. Forwarded client addresses are preferred over the proxy socket. Still a source-level implementation, not a penetration-test result. | [brute.rs](../crates/ghost-api/src/brute.rs) |
| Integration permission model | Implemented | Ghost's `rolePermissions` matrix is ported for all four integration roles, and enforced as a layer over the Admin API. This closed a real privilege-escalation gap: a key still resolves to the owner internally, so before this an Admin API key could mint further API keys and integrations, reset the site's authentication, read identity tokens and export or wipe the database — all eleven resources Ghost withholds from an Admin Integration. The narrowed grants are enforced too (newsletters, tiers, offers and automations can be created and edited but not destroyed; mentions browse-only; member sign-in URLs read-only). A path the mapping does not recognise falls through to the existing role checks rather than being denied, so an unmapped endpoint cannot break an integration in the field. Session requests and user-scoped keys are untouched. Verified live with a real signed key. | [integration_permissions.rs](../crates/ghost-api/src/integration_permissions.rs) |

## Themes, recommendations and editor gaps

| Area | Status | Remaining gap | Port evidence |
| --- | --- | --- | --- |
| `{{#collection}}` | Implemented | Renders posts with a default limit of three. Faithful to a surprise upstream: Ghost 6.73's `collection` option reaches only the posts cache key and nothing in the model layer reads it, and its `parseOptions` returns only a limit — so the helper lists latest posts and discards `filter`/`order`/`include`. A theme written against Ghost gets the same output. YAML routing collections remain separate. | [CollectionHelper](../crates/ghost-render/src/get_helper.rs) |
| `{{#get "tiers"}}` and `{{#get "newsletters"}}` | Implemented | Both resolve through one shared projection also used by the public endpoints, with filter, order, limit and page. A resource outside upstream's `RESOURCES` now renders `{{else}}` rather than an empty list. | [get_helper.rs](../crates/ghost-render/src/get_helper.rs), [shared browse](../crates/ghost-db/src/repos/tiers.rs) |
| Full `get` helper options and interpolation | Implemented | `id`/`slug` read selectors (with no pagination meta, as upstream's read returns none), `fields` (keeping the serializer-attached `url` for posts/pages/tags), `formats`, and upstream's `applyLimitCap`. Interpolation is a port of `resolvePaths`: `[*]` expands the array, `post.tags`/`post.author` resolve through Ghost's path aliases, `@`-paths read the globals, `.[0]` is normalised, and an unsupported segment resolves to nothing rather than to element 0. | [get_helper.rs](../crates/ghost-render/src/get_helper.rs) |
| Recommendation metadata | Implemented | Migration 0031 adds `excerpt`, `featured_image` and `favicon`; `/recommendations/check` already read all three from the candidate publication and they are now stored, served by both APIs, carried into the theme context and shown in the settings list. | migration 0031, [recommendations repo](../crates/ghost-db/src/repos/recommendations.rs) |
| Recommendation helper options | Implemented | The helper runs its own browse, so `limit`, `order`, `filter` and `page` apply, and it exposes pagination. A route that already prepared the default list is still used when no option narrows the query, so the common case costs no second query. | [Recommendations helper](../crates/ghost-render/src/helpers.rs) |
| Content API query consistency | Implemented | Tiers, newsletters and recommendations all apply filter, order, fields, limit and page, and return full `meta.pagination` instead of an empty meta object. Order strings are validated against known columns rather than interpolated. | [content.rs](../crates/ghost-api/src/content.rs) |
| Automation email editor integrations | Implemented | Both editors now build their Koenig integrations from one shared module, so the automation editor has the same upload hooks (five file types), snippet create/delete with the same permission gating, Unsplash, Pintura, embeds and the grouped `searchLinks` the audit reported as flat. Verified in a browser against a real `send_email` step: the composer mounts, typing syncs to the stored Lexical, and the grouped link search returns `[{label, items}]`. One deliberate difference remains: the post editor posts Koenig's browser-rendered HTML while automation emails are rendered server-side from Lexical — not a fidelity loss, because that renderer is pinned byte-for-byte to Ghost's own across every card it ships (`ghost_card_parity`, empty unsupported list). | [koenig-integrations.js](../crates/ghost-admin-ui/assets/koenig-integrations.js), [automation-editor.js](../crates/ghost-admin-ui/assets/automation-editor.js) |
| Automation canvas/UI | Different UI, partial parity | The native editor is a vertical form/card flow enhanced with drag ordering and zoom. It is not Ghost's full React Flow interaction model. Triggers, waits, sends, tier scopes, immutable revisions and execution analytics are implemented; branching must not be called a missing feature when the upstream model also restricts flows to a linear chain. | [automations UI](../crates/ghost-admin-ui/src/automations.rs), [automation domain rules](../crates/ghost-core/src/automations.rs) |
| Full GScan theme validation | Implemented, correcting this row | All 266 rules of gscan 6.6.1 are ported, with upstream's codes, levels, wording and fatal split. The rule table is **generated** from the real tool's own merged v6 spec by `tools/gscan-rules.js` rather than transcribed. **This row's original conclusion was wrong**: it said the legacy deprecation catalogue "does not apply to a port that does not claim to run Ghost 1.x themes". But gscan's v6 spec is `_.merge` over v5 down to v1, so every deprecation rule is live in a v6 check — that is their purpose, telling the author of a theme being brought forward which helper stopped working. 147 rules were missing. Reading gscan closely also corrected three behaviours here: a missing `package.json` fails 23 rules rather than one, `{{ghost_head}}` is reported even for a theme with no templates, and only partials reachable from a routable template are walked. Six `GS090` rules remain, each needing real Handlebars scope rather than path matching, named and justified in a test that fails if a seventh is added. Verified by running **both tools** over six purpose-built fixtures and 16 installed themes: identical codes and levels throughout. | [gscan.rs](../crates/ghost-render/src/gscan.rs), [generated rules](../crates/ghost-render/src/gscan_rules.rs), [parity test](../crates/ghost-render/tests/gscan_parity.rs) |

## Confirmed Admin API contract omissions

These paths are relative to `/ghost/api/admin`. The initial automated inventory
covered 240 method/path declarations in the upstream main registry. Parameterized
Rust loops, mounted uploads and the separately merged retry router were checked
manually to avoid falsely calling them missing.

| Method/path | Finding |
| --- | --- |
| `PUT /authentication/setup` | **Implemented.** Owner-only, and only after setup is complete; updates the owner's name, email and password and the site title/description. |
| `POST /authentication/reset` | **Implemented**, and the native `danger_reset_auth` button now shares the same service — which also rotates every API key secret and locks every staff account, not only sessions and tokens. |
| `DELETE /members/:id/suppression` | **Implemented**, resolving the member's address and reusing the existing suppression removal. |
| `DELETE /settings/stripe/connect` | **Implemented.** Removes the registered webhook while the key still works, clears the keys and turns paid memberships off; subscriptions are untouched on Stripe's side. |
| `GET /members/stripe_connect` | **Implemented** to upstream's contract: validates `mode` against `live`/`test`, refuses a production site without HTTPS using upstream's message, mints a `state` onto the caller's session and answers `302` to Stripe's OAuth URL with Ghost's own client ids. The handshake is one shared module used by both this endpoint and the native admin's form, and `PUT /settings` now completes it through `stripe_connect_integration_token` — verified against the session's nonce, then discarded rather than stored. Correction to the initial audit: the earlier version reported connection status, which was honest about the port but left Ghost's own admin no way to start a connection. |
| `POST /exports`, `GET /exports/download` | **Implemented** to upstream's contract; see the export findings above. |
| `GET /config/featurebase`, `GET /featurebase/token` | **Implemented.** Off unless `featurebase_enabled` and a JWT secret are configured, which is the normal self-hosted state; the token is a real HS256 JWT, since a third party verifies it. |
| `POST /support_requests` | **Implemented.** Upstream's validation and 202; posts to a configured host webhook in upstream's signed format, and otherwise emails the owner, since a self-hosted install has no host to ask. |
| `GET /upgrades`, `POST /upgrades`, `GET /upgrades/:id` | **Deliberately not implemented.** These are a Ghost(Pro) host-managed upgrade workflow; a self-hosted binary has nothing to stand behind them, and an endpoint that reported upgrades it cannot perform would be worse than its absence. |

This is not a claim that all other registered APIs match upstream payloads,
permissions, validation, pagination, error responses and side effects exactly.
The gift-link and export examples demonstrate why route-count parity is insufficient.

## Infrastructure differences and external verification

| Area | Finding |
| --- | --- |
| MySQL | No MySQL backend. SQLite is the working native backend and is an intentional project choice. |
| Ghost's Node adapter ecosystem | Built-in local and S3 storage exist. Arbitrary Ghost JavaScript storage/cache/scheduling adapter modules cannot be installed and executed by this Rust runtime. |
| Tinybird | Implemented, warehouse provisioning out of scope | The ingestion/synchronisation pipeline is ported from `services/tinybird-sync`: a per-table watermark in `tinybird_syncs` (migration 0033), a compound `(updated_at, id)` cursor so rows sharing a timestamp are each sent exactly once, upstream's 60-second safety lag so the watermark cannot pass a row whose transaction was still open, keyset batches, and NDJSON chunked by both the byte and message bounds. Delivery is at-least-once: the watermark advances only after a batch is accepted. Runs as a background worker and does nothing when no warehouse is configured, which is the normal self-hosted case. Verified end to end against an HTTP receiver. Provisioning the warehouse itself remains outside the port. | [tinybird_sync.rs](../crates/ghost-api/src/tinybird_sync.rs) |
| Federation interoperability | Implemented, interoperability still unverified | Outbound document GETs are signed, so instances in Mastodon's authorised-fetch (secure) mode answer them — unsigned reads were 401ed there, which made those instances undiscoverable rather than merely degraded. Each redirect hop is signed separately, since the signature covers host and path. Inbox verification accepts RFC 9421 HTTP Message Signatures (`Signature-Input`/`Signature` structured fields, `@method`/`@target-uri`/`@path`/`@authority` components, `created` freshness, mandatory digest coverage) alongside the legacy draft-cavage header, dispatching on which is present. Outbound POSTs still sign draft-cavage, which is what the deployed network verifies. Tests verify against independently constructed signatures in both directions. Live interoperability against real peers is still not certified. | [crypto.rs](../crates/ghost-api/src/federation/crypto.rs), [network.rs](../crates/ghost-api/src/federation/network.rs) |
| Ghost CLI and hosted operations | Native setup/doctor/build/service commands differ from Ghost CLI. Ghost(Pro) host billing, support and managed upgrades are not reproduced. These are platform/service differences, not absent publishing features. |
| Provider acceptance | Real Stripe checkout/Connect/retention/gifts, production Mailgun delivery/reconciliation, S3, Explore synchronization, Transistor, machine-payment settlement and broad federation interoperability still need controlled end-to-end verification. Their implementations exist; they are not classified as wholly missing. |
| Full admin acceptance parity | Exact Ghost Admin interaction, localization, accessibility, keyboard behavior and all state transitions have not been certified. Native UI differences should be evaluated by workflow, not assumed to be absent backend features. |

## Features that should no longer be called missing

Implemented code exists for: Koenig/Lexical cards including Transistor; revisions
and restore; duplication; bulk post actions; tag filtering and internal editor
tags; saved views; internal-link search; web/email/social previews and test sends;
member filtering/CSV/bulk actions/impersonation; custom member fields; roles and
invitations; staff password recovery and email verification; paid tiers/offers;
Portal subscription changes/cancellation/retention offers; publisher gift links;
purchased gifts/scheduled delivery/reminders; checkout branding/design; newsletter
segmentation/paywall variants; provider suppression/delivery reconciliation;
automation authoring/sending/design/revision tracking/engagement analytics;
webmentions; recommendations; social web; Explore sync; JPEG/PNG/WebP/AVIF and
animated-GIF image processing; local/S3 storage; the staff toolbar; native
visitor/device/browser/location/campaign analytics; machine payments; and gated
experimental app-installation lifecycle APIs.

Presence does not erase the specific limitations listed above. The recent
card-text, recommendation-context, GeoIP and tag UI fixes are included in this audit.

## Verification

As originally audited, before the implementation work recorded above:

- `cargo test --workspace --offline --quiet`: **653 passed, 0 failed, 2 ignored**.
- `node --test crates/ghost-admin-ui/tests/koenig-saving.cjs`: **11 passed**.
- The workspace test log is `/tmp/ghost-audit-tests-20261030.log`.
- No live payments, production emails, remote federation writes or provider
  certification were performed for this audit.

Current state of the findings this report opened is in
[implementation progress](parity-implementation-progress.md), which carries its
own test and live-verification record. Rows above marked **Implemented** have
been changed since the audit ran; the rest remain the baseline.

Still outstanding, as of the latest implementation batch: the automation canvas
interaction model, deliberate; six `GS090` rules that need Handlebars scope
rather than path matching; and the infrastructure and provider-acceptance
items, which need a controlled end-to-end environment and real credentials
rather than source changes. The
rows this report opened are otherwise closed — see
[implementation progress](parity-implementation-progress.md) for each one's
test and live-verification record.
