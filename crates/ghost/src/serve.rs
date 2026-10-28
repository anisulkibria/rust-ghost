//! Listening: plain HTTP, HTTPS from certificate files, or HTTPS with
//! certificates obtained and renewed automatically.
//!
//! The point of terminating TLS in-process is that it removes the two hardest
//! steps of a real install. There is no reverse proxy to configure and no
//! certbot to schedule: `"ssl": true` in the config is the whole of it, and
//! renewal happens inside the running server.
//!
//! A reverse proxy is still perfectly supported — leave `ssl` off and let the
//! proxy terminate TLS, which is what [`ServePlan`]'s `TlsPlan::Off` is for.
use std::io;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use ghost_core::config::{ServePlan, TlsPlan};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use tokio::net::TcpListener;
use tower::{Service, ServiceExt};

/// HTTP/2 first, then HTTP/1.1 — the ALPN list a browser expects from a
/// modern server, and what lets h2 be negotiated at all.
const ALPN: [&[u8]; 2] = [b"h2", b"http/1.1"];

/// The app as a cloneable tower service. One clone is handed to each
/// connection.
pub trait AppService:
    Service<
        axum::extract::Request,
        Response = axum::response::Response,
        Error = std::convert::Infallible,
    > + Clone
    + Send
    + 'static
{
}

impl<T> AppService for T
where
    T: Service<
            axum::extract::Request,
            Response = axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    T::Future: Send,
{
}

/// Bind and serve until the process is stopped.
pub async fn run<S>(plan: &ServePlan, app: S, canonical_url: &str) -> io::Result<()>
where
    S: AppService,
    S::Future: Send,
{
    install_crypto_provider();
    if let Some(port) = plan.redirect_http {
        spawn_http_redirect(&plan.host, port, canonical_url.to_string()).await?;
    }

    match &plan.tls {
        TlsPlan::Off => serve_plain(plan, app).await,
        TlsPlan::Manual { cert, key } => serve_manual_tls(plan, app, cert, key).await,
        TlsPlan::Acme { domains, email, staging, cache_dir } => {
            serve_acme(plan, app, domains, email.as_deref(), *staging, cache_dir).await
        }
    }
}

async fn serve_plain<S>(plan: &ServePlan, app: S) -> io::Result<()>
where
    S: AppService,
    S::Future: Send,
{
    let listener = bind(&plan.bind_addr()).await?;
    loop {
        let (stream, peer) = listener.accept().await?;
        tokio::spawn(serve_connection(stream, app.clone(), Some(peer)));
    }
}

async fn serve_manual_tls<S>(plan: &ServePlan, app: S, cert: &Path, key: &Path) -> io::Result<()>
where
    S: AppService,
    S::Future: Send,
{
    install_crypto_provider();
    let config = load_cert_and_key(cert, key)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = bind(&plan.bind_addr()).await?;
    tracing::info!("TLS enabled using {} and {}", cert.display(), key.display());

    loop {
        let (tcp, peer) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            match acceptor.accept(tcp).await {
                // a failed handshake is one client's problem, not the
                // server's: a port scanner or an HTTP request on the HTTPS
                // port lands here constantly
                Ok(stream) => serve_connection(stream, app, Some(peer)).await,
                Err(e) => tracing::debug!("TLS handshake failed: {e}"),
            }
        });
    }
}

async fn serve_acme<S>(
    plan: &ServePlan,
    app: S,
    domains: &[String],
    email: Option<&str>,
    staging: bool,
    cache_dir: &Path,
) -> io::Result<()>
where
    S: AppService,
    S::Future: Send,
{
    use rustls_acme::caches::DirCache;
    use rustls_acme::AcmeConfig;

    install_crypto_provider();
    tokio::fs::create_dir_all(cache_dir).await?;

    let mut acme = AcmeConfig::new(domains)
        .cache(DirCache::new(cache_dir.to_path_buf()))
        .directory_lets_encrypt(!staging);
    if let Some(email) = email {
        acme = acme.contact_push(format!("mailto:{email}"));
    }

    let listener = bind(&plan.bind_addr()).await?;
    tracing::info!(
        "requesting certificates for {} from Let's Encrypt{} (cached in {})",
        domains.join(", "),
        if staging { " staging" } else { "" },
        cache_dir.display()
    );
    if staging {
        tracing::warn!(
            "ssl.staging is on: certificates come from Let's Encrypt's test CA and \
             browsers will not trust them. Remove \"staging\" once the setup works."
        );
    }

    // `tokio_incoming` drives the whole certificate lifecycle — the initial
    // order, the TLS-ALPN-01 challenge, and renewal weeks later — off the same
    // stream that yields accepted connections. Nothing else has to be
    // scheduled.
    let mut incoming = acme.state().tokio_incoming(
        tokio_stream::wrappers::TcpListenerStream::new(listener),
        ALPN.iter().map(|p| p.to_vec()).collect(),
    );

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let app = app.clone();
                let peer = stream.get_ref().get_ref().0.get_ref().peer_addr().ok();
                tokio::spawn(serve_connection(stream, app, peer));
            }
            Err(e) => tracing::debug!("TLS connection failed: {e}"),
        }
    }
    Ok(())
}

/// Serve one accepted connection, letting hyper negotiate HTTP/1.1 or HTTP/2
/// from what ALPN settled on.
async fn serve_connection<I, S>(stream: I, app: S, peer: Option<std::net::SocketAddr>)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    S: AppService,
    S::Future: Send,
{
    let service = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
        if let Some(peer) = peer { req.extensions_mut().insert(axum::extract::ConnectInfo(peer)); }
        // `oneshot` takes care of readiness; the app is always ready, but the
        // Service contract still has to be honoured
        app.clone().oneshot(req.map(axum::body::Body::new))
    });
    if let Err(e) = ConnBuilder::new(TokioExecutor::new())
        // upgrades keep websockets working through the TLS terminator
        .serve_connection_with_upgrades(TokioIo::new(stream), service)
        .await
    {
        tracing::debug!("connection error: {e}");
    }
}

/// A listener on `port` that permanently redirects everything to the canonical
/// HTTPS url, so links and bookmarks written against `http://` keep working.
///
/// This also covers Let's Encrypt's HTTP-01 challenge path being reachable,
/// which some setups rely on; the certificate flow itself uses TLS-ALPN-01 on
/// 443 and does not need this port.
async fn spawn_http_redirect(host: &str, port: u16, canonical_url: String) -> io::Result<()> {
    let addr = format!("{host}:{port}");
    let listener = match bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            // Not fatal: HTTPS still works, only the convenience redirect is
            // missing. Worth being loud about, because "http:// doesn't
            // redirect" is otherwise a mystery.
            tracing::warn!(
                "could not bind {addr} for the HTTP→HTTPS redirect ({e}); \
                 plain-HTTP visitors will not be redirected"
            );
            return Ok(());
        }
    };

    let target = canonical_url.clone();
    let redirect = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let canonical_url = target.clone();
        async move {
            let tail = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
            axum::response::Redirect::permanent(&https_target(
                &canonical_url,
                ghost_core::url::subdir(),
                tail,
            ))
        }
    });

    tracing::info!("redirecting HTTP on :{port} to {canonical_url}");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, redirect).await {
            tracing::warn!("HTTP redirect listener stopped: {e}");
        }
    });
    Ok(())
}

/// Where an HTTP request should be sent on the HTTPS side.
///
/// The redirect listener is deliberately *not* behind the base-path layer, so
/// `tail` still carries the base path when there is one. That is why the
/// target is built against the origin: concatenating onto `canonical_url`,
/// which carries the base path too, yields `/blog/blog/…`.
fn https_target(canonical_url: &str, subdir: &str, tail: &str) -> String {
    let site = format!("{}/", canonical_url.trim_end_matches('/'));

    // `//evil.com` is a protocol-relative URL, not a path on this site.
    // Echoing one into a Location header would turn this listener into an
    // open redirect, so anything that is not a plain path goes to the site.
    if !ghost_core::url::is_root_relative(tail) {
        return site;
    }
    // A prefix match is not enough: under `/blog`, the path `/blogging/`
    // belongs to whatever is at the domain root, not to this site.
    let carries_base =
        subdir.is_empty() || tail == subdir || ghost_core::url::unjoin(subdir, tail) != tail;
    match carries_base {
        true => ghost_core::url::absolute_with(subdir, canonical_url, tail),
        // the bare domain on a subdirectory install: nothing is served at the
        // root, so send them to the site itself
        false => site,
    }
}

/// Bind, turning the two failures people actually hit into advice.
async fn bind(addr: &str) -> io::Result<TcpListener> {
    match TcpListener::bind(addr).await {
        Ok(l) => Ok(l),
        Err(e) => {
            let hint = match e.kind() {
                io::ErrorKind::PermissionDenied => Some(
                    "ports below 1024 need privileges — grant them once with \
                     `sudo setcap 'cap_net_bind_service=+ep' /path/to/ghost`, \
                     or run under the systemd unit from `ghost systemd`, which \
                     does it for you",
                ),
                io::ErrorKind::AddrInUse => {
                    Some("something else is already listening there — `sudo lsof -i :PORT` shows what")
                }
                _ => None,
            };
            if let Some(hint) = hint {
                tracing::error!("cannot listen on {addr}: {e}\n  {hint}");
            }
            Err(e)
        }
    }
}

/// rustls needs a process-wide crypto provider chosen before any config is
/// built. Both TLS paths here use ring, so install it once and ignore a second
/// call.
fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn load_cert_and_key(cert: &Path, key: &Path) -> io::Result<rustls::ServerConfig> {
    let certs = rustls_pemfile::certs(&mut io::BufReader::new(std::fs::File::open(cert).map_err(
        |e| io::Error::new(e.kind(), format!("ssl.cert {}: {e}", cert.display())),
    )?))
    .collect::<Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("ssl.cert {} contains no certificates", cert.display()),
        ));
    }

    let key_file = std::fs::File::open(key)
        .map_err(|e| io::Error::new(e.kind(), format!("ssl.key {}: {e}", key.display())))?;
    let Some(key_der) = rustls_pemfile::private_key(&mut io::BufReader::new(key_file))? else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("ssl.key {} contains no private key", key.display()),
        ));
    };

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key_der)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("certificate and key do not match: {e}")))?;
    config.alpn_protocols = ALPN.iter().map(|p| p.to_vec()).collect();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::https_target;

    /// At a domain root the incoming path is the target path.
    #[test]
    fn redirects_a_root_install_path_for_path() {
        let c = "https://example.com";
        assert_eq!(https_target(c, "", "/"), "https://example.com/");
        assert_eq!(https_target(c, "", "/about/"), "https://example.com/about/");
        assert_eq!(https_target(c, "", "/s/?q=a&p=2"), "https://example.com/s/?q=a&p=2");
    }

    /// The bug this function exists to prevent: the incoming path already
    /// carries the base path, and so does the canonical url.
    #[test]
    fn does_not_double_the_base_path() {
        let c = "https://example.com/blog";
        assert_eq!(https_target(c, "/blog", "/blog/post/"), "https://example.com/blog/post/");
        assert_eq!(https_target(c, "/blog", "/blog/"), "https://example.com/blog/");
        assert_eq!(https_target(c, "/blog", "/blog"), "https://example.com/blog");
        assert_eq!(
            https_target(c, "/blog", "/blog/s/?q=a&p=2"),
            "https://example.com/blog/s/?q=a&p=2"
        );
    }

    /// Nothing is served at the root of a subdirectory install, so a visitor
    /// who typed the bare domain is sent to the site rather than to a 404.
    #[test]
    fn sends_the_bare_domain_to_the_site() {
        let c = "https://example.com/blog";
        assert_eq!(https_target(c, "/blog", "/"), "https://example.com/blog/");
        assert_eq!(https_target(c, "/blog", "/elsewhere"), "https://example.com/blog/");
        // a sibling path that merely shares the prefix is not the base path
        assert_eq!(https_target(c, "/blog", "/blogging/"), "https://example.com/blog/");
    }

    /// A protocol-relative path would make the redirect listener an open
    /// redirect if it were echoed back.
    #[test]
    fn refuses_to_redirect_off_site() {
        assert_eq!(https_target("https://example.com", "", "//evil.com/"), "https://example.com/");
        assert_eq!(
            https_target("https://example.com/blog", "/blog", "//evil.com/"),
            "https://example.com/blog/"
        );
        // and anything that is not a path at all
        assert_eq!(https_target("https://example.com", "", "http://evil.com"), "https://example.com/");
    }
}
