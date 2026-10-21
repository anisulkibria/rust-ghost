//! Newsletter rendering, ported from Ghost's `email-service`.
//!
//! The templates in `assets/email-templates/` are Ghost's own
//! (`core/server/services/email-service/email-templates/` plus
//! `core/server/services/email-rendering/partials/`), copied verbatim so the
//! markup and CSS — including every Koenig card style — match what Ghost
//! sends. This module is the Rust half of `email-renderer.js`: it resolves
//! the design settings, builds the template context, and renders it.
//!
//! Like Ghost, the rendered document is then inlined — the stylesheet is
//! moved into `style` attributes, with only the rules that cannot live in an
//! attribute left in `<head>`. Ghost uses `juice` for this; here it is the
//! `css-inline` crate. It is not cosmetic: Gmail discards a `<style>` block
//! the size of these templates' and renders the mail as bare tables.
//!
//! Two deliberate differences from Ghost remain, each marked at the point it
//! matters:
//!
//! 1. Ghost measures every image to emit `width`/`height` attributes. There
//!    is no image prober here, so the caller supplies a width when it knows
//!    one and the attribute is omitted otherwise — `max-width: 100%` in the
//!    stylesheet keeps images inside the 600px column either way.
//! 2. juice materialises `::before`/`::after` content as real elements.
//!    `css-inline` has no equivalent, so those rules are kept in the
//!    `<style>` block instead and only Outlook for Windows misses them.
//!
//! Three `{{else if}}` chains in `styles.hbs` are written as nested
//! `{{else}}{{#if}}`, which handlebars-rust parses and means the same thing.

use ghost_core::{GhostError, Result};
use ghost_db::repos::newsletters::Newsletter;
use handlebars::{handlebars_helper, Handlebars};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// handlebars registry
// ---------------------------------------------------------------------------

/// JavaScript truthiness, which is what Ghost's own `and`/`or`/`not` helpers
/// use. It differs from handlebars-rust's in one way that matters here: an
/// empty object or array is truthy in JS, and `feedbackButtons` is an object.
fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

// handlebars-rust ships `and`/`or` but they take exactly two arguments, and
// the template calls `(or feedbackButtons newsletter.showCommentCta
// post.shareUrl)` with three — so these replace them with Ghost's variadic
// versions.
handlebars_helper!(hb_and: |*args| args.iter().all(|v| js_truthy(v)));
handlebars_helper!(hb_or: |*args| args.iter().any(|v| js_truthy(v)));
handlebars_helper!(hb_not: |*args| args.iter().any(|v| !js_truthy(v)));
handlebars_helper!(hb_eq: |a: Json, b: Json| a == b);

// Ghost's i18n helper. This port has no translation catalogue, so the key
// *is* the English string — all that is left to do is interpolate the named
// arguments, exactly as i18next would.
//
// Only names present in the hash are substituted, which is what keeps the
// `%%{status}%%` replacement tokens in `template.hbs` intact: they look like
// `{status}` to a naive replacer.
handlebars_helper!(hb_t: |key: Json, **kwargs| {
    let mut out = key.as_str().map(str::to_string).unwrap_or_else(|| key.to_string());
    for (name, value) in kwargs.iter() {
        let v = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        out = out.replace(&format!("{{{name}}}"), &v);
    }
    out
});

const TEMPLATE: &str = include_str!("../assets/email-templates/template.hbs");
const EMAIL_WRAPPER: &str = include_str!("../assets/email-templates/partials/email-wrapper.hbs");
const STYLES: &str = include_str!("../assets/email-templates/partials/styles.hbs");
const BASE_STYLES: &str = include_str!("../assets/email-templates/partials/base-styles.hbs");
const CONTENT_STYLES: &str = include_str!("../assets/email-templates/partials/content-styles.hbs");
const CARD_STYLES: &str = include_str!("../assets/email-templates/partials/card-styles.hbs");
const LATEST_POSTS: &str = include_str!("../assets/email-templates/partials/latest-posts.hbs");
const FEEDBACK_BUTTON: &str = include_str!("../assets/email-templates/partials/feedback-button.hbs");
const PAYWALL: &str = include_str!("../assets/email-templates/partials/paywall.hbs");

/// The compiled templates. Built once: compiling ~3,000 lines of partials per
/// send would dominate the cost of rendering a newsletter.
static REGISTRY: Lazy<Handlebars<'static>> = Lazy::new(|| {
    let mut hb = Handlebars::new();
    hb.register_helper("and", Box::new(hb_and));
    hb.register_helper("or", Box::new(hb_or));
    hb.register_helper("not", Box::new(hb_not));
    hb.register_helper("eq", Box::new(hb_eq));
    hb.register_helper("t", Box::new(hb_t));

    // Partial names are the ones the templates use, which are Ghost's
    // `registerPartial` names rather than the file names.
    for (name, source) in [
        ("emailWrapper", EMAIL_WRAPPER),
        ("styles", STYLES),
        ("baseStyles", BASE_STYLES),
        ("contentStyles", CONTENT_STYLES),
        ("cardStyles", CARD_STYLES),
        ("latestPosts", LATEST_POSTS),
        ("feedbackButton", FEEDBACK_BUTTON),
        ("paywall", PAYWALL),
    ] {
        // These are compiled-in assets: a parse failure is a bug in this
        // crate, not something a running site can cause or recover from.
        hb.register_partial(name, source)
            .unwrap_or_else(|e| panic!("newsletter partial {name} does not compile: {e}"));
    }
    hb.register_template_string("newsletter", TEMPLATE)
        .expect("newsletter template does not compile");
    hb
});

// ---------------------------------------------------------------------------
// design resolution — a port of email-rendering/email-design.js
// ---------------------------------------------------------------------------

const DEFAULT_ACCENT_COLOR: &str = "#15212A";
const DEFAULT_DIVIDER_COLOR: &str = "#e0e7eb";

/// `#abc` or `#aabbcc`, the only forms Ghost accepts.
///
/// Public because the settings form has to make the same judgement: a colour
/// column holds either one of these or a sentinel like `accent`, and the two
/// are told apart by exactly this test.
pub fn is_hex_color(v: &str) -> bool {
    let body = match v.strip_prefix('#') {
        Some(b) => b,
        None => return false,
    };
    (body.len() == 3 || body.len() == 6) && body.bytes().all(|b| b.is_ascii_hexdigit())
}

fn hex_or(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(v) if is_hex_color(v) => v.to_string(),
        _ => fallback.to_string(),
    }
}

/// Black or white, whichever reads on `background`.
///
/// The YIQ threshold of 186 is `@tryghost/color-utils`'
/// `textColorForBackgroundColor`, shared with Portal — so a background that
/// Ghost calls dark is dark here too, and the whole template agrees with it.
fn text_color_for_background(background: &str) -> &'static str {
    let (r, g, b) = parse_hex(background).unwrap_or((255, 255, 255));
    let yiq = r as f64 * 0.299 + g as f64 * 0.587 + b as f64 * 0.114;
    if yiq >= 186.0 {
        "#000000"
    } else {
        "#FFFFFF"
    }
}

fn parse_hex(v: &str) -> Option<(u8, u8, u8)> {
    let body = v.strip_prefix('#')?;
    let full = match body.len() {
        3 => body.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => body.to_string(),
        _ => return None,
    };
    let n = u32::from_str_radix(&full, 16).ok()?;
    Some((((n >> 16) & 0xff) as u8, ((n >> 8) & 0xff) as u8, (n & 0xff) as u8))
}

/// A background is "dark" when the readable text on it is white.
fn is_dark(color: &str) -> bool {
    text_color_for_background(color) == "#FFFFFF"
}

/// Everything the stylesheet needs, resolved from the newsletter's raw
/// settings and the site accent colour.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailDesign {
    pub accent_color: String,
    pub accent_contrast_color: String,
    pub background_color: String,
    pub background_is_dark: bool,
    pub button_border_radius: String,
    pub button_color: String,
    pub button_text_color: String,
    pub divider_color: String,
    pub has_outline_buttons: bool,
    pub has_rounded_image_corners: bool,
    pub header_background_color: Option<String>,
    pub header_background_is_dark: bool,
    pub link_color: String,
    pub link_style: String,
    pub post_title_color: String,
    pub section_title_color: Option<String>,
    pub text_color: String,
    pub title_strong_weight: String,
    pub title_weight: String,
}

/// Resolve a newsletter's design. Mirrors `getEmailDesign` branch for branch,
/// including the three-way distinction Ghost draws between a NULL colour
/// column, the literal `'accent'`, and a hex value — they mean different
/// things, and collapsing them would change what subscribers see.
pub fn email_design(accent_color: &str, n: &Newsletter) -> EmailDesign {
    let accent = hex_or(Some(accent_color), DEFAULT_ACCENT_COLOR);
    let accent_contrast = text_color_for_background(&accent).to_string();

    // `background_color` holds 'light' by default, which is not a hex value
    // and so falls through to white — the same way it does in Ghost.
    let background = hex_or(Some(&n.background_color), "#ffffff");

    let button_border_radius = match n.button_corners.as_str() {
        "square" => "0",
        "pill" => "9999px",
        _ => "6px",
    }
    .to_string();

    let button_color = match n.button_color.as_deref() {
        Some("accent") => accent.clone(),
        None => text_color_for_background(&background).to_string(),
        other => hex_or(other, &accent),
    };

    let divider_color = match n.divider_color.as_deref() {
        Some("accent") => accent.clone(),
        other => hex_or(other, DEFAULT_DIVIDER_COLOR),
    };

    // 'transparent' (the column default) is neither 'accent' nor a hex
    // value, so it resolves to None — which the template reads as "no header
    // background", not as a colour.
    let header_background_color = match n.header_background_color.as_str() {
        "accent" => Some(accent.clone()),
        v if is_hex_color(v) => Some(v.to_string()),
        _ => None,
    };

    let link_color = match n.link_color.as_deref() {
        Some("accent") => accent.clone(),
        None => text_color_for_background(&background).to_string(),
        other => hex_or(other, &accent),
    };

    let post_title_color = match n.post_title_color.as_deref() {
        Some("accent") => accent.clone(),
        Some(v) if is_hex_color(v) => v.to_string(),
        // Falls back to whatever reads on the surface the title sits on,
        // which is the header background when there is one.
        _ => text_color_for_background(
            header_background_color.as_deref().unwrap_or(&background),
        )
        .to_string(),
    };

    let section_title_color = match n.section_title_color.as_deref() {
        Some("accent") => Some(accent.clone()),
        Some(v) if is_hex_color(v) => Some(v.to_string()),
        _ => None,
    };

    let title_weight: u32 = match n.title_font_weight.as_str() {
        "normal" => 400,
        "medium" => 500,
        "semibold" => 600,
        _ => 700,
    };

    EmailDesign {
        accent_contrast_color: accent_contrast,
        background_is_dark: is_dark(&background),
        button_border_radius,
        button_text_color: text_color_for_background(&button_color).to_string(),
        has_outline_buttons: n.button_style == "outline",
        has_rounded_image_corners: n.image_corners == "rounded",
        header_background_is_dark: is_dark(
            header_background_color.as_deref().unwrap_or(&background),
        ),
        header_background_color,
        link_color,
        link_style: match n.link_style.as_str() {
            "" => "underline".to_string(),
            s => s.to_string(),
        },
        post_title_color,
        section_title_color,
        text_color: text_color_for_background(&background).to_string(),
        title_strong_weight: (if title_weight < 700 { "700" } else { "800" }).to_string(),
        title_weight: title_weight.to_string(),
        button_color,
        divider_color,
        background_color: background,
        accent_color: accent,
    }
}

// ---------------------------------------------------------------------------
// what the caller supplies
// ---------------------------------------------------------------------------

/// The site, as the email header presents it.
#[derive(Debug, Clone)]
pub struct SiteContext {
    pub title: String,
    /// Absolute, no trailing slash expectations — used verbatim in links.
    pub url: String,
    pub icon_url: Option<String>,
    /// BCP-47-ish, straight from the `locale` setting.
    pub locale: String,
    /// `ltr` or `rtl`.
    pub direction: String,
    pub accent_color: String,
}

/// The post being sent.
#[derive(Debug, Clone, Default)]
pub struct PostContext {
    pub id: String,
    pub title: String,
    /// Absolute URL of the post on the site.
    pub url: String,
    /// The post body, already rendered and with absolute asset URLs.
    pub html: String,
    pub plaintext: String,
    pub custom_excerpt: Option<String>,
    /// Pre-formatted, e.g. "20 Oct 2026" — see the module note on dates.
    pub published_at: String,
    /// Ghost's own joining: "A & B" for two, "A & 2 others" beyond that.
    pub authors: String,
    pub feature_image: Option<String>,
    pub feature_image_width: Option<u32>,
    pub feature_image_alt: Option<String>,
    pub feature_image_caption: Option<String>,
    /// An email-only post has no page to share, comment on or read online.
    pub email_only: bool,
    /// Whether the body was truncated at the paywall for this segment.
    pub paywall: bool,
}

/// One of the "Keep reading" entries.
#[derive(Debug, Clone)]
pub struct LatestPost {
    pub title: String,
    pub url: String,
    pub feature_image: Option<String>,
    pub excerpt: Option<String>,
}

/// The per-member values that replace the `%%{…}%%` tokens.
#[derive(Debug, Clone)]
pub struct Recipient {
    pub uuid: String,
    pub email: String,
    pub name: Option<String>,
    /// `free` | `paid` | `comped` | `gift`.
    pub status: String,
    /// Pre-formatted member-since date.
    pub created_at: String,
    pub unsubscribe_url: String,
    pub manage_account_url: String,
    /// The HMAC that authenticates `uuid` in feedback and unsubscribe links.
    pub key: String,
}

/// One newsletter, ready to render.
pub struct NewsletterEmail<'a> {
    pub site: &'a SiteContext,
    pub newsletter: &'a Newsletter,
    pub post: &'a PostContext,
    pub latest_posts: &'a [LatestPost],
    /// `comments_enabled` is not "off" — Ghost gates the comment CTA on both
    /// this and the newsletter's own toggle.
    pub comments_enabled: bool,
}

/// Ghost's `ctaBgColors`, used to build the CTA-card colour exclusions.
const CTA_BG_COLORS: [&str; 7] = ["grey", "blue", "green", "yellow", "red", "pink", "purple"];

impl NewsletterEmail<'_> {
    /// The subject line. Ghost prefers `posts_meta.email_subject`; the caller
    /// resolves that, so this is just the title.
    pub fn subject(&self) -> String {
        self.post.title.clone()
    }

    /// Render the HTML. The result still carries `%%{…}%%` tokens — run it
    /// through [`personalize`] per recipient.
    ///
    /// Rendering once and substituting per member is how Ghost does it, and
    /// it is the difference between compiling one email and compiling one per
    /// subscriber.
    pub fn render(&self) -> Result<String> {
        let data = self.template_data();
        let html = REGISTRY
            .render("newsletter", &data)
            .map_err(|e| GhostError::Render(format!("newsletter render failed: {e}")))?;
        Ok(post_process(&html, self.design().background_is_dark))
    }

    fn design(&self) -> EmailDesign {
        email_design(&self.site.accent_color, self.newsletter)
    }

    fn template_data(&self) -> Value {
        let n = self.newsletter;
        let p = self.post;
        let design = self.design();

        let show_feature_image = n.show_feature_image && p.feature_image.is_some();
        let show_header_icon = n.show_header_icon && self.site.icon_url.is_some();
        let show_share_button = n.show_share_button && !p.email_only;
        let show_comment_cta = n.show_comment_cta && self.comments_enabled && !p.email_only;
        let has_feedback_buttons = n.feedback_enabled;

        // Ghost divides the row evenly between however many buttons are on.
        let button_count = (if has_feedback_buttons { 2 } else { 0 })
            + usize::from(show_comment_cta)
            + usize::from(show_share_button);
        let cell_width = match button_count {
            0 => Value::Null,
            k => {
                let pct = 100.0 / k as f64;
                let s = format!("{pct:.2}");
                Value::String(format!("{}%", s.trim_end_matches(".00")))
            }
        };

        let feedback_buttons = if has_feedback_buttons {
            json!({
                "likeHref": self.feedback_link(1),
                "dislikeHref": self.feedback_link(0),
            })
        } else {
            Value::Null
        };

        // The toggle is enforced here rather than trusted to the caller: a
        // newsletter with "Keep reading" off must never show it, however the
        // list was assembled.
        let latest = if n.show_latest_posts { self.latest_posts } else { &[] };
        let latest_posts: Vec<Value> = latest
            .iter()
            .map(|lp| {
                json!({
                    "title": lp.title,
                    "url": lp.url,
                    "excerpt": lp.excerpt,
                    "featureImage": lp.feature_image.as_ref().map(|src| json!({
                        "src": self.asset(src),
                        "width": 100,
                        "height": 100,
                    })),
                })
            })
            .collect();
        let latest_posts_has_images = latest.iter().any(|lp| lp.feature_image.is_some());

        let has_header_content = n.header_image.is_some()
            || show_header_icon
            || n.show_header_title
            || n.show_header_name
            || n.show_post_title_section
            || show_feature_image;

        let mut data = Map::new();
        // The design fields are spread at the top level, the way
        // `...emailDesign` does in Ghost — the stylesheet reads them there.
        if let Ok(Value::Object(m)) = serde_json::to_value(&design) {
            data.extend(m);
        }

        data.insert("emailTitle".into(), json!(p.title));
        data.insert(
            "site".into(),
            json!({
                "title": self.site.title,
                "url": self.site.url,
                "iconUrl": self.site.icon_url,
                "locale": self.site.locale,
                "direction": self.site.direction,
            }),
        );
        data.insert("preheader".into(), json!(self.preheader()));
        // Zero-width characters, so a client's preview pane shows only the
        // preheader instead of running on into the body text.
        data.insert(
            "preheaderSpacing".into(),
            json!(format!(
                "{}{} &nbsp;",
                "&#8199;&#847; ".repeat(150),
                "&shy; ".repeat(200)
            )),
        );
        data.insert("html".into(), json!(p.html));

        data.insert(
            "post".into(),
            json!({
                "title": p.title,
                "url": p.url,
                "shareUrl": if show_share_button { Value::String(format!("{}#/share", p.url)) } else { Value::Null },
                "commentUrl": format!("{}#ghost-comments-root", p.url),
                "authors": p.authors,
                "publishedAt": p.published_at,
                "customExcerpt": p.custom_excerpt,
                "feature_image": p.feature_image.as_ref().map(|i| self.asset(i)),
                "feature_image_width": p.feature_image_width,
                "feature_image_alt": p.feature_image_alt,
                "feature_image_caption": p.feature_image_caption,
            }),
        );

        data.insert(
            "newsletter".into(),
            json!({
                "name": n.name,
                "showPostTitleSection": n.show_post_title_section,
                "showExcerpt": n.show_excerpt,
                "showCommentCta": show_comment_cta,
                "showSubscriptionDetails": n.show_subscription_details,
            }),
        );

        data.insert("hasHeaderContent".into(), json!(has_header_content));
        data.insert("latestPosts".into(), json!(latest_posts));
        data.insert("latestPostsHasImages".into(), json!(latest_posts_has_images));
        data.insert("showBadge".into(), json!(n.show_badge));
        data.insert(
            "headerImage".into(),
            json!(n.header_image.as_ref().map(|i| self.asset(i))),
        );
        data.insert("headerImageWidth".into(), Value::Null);
        data.insert("showHeaderIcon".into(), json!(show_header_icon));
        data.insert("showHeaderTitle".into(), json!(n.show_header_title));
        data.insert("showHeaderName".into(), json!(n.show_header_name));
        data.insert("showFeatureImage".into(), json!(show_feature_image));
        data.insert("footerContent".into(), json!(n.footer_content));
        data.insert("ctaBgColors".into(), json!(CTA_BG_COLORS));
        data.insert("classes".into(), self.classes(show_feature_image));
        data.insert("feedbackButtons".into(), feedback_buttons);
        data.insert("feedbackButtonCellWidth".into(), cell_width);
        data.insert(
            "paywall".into(),
            if p.paywall {
                json!({ "signupUrl": format!("{}#/portal/signup", p.url) })
            } else {
                Value::Null
            },
        );
        data.insert("year".into(), json!(current_year()));

        Value::Object(data)
    }

    /// The class lists Ghost builds with `clsx`.
    fn classes(&self, show_feature_image: bool) -> Value {
        let n = self.newsletter;
        let title_serif = n.title_font_category == "serif";
        let body_sans = n.body_font_category == "sans_serif";
        let left = n.title_alignment == "left";
        let has_excerpt = self
            .post
            .custom_excerpt
            .as_deref()
            .is_some_and(|e| !e.is_empty());

        let join = |parts: &[(&str, bool)]| -> String {
            parts
                .iter()
                .filter(|(_, on)| *on)
                .map(|(c, _)| *c)
                .collect::<Vec<_>>()
                .join(" ")
        };

        json!({
            "container": join(&[("container", true), ("title-serif", title_serif)]),
            "title": join(&[
                ("post-title", true),
                ("post-title-with-excerpt", has_excerpt),
                ("post-title-no-excerpt", !has_excerpt),
                ("post-title-serif", title_serif),
                ("post-title-left", left),
            ]),
            "titleLink": join(&[("post-title-link", true), ("post-title-link-left", left)]),
            "excerpt": join(&[
                ("post-excerpt", true),
                ("post-excerpt-with-feature-image", show_feature_image),
                ("post-excerpt-no-feature-image", !show_feature_image),
                ("post-excerpt-serif-serif", title_serif && !body_sans),
                ("post-excerpt-serif-sans", title_serif && body_sans),
                ("post-excerpt-left", left),
            ]),
            "meta": join(&[
                ("post-meta", true),
                ("post-meta-left", left),
                ("post-meta-center", !left),
            ]),
            "body": join(&[
                ("post-content-sans-serif", body_sans),
                ("post-content", !body_sans),
            ]),
        })
    }

    /// Ghost's audience-feedback link shape, with the member tokens left in
    /// so [`personalize`] can fill them per recipient.
    fn feedback_link(&self, score: u8) -> String {
        format!(
            "{}/#/feedback/{}/{}/?uuid=%%{{uuid}}%%&key=%%{{key}}%%",
            self.site.url.trim_end_matches('/'),
            self.post.id,
            score
        )
    }

    /// Absolute URL for a stored asset path. An email has no page to resolve
    /// `/content/images/…` against, so a relative `src` simply never loads.
    fn asset(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            return path.to_string();
        }
        ghost_core::url::absolute(&self.site.url, &ghost_core::url::base_asset(path))
    }

    /// Ghost's preheader: the custom excerpt, else the first 500 characters
    /// of the plaintext, else the title.
    fn preheader(&self) -> String {
        if let Some(e) = self.post.custom_excerpt.as_deref().filter(|e| !e.is_empty()) {
            return e.to_string();
        }
        if !self.post.plaintext.is_empty() {
            return self.post.plaintext.chars().take(500).collect();
        }
        format!("{} – ", self.post.title)
    }
}

fn current_year() -> String {
    // `now_db()` is `YYYY-MM-DD HH:mm:ss`, so the year is the first four
    // characters. Cheaper than pulling chrono into this crate for one field.
    ghost_core::dates::now_db().chars().take(4).collect()
}

// ---------------------------------------------------------------------------
// post-processing
// ---------------------------------------------------------------------------

/// The passes Ghost runs over the rendered HTML, in Ghost's order: the
/// figcaption class has to be on before the stylesheet is inlined, and the
/// element renaming has to happen after, once the element-selector rules have
/// already been applied.
fn post_process(html: &str, background_is_dark: bool) -> String {
    let mut out = tag_figcaptions(html);
    out = inline_styles(&out);
    out = figures_to_divs(&out);
    out = drop_mismatched_theme_images(&out, background_is_dark);
    out = force_target_blank(&out);

    // Handlebars escapes `=` and a backtick inside attribute values, which
    // leaves query strings as `?uuid&#x3D;…`. Ghost's output does not have
    // these because juice and cheerio reparse the document and re-serialize
    // only what HTML actually requires; neither character ever does, so
    // undoing it matches Ghost and keeps stored URLs readable.
    out = out.replace("&#x3D;", "=").replace("&#x60;", "`");

    // Outlook renders these entities as mojibake; Ghost substitutes the
    // numeric or named forms it does understand.
    out = out.replace("&apos;", "&#39;");
    out = out.replace('\u{2192}', "&rarr;");
    out = out.replace('\u{2013}', "&ndash;");
    out = out.replace('\u{201C}', "&ldquo;");
    out = out.replace('\u{201D}', "&rdquo;");
    out
}

/// Koenig emits both a light and a dark variant of some card images and hides
/// one with CSS. Outlook ignores that CSS and shows both, so the wrong one is
/// removed outright — which is why this cannot wait for the client.
fn drop_mismatched_theme_images(html: &str, background_is_dark: bool) -> String {
    let unwanted = if background_is_dark {
        "is-light-background"
    } else {
        "is-dark-background"
    };
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<img") {
        let Some(len) = rest[start..].find('>') else {
            break;
        };
        let end = start + len + 1;
        let tag = &rest[start..end];
        out.push_str(&rest[..start]);
        if !tag.contains(unwanted) {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Every link opens in a new tab, the way Ghost forces it — a newsletter is
/// read inside a mail client, and replacing that view with the site is not
/// something a reader can navigate back from.
fn force_target_blank(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    while let Some(start) = rest.find("<a ") {
        let Some(len) = rest[start..].find('>') else {
            break;
        };
        let end = start + len + 1;
        let tag = &rest[start..end];
        out.push_str(&rest[..start]);
        if tag.contains("target=") {
            out.push_str(tag);
        } else {
            // Insert before the closing bracket, keeping a trailing `/` on a
            // self-closed tag where it is.
            let (body, close) = tag.split_at(tag.len() - 1);
            let body = body.trim_end();
            let (body, close) = match body.strip_suffix('/') {
                Some(b) => (b.trim_end(), format!("/{close}")),
                None => (body, close.to_string()),
            };
            out.push_str(body);
            out.push_str(" target=\"_blank\"");
            out.push_str(&close);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Ghost adds this class so figcaptions can be styled from the stylesheet.
fn tag_figcaptions(html: &str) -> String {
    html.replace("<figcaption>", "<figcaption class=\"kg-card-figcaption\">")
        .replace("<figcaption ", "<figcaption class=\"kg-card-figcaption\" ")
}

/// Move the stylesheet into `style` attributes, which is what Ghost's `juice`
/// pass does and what mail clients actually honour.
///
/// A head stylesheet is not a reliable way to style an email: Outlook for
/// Windows ignores one outright, and Gmail drops a `<style>` block of this
/// size — around 68KB once every Koenig card's rules are in it — which leaves
/// the message rendering as unstyled tables. The rules that cannot become an
/// attribute (media queries, `@font-face`, `@keyframes`, pseudo-elements) stay
/// behind in a much smaller `<style>`, the same split juice makes.
fn inline_styles(html: &str) -> String {
    let options = css_inline::InlineOptions::default()
        // Everything the email needs is in the document already; a remote
        // fetch here would mean a network round trip per send.
        .load_remote_stylesheets(false)
        // juice's `preserveMediaQueries`: the responsive rules are the whole
        // reason the email reflows on a phone.
        .keep_at_rules(true);

    match css_inline::CSSInliner::new(options).inline(html) {
        Ok(inlined) => restore_uninlinable_rules(&inlined, html),
        Err(e) => {
            // Inlining is an improvement, not a precondition. The uninlined
            // document is still a complete email in every client that honours
            // head styles, so a send is better than a failure here.
            tracing::warn!("newsletter: could not inline css, sending head styles as-is: {e}");
            html.to_string()
        }
    }
}

/// Pseudo-class and pseudo-element rules, which have no element to attach to
/// at inlining time and would otherwise be dropped along with the stylesheet.
///
/// juice has `inlinePseudoElements`, which materialises a `::before` as a real
/// `<span>`; there is no equivalent here, so the rules are put back into the
/// kept `<style>` instead. Clients that honour head styles — which is all of
/// them except Outlook for Windows — render them as before.
fn restore_uninlinable_rules(inlined: &str, original: &str) -> String {
    let rules = uninlinable_rules(original);
    if rules.is_empty() {
        return inlined.to_string();
    }
    let block = format!("<style>{rules}</style>");
    match inlined.find("</head>") {
        Some(at) => format!("{}{block}{}", &inlined[..at], &inlined[at..]),
        // No head to put it in: a document shaped like that did not come from
        // these templates, so leave it exactly as it is.
        None => inlined.to_string(),
    }
}

/// Pseudo selectors that cannot be resolved against a static document. A rule
/// whose selector mentions one of these is kept in the stylesheet.
const UNINLINABLE_PSEUDOS: [&str; 6] = [
    ":hover",
    ":focus",
    ":active",
    ":visited",
    ":before",
    ":after",
];

/// Collect those rules out of the document's `<style>` blocks.
///
/// At-rules are skipped: `keep_at_rules` already puts those back, and copying
/// them again would double the media queries.
fn uninlinable_rules(html: &str) -> String {
    let mut out = String::new();
    for css in style_blocks(html) {
        let css = strip_comments(css);
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let selector = rest[..open].trim();
            let Some(close) = matching_brace(&rest[open..]) else {
                break;
            };
            let end = open + close + 1;
            let is_at_rule = selector.starts_with('@') || selector.contains('}');
            if !is_at_rule
                && UNINLINABLE_PSEUDOS
                    .iter()
                    .any(|p| selector.contains(p))
            {
                out.push_str(selector);
                out.push_str(&rest[open..end]);
            }
            rest = &rest[end..];
        }
    }
    out
}

/// The text inside each `<style>` element.
fn style_blocks(html: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<style") {
        let Some(open) = rest[start..].find('>') else {
            break;
        };
        let body = start + open + 1;
        let Some(len) = rest[body..].find("</style>") else {
            break;
        };
        blocks.push(&rest[body..body + len]);
        rest = &rest[body + len..];
    }
    blocks
}

/// The offset of the `}` closing the `{` this slice starts with, counting
/// nested blocks so an at-rule's body is passed over whole.
fn matching_brace(css: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in css.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `/* … */`, removed so a comment's braces cannot be read as a rule. These
/// stylesheets have no `content: "/*"` to be confused by.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(len) => rest = &rest[start + len + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Ghost renames these once the styling is inline, because Outlook applies
/// margins to a `div` and not to a `figure`. Doing it any earlier would mean
/// the `figure`/`figcaption` element selectors never matched.
fn figures_to_divs(html: &str) -> String {
    html.replace("<figcaption", "<div")
        .replace("</figcaption>", "</div>")
        .replace("<figure", "<div")
        .replace("</figure>", "</div>")
}

// ---------------------------------------------------------------------------
// per-recipient substitution
// ---------------------------------------------------------------------------

/// Ghost's subscription-status sentence, keyed off the member's status.
fn status_text(status: &str) -> &'static str {
    match status {
        "comped" => "Your subscription will expire on the date shown in your account.",
        "paid" => "Your subscription will renew on the date shown in your account.",
        _ => "",
    }
}

/// Fill in the `%%{…}%%` tokens for one member.
///
/// Ghost supports a default in the token — `%%{name, "not provided"}%%` — and
/// the subscription box relies on it, so that form is handled too.
pub fn personalize(html: &str, r: &Recipient) -> String {
    let name = r.name.as_deref().unwrap_or("").trim().to_string();
    let mut out = html.to_string();

    // The name row is removed outright rather than hidden with a class.
    // Ghost does the same, and for the same reason: the class is only known
    // per member, which is after the stylesheet has been inlined, so there is
    // no `.hidden` rule left for it to match.
    if name.is_empty() {
        out = drop_name_row(&out);
    }

    // The defaulted form first: it also matches the bare `%%{name}%%`
    // pattern's prefix, so doing it second would leave the literal behind.
    out = replace_defaulted(&out, "name", &name);

    for (token, value) in [
        ("uuid", r.uuid.as_str()),
        ("key", r.key.as_str()),
        ("email", r.email.as_str()),
        ("name", name.as_str()),
        ("status", r.status.as_str()),
        ("status_text", status_text(&r.status)),
        ("created_at", r.created_at.as_str()),
        ("unsubscribe_url", r.unsubscribe_url.as_str()),
        ("manage_account_url", r.manage_account_url.as_str()),
        // Ghost hides the name row entirely when there is no name.
        (
            "name_class",
            if name.is_empty() { "hidden" } else { "" },
        ),
    ] {
        out = out.replace(&format!("%%{{{token}}}%%"), value);
    }
    out
}

/// Remove the `<p>` carrying the `%%{name_class}%%` marker, label and all.
///
/// The marker is only ever on that one element, in the subscription box; a
/// text part has no tags for it to be inside, which is what the missing `<p`
/// means — there the token substitution falls back to "not provided".
fn drop_name_row(html: &str) -> String {
    const MARKER: &str = "%%{name_class}%%";
    let Some(at) = html.find(MARKER) else {
        return html.to_string();
    };
    let Some(start) = html[..at].rfind("<p") else {
        return html.to_string();
    };
    let Some(len) = html[at..].find("</p>") else {
        return html.to_string();
    };
    let mut out = String::with_capacity(html.len());
    out.push_str(&html[..start]);
    out.push_str(&html[at + len + "</p>".len()..]);
    out
}

/// `%%{token, "fallback"}%%` — used when the member has no value for it.
fn replace_defaulted(html: &str, token: &str, value: &str) -> String {
    let open = format!("%%{{{token}, ");
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(&open) {
        let Some(len) = rest[start..].find("}%%") else {
            break;
        };
        let end = start + len + 3;
        out.push_str(&rest[..start]);
        if value.is_empty() {
            // The fallback is a quoted literal inside the token.
            let inner = &rest[start + open.len()..end - 3];
            out.push_str(inner.trim().trim_matches('"'));
        } else {
            out.push_str(value);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Tags whose content never belongs in a text part.
const DROPPED_ELEMENTS: [&str; 4] = ["head", "style", "script", "title"];

/// Tags that end a line. Anything not listed is treated as inline, so a
/// `<strong>` inside a sentence does not break it.
const BLOCK_ELEMENTS: [&str; 22] = [
    "p", "div", "br", "tr", "table", "h1", "h2", "h3", "h4", "h5", "h6", "li", "ul", "ol",
    "blockquote", "figure", "figcaption", "hr", "section", "header", "footer", "pre",
];

/// The plain-text alternative, derived from the rendered email.
///
/// Ghost runs the finished HTML through `@tryghost/html-to-plaintext`, whose
/// email preset keeps link targets and block structure. The three things that
/// make the difference between a usable text part and a wall of noise, and
/// which a generic tag-stripper gets wrong, are handled explicitly:
///
/// * `<head>`, `<style>` and the `<!--[if mso]>` conditionals are dropped, or
///   the part opens with a stylesheet and the string "96".
/// * the preheader and its ~2KB of zero-width spacing characters are dropped;
///   they exist only to control a preview pane.
/// * a link becomes `text [url]`, because a text-only reader otherwise has no
///   way to follow it.
pub fn plaintext(html: &str) -> String {
    // Start at the body: everything above it is metadata.
    let source = match html.find("<body") {
        Some(i) => &html[i..],
        None => html,
    };

    let mut out = String::with_capacity(source.len() / 4);
    let mut rest = source;
    // Where the current anchor's text began, and what it points at.
    let mut anchor: Option<(usize, String)> = None;

    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];

        // Comments, including Outlook's conditional blocks.
        if let Some(body) = after.strip_prefix("<!--") {
            rest = match body.find("-->") {
                Some(i) => &body[i + 3..],
                None => "",
            };
            continue;
        }

        let Some(close) = after.find('>') else { break };
        let tag = &after[1..close];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let name = name.to_ascii_lowercase();
        rest = &after[close + 1..];

        if !closing && (DROPPED_ELEMENTS.contains(&name.as_str()) || tag.contains("class=\"preheader"))
        {
            // Skip to this element's own closing tag. None of these nest, so
            // the first match is the right one.
            let end_tag = format!("</{name}");
            rest = match rest.find(&end_tag) {
                Some(i) => &rest[i..],
                None => "",
            };
            continue;
        }

        match (name.as_str(), closing) {
            ("a", false) => {
                let href = attr(tag, "href").unwrap_or_default();
                anchor = Some((out.len(), href));
            }
            ("a", true) => {
                if let Some((text_start, href)) = anchor.take() {
                    let text = out[text_start..].trim().to_string();
                    // A bare fragment, a replacement token or a link whose
                    // text already *is* the URL adds nothing in brackets.
                    let worth_showing = !href.is_empty()
                        && !href.starts_with('#')
                        && !href.starts_with("%%{")
                        && href != text;
                    if worth_showing {
                        out.push_str(&format!(" [{href}]"));
                    }
                }
            }
            _ => {}
        }

        if BLOCK_ELEMENTS.contains(&name.as_str()) {
            out.push('\n');
        }
    }
    out.push_str(rest);

    tidy_text(&decode_entities(&out))
}

/// The value of one attribute, for the few places this needs one.
fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let len = tag[at..].find('"')?;
    Some(tag[at..at + len].to_string())
}

fn decode_entities(s: &str) -> String {
    let mut out = s.to_string();
    for (entity, replacement) in [
        ("&shy;", ""),
        ("&#847;", ""),
        ("&#8199;", " "),
        ("&nbsp;", " "),
        ("&copy;", "©"),
        ("&ndash;", "–"),
        ("&rarr;", "→"),
        ("&ldquo;", "\u{201C}"),
        ("&rdquo;", "\u{201D}"),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&#x3D;", "="),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        // Last, so a literal "&amp;lt;" does not become "<".
        ("&amp;", "&"),
    ] {
        out = out.replace(entity, replacement);
    }
    out
}

/// Collapse runs of spaces within a line and runs of blank lines between
/// them, so the table-based layout does not come through as whitespace.
fn tidy_text(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in s.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        // At most one blank line in a row.
        if line.is_empty() && lines.last().is_some_and(|l: &String| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn newsletter() -> Newsletter {
        Newsletter {
            id: "n1".into(),
            uuid: None,
            name: "Weekly".into(),
            description: None,
            slug: "weekly".into(),
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
            show_excerpt: false,
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
            show_latest_posts: false,
            show_share_button: false,
            feedback_enabled: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn yiq_threshold_matches_ghosts_colour_utils() {
        // 186 is the cut, and these two straddle it
        assert_eq!(text_color_for_background("#ffffff"), "#000000");
        assert_eq!(text_color_for_background("#15212A"), "#FFFFFF");
        // three-digit hex is accepted, like Ghost's regex allows
        assert_eq!(text_color_for_background("#fff"), "#000000");
        assert!(is_dark("#000000"));
        assert!(!is_dark("#ffffff"));
    }

    #[test]
    fn column_sentinels_are_not_mistaken_for_colours() {
        let n = newsletter();
        let d = email_design("#FF1A75", &n);
        // 'light' is not a hex value, so the background is white
        assert_eq!(d.background_color, "#ffffff");
        // 'transparent' means "no header background", not a colour
        assert_eq!(d.header_background_color, None);
        // 'accent' resolves to the site accent for links and buttons
        assert_eq!(d.link_color, "#FF1A75");
        assert_eq!(d.button_color, "#FF1A75");
        // and the default divider is Ghost's own grey, not the accent
        assert_eq!(d.divider_color, "#e0e7eb");
    }

    #[test]
    fn a_null_button_colour_means_contrast_not_accent() {
        // Ghost distinguishes SQL NULL from any other value here: NULL asks
        // for whatever reads on the background, not the accent colour.
        let mut n = newsletter();
        n.button_color = None;
        n.link_color = None;
        let d = email_design("#FF1A75", &n);
        assert_eq!(d.button_color, "#000000");
        assert_eq!(d.link_color, "#000000");
    }

    #[test]
    fn dark_background_flips_the_derived_colours() {
        let mut n = newsletter();
        n.background_color = "#15212A".into();
        let d = email_design("#FF1A75", &n);
        assert!(d.background_is_dark);
        assert_eq!(d.text_color, "#FFFFFF");
        // no explicit post title colour, so it contrasts the background
        assert_eq!(d.post_title_color, "#FFFFFF");
    }

    #[test]
    fn button_corners_and_style_map_to_css() {
        let mut n = newsletter();
        n.button_corners = "pill".into();
        n.button_style = "outline".into();
        n.image_corners = "rounded".into();
        let d = email_design("#FF1A75", &n);
        assert_eq!(d.button_border_radius, "9999px");
        assert!(d.has_outline_buttons);
        assert!(d.has_rounded_image_corners);

        n.button_corners = "square".into();
        assert_eq!(email_design("#FF1A75", &n).button_border_radius, "0");
    }

    #[test]
    fn title_weight_drives_the_strong_weight() {
        let mut n = newsletter();
        n.title_font_weight = "normal".into();
        let d = email_design("#FF1A75", &n);
        assert_eq!(d.title_weight, "400");
        // a bold run inside a lighter heading still has to look bolder
        assert_eq!(d.title_strong_weight, "700");

        n.title_font_weight = "bold".into();
        let d = email_design("#FF1A75", &n);
        assert_eq!(d.title_weight, "700");
        assert_eq!(d.title_strong_weight, "800");
    }

    #[test]
    fn templates_compile_and_render_a_whole_email() {
        let site = SiteContext {
            title: "Acme".into(),
            url: "https://acme.test".into(),
            icon_url: Some("https://acme.test/favicon.png".into()),
            locale: "en".into(),
            direction: "ltr".into(),
            accent_color: "#FF1A75".into(),
        };
        let post = PostContext {
            id: "p1".into(),
            title: "Hello world".into(),
            url: "https://acme.test/hello/".into(),
            html: "<p>Body text</p>".into(),
            plaintext: "Body text".into(),
            published_at: "20 Oct 2026".into(),
            authors: "Ada".into(),
            ..Default::default()
        };
        let mut n = newsletter();
        n.show_subscription_details = true;
        let email = NewsletterEmail {
            site: &site,
            newsletter: &n,
            post: &post,
            latest_posts: &[],
            comments_enabled: true,
        };
        let html = email.render().expect("renders");

        // the wrapper, the stylesheet and the body all made it in. The
        // doctype is upper-cased by the serializer the inliner reparses
        // through, as it is in Ghost's own output.
        assert!(html.starts_with("<!DOCTYPE html>"), "{}", &html[..60]);
        assert!(html.contains("Body text"));
        assert!(html.contains("Hello world"));
        // the site accent reached the stylesheet, and from there the markup
        assert!(html.contains("#FF1A75"));
        // the `t` helper interpolated its named argument
        assert!(html.contains("By Ada"), "the t helper did not interpolate {{authors}}");
        // tokens survive rendering, for personalize() to fill in
        assert!(html.contains("%%{unsubscribe_url}%%"));
        assert!(html.contains("%%{status}%%"));
        // query strings are not left half-escaped by the templating
        assert!(!html.contains("&#x3D;"), "an escaped = leaked into a URL");
        // links are forced to open in a new tab
        assert!(html.contains("target=\"_blank\""));
    }

    /// The failure this guards against is invisible in a browser and total in
    /// Gmail: Gmail discards a `<style>` block this size, so anything still
    /// relying on one arrives as unstyled tables.
    #[test]
    fn the_stylesheet_is_inlined_rather_than_left_in_the_head() {
        let site = SiteContext {
            title: "Acme".into(),
            url: "https://acme.test".into(),
            icon_url: None,
            locale: "en".into(),
            direction: "ltr".into(),
            accent_color: "#FF1A75".into(),
        };
        let post = PostContext {
            id: "p1".into(),
            title: "Hello world".into(),
            url: "https://acme.test/hello/".into(),
            html: "<p>Body text</p>".into(),
            plaintext: "Body text".into(),
            published_at: "20 Oct 2026".into(),
            authors: "Ada".into(),
            ..Default::default()
        };
        let n = newsletter();
        let html = NewsletterEmail {
            site: &site,
            newsletter: &n,
            post: &post,
            latest_posts: &[],
            comments_enabled: true,
        }
        .render()
        .expect("renders");

        // the preheader is hidden by an attribute, not by a class — this is
        // the one that shows up as a wall of body text at the top of Gmail
        let preheader = html
            .find("class=\"preheader\"")
            .expect("the preheader is still there");
        let tag = &html[preheader..preheader + 300];
        assert!(tag.contains("display: none"), "preheader not inlined: {tag}");

        // the post title carries its own colour rather than rendering as a
        // default blue link
        assert!(html.contains("class=\"post-title-link"));
        assert!(html.contains("color: #15212A"), "the title link was not inlined");

        // what is left in <head> is only what cannot be an attribute
        let kept: String = style_blocks(&html).concat();
        assert!(kept.contains("@media"), "the responsive rules were dropped");
        assert!(
            !kept.contains("border-collapse"),
            "an inlinable rule was left in the stylesheet"
        );
        // and that leftover is small enough for Gmail to keep
        assert!(kept.len() < 30_000, "kept stylesheet is {} bytes", kept.len());
    }

    #[test]
    fn personalize_fills_every_token() {
        let r = Recipient {
            uuid: "u-1".into(),
            email: "a@b.c".into(),
            name: Some("Ada".into()),
            status: "paid".into(),
            created_at: "1 Jan 2026".into(),
            unsubscribe_url: "https://acme.test/unsub".into(),
            manage_account_url: "https://acme.test/#/portal/account".into(),
            key: "deadbeef".into(),
        };
        let html = "%%{name}%% %%{email}%% %%{status}%% %%{uuid}%% %%{key}%% \
                    %%{created_at}%% %%{unsubscribe_url}%% %%{manage_account_url}%% \
                    %%{name, \"not provided\"}%% [%%{name_class}%%]";
        let out = personalize(html, &r);
        assert!(!out.contains("%%{"), "unsubstituted token left in {out}");
        assert!(out.contains("a@b.c"));
        assert!(out.contains("deadbeef"));
        assert!(out.contains("[]"), "a named member has no hidden class");
    }

    #[test]
    fn a_nameless_member_gets_the_tokens_fallback() {
        let r = Recipient {
            uuid: "u-1".into(),
            email: "a@b.c".into(),
            name: None,
            status: "free".into(),
            created_at: "1 Jan 2026".into(),
            unsubscribe_url: "u".into(),
            manage_account_url: "m".into(),
            key: "k".into(),
        };
        let out = personalize("Name: %%{name, \"not provided\"}%% [%%{name_class}%%]", &r);
        assert!(out.contains("not provided"), "{out}");
        // and the row is marked hidden so it does not render at all
        assert!(out.contains("[hidden]"), "{out}");

        // in the HTML part the row is removed outright: there is no
        // stylesheet left to act on the class by the time it is set
        let row = r#"<td><p class="%%{name_class}%%" style="margin: 0;">Name: %%{name, "not provided"}%%</p><p>Email: x</p></td>"#;
        let out = personalize(row, &r);
        assert!(!out.contains("Name:"), "the nameless row survived: {out}");
        assert!(out.contains("Email: x"), "the rest of the box went with it: {out}");
    }

    #[test]
    fn the_wrong_theme_variant_of_an_image_is_removed() {
        let html = r#"<p><img src="a.png" class="is-light-background"><img src="b.png" class="is-dark-background"></p>"#;
        let light = drop_mismatched_theme_images(html, false);
        assert!(light.contains("a.png"));
        assert!(!light.contains("b.png"));

        let dark = drop_mismatched_theme_images(html, true);
        assert!(!dark.contains("a.png"));
        assert!(dark.contains("b.png"));
    }

    #[test]
    fn the_text_part_drops_everything_not_meant_to_be_read() {
        let site = SiteContext {
            title: "Acme".into(),
            url: "https://acme.test".into(),
            icon_url: None,
            locale: "en".into(),
            direction: "ltr".into(),
            accent_color: "#FF1A75".into(),
        };
        let post = PostContext {
            id: "p1".into(),
            title: "Hello world".into(),
            url: "https://acme.test/hello/".into(),
            html: "<p>Read the <a href=\"https://acme.test/docs/\">docs</a>.</p>".into(),
            plaintext: "Read the docs.".into(),
            custom_excerpt: Some("A short dispatch.".into()),
            published_at: "20 Oct 2026".into(),
            authors: "Ada".into(),
            ..Default::default()
        };
        let n = newsletter();
        let html = NewsletterEmail {
            site: &site,
            newsletter: &n,
            post: &post,
            latest_posts: &[],
            comments_enabled: true,
        }
        .render()
        .expect("renders");
        let text = plaintext(&html);

        assert!(text.contains("Hello world"));
        // the body came through, with the link's target alongside its text —
        // a text-only reader otherwise has no way to follow it
        assert!(text.contains("Read the docs [https://acme.test/docs/]."), "{text}");
        // the stylesheet is not content
        assert!(!text.contains("border-collapse"), "{text}");
        // nor is Outlook's PixelsPerInch conditional, which leaks as "96"
        assert!(!text.starts_with("96"), "{text}");
        // nor the ~2KB of zero-width characters that pad the preview pane
        assert!(!text.contains("&#8199;") && !text.contains("&shy;"), "{text}");
        // entities that would otherwise show up raw
        assert!(text.contains('©'), "{text}");
        assert!(!text.contains("&copy;"), "{text}");
        // and it is laid out in lines, not collapsed onto one
        assert!(text.lines().count() > 3, "{text}");
    }

    #[test]
    fn target_blank_is_added_once_and_only_where_missing() {
        let out = force_target_blank(r#"<a href="x">a</a><a href="y" target="_self">b</a>"#);
        assert_eq!(
            out,
            r#"<a href="x" target="_blank">a</a><a href="y" target="_self">b</a>"#
        );
    }
}
