# Integrations and headless frontends

Ghost's [integration directory](https://ghost.org/integrations/) contains several
kinds of connections. API clients use a custom integration's keys, automation
and build services subscribe to webhooks, and many analytics, forms, community,
and ecommerce tools use code injection or HTML/embed cards. They are configured
individually; creating an API key does not install a third-party service.

## Connect a frontend

Open **Settings → Integrations**, add a custom integration, and copy its Content
API key and API URL. Content keys are publishable. Admin keys stay on your server.
Both keys are created together, including when an integration is created through
the Admin API.

This example uses `fetch` and works in a server-side Next.js, Nuxt, Astro, or other
JavaScript build. `GHOST_URL` is the site origin, including any installation
subdirectory, without `/ghost/api`.

```js
const origin = process.env.GHOST_URL.replace(/\/$/, '');
const key = process.env.GHOST_CONTENT_API_KEY;
let page = 1;
const posts = [];
do {
    const query = new URLSearchParams({
        key, limit: '100', page: String(page), include: 'tags,authors',
        formats: 'html,plaintext'
    });
    const response = await fetch(`${origin}/ghost/api/content/posts/?${query}`, {
        headers: {'Accept-Version': 'v6.0'}
    });
    if (!response.ok) throw new Error(`Ghost API returned ${response.status}`);
    const result = await response.json();
    posts.push(...result.posts);
    page = result.meta.pagination.next;
} while (page);
```

Read one post with `/ghost/api/content/posts/slug/{slug}/?key=...`.
`fields=title,slug,url` selects the returned fields on reads and lists.
`formats=plaintext` requests text instead of HTML. `include=authors,tags` includes
relations and their primary values. Returned post, author, tag, and image URLs
are absolute; embedded image URLs in post HTML also point to the Ghost origin.

A Content key exposes published content only. It does not unlock paid or
members-only bodies. A post with a `<!--members-only-->` marker exposes its
public preview; otherwise its restricted body is empty. Internal tags and
Lexical source are excluded. Use Ghost's own frontend for its membership flows;
headless frontends need their own implementation of frontend functionality.
See the [Ghost headless guide](https://docs.ghost.org/jamstack/).

## Publish from an integration

Use the Admin API with `Authorization: Ghost <JWT>`. Split the Admin key at `:`,
put the id in the JWT `kid` header, and sign with HS256 using the secret decoded
from hex. Set `iat` to now, `exp` to at most five minutes later, and `aud` to
`/admin/`. The official `@tryghost/admin-api` client handles this for you.

```json
{"posts":[{"title":"Hello","html":"<p>Published through the API</p>","status":"published"}]}
```

POST this to `/ghost/api/admin/posts/?source=html`. PUT edits to
`/ghost/api/admin/posts/{id}/`. Use `formats=lexical` to read saved Lexical source,
or `formats=html,plaintext` to read rendered formats. Explicit null metadata
values clear those fields; omitted fields retain their current values.
Scheduled posts require a future ISO8601 `published_at` date.

The standard paths work with or without a trailing slash. The server also
accepts legacy `/ghost/api/v5/content/` and `/ghost/api/v5/admin/` prefixes.
Content responses allow cross-origin reads, including `Accept-Version`
preflights. Responses include `Content-Version`.

## Trigger rebuilds and automations

Add a webhook in the custom integration screen, or POST to
`/ghost/api/admin/webhooks/` with an integration token:

```json
{"webhooks":[{"event":"post.published","target_url":"https://frontend.example/hooks/ghost","secret":"your-webhook-secret"}]}
```

The integration is inferred from the token. An integration can edit and delete
its own webhooks. A staff administrator can manage integrations and their hooks.
API publishing sends publish events just as editor publishing does. Published
edits, unpublishing, deletion, scheduling, and tag attachment changes also notify
listeners. Public content changes send `site.changed` for whole-site rebuilds.

Signed deliveries include `X-Ghost-Signature: sha256=<digest>, t=<milliseconds>`.
Verify the HMAC-SHA256 of the raw request body followed by the timestamp using
your webhook secret. Reject old timestamps in your receiver. Delivery failures
are recorded in the integration screen and transient failures are retried.

## Built-in services

Configure Slack's incoming webhook, Unsplash, FirstPromoter, Pintura, and
Transistor in their individual settings screens. FirstPromoter's account id is
passed to Portal when enabled. Pintura requires your licensed JavaScript and
stylesheet URLs; those assets are loaded before Koenig and passed to its image
cards. Transistor requires a private-feed URL template and member provisioning
in the external service.

External accounts, licences, deployed webhook receivers, and provider-specific
credentials are still required. The automated checks cover the local API and
webhook protocols; they do not certify every external service in the directory.

## Verify locally

```sh
cargo test -p ghost-api --test headless
cargo test -p ghost-api -p ghost-admin-ui -p ghost-render --lib
node --test crates/ghost-admin-ui/tests/koenig-saving.cjs
```

The webhook test opens an isolated localhost HTTP listener. All API tests use
in-memory databases and do not publish content on your running site.
