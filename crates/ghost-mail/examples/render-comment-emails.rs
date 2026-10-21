//! Renders the three comment notification emails with the sample data
//! Ghost's own templates were checked against, and writes them to a
//! directory (`/tmp/comment-emails` unless one is given as the first
//! argument).
//!
//! `cargo run -p ghost-mail --example render-comment-emails` produces the
//! exact HTML and text parts a recipient would get. The point is diffing:
//! rendering the same data through Ghost 6.39's
//! `CommentsServiceEmailRenderer` and comparing the two outputs is how the
//! vendored templates are kept honest across a Ghost upgrade.

use ghost_mail::comments::{CommentEmail, Kind};

fn main() {
    let email = CommentEmail {
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
    };

    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp/comment-emails".into());
    std::fs::create_dir_all(&dir).expect("create the output directory");

    // The report email has two shapes, depending on whether the moderation
    // screen is available, so both are written.
    let variants = [
        ("new-comment", Kind::NewComment, email.clone()),
        ("new-comment-reply", Kind::NewCommentReply, email.clone()),
        ("report", Kind::Report, email.clone()),
        (
            "report-moderation",
            Kind::Report,
            CommentEmail { moderation_enabled: true, ..email.clone() },
        ),
    ];

    for (name, kind, e) in variants {
        let html = e.render_html(kind).expect("the template renders");
        std::fs::write(format!("{dir}/{name}.html"), html).expect("write the HTML part");
        std::fs::write(format!("{dir}/{name}.txt"), e.render_text(kind)).expect("write the text part");
        println!("{dir}/{name}.html  ({})", e.subject(kind));
    }
}
