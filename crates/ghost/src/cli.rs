//! The `ghost` command line: `setup`, `doctor`, `systemd`, `seed-demo`.
//!
//! These exist so that installing the CMS is a short, checkable sequence
//! rather than a research project. `setup` writes the config, `doctor` tells
//! you whether this machine can actually serve it, and `systemd` emits the
//! unit — including the one line (`AmbientCapabilities`) that otherwise makes
//! port 443 fail for a non-root user.
use ghost_core::config::TlsPlan;
use ghost_core::Config;
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

pub enum Command {
    /// Run the server (the default).
    Start,
    Setup(Args),
    Doctor,
    Systemd(Args),
    SeedDemo,
    Help,
    Version,
}

/// `--key value` and `--flag` parsed into a map; `--flag` stores `"true"`.
pub struct Args(HashMap<String, String>);

impl Args {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
    fn flag(&self, key: &str) -> bool {
        matches!(self.0.get(key).map(String::as_str), Some("true"))
    }
}

pub fn parse() -> Command {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (name, rest) = match argv.split_first() {
        None => return Command::Start,
        Some((n, r)) => (n.as_str(), r),
    };

    let mut args = HashMap::new();
    let mut i = 0;
    while i < rest.len() {
        let Some(key) = rest[i].strip_prefix("--") else {
            i += 1;
            continue;
        };
        // `--key=value`, `--key value`, or a bare `--flag`
        if let Some((k, v)) = key.split_once('=') {
            args.insert(k.to_string(), v.to_string());
            i += 1;
            continue;
        }
        match rest.get(i + 1) {
            Some(v) if !v.starts_with("--") => {
                args.insert(key.to_string(), v.clone());
                i += 2;
            }
            _ => {
                args.insert(key.to_string(), "true".into());
                i += 1;
            }
        }
    }
    let args = Args(args);

    match name {
        "setup" | "install" => Command::Setup(args),
        "doctor" | "check" => Command::Doctor,
        "systemd" | "service" => Command::Systemd(args),
        "seed-demo" => Command::SeedDemo,
        "start" | "serve" | "run" => Command::Start,
        "version" | "--version" | "-V" => Command::Version,
        _ => Command::Help,
    }
}

pub fn print_help() {
    println!(
        r#"ghost — a Rust port of the Ghost publishing platform

USAGE
    ghost [command]

COMMANDS
    start            Run the server (default when no command is given)
    setup            Write a config file: domain, HTTPS, port
    doctor           Check this machine can serve the configured site
    systemd          Print a systemd unit for running ghost as a service
    seed-demo        Fill the site with demo content, then exit
    version          Print the version

SETUP OPTIONS (all optional; omitted values are asked for interactively)
    --url <url>      Site url, e.g. https://example.com or https://example.com/blog
    --ssl            Get HTTPS certificates automatically (Let's Encrypt)
    --no-ssl         Serve plain HTTP (correct behind a TLS-terminating proxy)
    --email <addr>   Contact address for certificate expiry notices
    --port <n>       Port to listen on (default 443 with HTTPS, else 2368)
    --host <addr>    Address to bind (default 0.0.0.0 with HTTPS, else 127.0.0.1)
    --env <name>     Config environment: production (default) or development
    --staging        Use Let's Encrypt's staging CA while testing
    --force          Overwrite an existing config file

SYSTEMD OPTIONS
    --user <name>    Account to run as (default: the current user)
    --write          Write to /etc/systemd/system/ghost.service instead of stdout

EXAMPLES
    ghost setup --url https://example.com --ssl --email me@example.com
    ghost doctor
    ghost systemd --user ghost --write

Full installation guide: see README.md
"#
    );
}

pub fn print_version() {
    println!("ghost {}", env!("CARGO_PKG_VERSION"));
}

// ---------------------------------------------------------------------------
// setup
// ---------------------------------------------------------------------------

/// Write a config file from flags, asking for anything missing.
pub fn setup(args: &Args) -> Result<(), String> {
    let env = args.get("env").unwrap_or("production").to_string();
    let path = PathBuf::from(format!("config.{env}.json"));
    if path.exists() && !args.flag("force") {
        return Err(format!(
            "{} already exists. Edit it directly, or re-run with --force to replace it.",
            path.display()
        ));
    }

    let interactive = std::io::stdin().is_terminal();
    println!("\nSetting up Ghost-rust ({env})\n");

    // --- url ---------------------------------------------------------------
    let url = match args.get("url") {
        Some(u) => u.to_string(),
        None if interactive => ask(
            "Site url",
            "Where the site will be reached, including https:// — a path is fine\n  (https://example.com, or https://example.com/blog for a subdirectory)",
            Some("http://localhost:2368"),
        )?,
        None => return Err("--url is required when stdin is not a terminal".into()),
    };
    let url = normalize_url(&url)?;

    // --- https -------------------------------------------------------------
    let probe = probe_config(&url)?;
    let host_public = probe.host_is_public();
    // A single-label hostname is legal on an intranet, so this is a note and
    // not a refusal — but it is far more often a typo, and the url decides
    // every link the site writes, so it is worth saying out loud.
    let host = probe.host();
    if !host_public && host != "localhost" && !host.contains('.') && host.parse::<std::net::IpAddr>().is_err() {
        println!("  note: '{host}' is not a public domain. Fine on an intranet; if you meant");
        println!("        a domain name, re-run with the full url.\n");
    }
    let ssl = if args.flag("no-ssl") {
        false
    } else if args.flag("ssl") {
        true
    } else if !host_public {
        // localhost and bare IPs cannot get a public certificate, so there is
        // nothing to ask about
        false
    } else if interactive {
        ask_yes_no(
            "Enable HTTPS with automatic certificates?",
            "Certificates come from Let's Encrypt and renew themselves. Say no only if\n  something in front of this server already terminates TLS.",
            true,
        )?
    } else {
        false
    };

    if ssl && !host_public {
        return Err(format!(
            "automatic certificates need a public domain, but the url is '{url}'.\n\
             For local HTTPS, set ssl.mode to \"manual\" with your own cert and key."
        ));
    }

    // an https url served without TLS only makes sense behind a proxy; an
    // http url with TLS never makes sense
    let url = match (ssl, url.starts_with("http://")) {
        (true, true) => url.replacen("http://", "https://", 1),
        _ => url,
    };

    // --- email -------------------------------------------------------------
    let email = match (ssl, args.get("email")) {
        (false, _) => None,
        (true, Some(e)) => Some(e.to_string()),
        (true, None) if interactive => {
            let answer = ask(
                "Contact email for certificates",
                "Let's Encrypt sends expiry warnings here if renewal ever stops working.\n  Leave blank to skip.",
                Some(""),
            )?;
            Some(answer).filter(|a| !a.trim().is_empty())
        }
        (true, None) => None,
    };

    // --- host and port -----------------------------------------------------
    let port: Option<u16> = match args.get("port") {
        Some(p) => Some(p.parse().map_err(|_| format!("--port '{p}' is not a number"))?),
        None => None,
    };
    // bind everywhere when serving the public directly; stay on loopback when
    // something else is fronting us
    let default_host = if ssl { "0.0.0.0" } else { "127.0.0.1" };
    let host = args.get("host").unwrap_or(default_host).to_string();

    // --- write -------------------------------------------------------------
    let db = if env == "production" { "content/data/ghost.db" } else { "content/data/ghost-dev.db" };

    // Built as text rather than through serde, which sorts object keys
    // alphabetically — `url` and `ssl` belong at the top of a file people are
    // going to open and edit, not wherever the alphabet puts them.
    let ssl_block = match (ssl, &email, args.flag("staging")) {
        (false, _, _) => String::new(),
        (true, None, false) => "  \"ssl\": true,\n".to_string(),
        (true, email, staging) => {
            let mut fields: Vec<String> = Vec::new();
            if let Some(email) = email {
                fields.push(format!("\"email\": {}", json_str(email)));
            }
            if staging {
                fields.push("\"staging\": true".into());
            }
            format!("  \"ssl\": {{ {} }},\n", fields.join(", "))
        }
    };
    let port_field = match port {
        Some(p) => format!(", \"port\": {p}"),
        None => String::new(),
    };

    let body = format!(
        r#"{{
  "url": {url},
{ssl_block}  "server": {{ "host": {host}{port_field} }},
  "database": {{
    "client": "sqlite3",
    "connection": {{ "filename": {db} }}
  }},
  "paths": {{ "contentPath": "content" }},
  "mail": {{ "transport": "Direct" }}
}}
"#,
        url = json_str(&url),
        host = json_str(&host),
        db = json_str(db),
    );
    std::fs::write(&path, &body).map_err(|e| format!("{}: {e}", path.display()))?;

    // Prove the file we just wrote actually loads, rather than leaving the
    // person to discover it at first boot.
    let check = Config::load(Some(&env)).map_err(|e| {
        format!("wrote {} but it does not load: {}", path.display(), e.message())
    })?;
    let plan = check.serve_plan().map_err(|e| e.message())?;

    println!("\n  wrote {}\n", path.display());
    println!("  site        {}", check.canonical_url());
    println!("  admin       {}/ghost/", check.canonical_url());
    println!("  listening   {}", plan.bind_addr());
    match &plan.tls {
        TlsPlan::Off => println!("  https       off (plain HTTP)"),
        TlsPlan::Acme { domains, staging, .. } => println!(
            "  https       automatic for {}{}",
            domains.join(", "),
            if *staging { " (staging CA)" } else { "" }
        ),
        TlsPlan::Manual { .. } => println!("  https       from certificate files"),
    }
    if let Some(p) = plan.redirect_http {
        println!("  redirect    http on :{p} → https");
    }

    println!("\nNext:");
    if env == "production" {
        println!("  GHOST_ENV=production ghost doctor     # check this machine can serve it");
        if plan.port < 1024 {
            println!("  ghost systemd --write                 # run as a service on port {}", plan.port);
        }
        println!("  GHOST_ENV=production ghost            # or start it in the foreground");
    } else {
        println!("  ghost                                 # start the server");
    }
    println!("\nThen open {}/ghost/ to create the owner account.\n", check.canonical_url());
    Ok(())
}

/// Minimal JSON string escaping, for the hand-built config above.
fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

/// Accept what people actually type — `example.com`, `example.com/blog`,
/// `https://example.com/` — and return a url the rest of the system accepts.
fn normalize_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("a site url is required".into());
    }
    let with_scheme = match trimmed.contains("://") {
        true => trimmed.to_string(),
        // bare hostname: assume https, which is what a public site wants
        false => format!("https://{trimmed}"),
    };
    // reuse the real validator so `setup` cannot write a config that `start`
    // would reject
    let cfg = probe_config(&with_scheme)?;
    ghost_core::url::normalize(&path_of(&cfg.url)).map_err(|e| e.to_string())?;
    Ok(with_scheme)
}

fn path_of(url: &str) -> String {
    url.splitn(4, '/').nth(3).map(|s| format!("/{s}")).unwrap_or_default()
}

/// Build a throwaway `Config` for a url, to borrow its validation and
/// host-parsing without touching the filesystem.
fn probe_config(url: &str) -> Result<Config, String> {
    let json = serde_json::json!({
        "url": url,
        "server": { "host": "127.0.0.1" },
        "database": { "client": "sqlite3", "connection": { "filename": "x.db" } },
        "paths": { "contentPath": "content" },
    });
    serde_json::from_value::<Config>(json)
        .map_err(|e| format!("'{url}' is not a usable site url: {e}"))
        .and_then(|mut c| {
            if !c.url.contains("://") {
                return Err(format!("'{url}' needs a scheme, e.g. https://{url}"));
            }
            c.base_dir = std::env::current_dir().unwrap_or_default();
            Ok(c)
        })
}

// ---------------------------------------------------------------------------
// prompts
// ---------------------------------------------------------------------------

fn ask(label: &str, help: &str, default: Option<&str>) -> Result<String, String> {
    println!("{label}\n  {help}");
    let suffix = match default {
        Some(d) if !d.is_empty() => format!(" [{d}]"),
        _ => String::new(),
    };
    print!("  >{suffix} ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    let answer = line.trim().to_string();
    println!();
    match (answer.is_empty(), default) {
        (true, Some(d)) => Ok(d.to_string()),
        (true, None) => Err(format!("{label} is required")),
        _ => Ok(answer),
    }
}

fn ask_yes_no(label: &str, help: &str, default: bool) -> Result<bool, String> {
    let d = if default { "Y/n" } else { "y/N" };
    let answer = ask(label, help, Some(d))?;
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

/// Check the things that actually break a real install, and say what to do
/// about each one. Returns false if anything is a hard failure.
pub async fn doctor() -> bool {
    let mut report = Report::default();

    // --- config ------------------------------------------------------------
    let config = match Config::load(None) {
        Ok(c) => {
            report.ok("config", &format!("loaded for env '{}'", c.env));
            c
        }
        Err(e) => {
            report.fail("config", &e.message(), "run `ghost setup` to write one");
            report.finish();
            return false;
        }
    };

    let subdir = config.subdir();
    report.ok(
        "url",
        &match subdir.is_empty() {
            true => config.canonical_url(),
            false => format!("{} (base path {subdir})", config.canonical_url()),
        },
    );

    let plan = match config.serve_plan() {
        Ok(p) => p,
        Err(e) => {
            report.fail("ssl", &e.message(), "fix `url` or `ssl` in the config file");
            report.finish();
            return false;
        }
    };

    // --- content directories ----------------------------------------------
    match writable(&config.content_path()) {
        Ok(()) => report.ok("content", &format!("{} is writable", config.content_path().display())),
        Err(e) => report.fail(
            "content",
            &format!("{} is not writable: {e}", config.content_path().display()),
            "check the directory exists and the account running ghost owns it",
        ),
    }

    // --- database ----------------------------------------------------------
    let db_path = config.db_path();
    match db_path.parent().map(writable) {
        Some(Err(e)) => report.fail(
            "database",
            &format!("cannot write to {}: {e}", db_path.parent().unwrap().display()),
            "create the directory, or point database.connection.filename somewhere writable",
        ),
        _ => match ghost_db::connect(&db_path).await {
            Ok(pool) => {
                let seeded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);
                let owner: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE status = 'active'")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);
                report.ok("database", &format!("{} (migrated)", db_path.display()));
                if seeded == 0 {
                    report.warn("seed", "not seeded yet", "happens automatically on first start");
                } else if owner == 0 {
                    report.warn(
                        "owner",
                        "no owner account yet",
                        &format!("open {}/ghost/ to create it", config.canonical_url()),
                    );
                } else {
                    report.ok("owner", "owner account exists");
                }
            }
            Err(e) => report.fail("database", &e.message(), "check the path and permissions"),
        },
    }

    // --- themes ------------------------------------------------------------
    let themes = ghost_render::list_themes(&config.themes_dir());
    if themes.is_empty() {
        report.fail(
            "themes",
            &format!("no themes in {}", config.themes_dir().display()),
            "install one, e.g. unzip a theme into that directory",
        );
    } else {
        report.ok("themes", &format!("{} installed", themes.len()));
    }

    // --- ports -------------------------------------------------------------
    let mut ports = vec![plan.port];
    if let Some(p) = plan.redirect_http {
        ports.push(p);
    }
    for port in ports {
        let addr = format!("{}:{}", plan.host, port);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                drop(l);
                report.ok("port", &format!("{addr} is free"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => report.fail(
                "port",
                &format!("not allowed to bind {addr}"),
                "ports under 1024 need a capability: `ghost systemd --write` sets it up, \
                 or grant it directly with `sudo setcap 'cap_net_bind_service=+ep' $(which ghost)`",
            ),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => report.warn(
                "port",
                &format!("{addr} is already in use"),
                "expected if ghost is already running; otherwise `sudo lsof -i :PORT`",
            ),
            Err(e) => report.fail("port", &format!("cannot bind {addr}: {e}"), "check server.host"),
        }
    }

    // --- dns and certificates ---------------------------------------------
    match &plan.tls {
        TlsPlan::Off => {
            report.note("https", "off — correct for local use or behind a TLS-terminating proxy");
        }
        TlsPlan::Manual { cert, key } => {
            for (what, path) in [("ssl.cert", cert), ("ssl.key", key)] {
                if path.is_file() {
                    report.ok(what, &path.display().to_string());
                } else {
                    report.fail(what, &format!("{} not found", path.display()), "check the path");
                }
            }
        }
        TlsPlan::Acme { domains, email, staging, cache_dir } => {
            for domain in domains {
                check_dns(domain, &mut report).await;
            }
            if email.is_none() {
                report.warn(
                    "ssl.email",
                    "no contact address",
                    "add `\"ssl\": {\"email\": \"you@example.com\"}` to hear about renewal problems",
                );
            }
            if *staging {
                report.warn(
                    "ssl.staging",
                    "using Let's Encrypt staging",
                    "browsers will not trust these certificates — remove \"staging\" when ready",
                );
            }
            let cached = cache_dir.is_dir()
                && std::fs::read_dir(cache_dir).map(|mut d| d.next().is_some()).unwrap_or(false);
            if cached {
                report.ok("certificates", &format!("cached in {}", cache_dir.display()));
            } else {
                report.note(
                    "certificates",
                    "none cached yet — one will be requested on the first HTTPS request",
                );
            }
        }
    }

    report.finish()
}

/// Does the domain resolve, and does it point at an address on this machine?
///
/// A mismatch is the single most common reason certificate issuance fails, and
/// it is invisible from the logs, so it is worth checking up front. It is a
/// warning rather than a failure: behind NAT or a load balancer the public
/// address legitimately is not local.
async fn check_dns(domain: &str, report: &mut Report) {
    use std::net::ToSocketAddrs;

    let lookup = format!("{domain}:443");
    let resolved: Vec<std::net::IpAddr> = match lookup.to_socket_addrs() {
        Ok(addrs) => addrs.map(|a| a.ip()).collect(),
        Err(e) => {
            report.fail(
                "dns",
                &format!("{domain} does not resolve ({e})"),
                "add an A/AAAA record pointing at this server, then wait for it to propagate",
            );
            return;
        }
    };
    let list = resolved.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");

    let local = local_addresses();
    if local.is_empty() || resolved.iter().any(|ip| local.contains(ip)) {
        report.ok("dns", &format!("{domain} → {list}"));
    } else {
        report.warn(
            "dns",
            &format!("{domain} → {list}, which is not an address on this machine"),
            "fine behind NAT, a load balancer or Cloudflare; otherwise the certificate \
             request will fail — point the record at this server",
        );
    }
}

/// Addresses bound on this machine. Best-effort: used only to make the DNS
/// check's advice sharper, never to fail anything.
fn local_addresses() -> Vec<std::net::IpAddr> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        // ip(8) on Linux, ifconfig(8) on macOS and the BSDs
        .arg("ip -o addr show 2>/dev/null || ifconfig 2>/dev/null")
        .output();
    let Ok(out) = out else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|tok| {
            tok.split('/').next().unwrap_or(tok).trim_end_matches(',').parse::<std::net::IpAddr>().ok()
        })
        .collect()
}

fn writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".ghost-write-test-{}", std::process::id()));
    std::fs::write(&probe, b"")?;
    std::fs::remove_file(&probe)
}

#[derive(Default)]
struct Report {
    failures: usize,
    warnings: usize,
}

impl Report {
    fn ok(&mut self, what: &str, detail: &str) {
        println!("  ok    {what:<14} {detail}");
    }
    fn note(&mut self, what: &str, detail: &str) {
        println!("  --    {what:<14} {detail}");
    }
    fn warn(&mut self, what: &str, detail: &str, advice: &str) {
        self.warnings += 1;
        println!("  warn  {what:<14} {detail}\n        → {advice}");
    }
    fn fail(&mut self, what: &str, detail: &str, advice: &str) {
        self.failures += 1;
        println!("  FAIL  {what:<14} {detail}\n        → {advice}");
    }
    /// Returns false when something has to be fixed before starting.
    fn finish(&self) -> bool {
        println!();
        match (self.failures, self.warnings) {
            (0, 0) => println!("All checks passed."),
            (0, w) => println!("Ready, with {w} warning(s) to be aware of."),
            (f, _) => println!("{f} problem(s) must be fixed before the site will serve."),
        }
        self.failures == 0
    }
}

// ---------------------------------------------------------------------------
// systemd
// ---------------------------------------------------------------------------

const UNIT_PATH: &str = "/etc/systemd/system/ghost.service";

/// Emit a systemd unit for the current directory and binary.
pub fn systemd(args: &Args) -> Result<(), String> {
    let binary = std::env::current_exe()
        .map_err(|e| format!("cannot find this binary's path: {e}"))?;
    let workdir = std::env::current_dir().map_err(|e| e.to_string())?;
    let workdir = workdir.display().to_string();
    let binary = binary.display().to_string();
    let user = args
        .get("user")
        .map(String::from)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "ghost".into());
    let env = std::env::var("GHOST_ENV").unwrap_or_else(|_| "production".into());

    // only ask for the port capability when a privileged port is actually in
    // play, so the unit stays as unprivileged as the config allows
    let needs_low_ports = Config::load(Some(&env))
        .ok()
        .and_then(|c| c.serve_plan().ok())
        .map(|p| p.port < 1024 || p.redirect_http.is_some_and(|r| r < 1024))
        .unwrap_or(true);
    let capability = match needs_low_ports {
        true => "\n# Lets a non-root account bind 80 and 443 without running as root.\nAmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE",
        false => "",
    };

    let unit = format!(
        r#"[Unit]
Description=Ghost-rust publishing platform
Documentation=https://github.com/TryGhost/Ghost
# network-online rather than network: binding a specific address, and
# requesting certificates, both need routing to actually be up.
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={workdir}
Environment=GHOST_ENV={env}
ExecStart={binary}
Restart=on-failure
RestartSec=5s
# Certificate renewal and SQLite writes should not be cut off mid-flight.
KillSignal=SIGTERM
TimeoutStopSec=30s
{capability}

# Hardening. The content directory is the only thing that needs writing.
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths={workdir}
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
RestrictNamespaces=yes
LockPersonality=yes

[Install]
WantedBy=multi-user.target
"#
    );

    if !args.flag("write") {
        print!("{unit}");
        println!("# Save this as {UNIT_PATH}, then:");
        println!("#   sudo systemctl daemon-reload");
        println!("#   sudo systemctl enable --now ghost");
        println!("#   sudo systemctl status ghost");
        println!("#\n# Or let ghost write it: sudo ghost systemd --write");
        return Ok(());
    }

    std::fs::write(UNIT_PATH, &unit).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("cannot write {UNIT_PATH} — re-run with sudo")
        }
        _ => format!("cannot write {UNIT_PATH}: {e}"),
    })?;
    println!("wrote {UNIT_PATH}\n");
    println!("  sudo systemctl daemon-reload");
    println!("  sudo systemctl enable --now ghost");
    println!("  sudo systemctl status ghost");
    Ok(())
}
