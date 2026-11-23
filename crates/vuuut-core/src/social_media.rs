//! Social media: the rules for sharing this site's posts to the accounts it
//! has on other platforms — X, Bluesky, Mastodon, Threads, LinkedIn, a
//! Facebook Page, a Telegram channel, a Discord server — and for the share
//! buttons readers see under a post.
//!
//! Not to be confused with the **social web** (`vuuut_api::federation`),
//! which makes the publication itself an account on the fediverse that
//! people follow directly. This module is the other thing: the site posting
//! *to* accounts it holds elsewhere.
//!
//! Pure. What lives here:
//!
//! * [`Network`] — each platform's name, character limit, how it counts a
//!   link, and the credentials it needs.
//! * [`credentials`] — the trust boundary for what staff paste in: every
//!   field required, bounded, and checked for shape (an instance is an https
//!   URL, a Discord webhook is a Discord webhook, an author is a LinkedIn
//!   URN). Rebuilt, never stored as sent.
//! * [`compose`] — a share's text from a template, shortened to fit the
//!   network without ever cutting the link.
//! * [`with_utm`], [`hashtags`], [`url_ranges`] — the details that make a
//!   share work: campaign tags, tags as hashtags, Bluesky's link facets.
//! * [`ShareButton`] and [`ShareSettings`] — the buttons under a post: plain
//!   links, no scripts, no cookies.

use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// networks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    X,
    Bluesky,
    Mastodon,
    Threads,
    LinkedIn,
    Facebook,
    Telegram,
    Discord,
}

/// One credential a network needs: form name, label, whether it is a secret
/// (never shown back), and a hint about where to find it.
pub type Field = (&'static str, &'static str, bool, &'static str);

impl Network {
    pub const ALL: [Network; 8] = [
        Network::X, Network::Bluesky, Network::Mastodon, Network::Threads,
        Network::LinkedIn, Network::Facebook, Network::Telegram, Network::Discord,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Network::X => "x",
            Network::Bluesky => "bluesky",
            Network::Mastodon => "mastodon",
            Network::Threads => "threads",
            Network::LinkedIn => "linkedin",
            Network::Facebook => "facebook",
            Network::Telegram => "telegram",
            Network::Discord => "discord",
        }
    }

    pub fn from_key(key: &str) -> Option<Network> {
        Network::ALL.into_iter().find(|n| n.key() == key.trim())
    }

    pub fn label(self) -> &'static str {
        match self {
            Network::X => "X (Twitter)",
            Network::Bluesky => "Bluesky",
            Network::Mastodon => "Mastodon",
            Network::Threads => "Threads",
            Network::LinkedIn => "LinkedIn",
            Network::Facebook => "Facebook Page",
            Network::Telegram => "Telegram channel",
            Network::Discord => "Discord channel",
        }
    }

    /// Two letters for the account's badge — no third-party logos.
    pub fn mark(self) -> &'static str {
        match self {
            Network::X => "X",
            Network::Bluesky => "Bs",
            Network::Mastodon => "Ma",
            Network::Threads => "Th",
            Network::LinkedIn => "in",
            Network::Facebook => "f",
            Network::Telegram => "Tg",
            Network::Discord => "Dc",
        }
    }

    pub fn tint(self) -> &'static str {
        match self {
            Network::X => "slate",
            Network::Bluesky => "blue",
            Network::Mastodon => "purple",
            Network::Threads => "slate",
            Network::LinkedIn => "blue",
            Network::Facebook => "blue",
            Network::Telegram => "teal",
            Network::Discord => "purple",
        }
    }

    /// The longest post the network accepts.
    pub fn limit(self) -> usize {
        match self {
            Network::X => 280,
            Network::Bluesky => 300,
            Network::Mastodon => 500,
            Network::Threads => 500,
            Network::LinkedIn => 3000,
            Network::Facebook => 5000,
            Network::Telegram => 4096,
            Network::Discord => 2000,
        }
    }

    /// How many characters a link counts as, where the network shortens
    /// links itself (X and Mastodon both count any URL as 23).
    pub fn link_weight(self) -> Option<usize> {
        match self {
            Network::X | Network::Mastodon => Some(23),
            _ => None,
        }
    }

    pub fn fields(self) -> &'static [Field] {
        match self {
            Network::X => &[
                ("api_key", "API key", true, "From your app's Keys and tokens page on developer.x.com."),
                ("api_secret", "API key secret", true, ""),
                ("access_token", "Access token", true, "Generated for your own account, with Read and Write permission."),
                ("access_secret", "Access token secret", true, ""),
            ],
            Network::Bluesky => &[
                ("handle", "Handle", false, "For example publication.bsky.social."),
                ("app_password", "App password", true, "Settings → Privacy and security → App passwords. Never your main password."),
                ("pds", "Server", false, "Leave as https://bsky.social unless your account lives on another server."),
            ],
            Network::Mastodon => &[
                ("instance", "Server address", false, "For example https://mastodon.social."),
                ("access_token", "Access token", true, "Preferences → Development → New application, with the write:statuses scope."),
            ],
            Network::Threads => &[
                ("user_id", "Threads user ID", false, "From the Threads API: GET /me with your token."),
                ("access_token", "Long-lived access token", true, "With the threads_basic and threads_content_publish permissions."),
            ],
            Network::LinkedIn => &[
                ("author", "Author URN", false, "urn:li:person:… for you, or urn:li:organization:… for a company page."),
                ("access_token", "Access token", true, "With w_member_social (people) or w_organization_social (pages)."),
            ],
            Network::Facebook => &[
                ("page_id", "Page ID", false, "Page → About → Page transparency, or the number in the Page's URL."),
                ("access_token", "Page access token", true, "A Page token with pages_manage_posts, from Graph API Explorer."),
            ],
            Network::Telegram => &[
                ("bot_token", "Bot token", true, "From @BotFather. Add the bot to the channel as an administrator."),
                ("chat_id", "Channel", false, "@yourchannel, or the numeric id of a private channel."),
            ],
            Network::Discord => &[
                ("webhook_url", "Webhook URL", true, "Channel settings → Integrations → Webhooks → Copy webhook URL."),
            ],
        }
    }
}

pub const MAX_FIELD: usize = 600;

fn clean(raw: &str, max: usize) -> String {
    raw.trim().chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// An `https://host[/]` base with nothing else on it.
fn https_base(raw: &str) -> Option<String> {
    let v = raw.trim().trim_end_matches('/');
    let rest = v.strip_prefix("https://")?;
    let ok = !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        && rest.contains('.');
    ok.then(|| v.to_string())
}

/// Rebuild a network's credentials from a form. `existing` supplies a secret
/// left blank on an edit — the form never shows a stored secret back, so an
/// empty secret field means "keep it".
pub fn credentials(network: Network, form: &impl Fn(&str) -> Option<String>, existing: Option<&Map<String, Value>>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (name, label, secret, _) in network.fields() {
        let mut v = clean(&form(name).unwrap_or_default(), MAX_FIELD);
        if v.is_empty() && *secret {
            if let Some(old) = existing.and_then(|e| e.get(*name)).and_then(Value::as_str) {
                v = old.to_string();
            }
        }
        if v.is_empty() && *name == "pds" {
            v = "https://bsky.social".into();
        }
        if v.is_empty() {
            return Err(format!("{} needs its {}.", network.label(), label.to_lowercase()));
        }
        let ok = match (network, *name) {
            (Network::Mastodon, "instance") | (Network::Bluesky, "pds") => match https_base(&v) {
                Some(b) => {
                    v = b;
                    true
                }
                None => false,
            },
            (Network::Bluesky, "handle") => {
                v = v.trim_start_matches('@').to_lowercase();
                v.contains('.') && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            }
            (Network::LinkedIn, "author") => {
                (v.starts_with("urn:li:person:") || v.starts_with("urn:li:organization:"))
                    && v.rsplit(':').next().is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            }
            (Network::Facebook, "page_id") | (Network::Threads, "user_id") => v.chars().all(|c| c.is_ascii_digit()),
            (Network::Telegram, "chat_id") => {
                (v.starts_with('@') && v.len() > 5 && v[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                    || v.trim_start_matches('-').chars().all(|c| c.is_ascii_digit())
            }
            (Network::Telegram, "bot_token") => v.contains(':') && !v.contains('/'),
            (Network::Discord, "webhook_url") => {
                ["https://discord.com/api/webhooks/", "https://discordapp.com/api/webhooks/", "https://ptb.discord.com/api/webhooks/", "https://canary.discord.com/api/webhooks/"]
                    .iter()
                    .any(|p| v.starts_with(p))
                    && !v.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>'))
            }
            // Tokens and keys: printable, no spaces.
            _ => !v.chars().any(char::is_whitespace),
        };
        if !ok {
            return Err(format!("That {} does not look right for {}.", label.to_lowercase(), network.label()));
        }
        out.insert((*name).to_string(), Value::String(v));
    }
    Ok(out)
}

/// A name for the account, from what identifies it publicly — never from a
/// secret.
pub fn display_name(network: Network, creds: &Map<String, Value>) -> String {
    let get = |k: &str| creds.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match network {
        Network::Bluesky => format!("@{}", get("handle")),
        Network::Mastodon => get("instance").trim_start_matches("https://").to_string(),
        Network::Telegram => get("chat_id"),
        Network::LinkedIn => if get("author").contains("organization") { "Company page".into() } else { "Personal profile".into() },
        Network::Facebook => format!("Page {}", get("page_id")),
        Network::Threads => format!("User {}", get("user_id")),
        Network::X => "X account".into(),
        Network::Discord => "Webhook".into(),
    }
}

// ---------------------------------------------------------------------------
// composing a share
// ---------------------------------------------------------------------------

pub const DEFAULT_TEMPLATE: &str = "{title}\n\n{url}";
pub const MAX_TEMPLATE: usize = 1000;

/// What a template may mention.
#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub title: String,
    pub excerpt: String,
    pub url: String,
    pub site: String,
    pub hashtags: String,
}

/// The weight of a text on a network: characters, with each link counted as
/// the network counts it.
pub fn weight(text: &str, network: Network) -> usize {
    match network.link_weight() {
        None => text.chars().count(),
        Some(w) => {
            let mut total = 0;
            for word in text.split_inclusive(char::is_whitespace) {
                let bare = word.trim_end();
                if bare.starts_with("http://") || bare.starts_with("https://") {
                    total += w + (word.chars().count() - bare.chars().count());
                } else {
                    total += word.chars().count();
                }
            }
            total
        }
    }
}

/// A share's text: the template filled in, then shortened to fit. Only the
/// title and excerpt are ever shortened — the link and the hashtags are kept
/// whole, because a share whose link was cut is a share that goes nowhere.
pub fn compose(template: &str, v: &Vars, network: Network) -> String {
    let template = if template.trim().is_empty() { DEFAULT_TEMPLATE } else { template };
    let fill = |title: &str, excerpt: &str| {
        template
            .replace("{title}", title)
            .replace("{excerpt}", excerpt)
            .replace("{url}", &v.url)
            .replace("{site}", &v.site)
            .replace("{hashtags}", &v.hashtags)
            .trim()
            .to_string()
    };
    let limit = network.limit();
    let mut excerpt: String = v.excerpt.clone();
    let mut title: String = v.title.clone();
    let mut text = fill(&title, &excerpt);
    // Shorten the excerpt first, then the title, a word at a time.
    while weight(&text, network) > limit && !excerpt.is_empty() {
        excerpt = shorten(&excerpt);
        text = fill(&title, &excerpt);
    }
    while weight(&text, network) > limit && !title.is_empty() {
        title = shorten(&title);
        text = fill(&title, &excerpt);
    }
    text
}

/// Drop the last word and end with an ellipsis; empty when nothing is left.
fn shorten(s: &str) -> String {
    let s = s.trim_end_matches('…').trim_end();
    match s.rfind(char::is_whitespace) {
        Some(at) if at > 0 => format!("{}…", s[..at].trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())),
        _ => String::new(),
    }
}

/// Tag names as hashtags: `Product design` → `#ProductDesign`. Up to five.
pub fn hashtags(tags: &[String]) -> String {
    tags.iter()
        .filter_map(|t| {
            let camel: String = t
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut cs = w.chars();
                    cs.next().map(|f| f.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default()
                })
                .collect();
            (!camel.is_empty() && !camel.chars().all(|c| c.is_ascii_digit())).then(|| format!("#{camel}"))
        })
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The post's link with campaign tags, so the site's analytics can tell
/// which network sent a reader. Existing query strings are kept.
pub fn with_utm(url: &str, network: Network, campaign: &str) -> String {
    let (base, frag) = match url.split_once('#') {
        Some((b, f)) => (b, format!("#{f}")),
        None => (url, String::new()),
    };
    let joiner = if base.contains('?') { '&' } else { '?' };
    let campaign: String = campaign.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect();
    format!("{base}{joiner}utm_source={}&utm_medium=social&utm_campaign={}{frag}", network.key(), if campaign.is_empty() { "share" } else { &campaign })
}

/// Byte ranges of every http(s) link in a text — Bluesky only makes a link
/// clickable when the post says where it is, in bytes of UTF-8.
pub fn url_ranges(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    for word in text.split_inclusive(char::is_whitespace) {
        let bare = word.trim_end().trim_end_matches(['.', ',', ')', '!', '?']);
        if bare.starts_with("https://") || bare.starts_with("http://") {
            out.push((at, at + bare.len(), bare.to_string()));
        }
        at += word.len();
    }
    out
}

// ---------------------------------------------------------------------------
// share buttons on the site
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareButton {
    X,
    Facebook,
    LinkedIn,
    Bluesky,
    Threads,
    WhatsApp,
    Telegram,
    Reddit,
    Email,
    Copy,
}

impl ShareButton {
    pub const ALL: [ShareButton; 10] = [
        ShareButton::X, ShareButton::Facebook, ShareButton::LinkedIn, ShareButton::Bluesky, ShareButton::Threads,
        ShareButton::WhatsApp, ShareButton::Telegram, ShareButton::Reddit, ShareButton::Email, ShareButton::Copy,
    ];
    pub fn key(self) -> &'static str {
        match self {
            ShareButton::X => "x",
            ShareButton::Facebook => "facebook",
            ShareButton::LinkedIn => "linkedin",
            ShareButton::Bluesky => "bluesky",
            ShareButton::Threads => "threads",
            ShareButton::WhatsApp => "whatsapp",
            ShareButton::Telegram => "telegram",
            ShareButton::Reddit => "reddit",
            ShareButton::Email => "email",
            ShareButton::Copy => "copy",
        }
    }
    pub fn from_key(k: &str) -> Option<ShareButton> {
        ShareButton::ALL.into_iter().find(|b| b.key() == k.trim())
    }
    pub fn label(self) -> &'static str {
        match self {
            ShareButton::X => "X",
            ShareButton::Facebook => "Facebook",
            ShareButton::LinkedIn => "LinkedIn",
            ShareButton::Bluesky => "Bluesky",
            ShareButton::Threads => "Threads",
            ShareButton::WhatsApp => "WhatsApp",
            ShareButton::Telegram => "Telegram",
            ShareButton::Reddit => "Reddit",
            ShareButton::Email => "Email",
            ShareButton::Copy => "Copy link",
        }
    }
    /// The intent link. Every network's own public share address — the
    /// button is a link, so nothing loads from that network until a reader
    /// chooses it. `None` for Copy, which the page's script handles.
    pub fn href(self, url: &str, title: &str) -> Option<String> {
        let u = encode(url);
        let t = encode(title);
        let both = encode(&format!("{title} {url}"));
        Some(match self {
            ShareButton::X => format!("https://x.com/intent/post?text={t}&url={u}"),
            ShareButton::Facebook => format!("https://www.facebook.com/sharer/sharer.php?u={u}"),
            ShareButton::LinkedIn => format!("https://www.linkedin.com/sharing/share-offsite/?url={u}"),
            ShareButton::Bluesky => format!("https://bsky.app/intent/compose?text={both}"),
            ShareButton::Threads => format!("https://www.threads.net/intent/post?text={both}"),
            ShareButton::WhatsApp => format!("https://wa.me/?text={both}"),
            ShareButton::Telegram => format!("https://t.me/share/url?url={u}&text={t}"),
            ShareButton::Reddit => format!("https://www.reddit.com/submit?url={u}&title={t}"),
            ShareButton::Email => format!("mailto:?subject={t}&body={u}"),
            ShareButton::Copy => return None,
        })
    }
}

/// Percent-encode for a query value.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub const SETTING_BUTTONS: &str = "social_share_buttons";
pub const SETTING_BUTTON_LIST: &str = "social_share_networks";
pub const SETTING_BUTTON_POSITION: &str = "social_share_position";
pub const SETTING_BUTTON_LABEL: &str = "social_share_label";
pub const SETTING_TEMPLATE: &str = "social_default_template";
pub const SETTING_UTM: &str = "social_utm";
pub const SETTING_HASHTAGS: &str = "social_hashtags";

pub const DEFAULT_BUTTONS: &str = "x,facebook,linkedin,bluesky,threads,whatsapp,email,copy";
pub const DEFAULT_LABEL: &str = "Share this post";

#[derive(Debug, Clone, PartialEq)]
pub struct ShareSettings {
    /// Share buttons under posts. Off by default: installing changes nothing.
    pub buttons: bool,
    pub list: Vec<ShareButton>,
    /// `bottom`, `top` or `both`.
    pub position: String,
    pub label: String,
    pub template: String,
    pub utm: bool,
    pub hashtags: bool,
}

impl Default for ShareSettings {
    fn default() -> Self {
        ShareSettings::read(|_| None)
    }
}

impl ShareSettings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> ShareSettings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        let list_raw = get(SETTING_BUTTON_LIST).unwrap_or_else(|| DEFAULT_BUTTONS.into());
        let mut list: Vec<ShareButton> = Vec::new();
        for k in list_raw.split(',') {
            if let Some(b) = ShareButton::from_key(k) {
                if !list.contains(&b) {
                    list.push(b);
                }
            }
        }
        let position = match get(SETTING_BUTTON_POSITION).as_deref().map(str::trim) {
            Some("top") => "top",
            Some("both") => "both",
            _ => "bottom",
        };
        let label = clean(&get(SETTING_BUTTON_LABEL).unwrap_or_default(), 60);
        let template: String = get(SETTING_TEMPLATE)
            .unwrap_or_default()
            .replace("\\n", "\n")
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(MAX_TEMPLATE)
            .collect();
        ShareSettings {
            buttons: flag(SETTING_BUTTONS, false),
            list,
            position: position.into(),
            label: if label.is_empty() { DEFAULT_LABEL.into() } else { label },
            template: if template.trim().is_empty() { DEFAULT_TEMPLATE.into() } else { template.trim().to_string() },
            utm: flag(SETTING_UTM, true),
            hashtags: flag(SETTING_HASHTAGS, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn credentials_are_checked_for_shape_and_a_blank_secret_keeps_the_old_one() {
        let c = credentials(Network::Mastodon, &form(&[("instance", "https://mastodon.social/"), ("access_token", "abc")]), None).unwrap();
        assert_eq!(c["instance"], "https://mastodon.social");
        for bad in ["http://mastodon.social", "https://localhost", "https://a.b/path", "javascript:x"] {
            assert!(credentials(Network::Mastodon, &form(&[("instance", bad), ("access_token", "abc")]), None).is_err(), "{bad}");
        }
        let kept = credentials(Network::Mastodon, &form(&[("instance", "https://m.example"), ("access_token", "")]), Some(&c)).unwrap();
        assert_eq!(kept["access_token"], "abc");
        assert!(credentials(Network::Discord, &form(&[("webhook_url", "https://evil.example/api/webhooks/1/x")]), None).is_err());
        assert!(credentials(Network::Discord, &form(&[("webhook_url", "https://discord.com/api/webhooks/1/abc")]), None).is_ok());
        assert!(credentials(Network::LinkedIn, &form(&[("author", "urn:li:person:"), ("access_token", "t")]), None).is_err());
        assert!(credentials(Network::Telegram, &form(&[("bot_token", "123:abc"), ("chat_id", "@news_room")]), None).is_ok());
        let bs = credentials(Network::Bluesky, &form(&[("handle", "@Pub.bsky.social"), ("app_password", "x-y")]), None).unwrap();
        assert_eq!((bs["handle"].as_str(), bs["pds"].as_str()), (Some("pub.bsky.social"), Some("https://bsky.social")));
        assert!(credentials(Network::X, &form(&[("api_key", "a b")]), None).unwrap_err().contains("does not look right"));
    }

    #[test]
    fn a_share_is_shortened_but_its_link_is_never_cut() {
        let v = Vars {
            title: "A very long title ".repeat(10).trim().to_string(),
            excerpt: "An excerpt that goes on and on ".repeat(20),
            url: format!("https://example.com/{}", "a".repeat(80)),
            ..Vars::default()
        };
        let text = compose("{title}\n\n{excerpt}\n\n{url}", &v, Network::X);
        assert!(weight(&text, Network::X) <= 280, "{}", weight(&text, Network::X));
        assert!(text.ends_with(&v.url), "{text}");
        // On Bluesky the link counts in full, and still survives.
        let bs = compose("{title} {url}", &v, Network::Bluesky);
        assert!(bs.chars().count() <= 300 && bs.contains(&v.url), "{bs}");
        assert_eq!(compose("", &Vars { title: "T".into(), url: "https://e.x/".into(), ..Vars::default() }, Network::LinkedIn), "T\n\nhttps://e.x/");
    }

    #[test]
    fn x_and_mastodon_count_a_link_as_23() {
        let text = format!("hi https://example.com/{}", "x".repeat(100));
        assert_eq!(weight(&text, Network::X), 3 + 23);
        assert_eq!(weight(&text, Network::Bluesky), text.chars().count());
    }

    #[test]
    fn utm_tags_keep_the_existing_query_and_fragment() {
        assert_eq!(with_utm("https://a.b/p/", Network::Bluesky, "auto"), "https://a.b/p/?utm_source=bluesky&utm_medium=social&utm_campaign=auto");
        assert_eq!(with_utm("https://a.b/p/?x=1#top", Network::X, "<bad>"), "https://a.b/p/?x=1&utm_source=x&utm_medium=social&utm_campaign=bad#top");
    }

    #[test]
    fn hashtags_are_camel_case_and_bounded() {
        let tags: Vec<String> = ["product design", "AI", "2026", "c++", "a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(hashtags(&tags), "#ProductDesign #AI #C #A #B");
    }

    #[test]
    fn bluesky_link_ranges_are_utf8_bytes() {
        let text = "Café time: https://a.b/x. More";
        let r = url_ranges(text);
        assert_eq!(r.len(), 1);
        assert_eq!(&text[r[0].0..r[0].1], "https://a.b/x");
    }

    #[test]
    fn share_buttons_default_off_and_intent_links_are_encoded() {
        let s = ShareSettings::default();
        assert!(!s.buttons && s.utm && !s.hashtags);
        assert_eq!(s.list.len(), 8);
        let href = ShareButton::X.href("https://a.b/p?q=1", "Tom & Jerry").unwrap();
        assert_eq!(href, "https://x.com/intent/post?text=Tom%20%26%20Jerry&url=https%3A%2F%2Fa.b%2Fp%3Fq%3D1");
        assert!(ShareButton::Copy.href("x", "y").is_none());
        let s = ShareSettings::read(|k| (k == SETTING_BUTTON_LIST).then(|| "email,bogus,email,x".into()));
        assert_eq!(s.list, vec![ShareButton::Email, ShareButton::X]);
    }
}
