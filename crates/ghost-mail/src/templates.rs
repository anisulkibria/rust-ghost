//! The member magic-link emails, mirroring Ghost's
//! `core/server/services/members/emails/*.js` — same subjects, same copy, and
//! the same table-based HTML skeleton that survives Outlook and Gmail.

/// Why a magic link was issued. Ghost calls this the email `type`, and it
/// picks both the subject and the body copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkType {
    Signup,
    Signin,
    Subscribe,
    UpdateEmail,
}

impl LinkType {
    /// Ghost resolves the action from whether the member exists, so this maps
    /// the stored action string back to a template.
    pub fn from_action(action: &str) -> Self {
        match action {
            "signup" => LinkType::Signup,
            "subscribe" => LinkType::Subscribe,
            "updateEmail" => LinkType::UpdateEmail,
            _ => LinkType::Signin,
        }
    }

    pub fn subject(self, site_title: &str) -> String {
        match self {
            LinkType::Subscribe => format!("📫 Confirm your subscription to {site_title}"),
            LinkType::Signup => format!("🙌 Complete your sign up to {site_title}!"),
            LinkType::UpdateEmail => format!("📫 Confirm your email update for {site_title}!"),
            LinkType::Signin => format!("🔑 Secure sign in link for {site_title}"),
        }
    }

    fn heading(self, site_title: &str) -> String {
        match self {
            LinkType::Subscribe => format!("Confirm your subscription to {site_title}"),
            LinkType::Signup => format!("Complete signup for {site_title}!"),
            LinkType::UpdateEmail => format!("Confirm your email update for {site_title}!"),
            LinkType::Signin => format!("Sign in to {site_title}"),
        }
    }

    fn lead(self, site_title: &str) -> String {
        match self {
            LinkType::Subscribe => format!(
                "Tap the link below to confirm your subscription to {site_title}:"
            ),
            LinkType::Signup => format!(
                "Tap the link below to complete the signup process for {site_title}, and be automatically signed in:"
            ),
            LinkType::UpdateEmail => {
                "Tap the link below to confirm your new email address:".to_string()
            }
            LinkType::Signin => format!(
                "Welcome back! Use this link to securely sign in to your {site_title} account:"
            ),
        }
    }

    fn button(self) -> &'static str {
        match self {
            LinkType::Subscribe => "Confirm subscription",
            LinkType::Signup => "Complete signup",
            LinkType::UpdateEmail => "Confirm email address",
            LinkType::Signin => "Sign in",
        }
    }

    /// Ghost is careful to say different things here: an unwanted *signup*
    /// promises no account was created, while an unwanted *signin* is just
    /// noise to ignore.
    fn footnote(self) -> &'static str {
        match self {
            LinkType::Signup | LinkType::Subscribe => {
                "If you did not make this request, you can simply delete this message. You will not be signed up, and no account will be created for you."
            }
            _ => "If you did not make this request, you can safely ignore this email.",
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The plain-text alternative. Some clients show only this, and it is the
/// part that still works if the HTML is stripped by a corporate gateway — so
/// the raw URL has to appear here in full.
pub fn magic_link_text(
    kind: LinkType,
    site_title: &str,
    site_domain: &str,
    email: &str,
    url: &str,
) -> String {
    format!(
        "{heading}\n\n{lead}\n\n{url}\n\nFor your security, the link will expire in 24 hours time.\n\n{footnote}\n\n---\n\nThis message was sent from {site_domain} to {email}.",
        heading = kind.heading(site_title),
        lead = kind.lead(site_title),
        url = url,
        footnote = kind.footnote(),
        site_domain = site_domain,
        email = email,
    )
}

/// The HTML part. Tables and inline styles on purpose: mail clients are not
/// browsers, and flexbox/`<div>` layouts fall apart in Outlook.
pub fn magic_link_html(
    kind: LinkType,
    site_title: &str,
    site_domain: &str,
    email: &str,
    url: &str,
    accent_color: &str,
) -> String {
    let accent = if accent_color.trim().is_empty() { "#15212A" } else { accent_color };
    format!(
        r#"<!doctype html>
<html>
<head>
<meta name="viewport" content="width=device-width">
<meta http-equiv="Content-Type" content="text/html; charset=UTF-8">
<meta name="color-scheme" content="light only">
<meta name="supported-color-schemes" content="light">
<title>{subject}</title>
<style>
@media only screen and (max-width: 620px) {{
  h1 {{ font-size: 28px !important; margin-bottom: 10px !important; }}
  p, td, a {{ font-size: 16px !important; }}
  .wrapper {{ padding: 16px !important; }}
  .container {{ padding: 0 !important; width: 100% !important; }}
  .btn a {{ width: 100% !important; }}
}}
</style>
</head>
<body style="background-color:#f6f6f6;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;font-size:16px;line-height:1.5;margin:0;padding:0;color:#15212A;">
<table role="presentation" class="body" border="0" cellpadding="0" cellspacing="0" width="100%" style="background-color:#f6f6f6;">
<tr><td>&nbsp;</td>
<td class="container" style="display:block;max-width:560px;margin:0 auto;padding:24px 10px;">
<table role="presentation" class="main" border="0" cellpadding="0" cellspacing="0" width="100%" style="background:#ffffff;border-radius:10px;">
<tr><td class="wrapper" style="padding:32px;">
<h1 style="font-size:26px;font-weight:700;margin:0 0 20px;line-height:1.2;color:#15212A;">{heading}</h1>
<p style="margin:0 0 24px;color:#15212A;">{lead}</p>
<table role="presentation" class="btn" border="0" cellpadding="0" cellspacing="0" style="margin:0 0 24px;">
<tr><td align="left">
<a href="{url}" target="_blank" rel="noopener" style="display:inline-block;background:{accent};border-radius:6px;color:#ffffff;font-size:16px;font-weight:600;padding:12px 24px;text-decoration:none;">{button}</a>
</td></tr></table>
<p style="margin:0 0 8px;color:#738a94;font-size:14px;">For your security, the link will expire in 24 hours time.</p>
<p style="margin:0 0 6px;color:#738a94;font-size:14px;">You can also copy &amp; paste this URL into your browser:</p>
<p style="margin:0 0 24px;font-size:13px;word-break:break-all;"><a href="{url}" target="_blank" rel="noopener" style="color:{accent};">{url_text}</a></p>
<p style="margin:0;color:#738a94;font-size:14px;">{footnote}</p>
</td></tr></table>
<p style="color:#9baab1;font-size:12px;text-align:center;margin:18px 0 0;">This message was sent from {site_domain} to {email}.</p>
</td>
<td>&nbsp;</td></tr></table>
</body></html>"#,
        subject = esc(&kind.subject(site_title)),
        heading = esc(&kind.heading(site_title)),
        lead = esc(&kind.lead(site_title)),
        button = kind.button(),
        footnote = kind.footnote(),
        // the href is a server-built URL, but the visible copy is escaped
        url = url,
        url_text = esc(url),
        accent = esc(accent),
        site_domain = esc(site_domain),
        email = esc(email),
    )
}

// Newsletters are not rendered here. They go through Ghost's own Handlebars
// templates in `crate::newsletter`, which is the only way the design
// settings — colours, typography, corners, the Koenig card styles — can come
// out looking like a Ghost newsletter.

/// The staff invitation email. Ghost sends the invitee a link that carries
/// the token; following it is what lets them set a name and password.
pub fn staff_invite(
    site_title: &str,
    site_url: &str,
    inviter: &str,
    role: &str,
    url: &str,
) -> (String, String, String) {
    let subject = format!("{inviter} has invited you to join {site_title}");
    let text = format!(
        "{inviter} has invited you to join {site_title} as {article} {role}.\n\nAccept the invitation by opening this link and choosing a password:\n\n{url}\n\nThe invitation expires in one week. If you were not expecting it you can ignore this email — no account is created until the link is used.\n\n{site_url}",
        article = article_for(role),
    );
    let html = format!(
        r#"<!doctype html><html><head><meta name="viewport" content="width=device-width"><meta http-equiv="Content-Type" content="text/html; charset=UTF-8"><meta name="color-scheme" content="light only"></head>
<body style="background:#f6f6f6;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;margin:0;padding:24px;color:#15212A;">
<table role="presentation" width="100%" border="0" cellpadding="0" cellspacing="0"><tr><td align="center">
<table role="presentation" width="100%" border="0" cellpadding="0" cellspacing="0" style="max-width:560px;background:#fff;border-radius:10px;">
<tr><td style="padding:32px;">
<h1 style="font-size:24px;margin:0 0 16px;line-height:1.25;">You have been invited to join {title}</h1>
<p style="margin:0 0 20px;"><strong>{inviter}</strong> has invited you to contribute as {article} <strong>{role}</strong>.</p>
<table role="presentation" border="0" cellpadding="0" cellspacing="0" style="margin:0 0 24px;"><tr><td align="left">
<a href="{url}" target="_blank" rel="noopener" style="display:inline-block;background:#15212A;border-radius:6px;color:#fff;font-size:16px;font-weight:600;padding:12px 24px;text-decoration:none;">Accept the invitation</a>
</td></tr></table>
<p style="margin:0 0 6px;color:#738a94;font-size:14px;">Or paste this into your browser:</p>
<p style="margin:0 0 20px;font-size:13px;word-break:break-all;"><a href="{url}" style="color:#15212A;">{url_text}</a></p>
<p style="margin:0;color:#738a94;font-size:14px;">The invitation expires in one week. If you were not expecting it you can ignore this email — no account is created until the link is used.</p>
</td></tr></table></td></tr></table></body></html>"#,
        title = esc(site_title),
        inviter = esc(inviter),
        article = article_for(role),
        role = esc(role),
        url = url,
        url_text = esc(url),
    );
    (subject, html, text)
}

/// "an Author"/"an Editor"/"an Administrator" but "a Contributor".
fn article_for(role: &str) -> &'static str {
    match role.chars().next() {
        Some('A') | Some('E') | Some('I') | Some('O') | Some('U') => "an",
        _ => "a",
    }
}

/// The "does my configuration work?" message sent from Settings → Email.
pub fn test_email(site_title: &str, site_url: &str) -> (String, String, String) {
    let subject = format!("Test email from {site_title}");
    let text = format!(
        "This is a test email from {site_title}.\n\nIf you are reading this, your transactional email settings are working and member magic links will be delivered.\n\n{site_url}"
    );
    let html = format!(
        r#"<!doctype html><html><head><meta name="viewport" content="width=device-width"><meta http-equiv="Content-Type" content="text/html; charset=UTF-8"></head>
<body style="background:#f6f6f6;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;margin:0;padding:24px;color:#15212A;">
<table role="presentation" width="100%" border="0" cellpadding="0" cellspacing="0"><tr><td align="center">
<table role="presentation" width="100%" border="0" cellpadding="0" cellspacing="0" style="max-width:560px;background:#fff;border-radius:10px;">
<tr><td style="padding:32px;">
<h1 style="font-size:24px;margin:0 0 16px;">Your email settings work 🎉</h1>
<p style="margin:0 0 12px;">This is a test email from <strong>{title}</strong>.</p>
<p style="margin:0;color:#738a94;font-size:14px;">Member magic links for signup and signin will be delivered using these settings.</p>
</td></tr></table></td></tr></table></body></html>"#,
        title = esc(site_title),
    );
    (subject, html, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_match_ghosts() {
        assert_eq!(LinkType::Signup.subject("Acme"), "🙌 Complete your sign up to Acme!");
        assert_eq!(LinkType::Signin.subject("Acme"), "🔑 Secure sign in link for Acme");
        assert_eq!(LinkType::Subscribe.subject("Acme"), "📫 Confirm your subscription to Acme");
    }

    #[test]
    fn action_strings_map_to_templates() {
        assert_eq!(LinkType::from_action("signup"), LinkType::Signup);
        assert_eq!(LinkType::from_action("signin"), LinkType::Signin);
        // anything unrecognised must fall back to the conservative signin
        // copy, which never claims an account was created
        assert_eq!(LinkType::from_action("nonsense"), LinkType::Signin);
    }

    #[test]
    fn signup_and_signin_give_different_safety_advice() {
        assert!(LinkType::Signup.footnote().contains("no account will be created"));
        assert!(LinkType::Signin.footnote().contains("safely ignore"));
    }

    #[test]
    fn both_parts_carry_the_full_url() {
        let url = "https://example.com/members/?token=abc123&action=signup";
        let text = magic_link_text(LinkType::Signup, "Acme", "example.com", "a@b.c", url);
        let html = magic_link_html(LinkType::Signup, "Acme", "example.com", "a@b.c", url, "#ff0000");
        assert!(text.contains(url), "text part must carry a copy-pasteable link");
        assert!(html.contains(url), "html part must link to it");
        // the visible copy is escaped so a & in the query cannot break the markup
        assert!(html.contains("token=abc123&amp;action=signup"));
        assert!(html.contains("#ff0000"), "accent colour is applied to the button");
    }

    #[test]
    fn recipient_and_site_are_escaped_into_the_html() {
        let html = magic_link_html(
            LinkType::Signin,
            "<script>bad</script>",
            "example.com",
            "a@b.c",
            "https://e.com/",
            "",
        );
        assert!(!html.contains("<script>bad"), "site title must not inject markup");
        assert!(html.contains("&lt;script&gt;"));
    }
}
