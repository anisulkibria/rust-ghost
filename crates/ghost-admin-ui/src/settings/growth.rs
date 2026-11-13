//! Settings -> Growth: signup forms, recommendations and offers.

use super::*;

// --- growth: recommendations + signup forms (network & explore come later) ---

/// Everything the embeddable signup form is built from.
pub(crate) struct SignupFormEmbed<'a> {
    pub(crate) site_url: &'a str,
    /// `all-in-one` or `minimal`.
    pub(crate) layout: &'a str,
    pub(crate) background: &'a str,
    pub(crate) button: &'a str,
    pub(crate) title: &'a str,
    pub(crate) description: &'a str,
    pub(crate) icon: &'a str,
    pub(crate) locale: &'a str,
    pub(crate) labels: &'a [&'a str],
    /// A preview is the same embed sized to fill its frame instead of
    /// sitting in a page.
    pub(crate) preview: bool,
}

/// The `<script>` embed for the signup form, as Ghost's `generateCode` writes
/// it.
///
/// Three things here are load-bearing and were all wrong before. The script
/// comes from the CDN — the previous snippet pointed at
/// `{site}/public/signup-form.min.js`, which this server does not serve, so
/// every embed handed out was a 404 and the form never appeared. The script
/// is wrapped in a sized `<div>`, because the form renders into its container
/// and an unsized one collapses to nothing. And the two computed contrast
/// colours are sent: without `text-color` the copy on a dark background is
/// drawn dark on dark.
///
/// Attribute order follows Ghost's `preferredOrder` so a snippet from this
/// screen and one from Ghost diff cleanly.
pub(crate) fn signup_form_snippet(e: SignupFormEmbed<'_>) -> String {
    let all_in_one = e.layout != "minimal";
    let mut attrs: Vec<(String, String)> = Vec::new();

    if all_in_one {
        attrs.push(("background-color".into(), e.background.to_string()));
        attrs.push(("text-color".into(), ghost_render::color::contrast_text_color(e.background)));
    }
    attrs.push(("button-color".into(), e.button.to_string()));
    attrs.push(("button-text-color".into(), ghost_render::color::contrast_text_color(e.button)));
    if all_in_one {
        // Ghost sends these even when blank, so the form can tell "no title"
        // from "title not configured by this embed".
        attrs.push(("title".into(), e.title.to_string()));
        attrs.push(("description".into(), e.description.to_string()));
        if !e.icon.is_empty() {
            // Ghost asks the image server for a 192px square rather than
            // shipping the full-size icon to every embed.
            attrs.push(("icon".into(), icon_at_192(&ghost_core::url::base_asset(e.icon))));
        }
    }
    attrs.push(("site".into(), format!("{}/", e.site_url)));
    attrs.push(("locale".into(), e.locale.to_string()));
    for (i, label) in e.labels.iter().enumerate() {
        attrs.push((format!("label-{}", i + 1), (*label).to_string()));
    }

    let style = match (e.preview, all_in_one) {
        (false, true) => "height: 40vmin;min-height: 360px",
        (false, false) => "min-height: 58px;max-width: 440px;margin: 0 auto;width: 100%",
        (true, true) => "height: 100vh",
        (true, false) => {
            "min-height:58px;width:calc(100% - 48px);position:absolute;left:50%;top:50%;transform:translate(-50%, -50%)"
        }
    };

    let data: String =
        attrs.iter().map(|(k, v)| format!(r#" data-{k}="{}""#, esc(v))).collect();
    format!(
        r#"<div style="{style}"><script src="{src}"{data} async></script></div>"#,
        style = esc(style),
        src = ghost_render::apps::signup_form_script_url(),
        data = data,
    )
}

/// Rewrite a `/content/images/…` URL to ask for a 192px square, the way
/// Ghost's embed code does before handing the icon to the signup form.
///
/// Ghost's regex is `/content/images/(?:size/[^/]+/)*` — any size segment
/// already on the URL is dropped before the wanted one goes in, so asking
/// twice cannot produce `size/w192h192/size/w192h192/`. Written out by hand
/// because this workspace pulls in no regex crate, and this is the one
/// substitution that would need it.
pub(crate) fn icon_at_192(url: &str) -> String {
    const MARKER: &str = "/content/images/";
    let Some(at) = url.find(MARKER) else {
        return url.to_string();
    };
    let (head, mut rest) = url.split_at(at + MARKER.len());
    while let Some(stripped) = rest.strip_prefix("size/") {
        match stripped.find('/') {
            Some(slash) => rest = &stripped[slash + 1..],
            None => break,
        }
    }
    format!("{head}size/w192h192/{rest}")
}

pub(crate) async fn growth_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Each row is its own editing form, the way the custom-fields list works:
    // a recommendation is three short fields, so a separate page would be more
    // navigation than content.
    let mut rec_rows = String::new();
    if let Ok(recs) = sqlx::query_as::<_, (String, String, String, Option<String>, i64, Option<String>)>(
        "SELECT id, url, title, description, one_click_subscribe, favicon FROM recommendations \
         ORDER BY featured DESC, created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    {
        if recs.is_empty() {
            rec_rows = r#"<div class="row"><div class="body"><span class="muted">No recommended sites yet — add one below.</span></div></div>"#.into();
        }
        for (id, url, title, description, one_click, favicon) in recs {
            // The publication's own favicon, when it had one, so this list
            // shows what a member will actually see.
            let mark = match favicon.as_deref().filter(|f| f.starts_with("http")) {
                Some(favicon) => format!(
                    r#"<span class="ic"><img src="{}" alt="" width="20" height="20" style="border-radius:4px" loading="lazy" onerror="this.closest('.ic').innerHTML='{}'"></span>"#,
                    esc(favicon),
                    svg("globe").replace('\'', "&#39;").replace('\n', ""),
                ),
                None => format!(r#"<span class="ic tint-slate">{}</span>"#, svg("globe")),
            };
            rec_rows.push_str(&format!(
                r#"<div class="row" style="align-items:flex-start">{mark}
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/growth/recommendations/{id}/" class="inline-form" style="align-items:flex-end">
<label class="field" style="flex:1;min-width:160px;margin:0"><input name="title" value="{title}" placeholder=" " maxlength="191" required><span>Title</span></label>
<label class="field" style="flex:1.4;min-width:200px;margin:0"><input name="url" value="{url}" placeholder=" " maxlength="2000" required><span>URL</span></label>
<button class="btn tonal small" type="submit">Save</button>
</form>
<label class="field" style="margin:10px 0 0"><input name="description" value="{desc}" placeholder=" " maxlength="2000" form="rec-{id}"><span>Short description</span></label>
<label class="fchip" style="margin-top:10px"><input type="checkbox" name="one_click_subscribe" value="1"{oc} form="rec-{id}"><span class="ck">{check}</span>One-click subscribe</label>
<form method="post" action="/ghost/settings/growth/recommendations/{id}/" id="rec-{id}"></form>
</div>
<div class="trailing" style="align-items:flex-start">
<form method="post" action="/ghost/settings/growth/recommendations/{id}/delete/" data-confirm="Remove recommendation|&#8220;{title}&#8221; will no longer be recommended to your members.">
<button class="icon-btn danger" title="Remove" aria-label="Remove">{del}</button></form></div></div>"#,
                mark = mark,
                id = esc(&id),
                title = esc(&title),
                url = esc(&url),
                desc = esc(description.as_deref().unwrap_or("")),
                oc = if one_click != 0 { " checked" } else { "" },
                check = svg("check"),
                del = svg("delete"),
            ));
        }
    }
    let rec_add = format!(
        r#"<div class="card padded rise" style="--i:2">
<form method="post" action="/ghost/settings/growth/recommendations/new/" class="inline-form">
<label class="field" style="flex:1;min-width:200px;margin:0"><input name="url" placeholder="https://example.com" maxlength="2000" required><span>URL</span></label>
<label class="field" style="flex:1;min-width:160px;margin:0"><input name="title" placeholder=" " maxlength="191"><span>Title · optional</span></label>
<button class="btn tonal" type="submit">{add} Add recommendation</button>
</form>
<p class="muted" style="margin:12px 0 0">Leave the title blank to use the site's domain. Recommendations appear to members when they subscribe.</p>
</div>"#,
        add = svg("add"),
    );
    let origin = state.site_url();

    // --- signup form builder -----------------------------------------------
    //
    // Server-rendered, so the controls are a GET form that re-renders this
    // page with the snippet rebuilt — there is no client-side state to keep in
    // step with what gets copied, and the preview below is literally the
    // snippet above it running.
    let sf_layout = q.get("sf_layout").cloned().unwrap_or_else(|| "all-in-one".into());
    let sf_layout = if sf_layout == "minimal" { "minimal" } else { "all-in-one" };
    let accent = state
        .settings
        .get_str("accent_color")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "#15171A".into());
    let sf_bg = q.get("sf_bg").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "#F1F3F4".into());
    let sf_btn = q.get("sf_btn").cloned().filter(|c| !c.is_empty()).unwrap_or(accent);
    let sf_labels = q.get("sf_labels").cloned().unwrap_or_default();

    let signup_builder = {
        let site_title = state.settings.get_str("title").unwrap_or_default();
        let site_desc = state.settings.get_str("description").unwrap_or_default();
        let icon = state.settings.get_str("icon").unwrap_or_default();
        let locale = state.settings.get_str("locale").unwrap_or_else(|| "en".into());
        let labels: Vec<&str> =
            sf_labels.split(',').map(str::trim).filter(|l| !l.is_empty()).take(10).collect();

        let snippet = signup_form_snippet(SignupFormEmbed {
            site_url: origin.trim_end_matches('/'),
            layout: sf_layout,
            background: &sf_bg,
            button: &sf_btn,
            title: &site_title,
            description: &site_desc,
            icon: &icon,
            locale: &locale,
            labels: &labels,
            preview: false,
        });
        let preview_snippet = signup_form_snippet(SignupFormEmbed {
            site_url: origin.trim_end_matches('/'),
            layout: sf_layout,
            background: &sf_bg,
            button: &sf_btn,
            title: &site_title,
            description: &site_desc,
            icon: &icon,
            locale: &locale,
            labels: &labels,
            preview: true,
        });

        format!(
            r#"<div class="card padded rise" style="--i:3">
<form method="get" action="/ghost/settings/growth/" class="inline-form" style="align-items:flex-end">
<label class="field fixed" style="min-width:180px;margin:0"><select name="sf_layout" onchange="this.form.submit()">{layout_opts}</select><span>Layout</span>{car}</label>
{bg_field}
<label class="field" style="min-width:140px;margin:0"><input name="sf_btn" type="text" value="{btn}" placeholder=" "><span>Button color</span></label>
<label class="field" style="flex:1;min-width:180px;margin:0"><input name="sf_labels" value="{labels}" placeholder=" "><span>Labels at signup · comma separated</span></label>
<button class="btn tonal" type="submit">Update</button>
</form>
<p class="muted" style="margin:14px 0 8px">Preview — this is the snippet below, running:</p>
<iframe class="sf-preview{minimal}" title="Signup form preview" sandbox="allow-scripts allow-popups allow-forms" srcdoc="{preview_doc}"></iframe>
<p class="muted" style="margin:16px 0 8px">Embed this on any external site to collect members for <b>{origin_disp}</b>:</p>
<div class="key" style="align-items:flex-start"><span class="chip tonal">embed</span><pre style="margin:0;white-space:pre-wrap;word-break:break-all;font:inherit">{snippet_disp}</pre>
<button type="button" data-copy="{snippet_copy}" title="Copy" aria-label="Copy snippet">{copy}</button></div>
<p class="muted" style="margin:12px 0 0">{layout_note} Anyone signing up through this form is tagged with the labels above, so you can tell one embed's members from another's.</p>
</div>"#,
            layout_opts = select_options(
                &[("all-in-one", "All-in-one · title, description and icon"), ("minimal", "Minimal · just the form")],
                sf_layout,
            ),
            car = SELECT_CARET,
            // Ghost only offers a background colour for the all-in-one
            // layout; the minimal one is meant to sit on whatever is behind
            // it, so the embed never sets a background at all.
            bg_field = if sf_layout == "minimal" {
                String::new()
            } else {
                format!(
                    r#"<label class="field" style="min-width:140px;margin:0"><input name="sf_bg" type="text" value="{bg}" placeholder=" "><span>Background color</span></label>"#,
                    bg = esc(&sf_bg),
                )
            },
            btn = esc(&sf_btn),
            labels = esc(&sf_labels),
            origin_disp = esc(&origin),
            minimal = if sf_layout == "minimal" { " minimal" } else { "" },
            // The preview runs the real CDN script against this site, inside
            // a sandboxed frame: same-origin is withheld, so it cannot reach
            // the admin session that rendered it.
            preview_doc = esc(&format!(
                r#"<!doctype html><html><head><meta charset="utf-8"><style>html,body{{margin:0;height:100%}}</style></head><body>{preview_snippet}</body></html>"#,
            )),
            snippet_disp = esc(&snippet),
            snippet_copy = esc(&snippet),
            copy = svg("copy"),
            layout_note = if sf_layout == "minimal" {
                "The minimal layout renders the email field and button alone, for dropping into your own section."
            } else {
                "The all-in-one layout carries your site title, description and icon, taken from Settings → General and Design."
            },
        )
    };


    // --- offers: Ghost only shows these once Stripe is connected, since an
    // offer is a discount on a paid tier and nothing else ---------------------
    let offers_card = if ghost_stripe::StripeConfig::from_settings(&state.settings).is_some() {
        let offers = repos::offers::list(&state.db).await.unwrap_or_default();
        let mut rows = String::new();
        for o in &offers {
            let redemptions = repos::offers::redemption_count(&state.db, &o.id).await;
            rows.push_str(&format!(
                r#"<a class="row" href="/ghost/settings/growth/offers/{id}/"><span class="ic tint-green">{tag}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{summary} · code <code>{code}</code> · {redemptions} redemption{plural}</span></div>
<div class="trailing">{archived}{chev}</div></a>"#,
                id = esc(&o.id),
                tag = svg("tags"),
                name = esc(&o.name),
                summary = esc(&o.summary()),
                code = esc(&o.code),
                redemptions = redemptions,
                plural = if redemptions == 1 { "" } else { "s" },
                archived = if o.active {
                    String::new()
                } else {
                    r#"<span class="chip draft">archived</span>"#.to_string()
                },
                chev = svg("chevron"),
            ));
        }
        if rows.is_empty() {
            rows = r#"<div class="row"><div class="body"><span class="muted">No offers yet — add one to give readers a discount link.</span></div></div>"#.into();
        }
        format!(
            r#"<div class="section">Offers</div>
<div class="card rows rise" style="--i:3">{rows}</div>
<div style="padding:2px 4px 8px"><form method="post" action="/ghost/settings/growth/offers/new/">
<button class="btn tonal small" type="submit">{add} Add offer</button></form></div>
<p class="muted" style="margin:4px 0 0 16px">An offer is a discount on one paid tier, redeemed through a link you share. Archiving one stops new redemptions; members already on it keep their price.</p>"#,
            rows = rows,
            add = svg("add"),
        )
    } else {
        String::new()
    };

    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "growth").unwrap();
    page_for_role(actor_role, 
        "Growth",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/growth/" id="growth-form">
<div class="section">Recommendations</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-slate">{heart}</span>
<div class="body"><span class="title">Enable recommendations</span><span class="detail">Show recommended publications to your members and accept incoming recommendations</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="recommendations_enabled" value="1" {rec_ck}><span class="track"><span class="thumb"></span></span></label></div></div>
</div>
<div class="card rows rise" style="--i:2">{rec_rows}</div>
</form>
{rec_add}
<div class="section">Signup forms</div>
{signup_builder}
{offers_card}
<div class="section">Ghost Explore</div>
<form method="post" action="/ghost/settings/growth/explore/" id="explore-form">
<div class="card rows rise" style="--i:5">{explore_rows}</div>
<div style="padding:10px 4px 0"><button class="btn tonal small" type="submit">Save</button></div>
</form>
<p class="muted" style="margin:10px 2px 0">Ghost Explore is a directory of publications at <a href="https://ghost.org/explore/" target="_blank" rel="noopener">ghost.org/explore</a>.
When enabled, the publication synchronizes public publication details with Explore. Growth statistics are shared only when separately enabled. Private sites and local addresses are excluded.</p>
<div class="section">Social web</div>
<div class="card rows rise" style="--i:6">
{network}
</div>"#,
            head = group_head(group, Some("growth-form")),
            banner = flash_banner(&q),
            offers_card = offers_card,
            heart = svg("star"),
            rec_ck = if state.settings.get_bool("recommendations_enabled") { "checked" } else { "" },
            rec_rows = rec_rows,
            rec_add = rec_add,
            signup_builder = signup_builder,
            network = format!(r#"<a class="row" href="/ghost/social/"><span class="ic tint-teal">{}</span><div class="body"><span class="title">Social web</span><span class="detail">Reader, notes, follows and publication federation</span></div>{}</a>"#,svg("share"),svg("chevron")),
            explore_rows = format!(
                "{}{}",
                toggle_row(
                    "explore_ping",
                    "List this site on Ghost Explore",
                    "Announce the site to Ghost's public directory of publications",
                    state.settings.get_bool("explore_ping"),
                ),
                toggle_row(
                    "explore_ping_growth",
                    "Share growth data to rank higher?",
                    "Uses your revenue and member growth to rank the site more highly. Total member count is shown publicly; the rest is not.",
                    state.settings.get_bool("explore_ping_growth"),
                ),
            ),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn growth_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = form.get("recommendations_enabled").map(|v| v == "1").unwrap_or(false);
    match state.settings.set(&state.db, "recommendations_enabled", &serde_json::Value::Bool(on)).await {
        Ok(()) => settings_flash("growth", "ok", "Growth settings saved."),
        Err(e) => settings_flash("growth", "error", &e.message()),
    }
}

/// Ghost Explore's two opt-ins, saved on their own so they cannot clear the
/// recommendations switch that lives in a different form on the same page.
pub(crate) async fn growth_explore_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut ok = true;
    for key in ["explore_ping", "explore_ping_growth"] {
        let on = form.get(key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(on)).await.is_ok();
    }
    if ok {
        settings_flash("growth", "ok", "Ghost Explore settings saved.")
    } else {
        settings_flash("growth", "error", "Saving failed — check the logs.")
    }
}

// --- recommendations ---------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RecommendationForm {
    url: String,
    title: String,
    description: String,
    one_click_subscribe: String,
}

/// Normalise a recommended site's URL, and refuse one that is not a site.
///
/// A bare domain gets `https://`, because that is what someone types.
pub(crate) fn recommendation_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let with_scheme = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    // A host with a dot in it is the bar: `https://localhost` is a typo here,
    // not a publication worth recommending to members.
    let host = with_scheme.split("//").nth(1)?.split('/').next()?;
    (host.contains('.') && host.len() > 3).then_some(with_scheme)
}

/// Fall back to the domain, the way Ghost titles a recommendation it could
/// not fetch a name for.
pub(crate) fn recommendation_title(title: &str, url: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .map(|h| h.trim_start_matches("www.").to_string())
        .unwrap_or_else(|| "Recommended site".into())
}

/// What a candidate publication says about itself, for the badge.
#[derive(Debug, Default)]
pub(crate) struct FetchedRecommendation {
    title: Option<String>,
    description: Option<String>,
    excerpt: Option<String>,
    featured_image: Option<String>,
    favicon: Option<String>,
    one_click_subscribe: bool,
}

/// Fetch a recommended site's own metadata, failing soft.
///
/// This fetches a URL the publisher chose, so it goes through the same
/// `fetchable` guard the webmention receiver uses — an admin must not be able
/// to aim it at the host's private network — and gives up quickly rather than
/// holding the form submission open.
pub(crate) async fn fetch_recommendation_metadata(url: &str) -> FetchedRecommendation {
    let Some(parsed) = ghost_api::mentions::fetchable(url) else {
        return FetchedRecommendation::default();
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        // Every hop re-checked, not just the URL the publisher chose: a plain
        // redirect limit would let a public URL hand this fetch an address
        // inside the host's own network.
        .redirect(ghost_api::mentions::guarded_redirect(3))
        .build()
    else {
        return FetchedRecommendation::default();
    };
    let Ok(response) = client.get(parsed).send().await else {
        return FetchedRecommendation::default();
    };
    if !response.status().is_success() {
        return FetchedRecommendation::default();
    }
    let html = response.text().await.unwrap_or_default();
    let meta = ghost_api::mentions::metadata_of(&html);
    FetchedRecommendation {
        title: meta.site_title.or(meta.title),
        description: meta.excerpt.clone(),
        excerpt: meta.excerpt,
        featured_image: meta.image,
        favicon: meta.favicon,
        // Whether they run Portal, which is what makes one-click subscribe work.
        one_click_subscribe: html.contains("/members/api/") || html.contains("data-portal"),
    }
}

pub(crate) async fn recommendation_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<RecommendationForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(url) = recommendation_url(&form.url) else {
        return settings_flash("growth", "error", "Enter the full address of the site to recommend.");
    };
    // `url` is unique in Ghost — recommending the same site twice says nothing
    // the first one did not.
    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recommendations WHERE url = ?")
        .bind(&url)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    if taken > 0 {
        return settings_flash("growth", "error", "That site is already recommended.");
    }
    // Read the site's own name, description, social image and favicon before
    // saving, the way Ghost's add dialog does. Without these the member-facing
    // badge is a bare title, so a site that cannot be reached is still saved —
    // just with the domain as its title.
    let fetched = fetch_recommendation_metadata(&url).await;
    let now = ghost_core::dates::now_db();
    let res = sqlx::query(
        "INSERT INTO recommendations (id, url, title, description, excerpt, featured_image, favicon, \
           one_click_subscribe, featured, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(&url)
    .bind(recommendation_title(
        if form.title.trim().is_empty() { fetched.title.as_deref().unwrap_or("") } else { &form.title },
        &url,
    ))
    .bind(fetched.description.clone().unwrap_or_default())
    .bind(fetched.excerpt.clone())
    .bind(fetched.featured_image.clone())
    .bind(fetched.favicon.clone())
    .bind(fetched.one_click_subscribe)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    match res {
        Ok(_) => {
            // Tell them they have been recommended, by sending a webmention
            // between the two sites' recommendation files. That is how the
            // other end's "who recommends me" screen learns about it.
            ghost_api::mentions::announce_recommendation(&state, url.clone());
            settings_flash("growth", "ok", "Recommendation added.")
        }
        Err(e) => settings_flash("growth", "error", &format!("Could not add it: {e}")),
    }
}

pub(crate) async fn recommendation_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<RecommendationForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // The row carries two forms — one for title/url, one for the description
    // and the subscribe flag — so a field the submitted form did not include
    // keeps its stored value rather than being blanked.
    // Booleans are 0/1 integers; `Any` does not decode one as a `bool`.
    let existing: Option<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT url, title, description, one_click_subscribe FROM recommendations WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((old_url, old_title, old_desc, old_oc)) = existing else {
        return settings_flash("growth", "error", "That recommendation no longer exists.");
    };

    let url = if form.url.trim().is_empty() {
        old_url
    } else {
        match recommendation_url(&form.url) {
            Some(u) => u,
            None => return settings_flash("growth", "error", "That does not look like a site address."),
        }
    };
    let title = if form.title.trim().is_empty() {
        old_title
    } else {
        recommendation_title(&form.title, &url)
    };
    // Only the form that carries the description may change it.
    let description = if form.description.trim().is_empty() && form.one_click_subscribe.is_empty() {
        old_desc.unwrap_or_default()
    } else {
        form.description.trim().to_string()
    };
    let one_click = if form.description.trim().is_empty() && form.one_click_subscribe.is_empty() {
        old_oc != 0
    } else {
        form.one_click_subscribe == "1"
    };

    let res = sqlx::query(
        "UPDATE recommendations SET url = ?, title = ?, description = ?, one_click_subscribe = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&url)
    .bind(&title)
    .bind(&description)
    .bind(one_click)
    .bind(ghost_core::dates::now_db())
    .bind(&id)
    .execute(&state.db)
    .await;
    match res {
        Ok(_) => settings_flash("growth", "ok", "Recommendation saved."),
        Err(e) => settings_flash("growth", "error", &format!("Could not save it: {e}")),
    }
}

pub(crate) async fn recommendation_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let _ = sqlx::query("DELETE FROM recommendations WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await;
    settings_flash("growth", "ok", "Recommendation removed.")
}

// --- offers -----------------------------------------------------------------

pub(crate) fn offer_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("/ghost/settings/growth/offers/{id}/?{encoded}")).into_response()
}

/// Ghost's own slugification of an offer code: lower case, spaces to dashes,
/// nothing else — the code ends up in a URL.
pub(crate) fn offer_code(raw: &str) -> String {
    ghost_core::slug::safe_string(raw)
}

/// Create a blank offer and open it, the way "Add tier" works.
///
/// A new offer starts archived so a half-filled code is never live, and
/// carries no Stripe coupon until its terms are saved.
pub(crate) async fn offer_new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // An offer discounts a tier, so there has to be a paid one to discount.
    let tier: Option<String> = sqlx::query_scalar(
        "SELECT id FROM products WHERE type = 'paid' AND active = 1 ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(tier) = tier else {
        return settings_flash(
            "growth",
            "error",
            "Add a paid tier first — an offer is a discount on one.",
        );
    };

    // Unique by construction: the name and code both carry a short suffix, so
    // adding several in a row never collides.
    let suffix = ghost_core::ids::random_below(100_000);
    let input = repos::offers::OfferInput {
        name: format!("New offer {suffix}"),
        code: format!("offer-{suffix}"),
        product_id: tier,
        interval: "month".into(),
        discount_type: "percent".into(),
        discount_amount: 10,
        duration: "once".into(),
        duration_in_months: None,
        currency: None,
        portal_title: "Special offer".into(),
        portal_description: String::new(),
        stripe_coupon_id: None,
        active: false,
    };
    match repos::offers::create(&state.db, &input).await {
        Ok(o) => Redirect::to(&format!("/ghost/settings/growth/offers/{}/", o.id)).into_response(),
        Err(e) => settings_flash("growth", "error", &e.message()),
    }
}

pub(crate) async fn offer_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(offer)) = repos::offers::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That offer could not be found.");
    };
    let redemptions = repos::offers::redemption_count(&state.db, &id).await;
    // Terms are frozen once anyone has redeemed: the discount lives in a
    // Stripe coupon their subscription already carries, so changing it here
    // would describe something different from what they are being charged.
    let locked = redemptions > 0;

    let tiers: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM products WHERE type = 'paid' ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let tier_opts: String = tiers
        .iter()
        .map(|(tid, name)| {
            format!(
                r#"<option value="{v}"{sel}>{n}</option>"#,
                v = esc(tid),
                n = esc(name),
                sel = if *tid == offer.product_id { " selected" } else { "" },
            )
        })
        .collect();

    let site_url = state.site_url();
    let share = format!("{}/{}", site_url.trim_end_matches('/'), offer.code);

    let dis = if locked { " disabled" } else { "" };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "growth").unwrap();
    let _ = group;

    page_for_role(actor_role,
        "Offer",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/growth/" title="Back to growth">{back}</a>
<div><h1>{name}</h1><p class="sub">{summary} · {status}</p></div>
<div class="actions"><button class="btn" type="submit" form="offer-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/settings/growth/offers/{id}/" id="offer-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{tag}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " maxlength="191" required><span>Internal name · only you see this</span></label>
<label class="field" style="margin:0"><input name="code" value="{code}" placeholder=" " maxlength="191" required><span>Code · the end of the share link</span></label>
</div>
<p class="muted" style="margin:10px 0 0">Share link: <a href="{share}" target="_blank" rel="noopener">{share}</a></p>
</div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{notes}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><input name="portal_title" value="{portal_title}" placeholder=" " maxlength="191"><span>Display title · shown in the portal</span></label>
<label class="field" style="margin:12px 0 0"><textarea name="portal_description" rows="2" placeholder=" " maxlength="2000">{portal_description}</textarea><span>Display description</span></label>
</div></div>
</div>
<div class="section">Discount</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{coin}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field fixed" style="margin:0"><select name="product_id"{dis}>{tier_opts}</select><span>Tier</span>{car}</label>
<label class="field fixed" style="margin:0"><select name="interval"{dis}>{cadence_opts}</select><span>Cadence</span>{car}</label>
<label class="field fixed" style="margin:0"><select name="discount_type"{dis}>{type_opts}</select><span>Type</span>{car}</label>
<label class="field" style="margin:0"><input name="discount_amount" type="number" min="1" step="1" value="{amount}" placeholder=" "{dis}><span>{amount_label}</span></label>
<label class="field fixed" style="margin:0"><select name="duration"{dis}>{duration_opts}</select><span>Duration</span>{car}</label>
<label class="field" style="margin:0"><input name="duration_in_months" type="number" min="1" step="1" value="{months}" placeholder=" "{dis}><span>Months · repeating only</span></label>
</div>
<p class="muted" style="margin:10px 0 0">{terms_note}</p>
</div></div>
</div>
</form>
<div class="section">Status</div>
<div class="card rows rise" style="--i:3">
<div class="row"><span class="ic tint-amber">{flag}</span>
<div class="body"><span class="title">{status_title}</span><span class="detail">{status_detail}</span></div>
<div class="trailing"><form method="post" action="/ghost/settings/growth/offers/{id}/status/">
<input type="hidden" name="active" value="{next}">
<button class="btn tonal small" type="submit">{status_btn}</button></form></div></div>
<div class="row"><span class="ic tint-slate">{people}</span>
<div class="body"><span class="title">Redemptions</span><span class="detail">{redemptions} member{plural} signed up through this offer</span></div></div>
</div>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&id),
            name = esc(&offer.name),
            name_v = esc(&offer.name),
            summary = esc(&offer.summary()),
            status = offer.status(),
            code = esc(&offer.code),
            share = esc(&share),
            tag = svg("tags"),
            notes = svg("notes"),
            coin = svg("ghost"),
            flag = svg("flag"),
            people = svg("person"),
            car = SELECT_CARET,
            dis = dis,
            tier_opts = tier_opts,
            cadence_opts = select_options(
                &[("month", "Monthly"), ("year", "Yearly")],
                offer.interval.as_deref().unwrap_or("month"),
            ),
            type_opts = select_options(
                &[("percent", "Percentage off"), ("fixed", "Amount off"), ("trial", "Free trial")],
                offer.discount_type.as_deref().unwrap_or("percent"),
            ),
            amount = offer.discount_amount.unwrap_or(0),
            amount_label = match offer.discount_type.as_deref() {
                Some("trial") => "Trial days",
                Some("fixed") => "Amount off · in cents",
                _ => "Percent off · 1-100",
            },
            duration_opts = select_options(
                &[("once", "Once"), ("repeating", "Repeating"), ("forever", "Forever")],
                offer.duration.as_deref().unwrap_or("once"),
            ),
            months = offer.duration_in_months.map(|m| m.to_string()).unwrap_or_default(),
            portal_title = esc(offer.portal_title.as_deref().unwrap_or("")),
            portal_description = esc(offer.portal_description.as_deref().unwrap_or("")),
            terms_note = if locked {
                "Someone has already redeemed this offer, so its terms are fixed — their subscription is priced by the Stripe coupon behind it. Create a new offer to change the discount."
            } else {
                "A free trial ignores the duration. Saving mints a fresh Stripe coupon for these terms."
            },
            status_title = if offer.active { "Active" } else { "Archived" },
            status_detail = if offer.active {
                "The share link works and new members can redeem it."
            } else {
                "The share link no longer redeems. Members already on it keep their price."
            },
            next = if offer.active { "0" } else { "1" },
            status_btn = if offer.active { "Archive" } else { "Activate" },
            redemptions = redemptions,
            plural = if redemptions == 1 { "" } else { "s" },
        ),
        true,
    )
    .into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct OfferForm {
    name: String,
    code: String,
    product_id: String,
    interval: String,
    discount_type: String,
    discount_amount: String,
    duration: String,
    duration_in_months: String,
    portal_title: String,
    portal_description: String,
}

pub(crate) async fn offer_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<OfferForm>,
) -> Response {
    let user = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a.user,
        Err(r) => return r,
    };
    let Ok(Some(existing)) = repos::offers::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That offer could not be found.");
    };

    let name = form.name.trim().to_string();
    if name.is_empty() {
        return offer_flash(&id, "error", "An offer needs a name.");
    }
    let code = offer_code(&form.code);
    if code.is_empty() {
        return offer_flash(&id, "error", "An offer needs a code — it becomes the share link.");
    }
    // Both are unique in Ghost, and the code is a public URL, so a clash is
    // reported rather than left to the database constraint.
    match repos::offers::conflict(&state.db, &name, &code, Some(&id)).await {
        Ok(Some("name")) => return offer_flash(&id, "error", "Another offer already uses that name."),
        Ok(Some(_)) => return offer_flash(&id, "error", "Another offer already uses that code."),
        Err(e) => return offer_flash(&id, "error", &e.message()),
        Ok(None) => {}
    }

    let locked = repos::offers::redemption_count(&state.db, &id).await > 0;
    let mut input = repos::offers::OfferInput {
        name,
        code,
        portal_title: form.portal_title.trim().to_string(),
        portal_description: form.portal_description.trim().to_string(),
        active: existing.active,
        // Carried over; only replaced below when a new coupon is minted.
        stripe_coupon_id: None,
        product_id: existing.product_id.clone(),
        interval: existing.interval.clone().unwrap_or_else(|| "month".into()),
        discount_type: existing.discount_type.clone().unwrap_or_else(|| "percent".into()),
        discount_amount: existing.discount_amount.unwrap_or(0),
        duration: existing.duration.clone().unwrap_or_else(|| "once".into()),
        duration_in_months: existing.duration_in_months,
        currency: existing.currency.clone(),
    };

    if locked {
        // Presentation only — the terms inputs were disabled on the page.
        return match repos::offers::update(&state.db, &id, &input).await {
            Ok(_) => offer_flash(&id, "ok", "Offer saved."),
            Err(e) => offer_flash(&id, "error", &e.message()),
        };
    }

    let one_of = |v: &str, allowed: &[&str], fallback: &str| {
        if allowed.contains(&v) { v.to_string() } else { fallback.to_string() }
    };
    input.product_id = form.product_id.trim().to_string();
    input.interval = one_of(form.interval.trim(), &["month", "year"], "month");
    input.discount_type = one_of(form.discount_type.trim(), &["percent", "fixed", "trial"], "percent");
    input.duration = one_of(form.duration.trim(), &["once", "repeating", "forever"], "once");
    let amount: i64 = form.discount_amount.trim().parse().unwrap_or(0);
    input.duration_in_months = form.duration_in_months.trim().parse::<i64>().ok().filter(|m| *m > 0);

    // The tier has to exist and be paid — the select is built from that list,
    // but the form is a POST like any other.
    let tier: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, currency FROM products WHERE id = ? AND type = 'paid' LIMIT 1",
    )
    .bind(&input.product_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((_, tier_currency)) = tier else {
        return offer_flash(&id, "error", "Choose a paid tier for this offer.");
    };

    match input.discount_type.as_str() {
        "percent" if !(1..=100).contains(&amount) => {
            return offer_flash(&id, "error", "A percentage discount must be between 1 and 100.")
        }
        "trial" if amount < 1 => {
            return offer_flash(&id, "error", "A free trial needs at least one day.")
        }
        "fixed" if amount < 1 => {
            return offer_flash(&id, "error", "An amount discount must be more than zero.")
        }
        _ => {}
    }
    if input.discount_type == "repeating" && input.duration_in_months.is_none() {
        return offer_flash(&id, "error", "A repeating offer needs a number of months.");
    }
    input.discount_amount = amount;

    // A trial is not a coupon at all: Stripe applies it as the subscription's
    // trial period, so the duration is its own thing.
    if input.discount_type == "trial" {
        input.duration = "trial".into();
        input.duration_in_months = None;
        input.currency = None;
    } else if input.discount_type == "fixed" {
        // Stripe requires a currency on an amount-off coupon, and it has to be
        // the one the tier charges in or the discount cannot apply.
        input.currency = tier_currency.clone().or_else(|| Some("usd".into()));
    } else {
        input.currency = None;
    }
    if input.duration != "repeating" {
        input.duration_in_months = None;
    }

    // Mint a coupon for the new terms. Stripe coupons cannot be edited, so a
    // changed discount is always a new one; the old is left in place for the
    // subscriptions already carrying it.
    if input.discount_type != "trial" {
        let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
            return offer_flash(&id, "error", "Connect Stripe before saving an offer's discount.");
        };
        let discount = if input.discount_type == "fixed" {
            ghost_stripe::Discount::Fixed {
                amount_cents: input.discount_amount,
                currency: input.currency.as_deref().unwrap_or("usd"),
            }
        } else {
            ghost_stripe::Discount::Percent(input.discount_amount)
        };
        match ghost_stripe::create_coupon(
            &cfg,
            &input.name,
            discount,
            &input.duration,
            input.duration_in_months,
        )
        .await
        {
            Ok(coupon) => input.stripe_coupon_id = Some(coupon),
            Err(e) => return offer_flash(&id, "error", &format!("Stripe: {}", e.message())),
        }
    }

    // The terms live outside `update`, which only touches presentation, so
    // they are written here in one statement alongside it.
    let now = ghost_core::dates::now_db();
    let wrote = sqlx::query(
        "UPDATE offers SET product_id = ?, `interval` = ?, discount_type = ?, discount_amount = ?, \
         duration = ?, duration_in_months = ?, currency = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&input.product_id)
    .bind(&input.interval)
    .bind(&input.discount_type)
    .bind(input.discount_amount)
    .bind(&input.duration)
    .bind(input.duration_in_months)
    .bind(&input.currency)
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await
    .is_ok();
    if !wrote {
        return offer_flash(&id, "error", "Saving the discount failed — check the logs.");
    }
    match repos::offers::update(&state.db, &id, &input).await {
        Ok(_) => offer_flash(&id, "ok", "Offer saved."),
        Err(e) => offer_flash(&id, "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct OfferStatusForm {
    active: String,
}

pub(crate) async fn offer_set_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<OfferStatusForm>,
) -> Response {
    let user = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a.user,
        Err(r) => return r,
    };
    let Ok(Some(offer)) = repos::offers::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That offer could not be found.");
    };
    let active = form.active == "1";
    // An offer with no coupon has never had its terms saved, so there is
    // nothing for a redeemer to actually get.
    if active && offer.discount_type.as_deref() != Some("trial") && offer.stripe_coupon_id.is_none()
    {
        return offer_flash(&id, "error", "Save the discount first — this offer has no Stripe coupon yet.");
    }
    match repos::offers::set_active(&state.db, &id, active).await {
        Ok(()) => offer_flash(&id, "ok", if active { "Offer activated." } else { "Offer archived." }),
        Err(e) => offer_flash(&id, "error", &e.message()),
    }
}
