//! Reused SMTP connections and send pacing for personalized newsletter delivery.
use crate::{build_email, MailConfig, Message, SmtpSecurity, Transport};
use ghost_core::{GhostError, Result};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// One sender for the entire newsletter, retained across progress batches.
/// Each recipient gets their own MIME message and envelope; no recipient list
/// or personalization data is shared with another subscriber.
pub struct RelaySender {
    config: MailConfig,
    transport: AsyncSmtpTransport<Tokio1Executor>,
    next_send: Mutex<Instant>,
    interval: Duration,
}

impl RelaySender {
    pub fn new(config: MailConfig, messages_per_second: u32) -> Result<Self> {
        if let Some(error) = config.configuration_error() {
            return Err(GhostError::EmailError(error));
        }
        let (host, port, security, user, pass) = match &config.transport {
            Transport::Ses { region, endpoint, user, pass } => (
                ghost_core::mail_settings::ses_smtp_endpoint(region, endpoint).expect("validated SES region"),
                587,
                SmtpSecurity::StartTls,
                user,
                pass,
            ),
            Transport::Smtp {
                host,
                port,
                security,
                user,
                pass,
            } => (host.clone(), *port, *security, user, pass),
            Transport::Direct => {
                return Err(GhostError::EmailError(
                    "Newsletter delivery requires a relay".into(),
                ))
            }
        };
        let mut builder = match security {
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&host),
            SmtpSecurity::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host),
            SmtpSecurity::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &host,
            )),
        }
        .map_err(|e| GhostError::EmailError(format!("newsletter SMTP configuration: {e}")))?
        .port(port)
        .timeout(Some(Duration::from_secs(30)));
        if !user.is_empty() {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }
        Ok(Self {
            config,
            transport: builder.build(),
            next_send: Mutex::new(Instant::now()),
            interval: Duration::from_secs_f64(1.0 / messages_per_second.clamp(1, 10_000) as f64),
        })
    }

    pub fn config(&self) -> &MailConfig {
        &self.config
    }

    async fn pace(&self) {
        let mut next = self.next_send.lock().await;
        tokio::time::sleep_until(*next).await;
        *next = Instant::now() + self.interval;
    }

    pub async fn send(&self, message: &Message, unsubscribe_url: Option<&str>) -> Result<()> {
        crate::metered(self.config.meter.as_ref(), "newsletter", crate::provider(&self.config), 1, self.send_unmetered(message, unsubscribe_url)).await
    }

    async fn send_unmetered(&self, message: &Message, unsubscribe_url: Option<&str>) -> Result<()> {
        let mut email = build_email(&self.config, message)?;
        if let Some(url) = unsubscribe_url {
            use lettre::message::header::{HeaderName, HeaderValue};
            email.headers_mut().insert_raw(HeaderValue::new(
                HeaderName::new_from_ascii_str("List-Unsubscribe"),
                format!("<{url}>"),
            ));
        }
        // Retry only explicit SMTP 4xx refusals: a network error can occur
        // after acceptance and retrying it could send a duplicate message.
        for attempt in 0..=4 {
            self.pace().await;
            match self.transport.send(email.clone()).await {
                Ok(_) => return Ok(()),
                Err(error) if error.is_transient() && attempt < 4 => {
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
                Err(error) => {
                    return Err(GhostError::EmailError(format!("newsletter SMTP: {error}")))
                }
            }
        }
        unreachable!("last attempt returns")
    }
}
