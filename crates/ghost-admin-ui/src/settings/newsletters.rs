//! Settings -> Newsletters: each newsletter's own design and sending, and the site-wide defaults.

use super::*;

// --- newsletters: the newsletters themselves, and the Mailgun transport
//     that carries them. Ghost keeps both on this one settings screen. ---

/// Redirect back to one newsletter's editor with a one-shot banner.
pub(crate) fn newsletter_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/newsletters/{id}/?{encoded}")).into_response()
}

/// A colour setting, as a mode `<select>` plus a swatch for a custom value.
///
/// Every colour column in `newsletters` holds *either* a sentinel — `accent`,
/// `light`, `transparent` — *or* a literal `#rrggbb`, so one control cannot
/// express it. The select picks the meaning; the swatch only matters when the
/// meaning is "a specific colour".
///
/// `modes` are `(stored value, label)` pairs, where an empty stored value
/// means the column's "unset" state (NULL, or the column default).
/// A colour setting as Ghost-style swatch chips: one per named mode plus a
/// custom picker. Posts `{key}_mode` and `{key}_custom`, which
/// [`colour_from_form`] folds back into the stored value.
///
/// Each mode carries the colour its chip should show — `""` for a mode with
/// no single colour (Automatic), `"none"` for transparent, else a hex.
pub(crate) fn colour_row(
    key: &str,
    label: &str,
    detail: &str,
    modes: &[(&str, &str, &str)],
    stored: Option<&str>,
    default_mode: &str,
) -> String {
    let custom = stored.filter(|v| ghost_mail::newsletter::is_hex_color(v));
    let mode = match custom {
        Some(_) => "custom",
        None => stored
            .map(|v| v.trim())
            .filter(|v| modes.iter().any(|(m, _, _)| m == v))
            .unwrap_or(default_mode),
    };

    let chips: String = modes
        .iter()
        .map(|(value, name, swatch)| {
            let dot = match *swatch {
                "" => String::new(),
                "none" => r#"<span class="dot none"></span>"#.to_string(),
                hex => format!(r#"<span class="dot" style="background:{hex}"></span>"#),
            };
            format!(
                r#"<label class="swatch"><input type="radio" name="{key}_mode" value="{value}"{ck}>{dot}<span>{name}</span></label>"#,
                key = esc(key),
                value = esc(value),
                ck = if *value == mode { " checked" } else { "" },
                name = esc(name),
            )
        })
        .collect();

    // A swatch has to be a valid hex for the colour input to accept it, so an
    // unset value falls back to Ghost's own dark ink.
    let custom_swatch = custom.unwrap_or("#15212a");

    format!(
        r#"<div class="row"><span class="ic tint-pink">{palette}</span>
<div class="body"><span class="title">{label}</span><span class="detail">{detail}</span>
<div class="swatches under">{chips}
<label class="swatch custom"><input type="radio" name="{key}_mode" value="custom"{ck_custom}><input type="color" name="{key}_custom" value="{custom_swatch}" aria-label="{label} custom colour"><span>Custom</span></label>
</div></div></div>"#,
        palette = svg("themes"),
        label = esc(label),
        detail = esc(detail),
        chips = chips,
        key = esc(key),
        ck_custom = if mode == "custom" { " checked" } else { "" },
        custom_swatch = esc(custom_swatch),
    )
}

/// Collapse a `colour_row`'s two inputs back into the single stored value.
pub(crate) fn colour_from_form(form: &HashMap<String, String>, key: &str) -> Option<String> {
    let mode = form.get(&format!("{key}_mode")).map(String::as_str).unwrap_or("");
    match mode {
        "custom" => form
            .get(&format!("{key}_custom"))
            .map(|v| v.trim().to_lowercase())
            .filter(|v| ghost_mail::newsletter::is_hex_color(v)),
        // The "unset" option, which the nullable colour columns store as NULL
        // and the renderer reads as "derive it".
        "" => None,
        m => Some(m.to_string()),
    }
}

/// The per-newsletter design editor. Every setting Ghost exposes for a
/// newsletter is here, grouped the way Ghost groups them.
pub(crate) async fn newsletter_edit_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(n)) = repos::newsletters::find_by_id(&state.db, &id).await else {
        return settings_flash("newsletters", "error", "That newsletter no longer exists.");
    };
    let subscribers = repos::newsletters::subscriber_count(&state.db, &n.id).await;

    // The site accent colour, for the swatch chips that preview it. Ghost's
    // own fallback when a site never set one is the dark ink.
    let accent = state
        .settings
        .get_str("accent_color")
        .filter(|c| ghost_mail::newsletter::is_hex_color(c))
        .unwrap_or_else(|| "#15212A".into());
    let accent_chip: (&str, &str, &str) = ("accent", "Accent", &accent);
    let contrast_chip: (&str, &str, &str) = ("", "Contrast", "");

    // A settings row: leading tinted icon, title over a supporting line, and
    // the control in the trailing slot — the Material list item the whole
    // editor is built from.
    let setting = |tint: &str, icon: &str, title: &str, hint: &str, control: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{hint}</span></div>
<div class="trailing">{control}</div></div>"#,
            tint = tint,
            ic = svg(icon),
            title = esc(title),
            hint = esc(hint),
            control = control,
        )
    };

    let body = format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/newsletters/" title="Back to newsletters">{back}</a>
<div><h1>{name}</h1><p class="sub">{subscribers} subscriber{plural} · {status}</p></div></header>
{banner}
<form method="post" action="/ghost/settings/newsletters/{id}/">
<div class="settings">

<div class="section">Details</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{title_ic}</span>
<div class="body" style="width:100%">{name_field}{desc_field}</div></div>
{subscribe_on_signup}
{visibility_row}
{status_row}
</div>

<div class="section">Sender</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{send_ic}</span>
<div class="body" style="width:100%"><div class="grid2">{sender_name}{sender_email}</div>
<p class="muted" style="margin:8px 0 0">Leave the name empty to send as the site title, and the address empty to use the one from <a href="/ghost/settings/email/">Settings → Email</a>.</p></div></div>
{reply_to}
</div>

<div class="section">Header</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{image_ic}</span>
<div class="body" style="width:100%">{header_image}
<p class="muted" style="margin:8px 0 0">A banner across the top of every email. Use a full URL or an uploaded image path.</p></div></div>
{show_header_icon}
{show_header_title}
{show_header_name}
{header_background}
</div>

<div class="section">Post title</div>
<div class="card rows rise" style="--i:4">
{show_post_title_section}
{title_font}
{title_weight}
{title_alignment}
{post_title_colour}
{show_excerpt}
{show_feature_image}
</div>

<div class="section">Body</div>
<div class="card rows rise" style="--i:5">
{body_font}
{background_colour}
{section_title_colour}
{divider_colour}
{image_corners}
</div>

<div class="section">Links &amp; buttons</div>
<div class="card rows rise" style="--i:6">
{link_colour}
{link_style}
{button_colour}
{button_style}
{button_corners}
</div>

<div class="section">Footer</div>
<div class="card rows rise" style="--i:7">
<div class="row" style="align-items:flex-start"><span class="ic tint-slate">{notes_ic}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="footer_content" rows="3" placeholder=" ">{footer_content}</textarea><span>Footer · HTML, shown above the unsubscribe line</span></label></div></div>
{show_badge}
{show_latest_posts}
{show_comment_cta}
{show_share_button}
{feedback_enabled}
{show_subscription_details}
</div>

</div>
<div style="margin:18px 4px 8px"><button class="btn" type="submit">Save newsletter</button></div>
</form>

<div class="section">Preview</div>
<div class="card padded rise" style="--i:8">
<p class="muted" style="margin:0 0 12px">Rendered with the saved settings, using your most recent post. Save to see a change here.</p>
<iframe src="/ghost/settings/newsletters/{id}/preview/" title="Newsletter preview" style="width:100%;height:760px;border:1px solid var(--outline-variant);border-radius:16px;background:#fff"></iframe>
</div>"#,
        back = svg("chevron"),
        name = esc(&n.name),
        subscribers = subscribers,
        plural = if subscribers == 1 { "" } else { "s" },
        status = esc(&n.status),
        banner = flash_banner(&q),
        id = esc(&n.id),
        title_ic = svg("title"),
        send_ic = svg("send"),
        image_ic = svg("upload"),
        notes_ic = svg("notes"),
        name_field = text_field("name", "Newsletter name", &n.name),
        desc_field = format!(
            r#"<label class="field" style="margin:12px 0 0"><textarea name="description" rows="2" placeholder=" ">{}</textarea><span>Description · shown in the signup portal</span></label>"#,
            esc(n.description.as_deref().unwrap_or(""))
        ),
        subscribe_on_signup = toggle_row(
            "subscribe_on_signup",
            "Subscribe new members",
            "New members are added to this newsletter when they sign up",
            n.subscribe_on_signup
        ),
        // Ghost's newsletter `visibility`: who may subscribe to it at all.
        visibility_row = setting(
            "purple",
            "ghost",
            "Visibility",
            "Who is offered this newsletter in the signup portal",
            &segmented(
                "visibility",
                &[("members", "All members"), ("paid", "Paid-members only")],
                &n.visibility,
            ),
        ),
        status_row = setting(
            "purple",
            "flag",
            "Status",
            "Archived newsletters are hidden from the signup portal",
            &segmented("status", &[("active", "Active"), ("archived", "Archived")], &n.status),
        ),
        sender_name = text_field("sender_name", "Sender name", n.sender_name.as_deref().unwrap_or("")),
        sender_email = text_field("sender_email", "Sender email", n.sender_email.as_deref().unwrap_or("")),
        reply_to = setting(
            "teal",
            "mail",
            "Replies go to",
            "The address readers' replies are delivered to",
            &segmented(
                "sender_reply_to",
                &[("newsletter", "Newsletter address"), ("support", "Support address")],
                &n.sender_reply_to,
            ),
        ),
        header_image = text_field("header_image", "Header image URL", n.header_image.as_deref().unwrap_or("")),
        show_header_icon = toggle_row(
            "show_header_icon",
            "Publication icon",
            "Show the site icon at the top of the email",
            n.show_header_icon
        ),
        show_header_title = toggle_row(
            "show_header_title",
            "Publication title",
            "Show the site title at the top of the email",
            n.show_header_title
        ),
        show_header_name = toggle_row(
            "show_header_name",
            "Newsletter name",
            "Show this newsletter's name under the title",
            n.show_header_name
        ),
        header_background = colour_row(
            "header_background_color",
            "Header background",
            "A colour behind the header and post title.",
            &[
                ("transparent", "Transparent", "none"),
                ("accent", "Accent", &accent),
            ],
            Some(&n.header_background_color),
            "transparent",
        ),
        show_post_title_section = toggle_row(
            "show_post_title_section",
            "Post title",
            "Show the title, byline and date above the content",
            n.show_post_title_section
        ),
        title_font = setting(
            "amber",
            "title",
            "Title font",
            "The typeface of the post title",
            &segmented(
                "title_font_category",
                &[("sans_serif", "Sans-serif"), ("serif", "Serif")],
                &n.title_font_category,
            ),
        ),
        title_weight = setting(
            "amber",
            "title",
            "Title weight",
            "How heavy the post title's strokes are",
            &selbox(
                "title_font_weight",
                "Title weight",
                &[
                    ("normal", "Normal"),
                    ("medium", "Medium"),
                    ("semibold", "Semibold"),
                    ("bold", "Bold"),
                ],
                &n.title_font_weight,
            ),
        ),
        title_alignment = setting(
            "amber",
            "title",
            "Title alignment",
            "Centered under the header, or to the left",
            &segmented("title_alignment", &[("center", "Centered"), ("left", "Left")], &n.title_alignment),
        ),
        post_title_colour = colour_row(
            "post_title_color",
            "Post title colour",
            "Automatic picks black or white, whichever reads on the background.",
            &[("", "Automatic", ""), accent_chip],
            n.post_title_color.as_deref(),
            "",
        ),
        show_excerpt = toggle_row(
            "show_excerpt",
            "Excerpt",
            "Show the post's custom excerpt under the title",
            n.show_excerpt
        ),
        show_feature_image = toggle_row(
            "show_feature_image",
            "Feature image",
            "Show the post's feature image",
            n.show_feature_image
        ),
        body_font = setting(
            "green",
            "notes",
            "Body font",
            "The typeface of the post content",
            &segmented(
                "body_font_category",
                &[("sans_serif", "Sans-serif"), ("serif", "Serif")],
                &n.body_font_category,
            ),
        ),
        background_colour = colour_row(
            "background_color",
            "Background",
            "Light is white. A dark colour flips the text and borders to suit it.",
            &[("light", "Light", "#ffffff")],
            Some(&n.background_color),
            "light",
        ),
        section_title_colour = colour_row(
            "section_title_color",
            "Heading colour",
            "Applies to headings inside the post content.",
            &[("", "Automatic", ""), accent_chip],
            n.section_title_color.as_deref(),
            "",
        ),
        divider_colour = colour_row(
            "divider_color",
            "Divider colour",
            "Horizontal rules and the lines between sections.",
            &[("", "Default grey", "#e0e7eb"), accent_chip],
            n.divider_color.as_deref(),
            "",
        ),
        image_corners = setting(
            "green",
            "upload",
            "Image corners",
            "The corners of images inside the post",
            &segmented("image_corners", &[("square", "Square"), ("rounded", "Rounded")], &n.image_corners),
        ),
        link_colour = colour_row(
            "link_color",
            "Link colour",
            "Contrast uses black or white, whichever reads on the background.",
            &[accent_chip, contrast_chip],
            n.link_color.as_deref(),
            "accent",
        ),
        link_style = setting(
            "pink",
            "link",
            "Link style",
            "How links stand out from the surrounding text",
            &segmented(
                "link_style",
                &[("underline", "Underlined"), ("regular", "Regular"), ("bold", "Bold")],
                &n.link_style,
            ),
        ),
        button_colour = colour_row(
            "button_color",
            "Button colour",
            "Used by button and CTA cards in the post content.",
            &[accent_chip, contrast_chip],
            n.button_color.as_deref(),
            "accent",
        ),
        button_style = setting(
            "pink",
            "themes",
            "Button style",
            "Filled with the button colour, or outlined by it",
            &segmented("button_style", &[("fill", "Filled"), ("outline", "Outlined")], &n.button_style),
        ),
        button_corners = setting(
            "pink",
            "themes",
            "Button corners",
            "The corners of buttons in the post content",
            &segmented(
                "button_corners",
                &[("square", "Square"), ("rounded", "Rounded"), ("pill", "Pill")],
                &n.button_corners,
            ),
        ),
        footer_content = esc(n.footer_content.as_deref().unwrap_or("")),
        show_badge = toggle_row(
            "show_badge",
            "Powered by Ghost badge",
            "Show the badge at the very bottom of the email",
            n.show_badge
        ),
        show_latest_posts = toggle_row(
            "show_latest_posts",
            "Latest posts",
            "Add a \"Keep reading\" list of the three most recent posts",
            n.show_latest_posts
        ),
        show_comment_cta = toggle_row(
            "show_comment_cta",
            "Comments link",
            "Invite readers to comment on the post",
            n.show_comment_cta
        ),
        show_share_button = toggle_row(
            "show_share_button",
            "Share button",
            "Let readers share the post",
            n.show_share_button
        ),
        feedback_enabled = toggle_row(
            "feedback_enabled",
            "Audience feedback",
            "Add \"More like this\" and \"Less like this\" buttons",
            n.feedback_enabled
        ),
        show_subscription_details = toggle_row(
            "show_subscription_details",
            "Subscription details",
            "Print each member's own subscription details in the footer",
            n.show_subscription_details
        ),
    );

    page_for_role(actor_role, &n.name, "settings", Some(&user), &body, true).into_response()
}

pub(crate) async fn newsletter_edit_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(mut n)) = repos::newsletters::find_by_id(&state.db, &id).await else {
        return settings_flash("newsletters", "error", "That newsletter no longer exists.");
    };

    let get = |k: &str| form.get(k).cloned().unwrap_or_default().trim().to_string();
    let keep = |k: &str| Some(get(k)).filter(|v| !v.is_empty());
    let on = |k: &str| form.get(k).map(|v| v == "1").unwrap_or(false);
    // An unrecognised value is never written: these columns are enums in
    // Ghost, and the renderer's `match` arms assume it.
    let one_of = |k: &str, allowed: &[&str], fallback: &str| {
        let v = get(k);
        if allowed.contains(&v.as_str()) { v } else { fallback.to_string() }
    };

    let name = get("name");
    if name.is_empty() {
        return newsletter_flash(&id, "error", "A newsletter needs a name.");
    }
    n.name = name;
    n.description = keep("description");
    n.sender_name = keep("sender_name");
    n.sender_email = keep("sender_email");
    n.sender_reply_to = one_of("sender_reply_to", &["newsletter", "support"], "newsletter");
    n.status = one_of("status", &["active", "archived"], "active");
    n.visibility = one_of("visibility", &["members", "paid"], "members");
    n.subscribe_on_signup = on("subscribe_on_signup");

    n.header_image = keep("header_image");
    n.show_header_icon = on("show_header_icon");
    n.show_header_title = on("show_header_title");
    n.show_header_name = on("show_header_name");
    n.show_post_title_section = on("show_post_title_section");
    n.show_excerpt = on("show_excerpt");
    n.show_feature_image = on("show_feature_image");

    n.title_font_category = one_of("title_font_category", &["serif", "sans_serif"], "sans_serif");
    n.title_alignment = one_of("title_alignment", &["center", "left"], "center");
    n.title_font_weight = one_of(
        "title_font_weight",
        &["normal", "medium", "semibold", "bold"],
        "bold",
    );
    n.body_font_category = one_of("body_font_category", &["serif", "sans_serif"], "sans_serif");

    // The two NOT NULL colour columns keep their sentinel when nothing else
    // was chosen; the nullable ones are allowed to go back to NULL.
    n.background_color = colour_from_form(&form, "background_color").unwrap_or_else(|| "light".into());
    n.header_background_color =
        colour_from_form(&form, "header_background_color").unwrap_or_else(|| "transparent".into());
    n.post_title_color = colour_from_form(&form, "post_title_color");
    n.section_title_color = colour_from_form(&form, "section_title_color");
    n.divider_color = colour_from_form(&form, "divider_color");
    n.link_color = colour_from_form(&form, "link_color");
    n.button_color = colour_from_form(&form, "button_color");

    n.link_style = one_of("link_style", &["underline", "regular", "bold"], "underline");
    n.button_style = one_of("button_style", &["fill", "outline"], "fill");
    n.button_corners = one_of("button_corners", &["square", "rounded", "pill"], "rounded");
    n.image_corners = one_of("image_corners", &["square", "rounded"], "square");

    n.footer_content = keep("footer_content");
    n.show_badge = on("show_badge");
    n.show_latest_posts = on("show_latest_posts");
    n.show_comment_cta = on("show_comment_cta");
    n.show_share_button = on("show_share_button");
    n.feedback_enabled = on("feedback_enabled");
    n.show_subscription_details = on("show_subscription_details");

    match repos::newsletters::update(&state.db, &n).await {
        Ok(()) => newsletter_flash(&id, "ok", "Newsletter saved."),
        Err(e) => newsletter_flash(&id, "error", &e.message()),
    }
}

pub(crate) async fn newsletter_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let name = form.get("name").cloned().unwrap_or_default();
    match repos::newsletters::create(&state.db, &name).await {
        Ok(n) => Redirect::to(&format!("/ghost/settings/newsletters/{}/", n.id)).into_response(),
        Err(e) => settings_flash("newsletters", "error", &e.message()),
    }
}

/// The design editor's preview pane: the real renderer, on a real post.
///
/// Rendering the actual template rather than an approximation is the point —
/// a preview that agreed with a mock-up instead of the email would be worse
/// than none.
pub(crate) async fn newsletter_preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(n)) = repos::newsletters::find_by_id(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "No such newsletter").into_response();
    };
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    // The most recent published post, so the preview shows real content and
    // real card styling. A brand-new site has none, hence the placeholder.
    let sample_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE status IN ('published', 'sent') AND type = 'post' \
         ORDER BY published_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let sample = match sample_id {
        Some(id) => repos::posts::find_by_id(&state.db, &id).await.ok().flatten(),
        None => None,
    };

    let (post, post_title, html, plaintext, feature_image, published_at) = match sample {
        Some(p) => {
            let title = p.title.clone().unwrap_or_else(|| "Untitled".into());
            let html = p.html.clone().unwrap_or_default();
            let plaintext = p.plaintext.clone().unwrap_or_default();
            let feature_image = p.feature_image.clone();
            let published_at = p.published_at.clone().unwrap_or_default();
            (p, title, html, plaintext, feature_image, published_at)
        }
        None => (
            ghost_core::models::Post::default(),
            "Your next newsletter".to_string(),
            "<p>Publish a post and its content will appear here, styled exactly as subscribers will see it.</p>".to_string(),
            "Publish a post and its content will appear here.".to_string(),
            None,
            ghost_core::dates::now_db(),
        ),
    };
    // The permalink goes through the router for the same reason the send path
    // does it: a dated or prefixed structure is not /{slug}/.
    let post_json = ghost_core::models::post_to_json(&post, &[], &[], "");
    let router = state.router.read().await.clone();
    let path = match router.permalink_for(&post_json) {
        Some((_, p)) => ghost_routing::tree::build_permalink(&p, &post_json),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let post_url = format!("{site_url}{path}");

    let site_ctx = ghost_mail::newsletter::SiteContext {
        title: title.clone(),
        url: site_url.clone(),
        icon_url: state
            .settings
            .get_str("icon")
            .filter(|i| !i.is_empty())
            .map(|i| ghost_core::url::absolute(&site_url, &ghost_core::url::base_asset(&i))),
        locale: state.settings.get_str("locale").unwrap_or_else(|| "en".into()),
        direction: "ltr".into(),
        accent_color: state.settings.get_str("accent_color").unwrap_or_default(),
    };
    let post_ctx = ghost_mail::newsletter::PostContext {
        id: post.id.clone(),
        title: post_title,
        url: post_url,
        html: ghost_core::url::absolutize_html(&site_url, &html),
        plaintext,
        custom_excerpt: post.custom_excerpt.clone().filter(|e| !e.is_empty()),
        published_at: ghost_core::dates::short_date_in_zone(&published_at, &timezone),
        authors: title.clone(),
        feature_image,
        email_only: post.status == "sent",
        ..Default::default()
    };

    let rendered = ghost_mail::newsletter::NewsletterEmail {
        site: &site_ctx,
        newsletter: &n,
        post: &post_ctx,
        latest_posts: &[],
        comments_enabled: state
            .settings
            .get_str("comments_enabled")
            .map(|v| v != "off")
            .unwrap_or(false),
    };
    let html = match rendered.render() {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.message()).into_response(),
    };
    // The preview is not addressed to anybody, so the member tokens get
    // sample values rather than being left as `%%{…}%%` literals.
    let preview_member = ghost_mail::newsletter::Recipient {
        uuid: "preview".into(),
        email: "subscriber@example.com".into(),
        name: Some("Subscriber".into()),
        status: "free".into(),
        created_at: ghost_core::dates::short_date_in_zone(&ghost_core::dates::now_db(), &timezone),
        unsubscribe_url: "#".into(),
        manage_account_url: "#".into(),
        key: "preview".into(),
    };
    Html(ghost_mail::newsletter::personalize(&html, &preview_member)).into_response()
}

pub(crate) async fn newsletters_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "newsletters").unwrap();
    let base = state
        .settings
        .get_str("mailgun_base_url")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.mailgun.net/v3".into());
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_url = state.site_url();
    let mail_cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let status = match resolve_bulk_transport(&state, &mail_cfg) {
        Ok(transport) => {
            let name = match transport { BulkTransport::Mailgun(_) => "Mailgun", BulkTransport::Ses(_) => "Amazon SES", BulkTransport::Smtp => mail_cfg.transport_name() };
            format!(r#"<div class="banner ok">Newsletters use {name}.</div>"#)
        }
        Err(message) => format!(r#"<div class="banner info">{}</div>"#, esc(&message)),
    };

    // The newsletters themselves. Each row opens the design editor, which is
    // where every setting that shapes the email lives.
    let mut newsletter_rows = String::new();
    match repos::newsletters::list(&state.db).await {
        Ok(list) if !list.is_empty() => {
            for nl in &list {
                let count = repos::newsletters::subscriber_count(&state.db, &nl.id).await;
                newsletter_rows.push_str(&format!(
                    r#"<a class="row" href="/ghost/settings/newsletters/{id}/"><span class="ic tint-purple">{mail}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{count} subscriber{plural}{desc}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span>{chev}</div></a>"#,
                    id = esc(&nl.id),
                    mail = svg("mail"),
                    name = esc(&nl.name),
                    count = count,
                    plural = if count == 1 { "" } else { "s" },
                    desc = match nl.description.as_deref().filter(|d| !d.is_empty()) {
                        Some(d) => format!(" · {}", esc(d)),
                        None => String::new(),
                    },
                    chip = if nl.status == "active" { "published" } else { "draft" },
                    status = esc(&nl.status),
                    chev = svg("chevron"),
                ));
            }
        }
        _ => {
            newsletter_rows = r#"<div class="row"><div class="body"><span class="muted">No newsletters yet — add one below and posts will have somewhere to go.</span></div></div>"#.into();
        }
    }

    // --- newsletter sending + default recipients (Ghost's Email area) -----
    //
    // Ghost encodes both in the same pair of settings:
    // `editor_default_email_recipients` is `disabled` when sending is off, and
    // otherwise names the default audience — with `filter` deferring to
    // `editor_default_email_recipients_filter` for the segment cases.
    let recipients = state
        .settings
        .get_str("editor_default_email_recipients")
        .unwrap_or_else(|| "visibility".into());
    let recipients_filter = state.settings.get_str("editor_default_email_recipients_filter");
    let sending_on = recipients != "disabled";
    // With nobody able to sign up there is no one to email, so Ghost greys the
    // switch out and points at the setting that is really in the way.
    let signup_closed = state.settings.get_str("members_signup_access").as_deref() == Some("none");

    let sending_card = format!(
        r#"<div class="section">Newsletter sending</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-purple">{mail}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="newsletter_sending" form="newsletters-form" value="1"{ck}{dis}><span class="track"><span class="thumb"></span></span></label></div></div>
</div>
{note}"#,
        mail = svg("mail"),
        title = if sending_on && !signup_closed { "Enabled" } else { "Disabled" },
        detail = if sending_on && !signup_closed {
            "Newsletter features are active, posts can be sent by email"
        } else {
            "Posts cannot be sent by email while this is off"
        },
        ck = if sending_on && !signup_closed { " checked" } else { "" },
        dis = if signup_closed { " disabled" } else { "" },
        note = if signup_closed {
            r#"<p class="muted" style="margin:10px 2px 0">Your <a href="/ghost/settings/membership/">subscription access</a> is set to
&ldquo;Nobody can sign up&rdquo;, so there is no audience to email. Allow signups to turn newsletter sending back on.</p>"#
        } else {
            ""
        },
    );

    // The five audiences Ghost offers, flattened from the two settings.
    let recipients_value = match (recipients.as_str(), recipients_filter.as_deref()) {
        ("filter", Some("status:free,status:-free")) => "all-members",
        ("filter", Some("status:-free")) => "paid-only",
        ("filter", None) | ("filter", Some("")) => "none",
        ("filter", _) => "segment",
        (other, _) => other,
    };
    let recipients_card = if sending_on && !signup_closed {
        format!(
            r#"<div class="section">Default recipients</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{people}</span>
<div class="body" style="width:100%">
<label class="field fixed" style="margin:0;max-width:360px"><select name="editor_default_email_recipients" form="newsletters-form">{opts}</select><span>Who gets a new post by default</span>{car}</label>
<p class="muted" style="margin:8px 0 0">Set per post in the editor; this only picks the starting point. {segment_note}</p>
</div></div>
</div>"#,
            people = svg("ghost"),
            car = SELECT_CARET,
            opts = select_options(
                &{
                    let mut o = vec![
                        ("visibility", "Whoever has access to the post"),
                        ("all-members", "All members"),
                        ("paid-only", "Paid-members only"),
                        ("none", "Usually nobody"),
                    ];
                    // Only offered when it is already in use, so the saved
                    // segment stays selectable instead of the select falling
                    // back to its first option.
                    if recipients_value == "segment" {
                        o.push(("segment", "Specific people"));
                    }
                    o
                },
                recipients_value,
            ),
            // "Specific people" is a tier/label filter Ghost builds with a
            // picker this page has no equivalent of, so an existing one is
            // reported and preserved rather than silently rewritten.
            segment_note = if recipients_value == "segment" {
                format!(
                    "This site currently sends to a saved segment (<code>{}</code>); choosing another option replaces it.",
                    esc(recipients_filter.as_deref().unwrap_or("")),
                )
            } else {
                String::new()
            },
        )
    } else {
        String::new()
    };

    page_for_role(actor_role,
        "Newsletters",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}{status}
{sending_card}
{recipients_card}
<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:2">{newsletter_rows}</div>
<div class="card padded rise" style="--i:3"><form method="post" action="/ghost/settings/newsletters/new/" class="inline-form">
<div style="flex:1;min-width:220px">{new_name}</div>
<button class="btn tonal" type="submit">{add} Add newsletter</button>
</form>
<p class="muted" style="margin:10px 0 0">A new newsletter starts from Ghost's defaults. Open it to set the design — colours, typography, header and footer.</p></div>
<form method="post" action="/ghost/settings/newsletters/" id="newsletters-form">
{managed_notice}
<fieldset {managed_disabled} style="border:0;padding:0;margin:0;min-width:0">
<div class="section">Newsletter provider</div>
<div class="card padded"><label class="field fixed"><select name="newsletter_transport">{provider_opts}</select><span>Provider</span>{car}</label>
<label class="field" style="margin:0"><input name="bulk_email_from_address" value="{bulk_from}" placeholder=" "><span>From address · defaults to the transactional one</span></label>
<p class="muted">Configure Amazon SES credentials in <a href="/ghost/settings/email/">Email settings</a>. Automatic uses Mailgun when configured, then the transactional relay.</p></div>
<div class="section">Mailgun</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{send}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="mailgun_api_key" type="password" value="" placeholder=" " autocomplete="off"><span>API key</span></label>
<label class="field" style="margin:0"><input name="mailgun_webhook_signing_key" type="password" value="" placeholder=" " autocomplete="off"><span>Webhook signing key</span></label>
<label class="field" style="margin:0"><input name="mailgun_domain" value="{domain}" placeholder=" " autocomplete="off"><span>Sending domain · e.g. mg.example.com</span></label>
<label class="field fixed" style="margin:0"><select name="mailgun_base_url">{region_opts}</select><span>Region</span>{car}</label>
</div>
<p class="muted" style="margin:10px 0 0">{key_hint} Use the <strong>Sending</strong> API key, not the private account key. The region must match where the domain was created — an EU domain on the US endpoint fails with a 401 that looks like a bad key.</p>
</div></div>
</div>
</fieldset></form>
<div class="banner info rise" style="--i:4">{bulb}Newsletters are sent from the editor: choose <b>Publish and email</b> when publishing a post. Without Mailgun, a configured SMTP relay is used instead, one message per subscriber.</div>"#,
            head = group_head(group, Some("newsletters-form")),
            banner = flash_banner(&q),
            status = status,
            sending_card = sending_card,
            recipients_card = recipients_card,
            send = svg("send"),
            bulb = svg("mail"),
            car = SELECT_CARET,
            newsletter_rows = newsletter_rows,
            new_name = text_field("name", "Newsletter name", ""),
            add = svg("add"),
            managed_notice = mail_management_notice(&state),
            managed_disabled = if state.settings.mail_is_managed() { "disabled" } else { "" },
            provider_opts = select_options(&[("auto", "Automatic"), ("ses", "Amazon SES"), ("mailgun", "Mailgun"), ("smtp", "Transactional relay")], &state.settings.get_str("newsletter_transport").unwrap_or_else(|| "auto".into())),
            key_hint = secret_state(&state, "mailgun_api_key"),
            domain = s("mailgun_domain"),
            region_opts = select_options(
                &[
                    ("https://api.mailgun.net/v3", "US · api.mailgun.net"),
                    ("https://api.eu.mailgun.net/v3", "EU · api.eu.mailgun.net"),
                ],
                &base
            ),
            bulk_from = s("bulk_email_from_address"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn newsletters_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    if state.settings.mail_is_managed() && ghost_core::mail_settings::KEYS.iter().any(|key| form.contains_key(*key)) {
        return settings_flash("newsletters", "error", "Email delivery is managed by your hosting provider.");
    }
    let mut ok = true;
    if !state.settings.mail_is_managed() {
    if let Some(provider) = form.get("newsletter_transport") {
        if !["auto", "ses", "mailgun", "smtp"].contains(&provider.as_str()) {
            return settings_flash("newsletters", "error", "Choose a valid newsletter provider.");
        }
        ok &= state.settings.set(&state.db, "newsletter_transport", &serde_json::json!(provider)).await.is_ok();
    }
    for key in ["mailgun_domain", "mailgun_base_url", "bulk_email_from_address"] {
        let value = form.get(key).cloned().unwrap_or_default().trim().to_string();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    ok &= save_secret(&state, "mailgun_api_key", form.get("mailgun_api_key")).await;
    ok &= save_secret(&state, "mailgun_webhook_signing_key", form.get("mailgun_webhook_signing_key")).await;
    }

    // Newsletter sending and the default audience share two settings, the way
    // Ghost encodes them: `disabled` turns sending off, anything else names
    // the default audience, and the segment cases defer to the filter.
    let sending_on = form.get("newsletter_sending").map(|v| v == "1").unwrap_or(false);
    let (recipients, filter): (&str, Option<&str>) = if !sending_on {
        ("disabled", None)
    } else {
        match form.get("editor_default_email_recipients").map(String::as_str) {
            Some("all-members") => ("filter", Some("status:free,status:-free")),
            Some("paid-only") => ("filter", Some("status:-free")),
            Some("none") => ("filter", None),
            // An untouched saved segment: leave both settings exactly as they
            // are rather than flattening it to something this page can name.
            Some("segment") => ("", None),
            _ => ("visibility", None),
        }
    };
    if !recipients.is_empty() {
        ok &= state
            .settings
            .set(&state.db, "editor_default_email_recipients", &serde_json::json!(recipients))
            .await
            .is_ok();
        ok &= state
            .settings
            .set(
                &state.db,
                "editor_default_email_recipients_filter",
                &match filter {
                    Some(f) => serde_json::json!(f),
                    None => serde_json::Value::Null,
                },
            )
            .await
            .is_ok();
    }

    if ok {
        settings_flash("newsletters", "ok", "Bulk email settings saved.")
    } else {
        settings_flash("newsletters", "error", "Saving failed — check the logs.")
    }
}
