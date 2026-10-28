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
pub async fn get(raw: &str) -> ghost_core::Result<Value> {
    let (client, url) = client(raw).await?;
    let mut response = client
        .get(url)
        .header("accept", "application/activity+json, application/ld+json")
        .send()
        .await
        .map_err(|_| bad("Could not fetch remote actor"))?;
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
}
pub async fn actor(raw: &str) -> ghost_core::Result<Actor> {
    let data = get(raw).await?;
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
    })
}
pub async fn discover(handle: &str) -> ghost_core::Result<Actor> {
    if handle.starts_with("https://") {
        return actor(handle).await;
    }
    let handle = handle.trim_start_matches('@');
    let (_, host) = handle
        .split_once('@')
        .ok_or_else(|| bad("Enter @name@server or an actor URL"))?;
    let mut url = reqwest::Url::parse(&format!("https://{host}/.well-known/webfinger"))
        .map_err(|_| bad("Invalid account host"))?;
    url.query_pairs_mut()
        .append_pair("resource", &format!("acct:{handle}"));
    let response = get(url.as_str()).await?;
    let actor_url = response["links"]
        .as_array()
        .and_then(|links| {
            links
                .iter()
                .find(|link| link["rel"] == "self" && link["type"] == "application/activity+json")
        })
        .and_then(|link| link["href"].as_str())
        .ok_or_else(|| bad("No ActivityPub actor found"))?;
    actor(actor_url).await
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
}
