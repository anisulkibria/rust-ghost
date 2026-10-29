//! Outbound mail: the transactional transport (SMTP) that carries magic
//! links, and the bulk transport (Mailgun) that carries newsletters.
//!
//! Ghost configures both in `config.<env>.json` — `mail` for transactional
//! and `bulkEmail` for Mailgun — which means editing a file and restarting to
//! change them. Here they live in the settings table and are edited from
//! Settings, so this module reads its configuration fresh on every send and a
//! credential change takes effect immediately.
//!
//! Secrets (`mail_smtp_pass`, `mailgun_api_key`) are deliberately *not* in
//! the render context's public allowlist, so they never reach a theme or the
//! Content API; the Admin API masks them on read.

pub mod comments;
pub mod newsletter;
pub mod templates;

use ghost_core::{GhostError, Result};
use ghost_db::SettingsCache;

/// How to reach the SMTP server's TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurity {
    /// Plain connection, upgraded with STARTTLS — what port 587 expects.
    StartTls,
    /// TLS from the first byte — what port 465 expects.
    Tls,
    /// No encryption at all. Only sane against a local relay such as
    /// MailHog/Mailpit; credentials would otherwise cross the wire in clear.
    None,
}

impl SmtpSecurity {
    fn parse(s: &str) -> Self {
        match s {
            "tls" | "ssl" => SmtpSecurity::Tls,
            "none" => SmtpSecurity::None,
            _ => SmtpSecurity::StartTls,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            SmtpSecurity::Tls => "tls",
            SmtpSecurity::None => "none",
            SmtpSecurity::StartTls => "starttls",
        }
    }
}

/// The transactional transport, as configured in Settings → Email.
///
/// The names mirror Ghost's `mail.transport` values, so configuration
/// knowledge carries across: `"Direct"` and `"SMTP"`.
#[derive(Debug, Clone)]
pub enum Transport {
    /// Ghost's default. Looks up the recipient domain's MX records and
    /// delivers straight to their mail server, with no relay and no
    /// credentials.
    ///
    /// It needs outbound port 25, which most ISPs and every major cloud
    /// provider block, and mail from an IP with no SPF/DKIM/PTR is usually
    /// rejected or filed as spam. Ghost ships it as the default anyway, and
    /// says the same thing about it — real sites configure SMTP.
    Direct,
    Smtp {
        host: String,
        port: u16,
        security: SmtpSecurity,
        user: String,
        pass: String,
    },
}

/// A resolved mail configuration: transport plus the envelope identity every
/// message is sent as.
#[derive(Debug, Clone)]
pub struct MailConfig {
    pub transport: Transport,
    pub from_address: String,
    pub from_name: String,
    pub reply_to: Option<String>,
}

impl MailConfig {
    /// Read the current configuration out of the settings cache.
    ///
    /// An unset or unrecognised `mail_transport` means Direct, matching
    /// Ghost's `config.get('mail').transport || 'direct'`. SMTP selected
    /// without a host also falls back to Direct rather than failing every
    /// send, since a half-filled SMTP form should not take mail offline.
    pub fn from_settings(settings: &SettingsCache, site_title: &str, site_url: &str) -> Self {
        let s = |k: &str| settings.get_str(k).unwrap_or_default().trim().to_string();
        let host = s("mail_smtp_host");
        let transport = if s("mail_transport").eq_ignore_ascii_case("smtp") && !host.is_empty() {
            Transport::Smtp {
                host,
                port: s("mail_smtp_port").parse().unwrap_or(587),
                security: SmtpSecurity::parse(&s("mail_smtp_security")),
                user: s("mail_smtp_user"),
                pass: s("mail_smtp_pass"),
            }
        } else {
            Transport::Direct
        };
        let from_address = match s("mail_from_address") {
            a if !a.is_empty() => a,
            // Ghost's fallback: `noreply@<site host>`, built from the bare
            // `members_support_address` mailbox name when that is not already
            // a full address.
            _ => {
                let mailbox = match s("members_support_address") {
                    m if m.is_empty() => "noreply".to_string(),
                    m => m,
                };
                if mailbox.contains('@') {
                    mailbox
                } else {
                    format!("{mailbox}@{}", site_domain(site_url))
                }
            }
        };
        let from_name = match s("mail_from_name") {
            n if !n.is_empty() => n,
            _ => site_title.to_string(),
        };
        let reply_to = Some(s("mail_reply_to")).filter(|r| !r.is_empty());
        MailConfig { transport, from_address, from_name, reply_to }
    }

    /// Whether a relay is configured. Direct is always "available" but is
    /// not a configured relay — the UI uses this to decide whether to warn.
    pub fn uses_relay(&self) -> bool {
        matches!(self.transport, Transport::Smtp { .. })
    }

    pub fn transport_name(&self) -> &'static str {
        match self.transport {
            Transport::Direct => "Direct",
            Transport::Smtp { .. } => "SMTP",
        }
    }
}

/// The host part of the site URL, used to build a default sender address.
pub fn site_domain(site_url: &str) -> String {
    site_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("localhost")
        .split(':')
        .next()
        .unwrap_or("localhost")
        .to_string()
}

/// One message to one recipient.
pub struct Message {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
}

/// Send a transactional message. Errors carry the transport's own reason,
/// which is what makes a misconfiguration diagnosable.
pub async fn send(cfg: &MailConfig, msg: &Message) -> Result<()> {
    match &cfg.transport {
        Transport::Direct => send_direct(cfg, msg).await,
        Transport::Smtp { host, port, security, user, pass } => {
            send_smtp(cfg, msg, host, *port, *security, user, pass).await
        }
    }
}

/// Deliver straight to the recipient domain's mail servers.
///
/// Tries each MX host in preference order and reports the last failure if
/// none accept, because the useful error is usually from the lowest-priority
/// host that actually answered rather than the first one that timed out.
async fn send_direct(cfg: &MailConfig, msg: &Message) -> Result<()> {
    let domain = msg
        .to
        .rsplit('@')
        .next()
        .filter(|d| !d.is_empty() && d.len() < msg.to.len())
        .ok_or_else(|| GhostError::EmailError(format!("no domain in recipient {}", msg.to)))?
        .to_string();

    let hosts = lookup_mx(&domain).await?;
    let email = build_email(cfg, msg)?;
    let mut last: Option<String> = None;

    for host in &hosts {
        match deliver_to_mx(host, email.clone()).await {
            Ok(()) => {
                tracing::debug!("mail: delivered to {} via {host}", msg.to);
                return Ok(());
            }
            Err(e) => {
                tracing::debug!("mail: {host} rejected message for {}: {e}", msg.to);
                last = Some(format!("{host}: {e}"));
            }
        }
    }
    // Don't assert a cause here — the two common ones look nothing alike. A
    // blocked port 25 times out; a reachable server refuses on policy with a
    // 5xx naming the reason. Report what actually came back and let the
    // operator read it.
    Err(GhostError::EmailError(format!(
        "direct delivery to {domain} failed ({}). Direct sends from this server's own IP, which recipients widely refuse unless it has SPF/DKIM/PTR set up, and many networks block outbound port 25 — configure SMTP for dependable delivery. Last response: {}",
        if hosts.is_empty() { "no mail servers found".into() } else { format!("tried {} host(s)", hosts.len()) },
        last.unwrap_or_else(|| "none".into()),
    )))
}

/// The recipient domain's mail servers, best preference first.
///
/// A domain with no MX records falls back to its A record, which is what RFC
/// 5321 §5.1 calls the implicit MX — some small domains still rely on it.
async fn lookup_mx(domain: &str) -> Result<Vec<String>> {
    use hickory_resolver::proto::rr::RData;
    use hickory_resolver::Resolver;

    let resolver = Resolver::builder_tokio()
        .map_err(|e| GhostError::EmailError(format!("could not read DNS configuration: {e}")))?
        .build()
        .map_err(|e| GhostError::EmailError(format!("could not start DNS resolver: {e}")))?;

    if let Ok(resp) = resolver.mx_lookup(domain).await {
        let mut hosts: Vec<(u16, String)> = resp
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::MX(mx) => Some((
                    mx.preference,
                    mx.exchange.to_utf8().trim_end_matches('.').to_string(),
                )),
                _ => None,
            })
            .filter(|(_, h)| !h.is_empty())
            .collect();
        hosts.sort_by_key(|(pref, _)| *pref);
        if !hosts.is_empty() {
            return Ok(hosts.into_iter().map(|(_, h)| h).collect());
        }
    }
    match resolver.lookup_ip(domain).await {
        Ok(ips) if ips.iter().next().is_some() => Ok(vec![domain.to_string()]),
        _ => Err(GhostError::EmailError(format!(
            "{domain} has no mail servers (no MX or A record)"
        ))),
    }
}

/// One delivery attempt to one mail server.
///
/// Port 25 with opportunistic TLS: upgrade when the server offers STARTTLS,
/// continue in the clear when it does not. Certificates are not verified,
/// which is deliberate and is what MTAs do between themselves — public mail
/// servers routinely present certificates for a different hostname, and
/// refusing those would mean falling back to plaintext anyway.
async fn deliver_to_mx(host: &str, email: lettre::Message) -> std::result::Result<(), String> {
    use lettre::transport::smtp::client::{Tls, TlsParameters};
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

    let tls = TlsParameters::builder(host.to_string())
        .dangerous_accept_invalid_certs(true)
        .dangerous_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| e.to_string())?;

    AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
        .port(25)
        .tls(Tls::Opportunistic(tls))
        .timeout(Some(std::time::Duration::from_secs(20)))
        .build()
        .send(email)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Build the MIME message both transports send.
fn build_email(cfg: &MailConfig, msg: &Message) -> Result<lettre::Message> {
    use lettre::message::{header, MultiPart, SinglePart};

    let from = format!("{} <{}>", cfg.from_name, cfg.from_address)
        .parse::<lettre::message::Mailbox>()
        .map_err(|e| GhostError::EmailError(format!("invalid from address: {e}")))?;
    let to = msg
        .to
        .parse::<lettre::message::Mailbox>()
        .map_err(|e| GhostError::EmailError(format!("invalid recipient: {e}")))?;

    let mut builder = lettre::Message::builder().from(from).to(to).subject(&msg.subject);
    if let Some(rt) = &cfg.reply_to {
        if let Ok(rt) = rt.parse() {
            builder = builder.reply_to(rt);
        }
    }
    // Both parts, so text-only clients still get a usable link.
    builder
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_PLAIN)
                        .body(msg.text.clone()),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_HTML)
                        .body(msg.html.clone()),
                ),
        )
        .map_err(|e| GhostError::EmailError(format!("could not build message: {e}")))
}

async fn send_smtp(
    cfg: &MailConfig,
    msg: &Message,
    host: &str,
    port: u16,
    security: SmtpSecurity,
    user: &str,
    pass: &str,
) -> Result<()> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

    let email = build_email(cfg, msg)?;

    let mut transport = match security {
        SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| GhostError::EmailError(format!("smtp relay: {e}")))?,
        SmtpSecurity::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| GhostError::EmailError(format!("smtp starttls relay: {e}")))?,
        SmtpSecurity::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
    };
    transport = transport.port(port);
    if !user.is_empty() {
        transport = transport.credentials(Credentials::new(user.to_string(), pass.to_string()));
    }

    transport
        .build()
        .send(email)
        .await
        .map(|_| ())
        .map_err(|e| GhostError::EmailError(format!("{e}")))
}

// ---------------------------------------------------------------------------
// bulk mail — Mailgun
// ---------------------------------------------------------------------------

/// Mailgun credentials, as configured in Settings → Bulk email.
#[derive(Debug, Clone)]
pub struct MailgunConfig {
    pub api_key: String,
    pub domain: String,
    /// Region endpoint. Mailgun's EU region is a different host, and sending
    /// an EU domain's key to the US endpoint fails with a 401 that looks like
    /// a bad key, so this is worth being explicit about.
    pub base_url: String,
    pub from_address: String,
    pub from_name: String,
}

impl MailgunConfig {
    pub fn from_settings(settings: &SettingsCache, fallback: &MailConfig) -> Option<Self> {
        let s = |k: &str| settings.get_str(k).unwrap_or_default().trim().to_string();
        let api_key = s("mailgun_api_key");
        let domain = s("mailgun_domain");
        if api_key.is_empty() || domain.is_empty() {
            return None;
        }
        let base_url = match s("mailgun_base_url") {
            b if b.is_empty() => "https://api.mailgun.net/v3".to_string(),
            b => b.trim_end_matches('/').to_string(),
        };
        let from_address = match s("bulk_email_from_address") {
            a if !a.is_empty() => a,
            _ => fallback.from_address.clone(),
        };
        Some(MailgunConfig {
            api_key,
            domain,
            base_url,
            from_address,
            from_name: fallback.from_name.clone(),
        })
    }

    fn messages_url(&self) -> String {
        format!("{}/{}/messages", self.base_url, self.domain)
    }
}

/// How many recipients Mailgun accepts in one `recipient-variables` send.
/// Ghost's own default, and its hard ceiling on a batch.
pub const MAILGUN_BATCH_SIZE: usize = 1000;

/// One addressee of a batch, with the values its placeholders resolve to.
#[derive(Debug, Clone)]
pub struct BatchRecipient {
    pub email: String,
    /// Mailgun recipient variables, by variable name.
    pub variables: std::collections::BTreeMap<String, String>,
}

/// A batch body. The html and text carry `%recipient.<name>%` placeholders
/// rather than per-member values, which is what lets one request address a
/// thousand people.
#[derive(Debug, Clone)]
pub struct BatchMessage {
    pub subject: String,
    pub html: String,
    pub text: String,
    /// The `emails` row this batch belongs to. Mailgun echoes it back on
    /// every event as `email-id`, and it is the prefix of each recipient's
    /// Message-Id.
    pub email_id: String,
    pub track_opens: bool,
    pub track_clicks: bool,
}

/// What a batch submission produced.
#[derive(Debug, Clone)]
pub struct BatchOutcome {
    /// Mailgun's id for the request. `None` when the batch carried a
    /// per-recipient Message-Id, which is the case here: Mailgun echoes the
    /// template back rather than an id that identifies anything.
    pub provider_id: Option<String>,
    pub accepted: usize,
}

/// Each recipient's own Message-Id, so replies do not all thread together and
/// a delivery event can be attributed to one member.
///
/// Mailgun mints one Message-Id per API call, so a batch would otherwise
/// share it. Built exactly as Ghost builds it — `{email_id}.{digest}@{domain}`
/// with the address hashed rather than exposed — and deterministic in the
/// pair, so a retried batch reuses the same ids instead of orphaning the
/// events already recorded against them.
pub fn recipient_message_id(email_id: &str, recipient: &str, domain: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(format!("{email_id}:{recipient}").as_bytes()));
    format!("{email_id}.{}@{domain}", &digest[..32])
}

/// The recipient variable the `Message-Id` header is templated from.
pub const MESSAGE_ID_VARIABLE: &str = "message_id";

/// Send one batch through Mailgun.
///
/// `recipients` must be no longer than [`MAILGUN_BATCH_SIZE`] and must hold
/// no duplicate addresses: `recipient-variables` is keyed by address, so a
/// duplicate would silently reduce the send.
pub async fn send_mailgun_batch(
    cfg: &MailgunConfig,
    msg: &BatchMessage,
    recipients: &[BatchRecipient],
) -> Result<BatchOutcome> {
    if recipients.is_empty() {
        return Err(GhostError::EmailError("a batch needs at least one recipient".into()));
    }
    if recipients.len() > MAILGUN_BATCH_SIZE {
        return Err(GhostError::EmailError(format!(
            "mailgun accepts at most {MAILGUN_BATCH_SIZE} recipients per batch, got {}",
            recipients.len()
        )));
    }

    let mut variables = serde_json::Map::new();
    for recipient in recipients {
        let mut values: serde_json::Map<String, serde_json::Value> = recipient
            .variables
            .iter()
            .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
            .collect();
        values.insert(
            MESSAGE_ID_VARIABLE.into(),
            recipient_message_id(&msg.email_id, &recipient.email, &cfg.domain).into(),
        );
        variables.insert(recipient.email.clone(), values.into());
    }
    if variables.len() != recipients.len() {
        // Two recipients resolved to one key, so the request would address
        // fewer people than the caller counted. Better to refuse than to
        // under-send and record it as a success.
        return Err(GhostError::EmailError(format!(
            "batch holds duplicate addresses: {} recipients, {} distinct",
            recipients.len(),
            variables.len()
        )));
    }

    let mut form = reqwest::multipart::Form::new()
        .text("from", format!("{} <{}>", cfg.from_name, cfg.from_address))
        .text("subject", msg.subject.clone())
        .text("text", msg.text.clone())
        .text("html", msg.html.clone())
        .text("recipient-variables", serde_json::Value::Object(variables).to_string())
        // The header is a template; the substituted value is per recipient.
        .text("h:Message-Id", format!("<%recipient.{MESSAGE_ID_VARIABLE}%>"))
        .text("h:Sender", cfg.from_address.clone())
        .text("h:Auto-Submitted", "auto-generated")
        .text("h:X-Auto-Response-Suppress", "OOF, AutoReply")
        // Echoed back on every event, so an event can be traced to the send.
        .text("v:email-id", msg.email_id.clone())
        .text("o:tag", "ghost-email")
        .text("o:tag", "bulk-email")
        .text("o:tracking-opens", if msg.track_opens { "yes" } else { "no" })
        .text("o:tracking-clicks", if msg.track_clicks { "yes" } else { "no" });
    for recipient in recipients {
        form = form.text("to", recipient.email.clone());
    }

    let resp = reqwest::Client::new()
        .post(cfg.messages_url())
        .basic_auth("api", Some(&cfg.api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| GhostError::EmailError(format!("mailgun request failed: {e}")))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(GhostError::EmailError(format!("mailgun returned {status}: {body}")));
    }
    // Mailgun hands back the Message-Id it was given, which for this send is
    // the unsubstituted template — it identifies nothing, so it is dropped
    // rather than stored against the batch.
    let provider_id = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_owned))
        .map(|id| id.trim().trim_matches(|c| c == '<' || c == '>').to_string())
        .filter(|id| !id.is_empty() && !id.contains(&format!("%recipient.{MESSAGE_ID_VARIABLE}%")));
    Ok(BatchOutcome { provider_id, accepted: recipients.len() })
}

/// Send one message through Mailgun. Returns Mailgun's message id.
pub async fn send_mailgun(cfg: &MailgunConfig, msg: &Message) -> Result<String> {
    send_mailgun_with_reply(cfg,msg,None).await
}

pub async fn send_mailgun_with_reply(cfg:&MailgunConfig,msg:&Message,reply_to:Option<&str>)->Result<String>{
    let mut form = reqwest::multipart::Form::new()
        .text("from", format!("{} <{}>", cfg.from_name, cfg.from_address))
        .text("to", msg.to.clone())
        .text("subject", msg.subject.clone())
        .text("text", msg.text.clone())
        .text("html", msg.html.clone());

    if let Some(reply_to)=reply_to.filter(|s|!s.is_empty()){form=form.text("h:Reply-To",reply_to.to_owned());}
    let resp = reqwest::Client::new()
        .post(cfg.messages_url())
        // Mailgun authenticates with HTTP basic auth, user literally "api".
        .basic_auth("api", Some(&cfg.api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| GhostError::EmailError(format!("mailgun request failed: {e}")))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Mailgun's own message is the useful part (bad domain, unverified
        // sender, wrong region), so surface it rather than just the code.
        return Err(GhostError::EmailError(format!("mailgun returned {status}: {body}")));
    }
    let id = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_security_defaults_to_starttls() {
        // an unset or unrecognised value must not silently mean "no TLS"
        assert_eq!(SmtpSecurity::parse(""), SmtpSecurity::StartTls);
        assert_eq!(SmtpSecurity::parse("nonsense"), SmtpSecurity::StartTls);
        assert_eq!(SmtpSecurity::parse("tls"), SmtpSecurity::Tls);
        assert_eq!(SmtpSecurity::parse("ssl"), SmtpSecurity::Tls);
        assert_eq!(SmtpSecurity::parse("none"), SmtpSecurity::None);
    }

    #[test]
    fn site_domain_strips_scheme_and_port() {
        assert_eq!(site_domain("http://localhost:2368"), "localhost");
        assert_eq!(site_domain("https://example.com/"), "example.com");
        assert_eq!(site_domain("https://blog.example.com"), "blog.example.com");
    }

    /// Build a MailConfig without touching the settings table, so transport
    /// resolution can be asserted directly.
    fn cfg(transport: Transport) -> MailConfig {
        MailConfig {
            transport,
            from_address: "noreply@example.com".into(),
            from_name: "Example".into(),
            reply_to: None,
        }
    }

    #[test]
    fn direct_is_not_a_relay_but_is_still_a_real_transport() {
        // Ghost's default. It genuinely attempts delivery, so the UI must not
        // describe it as "nothing configured" — but it is not a relay either,
        // which is what the warning banner keys off.
        let direct = cfg(Transport::Direct);
        assert!(!direct.uses_relay());
        assert_eq!(direct.transport_name(), "Direct");

        let smtp = cfg(Transport::Smtp {
            host: "smtp.example.com".into(),
            port: 587,
            security: SmtpSecurity::StartTls,
            user: "u".into(),
            pass: "p".into(),
        });
        assert!(smtp.uses_relay());
        assert_eq!(smtp.transport_name(), "SMTP");
    }

    #[test]
    fn mailgun_url_is_region_aware() {
        let cfg = MailgunConfig {
            api_key: "k".into(),
            domain: "mg.example.com".into(),
            base_url: "https://api.eu.mailgun.net/v3".into(),
            from_address: "a@b.c".into(),
            from_name: "N".into(),
        };
        assert_eq!(cfg.messages_url(), "https://api.eu.mailgun.net/v3/mg.example.com/messages");
    }
}

#[cfg(test)]
mod direct_tests {
    use super::*;

    /// Hits real DNS, so it is ignored by default; run with
    /// `cargo test -p ghost-mail -- --ignored` when changing MX handling.
    #[tokio::test]
    #[ignore]
    async fn resolves_real_mx_records_in_preference_order() {
        let hosts = lookup_mx("gmail.com").await.expect("gmail has MX records");
        assert!(!hosts.is_empty());
        assert!(
            hosts[0].contains("google"),
            "lowest-preference host first, got {hosts:?}"
        );
        assert!(hosts.iter().all(|h| !h.ends_with('.')), "trailing dot stripped: {hosts:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn a_domain_with_no_mail_server_is_an_error() {
        let err = lookup_mx("invalid-domain-that-does-not-exist-xyzzy.test").await;
        assert!(err.is_err());
    }
}
