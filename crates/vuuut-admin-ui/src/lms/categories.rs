//! Course categories: how the catalogue is grouped. Administrator and above.

use super::*;

const CATS: &str = "/ghost/lms/categories/";
const TINTS: [&str; 8] = ["blue", "teal", "green", "amber", "purple", "pink", "red", "slate"];

pub(crate) async fn categories_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = lms_settings(&state);
    let cats = repo::categories(&state.db).await.unwrap_or_default();
    let counts = repo::category_counts(&state.db).await;
    let tint_opts = |current: &str| -> String {
        TINTS.iter().map(|t| format!(r#"<option value="{t}"{s}>{l}</option>"#, s = if *t == current { " selected" } else { "" }, l = t[..1].to_uppercase() + &t[1..])).collect()
    };
    let rows: String = cats
        .iter()
        .map(|c| {
            format!(
                r#"<details class="dr-cat"><summary><span class="ic tint-{tint}">{ic}</span><b>{name}</b><span class="muted">{path} · {n} live</span>{chev}</summary>
<form method="post" action="{CATS}{id}/" class="dr-cat-form">{fields}
<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">{del} Delete</button><button class="btn small" type="submit">{check} Save</button></div></form>
<form method="post" action="{CATS}{id}/delete/" id="del-{id}" data-confirm="Delete {name}|Its courses stay, without a category."></form></details>"#,
                tint = esc(&c.tint),
                ic = svg("folder"),
                name = esc(&c.name),
                path = esc(&s.paths.public_path(&format!("/courses/category/{}/", c.slug))),
                n = counts.get(&c.id).copied().unwrap_or(0),
                chev = svg("chevron"),
                id = esc(&c.id),
                fields = fields(Some(c), &tint_opts(&c.tint)),
                del = svg("delete"),
                check = svg("check"),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Course categories",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="dr-split dr-split-even">
<div><div class="section">Categories</div><div class="card padded rise" style="--i:2">{list}</div></div>
<div><div class="section">Add a category</div><form class="card padded rise" style="--i:3" method="post" action="{CATS}">{new_fields}
<div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{add} Add category</button></div></form></div>
</div>"#,
            head = crate::tools::tool_head_back("lms", BASE, None, ""),
            tabs = tabs_for(&state, "categories", &actor).await,
            banner = flash_banner(&q),
            list = if cats.is_empty() { r#"<p class="muted" style="margin:0">None yet. Start with a few subjects — Design, Development, Business, Wellbeing.</p>"#.to_string() } else { rows },
            new_fields = fields(None, &tint_opts("blue")),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

fn fields(c: Option<&repo::Category>, tints: &str) -> String {
    format!(
        r#"<label class="field"><input name="name" value="{n}" required maxlength="80" placeholder=" "><span>Name</span></label>
<div class="wf-two"><label class="field"><input name="slug" value="{s}" maxlength="80" placeholder=" "><span>Address (blank: from the name)</span></label>
<label class="field fixed"><select name="tint">{tints}</select><span>Colour</span>{caret}</label></div>
<label class="field"><input name="description" value="{d}" maxlength="300" placeholder=" "><span>Description, shown on its page</span></label>
<label class="field"><input name="image" value="{img}" maxlength="500" placeholder=" "><span>Image address — the category's tile on the site</span></label>
<label class="field" style="max-width:160px"><input type="number" name="sort_order" value="{o}" min="0" max="999" placeholder=" "><span>Order</span></label>"#,
        n = esc(c.map(|c| c.name.as_str()).unwrap_or("")),
        s = esc(c.map(|c| c.slug.as_str()).unwrap_or("")),
        d = esc(c.map(|c| c.description.as_str()).unwrap_or("")),
        o = c.map(|c| c.sort_order).unwrap_or(0),
        img = esc(c.map(|c| c.image.as_str()).unwrap_or("")),
        caret = SELECT_CARET,
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct CategoryForm {
    name: String,
    slug: String,
    tint: String,
    description: String,
    sort_order: String,
    image: String,
}

async fn save(state: &AppState, id: Option<&str>, f: &CategoryForm) -> Response {
    let name = vuuut_core::directory::clean(&f.name, 80);
    if name.is_empty() {
        return flash_to(CATS, "error", "A category needs a name.");
    }
    let slug = vuuut_core::directory::slugify(if f.slug.trim().is_empty() { &name } else { &f.slug });
    if slug.is_empty() {
        return flash_to(CATS, "error", "That address is not usable.");
    }
    if repo::category_slug_taken(&state.db, &slug, id).await {
        return flash_to(CATS, "error", "Another category already uses that address.");
    }
    let tint = if TINTS.contains(&f.tint.as_str()) { f.tint.as_str() } else { "blue" };
    let sort = f.sort_order.trim().parse::<i64>().unwrap_or(0).clamp(0, 999);
    let Some(image) = vuuut_core::directory::safe_image(&f.image) else {
        return flash_to(CATS, "error", "The image must be an http(s) address or an uploaded image.");
    };
    match repo::save_category(&state.db, id, &name, &slug, &vuuut_core::directory::clean(&f.description, 300), tint, &image, sort).await {
        Ok(_) => flash_to(CATS, "ok", &format!("{name} saved.")),
        Err(e) => {
            tracing::error!("lms: saving a category failed: {}", e.message());
            flash_to(CATS, "error", "That could not be saved.")
        }
    }
}

pub(crate) async fn category_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<CategoryForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    save(&state, None, &f).await
}

pub(crate) async fn category_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<CategoryForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(CATS, "error", "That category could not be found.");
    }
    save(&state, Some(&id), &f).await
}

pub(crate) async fn category_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(CATS, "error", "That category could not be found.");
    }
    match repo::delete_category(&state.db, &id).await {
        Ok(()) => flash_to(CATS, "ok", "Category deleted. Its courses are now uncategorised."),
        Err(_) => flash_to(CATS, "error", "That could not be deleted."),
    }
}
