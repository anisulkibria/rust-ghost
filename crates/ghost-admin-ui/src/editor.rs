//! The post and page editor: the Koenig canvas and the settings rail beside it.

use super::*;

// ---------------------------------------------------------------------------
// the post/page editor
//
// Content is edited in Koenig — Ghost's own editor, loaded from the bundle
// vendored in `assets/` — and everything Ghost keeps behind its slide-over
// "post settings" panel lives in a second column that is always open.
//
// Koenig is a React component, so the content field only exists once its
// JavaScript has run. The markdown textarea the editor replaced is still in
// the page underneath it, disabled on load and re-enabled if the bundle fails
// to start, which keeps the editor usable without JavaScript at all.
// ---------------------------------------------------------------------------

/// Everything the editor page renders from. Gathered by the handlers so the
/// markup function stays a pure function of its inputs.
pub(crate) struct Editor<'a> {
    post_type: &'a str,
    /// `None` when composing something that does not exist yet.
    post: Option<&'a ghost_core::models::Post>,
    tags: Vec<ghost_core::models::Tag>,
    all_tags: Vec<ghost_core::models::Tag>,
    authors: Vec<ghost_core::models::User>,
    all_authors: Vec<ghost_core::models::User>,
    /// `custom-*.hbs` offered by the active theme, as (file stem, label).
    templates: Vec<(String, String)>,
    /// True once this post has been emailed; the newsletter picker is then
    /// withheld so an edit cannot send it a second time.
    already_sent: bool,
    /// What a scheduled post is already set to do when its date arrives, so
    /// re-opening it shows that choice rather than resetting it.
    delivery: repos::posts::Delivery,
    /// Active newsletters, for the audience panel's picker, as (id, name).
    all_newsletters: Vec<(String, String)>,
    /// Which newsletter this post is aimed at, and at which slice of its
    /// subscribers. Default for a post that has not chosen yet.
    audience: ghost_api::newsletter_audience::Audience,
    /// False for Contributors, who may only save drafts.
    can_publish: bool,
    all_tiers: Vec<(String, String)>,
    members_enabled: bool,
    stripe_enabled: bool,
    /// Request headers for api.unsplash.com, or `Null` when the Unsplash app
    /// is off or has no access key. Koenig uses this object verbatim.
    unsplash_headers: serde_json::Value,
    pintura_config: serde_json::Value,
    /// The forms the form card can embed: id, name and a field summary.
    forms: serde_json::Value,
    /// Whether the card may link to the builder: it is Administrator-only,
    /// and a link that leads to "Not allowed" is worse than none.
    can_manage_forms: bool,
    site_url: String,
    site_title: String,
    site_description: String,
}

/// Ghost's registered Unsplash application — a public Client-ID, the one
/// Ghost's own admin ships and talks to api.unsplash.com with. An install
/// that has not supplied its own key gets the same out-of-the-box picker
/// Ghost gives.
pub(crate) const GHOST_UNSPLASH_KEY: &str = "8672af113b0a8573edae3aa3713886265d9bb741d707f6c01a486cde8c278980";

/// The Authorization header Koenig needs to query Unsplash, if this site can.
///
/// The app has to be switched on; the access key is optional, because Ghost's
/// own registered application key stands in for one, exactly as Ghost's admin
/// does it. Koenig uses this object verbatim.
pub(crate) fn unsplash_headers(state: &AppState) -> serde_json::Value {
    if !state.settings.get_bool("unsplash") {
        return serde_json::Value::Null;
    }
    let key = state.settings.get_str("unsplash_access_key").unwrap_or_default();
    let key = key.trim();
    let key = if key.is_empty() { GHOST_UNSPLASH_KEY } else { key };
    serde_json::json!({
        "Authorization": format!("Client-ID {key}"),
        "Accept-Version": "v1",
    })
}

pub(crate) fn pintura_config(settings: &ghost_db::SettingsCache) -> serde_json::Value {
    if !settings.get_bool("pintura") { return serde_json::Value::Null; }
    let js = settings.get_str("pintura_js_url").unwrap_or_default();
    let css = settings.get_str("pintura_css_url").unwrap_or_default();
    if js.trim().is_empty() || css.trim().is_empty() { return serde_json::Value::Null; }
    serde_json::json!({"jsUrl": js.trim(), "cssUrl": css.trim()})
}

impl Editor<'_> {
    fn singular(&self) -> &'static str {
        if self.post_type == "page" {
            "page"
        } else {
            "post"
        }
    }

    fn base(&self) -> &'static str {
        if self.post_type == "page" {
            "/ghost/pages/"
        } else {
            "/ghost/posts/"
        }
    }

    fn id(&self) -> Option<&str> {
        self.post.map(|p| p.id.as_str())
    }

    /// A `posts` column, or "" for a post being composed.
    fn field(&self, get: impl Fn(&ghost_core::models::Post) -> Option<&str>) -> &str {
        self.post.and_then(get).unwrap_or("")
    }

    /// A `posts_meta` column, or "" when there is no meta row yet.
    fn meta(&self, get: impl Fn(&ghost_core::models::PostMeta) -> Option<&str>) -> &str {
        self.post.and_then(|p| p.meta.as_ref()).and_then(get).unwrap_or("")
    }
}

/// Whether a `mobiledoc` column holds anything worth converting.
///
/// Rows written by this port carry the placeholder `{}` — and Ghost's own
/// empty document has a `sections` array with nothing in it. Either way there
/// is no content, and treating it as content would shadow the HTML a seeded or
/// imported post does have.
pub(crate) fn has_mobiledoc_content(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.is_empty() || raw == "{}" {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|doc| doc.get("sections").and_then(|s| s.as_array()).map(|s| !s.is_empty()))
        .unwrap_or(false)
}

/// A collapsible settings panel in the right-hand column. "Post settings" —
/// the URL, date and excerpt a post is opened to settle — is the one panel
/// that starts open; every other panel stands folded until asked for.
/// `<details>` means every panel folds and unfolds without any JavaScript.
pub(crate) fn panel(icon_name: &str, tint: &str, title: &str, body: &str, open: bool) -> String {
    format!(
        r#"<details class="card panel"{open_attr}><summary><span class="ic tint-{tint}">{ic}</span>{title}<span class="twist">{twist}</span></summary>
<div class="panel-body">{body}</div></details>"#,
        open_attr = if open { " open" } else { "" },
        tint = tint,
        ic = svg(icon_name),
        title = esc(title),
        twist = svg("down"),
        body = body,
    )
}

/// A `text_field` the editor's script has to reach by id — the feature image
/// URL, the slug — same markup, plus the handle.
pub(crate) fn text_field_id(name: &str, id: &str, label: &str, value: &str) -> String {
    format!(
        r#"<label class="field"><input name="{name}" id="{id}" value="{value}" placeholder=" "><span>{label}</span></label>"#,
        name = esc(name),
        id = esc(id),
        value = esc(value),
        label = esc(label),
    )
}

/// An outlined text field with a floating label.
pub(crate) fn text_field(name: &str, label: &str, value: &str) -> String {
    format!(
        r#"<label class="field"><input name="{name}" value="{value}" placeholder=" "><span>{label}</span></label>"#,
        name = esc(name),
        value = esc(value),
        label = esc(label),
    )
}

pub(crate) fn textarea_field(name: &str, label: &str, value: &str, rows: u32, code: bool) -> String {
    format!(
        r#"<label class="field"><textarea name="{name}" rows="{rows}" placeholder=" "{cls}>{value}</textarea><span>{label}</span></label>"#,
        name = esc(name),
        rows = rows,
        cls = if code { r#" class="code""# } else { "" },
        value = esc(value),
        label = esc(label),
    )
}

pub(crate) fn select_field(name: &str, label: &str, options: &str) -> String {
    format!(
        r#"<label class="field fixed"><select name="{name}">{options}</select><span>{label}</span><svg class="car" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 10l5 5 5-5z"/></svg></label>"#,
        name = esc(name),
        options = options,
        label = esc(label),
    )
}

pub(crate) fn option_row(value: &str, label: &str, selected: bool) -> String {
    format!(
        r#"<option value="{value}"{sel}>{label}</option>"#,
        value = esc(value),
        sel = if selected { " selected" } else { "" },
        label = esc(label),
    )
}

pub(crate) fn toggle_row(name: &str, title: &str, detail: &str, on: bool) -> String {
    format!(
        r#"<div class="row toggle-row"><div class="label"><b>{title}</b><span>{detail}</span></div>
<label class="switch"><input type="checkbox" name="{name}" value="1"{on}><span class="track"><span class="thumb"></span></span></label></div>"#,
        title = esc(title),
        detail = esc(detail),
        name = esc(name),
        on = if on { " checked" } else { "" },
    )
}

/// Who the newsletter reaches: which newsletter, and which slice of its
/// subscribers.
///
/// `publishing-workflows.js` used to build this one in the browser and append
/// it after every other panel, out of bare browser widgets — a `<summary>`
/// with no icon and no twist, an unstyled `<select>`, a raw checkbox, a
/// `<fieldset>` in place of the panel body. It was the only card in the
/// editor that did not look like the rest, because it was the only one not
/// built from these helpers. Rendering it here costs nothing — the newsletter
/// list and the saved audience are two queries the editor's context was
/// already shaped to carry — and it cannot drift from its neighbours again,
/// since it is now made of the same parts. It sits next to Email, the setting
/// it qualifies, rather than at the bottom of the rail.
///
/// The hidden `_audience` marker is what tells the save path that this form
/// carried the audience fields at all, so that a form posted without them
/// leaves a post's existing choice alone.
pub(crate) fn audience_panel(ed: &Editor<'_>) -> String {
    let current = ed.audience.newsletter_id.as_deref().unwrap_or("");
    let options: String = std::iter::once(option_row("", "Default newsletter", current.is_empty()))
        .chain(ed.all_newsletters.iter().map(|(id, name)| option_row(id, name, id == current)))
        .collect();
    panel(
        "send",
        "green",
        "Newsletter audience",
        &format!(
            r#"<input type="hidden" name="_audience" value="1">{newsletter}{filter}{preview}
<p class="hint">Leave the filter empty to reach every eligible subscriber. Example: <code>labels:vip+status:paid</code>. Subscribers outside the chosen newsletter are left out either way.</p>"#,
            newsletter = select_field("newsletter_id", "Newsletter", &options),
            filter = text_field(
                "audience_filter",
                "Audience filter · optional NQL",
                ed.audience.filter.as_deref().unwrap_or(""),
            ),
            preview = toggle_row(
                "newsletter_include_preview",
                "Send a paywalled preview",
                "Subscribers without access get the public part rather than nothing.",
                ed.audience.include_preview,
            ),
        ),
        false,
    )
}

/// A set of checkbox chips posting under one repeated name.
pub(crate) fn chip_set(name: &str, options: &[(String, String)], selected: &[String], empty: &str) -> String {
    if options.is_empty() {
        return format!(r#"<p class="hint">{}</p>"#, esc(empty));
    }
    let chips: String = options
        .iter()
        .map(|(value, label)| {
            format!(
                r#"<label class="fchip"><input type="checkbox" name="{name}" value="{value}"{on}><span class="ck">{check}</span>{label}</label>"#,
                name = esc(name),
                value = esc(value),
                on = if selected.iter().any(|s| s == value) { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
            )
        })
        .collect();
    format!(r#"<div class="chips">{chips}</div>"#)
}

/// The NQL filter control the three list pages share, sitting in the toolbar
/// beside search as a peer of the sort menu.
///
/// It used to be a script-injected button that opened a `<dialog>` of bare
/// browser widgets; everything it can do is here instead, in the admin's own
/// controls. The filter itself is a plain GET form, so it applies without
/// script and is linkable — `carry` holds the hidden inputs (tab, sort) that
/// must survive the submit, or the act of filtering would silently reset the
/// rest of the view. Saved views need the API, so the script fills the
/// `[data-views]` slot; with no script the filter field still works and the
/// slot stays empty rather than leaving a dead button on the toolbar.
pub(crate) fn filter_menu(base: &str, resource: &str, nql: &str, carry: &str, examples: &str) -> String {
    let on = !nql.trim().is_empty();
    format!(
        r#"<details class="sort-menu filter-menu{active}" data-menu>
<summary>{tune}<span class="fm-text">Filter</span>{dot}{caret}</summary>
<div class="pop">
<form method="get" action="{base}">{carry}
<label class="field"><input name="nql" data-nql value="{nql}" placeholder=" " autocomplete="off" spellcheck="false" class="code"><span>NQL filter</span></label>
<p class="fm-hint">{examples}</p>
<div class="fm-actions"><button class="btn small" type="submit">{check} Apply</button>{clear}</div>
</form>
<p class="fm-title">Saved views</p>
<div class="fm-views" data-views="{resource}"></div>
<div class="fm-save">
<label class="field"><input data-view-name placeholder=" " autocomplete="off"><span>Name this view</span></label>
<button class="btn tonal small" type="button" data-view-save>{folder} Save view</button>
</div>
</div>
</details>"#,
        active = if on { " on" } else { "" },
        base = base,
        carry = carry,
        nql = esc(nql),
        resource = esc(resource),
        examples = esc(examples),
        tune = svg("tune"),
        check = svg("check"),
        folder = svg("folder"),
        caret = SELECT_CARET,
        dot = if on { r#"<span class="fm-dot" aria-label="a filter is active"></span>"# } else { "" },
        clear = if on {
            format!(r#"<a class="btn tonal small" href="{base}">Clear</a>"#)
        } else {
            String::new()
        },
    )
}

/// The segmented filter bar the list pages share. Each tab is a `filter=`
/// value with a label and the bucket it counts — `None` is the "everything"
/// tab, whose count is the sum of the rest — and `suffix` carries any other
/// query params that should survive the click (the posts list keeps its sort
/// this way). A tab that would show nothing is not offered, except the one
/// already selected, which has to stay clickable to be unselectable.
pub(crate) fn seg_filter_bar(
    tabs: &[(&str, &str, Option<&str>)],
    base: &str,
    active: &str,
    suffix: &str,
    counts: &HashMap<String, i64>,
) -> String {
    let segs: String = tabs
        .iter()
        .map(|(key, label, bucket)| {
            let count = match bucket {
                Some(b) => counts.get(*b).copied().unwrap_or(0),
                None => counts.values().sum(),
            };
            if count == 0 && *key != active {
                return String::new();
            }
            format!(
                r#"<a class="seg{on}" href="{base}?filter={key}{suffix}">{label}<span class="seg-count">{count}</span></a>"#,
                on = if *key == active { " on" } else { "" },
                base = esc(base),
                key = esc(key),
                suffix = suffix,
                label = esc(label),
                count = count,
            )
        })
        .collect();
    format!(r#"<div class="seg-bar">{segs}</div>"#)
}

/// `2026-10-19 14:30:00` (how the column is stored) → the `YYYY-MM-DDTHH:MM`
/// a `datetime-local` input wants.
pub(crate) fn to_datetime_local(stored: &str) -> String {
    if stored.len() < 16 {
        return String::new();
    }
    format!("{}T{}", &stored[..10], &stored[11..16])
}

/// The inverse. Times are read and written as UTC, which is what the column
/// holds and what the rest of this admin displays.
pub(crate) fn from_datetime_local(input: &str) -> Option<String> {
    let input = input.trim();
    if input.len() < 16 {
        return None;
    }
    let (date, time) = input.split_once('T')?;
    Some(format!("{date} {time}:00"))
}

pub(crate) fn editor_html(ed: &Editor<'_>) -> String {
    let base = ed.base();
    let singular = ed.singular();
    let action = match ed.id() {
        Some(id) => format!("{base}{id}/"),
        None => format!("{base}new/"),
    };
    let status = ed.post.map(|p| p.status.as_str()).unwrap_or(if ed.can_publish { "published" } else { "draft" });
    let title = ed.field(|p| p.title.as_deref());

    // --- the content column ------------------------------------------------

    // Koenig reopens the Lexical state it saved. A post written before this
    // editor existed has only markdown (or, for a seeded row, only HTML), so
    // that is wrapped into a one-card document instead — opening an old post
    // must never be able to lose anything in it.
    let markdown = ed.field(|p| p.markdown.as_deref());
    let saved = ed.field(|p| p.lexical.as_deref().filter(|l| !l.trim().is_empty()));
    // A post from before Ghost 5 has its content in `mobiledoc`. Converting it
    // here is what makes it editable at all: wrapping its rendered HTML into a
    // single HTML card would turn every paragraph, heading and card in it into
    // one opaque block the publisher cannot edit. The conversion is saved back
    // as Lexical on the next save, the same point at which Ghost migrates it.
    let mobiledoc = ed.field(|p| p.mobiledoc.as_deref().filter(|m| has_mobiledoc_content(m)));
    let initial = if !saved.is_empty() {
        Some(saved.to_string())
    } else if !mobiledoc.is_empty() {
        Some(ghost_render::mobiledoc::to_lexical(Some(mobiledoc)))
    } else {
        ghost_render::lexical::wrap_existing(Some(markdown), Some(ed.field(|p| p.html.as_deref())))
    };
    // Content is stored without the install's base path, but Koenig renders
    // it live — an image card has to be able to load its `src`. The write path
    // normalizes it back, so reopening and saving a post does not persist the
    // base path. A no-op at a domain root.
    let initial = initial
        .as_deref()
        .map(ghost_core::url::unbase_assets)
        .map(|lexical| ghost_core::url::base_assets(&lexical));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": format!("Begin writing your {singular}..."),
        "postType": ed.post_type,
        "isNew": ed.id().is_none(),
        "savedStatus": ed.post.map(|p| p.status.as_str()).unwrap_or("draft"),
        "siteUrl": ed.site_url,
        "siteTitle": ed.site_title,
        "siteDescription": ed.site_description,
        "membersEnabled": ed.members_enabled,
        "stripeEnabled": ed.stripe_enabled,
        // the bridge prefixes its upload and oembed endpoints with this; it
        // cannot read the base path off the page, since its own script URL has
        // already been rewritten
        "basePath": ghost_core::url::subdir(),
        // Unsplash: the editor talks to api.unsplash.com directly, so what it
        // needs is the Authorization header. Sent only when the app is on and
        // a key exists; otherwise Koenig shows its bundled sample gallery.
        "unsplash": ed.unsplash_headers,
        "pinturaConfig": ed.pintura_config,
        // What the form card's picker lists. Names and field labels only —
        // never a response.
        "forms": ed.forms,
        "canManageForms": ed.can_manage_forms,
    });

    let content = format!(
        r#"<div class="editor-main">
<textarea class="editor-title" id="field-title" name="title" rows="1" placeholder="{singular_title} title">{title}</textarea>
<div id="koenig-mount" data-config="{config}"></div>
<input type="hidden" name="lexical" id="field-lexical" value="">
<input type="hidden" name="html" id="field-html" value="">
<input type="hidden" name="updated_at" id="field-updated-at" value="{updated_at}">
<div id="markdown-fallback">
<p class="hint" data-fallback-note>JavaScript is off, so the rich editor cannot run. This {singular} is editable as Markdown.</p>
{markdown_field}
</div>
<span class="wc-pill" id="word-count" aria-live="polite"></span>
</div>"#,
        singular_title = if singular == "page" { "Page" } else { "Post" },
        title = esc(title),
        config = esc(&config.to_string()),
        singular = singular,
        markdown_field = textarea_field("markdown", "Content · Markdown", markdown, 20, true),
        // The stamp this editor loaded. Posted back on save so the server can
        // tell this copy apart from one another session has since written.
        updated_at = esc(ed.post.and_then(|p| p.updated_at.as_deref()).unwrap_or("")),
    );

    // --- the settings column -----------------------------------------------
    //
    // The panels run in Ghost's own order — feature image first, then the
    // post's URL and date, tags, access, authors, template, code injection,
    // meta data, the social cards. "Post settings" is the one panel that
    // starts open; the rest stand folded, one summary line each, so the rail
    // reads as a menu of what there is to set rather than a wall of forms.
    // Publishing, holding the status select that stands in for Ghost's header
    // publish flow, is this port's own.
    let mut side = String::new();

    // Feature image: an image box — upload or Unsplash while empty, a
    // preview with Change and Remove once set — over the URL field and the
    // alt text and caption Ghost keeps in posts_meta.
    let feature_image = ed.field(|p| p.feature_image.as_deref());
    // The Unsplash picker is always offered; without an access key it opens
    // and says what is missing rather than hiding the option entirely.
    let unsplash_btn =
        format!(r#"<button type="button" class="btn small ghosted" id="fimg-unsplash">{search} Unsplash</button>"#, search = svg("search"));
    side.push_str(&panel(
        "image",
        "green",
        "Feature image",
        &format!(
            r#"<input type="file" id="fimg-file" accept="image/gif,image/jpeg,image/png,image/svg+xml,image/webp" hidden>
<div class="fimg-drop{set}" id="fimg-drop">
<div class="fimg-empty" id="fimg-empty"><div class="fimg-btns">
<button type="button" class="btn tonal small" id="fimg-upload">{upload} Upload</button>{unsplash}
</div></div>
<img id="fimg-img" src="{src}" alt="">
<div class="fimg-acts">
<button type="button" class="btn small ghosted" id="fimg-change">Change image</button>
<button type="button" class="btn small ghosted danger" id="fimg-remove">Remove</button>
</div>
<div class="fimg-prog" id="fimg-prog"><i></i></div>
</div>
<details class="panel-sub"><summary>Alt text, caption &amp; image URL{twist}</summary>
<div class="panel-sub-body">{alt}{caption}{url}</div></details>"#,
            set = if feature_image.is_empty() { "" } else { " is-set" },
            src = esc(&ghost_core::url::base_asset(feature_image)),
            upload = svg("upload"),
            unsplash = unsplash_btn,
            twist = svg("down"),
            alt = text_field("feature_image_alt", "Alt text", ed.meta(|m| m.feature_image_alt.as_deref())),
            caption = text_field_id(
                "feature_image_caption",
                "field-feature_image_caption",
                "Caption",
                ed.meta(|m| m.feature_image_caption.as_deref()),
            ),
            url = text_field_id("feature_image", "field-feature_image", "Image URL", feature_image),
        ),
        true,
    ));

    // General: URL, publish date, excerpt — the panel an open editor starts from.
    let slug = ed.field(|p| p.slug.as_deref());
    side.push_str(&panel(
        "link",
        "blue",
        &format!("{} settings", if singular == "page" { "Page" } else { "Post" }),
        &format!(
            r#"{slug_field}<p class="hint" id="slug-preview">{preview}</p>
<label class="field fixed"><input type="datetime-local" name="published_at" value="{published_at}"><span>Publish date · UTC</span></label>
{excerpt}<p class="hint">Shown in post lists and feeds in place of the opening lines.</p>"#,
            slug_field = text_field_id("slug", "field-slug", "URL slug", slug),
            // Only worth showing once there is a slug: the bare origin with
            // two slashes after it is not a URL anyone needs to see.
            preview = if slug.is_empty() {
                String::new()
            } else {
                format!("{}/<b>{}</b>/", esc(&ed.site_url), esc(slug))
            },
            published_at = esc(&to_datetime_local(ed.field(|p| p.published_at.as_deref()))),
            excerpt = textarea_field("custom_excerpt", "Excerpt", ed.field(|p| p.custom_excerpt.as_deref()), 3, false),
        ),
        true,
    ));

    // Status and, for a brand-new post, whether it also goes out as email.
    let status_options = format!(
        "{draft}{rest}",
        draft = option_row("draft", "Draft", status == "draft"),
        rest = if ed.can_publish {
            format!(
                "{}{}",
                option_row("published", "Published", status == "published"),
                option_row("scheduled", "Scheduled", status == "scheduled"),
            )
        } else {
            String::new()
        },
    );
    // Offered when creating a post, and while one is scheduled: the email for
    // a scheduled post has not gone out yet, so the choice is still live and
    // has to be editable right up to its date. Pages are never emailed, and
    // an edit to anything already out must never be able to mail subscribers
    // a second time.
    let scheduled = status == "scheduled";
    let publish_action = if ed.post_type == "page"
        || ed.already_sent
        || !ed.can_publish
        || (ed.id().is_some() && !scheduled)
    {
        String::new()
    } else {
        let current = ed.delivery;
        format!(
            r#"{select}<p class="hint">{hint}</p>"#,
            select = select_field(
                "publish_action",
                "When publishing",
                &format!(
                    "{}{}{}",
                    option_row("publish_only", "Publish only", !current.email),
                    option_row("publish_email", "Publish and email", current.email && !current.email_only),
                    option_row("email_only", "Email only", current.email_only),
                ),
            ),
            hint = if scheduled {
                "Decided when the date arrives. Change it any time before then."
            } else {
                "Emailing happens once, when the post is first created."
            },
        )
    };
    // The date is what a scheduled post is waiting for, so say so where it is
    // set rather than leaving an empty box to guess at.
    let schedule_hint = if scheduled {
        match ed.field(|p| p.published_at.as_deref()) {
            "" => r#"<p class="hint">Set a publish date above — a scheduled post needs one in the future.</p>"#.to_string(),
            at => format!(
                r#"<p class="hint">Goes out {} UTC, on its own.</p>"#,
                esc(&to_datetime_local(at).replace('T', " at ")),
            ),
        }
    } else {
        String::new()
    };
    side.push_str(&panel(
        "flag",
        "pink",
        "Publishing",
        &format!(
            "{status}{schedule_hint}{publish_action}{featured}{show_title}",
            status = select_field("status", "Status", &status_options),
            featured = toggle_row(
                "featured",
                "Feature this",
                "Themes can give featured posts their own treatment.",
                ed.post.map(|p| p.featured).unwrap_or(false),
            ),
            show_title = toggle_row(
                "show_title_and_feature_image",
                "Show title and feature image",
                "Turn off for a bare canvas with no header.",
                ed.post.map(|p| p.show_title_and_feature_image).unwrap_or(true),
            ),
        ),
        false,
    ));

    let tag_options: Vec<(String, String)> = ed
        .all_tags
        .iter()
        .map(|t| (t.id.clone(), t.name.clone().unwrap_or_default()))
        .collect();
    let tag_selected: Vec<String> = ed.tags.iter().map(|t| t.id.clone()).collect();
    side.push_str(&panel(
        "tags",
        "amber",
        "Tags",
        &chip_set("tags", &tag_options, &tag_selected, "No tags yet — create one under Tags."),
        false,
    ));

    // Access includes explicit paid-tier entitlements.
    let visibility = ed.post.map(|p| p.visibility.as_str()).unwrap_or("public");
    let access_body = if ed.members_enabled {
        select_field(
            "visibility",
            "Who can read this",
            &format!(
                "{}{}{}{}",
                option_row("public", "Everyone", visibility == "public"),
                option_row("members", "Members only", visibility == "members"),
                option_row("paid", "Paid members only", visibility == "paid"),
                option_row("tiers", "Specific paid tiers", visibility == "tiers"),
            ),
        )
    } else {
        format!(
            r#"<input type="hidden" name="visibility" value="{visibility}"><p class="hint">Memberships are off, so everything published is public. Turn them on under Settings → Membership.</p>"#,
            visibility = esc(visibility),
        )
    };
    let access_body = if ed.members_enabled {
        format!("{}<input type=\"hidden\" name=\"_tiers\" value=\"1\"><p class=\"hint\">Allowed tiers · used when access is set to Specific paid tiers.</p>{}", access_body,
            chip_set("tiers", &ed.all_tiers, ed.post.map(|p| p.tier_ids.as_slice()).unwrap_or(&[]), "Create a paid tier under Membership to restrict access by tier."))
    } else { access_body };
    side.push_str(&panel("lock", "slate", "Access", &access_body, false));

    // Email, when this post can still become one.
    if ed.post_type == "post" {
        let sent_note = if ed.already_sent {
            r#"<p class="hint">This post has already been emailed to subscribers. The subject is kept for the record.</p>"#
        } else {
            r#"<p class="hint">Used instead of the title in the subject line of the newsletter.</p>"#
        };
        side.push_str(&panel(
            "mail",
            "teal",
            "Email",
            &format!(
                "{subject}{sent_note}",
                subject = text_field("email_subject", "Email subject", ed.meta(|m| m.email_subject.as_deref())),
            ),
        false,
        ));
        side.push_str(&audience_panel(ed));
    }

    let author_options: Vec<(String, String)> =
        ed.all_authors.iter().map(|u| (u.id.clone(), u.name.clone().unwrap_or_default())).collect();
    let author_selected: Vec<String> = ed.authors.iter().map(|u| u.id.clone()).collect();
    side.push_str(&panel(
        "person",
        "purple",
        "Authors",
        &format!(
            r#"{chips}<p class="hint">With none ticked, the {singular} stays with whoever created it.</p>"#,
            chips = chip_set("authors", &author_options, &author_selected, "No staff users yet."),
            singular = singular,
        ),
        false,
    ));

    // Theme template, when the active theme offers a choice.
    let current_template = ed.field(|p| p.custom_template.as_deref());
    let template_body = if ed.templates.is_empty() {
        format!(
            r#"<input type="hidden" name="custom_template" value="{current}"><p class="hint">The active theme has no <code>custom-*.hbs</code> templates, so this {singular} uses the theme's default.</p>"#,
            current = esc(current_template),
            singular = singular,
        )
    } else {
        let options: String = std::iter::once(option_row("", "Default", current_template.is_empty()))
            .chain(ed.templates.iter().map(|(file, label)| option_row(file, label, file == current_template)))
            .collect();
        select_field("custom_template", "Template", &options)
    };
    side.push_str(&panel("layout", "slate", "Template", &template_body, false));

    // Per-post code injection.
    side.push_str(&panel(
        "code",
        "red",
        "Code injection",
        &format!(
            "{head}{foot}",
            head = textarea_field("codeinjection_head", "Head", ed.field(|p| p.codeinjection_head.as_deref()), 4, true),
            foot = textarea_field("codeinjection_foot", "Footer", ed.field(|p| p.codeinjection_foot.as_deref()), 4, true),
        ),
        false,
    ));

    // Search engines.
    side.push_str(&panel(
        "search",
        "blue",
        "Meta data",
        &format!(
            r#"{title}{description}{canonical}<p class="hint">Point search engines at the original when this is a repost.</p>"#,
            title = text_field("meta_title", "Meta title", ed.meta(|m| m.meta_title.as_deref())),
            description = textarea_field("meta_description", "Meta description", ed.meta(|m| m.meta_description.as_deref()), 3, false),
            canonical = text_field("canonical_url", "Canonical URL", ed.field(|p| p.canonical_url.as_deref())),
        ),
        false,
    ));

    // The two social cards Ghost lets you override per post.
    side.push_str(&panel(
        "share",
        "pink",
        "Social cards",
        &format!(
            r#"<p class="hint">Facebook, LinkedIn and anything else reading Open Graph.</p>{og_title}{og_desc}{og_image}
<p class="hint">X (Twitter).</p>{tw_title}{tw_desc}{tw_image}"#,
            og_title = text_field("og_title", "Open Graph title", ed.meta(|m| m.og_title.as_deref())),
            og_desc = textarea_field("og_description", "Open Graph description", ed.meta(|m| m.og_description.as_deref()), 2, false),
            og_image = text_field("og_image", "Open Graph image URL", ed.meta(|m| m.og_image.as_deref())),
            tw_title = text_field("twitter_title", "X title", ed.meta(|m| m.twitter_title.as_deref())),
            tw_desc = textarea_field("twitter_description", "X description", ed.meta(|m| m.twitter_description.as_deref()), 2, false),
            tw_image = text_field("twitter_image", "X image URL", ed.meta(|m| m.twitter_image.as_deref())),
        ),
        false,
    ));

    // --- head, and the page around both columns -----------------------------

    let heading = if ed.id().is_some() {
        format!("Edit {singular}")
    } else {
        format!("New {singular}")
    };
    let preview = format!(
        r#"<a class="btn text" id="editor-view" href="{site}/{slug}/" target="_blank" rel="noopener"{hidden}>{open} View {singular}</a>"#,
        site = esc(&ed.site_url),
        slug = esc(ed.field(|p| p.slug.as_deref())),
        hidden = if status == "published" && ed.id().is_some() { "" } else { " hidden" },
        open = svg("open"),
    );
    // The delete form cannot be nested inside the editor form, so it sits in
    // the head and the settings column links to it by id.
    let delete = match ed.id() {
        Some(id) => format!(
            r#"<form method="post" action="{base}{id}/delete/" id="delete-form" data-confirm="Delete {singular}|&#8220;{title}&#8221; will be permanently deleted."></form>"#,
            base = base,
            id = esc(id),
            singular = singular,
            title = esc(if title.is_empty() { "Untitled" } else { title }),
        ),
        None => String::new(),
    };
    if ed.id().is_some() {
        side.push_str(&format!(
            r#"<div class="card panel"><div class="panel-body" style="padding-top:14px">
<button class="btn text danger" type="submit" form="delete-form">{del} Delete {singular}</button></div></div>"#,
            del = svg("delete"),
            singular = singular,
        ));
    } else {
        // Nothing to delete until the first save creates the post; the
        // editor's script reveals this once it has an id to delete.
        side.push_str(&format!(
            r#"<div class="card panel" id="new-delete" hidden><div class="panel-body" style="padding-top:14px">
<button class="btn text danger" type="button">{del} Delete {singular}</button></div></div>"#,
            del = svg("delete"),
            singular = singular,
        ));
    }

    format!(
        r#"<style>
/* Koenig's type scale is written in rem against Ghost's admin, where the
   root is 62.5% so 1rem reads as 10px. This admin sizes everything in px and
   uses no rem at all, so re-basing the root here is invisible to the rest of
   the page and puts the editor — prose, toolbars, card menu — at the size it
   was drawn for. Scoped to this page rather than the shared stylesheet so a
   future rem elsewhere cannot be caught out by it. */
html {{ font-size: 62.5%; }}
</style>
<header class="page-head rise"><a class="icon-btn" href="{base}" title="Back to {plural}">{back}</a>
<div><h1>{heading}</h1></div>
<div class="actions"><a class="btn text" href="{base}">Cancel</a>{preview}
<span class="save-state" id="save-state" aria-live="polite"></span>
<input type="checkbox" id="side-toggle">
<label class="icon-btn side-btn" for="side-toggle" title="Show or hide settings">
<span class="when-shown">{hide}</span><span class="when-hidden">{show}</span></label>
<button class="btn" id="editor-save" type="submit" form="editor-form">{save} <span data-save-label>{save_label}</span></button></div></header>
{delete}
<form method="post" action="{action}" id="editor-form" class="editor-shell">
{content}
<aside class="editor-side">{side}</aside>
</form>
{pintura_assets}
<script src="{assets}/react.js" defer></script>
<script src="{assets}/react-jsx.js" defer></script>
<script src="{assets}/react-dom.js" defer></script>
<script src="{assets}/koenig-lexical.js" defer></script>
<script src="{assets}/unsplash-picker.js" defer></script>
<script src="{assets}/koenig-integrations.js" defer></script>
<script src="{assets}/koenig-forms.js" defer></script>
<script src="{assets}/koenig-bridge.js" defer></script>
<script>
/* Hide the Markdown fallback before anything is painted, and disable it so a
   stale copy of the content cannot be posted alongside the editor's. The
   bridge puts it back if Koenig fails to start. */
(function () {{
  var fallback = document.getElementById('markdown-fallback');
  if (!fallback) {{ return; }}
  fallback.hidden = true;
  fallback.querySelector('textarea').disabled = true;
  /* The title is a textarea so it can wrap; it grows with its content. */
  var title = document.getElementById('field-title');
  var grow = function () {{ title.style.height = 'auto'; title.style.height = title.scrollHeight + 'px'; }};
  title.addEventListener('input', grow);
  title.addEventListener('keydown', function (e) {{ if (e.key === 'Enter') {{ e.preventDefault(); }} }});
  grow();

  /* The settings rail hides and shows through a checkbox and CSS alone, so
     it still works with this script absent — all the script adds is
     remembering the choice for the next post you open. */
  var toggle = document.getElementById('side-toggle');
  try {{
    toggle.checked = localStorage.getItem('ghost:editor-settings-hidden') === '1';
    toggle.addEventListener('change', function () {{
      localStorage.setItem('ghost:editor-settings-hidden', toggle.checked ? '1' : '0');
    }});
  }} catch (e) {{ /* private browsing: the toggle still works, it just forgets */ }}
}})();
</script>"#,
        base = base,
        plural = if ed.post_type == "page" { "pages" } else { "posts" },
        back = svg("chevron"),
        heading = esc(&heading),
        hide = svg("panel-hide"),
        show = svg("panel-show"),
        preview = preview,
        save = svg("check"),
        save_label = if ed.id().is_none() && ed.can_publish { "Publish".to_string() } else { format!("Save {singular}") },
        delete = delete,
        action = action,
        content = content,
        side = side,
        assets = ASSETS,
        pintura_assets = pintura_asset_tags(&ed.pintura_config),
    )
}

// -------------------------------------------------------------------------
// Opening the editor: the context the page is built from.
// -------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// loading the editor
// ---------------------------------------------------------------------------

/// `custom-*.hbs` in the active theme, as (stem, human label).
pub(crate) async fn custom_templates(state: &AppState) -> Vec<(String, String)> {
    let theme = state.theme.read().await.clone();
    let mut out: Vec<(String, String)> = theme
        .templates
        .iter()
        .filter_map(|t| t.strip_prefix("custom-"))
        .map(|rest| {
            let file = format!("custom-{rest}");
            // "custom-full-width" reads as "Full width" in Ghost's picker.
            let mut label = rest.replace(['-', '_'], " ");
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            (file, label)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Fill in everything the editor page needs that does not come from the post.
pub(crate) async fn editor_context<'a>(
    state: &AppState,
    post_type: &'a str,
    post: Option<&'a ghost_core::models::Post>,
    tags: Vec<ghost_core::models::Tag>,
    authors: Vec<ghost_core::models::User>,
    can_publish: bool,
) -> Editor<'a> {
    let already_sent = match post {
        Some(p) => newsletter_already_sent(state, &p.id).await,
        None => false,
    };
    let delivery = match post {
        Some(p) => repos::posts::delivery(&state.db, &p.id).await,
        None => repos::posts::Delivery::default(),
    };
    // A post that does not exist yet has made no choice, so it takes the
    // default rather than the error `load` would return for a missing row.
    let audience = match post {
        Some(p) => ghost_api::newsletter_audience::load(state, &p.id).await.unwrap_or_default(),
        None => ghost_api::newsletter_audience::Audience::default(),
    };
    Editor {
        post_type,
        post,
        tags,
        all_tags: repos::tags::all(&state.db).await.unwrap_or_default(),
        authors,
        all_authors: repos::users::list(&state.db, false).await.unwrap_or_default(),
        templates: custom_templates(state).await,
        already_sent,
        delivery,
        all_newsletters: repos::newsletters::list(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.status == "active")
            .map(|n| (n.id, n.name))
            .collect(),
        audience,
        can_publish,
        all_tiers: sqlx::query_as("SELECT id, name FROM products WHERE type = 'paid' ORDER BY name").fetch_all(&state.db).await.unwrap_or_default(),
        members_enabled: state.settings.get_str("members_signup_access").as_deref() != Some("none"),
        stripe_enabled: state.settings.get_bool("paid_members_enabled"),
        unsplash_headers: unsplash_headers(state),
        pintura_config: pintura_config(&state.settings),
        forms: forms::editor_choices(state).await,
        can_manage_forms: false,
        site_url: state.site_url(),
        site_title: state.settings.get_str("title").unwrap_or_default(),
        site_description: state.settings.get_str("description").unwrap_or_default(),
    }
}

pub(crate) async fn editor_new_impl(state: &AppState, headers: &HeaderMap, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let mut ed = editor_context(state, post_type, None, vec![], vec![], actor.role.can_publish()).await;
    ed.can_manage_forms = actor.role.is_admin();
    page_for(
        "New",
        if post_type == "page" { "pages" } else { "posts" },
        Some(&actor.user),
        actor.role,
        &editor_html(&ed),
        true,
    )
    .into_response()
}

pub(crate) async fn editor_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    // ?type=page kept for links that predate the dedicated /pages/new/ route
    let post_type = if q.get("type").map(String::as_str) == Some("page") { "page" } else { "post" };
    editor_new_impl(&state, &headers, post_type).await
}

pub(crate) async fn page_new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    editor_new_impl(&state, &headers, "page").await
}

pub(crate) async fn editor_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !may_edit_post(&state, &actor, &id).await {
        return forbidden_page(&actor, Role::Editor);
    }
    let user = actor.user;
    let Some(post) = repos::posts::find_by_id(&state.db, &id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
        .await
        .unwrap_or_default();
    let tags = rel.tags_for(&post.id);
    let authors = rel.authors_for(&post.id);
    let mut ed = editor_context(&state, &post.post_type, Some(&post), tags, authors, actor.role.can_publish()).await;
    ed.can_manage_forms = actor.role.is_admin();
    page_for(
        "Edit",
        if post.post_type == "page" { "pages" } else { "posts" },
        Some(&user),
        actor.role,
        &format!("{}{}", flash_banner(&q), editor_html(&ed)),
        true,
    )
    .into_response()
}
