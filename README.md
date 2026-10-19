# Ghost-rust

A Rust port of the [Ghost](https://ghost.org) publishing platform, converted
from the Node.js source in `Ghost-main`. It runs real Ghost themes (verified
against the `flux` theme), exposes the Content and Admin REST APIs, and ships
a built-in server-rendered admin UI.

## Install

Three guides. Pick the one that matches what you are doing:

| | Guide | Result |
|---|---|---|
| **1** | [Local install](#guide-1--local-install) | Running on your own machine in a couple of minutes |
| **2** | [Production server](#guide-2--production-server-with-a-domain-and-https) | A real site on your domain, with HTTPS |
| **3** | [Subdomain or subdirectory](#guide-3--subdomain-or-subdirectory) | `blog.example.com` or `example.com/blog` |

---

## Guide 1 — Local install

For trying it out, writing a theme, or development. No configuration needed.

### Step 1 — Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Rust needs a C linker to produce a binary, which you already have if you build
software on this machine. If not: `xcode-select --install` on macOS,
`sudo apt install build-essential` on Debian or Ubuntu.

Nothing else is required: no Node, no database server, no system OpenSSL.

### Step 2 — Download the code and run it

```bash
git clone https://github.com/anisulkibria/rust-ghost.git ghost-rust
cd ghost-rust
cargo run
```

The first build takes a few minutes. After that you should see:

```
INFO ghost: database ready at .../content/data/ghost-dev.db
INFO ghost: seeded initial data (roles, settings, fixture posts)
INFO ghost: theme 'source' activated
INFO ghost: Ghost-rust ready in 0.6s — http://localhost:2368 (admin at http://localhost:2368/ghost/)
```

### Step 3 — Create your account

Open **http://localhost:2368/ghost/** and fill in the setup form. That creates
the owner account and names the site.

The site itself is at **http://localhost:2368**.

### Step 4 — Optional: fill it with demo content

```bash
cargo run -- seed-demo
```

Adds ten posts across seven tags, so archives, tag pages and theme layouts
have something to show.

**That's it.** The database, images and themes all live under `content/`.
Delete `content/data/ghost-dev.db` to start over.

To try the Content API, the seeded key is printed to the boot log and shown in
Admin → Integrations:

```bash
curl "http://localhost:2368/ghost/api/content/posts/?key=<content key>"
```

---

## Guide 2 — Production server with a domain and HTTPS

A real site: your domain, a trusted certificate, running as a service that
starts on boot.

HTTPS is handled by the server itself — certificates come from Let's Encrypt
and renew automatically. There is **no nginx to configure and no certbot to
schedule**. (If you would rather use a reverse proxy, you still can — see
[Behind a reverse proxy](#behind-a-reverse-proxy).)

**Before you start, you need:**

- A server with a public IP — any Linux distribution with systemd.
- A domain name whose DNS you can edit.
- Ports 80 and 443 reachable from the internet.
- Root or `sudo` on the server.

**Sizing.** The running site is small: 1 vCPU and 512 MB of RAM is plenty, and
the binary is 33 MB. *Compiling* it is the demanding part — around 470 crates,
so budget roughly 2 GB of RAM and 5 GB of free disk for the build. On a server
smaller than that, build somewhere else and copy the binary across (Step 4
covers both routes).

The examples use `example.com` and `/var/www/example.com`. Substitute your own.

### Step 1 — Point your domain at the server

At your DNS provider, add an `A` record for the domain pointing at the
server's IPv4 address (and an `AAAA` record if you have IPv6):

| Type | Name | Value |
|---|---|---|
| A | `example.com` | `203.0.113.10` |

**Do this first.** A certificate cannot be issued until the domain resolves to
the server, and DNS changes take anywhere from a minute to an hour to spread.

Check it from the server:

```bash
dig +short example.com
```

It should print your server's IP. If it prints nothing, wait and try again.

> **Using Cloudflare?** Set the record to **DNS only** (grey cloud), not
> Proxied (orange cloud). The orange-cloud proxy intercepts the certificate
> challenge and issuance will fail.

### Step 2 — Install the build tools

Git to fetch the code, and a C linker, which Rust needs to produce a binary. A
minimal server image has neither:

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y git curl build-essential

# RHEL / Fedora / Alma
sudo dnf install -y git curl gcc
```

Then Rust itself:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustc --version
```

Nothing else is needed — no Node, no database server, no system OpenSSL.

### Step 3 — Download the CMS

```bash
git clone https://github.com/anisulkibria/rust-ghost.git ~/ghost-rust
cd ~/ghost-rust
```

That is the whole CMS: server, admin UI, two themes, and the migrations. There
is no separate package to fetch.

### Step 4 — Build and install the binary

```bash
cargo build --release
```

This compiles ~470 crates and takes a few minutes the first time. When it
finishes, put the binary on the system path:

```bash
sudo install -m 755 target/release/ghost /usr/local/bin/ghost
ghost version
```

> **If the build runs out of memory** — the usual symptom on a 512 MB–1 GB
> server is the compiler being killed with `signal: 9` — either add swap
> temporarily:
>
> ```bash
> sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile
> sudo mkswap /swapfile && sudo swapon /swapfile
> ```
>
> or skip Steps 2–4 on the server entirely: run `cargo build --release` on
> another machine with **the same OS and CPU architecture**, then copy just the
> binary over and carry on from Step 5:
>
> ```bash
> scp target/release/ghost you@example.com:/tmp/ghost
> ssh you@example.com 'sudo install -m 755 /tmp/ghost /usr/local/bin/ghost'
> ```
>
> You will still need the repository's `content/themes/` directory on the
> server for Step 5, so copy that across too. A binary built on macOS will not
> run on Linux.

### Step 5 — Create a service user and a site directory

Running as a dedicated account with no login shell keeps the site's files
separate from the rest of the machine:

```bash
sudo useradd --system --create-home --shell /usr/sbin/nologin ghost
sudo mkdir -p /var/www/example.com
sudo chown -R ghost:ghost /var/www/example.com
```

Copy the bundled themes into the site directory (the server needs at least
one, and `source` is the default):

```bash
sudo -u ghost mkdir -p /var/www/example.com/content/themes
sudo cp -r ~/ghost-rust/content/themes/source /var/www/example.com/content/themes/
sudo cp -r ~/ghost-rust/content/themes/casper /var/www/example.com/content/themes/
sudo chown -R ghost:ghost /var/www/example.com/content
```

This directory is everything that is yours — config, database, uploads,
themes. It is the only place the server writes, and the only thing you need to
back up.

### Step 6 — Write the config

```bash
cd /var/www/example.com
sudo -u ghost ghost setup --url https://example.com --ssl --email you@example.com
```

That writes `config.production.json`:

```json
{
  "url": "https://example.com",
  "ssl": { "email": "you@example.com" },
  "server": { "host": "0.0.0.0" },
  "database": {
    "client": "sqlite3",
    "connection": { "filename": "content/data/ghost.db" }
  },
  "paths": { "contentPath": "content" },
  "mail": { "transport": "Direct" }
}
```

- `--ssl` turns on automatic certificates. The domain comes from `--url`, the
  port defaults to 443, and port 80 redirects to HTTPS.
- `--email` is where Let's Encrypt sends expiry warnings if renewal ever
  breaks. Optional, but worth setting.
- Leave off the flags and `ghost setup` asks you for each value instead.

> **Also want `www.example.com` to work?** Add a `CNAME` record for `www`
> pointing at `example.com`, then list both names so the certificate covers
> them — a certificate is only valid for the names it was issued for:
>
> ```json
> "ssl": {
>   "email": "you@example.com",
>   "domains": ["example.com", "www.example.com"]
> }
> ```
>
> Links the site writes always use the `url` you set, so visitors arriving on
> `www` see `example.com` links. That is deliberate: one canonical hostname.

### Step 7 — Check the server can serve it

```bash
sudo -u ghost GHOST_ENV=production ghost doctor
```

```
  ok    config         loaded for env 'production'
  ok    url            https://example.com
  ok    content        /var/www/example.com/content is writable
  ok    database       /var/www/example.com/content/data/ghost.db (migrated)
  ok    themes         2 installed
  ok    port           0.0.0.0:443 is free
  ok    port           0.0.0.0:80 is free
  ok    dns            example.com → 203.0.113.10
  --    certificates   none cached yet — one will be requested on the first HTTPS request

All checks passed.
```

**Do not skip this.** It catches the two things that actually break installs —
DNS pointing somewhere else, and ports already taken — and it names the fix for
anything it finds. Re-run it after any config change.

### Step 8 — Open the firewall

```bash
# Debian / Ubuntu
sudo ufw allow 80,443/tcp

# RHEL / Fedora / Alma
sudo firewall-cmd --permanent --add-service={http,https} && sudo firewall-cmd --reload
```

If your server is on a cloud provider, check its own firewall or security
group as well.

### Step 9 — Install the service

```bash
cd /var/www/example.com
sudo GHOST_ENV=production ghost systemd --write
sudo systemctl enable --now ghost
systemctl status ghost
```

The generated unit runs as the `ghost` user, restarts on failure, starts on
boot, and grants `CAP_NET_BIND_SERVICE` so an unprivileged account can bind
ports 80 and 443 without running as root.

Run `ghost systemd` without `--write` to read the unit before installing it.

> The unit is sandboxed: `ProtectSystem=strict` makes the filesystem read-only
> apart from the site directory, and `ProtectHome=yes` hides `/home`. If you
> keep your site under `/home` rather than `/var/www`, delete the
> `ProtectHome` line — otherwise the service starts but cannot read its own
> files.

### Step 10 — Create your account

Open **https://example.com/ghost/** and fill in the setup form.

The first HTTPS request is also when the certificate is issued, so that first
page load takes a few seconds. Watch it happen:

```bash
journalctl -u ghost -f
```

Your site is now live at **https://example.com**.

### Step 11 — Set up email

Email carries staff invitations, password resets and member sign-in links, so
a site with members needs it. Configure it in the admin at **Settings →
Email** — not in the config file. See [Email](#email) for which transport to
pick.

---

## Guide 3 — Subdomain or subdirectory

### On a subdomain — `blog.example.com`

**Exactly the same as [Guide 2](#guide-2--production-server-with-a-domain-and-https).**
A subdomain is just a hostname, so there are no extra steps. Two things change:

1. **Step 1**, the DNS record is for the subdomain:

   | Type | Name | Value |
   |---|---|---|
   | A | `blog.example.com` | `203.0.113.10` |

2. **Step 6**, the url is the subdomain:

   ```bash
   sudo -u ghost ghost setup --url https://blog.example.com --ssl --email you@example.com
   ```

Everything else — the service user, `doctor`, `systemd`, the firewall, the
certificate — is identical. The admin ends up at
`https://blog.example.com/ghost/`.

### Under a subdirectory — `example.com/blog`

Also the same as Guide 2, with **one** change: put the path on the url.

```bash
sudo -u ghost ghost setup --url https://example.com/blog --ssl --email you@example.com
```

That moves the whole install:

```
https://example.com/blog/          site
https://example.com/blog/ghost/    admin
https://example.com/blog/rss/      feed
```

The DNS record is for `example.com`, as in Guide 2 — a path is not something
DNS knows about.

Three things are worth knowing:

- **Reserved paths.** `/ghost`, `/content`, `/assets`, `/members`, `/public`,
  `/rss` and the other segments the app serves itself cannot be used as the
  base path. Picking one is refused at startup with the reason.

- **Nothing is served at the domain root.** `https://example.com/` redirects to
  `https://example.com/blog/`. If something else already lives at the root,
  put a reverse proxy in front — see
  [Proxying a subdirectory](#proxying-a-subdirectory).

- **Moving the site later is a config change, not a migration.** Internal URLs
  are stored *without* the base path (`/content/images/x.png`, `/about/`) and
  it is applied when a page is rendered. The same database serves correctly
  from `example.com`, from `example.com/blog`, or back again — change `url`
  and restart.

Cookies are scoped to the base path rather than to `/`, so two installs can
share one domain under different paths without logging each other out.

---

## Commands

| Command | What it does |
|---|---|
| `ghost` | Run the server |
| `ghost setup` | Write a config file: domain, HTTPS, port |
| `ghost doctor` | Check this machine can serve the configured site |
| `ghost systemd` | Print a systemd unit, or install it with `--write` |
| `ghost seed-demo` | Fill the site with demo content |
| `ghost version` | Print the version |
| `ghost --help` | All of the above, with every flag |

`ghost doctor` exits non-zero when something must be fixed, so it works in a
provisioning script.

Which config file is read depends on `GHOST_ENV`: `config.production.json`
when it is `production`, `config.development.json` otherwise (the default).
`ghost setup` writes the production one unless you pass `--env development`.

---

## Email

Configure this in the admin at **Settings → Email**, then send yourself a test
from that screen before relying on it.

| Transport | When to use it |
|---|---|
| **Direct** (default) | Delivers straight to the recipient's mail server. Needs no account, but mail from a new IP is usually spam-filtered — fine for testing, not for members. |
| **SMTP** | Any mail provider: Fastmail, Postmark, SES, your own server. Needs host, port, user, password, and STARTTLS or TLS. **Use this in production.** |
| **Mailgun** | Bulk newsletter sending, configured separately under Settings → Newsletters. |

---

## Certificate options

`"ssl": true` is the whole configuration for a normal site. Certificates are
cached in `content/acme/` and survive restarts; renewal happens inside the
running process.

The expanded form, when you need it:

```json
"ssl": {
  "email": "you@example.com",
  "domains": ["example.com", "www.example.com"],
  "staging": false,
  "redirect_http": 80,
  "cache_dir": "content/acme"
}
```

| Field | Meaning |
|---|---|
| `email` | Where expiry warnings go if renewal breaks. |
| `domains` | Hostnames to cover. Defaults to the host in `url`. |
| `staging` | Use Let's Encrypt's test CA (see below). |
| `redirect_http` | Port for the HTTP→HTTPS redirect. `0` disables it. |
| `cache_dir` | Where issued certificates are stored. |

**Test with `"staging": true` first** if you are iterating on a setup.
Let's Encrypt rate-limits failed orders on the production CA; the staging CA is
far looser. Its certificates are deliberately not trusted by browsers — once a
staging run succeeds, remove the flag and restart.

### Bringing your own certificate

```json
"ssl": {
  "mode": "manual",
  "cert": "/etc/ssl/example.com/fullchain.pem",
  "key":  "/etc/ssl/example.com/privkey.pem"
}
```

This is also how to get HTTPS on `localhost`, which no public CA will issue
for — generate a certificate with
[mkcert](https://github.com/FiloSottile/mkcert) and point `mode: "manual"` at
it.

---

## Behind a reverse proxy

Terminating TLS in the server is simpler, but a proxy is fully supported. Turn
`ssl` off and listen on loopback:

```json
{
  "url": "https://example.com",
  "ssl": false,
  "server": { "host": "127.0.0.1", "port": 2368 }
}
```

`url` stays `https://` because that is what visitors use and what every link
the site writes must say. `ssl: false` only means *this* process serves plain
HTTP to the proxy.

Caddy, which handles its own certificates:

```caddy
example.com {
    reverse_proxy 127.0.0.1:2368
}
```

nginx:

```nginx
server {
    listen 443 ssl;
    server_name example.com;
    ssl_certificate     /etc/letsencrypt/live/example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/example.com/privkey.pem;

    client_max_body_size 100m;   # theme and media uploads

    location / {
        proxy_pass http://127.0.0.1:2368;
        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
    }
}
```

### Proxying a subdirectory

Both nginx conventions work, so you do not have to know which one you wrote —
the app accepts requests whether or not the prefix was stripped:

```nginx
# forwards /blog/ through (no trailing slash on proxy_pass)
location /blog/ { proxy_pass http://127.0.0.1:2368; }

# strips /blog/ before forwarding (trailing slash)
location /blog/ { proxy_pass http://127.0.0.1:2368/; }
```

Either way the URLs the app writes carry `/blog`, so links stay correct.

---

## Backups

Everything that matters is the site directory. Stop the server for a consistent
SQLite copy, or use `sqlite3 .backup`, which is safe on a running database:

```bash
sqlite3 content/data/ghost.db ".backup '/backup/ghost-$(date +%F).db'"
tar czf /backup/content-$(date +%F).tar.gz content/images content/media content/files content/themes
```

Keep `config.production.json` too. `content/acme/` does not need backing up —
certificates are re-requested if missing.

The admin also has **Settings → Advanced → Export** for a Ghost-format JSON
export of posts and tags, which is what you want for moving content between
installs rather than for disaster recovery.

---

## Upgrading

```bash
cd ghost-rust && git pull && cargo build --release
sudo systemctl stop ghost
sudo install -m 755 target/release/ghost /usr/local/bin/ghost
sudo systemctl start ghost
journalctl -u ghost -n 50
```

Database migrations run automatically at startup. Back the database up first.

---

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `cannot listen on 0.0.0.0:443: Permission denied` | Ports under 1024 need a capability. Use the systemd unit from `ghost systemd`, or `sudo setcap 'cap_net_bind_service=+ep' $(which ghost)`. |
| `cannot listen on …: Address in use` | Something else holds the port — often an existing nginx or Apache. `sudo lsof -i :443`. |
| Browser shows a certificate warning | Either `"staging": true` is still set, or the certificate has not been issued yet. Check `journalctl -u ghost` for the ACME exchange. |
| Certificate never issues | Almost always DNS or a blocked port 443. Run `ghost doctor`; it checks both. Cloudflare's orange-cloud proxy also intercepts the challenge — set that record to "DNS only". |
| `ssl is on but url is 'http://…'` | Change `url` to `https://`. The message names the exact value. |
| `'/ghost' is reserved … as a base path` | Those path segments belong to the app. Pick another, e.g. `/blog`. |
| Admin loads but sign-in bounces back to the form | The session cookie is scoped to the base path; make sure you are visiting the `url` from the config, not an IP or a different hostname. |
| Images 404 after moving the site | Re-check `url`. Image paths are stored without the base path and prefixed at render time, so a wrong `url` breaks them all at once. |
| Theme fails to load | The server falls back to another installed theme and logs the reason, so the admin stays reachable. Fix it in Settings → Themes. |
| Members never get magic-link emails | The default `Direct` transport is usually spam-filtered. Configure SMTP in Settings → Email. |

Raise the log level for more detail:

```bash
sudo systemctl edit ghost     # add: Environment=RUST_LOG=debug
```

---

## Configuration reference

`config.<env>.json` in the working directory, where `<env>` is `GHOST_ENV`
(default `development`; `ghost setup` writes `production`). Every key can also
be set as an environment variable with `GHOST_` and `__` for nesting —
`GHOST_SERVER__PORT=8080`, `GHOST_DATABASE__CONNECTION__FILENAME=/data/g.db`.

| Key | Default | Meaning |
|---|---|---|
| `url` | `http://localhost:2368` | Public url, including any subdirectory. Decides the base path and every URL the site writes. |
| `ssl` | `false` | `true` for automatic certificates, `false` for plain HTTP, or an object (see above). |
| `server.host` | `127.0.0.1` | Address to bind. `ghost setup` writes `0.0.0.0` when serving the public directly. |
| `server.port` | 443 with TLS, else 2368 | Port to bind. |
| `database.connection.filename` | `content/data/ghost.db` | SQLite file, relative to the config's directory. |
| `paths.contentPath` | `content` | Themes, images, data, the ACME cache. |
| `mail.transport` | `Direct` | Starting value only; email is configured in the admin. |

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
                  + Content-API JSON serializers, base-path URL handling
                  (`url`: the subdirectory install's single choke point)
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
  ghost           the binary: CLI (setup/doctor/systemd), boot sequence,
                  HTTP composition, and listening — plain, TLS from files,
                  or TLS with automatic certificates
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
- **HTTPS**: terminated in-process, with certificates from Let's Encrypt
  requested and renewed automatically (`"ssl": true`). Ghost itself has no
  equivalent — it expects a reverse proxy and an external certbot. A proxy is
  still supported; see [Behind a reverse proxy](#behind-a-reverse-proxy).
- **Subdirectory installs**: a path on `url` works out of the box, which in
  Ghost itself takes a good deal of proxy and asset configuration to approach.
  See [Guide 3](#guide-3--subdomain-or-subdirectory).

## Tests

```bash
cargo test --workspace
```

250 unit tests: NQL parser/SQL compilation (incl. injection attempts), slug
rules, routes.yaml parsing + router matching, redirects, template resolution,
preprocessor rewrites, markdown pipeline, handlebars helper mechanics,
base-path joining/stripping with the HTML rewrite in both directions, and
`url` + `ssl` resolving into a listening plan.
