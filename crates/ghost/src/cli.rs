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
    --port <n>       Port to listen on. Without this: 443 with HTTPS, else the
                     first free port from 2368 upwards
    --host <addr>    Address to bind (default 0.0.0.0 with HTTPS, else 127.0.0.1)
    --env <name>     Config environment: production (default) or development
    --staging        Use Let's Encrypt's staging CA while testing
    --force          Overwrite an existing config file

SYSTEMD OPTIONS
    --user <name>    Account to run as (default: the current user)
    --name <slug>    Service name (default: ghost). Give each instance its own
                     name to run several sites on one server
    --write          Write the unit to /etc/systemd/system instead of stdout
    --force          Replace an existing unit of the same name

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
    // bind everywhere when serving the public directly; stay on loopback when
    // something else is fronting us
    let default_host = if ssl { "0.0.0.0" } else { "127.0.0.1" };
    let host = args.get("host").unwrap_or(default_host).to_string();

    let asked_for: Option<u16> = match args.get("port") {
        Some(p) => Some(p.parse().map_err(|_| format!("--port '{p}' is not a number"))?),
        None => None,
    };
    let (port, url) = choose_port(&host, &url, asked_for, ssl)?;

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

// ---------------------------------------------------------------------------
// port selection
// ---------------------------------------------------------------------------

/// How far above the starting port to keep looking before giving up.
const PORT_SEARCH_WINDOW: u16 = 64;

/// Decide which port to install on, and keep the url's port in step with it.
///
/// Returns `(port_to_write, url)`. The port is `None` when the default for
/// this configuration is correct and does not need stating in the file.
///
/// - An explicit `--port` is always honoured, free or not. If it is already
///   taken that is worth knowing now, but it is not this function's call to
///   override what was asked for.
/// - With HTTPS there is nothing to search: 443 is the port browsers use for
///   `https://example.com`, and quietly listening on 444 instead would make
///   every link the site writes point at a closed port. A busy 443 means
///   something else owns HTTPS on this machine, which is a decision to make,
///   not a number to step past.
/// - Otherwise — a local install, or one behind a reverse proxy — walk upwards
///   from the intended port until something is free. 2368 is Ghost's
///   traditional port and is often already serving another site.
fn choose_port(
    host: &str,
    url: &str,
    asked_for: Option<u16>,
    ssl: bool,
) -> Result<(Option<u16>, String), String> {
    // the url's own port is the stated intent, so it is where a search starts
    let from_url = url_port(url);

    if let Some(port) = asked_for {
        if port_status(host, port) == PortStatus::InUse {
            println!("  note: port {port} is already in use. Installing there anyway, as asked —");
            println!("        whatever is holding it will have to stop first.\n");
        }
        // `--port` wins over a port written into the url, and the url has to
        // agree or every link the site writes would name the wrong one
        return Ok((Some(port), set_url_port(url, port)));
    }

    if ssl {
        // Nothing to search for, but a busy 443 is worth hearing about now
        // rather than at first start — on a server that already runs nginx,
        // Apache or another Ghost, this is the normal case.
        for (port, what) in [(443u16, "HTTPS"), (80, "the HTTP redirect")] {
            if port_status(host, port) == PortStatus::InUse {
                println!("  note: port {port} ({what}) is already in use — something else on this");
                println!("        server owns it. Two programs cannot share it, so either stop that");
                println!("        one, or put it in front of this site as a reverse proxy and");
                println!("        re-run with --no-ssl --port <n>.\n");
            }
        }
        return Ok((None, url.to_string()));
    }

    let start = from_url.unwrap_or(2368);
    let port = find_free_port(host, start)?;
    if port != start {
        println!("  note: port {start} is in use, so this install will use {port} instead.");
        println!("        Pass --port to choose one yourself.\n");
    }

    // Writing the port only when it differs from the default keeps the common
    // config file short, which is the one people read.
    let needs_stating = port != 2368 || from_url.is_some();
    Ok((needs_stating.then_some(port), set_url_port(url, port)))
}

/// Walk upwards from `start` until a port nothing is listening on turns up.
fn find_free_port(host: &str, start: u16) -> Result<u16, String> {
    let last = start.saturating_add(PORT_SEARCH_WINDOW);
    (start..=last)
        .find(|port| port_status(host, *port) == PortStatus::Free)
        .ok_or_else(|| format!("no free port between {start} and {last}; pass --port to pick one"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortStatus {
    Free,
    /// Something is listening on it.
    InUse,
    /// Below 1024 and this process is not privileged. Says nothing about
    /// whether anyone holds it, so it must not be reported as "in use".
    Denied,
}

/// Whether `port` can be bound right now.
///
/// Binding and immediately dropping is the only honest test — a port can be
/// held by a process this one cannot see. It does leave a gap between the
/// check and the server's own bind, which is why a port that goes on to be
/// taken is reported at startup rather than treated as impossible here.
fn port_status(host: &str, port: u16) -> PortStatus {
    if port == 0 {
        return PortStatus::InUse;
    }
    let mut denied = false;
    for addr in probe_addresses(host) {
        match std::net::TcpListener::bind((addr.as_str(), port)) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied = true,
            Err(_) => return PortStatus::InUse,
        }
    }
    match denied {
        true => PortStatus::Denied,
        false => PortStatus::Free,
    }
}

/// Every address a bind on `host` could collide with.
///
/// One probe is not enough. Rust's `TcpListener` sets `SO_REUSEADDR`, which
/// lets a wildcard bind succeed alongside a loopback-only one — so probing
/// `0.0.0.0` alone reports a port as free while a neighbour is serving on
/// `127.0.0.1`, which is exactly how a stock Ghost install binds. Checking the
/// configured address *and* the wildcard *and* loopback is conservative: it can
/// pass over a port that would technically have worked, which for choosing an
/// install port is the right way to be wrong.
fn probe_addresses(host: &str) -> Vec<String> {
    let mut addrs = vec![host.to_string()];
    for also in ["0.0.0.0", "127.0.0.1"] {
        if !addrs.iter().any(|a| a == also) {
            addrs.push(also.to_string());
        }
    }
    addrs
}

/// The port written into a url, if it carries one.
fn url_port(url: &str) -> Option<u16> {
    authority(url)?.rsplit_once(':')?.1.parse().ok()
}

/// Replace the port in a url, leaving a url that carries none untouched — on a
/// public site the port is implied by the scheme and does not belong there.
fn set_url_port(url: &str, port: u16) -> String {
    let Some(auth) = authority(url) else { return url.to_string() };
    // `[::1]` has colons of its own; only a numeric tail is a port
    let Some((hostname, existing)) = auth.rsplit_once(':') else { return url.to_string() };
    if existing.parse::<u16>().is_err() {
        return url.to_string();
    }
    url.replacen(&format!("{hostname}:{existing}"), &format!("{hostname}:{port}"), 1)
}

/// The `host[:port]` part of a url.
fn authority(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    Some(match rest.find('/') {
        Some(i) => &rest[..i],
        None => rest,
    })
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

const UNIT_DIR: &str = "/etc/systemd/system";

/// Validate a `--name` into something that can only ever be a filename in
/// `/etc/systemd/system`, never a path out of it.
fn unit_name(args: &Args) -> Result<String, String> {
    let name = args.get("name").unwrap_or("ghost").trim().to_string();
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(format!(
            "--name '{name}' is not usable as a service name (letters, digits, '-' and '_' only)"
        ));
    }
    Ok(name)
}

/// Stop a second site from quietly taking over the first one's unit.
///
/// Installing another instance and forgetting `--name` would otherwise
/// overwrite `ghost.service`, leaving one of the two services pointed at the
/// wrong directory — and the symptom (a site serving someone else's content)
/// is a long way from the cause. The existing unit's `WorkingDirectory` goes
/// into the message, because that is what identifies which site is already
/// installed.
fn refuse_to_clobber(unit_path: &std::path::Path) -> Result<(), String> {
    let Ok(existing) = std::fs::read_to_string(unit_path) else {
        return Ok(()); // absent, or unreadable — the write will report that
    };
    let serving = existing
        .lines()
        .find_map(|l| l.strip_prefix("WorkingDirectory="))
        .unwrap_or("(unknown directory)");
    Err(format!(
        "{} already exists, serving {serving}.\n\n\
         To run this site alongside that one, give it its own name:\n\
         \n    sudo ghost systemd --write --name my-other-site\n\n\
         Or pass --force to replace the existing unit.",
        unit_path.display()
    ))
}

/// Emit a systemd unit for the current directory and binary.
pub fn systemd(args: &Args) -> Result<(), String> {
    let name = unit_name(args)?;
    let unit_path = format!("{UNIT_DIR}/{name}.service");
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
Description=Ghost-rust publishing platform ({name})
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
        println!("# Save this as {unit_path}, then:");
        println!("#   sudo systemctl daemon-reload");
        println!("#   sudo systemctl enable --now {name}");
        println!("#   sudo systemctl status {name}");
        println!("#\n# Or let ghost write it: sudo ghost systemd --write");
        return Ok(());
    }

    if !args.flag("force") {
        refuse_to_clobber(std::path::Path::new(&unit_path))?;
    }

    std::fs::write(&unit_path, &unit).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("cannot write {unit_path} — re-run with sudo")
        }
        _ => format!("cannot write {unit_path}: {e}"),
    })?;
    println!("wrote {unit_path}\n");
    println!("  sudo systemctl daemon-reload");
    println!("  sudo systemctl enable --now {name}");
    println!("  sudo systemctl status {name}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port_out_of_a_url() {
        assert_eq!(url_port("http://localhost:2368"), Some(2368));
        assert_eq!(url_port("http://localhost:2368/blog"), Some(2368));
        assert_eq!(url_port("https://example.com"), None);
        assert_eq!(url_port("https://example.com/blog"), None);
        // an IPv6 literal's colons are not a port
        assert_eq!(url_port("http://[::1]/"), None);
        assert_eq!(url_port("http://[::1]:2368/"), Some(2368));
    }

    /// The url and the listening port have to agree: a mismatch means every
    /// link the site writes names a port nothing is listening on.
    #[test]
    fn rewrites_the_url_port_in_step() {
        assert_eq!(set_url_port("http://localhost:2368", 2371), "http://localhost:2371");
        assert_eq!(
            set_url_port("http://localhost:2368/blog", 2371),
            "http://localhost:2371/blog"
        );
        assert_eq!(set_url_port("http://[::1]:2368/", 2371), "http://[::1]:2371/");
    }

    /// A public url carries no port — the scheme implies it — so there is
    /// nothing to rewrite, and adding one would be wrong.
    #[test]
    fn leaves_a_portless_url_alone() {
        assert_eq!(set_url_port("https://example.com", 2371), "https://example.com");
        assert_eq!(set_url_port("https://example.com/blog", 2371), "https://example.com/blog");
        assert_eq!(set_url_port("http://[::1]/", 2371), "http://[::1]/");
    }

    /// A hostname that merely contains digits after a colon elsewhere in the
    /// path must not be mistaken for the authority.
    #[test]
    fn only_the_authority_is_considered() {
        assert_eq!(authority("http://localhost:2368/a:1"), Some("localhost:2368"));
        assert_eq!(url_port("http://localhost/a:1"), None);
        assert_eq!(set_url_port("http://localhost/a:1", 2371), "http://localhost/a:1");
    }

    #[test]
    fn https_never_hunts_for_a_port() {
        // 443 is implied by the scheme; stepping to 444 would break every link
        let (port, url) = choose_port("127.0.0.1", "https://example.com", None, true).unwrap();
        assert_eq!(port, None);
        assert_eq!(url, "https://example.com");
    }

    #[test]
    fn an_explicit_port_is_honoured_and_synced_into_the_url() {
        let (port, url) =
            choose_port("127.0.0.1", "http://localhost:2368", Some(9000), false).unwrap();
        assert_eq!(port, Some(9000));
        assert_eq!(url, "http://localhost:9000");
    }

    /// With the default port free, nothing needs saying in the config file.
    #[test]
    fn leaves_the_default_port_unstated() {
        let free = find_free_port("127.0.0.1", 2368).unwrap();
        if free != 2368 {
            return; // something on this machine holds 2368; covered below
        }
        let (port, url) = choose_port("127.0.0.1", "http://localhost:2368", None, false).unwrap();
        assert_eq!(url, "http://localhost:2368");
        // 2368 is the default, but the url states it, so the file does too
        assert_eq!(port, Some(2368));
    }

    /// The behaviour this feature exists for: step past an occupied port and
    /// carry the url along with it.
    #[test]
    fn steps_past_an_occupied_port() {
        // hold a port for the duration of the test
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = held.local_addr().unwrap().port();

        let start_url = format!("http://localhost:{taken}");
        let (port, url) = choose_port("127.0.0.1", &start_url, None, false).unwrap();
        let port = port.expect("a stepped-to port is always stated");

        assert!(port > taken, "expected a port above {taken}, got {port}");
        assert_eq!(url, format!("http://localhost:{port}"));
        drop(held);
    }

    /// When nothing in the window can be bound, say so and name the way out
    /// rather than falling back to a port that will not work.
    #[test]
    fn reports_when_the_whole_window_is_unusable() {
        // an address this machine does not hold cannot be bound at any port
        let err = find_free_port("203.0.113.1", 2368).unwrap_err();
        assert!(err.contains("--port"), "{err}");
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use std::collections::HashMap;

    fn args(pairs: &[(&str, &str)]) -> Args {
        Args(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>())
    }

    #[test]
    fn defaults_to_one_service_called_ghost() {
        assert_eq!(unit_name(&args(&[])).unwrap(), "ghost");
    }

    #[test]
    fn names_each_instance_separately() {
        assert_eq!(unit_name(&args(&[("name", "site-a")])).unwrap(), "site-a");
        assert_eq!(unit_name(&args(&[("name", "blog_2")])).unwrap(), "blog_2");
    }

    /// The name becomes a filename under /etc/systemd/system, so it must not
    /// be able to point anywhere else.
    #[test]
    fn refuses_a_name_that_could_escape_the_unit_directory() {
        for bad in ["../../etc/passwd", "a/b", "", "  ", "with space", "sem;colon", "dot.service"] {
            assert!(
                unit_name(&args(&[("name", bad)])).is_err(),
                "should have rejected {bad:?}"
            );
        }
    }

    /// The bug this guards: a second install overwriting the first one's unit.
    #[test]
    fn refuses_to_overwrite_another_sites_unit() {
        let dir = std::env::temp_dir().join(format!("ghost-unit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ghost.service");

        // nothing there yet: nothing to protect
        assert!(refuse_to_clobber(&path).is_ok());

        std::fs::write(&path, "[Service]\nWorkingDirectory=/var/www/first-site\n").unwrap();
        let err = refuse_to_clobber(&path).unwrap_err();
        // names the site already installed, and the way to run both
        assert!(err.contains("/var/www/first-site"), "{err}");
        assert!(err.contains("--name"), "{err}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
