//! Property types: House, Apartment, Land… — how the catalogue is browsed.
//! Administrator and above.

use super::*;

const TYPES: &str = "/ghost/properties/types/";
const TINTS: [&str; 8] = ["blue", "teal", "green", "amber", "purple", "pink", "red", "slate"];

pub(crate) async fn types_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let paths = property_settings(&state).paths;
    let types = repo::types(&state.db).await;
    let counts = repo::type_counts(&state.db, false).await;
    let tint_opts = |current: &str| -> String {
        TINTS.iter().map(|t| format!(r#"<option value="{t}"{s}>{l}</option>"#, s = if *t == current { " selected" } else { "" }, l = t[..1].to_uppercase() + &t[1..])).collect()
    };
    let rows: String = types
        .iter()
        .map(|t| {
            format!(
                r#"<details class="dr-cat"><summary><span class="ic tint-{tint}">{ic}</span><b>{name}</b><span class="muted">{path} · {n}</span>{chev}</summary>
<form method="post" action="{TYPES}{id}/" class="dr-cat-form">{fields}
<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">{del} Delete</button><button class="btn small" type="submit">{check} Save</button></div></form>
<form method="post" action="{TYPES}{id}/delete/" id="del-{id}" data-confirm="Delete {name}|Its properties stay, without a type."></form></details>"#,
                tint = esc(&t.tint),
                ic = svg("house"),
                name = esc(&t.name),
                path = esc(&paths.public_path(&format!("/properties/type/{}/", t.slug))),
                n = countable(counts.get(&t.id).copied().unwrap_or(0), "property"),
                chev = svg("chevron"),
                id = esc(&t.id),
                fields = fields(Some(t), &tint_opts(&t.tint)),
                del = svg("delete"),
                check = svg("check"),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Property types",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="dr-split dr-split-even">
<div><div class="section">Types</div><div class="card padded rise" style="--i:2">{list}</div></div>
<div><div class="section">Add a type</div><form class="card padded rise" style="--i:3" method="post" action="{TYPES}">{new_fields}
<div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{add} Add type</button></div></form></div>
</div>"#,
            head = crate::tools::tool_head_back("properties", BASE, None, ""),
            tabs = tabs_for(&state, "types", &actor).await,
            banner = flash_banner(&q),
            list = if types.is_empty() { r#"<p class="muted" style="margin:0">None yet. Most sites start with House, Apartment, Townhouse, Land and Commercial.</p>"#.to_string() } else { rows },
            new_fields = fields(None, &tint_opts("blue")),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

fn fields(t: Option<&repo::PropertyType>, tints: &str) -> String {
    format!(
        r#"<label class="field"><input name="name" value="{n}" required maxlength="80" placeholder=" "><span>Name</span></label>
<div class="wf-two"><label class="field"><input name="slug" value="{s}" maxlength="80" placeholder=" "><span>Address (blank: from the name)</span></label>
<label class="field fixed"><select name="tint">{tints}</select><span>Colour</span>{caret}</label></div>
<label class="field"><input name="description" value="{d}" maxlength="300" placeholder=" "><span>Description, shown on its page</span></label>
<label class="field"><input name="image" value="{img}" maxlength="500" placeholder=" "><span>Image address — the type's tile on the site</span></label>
<label class="field" style="max-width:160px"><input type="number" name="sort_order" value="{o}" min="0" max="999" placeholder=" "><span>Order</span></label>"#,
        n = esc(t.map(|c| c.name.as_str()).unwrap_or("")),
        s = esc(t.map(|c| c.slug.as_str()).unwrap_or("")),
        d = esc(t.map(|c| c.description.as_str()).unwrap_or("")),
        o = t.map(|c| c.sort_order).unwrap_or(0),
        img = esc(t.map(|c| c.image.as_str()).unwrap_or("")),
        caret = SELECT_CARET,
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct TypeForm {
    name: String,
    slug: String,
    tint: String,
    description: String,
    sort_order: String,
    image: String,
}

async fn save(state: &AppState, id: Option<&str>, f: &TypeForm) -> Response {
    use vuuut_core::directory::{clean, safe_image, slugify};
    let name = clean(&f.name, 80);
    if name.is_empty() {
        return flash_to(TYPES, "error", "A type needs a name.");
    }
    let slug = slugify(if f.slug.trim().is_empty() { &name } else { &f.slug });
    if slug.is_empty() {
        return flash_to(TYPES, "error", "That address is not usable.");
    }
    if repo::type_slug_taken(&state.db, &slug, id).await {
        return flash_to(TYPES, "error", "Another type already uses that address.");
    }
    let tint = if TINTS.contains(&f.tint.as_str()) { f.tint.as_str() } else { "blue" };
    let sort = f.sort_order.trim().parse::<i64>().unwrap_or(0).clamp(0, 999);
    let Some(image) = safe_image(&f.image) else {
        return flash_to(TYPES, "error", "The image must be an http(s) address or an uploaded image.");
    };
    match repo::save_type(&state.db, id, &name, &slug, &clean(&f.description, 300), &image, tint, sort).await {
        Ok(_) => flash_to(TYPES, "ok", &format!("{name} saved.")),
        Err(e) => {
            tracing::error!("properties: saving a type failed: {}", e.message());
            flash_to(TYPES, "error", "That could not be saved.")
        }
    }
}

pub(crate) async fn type_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<TypeForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    save(&state, None, &f).await
}

pub(crate) async fn type_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<TypeForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(TYPES, "error", "That type could not be found.");
    }
    save(&state, Some(&id), &f).await
}

pub(crate) async fn type_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(TYPES, "error", "That type could not be found.");
    }
    match repo::delete_type(&state.db, &id).await {
        Ok(()) => flash_to(TYPES, "ok", "Type deleted. Its properties now have no type."),
        Err(_) => flash_to(TYPES, "error", "That could not be deleted."),
    }
}
