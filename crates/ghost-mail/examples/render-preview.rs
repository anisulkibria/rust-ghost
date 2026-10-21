//! Renders a newsletter email with representative sample data and writes it
//! to the path given as the first argument (or `/tmp/newsletter.html`).
//!
//! A design tool more than a demo: `cargo run -p ghost-mail --example
//! render-preview -- /tmp/out.html` produces the exact HTML a subscriber
//! would get, for opening in a browser while tuning the templates.

use ghost_core::dates;
use ghost_db::repos::newsletters::Newsletter;
use ghost_mail::newsletter::{NewsletterEmail, PostContext, Recipient, SiteContext};

fn main() {
    // Ghost's defaults for a fresh newsletter — the design every subscriber
    // sees before anyone touches the settings.
    let newsletter = Newsletter {
        id: "n1".into(),
        uuid: None,
        name: "Ghost Newsletter".into(),
        description: None,
        slug: "default-newsletter".into(),
        sender_name: None,
        sender_email: None,
        sender_reply_to: "newsletter".into(),
        status: "active".into(),
        visibility: "members".into(),
        subscribe_on_signup: true,
        sort_order: 0,
        header_image: None,
        show_header_icon: true,
        show_header_title: true,
        show_header_name: true,
        show_post_title_section: true,
        show_excerpt: true,
        show_feature_image: true,
        title_font_category: "sans_serif".into(),
        title_alignment: "center".into(),
        title_font_weight: "bold".into(),
        body_font_category: "sans_serif".into(),
        background_color: "light".into(),
        header_background_color: "transparent".into(),
        post_title_color: None,
        section_title_color: None,
        divider_color: None,
        link_color: Some("accent".into()),
        link_style: "underline".into(),
        button_color: Some("accent".into()),
        button_style: "fill".into(),
        button_corners: "rounded".into(),
        image_corners: "square".into(),
        footer_content: None,
        show_badge: true,
        show_comment_cta: true,
        show_subscription_details: false,
        show_latest_posts: true,
        show_share_button: true,
        feedback_enabled: true,
        created_at: None,
        updated_at: None,
    };

    let site = SiteContext {
        title: "Ghost".into(),
        url: "https://example.com".into(),
        icon_url: Some("https://example.com/icon.png".into()),
        locale: "en".into(),
        direction: "ltr".into(),
        accent_color: "#05bd76".into(),
    };
    let post = PostContext {
        id: "p1".into(),
        title: "Hello Rust Ghost".into(),
        url: "https://example.com/hello-rust-ghost/".into(),
        html: r#"<p>Hello, this is testing from Rust Ghost. This paragraph has an <a href="https://example.com/">inline link</a> so the link styling shows.</p><h2>A section heading</h2><blockquote><p>A short quotation to show how blockquotes read.</p></blockquote>"#.into(),
        plaintext: "Hello, this is testing from Rust Ghost.".into(),
        custom_excerpt: Some("A short excerpt under the title, exactly as subscribers see it.".into()),
        published_at: "20 Oct 2026".into(),
        authors: "Ghost".into(),
        feature_image: Some("https://example.com/feature.jpg".into()),
        feature_image_width: Some(1200),
        ..Default::default()
    };
    let latest = [ghost_mail::newsletter::LatestPost {
        title: "An earlier dispatch".into(),
        url: "https://example.com/earlier/".into(),
        feature_image: Some("https://example.com/earlier.jpg".into()),
        excerpt: None,
    }];

    let email = NewsletterEmail {
        site: &site,
        newsletter: &newsletter,
        post: &post,
        latest_posts: &latest,
        comments_enabled: true,
    };
    let html = email.render().expect("the template renders");
    let member = Recipient {
        uuid: "u1".into(),
        email: "subscriber@example.com".into(),
        name: Some("Subscriber".into()),
        status: "free".into(),
        created_at: dates::short_date_in_zone(&dates::now_db(), "Etc/UTC"),
        unsubscribe_url: "https://example.com/unsub/".into(),
        manage_account_url: "https://example.com/#/portal/account".into(),
        key: "k1".into(),
    };
    let out = ghost_mail::newsletter::personalize(&html, &member);

    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/newsletter.html".into());
    std::fs::write(&path, out).expect("write the rendered email");
    println!("wrote {path}");
}
