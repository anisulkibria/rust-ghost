//! Mail settings a hosting operator can manage through hostSettings.managedMail.
//! The fixed allowlist prevents a host mail block from overriding unrelated settings.
pub const KEYS: &[&str] = &[
    "mail_transport",
    "mail_smtp_host",
    "mail_smtp_port",
    "mail_smtp_security",
    "mail_smtp_user",
    "mail_smtp_pass",
    "mail_from_address",
    "mail_from_name",
    "mail_reply_to",
    "ses_region",
    "ses_smtp_endpoint",
    "ses_smtp_user",
    "ses_smtp_pass",
    "ses_max_send_rate",
    "newsletter_transport",
    "mailgun_api_key",
    "mailgun_domain",
    "mailgun_base_url",
    "mailgun_webhook_signing_key",
    "bulk_email_from_address",
];

pub fn is_secret(key: &str) -> bool {
    matches!(
        key,
        "ses_smtp_user"
            | "ses_smtp_pass"
            | "mail_smtp_pass"
            | "mailgun_api_key"
            | "mailgun_webhook_signing_key"
    )
}

/// SES SMTP credentials and verified identities belong to this region.
/// Reject malformed regions before deriving a hostname.
pub fn ses_smtp_host(region: &str) -> Option<String> {
    let region = region.trim();
    if region.is_empty()
        || !region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || region.split('-').count() < 3
    {
        return None;
    }
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    };
    Some(format!("email-smtp.{region}.{suffix}"))
}

/// Explicit AWS SMTP endpoints support both regional SES and Mail Manager.
/// A malformed override must never silently fall back to another endpoint.
pub fn ses_smtp_endpoint(region: &str, endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() { return ses_smtp_host(region); }
    let valid = valid_ses_smtp_endpoint(endpoint);
    valid.then(|| endpoint.to_string())
}

fn valid_ses_smtp_endpoint(endpoint: &str) -> bool {
    let aws_smtp = ["amazonaws.com", "amazonaws.com.cn", "amazonaws.eu"].iter().any(|suffix| {
        (endpoint.starts_with("email-smtp.") && endpoint.ends_with(&format!(".{suffix}")))
            || endpoint.ends_with(&format!(".mail-manager-smtp.{suffix}"))
    });
    aws_smtp && endpoint.len() <= 253 && endpoint.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mail_manager_endpoints_override_the_region_without_unsafe_fallback() {
        let endpoint = "example.fips.region.mail-manager-smtp.amazonaws.com";
        assert_eq!(ses_smtp_endpoint("us-east-1", endpoint).as_deref(), Some(endpoint));
        assert_eq!(ses_smtp_endpoint("us-east-1", "").as_deref(), Some("email-smtp.us-east-1.amazonaws.com"));
        for bad in ["http://localhost", "127.0.0.1", "evil.test", "email-smtp.us-east-1.amazonaws.com.evil.test", "example.mail-manager-smtp.amazonaws.com\r\nAUTH LOGIN"] {
            assert!(ses_smtp_endpoint("us-east-1", bad).is_none());
        }
    }

    #[test]
    fn ses_endpoints_are_regional_and_reject_host_injection() {
        assert_eq!(
            ses_smtp_host("us-east-1").as_deref(),
            Some("email-smtp.us-east-1.amazonaws.com")
        );
        assert_eq!(
            ses_smtp_host("cn-north-1").as_deref(),
            Some("email-smtp.cn-north-1.amazonaws.com.cn")
        );
        for bad in [
            "",
            "us-east-1.evil.test",
            "us-east-1/",
            "us-east-1\r\nother",
        ] {
            assert!(ses_smtp_host(bad).is_none());
        }
    }
}
