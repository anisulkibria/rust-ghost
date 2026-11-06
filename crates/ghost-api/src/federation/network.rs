//! DNS is resolved once and pinned for the request, preventing rebinding SSRF.
use super::*;
use std::net::{IpAddr, SocketAddr};
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && ip.octets()[0] != 0
                && ip.octets()[0] < 224
                && !(ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                public_ip(IpAddr::V4(v4))
            } else {
                !ip.is_loopback()
                    && !ip.is_unspecified()
                    && !ip.is_multicast()
                    && !ip.is_unique_local()
                    && !ip.is_unicast_link_local()
                    && ip.segments()[0] != 0
                    && !(ip.segments()[0] == 0x2001 && ip.segments()[1] == 0xdb8)
            }
        }
    }
}
pub async fn client(raw: &str) -> ghost_core::Result<(reqwest::Client, reqwest::Url)> {
    let url = reqwest::Url::parse(raw).map_err(|_| bad("Invalid federation URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| bad("Missing federation host"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(bad("Federation URLs must use HTTPS"));
    }
    let addresses = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
    )
    .await
    .map_err(|_| bad("Federation DNS timed out"))?
    .map_err(|_| bad("Federation host could not be resolved"))?
    .collect::<Vec<SocketAddr>>();
    if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
        return Err(bad("Private network federation URLs are not allowed"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .user_agent("NativeGhost/6.73 ActivityPub")
        .build()
        .map_err(|e| bad(&e.to_string()))?;
    Ok((client, url))
}
/// How many redirects a lookup will follow.
///
/// Three is enough for the shapes that actually occur — apex to `www`, http
/// to https, a host rename — without letting a server walk us around the
/// internet.
const MAX_REDIRECTS: usize = 3;

/// Who we are, when a remote server insists on knowing before it answers.
///
/// Mastodon's "authorised fetch" (secure mode) and several other servers
/// refuse an unsigned `GET` of an actor or object with a 401. Fetching
/// unsigned therefore does not merely lose a nicety — it makes those
/// instances completely undiscoverable, which is most of what "federation
/// interoperability is unverified" amounted to.
#[derive(Debug, Clone)]
pub struct Signer {
    pub key_id: String,
    pub private_pem: String,
}

/// Load the publication's signing identity.
///
/// Returns `None` rather than failing when no key exists yet: an unsigned
/// fetch still works against the servers that allow one, and a first-boot
/// lookup should not be blocked on key generation.
pub async fn signer(pool: &ghost_db::Db, actor_id: &str) -> Option<Signer> {
    let private_pem: String = sqlx::query_scalar(
        "SELECT private_pem FROM federation_keys WHERE id='publication'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    Some(Signer { key_id: format!("{actor_id}#main-key"), private_pem })
}

/// Sign an outbound `GET` the way draft-cavage HTTP Signatures requires.
///
/// `(request-target) host date` and no digest, because a GET has no body —
/// adding a digest of nothing is what some implementations reject. This is
/// the same construction the outbound POST uses, minus that line.
#[cfg(test)]
pub(super) fn sign_get_for_test(signer: &Signer, url: &reqwest::Url, date: &str) -> Option<String> {
    sign_get(signer, url, date)
}

fn sign_get(signer: &Signer, url: &reqwest::Url, date: &str) -> Option<String> {
    let host = url.host_str()?;
    let path = match url.query() {
        Some(query) => format!("{}?{}", url.path(), query),
        None => url.path().to_string(),
    };
    let canonical = format!(
        "(request-target): get {path}\nhost: {host}\ndate: {date}"
    );
    let signature = super::crypto::sign(&signer.private_pem, &canonical).ok()?;
    Some(format!(
        "keyId=\"{}\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date\",signature=\"{signature}\"",
        signer.key_id
    ))
}

pub async fn get(raw: &str) -> ghost_core::Result<Value> {
    get_as(raw, None).await
}

/// Fetch a remote document, signing the request when an identity is given.
pub async fn get_as(raw: &str, as_who: Option<&Signer>) -> ghost_core::Result<Value> {
    // Redirects are followed by hand rather than by reqwest, because every
    // hop has to go back through `client`: that is what re-checks the scheme,
    // re-resolves the host and pins it to public addresses. reqwest's own
    // redirect following would reuse the first hop's pinned resolution and
    // skip the public-IP check on the new host, which is exactly the
    // rebinding hole the pinning exists to close.
    //
    // Following them at all is not optional. Webfinger lives at the apex by
    // definition, and a great many sites — including Ghost ones — serve the
    // apex as a 301 to `www`. Treating that as a failed lookup made every
    // such handle unfollowable, which is what it did before.
    let mut target = raw.to_string();
    let mut response = None;
    for _ in 0..=MAX_REDIRECTS {
        let (client, url) = client(&target).await?;
        let mut request = client
            .get(url.clone())
            .header("accept", "application/activity+json, application/ld+json");
        // Signed per hop, not once: the signature covers the host and path,
        // so a redirect to another host needs its own.
        if let Some(as_who) = as_who {
            let date = httpdate::fmt_http_date(std::time::SystemTime::now());
            if let Some(signature) = sign_get(as_who, &url, &date) {
                request = request.header("date", date).header("signature", signature);
            }
        }
        let hop = request
            .send()
            .await
            .map_err(|_| bad("Could not fetch remote actor"))?;
        if hop.status().is_redirection() {
            let location = hop
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| bad("Remote redirect had no destination"))?;
            // Relative redirects are legal and common, so the destination is
            // resolved against the URL that produced it.
            target = url
                .join(location)
                .map_err(|_| bad("Remote redirect was not a valid URL"))?
                .to_string();
            continue;
        }
        response = Some(hop);
        break;
    }
    let Some(mut response) = response else {
        return Err(bad("Remote server redirected too many times"));
    };
    if !response.status().is_success() {
        return Err(bad("Remote actor lookup failed"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| bad("Could not read remote actor"))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(bad("Remote document exceeds 1MB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| bad("Remote actor returned invalid JSON"))
}
pub struct Actor {
    pub id: String,
    pub inbox: String,
    pub key_id: String,
    pub public_key: String,
    /// Who they are, for the screens that list them. All optional: an actor
    /// document is only obliged to carry an id, an inbox and a key, and a
    /// server that omits a display name must not make the follow fail.
    pub profile: Profile,
}

/// The displayable half of an actor document.
#[derive(Debug, Default, Clone)]
pub struct Profile {
    /// `@name@server`, derived from `preferredUsername` and the actor's host
    /// rather than trusted from the document: the handle is how a person is
    /// identified here, so it has to come from where the actor actually lives.
    pub handle: String,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub summary: Option<String>,
    pub url: Option<String>,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub posts: Option<i64>,
}

/// Read the profile fields off an actor document.
///
/// An avatar is only accepted over HTTPS, because it ends up as an `<img
/// src>` in the admin: an `http://` one would be a mixed-content warning on
/// every row, and a `javascript:` one would be worse.
fn profile_of(data: &Value, id: &str) -> Profile {
    let host = reqwest::Url::parse(id)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    let username = data["preferredUsername"].as_str().unwrap_or("").trim();
    let https = |value: Option<&str>| {
        value
            .filter(|raw| raw.len() < 2000)
            .filter(|raw| reqwest::Url::parse(raw).is_ok_and(|url| url.scheme() == "https"))
            .map(str::to_string)
    };
    let count = |key: &str| {
        data.pointer(&format!("/{key}/totalItems"))
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0)
    };
    Profile {
        handle: if username.is_empty() || host.is_empty() {
            id.to_string()
        } else {
            format!("@{username}@{host}")
        },
        // Bounded: these are rendered into the admin, and a remote server can
        // put anything it likes in them.
        name: data["name"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| name.chars().take(200).collect()),
        avatar: https(
            data.pointer("/icon/url")
                .or_else(|| data.get("icon"))
                .and_then(Value::as_str),
        ),
        summary: data["summary"]
            .as_str()
            .map(|s| ghost_core::comments::to_plaintext(s))
            .map(|s| s.trim().chars().take(500).collect::<String>())
            .filter(|s| !s.is_empty()),
        url: https(data["url"].as_str()).or_else(|| Some(id.to_string())),
        followers: count("followers"),
        following: count("following"),
        posts: count("outbox"),
    }
}

/// Fetch and validate a remote actor, signing the request when we can.
///
/// There is no unsigned variant: every caller has an identity to present,
/// and an unsigned fetch is refused outright by any instance in
/// authorised-fetch mode.
pub async fn actor_as(raw: &str, as_who: Option<&Signer>) -> ghost_core::Result<Actor> {
    let data = get_as(raw, as_who).await?;
    if data["id"].as_str() != Some(raw) {
        return Err(bad("Remote actor id mismatch"));
    }
    let inbox = data
        .pointer("/endpoints/sharedInbox")
        .or_else(|| data.get("inbox"))
        .and_then(Value::as_str)
        .ok_or_else(|| bad("Remote actor has no inbox"))?;
    let key = &data["publicKey"];
    if key["owner"].as_str() != Some(raw) {
        return Err(bad("Remote public key owner mismatch"));
    }
    let key_id = key["id"]
        .as_str()
        .ok_or_else(|| bad("Missing remote key id"))?;
    let public_key = key["publicKeyPem"]
        .as_str()
        .filter(|key| key.len() < 16384)
        .ok_or_else(|| bad("Missing remote public key"))?;
    let parsed = reqwest::Url::parse(inbox).map_err(|_| bad("Invalid remote inbox"))?;
    if parsed.scheme() != "https" {
        return Err(bad("Remote inbox must use HTTPS"));
    }
    Ok(Actor {
        id: raw.into(),
        inbox: inbox.into(),
        key_id: key_id.into(),
        public_key: public_key.into(),
        profile: profile_of(&data, raw),
    })
}
/// The other spelling of a host: `example.com` <-> `www.example.com`.
///
/// Only these two, and only for a host that has a registrable name to begin
/// with — this is the apex/`www` ambiguity, not a general subdomain search.
fn sibling_host(host: &str) -> Option<String> {
    match host.strip_prefix("www.") {
        Some(apex) => Some(apex.to_string()),
        // Two labels or more, so `localhost` and bare hostnames are left
        // alone rather than being asked for as `www.localhost`.
        None if host.matches('.').count() >= 1 => Some(format!("www.{host}")),
        None => None,
    }
}

/// Ask one host's webfinger about an account.
async fn webfinger(host: &str, handle: &str) -> ghost_core::Result<Value> {
    let mut url = reqwest::Url::parse(&format!("https://{host}/.well-known/webfinger"))
        .map_err(|_| bad("Invalid account host"))?;
    url.query_pairs_mut()
        .append_pair("resource", &format!("acct:{handle}"));
    get(url.as_str()).await
}

/// Resolve a handle to an actor, signing the actor fetch when we can.
///
/// Webfinger itself stays unsigned: it is not ActivityPub, it is served
/// before any actor relationship exists, and servers do not expect a
/// signature on it.
pub async fn discover_as(handle: &str, as_who: Option<&Signer>) -> ghost_core::Result<Actor> {
    if handle.starts_with("https://") {
        return actor_as(handle, as_who).await;
    }
    let handle = handle.trim_start_matches('@');
    let (_, host) = handle
        .split_once('@')
        .ok_or_else(|| bad("Enter @name@server or an actor URL"))?;

    // The host in the handle is the authority, and is asked first. When it
    // has nothing to say, the apex/`www` sibling is asked too: a site that
    // publishes `@you@example.com` while serving webfinger only on
    // `www.example.com` is a misconfiguration, but a common enough one that
    // refusing to find the account is the wrong answer. The reply is checked
    // below either way, so the sibling cannot answer for somebody else.
    let mut response = webfinger(host, handle).await;
    if response.is_err() {
        if let Some(sibling) = sibling_host(host) {
            if let Ok(found) = webfinger(&sibling, handle).await {
                response = Ok(found);
            }
        }
    }
    let response = response?;

    // Whoever answered has to be answering about the account that was asked
    // for. Without this a host could return its own actor for any handle on
    // any domain it is asked about.
    let subject = response["subject"].as_str().unwrap_or_default();
    if !subject.trim_start_matches("acct:").eq_ignore_ascii_case(handle) {
        return Err(bad("That server answered for a different account"));
    }

    let actor_url = response["links"]
        .as_array()
        .and_then(|links| {
            links
                .iter()
                .find(|link| link["rel"] == "self" && link["type"] == "application/activity+json")
        })
        .and_then(|link| link["href"].as_str())
        .ok_or_else(|| bad("No ActivityPub actor found"))?;
    let mut found = actor_as(actor_url, as_who).await?;
    // The handle the site declares, now that it has been checked against the
    // one that was asked for. It is the better of the two: a publication at
    // `www.example.com` that calls itself `@you@example.com` means it, and
    // listing it under the host its actor file happens to sit on would show
    // a handle that is not the one it hands out. The host-derived fallback in
    // `profile_of` still covers the case where an actor URL is followed
    // directly and there is no webfinger answer to check against.
    found.profile.handle = format!("@{handle}");
    Ok(found)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_and_documentation_addresses_are_rejected() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "::1",
            "::ffff:127.0.0.1",
            "100.64.0.1",
            "192.0.2.1",
            "fc00::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
    }

    /// Webfinger lives at the apex, but plenty of sites — Ghost ones among
    /// them — publish `@you@example.com` while only answering on
    /// `www.example.com`. The sibling is the one extra host worth trying.
    #[test]
    fn the_apex_and_www_spellings_are_siblings() {
        assert_eq!(sibling_host("example.com").as_deref(), Some("www.example.com"));
        assert_eq!(sibling_host("www.example.com").as_deref(), Some("example.com"));
        // Already a subdomain: `www.` goes in front, which is the only other
        // spelling worth an extra request.
        assert_eq!(sibling_host("social.example.com").as_deref(), Some("www.social.example.com"));
        // Not a domain at all — no second guess to make.
        assert_eq!(sibling_host("localhost"), None);
    }

    /// The handle is built from the actor's own host, not from anything the
    /// document claims — otherwise a server could present itself as
    /// `@ghost@ghost.org` and be listed under someone else's name.
    #[test]
    fn a_handle_is_built_from_the_host_the_actor_lives_on() {
        let doc = json!({
            "id": "https://mastodon.social/users/alice",
            "preferredUsername": "alice",
            // A lie: this is not where this actor lives.
            "name": "Alice",
        });
        let profile = profile_of(&doc, "https://mastodon.social/users/alice");
        assert_eq!(profile.handle, "@alice@mastodon.social");
        assert_eq!(profile.name.as_deref(), Some("Alice"));

        // No username to build one from: the URL is the only honest answer.
        let bare = profile_of(&json!({}), "https://example.test/actor");
        assert_eq!(bare.handle, "https://example.test/actor");
    }

    /// Profile fields are rendered into the admin, so what a remote server
    /// can put in them is bounded.
    #[test]
    fn remote_profile_fields_are_bounded_and_https_only() {
        let doc = json!({
            "preferredUsername": "bob",
            "name": "B".repeat(400),
            "icon": {"url": "http://insecure.test/a.png"},
            "summary": "<p>Hello <b>there</b></p><script>alert(1)</script>",
            "followers": {"totalItems": 1200},
            "outbox": {"totalItems": 48},
        });
        let p = profile_of(&doc, "https://example.test/users/bob");
        assert_eq!(p.name.map(|n| n.chars().count()), Some(200));
        // Plain http would be a mixed-content image on every row.
        assert_eq!(p.avatar, None);
        let summary = p.summary.unwrap();
        assert!(!summary.contains('<'), "{summary}");
        assert!(summary.contains("Hello"), "{summary}");
        assert_eq!(p.followers, Some(1200));
        assert_eq!(p.posts, Some(48));
        assert_eq!(p.following, None);

        // An https avatar is kept.
        let secure = profile_of(
            &json!({"preferredUsername": "c", "icon": {"url": "https://cdn.test/a.png"}}),
            "https://example.test/users/c",
        );
        assert_eq!(secure.avatar.as_deref(), Some("https://cdn.test/a.png"));
    }
}
