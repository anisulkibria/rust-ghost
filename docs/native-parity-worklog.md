# Native Ghost parity implementation

Scope: complete the remaining feature audit, including native infrastructure.
User constraints: no Docker prerequisite; SQLite remains the default; MySQL is optional.
Stripe Connect onboarding remains deferred; direct keys remain supported.

## Implemented in the working tree

- Portal subscription read/update/cancel/resume/change and retention-offer endpoints.
- Persistent signed Mailgun delivery events and suppression/re-enable operations.
- Newsletter redirect tracking, click counts and editing destinations after sending.
- Revision snapshots/restore, duplicate posts/pages, post bulk actions and saved views.
- Internal-link search backed by published content.
- Publisher article gift links, rotation/revocation and private caching.
- Member CRUD, server-side NQL filtering/pagination, filter exports, bulk actions,
  saved views and staff-authorized single-use impersonation links.
- Staff password recovery with hashed, expiring, single-use tokens and session revocation.
- Web/email/social publishing previews and test newsletter delivery.
- Theme-declared image uploads and request-local live design previews.
- Native JPEG/PNG/WebP resizing with bounded memory, concurrency and a disk cache.
  Animated GIF, SVG and AVIF preserve their originals.
- Cookie-free daily analytics identifiers, bot/DNT/GPC exclusion, realtime/web stats APIs,
  device/browser storage and a frontend staff toolbar.
- Native persistent gift-delivery scheduling/retries using publication timezone.
- Explore opt-in synchronization; private and local sites are excluded.
- Native optional S3-compatible uploads with Signature V4 and local originals retained.
- Roles, labels, tiers, offers, invites, action-log and ownership-transfer APIs.
- Native automation flow controls, Koenig email editing and immutable per-email designs.
  Action revisions retain design snapshots across later body or design edits.
- Stripe checkout design settings, branding lookup and preview APIs, with Membership
  settings controls. Preview sessions do not grant memberships.
- Native ActivityPub identity, signed inbox/outbox, delivery queue/retries, Reader,
  Notes, follows, replies, likes, reposts and blocking. Disabled by default.
- Native machine payments: MPP Stripe/Tempo and x402, article-bound challenges,
  provider verification, persistent replay protection and paid Markdown responses.
  Disabled by default; provider setup is still required.
- Independent newsletter filters and per-audience paywalled bodies.
- Subscription tier grants now synchronize atomically without deleting unrelated grants.

The full workspace suite passed: 531 tests passed, 2 existing tests ignored.
After the final webhook error-handling changes, all 30 focused API integration
tests also passed, including a forced database-write failure and Stripe retry.
Coverage includes immutable designs, saved-view updates, signed suppression-event
replays, subscription grants, federation signatures/ownership, image size/format
conversion, article-bound payment challenges and persistent replay-store operations.
No real payment was charged, and external federation, S3 and payment-provider
acceptance are not implied by these tests.

## Running the native features

- Open **Modules → Social web** to enable federation and use the reader/notes.
  Federation requires a publicly reachable HTTPS publication for remote peers.
- Open **Modules → Machine payments** to configure supported settlement methods.
  Paid Markdown requires valid provider credentials, an article price and verified
  settlement. Normal HTML membership access continues to use member entitlements.
- Open **Settings → Membership → Checkout design** to save checkout appearance
  and create a Stripe preview. Direct secret/publishable keys remain supported.
- Select **Email design** on an automation email to save a versioned design.
- Local image storage remains the default. Optional S3-compatible storage uses
  this section in `config.<env>.json` (supply your own credentials):

```json
{
  "storage": {
    "active": "s3",
    "s3": {
      "region": "us-east-1",
      "bucket": "your-publication-assets",
      "accessKeyId": "YOUR_ACCESS_KEY",
      "secretAccessKey": "YOUR_SECRET_KEY",
      "assetHost": "https://assets.example.com"
    }
  }
}
```

For compatible object stores, `endpoint` can override the S3 endpoint. For
short-lived credentials, `sessionToken` is supported. Local originals are kept.
The bucket or configured asset host must make the uploaded objects readable;
the upload client does not change bucket permissions.

## Release verification

The release binary was rebuilt and restarted at `http://localhost:2368`
(PID 22089). SQLite migrations 19–23 applied successfully, and `PRAGMA quick_check`
returned `ok`. The homepage, sign-in/password-recovery pages and new admin assets
returned 200. Unauthenticated Admin API requests returned 401. The disabled
ActivityPub actor returned 404. A private SQLite backup was taken before migration.

## Remaining work

- Finish provider-contract and regression coverage for the new workflows.
- Verify editor/design/automation UI in a browser against isolated fixtures.
- Exercise checkout branding, MPP/x402 settlement, S3 and federation against
  configured external services. Broader federation interoperability still needs
  coverage (remote repost content, signed GET and newer signature formats).
- Remaining Ghost API families: theme/database/import/export endpoints, notifications,
  mentions, incoming recommendations, app installations and bulk link editing.
- Optional MySQL backend remains unimplemented. SQLite is the working backend;
  no MySQL server, database migration to MySQL, or Docker service is required.
- Broader device/location/conversion analytics and full Tinybird contract parity.
- Native storage-adapter registration/configuration beyond built-in local/S3 and
  responsive variants when assets are served directly by remote object storage.


Do not mark all audit findings resolved until the remaining items are implemented
and tested. Keep legacy staged/unstaged user work intact.

## Follow-up parity fixes — 29 October 2026

The gaps in `ghost-feature-audit-2026-10-29.md` are being addressed in the
working tree. The current implementation adds transactional content transfer
with full supported-table columns and relationship/id remapping; per-send
open/click tracking for automation revisions; persistent gift expiry reminders;
the automated-email compatibility facade and shared design/sender controls;
missing bulk/CSV/staff/invitation/key/subscription APIs; filtered bulk link
editing; newsletter sender verification; Mailgun analytics reconciliation jobs;
individual theme downloads; and the PUT thumbnail-upload alias.

Media processing now includes AVIF decoding/encoding and animated GIF resizing.
`img_url` uses retained local originals to serve variants of assets whose
originals were uploaded to the configured S3 asset host. SVG stays vector.
Transistor cards render from Lexical, substitute member/publication identities
on the site, and become personalized listening links in newsletters.

Native analytics now collects all five UTM fields and exports measured clicks,
signups and paid conversions. Optional local geolocation uses a host-supplied
MaxMind database; optional Tinybird frontend access accepts a read-scoped token.
Experimental app APIs validate and normalize server-fetched manifests, check
reviewed digests, retain approval history, recheck sessions, suspend changed
manifests, and uninstall. They remain gated by `labs.apps`.

Configuration (no Docker or database server needed):

```json
{
  "hostSettings": {
    "analytics": {
      "geoipDatabase": "/path/to/GeoLite2-Country.mmdb",
      "tinybird": {
        "readToken": "YOUR_READ_ONLY_FRONTEND_TOKEN",
        "expiresAt": "2026-12-01T00:00:00Z"
      }
    }
  }
}
```

Both analytics integrations are optional. Supply and refresh your own licensed
MMDB data and Tinybird credentials. Configured Tinybird clients query their
configured Tinybird service; native SQL reports continue to work independently.
AVIF codecs are Rust libraries; on x86 the decoder dependency uses NASM for its
assembly optimizations. Keep the Rust toolchain current and install NASM on x86
build hosts. No AVIF system library is required.

Gift reminders run from the existing gift worker. Mailgun analytics jobs run
from a separate persistent queue and are scheduled/cancelled through the Ghost
email analytics endpoints. Sender verification tokens are hashed, expire after
a day, and are consumed once through the corresponding verification endpoint.
No live Stripe payment, production newsletter, or third-party app installation
was initiated by this work.

Final verification: `cargo test --workspace --offline --quiet` passed with
613 tests passed, 0 failed and 2 existing tests ignored. The test runner
needed loopback access for its isolated SMTP, webhook and port-selection fixtures.
`node --test crates/ghost-admin-ui/tests/koenig-saving.cjs` passed all 9 tests,
and `git diff --check` passed.

Coverage includes content round trips and rollback, standard Ghost multipart
imports and CSV requests, canonical publication dates, newsletter previews/test
sends against a local SMTP server, expiring single-use verification tokens,
per-send automation opens/clicks, scoped link changes, password/key rotation,
app manifest validation and feature gating, gift reminder deduplication, AVIF
conversion, GIF frame/timing preservation, remote image URLs and Ghost's actual
Transistor render output. Live provider and real-browser acceptance testing
remain separate from these regression results.

Migration `0028_parity_workflows.sql` is embedded and applies on the next server
startup. The running publication was not restarted or migrated during this work.
For configured Tinybird clients, `hostSettings.analytics.tinybird.stats` can
supply the public `endpoint`, `endpointBrowser`, `version` and `datasource`
properties exposed by `/config`; credential fields are excluded from that
response. The external service still needs its datasets/pipes configured.
