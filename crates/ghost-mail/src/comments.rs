//! The three comment notification emails.
//!
//! Ghost sends these from `core/server/services/comments/`:
//!
//! * **new-comment** — to each of the post's authors who has staff comment
//!   notifications on.
//! * **new-comment-reply** — to the member whose comment was replied to.
//! * **report** — to the site owner when a member reports a comment.
//!
//! The `.hbs` files under `assets/comment-templates/` are Ghost 6.39's own,
//! copied byte for byte, and rendered here with the same two helpers Ghost's
//! `CommentsServiceEmailRenderer` registers (`t` and `concat`). Keeping the
//! templates rather than reimplementing the markup is the only way the
//! output can be identical — these are 14 KB of inlined table layout each,
//! tuned against Outlook, and any paraphrase of them is a different email.
//!
//! The plain-text parts are ports of the `*.txt.js` siblings, which are
//! template literals rather than Handlebars, so they are built in Rust.

use handlebars::Handlebars;
use serde_json::{json, Value};

use ghost_core::{GhostError, Result};

const NEW_COMMENT: &str = include_str!("../assets/comment-templates/new-comment.hbs");
const NEW_COMMENT_REPLY: &str = include_str!("../assets/comment-templates/new-comment-reply.hbs");
const REPORT: &str = include_str!("../assets/comment-templates/report.hbs");

/// Guard against a template going missing in a refactor: an empty one would
/// send a blank email rather than fail.
const _: () = {
    assert!(!NEW_COMMENT.is_empty());
    assert!(!NEW_COMMENT_REPLY.is_empty());
    assert!(!REPORT.is_empty());
};

/// Which notification to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    NewComment,
    NewCommentReply,
    Report,
}

impl Kind {
    fn template(self) -> &'static str {
        match self {
            Kind::NewComment => NEW_COMMENT,
            Kind::NewCommentReply => NEW_COMMENT_REPLY,
            Kind::Report => REPORT,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::NewComment => "new-comment",
            Kind::NewCommentReply => "new-comment-reply",
            Kind::Report => "report",
        }
    }
}

/// Everything the three templates read. One struct rather than three, since
/// they overlap almost entirely and the unused fields render as empty.
#[derive(Debug, Default, Clone)]
pub struct CommentEmail {
    pub site_title: String,
    pub site_url: String,
    pub site_domain: String,
    pub post_title: String,
    /// Already carrying the `#ghost-comments-<id>` fragment, so the link
    /// scrolls to the comment rather than the top of the post.
    pub post_url: String,
    pub comment_html: String,
    /// The plain-text rendering of `comment_html`, for the report email.
    pub comment_text: String,
    pub comment_date: String,
    pub member_name: String,
    pub member_email: String,
    pub member_expertise: Option<String>,
    pub member_initials: String,
    pub accent_color: String,
    pub from_email: String,
    pub to_email: String,
    /// Where the recipient turns these emails off. Staff go to their own
    /// email-notifications screen; a member goes to the unsubscribe page.
    pub staff_url: String,
    pub profile_url: String,
    pub reporter_name: Option<String>,
    pub reporter_email: String,
    /// Ghost's `commentModeration` lab flag, which adds a second link
    /// straight into the admin moderation screen.
    pub moderation_enabled: bool,
    pub moderation_url: String,
}

impl CommentEmail {
    /// `"Name (email)"` if the reporter gave a name, else just the address —
    /// Ghost's `reporter` field.
    fn reporter(&self) -> String {
        match self.reporter_name.as_deref().filter(|n| !n.is_empty()) {
            Some(name) => format!("{name} ({})", self.reporter_email),
            None => self.reporter_email.clone(),
        }
    }

    fn data(&self, kind: Kind) -> Value {
        json!({
            "siteTitle": self.site_title,
            "siteUrl": self.site_url,
            "siteDomain": self.site_domain,
            "postTitle": self.post_title,
            "postUrl": self.post_url,
            // The reply template names the body `replyHtml`; the other two
            // call the same value `commentHtml`.
            "commentHtml": self.comment_html,
            "replyHtml": self.comment_html,
            "commentText": self.comment_text,
            "commentDate": self.comment_date,
            "replyDate": self.comment_date,
            "memberName": self.member_name,
            "memberEmail": self.member_email,
            "memberExpertise": self.member_expertise,
            "memberInitials": self.member_initials,
            "accentColor": self.accent_color,
            "fromEmail": self.from_email,
            "toEmail": self.to_email,
            "staffUrl": self.staff_url,
            "profileUrl": self.profile_url,
            "reporterName": self.reporter_name,
            "reporterEmail": self.reporter_email,
            "reporter": self.reporter(),
            "commentModerationEnabled": self.moderation_enabled,
            "moderationUrl": self.moderation_url,
            "_kind": kind.name(),
        })
    }

    /// Ghost's subjects, including the emoji — they are what makes these
    /// recognisable in a crowded inbox, and they are part of the string.
    pub fn subject(&self, kind: Kind) -> String {
        match kind {
            Kind::NewComment => format!("💬 New comment on your post: {}", self.post_title),
            Kind::NewCommentReply => {
                format!("↪️ New reply to your comment on {}", self.site_title)
            }
            Kind::Report => "🚩 A comment has been reported on your post".into(),
        }
    }

    pub fn render_html(&self, kind: Kind) -> Result<String> {
        let mut hb = Handlebars::new();
        hb.register_escape_fn(escape_expression);
        hb.register_helper("t", Box::new(TranslateHelper));
        hb.register_helper("concat", Box::new(ConcatHelper));
        hb.register_template_string(kind.name(), kind.template())
            .map_err(|e| GhostError::Render(e.to_string()))?;
        hb.render(kind.name(), &self.data(kind))
            .map_err(|e| GhostError::Render(e.to_string()))
    }

    /// The plain-text alternative, ported from Ghost's `*.txt.js`.
    ///
    /// The leading and trailing whitespace is deliberate where Ghost has it:
    /// these are template literals, and the newlines inside them are visible
    /// in a mail client that shows the text part.
    pub fn render_text(&self, kind: Kind) -> String {
        match kind {
            Kind::NewComment => format!(
                "\nHey there,\n\nSomeone just posted a comment on your post \"{post_title}\"\n\n{post_url}\n\n---\n\nSent to {to} from {domain}.\nYou can manage your notification preferences at {staff_url}.\n    ",
                post_title = self.post_title,
                post_url = self.post_url,
                to = self.to_email,
                domain = self.site_domain,
                staff_url = self.staff_url,
            ),
            Kind::NewCommentReply => format!(
                "Hey there,\n\nSomeone just replied to your comment on {post_title}.\n\n{post_url}\n\n---\n\nThis message was sent from {domain} to {to}.\nYou can unsubscribe from these notifications at {profile_url}.",
                post_title = self.post_title,
                post_url = self.post_url,
                domain = self.site_domain,
                to = self.to_email,
                profile_url = self.profile_url,
            ),
            Kind::Report => {
                let visibility_note = if self.moderation_enabled {
                    "This comment will remain visible until you choose to remove it."
                } else {
                    "This comment will remain visible until you choose to remove it, which can be done directly on the post."
                };
                let action_links = if self.moderation_enabled {
                    format!(
                        "View comment: {}\nModerate comment: {}",
                        self.post_url, self.moderation_url
                    )
                } else {
                    self.post_url.clone()
                };
                format!(
                    "Hey there,\n\n{reporter} has reported the comment below on {post_title}. {note}\n\n{member_name} ({member_email}):\n{comment_text}\n{links}\n\n---\n\nSent to {to} from {domain}.\nYou can manage your notification preferences at {staff_url}.",
                    reporter = self.reporter(),
                    post_title = self.post_title,
                    note = visibility_note,
                    member_name = self.member_name,
                    member_email = self.member_email,
                    comment_text = self.comment_text,
                    links = action_links,
                    to = self.to_email,
                    domain = self.site_domain,
                    staff_url = self.staff_url,
                )
            }
        }
    }

    /// Render both parts into a ready-to-send message.
    pub fn message(&self, kind: Kind) -> Result<crate::Message> {
        Ok(crate::Message {
            to: self.to_email.clone(),
            subject: self.subject(kind),
            html: self.render_html(kind)?,
            text: self.render_text(kind),
        })
    }
}

/// handlebars.js's `Handlebars.escapeExpression`, which is what a `{{…}}`
/// expression in these templates goes through.
///
/// It is not the same set handlebars-rust escapes by default: handlebars.js
/// also escapes `` ` `` and `=`, and it escapes every `&` rather than
/// skipping ones that already open an entity. Both differences are visible
/// in the output — an unsubscribe URL's `?uuid=u&key=k` comes out as
/// `?uuid&#x3D;u&amp;key&#x3D;k` — so matching Ghost here is the difference
/// between an identical email and a nearly identical one.
fn escape_expression(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '=' => out.push_str("&#x3D;"),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the two helpers Ghost's renderer registers
// ---------------------------------------------------------------------------

/// `{{t 'Some string' param=value}}` — i18next with `{param}` placeholders.
///
/// Ghost looks the key up in the active locale and falls back to the key
/// itself, which is English. The templates' keys *are* the English copy, so
/// for the default locale this is interpolation and nothing else.
struct TranslateHelper;

impl handlebars::HelperDef for TranslateHelper {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &handlebars::Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc handlebars::Context,
        _rc: &mut handlebars::RenderContext<'reg, 'rc>,
    ) -> std::result::Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let mut s = h.param(0).and_then(|p| p.value().as_str()).unwrap_or_default().to_string();
        for (key, value) in h.hash() {
            let text = match value.value() {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            s = s.replace(&format!("{{{key}}}"), &text);
        }
        Ok(handlebars::ScopedJson::Derived(Value::String(s)))
    }
}

/// `{{concat 'a' b 'c'}}` — joins its arguments, which is how the templates
/// build a link out of a translated string's placeholder.
struct ConcatHelper;

impl handlebars::HelperDef for ConcatHelper {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &handlebars::Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc handlebars::Context,
        _rc: &mut handlebars::RenderContext<'reg, 'rc>,
    ) -> std::result::Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let joined: String = h
            .params()
            .iter()
            .map(|p| match p.value() {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            })
            .collect();
        Ok(handlebars::ScopedJson::Derived(Value::String(joined)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CommentEmail {
        CommentEmail {
            site_title: "Test Site".into(),
            site_url: "https://site.test".into(),
            site_domain: "site.test".into(),
            post_title: "A Post Title".into(),
            post_url: "https://site.test/a-post/#ghost-comments-abc".into(),
            comment_html: "<p>Nice work</p>".into(),
            comment_text: "Nice work".into(),
            comment_date: "1 Jan 2026".into(),
            member_name: "Ada Lovelace".into(),
            member_email: "ada@site.test".into(),
            member_expertise: Some("Engineer".into()),
            member_initials: "AL".into(),
            accent_color: "#FF1A75".into(),
            from_email: "noreply@site.test".into(),
            to_email: "owner@site.test".into(),
            staff_url: "https://site.test/ghost/#/settings/staff/owner/email-notifications".into(),
            profile_url: "https://site.test/unsubscribe/?uuid=u&key=k&comments=1".into(),
            reporter_name: Some("Bob".into()),
            reporter_email: "bob@site.test".into(),
            moderation_enabled: false,
            moderation_url: "https://site.test/ghost/#/comments/?id=is:abc".into(),
        }
    }

    /// The expected strings below are taken from running Ghost 6.39's own
    /// `CommentsServiceEmailRenderer` over the same data.
    #[test]
    fn new_comment_html_matches_ghost() {
        let html = sample().render_html(Kind::NewComment).unwrap();
        assert!(html.contains("<title>💬 New comment on A Post Title</title>"));
        assert!(html.contains("Someone left a comment on your post"));
        assert!(html.contains(
            r#"Someone just left a comment on your post <a href="https://site.test/a-post/#ghost-comments-abc" target="_blank" style="font-weight: bold; text-decoration: underline; color: #15212A;">A Post Title</a>."#
        ));
        // the comment body goes in raw, not escaped
        assert!(html.contains("<p>Nice work</p>"));
        assert!(html.contains("Engineer &#8226; 1 Jan 2026"));
        assert!(html.contains("AL"));
        assert!(html.contains(r#"background-color: #FF1A75"#));
        assert!(html.contains(">View comments</a>"));
        assert!(!html.contains("{{"), "unresolved template tag in output");
    }

    #[test]
    fn reply_html_resolves_the_translated_link() {
        let html = sample().render_html(Kind::NewCommentReply).unwrap();
        // `{{{t '…{postTitle}…' postTitle=(concat …)}}}` has to come out as
        // a real anchor, not an escaped one
        assert!(html.contains(
            r#"Someone just replied to your comment on <a href="https://site.test/a-post/#ghost-comments-abc" target="_blank" style="font-weight: bold; text-decoration: underline; color: #15212A;">A Post Title</a>."#
        ));
        assert!(html.contains(
            r#"This message was sent from <a class="small" href="https://site.test" style="text-decoration: underline; color: #738A94; font-size: 11px;">site.test</a> to <a class="small" href="mailto:owner@site.test" style="text-decoration: underline; color: #738A94; font-size: 11px;">owner@site.test</a>."#
        ));
        assert!(html.contains("Unsubscribe from comment reply notifications"));
        assert!(html.contains("<p>Nice work</p>"));
        assert!(!html.contains("{{"), "unresolved template tag in output");
    }

    #[test]
    fn report_html_switches_on_the_moderation_flag() {
        let plain = sample().render_html(Kind::Report).unwrap();
        assert!(plain.contains("Bob (bob@site.test)"));
        assert!(plain.contains("which can be done directly on the post"));
        assert!(!plain.contains("https://site.test/ghost/#/comments/"));

        let moderated = CommentEmail { moderation_enabled: true, ..sample() }
            .render_html(Kind::Report)
            .unwrap();
        // handlebars.js escapes `=` in a `{{…}}` expression, so the href
        // carries `&#x3D;` — the browser decodes it back to `=`
        assert!(
            moderated.contains("https://site.test/ghost/#/comments/?id&#x3D;is:abc"),
            "{moderated}"
        );
        assert!(!moderated.contains("which can be done directly on the post"));
        assert!(!moderated.contains("{{"), "unresolved template tag in output");
    }

    #[test]
    fn expertise_is_omitted_when_unset() {
        let html = CommentEmail { member_expertise: None, ..sample() }
            .render_html(Kind::NewComment)
            .unwrap();
        assert!(html.contains(">1 Jan 2026<"));
        assert!(!html.contains("&#8226;"));
    }

    #[test]
    fn subjects_match_ghost() {
        let e = sample();
        assert_eq!(e.subject(Kind::NewComment), "💬 New comment on your post: A Post Title");
        assert_eq!(e.subject(Kind::NewCommentReply), "↪️ New reply to your comment on Test Site");
        assert_eq!(e.subject(Kind::Report), "🚩 A comment has been reported on your post");
    }

    #[test]
    fn text_parts_match_ghost() {
        let e = sample();
        assert_eq!(
            e.render_text(Kind::NewComment),
            "\nHey there,\n\nSomeone just posted a comment on your post \"A Post Title\"\n\nhttps://site.test/a-post/#ghost-comments-abc\n\n---\n\nSent to owner@site.test from site.test.\nYou can manage your notification preferences at https://site.test/ghost/#/settings/staff/owner/email-notifications.\n    "
        );
        assert_eq!(
            e.render_text(Kind::NewCommentReply),
            "Hey there,\n\nSomeone just replied to your comment on A Post Title.\n\nhttps://site.test/a-post/#ghost-comments-abc\n\n---\n\nThis message was sent from site.test to owner@site.test.\nYou can unsubscribe from these notifications at https://site.test/unsubscribe/?uuid=u&key=k&comments=1."
        );
        assert!(e.render_text(Kind::Report).contains("Bob (bob@site.test) has reported the comment below on A Post Title."));
        assert!(CommentEmail { moderation_enabled: true, ..sample() }
            .render_text(Kind::Report)
            .contains("Moderate comment: https://site.test/ghost/#/comments/?id=is:abc"));
    }

    /// handlebars.js escapes more than handlebars-rust does by default, and
    /// the extra characters show up in real URLs.
    #[test]
    fn escaping_matches_handlebars_js() {
        assert_eq!(
            escape_expression("?uuid=u&key=k&comments=1"),
            "?uuid&#x3D;u&amp;key&#x3D;k&amp;comments&#x3D;1"
        );
        assert_eq!(escape_expression("a & b"), "a &amp; b");
        // unlike a sanitiser, this does double-escape an existing entity
        assert_eq!(escape_expression("&amp;"), "&amp;amp;");
        assert_eq!(escape_expression("<i>'`</i>"), "&lt;i&gt;&#x27;&#x60;&lt;/i&gt;");
    }

    #[test]
    fn an_anonymous_reporter_is_just_an_address() {
        assert_eq!(
            CommentEmail { reporter_name: None, ..sample() }.reporter(),
            "bob@site.test"
        );
    }
}
