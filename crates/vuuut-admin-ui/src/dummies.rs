//! Tools → Dummy content: fill a site with demonstration content, and take it
//! away again.
//!
//! One screen, `/ghost/dummies/`, with three parts:
//!
//! * **Full demo site** — one click makes what a theme store's demo shows:
//!   tags, authors, posts, pages, members, store products and menu links.
//! * **Custom content** — any of those kinds, in the numbers chosen.
//! * **Generated content** — every batch still on the site, each removable,
//!   or all of it at once.
//!
//! The content itself (titles, bodies, people, products) is written by
//! [`vuuut_core::dummies`]; what was made is recorded by
//! [`vuuut_db::repos::dummies`], which is also where removal decides what may
//! go. Removal deletes only what is exactly as generated: anything edited
//! since, or now used by the site's own content, is kept as ordinary content.
//!
//! ## What generating does not do
//!
//! Generated content goes straight in through the repos, not through the
//! editor's save path, so it never sends a newsletter, posts to Slack or
//! social accounts, or fires webhooks: demonstration material must not
//! announce itself. Menu links are appended to the existing menus and removed
//! one by one, so a site's own menu is never replaced.
//!
//! ## The security contract
//!
//! Administrator and above, every write origin-checked. The screen creates
//! staff accounts and members, so it is not offered to lower roles even
//! through Permissions (it is not in the permissions catalogue, which leaves
//! the role check in force). Generated staff are Authors or Contributors with
//! a password hash nothing can match, so nobody can sign in as one; every
//! generated address is `@example.com`, and generated members have email
//! disabled. A batch id from the form is checked to be an object id and to
//! exist before it is used.

use super::*;
use vuuut_core::dummies::{self as rules, Generator, Kind, Menu, Plan};
use vuuut_db::repos::dummies as repo;

const BASE: &str = "/ghost/dummies/";
// Concurrent demo requests share the batch tables and the site's menus.
static DUMMY_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Stored as a generated author's password. Not a bcrypt hash, so
/// `bcrypt::verify` fails against it for every password there is.
const NO_PASSWORD: &str = "!dummy-content-cannot-sign-in";

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(kind: &str, message: &str) -> Response {
    crate::tools::tool_flash(BASE, kind, message)
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

pub(crate) async fn dummies_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let batches = repo::batches(&state.db).await;
    let full = Plan::full(true);

    let full_list: String = Kind::ALL
        .into_iter()
        .map(|k| match k {
            Kind::Navigation => "<li>Navigation links</li>".to_string(),
            _ => format!(r#"<li><span class="dm-count">{n}</span><span>{label}</span></li>"#, n = full.count(k), label = esc(k.label())),
        })
        .collect();

    let custom_rows: String = Kind::ALL
        .into_iter()
        .map(|k| {
            let control = if k == Kind::Navigation {
                r#"<label class="switch"><input type="checkbox" name="count_navigation" value="1" aria-label="Add menu links"><span class="track"><span class="thumb"></span></span></label>"#.to_string()
            } else {
                format!(
                    r#"<input class="dm-number" type="number" name="count_{key}" min="0" max="{max}" value="0" inputmode="numeric" aria-label="{label}, up to {max}">"#,
                    key = k.key(),
                    max = k.max(),
                    label = esc(k.label()),
                )
            };
            format!(
                r#"<div class="row dm-row"><div class="body"><span class="title">{label}</span><span class="detail">{hint}{limit}</span></div><div class="trailing">{control}</div></div>"#,
                label = esc(k.label()),
                hint = esc(k.hint()),
                limit = if k == Kind::Navigation { String::new() } else { format!(" Up to {}.", k.max()) },
            )
        })
        .collect();

    let generated = if batches.is_empty() {
        r#"<p class="muted dm-empty">Nothing generated yet. Whatever you make here is listed, and can be removed in one click.</p>"#.to_string()
    } else {
        let rows: String = batches
            .iter()
            .map(|b| {
                let left: Vec<String> = b.remaining.iter().map(|(k, n)| rules::counted(*n, *k)).collect();
                format!(
                    r#"<div class="row"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{name} <span class="chip tonal">{when}</span></span>
<span class="detail">{left} still on the site. Generated by {who}.</span></div>
<div class="trailing"><form method="post" action="{BASE}remove/" data-confirm="Remove this batch|Deletes the dummy content from this batch that is still exactly as generated. Anything edited since stays, as your own content.">
<input type="hidden" name="batch" value="{id}"><button class="btn tonal small danger" type="submit">Remove</button></form></div></div>"#,
                    tint = if b.label == "full" { "purple" } else { "blue" },
                    ic = svg(if b.label == "full" { "layout" } else { "box" }),
                    name = if b.label == "full" { "Full demo site" } else { "Custom content" },
                    when = esc(&friendly_date(&b.created_at)),
                    left = esc(&rules::join_and(&left)),
                    who = esc(b.created_by.as_deref().unwrap_or("a former staff member")),
                    id = esc(&b.id),
                )
            })
            .collect();
        format!(r#"<div class="card rows">{rows}</div>"#)
    };

    let remove_all = if batches.is_empty() {
        String::new()
    } else {
        format!(
            r#"<form method="post" action="{BASE}remove/" data-confirm="Remove all dummy content|Deletes every generated item that is still exactly as generated. Anything edited since, or now used by your own content, stays."><input type="hidden" name="batch" value="all"><button class="btn tonal small danger" type="submit">{} Remove all dummy content</button></form>"#,
            svg("delete")
        )
    };

    page_for_role(
        actor.role,
        "Dummy content",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<p class="muted dm-lead rise" style="--i:1">Fill the site with demonstration content to see a theme working, then remove it in one click. Removing deletes only what is still exactly as generated.</p>
<div class="dm-grid rise" style="--i:2">
<section class="card padded dm-card" aria-labelledby="dm-full-title">
<span class="ic tint-purple">{full_ic}</span>
<h2 id="dm-full-title">Full demo site</h2>
<p class="muted">Everything a theme's demo shows, in one go. Pages that already exist are linked rather than duplicated, and links are added to your menus, not swapped for them.</p>
<ul class="dm-list">{full_list}</ul>
<form method="post" action="{BASE}full/" class="dm-actions">
<label class="dm-check"><input type="checkbox" name="images" value="1" checked> Feature images on posts, from Unsplash</label>
<button class="btn" type="submit">Generate the full demo</button>
</form>
</section>
<section class="card padded dm-card" aria-labelledby="dm-custom-title">
<span class="ic tint-blue">{custom_ic}</span>
<h2 id="dm-custom-title">Custom content</h2>
<p class="muted">Choose what to make and how many. Posts are tagged with any tags you make here, or with your existing ones.</p>
<form method="post" action="{BASE}custom/">
<div class="dm-rows">{custom_rows}</div>
<div class="dm-options">
<label class="field fixed" style="margin:0"><select name="status"><option value="published" selected>Published</option><option value="draft">Draft</option></select><span>Posts, pages and products</span>{caret}</label>
<label class="dm-check"><input type="checkbox" name="images" value="1"> Feature images on posts, from Unsplash</label>
</div>
<div class="dm-actions"><button class="btn" type="submit">Generate</button></div>
</form>
</section>
</div>
<section class="dm-generated rise" style="--i:3" aria-labelledby="dm-generated-title">
<div class="dm-generated-head"><h2 id="dm-generated-title">Generated content</h2>{remove_all}</div>
{generated}
</section>"#,
            head = crate::tools::tool_head("dummies", None, ""),
            banner = flash_banner(&q),
            full_ic = svg("layout"),
            custom_ic = svg("tune"),
            caret = SELECT_CARET,
        ),
        true,
    )
    .into_response()
}

/// "24 Nov 2026" from a stored timestamp.
fn friendly_date(stored: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(stored, "%Y-%m-%d %H:%M:%S")
        .map(|d| d.format("%-d %b %Y, %H:%M").to_string())
        .unwrap_or_else(|_| stored.to_string())
}

// ---------------------------------------------------------------------------
// Generating
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct FullForm {
    images: String,
}

pub(crate) async fn generate_full(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<FullForm>) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let plan = Plan::full(!form.images.is_empty());
    finish(&state, &actor, &plan).await
}

pub(crate) async fn generate_custom(State(state): State<AppState>, headers: HeaderMap, Form(fields): Form<Vec<(String, String)>>) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match Plan::custom(&fields) {
        Ok(plan) => finish(&state, &actor, &plan).await,
        Err(message) => flash_to("error", &message),
    }
}

async fn finish(state: &AppState, actor: &Actor, plan: &Plan) -> Response {
    let _guard = DUMMY_WRITES.lock().await;
    match generate(state, actor, plan).await {
        Ok(made) if made.is_empty() => flash_to("error", "Nothing new was made: everything asked for already exists on the site."),
        Ok(made) => flash_to("ok", &format!("Generated {}.", rules::join_and(&made))),
        Err(e) => {
            tracing::error!("dummy content: generation stopped: {e}");
            flash_to("error", "Generation stopped part-way. What was made is listed below and can be removed.")
        }
    }
}

/// Make everything in `plan`, recording each item as it goes, so a failure
/// part-way still leaves a removable batch. Returns what was made, in words.
async fn generate(state: &AppState, actor: &Actor, plan: &Plan) -> vuuut_core::Result<Vec<String>> {
    let db = &state.db;
    let batch = repo::create_batch(db, if plan.full { "full" } else { "custom" }, &plan.summary(), &actor.user.id).await?;
    let seed = u64::from_str_radix(&batch[batch.len().saturating_sub(16)..], 16).unwrap_or(1);
    let mut g = Generator::new(seed);
    let mut made: Vec<(Kind, i64)> = Vec::new();
    let mut note = |kind: Kind, n: i64| {
        if n > 0 {
            made.push((kind, n));
        }
    };

    // Tags. One that already exists is used, not duplicated.
    let mut tag_ids: Vec<(String, String, String)> = Vec::new(); // id, name, slug
    let mut n = 0;
    for tag in g.tags(plan.count(Kind::Tags)) {
        let slug = vuuut_core::slug::safe_string(tag.name);
        if let Some(existing) = repos::tags::find_by_slug(db, &slug).await? {
            if existing.visibility == "public" {
                tag_ids.push((existing.id, existing.name.unwrap_or_else(|| tag.name.to_string()), slug));
            }
            continue;
        }
        let new = repos::tags::NewTag {
            name: tag.name.to_string(),
            slug: Some(slug.clone()),
            description: Some(tag.description.to_string()),
            visibility: "public".into(),
            accent_color: Some(tag.accent.to_string()),
            ..Default::default()
        };
        let created = repos::tags::insert(db, &new).await?;
        repo::record(db, &batch, Kind::Tags, &created.id).await?;
        tag_ids.push((created.id, tag.name.to_string(), slug));
        n += 1;
    }
    note(Kind::Tags, n);

    // Authors.
    let mut author_ids: Vec<String> = Vec::new();
    let mut n = 0;
    for person in g.people(plan.count(Kind::Users)) {
        let user = match repos::users::insert_staff(db, person.name, &person.email, NO_PASSWORD, person.role).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("dummy content: author {} skipped: {e}", person.email);
                continue;
            }
        };
        repo::set_user_profile(db, &user.id, person.bio, person.location).await?;
        repo::record(db, &batch, Kind::Users, &user.id).await?;
        author_ids.push(user.id);
        n += 1;
    }
    note(Kind::Users, n);

    // Posts, by this batch's authors (or whoever is generating) and tagged
    // with this batch's tags (or the site's own).
    let posts = plan.count(Kind::Posts);
    if posts > 0 {
        let tag_pool: Vec<String> = if tag_ids.is_empty() { repo::public_tag_ids(db, 12).await } else { tag_ids.iter().map(|t| t.0.clone()).collect() };
        let authors: Vec<String> = if author_ids.is_empty() { vec![actor.user.id.clone()] } else { author_ids.clone() };
        let mut n = 0;
        for post in g.posts(posts) {
            let author = authors[g.pick(authors.len())].clone();
            let tags: Vec<String> = g.pick_some(tag_pool.len()).into_iter().map(|i| tag_pool[i].clone()).collect();
            let published = chrono::Utc::now() - chrono::Duration::days(post.days_ago) - chrono::Duration::minutes(g.pick(600) as i64);
            let new = content(&post.title, None, post.excerpt, &post.lexical, "post", plan.publish, Some(published), post.featured, if plan.images { post.image } else { None }, &author, tags);
            let created = repos::posts::insert(db, &new, &author).await?;
            repo::record(db, &batch, Kind::Posts, &created.id).await?;
            n += 1;
        }
        note(Kind::Posts, n);
    }

    // Existing pages keep their content. Only published, public pages can
    // become navigation targets.
    let mut menu: Vec<(Menu, String, String)> = Vec::new(); // menu, label, url
    let mut n = 0;
    for page in g.pages(plan.count(Kind::Pages)) {
        let saved = match repos::posts::find_by_slug(db, page.slug, "page", false).await? {
            Some(existing) => existing,
            None => {
                let new = content(page.title, Some(page.slug), page.excerpt, &page.lexical, "page", plan.publish, None, false, None, &actor.user.id, Vec::new());
                let created = repos::posts::insert(db, &new, &actor.user.id).await?;
                repo::record(db, &batch, Kind::Pages, &created.id).await?;
                n += 1;
                created
            }
        };
        if saved.status == "published" && saved.visibility == "public" {
            if let Some(slug) = saved.slug {
                menu.push((page.menu, page.title.to_string(), format!("/{slug}/")));
            }
        }
    }
    note(Kind::Pages, n);

    // Members, never emailed.
    let mut n = 0;
    for member in g.members(plan.count(Kind::Members)) {
        let joined = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(member.days_ago) - chrono::Duration::minutes(g.pick(1400) as i64)));
        if let Some(id) = repo::insert_member(db, &member.name, &member.email, &joined).await? {
            repo::record(db, &batch, Kind::Members, &id).await?;
            n += 1;
        }
    }
    note(Kind::Members, n);

    // Store products.
    let mut n = 0;
    let vendor = state.settings.get_str("title").unwrap_or_default();
    for product in g.products(plan.count(Kind::Products)) {
        let input = repos::ecom::ProductInput {
            handle: vuuut_core::ecom::handleize(product.title),
            title: product.title.to_string(),
            status: if plan.publish { "active".into() } else { "draft".into() },
            description_html: vuuut_render::lexical::to_html(&product.lexical),
            description_lexical: product.lexical.clone(),
            excerpt: product.excerpt.to_string(),
            vendor: vendor.clone(),
            product_type: product.product_type.to_string(),
            tags: product.tags.clone(),
            requires_shipping: !product.digital,
            taxable: true,
            featured: product.featured,
            ..Default::default()
        };
        let variant = repos::ecom::VariantInput {
            price_cents: product.price_cents,
            compare_at_cents: product.compare_at_cents,
            track_inventory: !product.digital,
            inventory: if product.digital { 0 } else { 25 },
            ..Default::default()
        };
        let id = repos::ecom::save_product(db, None, &input, &[variant], &[]).await?;
        repo::record(db, &batch, Kind::Products, &id).await?;
        n += 1;
    }
    note(Kind::Products, n);

    // Append links while preserving any links already in either menu.
    if plan.count(Kind::Navigation) > 0 {
        if plan.count(Kind::Pages) == 0 && plan.count(Kind::Tags) == 0 {
            for page in rules::PAGES {
                if repos::posts::find_by_slug(db, page.slug, "page", true).await?
                    .is_some_and(|post| post.visibility == "public")
                {
                    menu.push((page.menu, page.title.to_string(), format!("/{}/", page.slug)));
                }
            }
            for (name, slug) in repo::public_tag_links(db, 4).await? {
                menu.push((Menu::Primary, name, format!("/tag/{slug}/")));
            }
        } else if menu.is_empty() {
            menu.extend(tag_ids.iter().take(4).map(|(_, name, slug)| (Menu::Primary, name.clone(), format!("/tag/{slug}/"))));
        }
        let mut n = 0;
        for (key, which) in [("navigation", Menu::Primary), ("secondary_navigation", Menu::Secondary)] {
            let mut links: Vec<serde_json::Value> = match state.settings.get(key) {
                Some(serde_json::Value::Array(links)) => links,
                _ => Vec::new(),
            };
            let mut added = Vec::new();
            for (_, label, url) in menu.iter().filter(|(m, _, _)| *m == which) {
                if links.iter().any(|l| l.get("url").and_then(|u| u.as_str()) == Some(url.as_str())) {
                    continue;
                }
                links.push(serde_json::json!({ "label": label, "url": url }));
                added.push((label.clone(), url.clone()));
            }
            if added.is_empty() {
                continue;
            }
            state.settings.set(db, key, &serde_json::Value::Array(links)).await?;
            for (label, url) in added {
                repo::record_with(db, &batch, Kind::Navigation, &format!("{key}|{url}"), &label).await?;
                n += 1;
            }
        }
        note(Kind::Navigation, n);
    }

    if made.is_empty() {
        // Nothing to track: leave no empty batch behind.
        let _ = repo::remove(db, &state.settings, Some(&batch)).await;
    }
    Ok(made.into_iter().map(|(k, n)| rules::counted(n, k)).collect())
}

/// A post or page, ready for `repos::posts::insert`, with its HTML rendered
/// from the Lexical document the way the editor's save would.
#[allow(clippy::too_many_arguments)]
fn content(
    title: &str,
    slug: Option<&str>,
    excerpt: &str,
    lexical: &str,
    post_type: &str,
    publish: bool,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    featured: bool,
    image: Option<&str>,
    author: &str,
    tag_ids: Vec<String>,
) -> repos::posts::NewPost {
    let plaintext = vuuut_render::lexical::to_plaintext(lexical);
    repos::posts::NewPost {
        title: title.to_string(),
        slug: slug.map(str::to_string),
        lexical: Some(lexical.to_string()),
        html: Some(vuuut_render::lexical::to_html(lexical)),
        reading_time: vuuut_render::markdown::reading_time(&plaintext),
        plaintext: Some(plaintext),
        custom_excerpt: Some(excerpt.to_string()),
        feature_image: image.map(str::to_string),
        featured,
        post_type: post_type.to_string(),
        status: if publish { "published".into() } else { "draft".into() },
        visibility: "public".into(),
        published_at: if publish { published_at.map(|d| vuuut_core::dates::to_db_string(&d)) } else { None },
        author_ids: vec![author.to_string()],
        tag_ids,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Removing
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct RemoveForm {
    /// A batch id, or "all".
    batch: String,
}

pub(crate) async fn remove(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<RemoveForm>) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let _guard = DUMMY_WRITES.lock().await;
    let batch = match form.batch.as_str() {
        "all" => None,
        id if vuuut_core::ids::is_objectid(id) && repo::batch_exists(&state.db, id).await => Some(id.to_string()),
        _ => return flash_to("error", "That batch is not on this site any more."),
    };
    match repo::remove(&state.db, &state.settings, batch.as_deref()).await {
        Ok(r) => flash_to("ok", &removal_message(&r)),
        Err(e) => {
            tracing::error!("dummy content: removal stopped: {e}");
            flash_to("error", "Removal stopped part-way. Whatever is still listed can be removed again.")
        }
    }
}

fn removal_message(r: &repo::Removal) -> String {
    let item = |n: i64| if n == 1 { "item" } else { "items" };
    let mut msg = if r.deleted > 0 { format!("Removed {} dummy {}.", r.deleted, item(r.deleted)) } else { "Nothing was removed.".to_string() };
    if r.kept > 0 {
        msg.push_str(&format!(
            " Kept {} that {} been edited or {} used by your own content; {} now ordinary content.",
            r.kept,
            if r.kept == 1 { "has" } else { "have" },
            if r.kept == 1 { "is" } else { "are" },
            if r.kept == 1 { "it is" } else { "they are" },
        ));
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_message_says_what_was_kept_and_why() {
        let r = repo::Removal { deleted: 12, kept: 2, gone: 1 };
        assert_eq!(removal_message(&r), "Removed 12 dummy items. Kept 2 that have been edited or are used by your own content; they are now ordinary content.");
        let r = repo::Removal { deleted: 1, kept: 0, gone: 0 };
        assert_eq!(removal_message(&r), "Removed 1 dummy item.");
        let r = repo::Removal { deleted: 0, kept: 1, gone: 0 };
        assert_eq!(removal_message(&r), "Nothing was removed. Kept 1 that has been edited or is used by your own content; it is now ordinary content.");
    }

    #[test]
    fn the_unmatchable_password_is_not_a_bcrypt_hash() {
        assert!(bcrypt::verify("", NO_PASSWORD).unwrap_or(false) == false);
        assert!(bcrypt::verify(NO_PASSWORD, NO_PASSWORD).unwrap_or(false) == false);
    }

    #[test]
    fn friendly_dates_fall_back_to_the_stored_text() {
        assert_eq!(friendly_date("2026-11-24 09:05:00"), "24 Nov 2026, 09:05");
        assert_eq!(friendly_date("soon"), "soon");
    }
}
