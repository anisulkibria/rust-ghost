# Native Vuuut Click

Vuuut has an optional native connector to an independently hosted or self-hosted
Vuuut Click service. Generic/local analytics continue to work without it.

Open **Analytics → Vuuut Click reports & connection** at
`/ghost/analytics/click/`. An Owner connects a verified Click website using its
website UUID and integration credential. Administrator/Owner can read native
reports; Owner alone manages connection, key rotation, disconnect and delivery
retries. The forms use the normal CMS session plus exact-origin protection.
Native reports include selected-post traffic, current CMS member/subscription
counts, period signup/subscription/newsletter events and currency-separated
revenue. Existing post analytics link through the actual permalink resolver.
The public tracker follows `web_analytics`, requires consent by default, and
injects only public collection attributes through `ghost_foot`.

Private credentials live in `click_connection`, outside the settings/API/theme
context. SQLite/MySQL migrations 0038–0039 add that state and a transactionally
captured, target-bound CMS event outbox. Member/subscription/payment/newsletter
source transactions and their outbox rows commit/roll back together. The worker
sends raw-body HMAC-signed minimized facts with stable opaque IDs; retries never
invent a fresh occurrence time. CMS events do not automatically stitch a member
to a browser session. No member email, raw provider ID or newsletter URL leaves
the outbox worker.

New Stripe endpoints subscribe to paid invoices/refunds. Existing endpoints can
be upgraded through **Update Stripe webhook events** in the native page, an
explicit Owner action preserving the signing secret. Signed invoice aliases,
checkout-versus-invoice duplication and cumulative refund replay are deduped by
stable provider fact IDs. Live provider verification is still an external gate.

Managed hosting supplies only a website-scoped `hostSettings.click` block:
`baseUrl`, `websiteId`, `collectionId`, `integrationKey`, `consentRequired`.
The CMS never needs the hosting platform's Click credential. Hosted connection
management stays with the platform, including during suspension/disconnect; hosted
settings override any old standalone database configuration. A restored site with a fresh analytics
mapping discards old-target queued records before capturing new facts.

The complete versioned provisioning, pinned JWKS/staff exchange, event signature,
lifecycle and retry contract is maintained in the adjacent Vuuut-click project:
`docs/GHOST_INTEGRATION.md`. The optional `tests/native-bridge` suite there runs
all three real adapters and desktop/phone/consent browser checks against isolated
databases. CMS `ghost-db --test click_outbox` and the native payment-webhook
headless test also run on MySQL via `GHOST_TEST_MYSQL_URL`.


Priority 3 adds lazy native report views for content/period comparisons,
retention/cohorts, newsletter editions, campaign costs and performance.
SQLite/MySQL migration 0040 captures newsletter editions and recipients in
source transactions. New edition links carry signed recipient proof;
legacy links remain supported. The worker sends edition-scoped hashes only;
newsletter titles are resolved locally for Admin display. Reports disclose
missing send/identity/engagement history rather than backfilling it.

Owners can save optional engagement and persistent-browser preferences without
reentering the connection key. Both default off; persistent identity requires
consent. Hosting controls these preferences for managed sites, through optional
`engagement` and `persistentIdentity` fields in its private scoped block.
Only public flags enter the theme. Owners can record/revise campaign costs
through the native form; credentials stay on the server and staff read tokens
cannot write. ROI remains unavailable without complete daily coverage and
matching attributed revenue/currency. Automatic member facts do not stitch
browser identity. Deploy the Click PostgreSQL 0011/ClickHouse 0003 build first.

Connected sites now use Click on the normal `/ghost/analytics/` page: overview,
traffic, content/engagement, returning visitors, newsletters, campaign returns
and performance. Growth and editorial email facts remain authoritative CMS
records. Post analytics displays its actual Click traffic inline. Connected
traffic never falls back to local `page_views` on a reporting failure. The
Ghost-compatible traffic stats API also uses Click for content, locations,
sources, per-post traffic and batch visitor counts, with explicit inclusive UTC
date bounds and canonical slash aliases. These private reports retain the
existing Administrator/Owner restriction; member/email API permissions remain
unchanged. The browser collector still respects the site/host analytics switch,
consent, DNT and GPC. Background registration completes managed setup before any
staff member opens Admin. Historical local traffic is not imported into Click.
