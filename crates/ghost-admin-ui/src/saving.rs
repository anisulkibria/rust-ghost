//! Saving a post or page: validation, scheduling, publishing, and sending it as a newsletter.

use super::*;

// ---------------------------------------------------------------------------
// saving
// ---------------------------------------------------------------------------

#[derive(Default, Debug, PartialEq)]
pub(crate) struct EditorForm {
    pub(crate) title: String,
    pub(crate) slug: Option<String>,
    /// Koenig's editor state, posted by the bridge script.
    pub(crate) lexical: Option<String>,
    /// The HTML Koenig rendered from that state in the browser. The cards
    /// know their own markup, so this is the authority; the server only
    /// renders the Lexical itself when this is missing.
    pub(crate) html: Option<String>,
    /// Only posted when the rich editor is not running.
    pub(crate) markdown: Option<String>,
    pub(crate) status: String,
    pub(crate) published_at: Option<String>,
    pub(crate) visibility: Option<String>,
    pub(crate) featured: bool,
    pub(crate) show_title_and_feature_image: bool,
    pub(crate) feature_image: Option<String>,
    pub(crate) feature_image_alt: Option<String>,
    pub(crate) feature_image_caption: Option<String>,
    pub(crate) custom_excerpt: Option<String>,
    /// One entry per ticked checkbox — they all post under the same name.
    pub(crate) tags: Vec<String>,
    pub(crate) authors: Vec<String>,
    pub(crate) tiers: Vec<String>,
    pub(crate) tiers_present: bool,
    pub(crate) custom_template: Option<String>,
    pub(crate) canonical_url: Option<String>,
    pub(crate) codeinjection_head: Option<String>,
    pub(crate) codeinjection_foot: Option<String>,
    pub(crate) meta_title: Option<String>,
    pub(crate) meta_description: Option<String>,
    pub(crate) og_title: Option<String>,
    pub(crate) og_description: Option<String>,
    pub(crate) og_image: Option<String>,
    pub(crate) twitter_title: Option<String>,
    pub(crate) twitter_description: Option<String>,
    pub(crate) twitter_image: Option<String>,
    pub(crate) email_subject: Option<String>,
    /// Used only by the transient preview's explicit test-send action.
    pub(crate) preview_email: Option<String>,
    /// `publish_only` | `publish_email` | `email_only`.
    pub(crate) publish_action: String,
    pub(crate) audience: ghost_api::newsletter_audience::Audience,
    pub(crate) audience_present: bool,
    /// The post's `updated_at` as it stood when this editor loaded it.
    ///
    /// Carried so a save can be refused when someone else has saved since —
    /// see [`ghost_api::collision`]. Absent on a hand-made request, which
    /// skips the check the same way Ghost's does.
    pub(crate) updated_at: Option<String>,
}

impl EditorForm {
    /// Parse an `application/x-www-form-urlencoded` body.
    ///
    /// `axum::Form` cannot be used here: it goes through `serde_urlencoded`,
    /// which has no notion of a repeated key. The tag and author checkboxes
    /// each post under one name, so a single ticked box fails to deserialize
    /// as a sequence at all and several would collapse to one value.
    pub(crate) fn parse(body: &str) -> Self {
        let mut form = EditorForm::default();
        let keep = |v: String| Some(v).filter(|s| !s.trim().is_empty());

        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            let value = value.into_owned();
            match key.as_ref() {
                "title" => form.title = value.trim().to_string(),
                "slug" => form.slug = keep(value),
                "lexical" => form.lexical = keep(value),
                "html" => form.html = keep(value),
                "markdown" => form.markdown = Some(value),
                "status" => form.status = value,
                "published_at" => form.published_at = from_datetime_local(&value),
                "visibility" => form.visibility = keep(value),
                "publish_action" => form.publish_action = value,
                "newsletter_id" => {form.audience.newsletter_id=keep(value);form.audience_present=true;},
                "audience_filter" => {form.audience.filter=keep(value);form.audience_present=true;},
                "newsletter_include_preview" => form.audience.include_preview=value=="1",
                "_audience" => form.audience_present=true,
                "featured" => form.featured = true,
                "show_title_and_feature_image" => form.show_title_and_feature_image = true,
                "feature_image" => form.feature_image = keep(value),
                "feature_image_alt" => form.feature_image_alt = keep(value),
                "feature_image_caption" => form.feature_image_caption = keep(value),
                "custom_excerpt" => form.custom_excerpt = keep(value),
                "custom_template" => form.custom_template = keep(value),
                "canonical_url" => form.canonical_url = keep(value),
                "codeinjection_head" => form.codeinjection_head = keep(value),
                "codeinjection_foot" => form.codeinjection_foot = keep(value),
                "meta_title" => form.meta_title = keep(value),
                "meta_description" => form.meta_description = keep(value),
                "og_title" => form.og_title = keep(value),
                "og_description" => form.og_description = keep(value),
                "og_image" => form.og_image = keep(value),
                "twitter_title" => form.twitter_title = keep(value),
                "twitter_description" => form.twitter_description = keep(value),
                "twitter_image" => form.twitter_image = keep(value),
                "email_subject" => form.email_subject = keep(value),
                "preview_email" => form.preview_email = keep(value),
                "updated_at" => form.updated_at = keep(value),
                "tags" => {
                    if !value.trim().is_empty() {
                        form.tags.push(value);
                    }
                }
                "_tiers" => form.tiers_present = true,
                "tiers" => { form.tiers_present = true; if !value.trim().is_empty() { form.tiers.push(value); } },
                "authors" => {
                    if !value.trim().is_empty() {
                        form.authors.push(value);
                    }
                }
                _ => {}
            }
        }
        form
    }

    /// (html, plaintext, reading time) for whichever editor was used.
    ///
    /// Koenig's own HTML is preferred because the cards render themselves;
    /// the Rust renderer covers a document that arrived without it.
    pub(crate) fn rendered(&self) -> (String, String, i64) {
        let html = match (&self.html, &self.lexical) {
            (Some(html), _) => html.clone(),
            (None, Some(lexical)) => ghost_render::lexical::to_html(lexical),
            (None, None) => ghost_render::markdown::to_html(self.markdown.as_deref().unwrap_or("")),
        };
        let plaintext = ghost_render::markdown::html_to_plaintext(&html);
        let reading_time = ghost_render::markdown::reading_time(&plaintext);
        (html, plaintext, reading_time)
    }

    /// Whether the rich editor was the one that posted this.
    pub(crate) fn is_koenig(&self) -> bool {
        self.lexical.is_some()
    }

    /// Whether this request carries the body of the post at all.
    ///
    /// It might not: the hidden fields are filled in by the editor as the
    /// form is submitted, so a save that somehow beats Koenig's startup —
    /// or a hand-made request — arrives with no content in it. Rendering
    /// that would write an empty post over a real one, so the save leaves
    /// the content columns alone instead and only applies the settings.
    pub(crate) fn has_content(&self) -> bool {
        self.lexical.is_some() || self.html.is_some() || self.markdown.is_some()
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for EditorForm {
    type Rejection = Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body = String::from_utf8_lossy(&bytes);
        Ok(EditorForm::parse(&body))
    }
}

pub(crate) fn form_to_new(form: &EditorForm, post_type: &str) -> repos::posts::NewPost {
    let (html, plaintext, reading_time) = form.rendered();
    repos::posts::NewPost {
        title: form.title.clone(),
        slug: form.slug.clone(),
        lexical: form.lexical.clone(),
        // A Koenig post has no markdown source, and storing the old field
        // alongside it would leave two disagreeing copies of the content.
        markdown: if form.is_koenig() { None } else { form.markdown.clone() },
        html: Some(html),
        plaintext: Some(plaintext),
        custom_excerpt: form.custom_excerpt.clone(),
        feature_image: form.feature_image.clone(),
        featured: form.featured,
        post_type: post_type.to_string(),
        status: form.status.clone(),
        visibility: form.visibility.clone().unwrap_or_else(|| "public".into()),
        published_at: form.published_at.clone(),
        codeinjection_head: form.codeinjection_head.clone(),
        codeinjection_foot: form.codeinjection_foot.clone(),
        custom_template: form.custom_template.clone(),
        canonical_url: form.canonical_url.clone(),
        show_title_and_feature_image: Some(form.show_title_and_feature_image),
        meta_title: form.meta_title.clone(),
        meta_description: form.meta_description.clone(),
        og_title: form.og_title.clone(),
        og_description: form.og_description.clone(),
        og_image: form.og_image.clone(),
        twitter_title: form.twitter_title.clone(),
        twitter_description: form.twitter_description.clone(),
        twitter_image: form.twitter_image.clone(),
        email_subject: form.email_subject.clone(),
        feature_image_alt: form.feature_image_alt.clone(),
        feature_image_caption: form.feature_image_caption.clone(),
        author_ids: form.authors.clone(),
        tag_ids: form.tags.clone(),
        tier_ids: form.tiers.clone(),
        reading_time,
    }
}

/// The columns the editor form owns. Listing them lets `update` tell "the
/// author emptied this box" from "this request never mentioned the field",
/// so clearing a meta title actually clears it.
pub(crate) fn editor_owned_fields(is_koenig: bool, has_content: bool) -> Vec<&'static str> {
    let mut fields = vec![
        "custom_excerpt",
        "feature_image",
        "codeinjection_head",
        "codeinjection_foot",
        "custom_template",
        "canonical_url",
        "meta_title",
        "meta_description",
        "og_title",
        "og_description",
        "og_image",
        "twitter_title",
        "twitter_description",
        "twitter_image",
        "email_subject",
        "feature_image_alt",
        "feature_image_caption",
    ];
    if has_content {
        fields.push("html");
        fields.push("plaintext");
        // Only the editor that posted may blank the other's source, so a
        // save from the Markdown fallback cannot wipe a Koenig document, or
        // vice versa — and a save carrying no body at all clears neither.
        fields.push(if is_koenig { "markdown" } else { "lexical" });
    }
    fields
}

/// What a save that asks for "Scheduled" actually amounts to.
pub(crate) enum Scheduling {
    /// Not a scheduled save; nothing to arrange.
    No,
    /// Hold it until this moment, then deliver it this way.
    At(repos::posts::Delivery),
    /// Scheduled to a date that has already passed, or to none at all.
    Undated,
}

/// Read the schedule off the form.
///
/// A date in the past is treated as no date rather than as "publish now": the
/// author picked a moment, and silently deciding they meant *this* moment is
/// how a half-written post ends up emailed to everyone.
pub(crate) fn scheduling(form: &EditorForm, post_type: &str, action: PublishAction) -> Scheduling {
    if form.status != "scheduled" {
        return Scheduling::No;
    }
    // `YYYY-MM-DD HH:MM:SS`, so later times sort later as text — no date
    // library needed to tell the future from the past.
    let future = form
        .published_at
        .as_deref()
        .is_some_and(|at| at.len() >= 19 && at > ghost_core::dates::now_db().as_str());
    if !future {
        return Scheduling::Undated;
    }
    // Pages have no newsletter; a page scheduled "and email" still only
    // appears on the site.
    let sends = post_type == "post" && action.sends_email();
    Scheduling::At(repos::posts::Delivery {
        email: sends,
        email_only: sends && action == PublishAction::EmailOnly,
    })
}

/// What the author is told when a schedule has no usable date. The post is
/// still saved — as a draft, with every word in it — because losing the
/// writing to a mis-set date would be the worse failure by far.
pub(crate) const UNDATED: &str = "Saved as a draft: a scheduled post needs a publish date in the future.";

/// True when a save came from the editor's own script, which asks for JSON —
/// a redirect would be followed to a full editor page it has no use for.
pub(crate) fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("application/json"))
}

/// What a save answers to the editor's script: the fields it has to adopt —
/// a first save creates the post, and the page then has to behave as though
/// it had always been `/posts/<id>/` — plus anything worth a banner.
pub(crate) async fn editor_saved_json(state: &AppState, post: &ghost_core::models::Post, message: &str, level: &str) -> Response {
    let url = if post.status == "published" {
        let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(post)).await.unwrap_or_default();
        let data = ghost_core::models::post_to_json(post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
        let router = state.router.read().await;
        let path = router.permalink_for(&data)
            .map(|(_, permalink)| ghost_routing::tree::build_permalink(&permalink, &data))
            .unwrap_or_else(|| format!("/{}/", post.slug.as_deref().unwrap_or_default()));
        Some(format!("{}{path}", state.site_url().trim_end_matches('/')))
    } else {
        None
    };
    let body = serde_json::json!({
        "id": post.id,
        "slug": post.slug,
        "status": post.status,
        "title": post.title,
        "url": url,
        "message": message,
        "level": level,
        // The stamp this save produced. The editor adopts it so the next save
        // compares against what it just wrote rather than against the version
        // it originally loaded, which would collide with itself.
        "updated_at": post.updated_at,
    });
    ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

/// A save the script made that the server refused, worded for the banner.
pub(crate) fn editor_error_json(message: &str, status: StatusCode) -> Response {
    let body = serde_json::json!({"message": message, "level": "error"});
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Refuse a save that would overwrite a newer edit from another session.
///
/// Returns the 409 to send, or `None` when this save is safe. The comparison
/// itself is [`ghost_api::collision::collides`], so the native editor and the
/// Admin API apply one rule. The submitted object is built from the fields
/// this form actually carried: a settings-only save does not claim to be
/// editing the body, and so cannot collide over it.
pub(crate) fn editor_collision(form: &EditorForm, current: &ghost_core::models::Post) -> Option<Response> {
    let submitted_stamp = form.updated_at.as_deref()?;
    let mut submitted = serde_json::Map::new();
    submitted.insert("updated_at".into(), serde_json::json!(submitted_stamp));
    submitted.insert("title".into(), serde_json::json!(form.title));
    if let Some(lexical) = &form.lexical {
        submitted.insert("lexical".into(), serde_json::json!(lexical));
    }
    if let Some(markdown) = &form.markdown {
        submitted.insert("markdown".into(), serde_json::json!(markdown));
    }
    for (key, value) in [
        ("slug", &form.slug),
        ("custom_excerpt", &form.custom_excerpt),
        ("feature_image", &form.feature_image),
        ("canonical_url", &form.canonical_url),
    ] {
        if let Some(value) = value {
            submitted.insert(key.into(), serde_json::json!(value));
        }
    }
    if !form.status.is_empty() {
        submitted.insert("status".into(), serde_json::json!(form.status));
    }
    submitted.insert("featured".into(), serde_json::json!(form.featured));

    let stored = serde_json::json!({
        "updated_at": current.updated_at,
        "title": current.title,
        "lexical": current.lexical,
        "markdown": current.markdown,
        "slug": current.slug,
        "custom_excerpt": current.custom_excerpt,
        "feature_image": current.feature_image,
        "canonical_url": current.canonical_url,
        "status": current.status,
        "featured": current.featured,
    });

    ghost_api::collision::collides(&serde_json::Value::Object(submitted), &stored).then(|| {
        editor_error_json(ghost_api::collision::MESSAGE, StatusCode::CONFLICT)
    })
}

pub(crate) async fn editor_create_impl(state: &AppState, headers: &HeaderMap, mut form: EditorForm, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !actor.role.can_publish() {
        form.status = "draft".into();
    }
    let user = actor.user;
    if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::validate(state,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
    let action = PublishAction::parse(&form.publish_action);
    let schedule = scheduling(&form, post_type, action);
    if matches!(schedule, Scheduling::Undated) {
        form.status = "draft".into();
    }
    // A scheduled post sends nothing now — that is the whole point of it.
    let going_out = !matches!(schedule, Scheduling::At(_))
        && post_type == "post"
        && action.sends_email()
        && (form.status == "published" || action == PublishAction::EmailOnly);
    let mut new = form_to_new(&form, post_type);
    // `status_for` turns an email-only post into Ghost's `sent`, which is
    // where it lands when it actually goes out, not while it waits.
    new.status = match schedule {
        Scheduling::At(_) => "scheduled".to_string(),
        _ => action.status_for(&form.status),
    };
    let json_mode = wants_json(headers);
    match repos::posts::insert(&state.db, &new, &user.id).await {
        Ok(post) => {
            if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::save(state,&post.id,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
            let _ = repos::actions::record(&state.db, post_type, &post.id, &user.id, "added").await;
            fire_post_webhooks(state, post_type, &post, "added", false);
            if post.status == "published" {
                notify_slack_published(state, &post).await;
                announce_published(state, &post).await;
            }
            if let Scheduling::At(delivery) = schedule {
                let _ = repos::posts::set_delivery(&state.db, &post.id, delivery).await;
            }
            if going_out {
                return match queue_newsletter_send(state, &post.id).await {
                    Ok(n) => {
                        // The send is under way rather than finished: the
                        // count is who it is going to, and the post's
                        // analytics page is where its progress shows.
                        let message = format!("Published. Sending to {n} subscriber{}.", if n == 1 { "" } else { "s" });
                        if json_mode { editor_saved_json(&state, &post, &message, "ok").await } else { posts_flash("ok", &message) }
                    }
                    Err(e) => {
                        let message = format!("Published, but the newsletter was not sent: {e}");
                        if json_mode {
                            editor_saved_json(&state, &post, &message, "error").await
                        } else {
                            posts_flash("error", &message)
                        }
                    }
                };
            }
            // Straight back into the editor, now that it has somewhere to
            // save to — the same place Ghost leaves you.
            let editor = format!("{}{}/", if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" }, post.id);
            match schedule {
                Scheduling::Undated => {
                    if json_mode {
                        editor_saved_json(&state, &post, UNDATED, "error").await
                    } else {
                        Redirect::to(&with_flash(&editor, "error", UNDATED)).into_response()
                    }
                }
                _ => {
                    if json_mode {
                        let message = match post.status.as_str() {
                            "published" => "Published.",
                            "scheduled" => "Scheduled.",
                            _ => "",
                        };
                        editor_saved_json(&state, &post, message, "ok").await
                    } else {
                        Redirect::to(&editor).into_response()
                    }
                }
            }
        }
        Err(e) => {
            if json_mode {
                editor_error_json(&e.message(), StatusCode::UNPROCESSABLE_ENTITY)
            } else {
                page_u("Error", "", Some(&user), &format!("<div class='banner error'>{}</div>", esc(&e.message())), true).into_response()
            }
        }
    }
}

pub(crate) async fn editor_create(State(state): State<AppState>, headers: HeaderMap, form: EditorForm) -> Response {
    editor_create_impl(&state, &headers, form, "post").await
}

pub(crate) async fn page_create(State(state): State<AppState>, headers: HeaderMap, form: EditorForm) -> Response {
    editor_create_impl(&state, &headers, form, "page").await
}

/// Redirect back to the posts list with a one-shot banner.
pub(crate) fn posts_flash(kind: &str, message: &str) -> Response {
    Redirect::to(&with_flash("/ghost/posts/", kind, message)).into_response()
}

/// A path carrying a one-shot banner for the page it lands on.
/// One query-string value, escaped for both the URL and the attribute it is
/// written into — the NQL filters the list toolbars carry between links are
/// full of `+`, `:` and quotes, every one of which has to survive the trip.
pub(crate) fn url_value(value: &str) -> String {
    esc(&form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>())
}

pub(crate) fn with_flash(path: &str, kind: &str, message: &str) -> String {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    format!("{path}?{encoded}")
}

// --- sending a post as a newsletter ---

/// What the editor asked for when saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishAction {
    PublishOnly,
    PublishAndEmail,
    /// Ghost's `status = 'sent'`: delivered to subscribers but not listed on
    /// the site.
    EmailOnly,
}

impl PublishAction {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "publish_email" => PublishAction::PublishAndEmail,
            "email_only" => PublishAction::EmailOnly,
            _ => PublishAction::PublishOnly,
        }
    }
    pub(crate) fn sends_email(self) -> bool {
        !matches!(self, PublishAction::PublishOnly)
    }
    /// The status a post ends up in. Email-only posts are `sent`, which keeps
    /// them off the site's own listings.
    pub(crate) fn status_for(self, requested: &str) -> String {
        match self {
            PublishAction::EmailOnly => "sent".to_string(),
            _ => requested.to_string(),
        }
    }
}

/// One member a newsletter is going to.
///
/// `status` and `created_at` are carried along because the "Subscription
/// details" box in the email prints them per recipient.
pub(crate) struct Subscriber {
    pub(crate) email: String,
    pub(crate) name: String,
    pub(crate) uuid: String,
    pub(crate) status: String,
    pub(crate) created_at: String,
}

/// Who a post goes to, from its visibility. Ghost derives the recipient
/// filter the same way: a paid-members post is not emailed to free members.
pub(crate) async fn newsletter_recipients(
    pool: &ghost_db::Db,
    newsletter_id: &str,
    visibility: &str,
    post_id: &str,
) -> Vec<Subscriber> {
    let paid_only = matches!(visibility, "paid" | "tiers");
    let base = "SELECT DISTINCT m.email, coalesce(m.name, ''), coalesce(m.uuid, ''), m.status, \
                coalesce(m.created_at, '') FROM members m \
                JOIN members_newsletters mn ON mn.member_id = m.id \
                WHERE mn.newsletter_id = ? AND m.email_disabled = 0 AND NOT EXISTS(SELECT 1 FROM email_suppressions es WHERE es.email=m.email)";
    // dialect-translated-after-assembly: the branches below are fragments, and
    // the finished statement goes through `dialect::sql_owned` a few lines down.
    let sql = if visibility == "tiers" {
        format!("{base} AND m.status IN ('paid', 'comped', 'gift') AND EXISTS(SELECT 1 FROM members_products mp JOIN posts_products pp ON pp.product_id = mp.product_id WHERE mp.member_id = m.id AND pp.post_id = ? AND (mp.expiry_at IS NULL OR datetime(mp.expiry_at) > datetime('now')))")
    } else if paid_only {
        format!("{base} AND m.status IN ('paid', 'comped', 'gift')")
    } else {
        base.to_string()
    };
    // Assembled from fragments above, so the owned translator.
    let sql = ghost_db::dialect::sql_owned(&sql);
    let query = sqlx::query_as::<_, (String, String, String, String, String)>(&sql).bind(newsletter_id);
    let query = if visibility == "tiers" { query.bind(post_id) } else { query };
    query.fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(email, name, uuid, status, created_at)| Subscriber {
            email,
            name,
            uuid,
            status,
            created_at,
        })
        .collect()
}

/// The byline a newsletter prints under the post title.
///
/// Ghost's own joining: one or two authors are listed in full, and beyond
/// that it becomes "First & N others" rather than a run-on line.
pub(crate) async fn newsletter_byline(state: &AppState, post_id: &str) -> String {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT u.name FROM posts_authors pa JOIN users u ON u.id = pa.author_id \
         WHERE pa.post_id = ? ORDER BY pa.sort_order",
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    match names.len() {
        0 => String::new(),
        1 | 2 => names.join(" & "),
        n => format!("{} & {} others", names[0], n - 1),
    }
}

/// Cut to `max` characters on a character boundary, with an ellipsis when
/// something was actually removed.
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Tell Slack a post went live, if the app is configured.
///
/// Fire-and-forget by design: a wedged or misconfigured webhook must not fail
/// a publish that has already happened, so a failure is logged and the
/// editor's response is unaffected. Ghost treats it the same way.
/// Tell the rest of the web about a published post: the search engines that
/// take an IndexNow ping, and the sites it links to.
///
/// Beside the Slack notification because they are the same moment, and
/// spawned for the same reason: a publish does not wait on anybody else.
pub(crate) async fn announce_published(state: &AppState, post: &ghost_core::models::Post) {
    let site_url = state.site_url();
    let router = state.router.read().await.clone();
    let json = ghost_core::models::post_to_json(post, &[], &[], "");
    let path = match router.permalink_for(&json) {
        Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, &json),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    ghost_api::announce::published(
        state,
        format!("{site_url}{path}"),
        // Absolute URLs, so a relative link in the body is not mistaken for
        // an external site worth notifying.
        ghost_core::url::absolutize_html(&site_url, post.html.as_deref().unwrap_or("")),
    );
}

pub(crate) async fn notify_slack_published(state: &AppState, post: &ghost_core::models::Post) {
    let Some(cfg) = ghost_api::slack::SlackConfig::from_settings(&state.settings) else {
        return;
    };
    let site_title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_url = state.site_url();
    let router = state.router.read().await.clone();
    let j = ghost_core::models::post_to_json(post, &[], &[], "");
    let path = match router.permalink_for(&j) {
        Some((_, pl)) => ghost_routing::tree::build_permalink(&pl, &j),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let excerpt = post
        .custom_excerpt
        .clone()
        .or_else(|| ghost_core::models::excerpt_from_plaintext(post.plaintext.as_deref(), 30))
        .unwrap_or_default();
    let author = repos::users::find_by_id(&state.db, post.published_by.as_deref().unwrap_or(""))
        .await
        .ok()
        .flatten()
        .and_then(|u| u.name);

    if let Err(e) = ghost_api::slack::notify_published(
        &cfg,
        &site_title,
        post.title.as_deref().unwrap_or("Untitled"),
        &format!("{site_url}{path}"),
        &excerpt,
        author.as_deref(),
    )
    .await
    {
        tracing::warn!("slack: publish notification failed: {}", e.message());
    }
}

/// Which transport carries a newsletter.
///
/// Mailgun is the right answer for a real list — it is built for bulk, and
/// pushing hundreds of messages through a transactional relay is how a sending
/// domain gets blocked. But requiring it outright makes a newsletter
/// untestable without a Mailgun account, and leaves a site with a dozen
/// members no way to mail them at all, so a configured SMTP relay is accepted
/// as a fallback. A local relay such as MailHog or Mailpit then works for
/// testing exactly as you would expect.
pub(crate) enum BulkTransport {
    Mailgun(ghost_mail::MailgunConfig),
    Ses(ghost_mail::RelaySender),
    /// The relay from Settings → Email, one message per recipient.
    Smtp,
}

impl BulkTransport {
    /// How many recipients one batch carries. Mailgun's own ceiling for a
    /// `recipient-variables` send; for a relay, which has no batch API, a
    /// smaller number that only decides how often progress is written down.
    fn batch_size(&self) -> usize {
        match self {
            BulkTransport::Mailgun(_) => ghost_mail::MAILGUN_BATCH_SIZE,
            BulkTransport::Smtp | BulkTransport::Ses(_) => 100,
        }
    }
}

pub(crate) fn resolve_bulk_transport_for_ses(state: &AppState, mail: &ghost_mail::MailConfig) -> Result<BulkTransport, String> {
    let cfg = ghost_mail::MailConfig::for_ses_newsletter(&state.settings, mail);
    if let Some(error) = cfg.configuration_error() {
        return Err(error);
    }
    let rate = state.settings.get_str("ses_max_send_rate").and_then(|s| s.parse::<u32>().ok()).unwrap_or(1).clamp(1, 10_000);
    ghost_mail::RelaySender::new(cfg, rate).map(BulkTransport::Ses).map_err(|e| e.message())
}

/// Explicit provider selection never falls back to stale credentials for another provider.
pub(crate) fn resolve_bulk_transport(state: &AppState, mail: &ghost_mail::MailConfig) -> Result<BulkTransport, String> {
    let provider = state.settings.get_str("newsletter_transport").unwrap_or_default();
    if provider == "smtp" && matches!(mail.transport, ghost_mail::Transport::Ses { .. }) {
        return resolve_bulk_transport_for_ses(state, mail);
    }
    let mg = ghost_mail::MailgunConfig::from_settings(&state.settings, mail);
    match provider.as_str() {
        "ses" => resolve_bulk_transport_for_ses(state, mail),
        "mailgun" => mg.map(BulkTransport::Mailgun).ok_or_else(|| "Configure Mailgun in Settings → Newsletters.".into()),
        "smtp" if mail.uses_relay() => Ok(BulkTransport::Smtp),
        "auto" | "" => match mg {
            Some(mg) => Ok(BulkTransport::Mailgun(mg)),
            None if matches!(mail.transport, ghost_mail::Transport::Ses { .. }) => resolve_bulk_transport_for_ses(state, mail),
            None if mail.uses_relay() => Ok(BulkTransport::Smtp),
            _ => Err("Configure Amazon SES or Mailgun for newsletters, or set up an SMTP relay.".into()),
        },
        _ => Err("The selected newsletter provider is not configured.".into()),
    }
}

/// Render and send `post` to its subscribers, and wait for it to finish.
///
/// Returns how many messages were accepted. Used by the scheduler, which has
/// nobody waiting on a response and does want the outcome in its log.
pub(crate) async fn send_post_as_newsletter(state: &AppState, post_id: &str) -> Result<usize, String> {
    deliver_newsletter(state, post_id, None).await
}

/// Send and wait, for the send-loop tests.
///
/// The publish path queues instead, so there is otherwise no way in from
/// outside the crate to a send whose outcome can be asserted on.
#[doc(hidden)]
pub async fn send_newsletter_for_test(state: &AppState, post_id: &str) -> Result<usize, String> {
    send_post_as_newsletter(state, post_id).await
}

/// Hand a send off to run on its own, and report how many it will go to.
///
/// A large list is minutes of provider requests even in batches of a
/// thousand, and holding the publish request open for all of it means any
/// proxy in front of the server can time the author out and leave them with
/// no idea whether their newsletter went. So everything that can be known
/// up front is checked up front — the post, the newsletter, the transport,
/// whether there is anybody to send to — and only then does the work start
/// in the background, with the `emails` row as its progress record.
pub(crate) async fn queue_newsletter_send(state: &AppState, post_id: &str) -> Result<usize, String> {
    let recipients = newsletter_preflight(state, post_id).await?;
    let state = state.clone();
    let post = post_id.to_string();
    tokio::spawn(async move {
        match deliver_newsletter(&state, &post, None).await {
            Ok(sent) => tracing::info!("newsletter: {post} emailed to {sent} subscriber(s)"),
            Err(e) => tracing::warn!("newsletter: {post} was published but not emailed: {e}"),
        }
    });
    Ok(recipients)
}

/// Everything about a send that can be refused before any mail moves, and
/// the number of people it would go to.
///
/// Deliberately the same checks, in the same order, that the send itself
/// makes: the point is that an author is told "no newsletter configured"
/// while they are still looking at the screen, rather than finding it in a
/// log afterwards.
pub(crate) async fn newsletter_preflight(state: &AppState, post_id: &str) -> Result<usize, String> {
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, post_id).await else {
        return Err("post not found".into());
    };
    if newsletter_already_sent(state, post_id).await {
        return Err("This post has already been emailed.".into());
    }
    let newsletter = match post.newsletter_id.as_deref() {
        Some(id) => repos::newsletters::find_by_id(&state.db, id).await.ok().flatten(),
        None => repos::newsletters::default_newsletter(&state.db).await.ok().flatten(),
    };
    let Some(newsletter) = newsletter else {
        return Err("No active newsletter to send on. Create one in Settings → Newsletters.".into());
    };
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let mail_cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &state.site_url());
    resolve_bulk_transport(state, &mail_cfg)?;
    let recipients = newsletter_audience(state, &post, &newsletter).await?;
    if recipients.is_empty() {
        return Err("No subscribers to send to.".into());
    }
    Ok(recipients.len())
}

/// Who a send goes to: the newsletter's subscribers the post's visibility
/// allows, narrowed by any audience filter saved against the post.
pub(crate) async fn newsletter_audience(
    state: &AppState,
    post: &ghost_core::models::Post,
    newsletter: &repos::newsletters::Newsletter,
) -> Result<Vec<Subscriber>, String> {
    let audience = ghost_api::newsletter_audience::load(state, &post.id).await.map_err(|e| e.message())?;
    let mut recipients = newsletter_recipients(
        &state.db,
        &newsletter.id,
        if audience.include_preview { "public" } else { &post.visibility },
        &post.id,
    )
    .await;
    if let Some(filter) = audience.filter.as_deref().filter(|s| !s.is_empty()) {
        let (members, _) = repos::members::browse(&state.db, Some(filter), -1, 1).await.map_err(|e| e.message())?;
        let emails: std::collections::HashSet<String> =
            members.into_iter().map(|member| member.email.to_lowercase()).collect();
        recipients.retain(|member| emails.contains(&member.email.to_lowercase()));
    }
    Ok(recipients)
}

/// Send again to the recipients an earlier send could not reach.
///
/// Everything is resolved the same way as the first time — audience, tiers,
/// body variants — and then narrowed to the members whose recipient row says
/// `failed`, so a retry cannot mail anybody twice. The original record is
/// reused rather than replaced: its counts, batches and failure history are
/// the account of the whole send, not of one attempt at it.
pub async fn retry_email(state: &AppState, email_id: &str) -> Result<usize, String> {
    let post_id: Option<String> = sqlx::query_scalar("SELECT post_id FROM emails WHERE id = ?")
        .bind(email_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(post_id) = post_id else {
        return Err("No such email.".into());
    };
    deliver_newsletter(state, &post_id, Some(email_id)).await
}

pub(crate) async fn deliver_newsletter(state: &AppState, post_id: &str, retry: Option<&str>) -> Result<usize, String> {
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, post_id).await else {
        return Err("post not found".into());
    };
    // A retry is the one case where an existing record is not a refusal.
    if retry.is_none() && newsletter_already_sent(state, post_id).await {
        return Err("This post has already been emailed.".into());
    }

    // Which newsletter's design and sender identity this goes out with: the
    // one the post names, else the default. Without one there is nothing to
    // send — the design settings live on the newsletter, not in `settings`.
    let newsletter = match post.newsletter_id.as_deref() {
        Some(id) => repos::newsletters::find_by_id(&state.db, id).await.ok().flatten(),
        None => repos::newsletters::default_newsletter(&state.db).await.ok().flatten(),
    };
    let Some(newsletter) = newsletter else {
        return Err("No active newsletter to send on. Create one in Settings → Newsletters.".into());
    };

    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let accent = state.settings.get_str("accent_color").unwrap_or_default();
    let mail_cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let bulk = resolve_bulk_transport(state, &mail_cfg)?;
    let bulk_from = match &bulk {
        BulkTransport::Mailgun(cfg) => &cfg.from_address,
        BulkTransport::Ses(sender) => &sender.config().from_address,
        BulkTransport::Smtp => &mail_cfg.from_address,
    };

    let mut recipients = newsletter_audience(state, &post, &newsletter).await?;

    // The host's two email ceilings, checked once the audience is known
    // because both are about its size. Ghost asks them in this order and for
    // different reasons: `members` is "is this list already bigger than the
    // plan allows", which a site can become without sending anything;
    // `emails` is "would this send go over the allowance for the period",
    // which is about this send specifically.
    if let Err(e) = state
        .limits
        .error_if_is_over(&state.db, "members", ghost_db::limits::Check::default())
        .await
    {
        return Err(e.message());
    }
    if let Err(e) = state
        .limits
        .error_if_would_go_over(&state.db, "emails", ghost_db::limits::Check::adding(recipients.len() as i64))
        .await
    {
        return Err(e.message());
    }

    if let Some(email_id) = retry {
        let failed: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
            "SELECT member_uuid FROM email_recipients WHERE email_id = ? AND status = 'failed' AND member_uuid IS NOT NULL",
        )
        .bind(email_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
        recipients.retain(|member| failed.contains(&member.uuid));
        if recipients.is_empty() {
            return Err("Every recipient of this email was reached; there is nothing to retry.".into());
        }
        // Their old rows go, so this attempt writes the current one. The
        // failure records stay: they are the history of what went wrong.
        let _ = sqlx::query("DELETE FROM email_recipients WHERE email_id = ? AND status = 'failed'")
            .bind(email_id)
            .execute(&state.db)
            .await;
    }
    if recipients.is_empty() {
        return Err("No subscribers to send to.".into());
    }

    let router = state.router.read().await.clone();
    let j = ghost_core::models::post_to_json(&post, &[], &[], "");
    let path = match router.permalink_for(&j) {
        Some((_, p)) => ghost_routing::tree::build_permalink(&p, &j),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let post_url = format!("{site_url}{path}");
    let post_title = post.title.clone().unwrap_or_else(|| "Untitled".into());
    // an email has no base to resolve `/content/images/…` against, so the
    // body goes out with full URLs
    let content_html = ghost_core::url::absolutize_html(
        &site_url,
        &post.html.clone().unwrap_or_default(),
    );
    let plaintext = post.plaintext.clone().unwrap_or_default();

    let secret = state
        .settings
        .get_str("members_email_auth_secret")
        .unwrap_or_default();
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    let site_ctx = ghost_mail::newsletter::SiteContext {
        title: title.clone(),
        url: site_url.clone(),
        icon_url: state
            .settings
            .get_str("icon")
            .filter(|i| !i.is_empty())
            .map(|i| ghost_core::url::absolute(&site_url, &ghost_core::url::base_asset(&i))),
        locale: state.settings.get_str("locale").unwrap_or_else(|| "en".into()),
        // Ghost takes this from i18next's `dir(locale)`. There is no
        // catalogue here, so left-to-right it is until one exists.
        direction: "ltr".into(),
        accent_color: accent,
    };

    // `posts_meta` is not attached by `find_by_id`, and three of its columns
    // are visible in the email, so it is fetched directly.
    let (email_subject, feature_image_alt, feature_image_caption) =
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT email_subject, feature_image_alt, feature_image_caption \
             FROM posts_meta WHERE post_id = ?",
        )
        .bind(&post.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .unwrap_or((None, None, None));

    let post_ctx = ghost_mail::newsletter::PostContext {
        id: post.id.clone(),
        title: post_title.clone(),
        url: post_url.clone(),
        html: content_html.clone(),
        plaintext: plaintext.clone(),
        custom_excerpt: post.custom_excerpt.clone(),
        published_at: ghost_core::dates::short_date_in_zone(
            post.published_at.as_deref().unwrap_or(""),
            &timezone,
        ),
        authors: newsletter_byline(state, &post.id).await,
        feature_image: post.feature_image.clone(),
        feature_image_width: None,
        feature_image_alt,
        feature_image_caption,
        email_only: post.status == "sent",
        // Paywalled sends would need per-segment bodies; one body for
        // everyone means nothing is truncated, so there is no paywall card.
        paywall: false,
    };

    // "Keep reading": the three most recent published posts, excluding this
    // one. Permalinks go through the router rather than being assumed to be
    // `/{slug}/`, so a site with a dated or prefixed structure links
    // correctly from the email.
    let mut latest: Vec<ghost_mail::newsletter::LatestPost> = Vec::new();
    if newsletter.show_latest_posts {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE status = 'published' AND type = 'post' AND id != ? \
             ORDER BY published_at DESC LIMIT 3",
        )
        .bind(&post.id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        for id in ids {
            let Ok(Some(p)) = repos::posts::find_by_id(&state.db, &id).await else {
                continue;
            };
            let pj = ghost_core::models::post_to_json(&p, &[], &[], "");
            let ppath = match router.permalink_for(&pj) {
                Some((_, pl)) => ghost_routing::tree::build_permalink(&pl, &pj),
                None => format!("/{}/", p.slug.clone().unwrap_or_default()),
            };
            latest.push(ghost_mail::newsletter::LatestPost {
                title: p.title.clone().unwrap_or_else(|| "Untitled".into()),
                url: format!("{site_url}{ppath}"),
                feature_image: p.feature_image.clone(),
                excerpt: p
                    .custom_excerpt
                    .clone()
                    .or_else(|| Some(ghost_mail::newsletter::plaintext(&ghost_core::models::public_html(&p))))
                    .map(|e| truncate_chars(&e, 120))
                    .filter(|e| !e.is_empty()),
            });
        }
    }

    let rendered = ghost_mail::newsletter::NewsletterEmail {
        site: &site_ctx,
        newsletter: &newsletter,
        post: &post_ctx,
        latest_posts: &latest,
        comments_enabled: state
            .settings
            .get_str("comments_enabled")
            .map(|v| v != "off")
            .unwrap_or(false),
    };

    let template_html=rendered.render().map_err(|e|e.message())?;
    let template_text=ghost_mail::newsletter::plaintext(&template_html);

    // Ghost lets the editor override the subject without changing the title.
    let subject = email_subject
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| post_title.clone());

    // The `emails` row is written before the first message rather than after
    // the last, for two reasons: its id is what the open pixel in each body
    // points at, and `post_id` is unique, so the row is also the lock that
    // stops a second send racing this one. Counts and status are settled as
    // the send progresses: pending → submitting → submitted or failed.
    let email_id = retry.map(str::to_owned).unwrap_or_else(ghost_core::ids::new_id);
    let now = ghost_core::dates::now_db();
    // Everyone the audience resolved to, before access and duplicate
    // addresses narrow it down. `preflight_email_count` is what is left.
    let candidate_count = recipients.len();
    let send_options = SendOptions {
        track_opens: state.settings.get_bool("email_track_opens"),
        track_clicks: state.settings.get_bool("email_track_clicks"),
    };
    let manage_account_url = format!("{site_url}/#/portal/account");
    let claimed = if retry.is_some() {
        // Back to `pending` with the previous attempt's error cleared, so the
        // status reads as this attempt rather than the last one.
        sqlx::query("UPDATE emails SET status='pending', error=NULL, updated_at=? WHERE id=?")
            .bind(&now)
            .bind(&email_id)
            .execute(&state.db)
            .await
    } else {
        sqlx::query(
        "INSERT INTO emails (id, uuid, post_id, status, recipient_filter, email_count, candidate_count, \
         subject, `from`, html, plaintext, source, source_type, track_opens, track_clicks, \
         feedback_enabled, submitted_at, newsletter_id, created_at, updated_at) \
         VALUES (?, ?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?, 'html', 'html', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&email_id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(post_id)
    .bind(&post.visibility)
    .bind(candidate_count as i64)
    .bind(candidate_count as i64)
    .bind(&subject)
    // the envelope sender, whichever transport carried it
    .bind(bulk_from)
    // the email as it went out, not the post body — this is the record of
    // what subscribers actually received
    .bind(&template_html)
    .bind(&template_text)
    .bind(send_options.track_opens)
    .bind(send_options.track_clicks)
    .bind(newsletter.feedback_enabled)
    .bind(&now)
    .bind(newsletter.id.clone())
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    };
    // `post_id` is unique on `emails`, so this insert is the claim on sending
    // this post. Losing it means another send got there first — which is the
    // one thing between a queued send and a subscriber being mailed twice,
    // so it has to be an error rather than something to carry on past.
    if let Err(e) = claimed {
        tracing::warn!("newsletter: could not claim the send for {post_id}: {e}");
        return Err("This post is already being emailed.".into());
    }

    // --- prepare ----------------------------------------------------------
    //
    // A batch addresses many people with one body, so every recipient in it
    // has to resolve the same body: same access to the post, same segment,
    // and — because a member with no name has the name row cut out of the
    // document rather than filled in — the same answer to "has a name".
    // Those three make the group; the transport's limit chops each group into
    // batches.
    let mut groups: std::collections::BTreeMap<(String, bool, bool), Vec<Prepared>> = Default::default();
    let mut variants: HashMap<String, (String, String)> = HashMap::new();
    let mut fatal: Option<String> = None;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sub in &recipients {
        macro_rules! or_abort {($result:expr) => {match $result {Ok(value)=>value,Err(e)=>{fatal=Some(e);break}}}}
        // Mailgun keys `recipient-variables` by address, so a repeated
        // address would quietly shrink the batch below the count recorded
        // against it. Dropped here, where it can be counted as excluded.
        if !seen.insert(sub.email.to_lowercase()) {
            continue;
        }
        let member_id=or_abort!(sqlx::query_scalar::<_,String>("SELECT id FROM members WHERE uuid=?").bind(&sub.uuid).fetch_one(&state.db).await.map_err(|e|e.to_string()));
        let tiers=or_abort!(ghost_db::repos::tiers::member_tier_ids(&state.db,&member_id).await.map_err(|e|e.message()));
        let access=ghost_core::comments::check_post_tier_access(&post.visibility,Some(&sub.status),&post.tier_ids,&tiers);
        let variant_key=format!("{}:{access}",sub.status);
        if !variants.contains_key(&variant_key){
            let mut variant=post_ctx.clone();
            let raw=if access{post.html.as_deref().unwrap_or("")}else{post.html.as_deref().unwrap_or("").split_once("<!--members-only-->").map(|(preview,_)|preview).unwrap_or("")};
            variant.html=ghost_core::url::absolutize_html(&site_url,&ghost_core::models::segment_html(raw,Some(&sub.status)));
            variant.plaintext=ghost_mail::newsletter::plaintext(&variant.html);variant.paywall=!access;
            let body=or_abort!(ghost_mail::newsletter::NewsletterEmail{site:&site_ctx,newsletter:&newsletter,post:&variant,latest_posts:&latest,comments_enabled:rendered.comments_enabled}.render().map_err(|e|e.message()));
            let body=or_abort!(ghost_api::newsletter_events::track_edition_links(state,&post.id,&email_id,&body).await.map_err(|e|e.message()));
            // Plaintext is taken before the pixel goes in: an invisible image
            // has nothing to say in a text-only reader.
            let text=ghost_mail::newsletter::plaintext(&body);
            let body=ghost_api::newsletter_events::track_opens(state,&email_id,&body);
            variants.insert(variant_key.clone(),(body,text));
        }
        let name = Some(sub.name.clone()).filter(|n| !n.trim().is_empty());
        // Per-recipient unsubscribe link, same HMAC the members API verifies.
        let key = hmac_hex(&secret, &sub.uuid);
        groups.entry((sub.status.clone(), access, name.is_some())).or_default().push(Prepared {
            member_id,
            recipient: ghost_mail::newsletter::Recipient {
                uuid: sub.uuid.clone(),
                email: sub.email.clone(),
                name,
                status: sub.status.clone(),
                created_at: ghost_core::dates::short_date_in_zone(&sub.created_at, &timezone),
                unsubscribe_url: format!("{site_url}/unsubscribe/?uuid={}&key={key}", sub.uuid),
                manage_account_url: manage_account_url.clone(),
                key,
            },
        });
    }

    let preflight: usize = groups.values().map(Vec::len).sum();
    // Nothing to send and a reason why: the record goes away so the post can
    // be tried again, and the recipient rows cascade with it.
    if preflight == 0 {
        if retry.is_none() {
            let _ = sqlx::query("DELETE FROM emails WHERE id=?").bind(&email_id).execute(&state.db).await;
        }
        return Err(fatal.unwrap_or_else(|| "No subscribers to send to.".into()));
    }
    let prepared_at = ghost_core::dates::now_db();
    let _ = if retry.is_some() {
        sqlx::query("UPDATE emails SET status='submitting', prepared_at=?, updated_at=? WHERE id=?")
            .bind(&prepared_at)
            .bind(&prepared_at)
            .bind(&email_id)
            .execute(&state.db)
            .await
    } else {
        sqlx::query(
            "UPDATE emails SET status='submitting', email_count=?, preflight_email_count=?, \
             preparation_excluded_count=?, prepared_at=?, updated_at=? WHERE id=?",
        )
        .bind(preflight as i64)
        .bind(preflight as i64)
        .bind((candidate_count.saturating_sub(preflight)) as i64)
        .bind(&prepared_at)
        .bind(&prepared_at)
        .bind(&email_id)
        .execute(&state.db)
        .await
    };

    // --- send -------------------------------------------------------------
    let mut sent = 0usize;
    let mut last_error: Option<String> = None;
    for ((status, access, named), members) in groups {
        let (body, text) = variants.get(&format!("{status}:{access}")).cloned().unwrap_or_default();
        // The body is rewritten once per group, not once per member: the
        // per-member values become provider placeholders.
        let batch_html = ghost_mail::newsletter::personalize(
            &body,
            &ghost_mail::newsletter::batch_placeholders(&status, named, true, &manage_account_url),
        );
        let batch_text = ghost_mail::newsletter::personalize(
            &text,
            &ghost_mail::newsletter::batch_placeholders(&status, named, false, &manage_account_url),
        );
        let segment = format!("status:{status} access:{access} named:{named}");
        for chunk in members.chunks(bulk.batch_size()) {
            let batch_id = match ghost_api::newsletter_events::create_batch(state, &email_id, &segment, chunk.len()).await {
                Ok(id) => id,
                Err(e) => {
                    // Without a batch record the send would be unreportable
                    // and unretryable, which is worse than stopping.
                    fatal = Some(e.message());
                    break;
                }
            };
            let outcome = match &bulk {
                BulkTransport::Mailgun(mg) => {
                    send_mailgun_chunk(mg, &email_id, &subject, &batch_html, &batch_text, chunk, named, &send_options).await
                }
                BulkTransport::Ses(sender) => {
                    send_smtp_chunk(sender.config(), Some(sender), &subject, &body, &text, chunk).await
                }
                BulkTransport::Smtp => {
                    send_smtp_chunk(&mail_cfg, None, &subject, &body, &text, chunk).await
                }
            };
            let batch_error = outcome.error.clone();
            for (member, provider_id, error) in outcome.recipients {
                match &error {
                    None => sent += 1,
                    Some(message) => last_error = Some(message.clone()),
                }
                // Losing a record must not stop a send: the subscriber has
                // the email either way, and a missing statistic is the
                // lesser problem.
                if let Err(e) = ghost_api::newsletter_events::record_recipient(
                    state,
                    ghost_api::newsletter_events::RecipientRecord {
                        email_id: &email_id,
                        batch_id: Some(&batch_id),
                        member_id: &member.member_id,
                        member_uuid: &member.recipient.uuid,
                        email: &member.recipient.email,
                        provider_id: provider_id.as_deref(),
                        error: error.as_deref(),
                    },
                )
                .await
                {
                    tracing::warn!("newsletter: could not record recipient {}: {}", member.recipient.email, e.message());
                }
            }
            let accepted = chunk.len() - outcome.failed;
            let _ = ghost_api::newsletter_events::settle_batch(
                state,
                &batch_id,
                outcome.provider_id.as_deref(),
                accepted,
                batch_error.as_deref(),
            )
            .await;
        }
        if fatal.is_some() {
            break;
        }
    }

    // A send that fell over before reaching anybody can be tried again, so
    // its record — which is also the lock that stops a post being emailed
    // twice — goes away with it; the recipient rows cascade. One that did
    // reach somebody keeps its record, because the alternative is mailing
    // those subscribers a second time.
    if fatal.is_some() && sent == 0 && retry.is_none() {
        let _ = sqlx::query("DELETE FROM emails WHERE id=?").bind(&email_id).execute(&state.db).await;
        return Err(fatal.unwrap_or_default());
    }

    // Settle the record now the outcome is known.
    let error = fatal.or_else(|| {
        (sent == 0).then(|| last_error.clone().unwrap_or_else(|| "no messages were accepted".into()))
    });
    // Counted from the rows rather than from this attempt's tallies, so a
    // retry reports the whole send and not just the part it just fixed.
    let _ = sqlx::query(
        "UPDATE emails SET status=?, \
         delivered_count=(SELECT COUNT(*) FROM email_recipients WHERE email_id=emails.id AND status='sent'), \
         failed_count=(SELECT COUNT(*) FROM email_recipients WHERE email_id=emails.id AND status='failed'), \
         error=?, updated_at=? WHERE id=?",
    )
    .bind(if error.is_none() { "submitted" } else { "failed" })
    .bind(error.as_deref())
    .bind(ghost_core::dates::now_db())
    .bind(&email_id)
    .execute(&state.db)
    .await;

    match error {
        Some(error) => Err(error),
        None => Ok(sent),
    }
}

/// A recipient with everything the send needs about them resolved.
#[derive(Clone)]
pub(crate) struct Prepared {
    member_id: String,
    recipient: ghost_mail::newsletter::Recipient,
}

/// Whether the provider should be asked to track opens and clicks on its
/// side. Separate from this port's own pixel and redirects, which work on
/// every transport.
pub(crate) struct SendOptions {
    track_opens: bool,
    track_clicks: bool,
}

/// What one chunk's submission produced, per recipient and as a whole.
pub(crate) struct ChunkOutcome {
    /// `(member, provider id for their message, why it failed)`.
    recipients: Vec<(Prepared, Option<String>, Option<String>)>,
    provider_id: Option<String>,
    error: Option<String>,
    failed: usize,
}

/// One Mailgun request for the whole chunk.
///
/// This is the change that makes a large list send in minutes rather than
/// hours: a thousand addresses, one request, and the per-member values go as
/// `recipient-variables` for Mailgun to substitute. Each recipient's
/// Message-Id is computed the same way on both sides, so a delivery or open
/// event still lands on the right member.
pub(crate) async fn send_mailgun_chunk(
    mg: &ghost_mail::MailgunConfig,
    email_id: &str,
    subject: &str,
    html: &str,
    text: &str,
    chunk: &[Prepared],
    named: bool,
    options: &SendOptions,
) -> ChunkOutcome {
    let batch: Vec<ghost_mail::BatchRecipient> = chunk
        .iter()
        .map(|member| ghost_mail::BatchRecipient {
            email: member.recipient.email.clone(),
            variables: ghost_mail::newsletter::batch_variables(&member.recipient, named),
        })
        .collect();
    let message = ghost_mail::BatchMessage {
        subject: subject.to_string(),
        html: html.to_string(),
        text: text.to_string(),
        email_id: email_id.to_string(),
        track_opens: options.track_opens,
        track_clicks: options.track_clicks,
    };
    match ghost_mail::send_mailgun_batch(mg, &message, &batch).await {
        Ok(outcome) => ChunkOutcome {
            recipients: chunk
                .iter()
                .map(|member| {
                    let provider_id =
                        ghost_mail::recipient_message_id(email_id, &member.recipient.email, &mg.domain);
                    (member.clone(), Some(provider_id), None)
                })
                .collect(),
            provider_id: outcome.provider_id,
            error: None,
            failed: 0,
        },
        // Mailgun takes or refuses a batch whole, so one refusal is a
        // failure for everyone in it.
        Err(e) => {
            let message = e.message();
            tracing::warn!("newsletter: batch of {} refused: {message}", chunk.len());
            ChunkOutcome {
                recipients: chunk.iter().map(|member| (member.clone(), None, Some(message.clone()))).collect(),
                provider_id: None,
                error: Some(message),
                failed: chunk.len(),
            }
        }
    }
}

/// One message per recipient, for a plain SMTP relay.
///
/// A relay has no batch API and no recipient variables, so this is still a
/// message at a time — but it is chunked like the Mailgun path so that a
/// stalled send says how far it got, and so one refusal is one recipient
/// rather than the whole send.
pub(crate) async fn send_smtp_chunk(
    mail_cfg: &ghost_mail::MailConfig,
    sender: Option<&ghost_mail::RelaySender>,
    subject: &str,
    html: &str,
    text: &str,
    chunk: &[Prepared],
) -> ChunkOutcome {
    let mut recipients = Vec::with_capacity(chunk.len());
    let mut failed = 0;
    for member in chunk {
        let msg = ghost_mail::Message {
            to: member.recipient.email.clone(),
            subject: subject.to_string(),
            html: ghost_mail::newsletter::personalize(html, &member.recipient),
            text: ghost_mail::newsletter::personalize(text, &member.recipient),
        };
        let result = match sender {
            Some(sender) => sender.send(&msg, Some(&member.recipient.unsubscribe_url)).await,
            None => ghost_mail::send_newsletter(mail_cfg, &msg).await,
        };
        match result {
            Ok(()) => recipients.push((member.clone(), None, None)),
            Err(e) => {
                tracing::warn!("newsletter: {} failed: {}", member.recipient.email, e.message());
                failed += 1;
                recipients.push((member.clone(), None, Some(e.message())));
            }
        }
    }
    ChunkOutcome { recipients, provider_id: None, error: None, failed }
}


/// HMAC-SHA256 hex, for the unsubscribe key. Mirrors the members API so the
/// links this emits validate there.
pub(crate) fn hmac_hex(key: &str, msg: &str) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut k = key.as_bytes().to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg.as_bytes());
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    hex::encode(outer.finalize())
}

/// Whether a newsletter has already gone out for this post. The `emails`
/// row is the record of that, and it is unique per post — which is what
/// stops an ordinary edit from mailing subscribers a second time.
pub(crate) async fn newsletter_already_sent(state: &AppState, post_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM emails WHERE post_id = ?")
        .bind(post_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0)
        > 0
}

/// The post's first author, for an audit entry written with nobody signed in.
pub(crate) async fn post_author_id(state: &AppState, post_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT author_id FROM posts_authors WHERE post_id = ? ORDER BY sort_order, id LIMIT 1",
    )
    .bind(post_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

pub(crate) async fn editor_save(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, mut form: EditorForm) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !may_edit_post(&state, &actor, &id).await {
        return forbidden_page(&actor, Role::Editor);
    }
    // Contributors may write but not publish, so any status they ask for
    // collapses back to draft. Enforced here rather than only in the form:
    // the status field is a plain select anyone can retype.
    if !actor.role.can_publish() {
        form.status = "draft".into();
    }
    let user = actor.user;
    if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::validate(&state,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
    let existing = repos::posts::find_by_id(&state.db, &id).await.ok().flatten();
    // Someone else may have saved this post while this editor sat open. Refuse
    // rather than overwrite them — the editor turns this into a prompt that
    // offers the publisher a copy of their own version.
    if let Some(current) = existing.as_ref() {
        if let Some(conflict) = editor_collision(&form, current) {
            return conflict;
        }
    }
    let post_type = existing
        .as_ref()
        .map(|p| p.post_type.clone())
        .unwrap_or_else(|| "post".into());
    // Slack hears about a post the moment it goes live, and only then — an
    // edit to something already published is not news.
    let was_published = existing.as_ref().is_some_and(|p| p.status == "published");
    // Kept alongside, because the scheduling webhooks need to know what the
    // status *was*, not only whether it was published.
    let was_status = existing.as_ref().map(|p| p.status.clone()).unwrap_or_default();
    // The tags before this save, so attach/detach can be told apart from a
    // plain edit. Read as ids, which is what the form posts back.
    let tags_before: Vec<String> = sqlx::query_scalar(
        "SELECT tag_id FROM posts_tags WHERE post_id = ? ORDER BY sort_order",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    // Scheduling is settled here as well as at creation: a draft can be
    // scheduled later, and a scheduled post's date and delivery can both be
    // changed right up until it goes out.
    let schedule = scheduling(&form, &post_type, PublishAction::parse(&form.publish_action));
    if matches!(schedule, Scheduling::Undated) {
        form.status = "draft".into();
    }
    // A save with no body in it is a settings-only save; the content columns
    // keep what they already hold rather than being rendered from nothing.
    let content = form.has_content().then(|| form.rendered());
    let (html, plaintext, reading_time) = match &content {
        Some((html, plaintext, reading_time)) => (Some(html.clone()), Some(plaintext.clone()), Some(*reading_time)),
        None => (None, None, None),
    };
    // Editing never sends: a post goes out once, when it is created, or not
    // at all. `publish_action` is deliberately ignored here, so a replayed or
    // hand-crafted request cannot mail subscribers from an edit.
    let changes = repos::posts::PostChanges {
        title: Some(form.title.clone()),
        slug: form.slug.clone(),
        lexical: form.lexical.clone(),
        markdown: if form.is_koenig() { None } else { form.markdown.clone() },
        html,
        plaintext,
        custom_excerpt: form.custom_excerpt.clone(),
        feature_image: form.feature_image.clone(),
        featured: Some(form.featured),
        // A save that arrives without a status or a visibility — a body this
        // parser could not read, or a hand-made one — must not blank the
        // column: no value means "unchanged", not "empty".
        status: (!form.status.is_empty()).then(|| form.status.clone()),
        visibility: form.visibility.clone().filter(|v| !v.is_empty()),
        published_at: form.published_at.clone(),
        codeinjection_head: form.codeinjection_head.clone(),
        codeinjection_foot: form.codeinjection_foot.clone(),
        custom_template: form.custom_template.clone(),
        canonical_url: form.canonical_url.clone(),
        show_title_and_feature_image: Some(form.show_title_and_feature_image),
        meta_title: form.meta_title.clone(),
        meta_description: form.meta_description.clone(),
        og_title: form.og_title.clone(),
        og_description: form.og_description.clone(),
        og_image: form.og_image.clone(),
        twitter_title: form.twitter_title.clone(),
        twitter_description: form.twitter_description.clone(),
        twitter_image: form.twitter_image.clone(),
        email_subject: form.email_subject.clone(),
        feature_image_alt: form.feature_image_alt.clone(),
        feature_image_caption: form.feature_image_caption.clone(),
        reading_time,
        tag_ids: Some(form.tags.clone()),
        tier_ids: if form.tiers_present { Some(form.tiers.clone()) } else { None },
        // Authors are only rewritten when the form offered a choice;
        // emptying the list would otherwise orphan the post.
        author_ids: if form.authors.is_empty() { None } else { Some(form.authors.clone()) },
        clearing: editor_owned_fields(form.is_koenig(), form.has_content()),
    };
    let json_mode = wants_json(&headers);
    match repos::posts::update(&state.db, &id, &changes).await {
        Ok(post) => {
            if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::save(&state,&post.id,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
            let _ = repos::actions::record(&state.db, &post_type, &post.id, &user.id, "edited").await;
            fire_post_webhooks_from(
                &state,
                &post_type,
                &post,
                "edited",
                was_published,
                &was_status,
                existing.as_ref(),
            );
            fire_tag_link_webhooks(&state, &post_type, &post, &tags_before, &form.tags).await;
            if !was_published && post.status == "published" {
                notify_slack_published(&state, &post).await;
                announce_published(&state, &post).await;
            }
            let list = if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" };
            // What the script's banner should say, if anything: only a change
            // in the post's standing is worth interrupting the writing for.
            let note = |post: &ghost_core::models::Post| match post.status.as_str() {
                "scheduled" => format!(
                    "Scheduled for {} UTC.",
                    to_datetime_local(post.published_at.as_deref().unwrap_or("")).replace('T', " at ")
                ),
                _ if !was_published && post.status == "published" => "Published.".to_string(),
                _ => String::new(),
            };
            match schedule {
                Scheduling::At(delivery) => {
                    let _ = repos::posts::set_delivery(&state.db, &post.id, delivery).await;
                    if json_mode {
                        editor_saved_json(&state, &post, &note(&post), "ok").await
                    } else {
                        Redirect::to(list).into_response()
                    }
                }
                // Anything else means the post is no longer waiting on a
                // date, and a standing instruction to email itself must not
                // outlive the schedule that carried it — the post could be
                // scheduled again later by someone who never saw this form.
                // A post that was *already* emailed keeps its record: that
                // one is in `emails`, and this only clears the intent.
                other => {
                    if existing.as_ref().is_some_and(|p| p.status == "scheduled") {
                        let _ = repos::posts::set_delivery(&state.db, &post.id, Default::default()).await;
                    }
                    match other {
                        // The date was unusable, so this came back as a draft
                        // and the author needs to know before closing the tab.
                        Scheduling::Undated => {
                            if json_mode {
                                editor_saved_json(&state, &post, UNDATED, "error").await
                            } else {
                                Redirect::to(&with_flash(list, "error", UNDATED)).into_response()
                            }
                        }
                        _ => {
                            if json_mode {
                                editor_saved_json(&state, &post, &note(&post), "ok").await
                            } else {
                                Redirect::to(list).into_response()
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            if json_mode {
                editor_error_json(&e.message(), StatusCode::UNPROCESSABLE_ENTITY)
            } else {
                page_u("Error", "", Some(&user), &format!("<div class='banner error'>{}</div>", esc(&e.message())), true).into_response()
            }
        }
    }
}

pub(crate) async fn delete_impl(state: &AppState, headers: &HeaderMap, id: &str, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !may_edit_post(state, &actor, id).await {
        return forbidden_page(&actor, Role::Editor);
    }
    let user = actor.user;
    // Read it before it goes: a delivery that cannot name what was deleted is
    // not much use to a listener.
    let doomed = repos::posts::find_by_id(&state.db, id).await.ok().flatten();
    let _ = repos::posts::destroy(&state.db, id).await;
    let _ = repos::actions::record(&state.db, post_type, id, &user.id, "deleted").await;
    if let Some(p) = doomed {
        fire_post_webhooks(state, post_type, &p, "deleted", p.status == "published");
    }
    Redirect::to(if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" }).into_response()
}

pub(crate) async fn post_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    delete_impl(&state, &headers, &id, "post").await
}

pub(crate) async fn page_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    delete_impl(&state, &headers, &id, "page").await
}

/// What the batch bar on the posts and pages list posted: one action, and
/// the rows it applies to repeating under one name. `Form` would keep only
/// the last of the repeated keys, so this parses the body itself — the same
/// arrangement the member editor's newsletter checkboxes use.
#[derive(Debug, Default)]
pub(crate) struct BulkPostsForm {
    action: String,
    ids: Vec<String>,
    /// The access popover's choice. Only read by `visibility`.
    visibility: String,
    /// The tag picker's ticked chips. Only read by `tags`, where an empty
    /// list is a real answer — it clears every tag — so it cannot be
    /// distinguished by emptiness the way `visibility` can.
    tags: Vec<String>,
}

impl BulkPostsForm {
    pub(crate) fn parse(body: &str) -> Self {
        let mut f = BulkPostsForm::default();
        for (k, v) in form_urlencoded::parse(body.as_bytes()) {
            match k.as_ref() {
                "action" => f.action = v.into_owned(),
                "ids" => f.ids.push(v.into_owned()),
                "visibility" => f.visibility = v.into_owned(),
                "tags" => f.tags.push(v.into_owned()),
                _ => {}
            }
        }
        f
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for BulkPostsForm {
    type Rejection = Response;
    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(BulkPostsForm::parse(&String::from_utf8_lossy(&bytes)))
    }
}

/// The posts and pages list's batch actions: feature, unfeature, delete.
///
/// Ghost applies bulk edits item by item under the same permission checks
/// as the single-item endpoints, and so does this — an Author who cannot
/// touch someone else's post selects it alongside their own and the banner
/// reports what was skipped rather than refusing the whole request. That
/// matters for the list too: the alternative is a page that empties itself
/// of rows the user never had visible rights over.
pub(crate) async fn bulk_impl(state: &AppState, headers: &HeaderMap, form: &BulkPostsForm, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let base = if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" };
    let (word, plural) = if post_type == "page" { ("page", "pages") } else { ("post", "posts") };
    if form.ids.is_empty() {
        return Redirect::to(&with_flash(base, "error", "Nothing was selected.")).into_response();
    }
    if !matches!(
        form.action.as_str(),
        "delete" | "feature" | "unfeature" | "publish" | "draft" | "visibility" | "tags"
    ) {
        return Redirect::to(&with_flash(base, "error", "Unknown batch action.")).into_response();
    }
    // Validate the one argument an action carries before touching anything,
    // so a typo cannot half-apply across a selection.
    if form.action == "visibility" && !matches!(form.visibility.as_str(), "public" | "members" | "paid") {
        return Redirect::to(&with_flash(base, "error", "Choose who can read them first."))
            .into_response();
    }

    let mut done = 0usize;
    let mut skipped = 0usize;
    for id in &form.ids {
        if !may_edit_post(state, &actor, id).await {
            skipped += 1;
            continue;
        }
        if matches!(form.action.as_str(), "publish" | "draft" | "visibility" | "tags") {
            // These go through the repo's own update, as the single-post
            // editor does: status changes touch `published_at`, and tags are
            // a join table, neither of which a bare UPDATE would get right.
            let changes = repos::posts::PostChanges {
                status: match form.action.as_str() {
                    "publish" => Some("published".to_string()),
                    "draft" => Some("draft".to_string()),
                    _ => None,
                },
                visibility: (form.action == "visibility").then(|| form.visibility.clone()),
                tag_ids: (form.action == "tags").then(|| form.tags.clone()),
                ..Default::default()
            };
            // The type check keeps an id lifted from one list from acting on
            // the other kind of row, as the other branches do.
            let wrong_kind = repos::posts::find_by_id(&state.db, id)
                .await
                .ok()
                .flatten()
                .is_none_or(|p| p.post_type != post_type);
            if wrong_kind {
                skipped += 1;
                continue;
            }
            match repos::posts::update(&state.db, id, &changes).await {
                Ok(p) => {
                    let _ = repos::actions::record(&state.db, post_type, id, &actor.user.id, "edited").await;
                    fire_post_webhooks(state, post_type, &p, "edited", p.status == "published");
                    done += 1;
                }
                Err(_) => skipped += 1,
            }
            continue;
        }
        if form.action == "delete" {
            // Read it before it goes, as the single delete does: a webhook
            // that cannot name what was deleted is not much use to a listener.
            // The type check keeps an id lifted from one list from acting on
            // the other kind of row.
            let doomed = repos::posts::find_by_id(&state.db, id)
                .await
                .ok()
                .flatten()
                .filter(|p| p.post_type == post_type);
            let Some(p) = doomed else {
                skipped += 1;
                continue;
            };
            let _ = repos::posts::destroy(&state.db, id).await;
            let _ = repos::actions::record(&state.db, post_type, id, &actor.user.id, "deleted").await;
            fire_post_webhooks(state, post_type, &p, "deleted", p.status == "published");
        } else {
            // `AND type = ?` keeps an id lifted from one list from acting on
            // the other kind of row.
            let _ = sqlx::query("UPDATE posts SET featured = ?, updated_at = ? WHERE id = ? AND type = ?")
                .bind(form.action == "feature")
                .bind(ghost_core::dates::now_db())
                .bind(id)
                .bind(post_type)
                .execute(&state.db)
                .await;
            let _ = repos::actions::record(&state.db, post_type, id, &actor.user.id, "edited").await;
            if let Some(p) = repos::posts::find_by_id(&state.db, id).await.ok().flatten() {
                fire_post_webhooks(state, post_type, &p, "edited", p.status == "published");
            }
        }
        done += 1;
    }

    if done == 0 {
        return Redirect::to(&with_flash(
            base,
            "error",
            &format!("None of those {plural} are yours to change."),
        ))
        .into_response();
    }
    let verb = match form.action.as_str() {
        "delete" => "Deleted",
        "feature" => "Featured",
        "unfeature" => "Unfeatured",
        "publish" => "Published",
        "draft" => "Moved to drafts:",
        "visibility" => "Changed access on",
        _ => "Retagged",
    };
    let mut message = format!("{verb} {} {}.", done, if done == 1 { word } else { plural });
    if skipped > 0 {
        message.push_str(&format!(
            " {} {} skipped — not yours to change.",
            skipped,
            if skipped == 1 { word } else { plural }
        ));
    }
    Redirect::to(&with_flash(base, "ok", &message)).into_response()
}

pub(crate) async fn posts_bulk(State(state): State<AppState>, headers: HeaderMap, form: BulkPostsForm) -> Response {
    bulk_impl(&state, &headers, &form, "post").await
}

pub(crate) async fn pages_bulk(State(state): State<AppState>, headers: HeaderMap, form: BulkPostsForm) -> Response {
    bulk_impl(&state, &headers, &form, "page").await
}
