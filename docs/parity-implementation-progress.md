# Parity implementation progress

The reference checklist is [the 30 October audit](ghost-feature-audit-2026-10-30.md).
The user asked to complete posts/pages listing first; other audit work remains queued.

## Posts/pages list

- Database queries apply author permissions, search, NQL, tag and status filters
  before sorting and pagination.
- Lists show 25 records per page, Previous/Next controls, the current page and
  the visible range within the matching total.
- Search covers title, author names/slugs, custom excerpts and plaintext across
  the complete collection. Submit with Enter or Search; Clear keeps other filters.
- Status counts cover all matching authorized records, independently of the
  selected status and page.
- Newest/oldest/title/views sorting ranks the complete matching collection.
- Changing filters/sort resets the page; pagination preserves all selections.
- Empty and stale/out-of-range page requests remain navigable.
- Featured stays represented by the existing star.

Completed and loaded on the local site. Four database integration tests with
170 records and 22 posts UI tests pass. Chrome verified a separate fixture with
135 posts and 35 pages: pagination, counts, global search/sorting, combined
filters, empty/stale pages, batch selection, mobile layout and owner/author
visibility. Both running localhost lists were also checked after restart.

## Export safety, article gifts and engagement

- Content exports exclude core settings, Ghost's protected credential keys and
  native SMTP/provider credentials. Imports cannot replace protected destination
  keys, including by falsifying the imported settings group.
- Newsletter and automation tracking ignores HEAD requests; redirects keep working.
  Newsletter redirects also honor disabled click tracking. Pixel opens require GET.
- Article gift GET is read-only, PUT ensures the same live token even concurrently,
  POST resets the link, and revocation preserves historical rows. Tokens use
  24 random bytes encoded as 32 base64url characters. Responses include token,
  creation time and a URL for the native editor. Bulk revocation returns meta.count.
- Gift actions record add/reset/remove with the staff or integration actor.
- The initial audit's implication that Ghost supports multiple simultaneously live
  links was incorrect: Ghost 6.73 also has one live link per post and retains history.
- Migration 0030 adds the upstream-shaped history/association tables. Old hashed
  native tokens survive reads and startup. Their original text cannot be recovered;
  an explicit ensure/reset replaces the old token with one that can be read back.
- Media ZIP export and restore include content/media (audio/video) as well as
  images/files. Export skips filesystem symlinks.

Validation: content transfer tests cover redaction, import protection and round trips;
headless tests cover gift lifecycle, legacy compatibility, permissions, audit actors,
and HEAD/GET tracking. A file-backed test exercises eight concurrent ensures across
four database connections. The media archive test checks bytes and restore paths.
Completed and loaded on localhost:1971. The 63-test headless suite passed;
the two gift tests were rechecked after adding audit actor assertions. Three
transfer tests, the four-connection concurrency test and the media archive test
pass. The release build passed. Live smoke checks verified redacted exports,
stable uncached gift reads and unchanged legacy gift rows after migration 0030.
The database was backed up before migration.

## Theme helpers, recommendations and the Content API

- `{{#collection}}` renders posts instead of nothing. It reproduces an upstream
  surprise deliberately: Ghost 6.73's `collection` option reaches only the
  posts cache key and nothing in the model layer reads it, so the helper lists
  the latest posts with a default limit of three and discards
  `filter`/`order`/`include`. A theme written against Ghost gets the same
  output. YAML routing collections are separate and unchanged.
- `{{#get}}` accepts `tiers` and `newsletters`, refuses a resource Ghost has no
  public controller for by rendering `{{else}}`, reads by `id`/`slug` (with no
  pagination meta, as upstream's read returns none), applies `fields` and
  `formats`, and caps `limit` at 100 with `all` capped rather than unbounded.
- Filter interpolation is upstream's `resolvePaths`: `[*]` expands an array
  instead of falling back to index 0, `post.tags` and `post.author` resolve
  through Ghost's path aliases, `@`-paths read the globals, and an unsupported
  path segment yields nothing rather than a silently wrong value.
- Recommendations store and serve favicon, featured image and excerpt.
  Migration 0031 adds the columns; `/recommendations/check` already read them
  from the candidate site and now they are saved. The settings list shows each
  publication's favicon.
- `{{recommendations}}` runs its own browse, so `limit`, `order`, `filter` and
  `page` apply. A route that already prepared the list is still used when no
  option narrows the query.
- `/content/tiers`, `/content/newsletters` and `/content/recommendations` apply
  filter, order, fields and real pagination metadata. Tiers and newsletters now
  come from one shared projection used by both the endpoint and the helper.

## Concurrent editing

- Post saves compare the submitted `updated_at` with the stored row and refuse
  a stale one with 409 and code `UPDATE_COLLISION`. The rule is upstream's
  bookshelf-collision plugin, including both escape hatches: a request with no
  `updated_at` is unchecked, and a stale request that changes nothing is
  allowed. Server-owned fields (`html`, `plaintext`, timestamps, `id`) do not
  count as edits.
- The check runs before anything is written, where upstream has to roll back.
- The editor posts the stamp it loaded, adopts the one each save returns, and
  on a 409 stops autosaving and tells the publisher to copy their work before
  reloading — retrying would hit the same wall every three seconds.

## Authentication throttling

- A port of Ghost's `express-brute` setup over the `brute` table this port
  already carried and never used, with the budgets from upstream's
  `config/defaults.json`. Free retries are free; each later failure doubles the
  wait from `min_wait` to `max_wait`; a window nobody touches for `lifetime`
  expires.
- Applied to password login (both `POST /session` and the HTML sign-in form,
  sharing one per-IP key), password-reset requests (per IP and per address),
  sending and submitting verification codes, member magic links (per address,
  so rotating IPs cannot flood one inbox), and the Content API key — the last
  resetting on any successful request, so only rejected keys count.
- A correct password or code clears its own count. A caller already serving a
  wait is not counted again, so hammering cannot extend a block indefinitely.
- Forwarded headers are preferred over the socket address; keying on a reverse
  proxy's own address would give the whole internet one shared budget.

## Legacy Mobiledoc conversion

- A port of `@tryghost/kg-converters`: markup, list and card sections, the
  markup-to-format bitmask, link nodes with `rel`, `soft-return` linebreaks,
  renamed cards (`code`→`codeblock`, `hr`→`horizontalrule`), the `embed` type
  rename and the callout colour fallback.
- Applied where Ghost applies it and in two places Ghost cannot: imports
  convert Mobiledoc-only posts so they render, Admin API writes convert an
  incoming Mobiledoc and migrate a stored one on its next save, the editor
  converts on open so a legacy post is editable as real Lexical rather than one
  opaque HTML card, and a one-off startup pass converts posts nobody edits.
  Existing rendered HTML is preserved; only the editable source is migrated.

## Exports

- `GET /exports/download?components=` builds a selected-component ZIP from
  upstream's five synchronous components, refusing an empty selection and an
  unknown name. `POST /exports` validates upstream's component object and
  answers 202, then builds the archive — media included — in the background and
  emails the requester a link. `GET /exports` reports queued/finished/failed.
- Archives are written to disk and streamed back in chunks, and directory
  contents are copied rather than read whole. The previous builder held the
  entire archive, media and all, in one `Vec<u8>`.
- Archive entry names are upstream's (`export.json`, `post-analytics.csv`,
  routes and redirects at the root), so an export from here restores into
  Ghost. The reader already accepted any JSON that parses as an export.
- Extended backup coverage: `export_with`/`include` carries upstream's
  `BACKUP_TABLES` — members and their whole event history, emails, comments,
  automations, sessions, keys — plus this port's own operational tables. It is
  opt-in, named in the admin as "Members & full history", because the default
  export is the portable one. Only tables on that list can be named.
- `offer_redemptions` is projected into and out of Ghost's shape from
  `members.offer_id`, with a derived id that is stable across exports so
  re-importing cannot double-count.
- Migration 0032 adds `export_requests`.

## Admin API contracts

- `PUT /authentication/setup` (owner only, after setup), `POST
  /authentication/reset`, `DELETE /members/:id/suppression`, `DELETE
  /settings/stripe/connect`, `POST /support_requests`, `GET
  /config/featurebase` and `GET /featurebase/token` are implemented.
- Reset-authentication is now a shared service that does the whole upstream
  job: it rotates every API key secret and locks every staff account as well as
  destroying sessions, member transient ids and outstanding tokens. The admin's
  existing danger-zone button goes through the same code, which previously only
  cleared sessions and tokens.
- Support requests post to a configured host webhook when there is one, in
  upstream's signed format, and otherwise email the owner — a self-hosted
  install has no host to ask. Featurebase tokens are real HS256 JWTs, since a
  third party verifies them.
- `GET`/`POST`/`GET /upgrades/:id` are deliberately not implemented: they are a
  Ghost(Pro) host-managed upgrade workflow, and a self-hosted binary has no
  equivalent to stand behind them.

## Publishing previews

- Previews already rendered the submitted editor state rather than the saved
  post (`preview.rs` plus the bridge's `ghostPreviewData`, which serializes the
  live Koenig state). What was missing was the preview experience around it.
- The social tab shows Facebook and X as two separate cards, each reading its
  own meta fields and cropping and truncating the way that network does — one
  generic card could not tell a publisher whether either headline survived.
- The email tab names the newsletter the preview was rendered as.
- A shareable link to the saved version is offered for a saved post, labelled
  as such, since the rendered preview is the unsaved state and `/p/<uuid>/` is
  not. Device widths stay hidden on the social tab, where they mean nothing.
- Fixed alongside: `ASSETS` and two page templates had hardcoded `1.8.17` while
  `KOENIG_VERSION` moved, so bumping the version busted no cache and a browser
  kept a stale bridge. All asset URLs now derive from one literal.
- The device toggle did nothing: the shared `dialog` rule clamps every dialog
  to `max-width: min(420px, 90vw)`, and the preview only overrode `width`. The
  frame inherited the clamp, so the page inside sized itself to ~346px and
  every device rendered as mobile. Measured in headless Chrome rather than
  guessed at. The dialog now restates every clamp the shared rule sets — the
  trap the Unsplash picker already documents — and the frame is sized in the
  device's own CSS pixels (1320/768/375) and scaled to fit, so Desktop is a
  real desktop viewport even on a narrow screen rather than whatever width the
  dialog happens to have. Verified: 1318px inner viewport on a 1600px window
  and, scaled to 0.72, still 1318px on a 1024px one.
- The dialog is now a full-height preview surface: tabs and device toggles
  share one toolbar row, the link and test-send controls share one compact
  footer row with the status line, and the frame takes every pixel left over.
  Previously five stacked rows of labelled fields left the preview itself a
  minority of the dialog.

## Validation of this batch

`cargo test --workspace --offline`: **757 passed, 0 failed**.
`node --test crates/ghost-admin-ui/tests/koenig-saving.cjs`: **14 passed** (three
new, covering the collision stamp moving forward on a save, staying put on a
409, and autosave stopping only for a collision).

Loaded on `localhost:1971` from a release build and checked live. The database
was backed up with `.backup` and the copy verified against the original before
migrations 0031 and 0032 applied.

- Content API: tiers, newsletters and recommendations return real
  `meta.pagination`; recommendation rows carry favicon/featured_image/excerpt.
- Throttling: three rejected Content API keys recorded one `brute` row at
  count 3 and still answered 401 (the budget is 99); a single authorized
  request cleared it, and successful requests write nothing.
- Collision: a stale `updated_at` with a real edit returned 409
  `UPDATE_COLLISION` and left the stored title untouched; a stale stamp with
  no edit, and a request with no stamp at all, both still saved.
- Exports: `?components=content,routes` produced a 39 KB zip containing
  `export.json` and `routes.yaml` under Ghost's filename; every validation
  branch of both routes returned upstream's message and status; `POST /exports`
  answered 202 and built a 2.4 MB archive of 160 entries including
  `content/images/` and `themes/casper/`, streamed back with an exact
  `content-length`. The default content export carried 19 tables and no
  members, keys, sessions or tokens; with `state=1` it carried 82 including
  members, api_keys, actions, automations and page_views.
- Mobiledoc: a planted Ghost 4 post (heading, bold, link, hr, list) was
  converted at startup — "converted 1 legacy Mobiledoc post(s) to Lexical" —
  and rendered correctly on the front end. Removed afterwards.
- Helpers, through a temporary custom template on one post: `{{#collection}}`
  listed three posts; `{{#get "tiers"}}`/`"newsletters"` resolved; `members`
  rendered `{{else}}`; `fields="title"` kept title and url and dropped html;
  `limit="all"` reported 100; `slug="engineering"` read one tag with no
  pagination meta, and a missing slug returned an empty set without erroring;
  `tags:[{{post.tags}}]+id:-{{post.id}}` and
  `authors:{{post.primary_author.slug}}` both resolved; `@site.title` read the
  globals. The template was removed and the post restored.
- New Admin API routes answered: featurebase config/token, setup update,
  member suppression (204, and 404 for a missing member), and support-request
  validation.
- `formats="plaintext"` was found to drop the html and supply nothing, because
  the serialized post has no `plaintext` key — it is a column the excerpt is
  derived from. Fixed by deriving it from the record's html, with tests, and
  re-verified live.
- Post, tag, user, member, newsletter, tier, recommendation, automation and
  comment counts match the pre-migration backup. Verification artifacts (the
  test export, its row, the probe template, the planted post) were removed and
  the site title restored after the setup-update check changed it.

Not covered by this: real provider traffic. No payments, production email,
remote federation writes or third-party Featurebase/host-webhook calls were
made — the support-request and export-webhook paths were exercised only
through their validation and unconfigured branches.

## Automation email editor

- Both editors already built their Koenig integrations from one shared module
  (`koenig-integrations.js`), so the audit's finding was stale: the automation
  editor has the same upload hooks, snippet management with the same
  permission gating, Unsplash, Pintura, embeds, and the grouped `searchLinks`
  the audit reported as a flat list. Verified in a browser against a real
  `send_email` step — composer mounts, typing syncs to the stored Lexical,
  link search returns `[{label, items}]` with 20 entries.
- One real bug found while verifying: the Markdown fallback stayed visible
  underneath the rich editor. `node.hidden = true` does nothing when a class
  sets `display`, and this admin sets `display` on nearly every component
  class. It had already been patched narrowly twice (`#editor-view[hidden]`,
  `dialog.unsplash [hidden]`); replaced with one global
  `[hidden] { display: none !important; }` and the two narrow patches removed.
- One deliberate difference remains: the post editor posts Koenig's
  browser-rendered HTML, while automation emails are rendered server-side from
  Lexical. Not a fidelity loss — that renderer is pinned byte-for-byte to
  Ghost's own across every card it ships (`ghost_card_parity`, 32 cases, empty
  unsupported list) — so a 29-site change to carry HTML through the domain
  model was judged not worth it.

## GScan theme validation

- A port of gscan 6.6.1 with upstream's rule codes, levels, wording and
  fatal/non-fatal split: GS005 template compilation, GS010 package metadata,
  GS020 structure, GS040 `{{ghost_head}}`/`{{ghost_foot}}`, GS050 all 103
  Koenig card classes, GS080 empty translations, and the v6 GS090 rules for
  conditional arguments and `{{#get}}` limits. Findings carry `file:line`.
- Activation is refused on a fatal finding, naming the rule that refused it;
  the full report is shown per theme in Settings → Themes, folded away when a
  theme is clean.
- Three corrections came out of comparing against the real tool rather than
  trusting the rule list. `card_assets` defaults to **true** in Ghost 6
  (gscan's `defaultPackageJSON`), and its tagged rules are skipped when it is
  on — without that, every official theme reported ~100 faults. gscan searches
  `.hbs` as well as `.css`. And its rule regexes have no word boundary, so
  matching more strictly than Ghost would have reported faults that are not
  faults. Casper, Source and Flux now report the same clean result in both
  tools.
- Not ported, and said plainly: the legacy deprecation catalogue for Ghost
  1.x–5.x themes, which is most of upstream's 266 rules and does not apply to
  a port that does not claim to run those themes.

## Integration permissions

- Ghost's `rolePermissions` matrix is ported for all four integration roles
  and enforced as a layer over the Admin API.
- This closed a real privilege-escalation gap rather than a cosmetic one. An
  integration key that names no staff user resolves to the **owner** here, and
  the role hierarchy then has nothing left to say — so before this, any Admin
  API key could mint further API keys and integrations, reset the site's
  authentication, read identity tokens, and export or wipe the database. Those
  are among the eleven resources Ghost withholds from an Admin Integration.
- The narrowed grants are enforced too: newsletters, tiers, offers and
  automations can be created and edited but not destroyed, mentions are
  browse-only, member sign-in URLs read-only.
- A path the mapping does not recognise falls through to the role checks that
  were already there rather than being denied, so an endpoint nobody has
  mapped cannot break an integration in the field. The eleven withheld
  resources are all mapped, so the escalation paths are closed.
- Session requests and keys that name a staff user are untouched.

## Validation of this batch

`cargo test --workspace --offline`: **791 passed, 0 failed** (23 new gscan
tests, 11 integration-permission tests). `node --test koenig-saving.cjs`: 14
passed.

Checked live on `localhost:1971` from a release build:

- Automation editor: `koenigMounted: true` with a real email step, five upload
  file types, snippets, Unsplash, grouped link search; typed text reached the
  stored Lexical; the duplicate Markdown fallback is gone.
- GScan: Casper, Source and Flux all clean, matching the real gscan. A
  deliberately broken theme reported 7 errors, 3 warnings and 1 recommendation
  with correct `file:line`, and activating it was refused with
  `GS005-TPL-ERR` — the active theme did not change.
- Integration permissions, with a real signed Admin API key: posts, tags,
  users, members and newsletters all 200; `db`, `exports`, `integrations`,
  `authentication`, `identities` and Stripe connect all 403; destroying a
  newsletter or tier 403 while browsing them is 200; a signed-in owner still
  200 on all of them. The destructive calls were **not** fired at the live
  site — `db` is withheld as a whole resource, so a refused read settles the
  delete.

## Federation signatures

- Outbound document GETs are signed. Mastodon's authorised-fetch (secure)
  mode 401s an unsigned read, so those instances were not merely degraded —
  they were undiscoverable. Each redirect hop signs separately, because the
  signature covers host and path.
- Webfinger stays unsigned, deliberately: it is not ActivityPub, it is served
  before any actor relationship exists, and servers do not expect a signature.
- Inbox verification accepts RFC 9421 HTTP Message Signatures —
  `Signature-Input`/`Signature` structured fields, `@method`, `@target-uri`,
  `@path`, `@authority`, `created` freshness — alongside the legacy
  draft-cavage header, dispatching on which is present. Both paths require
  the method, the target and a digest of the body to be covered, so a
  signature lifted from another request cannot authenticate this one.
- Outbound POSTs still sign draft-cavage. That is what the deployed network
  verifies today; sending 9421 to Mastodon would simply fail.
- Tests sign with an independently constructed base rather than reusing the
  module's own signer, so they would catch the module agreeing with itself.

## Tinybird ingestion

- The pipeline from `services/tinybird-sync`, which is what the port was
  missing: it had SQL analytics and a read token, but nothing that *put* data
  in a warehouse.
- Four details carry the correctness, all upstream's: a per-table watermark
  (migration 0033 adds `tinybird_syncs`); a compound `(updated_at, id)` cursor,
  because a watermark on a non-unique column either skips rows sharing a
  second or resends them forever; a 60-second safety lag, because a row's
  `updated_at` is set before its transaction commits and the watermark must
  not pass a row that was still open; and chunking by both the byte and the
  message bound.
- At-least-once by construction: the watermark advances only after a batch is
  accepted, so a crash resends rather than drops.
- Does nothing when no warehouse is configured — the normal self-hosted case.
  Provisioning the warehouse is still outside the port.

## Integration roles, tightened

- Following the permission matrix: the two internal integrations were seeded
  with the broad `Admin Integration` role, so the scheduler's key could edit
  settings, themes and users when its job is to publish a post, and the
  backup's key could do the same when its job is to read. Both keys sit in
  the database and on the integrations screen, so that difference is blast
  radius. They now carry Ghost's own `Scheduler Integration` and
  `DB Backup Integration` roles. Migration 0034 retightens existing installs,
  scoped by integration slug so a custom integration an operator deliberately
  gave the admin role keeps it.
- `PUT /schedules/{resource}/{id}` maps to `post`/`publish` rather than
  `post`/`edit`, or the scheduler — granted exactly `post: publish` — would
  have been refused its only job.
- One pre-existing gap surfaced while verifying and was fixed: the admin's
  REST handlers resolved the caller from the session cookie alone, so every
  route merged into `/ghost/api/admin` from that crate — the content export,
  themes, the routing files — was unreachable with an API key. The DB Backup
  integration exists precisely to call the export with one.

## Validation of this batch

`cargo test --workspace --offline`: **815 passed, 0 failed** (10 federation
signature tests, 10 Tinybird unit tests, 3 end-to-end pipeline tests, and
additions to the permission matrix).

Checked live on `localhost:1971` from a release build, after backing the
database up and verifying the copy; migrations 0033 and 0034 applied cleanly.

- Scheduler key: its publish path reaches the handler; posts, settings, users,
  themes and members all 403.
- Backup key: exports 19 tables and 43 posts, reads posts and members, and is
  403 on settings, users and themes.
- Zapier (general Admin Integration): refused the export, reads posts.
- Zapier and the custom integration kept the broad role, as intended.

The 44→43 post change between sessions is the owner's own deletions, logged
in `actions`; it is not from this work.

## The default port moved off 2368

Stock Ghost holds 2368, and Ghost CLI hands 2369, 2370 and upwards to further
instances — so a machine running this port to compare against Ghost had two
servers contending for one number, and one of them simply failed to start.
The default is now `DEFAULT_PORT = 1971`: clear of that whole range, and above
1024 so binding it needs no privileges. One named constant, used by the config
default, the generated config's url, `ghost setup`'s free-port walk, `run.sh`
and the README.

Changing it surfaced a real bug. `serve_plan` **never read the port out of the
url** — it used `server.port` or the built-in default — and that went unnoticed
only because the default happened to equal the port everyone writes in their
url. The moment the two differ, a config with `url: http://host:8080` and no
explicit `server.port` binds one port while writing every absolute link to
another: broken RSS, sitemap and canonical urls. Precedence is now explicit —
`server.port`, then the port in `url`, then the scheme's default — which also
removes the whole class of listener/link disagreement, while still letting an
explicit `server.port` differ from a public url, as a reverse proxy needs.

The illustrative `localhost:2368` urls throughout the source and tests were
changed to 1971 as well; they carried no meaning beyond "this project's port"
and read as stale. Four mentions were deliberately kept, because in each the
number *is* stock Ghost's: the `DEFAULT_PORT` rationale comment, the test
pinning the default away from 2368 and out of Ghost CLI's range, and the
README's note on running alongside an existing Ghost. The validation records
in this file and in `native-parity-worklog.md` were later rewritten to 1971 as
well: they read as instructions for where to look, and an address nothing
serves is worse than a small loss of historical accuracy about which port a
check happened to run on.

## Stripe Connect, and the settings contract it belongs to

`GET /members/stripe_connect` was the last audit row that was neither
implemented nor deliberately declined. It reported connection status; upstream
mints a `state` onto the caller's session and answers `302` to Stripe's OAuth
url. Status was honest about what the port did, but it meant Ghost's own admin,
pointed at this backend, had no way to start a connection at all.

- The handshake now lives in one module, `ghost_api::stripe_connect`, shared by
  the Admin API endpoint and the native admin's form. Upstream's `mode`
  validation (`live`/`test`, lowercased, defaulting to `live`, `422` otherwise),
  its HTTPS-in-production refusal with upstream's message, Ghost's own public
  client ids and `stripe.ghost.org` redirect.
- The site never holds a Connect client secret: Ghost's broker does, and what
  crosses the browser is bound to one admin session by the nonce. That binding
  is the entire security property, so it has exactly one implementation —
  previously the native admin had its own copy.
- The nonce is single-use and expires after thirty minutes, and is spent
  *before* the keys are adopted. A blob that got as far as a Stripe call must
  not stay replayable because that call failed. The window is not upstream's:
  an express session expires with its cookie, while this port keeps long-lived
  session rows, so without a stamp a `state` stayed redeemable for as long as
  the admin stayed signed in.
- `PUT /settings` now handles `stripe_connect_integration_token`, which is how
  Ghost's admin finishes the handshake. The token is verified and discarded,
  never stored — it is a one-time bearer of a live secret key, and a settings
  row is readable back through `GET /settings`.
- Verified before any write, as upstream does: a token that fails its state
  check refuses the whole request rather than saving the rest of the document
  and silently skipping the Stripe half.

Three refusals upstream applies on that same path were missing, and one was a
live data-loss bug:

- **Redacted credentials coming back.** `GET /settings` replaces every stored
  credential with `••••••••`. The edit path wrote back whatever it was given,
  so a client that read the settings, changed one field and sent them all back
  — the normal case for an admin UI — destroyed the Mailgun key, the SMTP
  password and the session secrets in one request. The placeholder now means
  "unchanged", matched against the same predicate browse redacts with.
- **Core settings.** `db_hash`, the session secrets, `members_email_auth_secret`.
  An administrator could overwrite the last of those over the API and invalidate
  every magic link in every inbox at once. Now `403` with upstream's message.
- **Unknown keys** were silently inserted as new settings; now `404` with
  upstream's `Problem finding setting: {key}`.
- Direct writes to the five `stripe_connect_*` settings are dropped, as upstream
  drops them — otherwise any admin key could point the site at an arbitrary
  Stripe account with no key verification at all. Dropped rather than refused,
  because Ghost's admin sends the whole document back and refusing would fail
  unrelated saves.
- `publicSiteAccess` and `limitStripeConnect` host limits are honoured, through
  the limits service already in the port.

And `DELETE /settings/stripe/connect` was missing upstream's guard: it now
refuses while any member still holds a paid subscription, on the API and on the
membership screen. The two halves combine badly without it — the subscriptions
keep billing on Stripe while the site loses the key that reconciles them, so
members pay for access it can no longer grant and no screen can show why.

The key-adoption work the native form already did — verifying against
`/v1/account`, clearing price ids that belonged to another account, registering
the webhook — moved into the shared module too, returning facts rather than an
HTTP response, since its two callers differ only in reporting one as JSON and
the other as a flash redirect.

## Validation of this batch

`cargo test --workspace --offline`: **831 passed, 0 failed, 2 ignored**, no
warnings. `node --test crates/ghost-admin-ui/tests/koenig-saving.cjs`: 11
passed. Eight module tests cover the handshake against Ghost 6.73's own client
ids and messages; five integration tests cover the endpoint, the token, the
three settings refusals and the disconnect guard. Two admin-UI tests were
removed as duplicates of the shared module's.

Checked live on `localhost:1971` from a release build, after a full
`.backup` snapshot. The earlier file-copy backup turned out to miss the WAL,
which is worth remembering: `cp` of the `.db` alone is not a backup of a
WAL-mode database.

- `GET /members/stripe_connect/` → `303` to `connect.stripe.com` with
  `response_type=code`, `scope=read_write`, Ghost's live client id and its
  redirect; `?mode=test` switches to the test client id; `?mode=sandbox` →
  `422 Validation (AllowedValues) failed for mode`; no session → `401`.
- The base64 `state` in the `Location` decoded to the row planted on that
  session, carrying the requested mode.
- The native form's start and finish paths behave identically, and a blob
  carrying another session's state is refused.
- `members_email_auth_secret` → `403`, value unchanged. `not_a_real_setting` →
  `404`, no row created. `stripe_connect_account_id: acct_attacker` was dropped
  while `description` in the same request saved — against this install's real
  connected account, which was untouched.
- `GET /settings` redacts `stripe_secret_key`, `mailgun_api_key` and
  `indexnow_api_key`; sending those redactions straight back left all three at
  their original lengths while a non-secret field in the same request saved.
  Compared by length, never read.

Every setting touched by the probe was restored, and the probe session deleted;
a final diff against the snapshot shows the install unchanged.

## GScan, all 266 rules — and why the last entry was wrong

The previous entry in this file said the legacy deprecation catalogue was
"skipped because this port does not claim to run those themes". **That
reasoning was wrong**, and it is worth writing down why, because it was
plausible.

gscan's `specs/v6` ends with `rules = _.merge({}, previousRules, rules)` over
v5, which merges v4, down to v1. **Every deprecation rule is live when
checking against v6.** They are not a description of old Ghost; they are what
tells the author of a theme being brought forward which helper stopped
working. A theme using `{{twitter_url}}`, `{{author.cover}}` or `{{@blog}}`
gets told so by Ghost and got nothing here. 147 of 266 rules were missing —
not only `GS001`, but the whole of `GS030`, `GS051`, `GS060`, `GS070`,
`GS100`, `GS110`, `GS120`, `GS130`, most of `GS010`, and 13 of 16 `GS090`.

All 266 are now ported.

### The table is generated, not transcribed

`tools/gscan-rules.js` reads the real tool's merged v6 spec and emits
`crates/ghost-render/src/gscan_rules.rs`: every code, level, fatal flag,
regex, helper name and wording. Hand-copying 266 rules is how a port drifts
from the thing it claims to match, so none of it is hand-copied. Three
transformations are applied and each refuses rather than guesses:

- `\/` is a JavaScript delimiter escape and means nothing in Rust.
- Literal braces are escaped. This is the one that mattered: JavaScript lets a
  bare `{` stand for itself, Rust's `regex` does not, and **93 of the first
  107 generated patterns failed to compile** — silently, logged and skipped,
  so a hundred rules did nothing while the suite stayed green. There is now a
  test that every pattern compiles *and* that every compilable one is loaded.
- Lookarounds and backreferences are a hard error in the generator, listed by
  name, so a rule that grows one in a later gscan fails the build instead of
  disappearing. Three are implemented by hand for that reason: the `{{asset}}`
  rule ("an `/assets/` path that is not already absolute"), the custom-fonts
  rule ("both properties present"), and the empty-`{{t}}` rule.

### What reading gscan properly corrected

Three behaviours in this port were the tidier-looking answer and wrong:

- **A missing `package.json`** failed one rule here and 23 upstream. Upstream
  is right: the report then lists what the file has to contain, not merely
  that it is absent.
- **`{{ghost_head}}`** was reported only for a theme that had templates. It is
  reported regardless — a theme with no templates is missing it as surely as
  one that forgot it.
- **Partial reachability.** gscan does not scan every `.hbs`. Check 005 resets
  the helper set and walks only routable templates — everything outside
  `partials/` — following `{{> name}}` from there. A partial nothing includes
  is never opened. Scanning every file made this port disagree with gscan in
  *both* directions at once on a real theme whose pages are `.html`: quiet
  about its missing `{{ghost_head}}` (the text sits in an orphan
  `partials/ghost_head.hbs` Ghost would never render), and reporting unknown
  globals out of files gscan never looked at. That theme was not in the set
  tested by hand — the parity test found it.

Six `GS090` rules remain unported and are listed by name in
`every_rule_in_the_spec_is_applied_by_some_check`, each with its reason. All
six need a real Handlebars scope — which block a path sits inside — rather
than the path itself: `{{author}}` while rendering a post is the same text as
`{{author}}` on an author page, where it is correct. That test fails if a
seventh is ever added to the list, so the exemption cannot grow quietly.

### What came out of this file

The generated table made two hand-maintained copies redundant, and both are
gone: the 103-entry `KOENIG_CSS` list of card classes with their levels and
`cardAsset` tags, and a second copy of `knownHelpers`. GS050, GS020, GS005 and
the three original GS090 rules now take their level, fatal flag and wording
from the same place as the other 260, so there is one source for every
sentence a theme author reads. Net: `gscan.rs` carries more checks in fewer
lines of data.

### Performance

The naive shape — every rule against every file — measured 33–47 ms per theme,
and the themes screen scans every installed theme on each render, so ten
themes meant nearly half a second of regex. A `RegexSet` compiles all 160-odd
patterns into one automaton and reports which matched after a single pass;
only those few then get an individual search for a line number. Same answers:
**0.78 ms** for the whole catalogue. Measuring rather than guessing also
showed the remaining cost is the pre-existing Handlebars parse for GS005
(18 ms), not the rules — and that two directory walks were reading every file
twice, now one.

### Validation of this batch

`cargo test --workspace --offline`: **854 passed, 0 failed, 2 ignored**, no
warnings.

The claim that matters is not "this port reports X" but "Ghost agrees", so
`crates/ghost-render/tests/gscan_parity.rs` runs **both tools** over the same
themes and compares code for code and level for level. It skips rather than
fails when the real gscan is not installed, so a contributor without a Ghost
checkout does not see red; `GSCAN_PATH` overrides the search.

- Six purpose-built fixtures: a clean theme, the legacy catalogue, the
  data-path checks, the structural rules including both fatal ones, every
  `package.json` mistake, and a theme with no `package.json`. **Identical
  output in both tools for all six.**
- **16 installed themes** compared the same way, including ones not written
  for this port. Identical. Earlier, by hand: 13 themes, 69 findings each way,
  zero misses and zero false positives; then a fixture built to trigger the
  deprecation catalogue, 72 findings each way, every level matching.
- New fatal rules reach activation automatically, since `theme::validate`
  already refuses a fatal report: recursive layouts, symlinks, unparseable
  translations and inline dynamic partials. Both bundled themes (`source`,
  `casper`) scan clean, so the live install's active theme is unaffected.

**One consequence worth stating plainly.** 44 of gscan's rules are fatal, and
the ported set is byte-for-byte those 44 — checked by listing both. 35 of them
are `GS001` deprecations, so **a theme written for Ghost 1.x that activated on
this port before will now be refused.** That is not an overreach: real Ghost
refuses it too, for the same rules, with the same message. But it is a change
in what this install accepts, and anybody carrying an old theme forward should
expect to be told what to fix rather than silently serving a half-broken site.
Both bundled themes and all 16 installed ones are unaffected.

One deliberate difference: the real gscan *aborts* on a symlinked theme file
("Failed theme files check") because its reader cannot open it. This port
reports `GS030-ASSET-SYM` and refuses activation, which is the outcome the
rule is for.

## Remaining audit work

Pending, and each for a stated reason rather than for want of time:

- The automation canvas interaction model — a deliberate UI difference. The
  audit itself notes branching must not be called a missing feature, since
  the upstream model is also a linear chain.
- Six `GS090` rules that need Handlebars scope rather than path matching,
  named and justified in the test that guards the list.
- MySQL, and executing Ghost's JavaScript adapter modules — intentional
  architecture, not gaps.
- Warehouse provisioning, and provider acceptance (real Stripe, production
  Mailgun, S3, Transistor, live federation peers). These need a controlled
  end-to-end environment and real credentials, not source changes.

