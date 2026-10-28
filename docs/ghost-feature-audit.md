# Ghost 6.73.0 feature comparison

Audit date: 28 October 2026 (Asia/Dhaka).

The latest stable release checked through GitHub's release API was
[Ghost 6.73.0](https://github.com/TryGhost/Ghost/releases/tag/v6.73.0), released
27 October 2026. The supplied installation at
`/Users/anisulkibria/Documents/Ghost/Ghost/current` also reports 6.73.0.
The Rust comparison includes the working tree, including recent changes,
against repository HEAD `2a642a2`.

This is a source and route audit, not an exhaustive acceptance test or a
certification of third-party services. “Missing” means no implementation of
the stated workflow was found. “Partial” means an implementation exists but
the specified behavior differs from Ghost. Some Ghost features require labs
flags, external services, or provider configuration.

## Access-control gaps to address first

**Remediation update — 28 October 2026:** the three findings below have now
been addressed. They are retained here as the original audit record.

- Admin API role checks now cover site-wide mutations, post ownership,
  contributor publishing, staff profile access, and automation administration.
- The editor and Admin API save explicit paid-tier assignments. Matching,
  unexpired grants are required for tier-restricted page bodies, comments,
  and newsletter recipients. Invalid published-tier updates are rejected
  before writes; content and assignment changes commit together.
- Markdown serves only the public body/preview, with private cache headers.
  Public feeds, generated excerpts, and theme content queries also apply
  access rules. The fix includes segment-gated content in Markdown exports.

Regression coverage is in `crates/ghost-api/tests/headless.rs`,
`crates/ghost-front/tests/access.rs`, and the newsletter-recipient test in
`crates/ghost-admin-ui/src/lib.rs`. The tables below preserve the original
audit baseline. For subsequent native implementations, verification and the
remaining gaps, see [the implementation worklog](native-parity-worklog.md).
SQLite remains the default; MySQL and Docker are not prerequisites.

| Finding | Evidence in the Rust port | Consequence |
| --- | --- | --- |
| Admin API role and ownership checks are incomplete | `crates/ghost-api/src/admin.rs:896` and `:1347` use `require_user` for editing posts and settings. The helper checks authentication, without Ghost's per-resource permission checks. The server-rendered admin and snippets have additional checks, but these do not protect other API handlers. | A signed-in lower-privilege staff account can reach API operations its role should not permit. This finding is based on the handlers; existing live accounts were not used to demonstrate it. |
| Specific-tier content access is not implemented | `crates/ghost-core/src/comments.rs:656` handles `paid` and `tiers` with the same “not free” check. The editor at `crates/ghost-admin-ui/src/lib.rs:5306` offers only public, members, and paid access. | Restricting content to particular paid tiers is unavailable; imported tier-restricted content is treated as accessible to any paid member. [Ghost supports specific-tier access](https://ghost.org/help/post-settings/). |
| Markdown content exports bypass membership access checks | The `.md` branch at `crates/ghost-front/src/routes.rs:66` finds a published post and returns its stored Markdown or plaintext without applying member access checks. | When LLM/Markdown support is enabled, this path can disclose gated text. Ghost 6.73's `core/frontend/services/routing/controllers/entry/markdown.js` explicitly gates this representation. This is conditional on that feature being enabled; no live private post body was retrieved during the audit. |

## Publishing and editor

| Feature | Status | What is missing or different | Evidence |
| --- | --- | --- | --- |
| Post revision history and restore | Missing | Save historical versions, browse edits, and restore a prior version. The site activity/history log is a different feature and already exists. | No revision creation or restore code was found in Rust post repositories or APIs. `post_revisions` appears in the content-deletion table list, but that does not implement revisions. [Ghost post history](https://ghost.org/help/post-settings/). |
| Duplicate posts and pages | Missing | Copy content, embeds, metadata, tags, and settings into a new draft through the admin workflow. | Rust has no copy route or post-list duplicate action. Ghost's admin routes expose `/posts/:id/copy` and `/pages/:id/copy`. [Ghost duplication](https://ghost.org/changelog/duplicating-posts/). |
| Bulk post/page actions and saved views | Missing | Multi-select changes to tags, access, featured state, publishing state, and reusable filtered views. | Rust post-list controls provide individual operations, status filters, sorting, and text filtering; no bulk handlers or saved-view workflow were found. [Ghost content organization](https://ghost.org/help/organizing-content/). |
| Unified publishing previews and test newsletters | Partial | Ghost's combined web/email/social preview experience and test-email workflow for the current post. | Draft preview rendering exists in `crates/ghost-front/src/routes.rs:520`, and newsletter template preview exists. The editor at `crates/ghost-admin-ui/src/lib.rs:5430` exposes a View link rather than Ghost's complete preview workflow. [Ghost publishing previews](https://ghost.org/help/delivering-emails/). |
| Editor internal-link search | Partial | Search posts/pages while inserting internal links and offer actual content suggestions. | `crates/ghost-admin-ui/assets/koenig-bridge.js:534` offers only the homepage for autocomplete; `:537` returns an empty result for `searchLinks`. Public site search is implemented separately and is not missing. |
| Publisher gift links for individual posts/pages | Missing | Share or reset a link granting access to one gated article without requiring signup. | No corresponding gift-link routes, frontend token gating, or editor/list action were found. Paid gift subscriptions are implemented and are a different feature. [Ghost publisher gift links](https://ghost.org/help/publisher-gift-links/). |

## Members, payments, and newsletters

| Feature | Status | What is missing or different | Evidence |
| --- | --- | --- | --- |
| Advanced member filtering, saved views, and bulk actions | Partial | Combine membership, tier, billing, newsletter, engagement, and date criteria; save views; bulk label, unsubscribe, export, or delete a filtered audience. | `crates/ghost-admin-ui/src/lib.rs:3556` loads the newest 500 members and filters the rendered list in the browser. Individual editing, labels, CSV import, and export exist. [Ghost member management](https://ghost.org/help/member-management/). |
| Member impersonation | Missing | Generate an administrator-authorized single-use link to view the site as a particular member. | No member sign-in URL API or member-profile impersonation action was found. Moderator impersonation parameters in comments are a separate mechanism. [Ghost member impersonation](https://ghost.org/help/impersonate-members/). |
| Direct Portal subscription updates and retention-offer application | Partial | Ghost's direct subscription update/resume/cancel/change workflow and applying a retention offer. | `crates/ghost-api/src/members.rs:109` sends subscription PUT requests to `stripe_unavailable`, which always returns an error. No `/subscriptions/:id/apply-offer` handler was found. Paid checkout and Stripe billing-portal session creation are implemented, so this does not mean all billing management is missing. |
| Stripe Checkout design customization | Missing | Background/accent colors, corners, fonts, and previewing checkout for each tier from admin. | No `/stripe/checkout/config`, `/branding`, or `/preview` API and no equivalent admin controls were found. [Ghost Checkout customization](https://ghost.org/help/customizing-stripe/). |
| Newsletter click tracking and editing links after sending | Missing | Track clicked links and update their redirect destinations after delivery. | The newsletter analytics UI explicitly states this is unimplemented at `crates/ghost-admin-ui/src/lib.rs:3539`. No link-redirection service or links API was found. [Ghost updating newsletter links](https://ghost.org/help/updating-links/). |
| Email bounce/complaint handling and suppression | Missing | Import provider delivery failures and complaints, disable affected recipients, and maintain/re-enable actual suppressions. | `crates/ghost-api/src/members.rs:1597` always reports no suppression. `:2572` acknowledges removing suppression without maintaining a suppression list. Mailgun sending itself exists. |
| Advanced newsletter audience selection and paywalled variants | Partial | Independently select labels/audiences and render the appropriate free-preview/paid version per audience. | The access-control remediation now limits tier-restricted sends to matching unexpired tier grants. The renderer still uses one content body with `paywall: false`; independent label-based targeting remains incomplete. [Ghost segmented newsletter delivery](https://ghost.org/help/delivering-emails/). |
| Scheduled delivery of purchased gifts | Partial | Choose a future delivery date for a gift email. | Gift purchase, personalized messages, several durations, and redemption exist. `CheckoutRequest` at `crates/ghost-api/src/members.rs:150` has no delivery-date field; the payment-completion path sends the gift email immediately. [Ghost gift delivery](https://ghost.org/help/gift-subscriptions/) supports scheduling. |
| Staff self-service forgotten-password recovery | Missing | Request a password-reset email while signed out and redeem a reset token. | The admin UI exposes signin/verification and signed-in password changes, but no forgotten-password route. The Admin API lacks Ghost's `/authentication/password_reset` handlers. Invitations and staff email verification are implemented. |

## Growth, analytics, design, and integrations

| Feature | Status | What is missing or different | Evidence |
| --- | --- | --- | --- |
| Social web / ActivityPub | Missing | Publication federation, Reader, Notes, follows, likes, reposts, replies, and social-web discovery. | No ActivityPub service or routes were found. Growth settings explicitly render Network as planned at `crates/ghost-admin-ui/src/lib.rs:12828`. [Ghost social web](https://ghost.org/help/social-web/). |
| Ghost Explore publication discovery | Partial | Actually announce the publication and share opted-in growth data with Explore. | `crates/ghost-admin-ui/src/lib.rs:12866` saves opt-in settings, but no Explore ping job/client was found. Saving a checkbox does not perform registration or synchronization. |
| Native analytics parity | Partial | Cookie-free daily unique-visitor counting, real-time visitor reporting, and the fuller source/conversion/device/location analytics service. | `crates/ghost-front/src/analytics.rs:15` uses a year-long `ghost_vid` cookie, and tracking stores path, viewer, referring host, and audience. Views, members, MRR, opens, source tables, and audience/date filters exist. This differs from [Ghost's cookie-free analytics model](https://ghost.org/help/native-analytics/) and its installed stats services. |
| Automation authoring and email-design parity | Partial | Ghost's visual flow canvas, rich email editor, and separate automated-email design controls/APIs. | `crates/ghost-admin-ui/src/automations.rs` uses form-based step editing and Markdown email bodies. `crates/ghost-api/src/automations.rs:134` returns no design-setting reference. The runtime engine, tier-specific triggers, run history, and SQL performance stats exist and must not be counted as missing. [Ghost automations](https://ghost.org/help/automations-beta/). |
| Real responsive-image processing | Partial | Generate resized images for Ghost's `/content/images/size/...` URLs rather than returning the original bytes. | `crates/ghost-front/src/routes.rs:600` strips the size prefix and serves the original file. Uploads and CSS-sized previews already work. |
| Design live preview and custom theme-image uploads | Partial | Preview the theme while changing settings, and upload images for theme-declared custom image settings. | The design page renders forms without a live site-preview pane. `design_field_row` at `crates/ghost-admin-ui/src/lib.rs:7844` implements custom image settings as URL fields. Publication icon/logo/cover uploads and cover Unsplash selection are already implemented. |
| Staff toolbar on the public site | Missing | Ghost's frontend toolbar for staff navigation/editing while viewing the publication. | No corresponding frontend injection or toolbar implementation was found. The [Ghost changelog](https://ghost.org/changelog/) lists the admin toolbar feature. |
| Full Admin API resource coverage | Partial | REST access to resources managed only through this port's HTML forms, and compatibility with the wider Ghost administration API. | `crates/ghost-api/src/admin.rs:13` and API composition in `crates/ghost/src/main.rs` lack multiple Ghost resource families. See the inventory below. Existing Content API, core post APIs, snippets, comments, automations, uploads, integration keys/JWT, and webhooks are implemented. |

## Admin API gaps

Compared with the installed Ghost 6.73 route registry at
`core/server/web/api/endpoints/admin/routes.js`, no matching Rust Admin API
families were found for these examples:

- Members CRUD/bulk operations and member sign-in URLs.
- Labels, tiers, offers, invitations, and roles resources.
- Post/page copy and bulk operations.
- Post revisions in API responses and restoring them.
- Links and bulk link editing.
- Stripe checkout design configuration, branding, and preview.
- Publisher article gift links and their rotation/reset.
- General stats/Tinybird APIs.
- Theme management, database import/export, and action-log APIs matching Ghost's REST endpoints.
- Notifications, mentions, incoming recommendations, and app installations.
- Separate automated-email/design/action-link APIs.
- Staff forgotten-password reset and ownership-transfer API operations.

Many of these functions have partial equivalents in HTML settings pages.
An HTML form handler does not make a Ghost SDK's corresponding REST request
compatible. Content API and Admin API gaps should also be tracked separately.

## Intentional or infrastructure differences

- **Stripe Connect onboarding is deferred by request.** Direct secret and
  publishable-key setup is implemented. The hosted Ghost flow is hidden until
  this project has its own integration; this is not a newly discovered regression.
- **MySQL support is absent.** The workspace enables SQLx's SQLite backend and
  uses `SqlitePool` throughout. It does not match Ghost's production MySQL
  deployment model.
- **Storage adapters and analytics infrastructure differ.** The port serves
  local filesystem assets and local SQL analytics, rather than implementing
  Ghost's storage-adapter ecosystem and Tinybird synchronization service.
- Ghost 6.73 also contains optional/experimental services such as machine
  payments. The Rust Features page declares a `machinePayments` labs toggle,
  but a corresponding payment protocol service was not found. Whether to target
  these depends on the desired labs/service scope, rather than assuming every
  service present in Ghost's source is enabled in every Ghost installation.

## Features already implemented

The following have substantial implementations and should not be put on a
“missing features” list merely because the UI differs:

- Posts/pages, the Koenig editor and cards, snippets, Unsplash, uploads,
  featured image metadata, autosave, publishing, and scheduled publishing.
- Tags, authors, theme rendering, theme installation, custom settings,
  navigation, redirects/routes configuration, RSS, sitemaps, SEO metadata,
  code injection, public search feeds, and private-site access codes.
- Free membership, magic links, member profiles, custom member fields,
  newsletter preferences, CSV import/export, paid tiers, Stripe checkout,
  webhook synchronization, offers, trials, tips, and gift-subscription flows.
- Multiple newsletters, template controls, SMTP/Mailgun delivery, basic opens
  tracking, unsubscribe links, and audience feedback.
- Comments, replies, reactions, moderation, and notification emails.
- Automation scheduling/execution, wait/email steps, selected-paid-tier
  triggers, run history, and basic analytics.
- Staff invitations, roles in the server-rendered UI, email verification,
  activity history, integrations, API keys/JWT, and signed webhooks.

“Implemented” here means code exists for the capability. This audit did not
retest every provider configuration or claim full Ghost behavior for each one.

## Suggested order

1. **Completed:** close the audited Admin API permission and protected-content gaps.
2. Complete Portal subscription changes, advanced email segmentation,
   suppression, and click tracking.
3. Add revision history, member filters/views/bulk actions, duplication, and
   the missing REST APIs needed by integrations.
4. Improve checkout design, previews, responsive images, and automation editing.
5. Add social-web infrastructure, Explore synchronization, and the intentionally
   deferred or optional service integrations according to product priorities.
