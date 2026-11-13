//! The tags list and the tag editor.

use super::*;

// ---------------------------------------------------------------------------
// tags
// ---------------------------------------------------------------------------

/// The visibility filters along the top. `None` is "everything"; internal
/// tags are the `#`-prefixed ones Ghost keeps out of the site's tag lists.
pub(crate) const TAG_FILTERS: &[(&str, &str, Option<&str>)] = &[
    ("all", "All Tags", None),
    ("public", "Public Tags", Some("public")),
    ("internal", "Internal Tags", Some("internal")),
];

pub(crate) async fn tags_list(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Editor).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let (mut tags, _) = repos::tags::browse(&state.db, None, None, 100, 1, false, true).await.unwrap_or_default();
    // The tabs count the whole list, then the selected one narrows it, so a
    // tab's number does not change when you click a different tab.
    let mut counts: HashMap<String, i64> = HashMap::new();
    for t in &tags {
        *counts.entry(t.visibility.clone()).or_insert(0) += 1;
    }
    let selected = q.get("filter").map(String::as_str).unwrap_or("all");
    let selected = TAG_FILTERS
        .iter()
        .find(|(key, _, _)| *key == selected)
        .unwrap_or(&TAG_FILTERS[0]);
    if let Some(visibility) = selected.2 {
        tags.retain(|t| t.visibility == visibility);
    }
    let rows: String = tags
        .iter()
        .map(|t| {
            let count = t.count_posts.unwrap_or(0);
            let internal = t.visibility == "internal";
            format!(
                r#"<div class="row" data-filterable>
<span class="ic tint-{tint}">{ic}</span>
<a class="body" href="/ghost/tags/{id}/"><span class="title">{name}</span><span class="detail"><code>/{slug}/</code></span></a>
<div class="trailing">{internal_chip}<span class="chip tonal">{count} {post_word}</span>
<form method="post" action="/ghost/tags/{id}/delete/" data-confirm="Delete tag|&#8220;{name}&#8221; will be removed from every post.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>{chev}</div>
</div>"#,
                tint = if internal { "slate" } else { "amber" },
                ic = svg("tags"),
                name = esc(t.name.as_deref().unwrap_or("")),
                slug = esc(t.slug.as_deref().unwrap_or("")),
                internal_chip = if internal { r#"<span class="chip draft">internal</span>"# } else { "" },
                count = count,
                post_word = if count == 1 { "post" } else { "posts" },
                id = esc(&t.id),
                del = svg("delete"),
                chev = svg("chevron"),
            )
        })
        .collect();
    page_for_role(actor_role, 
        "Tags",
        "tags",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Tags</h1><p class="sub">{count} {tag_word}</p></div></header>
{filters}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/tags/" class="inline-form">
<label class="field" style="flex:1;min-width:220px;margin:0"><input name="name" placeholder=" " required><span>New tag name</span></label>
<button class="btn tonal" type="submit">{add} Add tag</button></form></div>
<div class="section">{section}</div>
<div class="search rise" style="--i:2">{search}<input data-filter type="search" placeholder="Search tags" aria-label="Search tags"></div>
<div class="card rows rise" style="--i:3">{rows}</div>"#,
            filters = seg_filter_bar(TAG_FILTERS, "/ghost/tags/", selected.0, "", &counts),
            section = esc(selected.1),
            count = tags.len(),
            tag_word = if tags.len() == 1 { "tag" } else { "tags" },
            add = svg("add"),
            search = svg("search"),
            rows = rows,
        ),
        true,
    )
    .into_response()
}

/// Ghost's tag editor, field for field: the basics, then the metadata and
/// social cards that `ghost_head` emits on the tag's archive page, then code
/// injection.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct TagEditForm {
    name: String,
    slug: String,
    description: String,
    accent_color: String,
    feature_image: String,
    meta_title: String,
    meta_description: String,
    canonical_url: String,
    og_title: String,
    og_description: String,
    og_image: String,
    twitter_title: String,
    twitter_description: String,
    twitter_image: String,
    codeinjection_head: String,
    codeinjection_foot: String,
}

/// An author's public archive URL, resolved through the router so it follows
/// routes.yaml (`/author/{slug}/` by default) instead of assuming a path.
pub(crate) async fn author_archive_url(state: &AppState, user: &ghost_core::models::User) -> String {
    let router = state.router.read().await.clone();
    let j = ghost_core::models::user_public_json(user, None);
    match router.permalink_for(&j) {
        Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, &j),
        None => format!("/author/{}/", user.slug.clone().unwrap_or_default()),
    }
}

/// The tag's public archive URL, resolved through the same router the
/// frontend uses so it always matches routes.yaml.
pub(crate) async fn tag_archive_url(state: &AppState, tag: &ghost_core::models::Tag) -> Option<String> {
    let router = state.router.read().await.clone();
    let j = ghost_core::models::tag_to_json(tag, None);
    let (_, permalink) = router.permalink_for(&j)?;
    Some(ghost_routing::tree::build_permalink(&permalink, &j))
}

pub(crate) fn tag_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/tags/{id}/?{encoded}")).into_response()
}

pub(crate) async fn tag_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Editor).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(tag)) = repos::tags::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That tag could not be found.");
    };
    let v = |s: Option<&str>| esc(s.unwrap_or(""));
    let internal = tag.visibility == "internal";
    let slug = tag.slug.clone().unwrap_or_default();
    let accent = tag.accent_color.clone().unwrap_or_default();

    page_for_role(actor_role, 
        "Tag",
        "tags",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/tags/" title="Back to tags">{back}</a>
<div><h1>{name}</h1><p class="sub">{sub}</p></div>
<div class="actions"><button class="btn" type="submit" form="tag-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/tags/{id}/" id="tag-form">
<div class="section">Basics</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{ic}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " required><span>Name</span></label>
<label class="field" style="margin:0"><input name="slug" value="{slug}" placeholder=" "><span>Slug</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="description" rows="2" placeholder=" " maxlength="500">{description}</textarea><span>Description · max 500 characters</span></label>
<div class="grid2" style="gap:12px;margin-top:12px">
<label class="field" style="margin:0"><input name="accent_color" value="{accent}" placeholder=" " pattern="^#?[0-9A-Fa-f]{{3,8}}$"><span>Colour · hex, e.g. #ff1a75</span></label>
<label class="field" style="margin:0"><input name="feature_image" value="{feature_image}" placeholder=" "><span>Tag image URL</span></label>
</div>
<p class="muted" style="margin:10px 0 0">A name starting with <code>#</code> makes the tag internal: it stays off the site and out of search, but still groups posts. {internal_note}</p>
</div></div>
</div>
<div class="section">Metadata</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{notes}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="meta_title" value="{meta_title}" placeholder=" "><span>Meta title</span></label>
<label class="field" style="margin:0"><input name="canonical_url" value="{canonical_url}" placeholder=" "><span>Canonical URL</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="meta_description" rows="2" placeholder=" " maxlength="500">{meta_description}</textarea><span>Meta description</span></label>
<p class="muted" style="margin:10px 0 0">Used by <code>{{{{ghost_head}}}}</code> on <code>/{slug}/</code>. Blank falls back to the tag name and description.</p>
</div></div>
</div>
<div class="section">Social cards</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{link}</span>
<div class="body" style="width:100%">
<span class="title" style="display:block;margin-bottom:8px">Facebook / Open Graph</span>
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="og_title" value="{og_title}" placeholder=" "><span>Title</span></label>
<label class="field" style="margin:0"><input name="og_image" value="{og_image}" placeholder=" "><span>Image URL</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="og_description" rows="2" placeholder=" ">{og_description}</textarea><span>Description</span></label>
</div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{link}</span>
<div class="body" style="width:100%">
<span class="title" style="display:block;margin-bottom:8px">X (Twitter)</span>
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="twitter_title" value="{twitter_title}" placeholder=" "><span>Title</span></label>
<label class="field" style="margin:0"><input name="twitter_image" value="{twitter_image}" placeholder=" "><span>Image URL</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="twitter_description" rows="2" placeholder=" ">{twitter_description}</textarea><span>Description</span></label>
</div></div>
</div>
<div class="section">Code injection</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{code}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0 0 12px"><textarea name="codeinjection_head" class="code" rows="4" placeholder=" ">{cih}</textarea><span>Head · injected on this tag's archive page</span></label>
<label class="field" style="margin:0"><textarea name="codeinjection_foot" class="code" rows="4" placeholder=" ">{cif}</textarea><span>Foot</span></label>
</div></div>
</div>
</form>
<div class="section">Danger zone</div>
<div class="card rows rise" style="--i:5">
<div class="row"><span class="ic tint-red">{del}</span>
<div class="body"><span class="title">Delete this tag</span><span class="detail">It is removed from every post; the posts themselves are kept.</span></div>
<div class="trailing">
<form method="post" action="/ghost/tags/{id}/delete/" data-confirm="Delete tag|&#8220;{name}&#8221; will be removed from every post.">
<button class="btn tonal small danger" type="submit">Delete</button></form></div></div>
</div>
<p class="muted" style="margin:14px 0 0 16px">{public_link}</p>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&id),
            ic = svg("tags"),
            notes = svg("notes"),
            link = svg("link"),
            code = svg("code"),
            del = svg("delete"),
            name = v(tag.name.as_deref()),
            name_v = v(tag.name.as_deref()),
            sub = if internal { "Internal tag".to_string() } else { format!("{} posts", tag.count_posts.unwrap_or(0)) },
            slug = esc(&slug),
            description = v(tag.description.as_deref()),
            accent = esc(&accent),
            feature_image = v(tag.feature_image.as_deref()),
            internal_note = if internal { "This tag is internal." } else { "" },
            meta_title = v(tag.meta_title.as_deref()),
            meta_description = v(tag.meta_description.as_deref()),
            canonical_url = v(tag.canonical_url.as_deref()),
            og_title = v(tag.og_title.as_deref()),
            og_description = v(tag.og_description.as_deref()),
            og_image = v(tag.og_image.as_deref()),
            twitter_title = v(tag.twitter_title.as_deref()),
            twitter_description = v(tag.twitter_description.as_deref()),
            twitter_image = v(tag.twitter_image.as_deref()),
            cih = v(tag.codeinjection_head.as_deref()),
            cif = v(tag.codeinjection_foot.as_deref()),
            public_link = if internal {
                "Internal tags have no public archive page.".to_string()
            } else {
                // Ask the router rather than assuming `/{slug}/`: the archive
                // path comes from routes.yaml, which defaults to /tag/{slug}/
                // and can be changed.
                match tag_archive_url(&state, &tag).await {
                    Some(u) => format!(
                        r#"Public archive: <a href="{u}" target="_blank" rel="noopener">{u}</a>"#,
                        u = esc(&u)
                    ),
                    None => "This tag has no public archive page.".to_string(),
                }
            },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn tag_edit_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<TagEditForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    let name = form.name.trim();
    if name.is_empty() {
        return tag_flash(&id, "error", "A tag needs a name.");
    }
    // Ghost accepts a colour with or without the leading hash; store it the
    // way themes expect to read it.
    let accent = match form.accent_color.trim().trim_start_matches('#') {
        "" => String::new(),
        hex => format!("#{hex}"),
    };
    let t = |s: &str| Some(s.trim().to_string());
    let changes = repos::tags::TagChanges {
        name: Some(name.to_string()),
        slug: t(&form.slug),
        description: t(&form.description),
        feature_image: t(&form.feature_image),
        accent_color: Some(accent),
        meta_title: t(&form.meta_title),
        meta_description: t(&form.meta_description),
        canonical_url: t(&form.canonical_url),
        og_title: t(&form.og_title),
        og_description: t(&form.og_description),
        og_image: t(&form.og_image),
        twitter_title: t(&form.twitter_title),
        twitter_description: t(&form.twitter_description),
        twitter_image: t(&form.twitter_image),
        // code injection is markup — keep it byte for byte
        codeinjection_head: Some(form.codeinjection_head.clone()),
        codeinjection_foot: Some(form.codeinjection_foot.clone()),
        visibility: None,
    };
    match repos::tags::update(&state.db, &id, &changes).await {
        Ok(tag) => {
            fire_tag_webhook(&state, "tag.edited", &tag);
            tag_flash(&id, "ok", "Tag saved.")
        }
        Err(e) => tag_flash(&id, "error", &e.message()),
    }
}

#[derive(Deserialize)]
pub(crate) struct TagForm {
    name: String,
}

pub(crate) async fn tag_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<TagForm>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    if let Ok(tag) = repos::tags::insert(
        &state.db,
        &repos::tags::NewTag {
            name: form.name,
            ..Default::default()
        },
    )
    .await
    {
        fire_tag_webhook(&state, "tag.added", &tag);
    }
    Redirect::to("/ghost/tags/").into_response()
}

pub(crate) async fn tag_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    // Read it first: once the row is gone there is nothing to describe, and
    // a delete webhook that carries only an id is close to useless.
    let doomed = repos::tags::find_by_id(&state.db, &id).await.ok().flatten();
    let destroyed = repos::tags::destroy(&state.db, &id).await.is_ok();
    if let (true, Some(tag)) = (destroyed, doomed) {
        fire_tag_webhook(&state, "tag.deleted", &tag);
    }
    Redirect::to("/ghost/tags/").into_response()
}
