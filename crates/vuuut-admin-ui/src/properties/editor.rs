//! The property editor: the same two-column editor as a store product, a
//! directory listing or a course. Down the middle: the title and the
//! description in Koenig, photos (drag to order; the first is the cover),
//! the facts (rooms, sizes, year, energy), the location with a live map,
//! features, floor plans, open houses, video / virtual tour / brochure, and
//! the search-engine listing. Down the side: status and market, price,
//! organisation (type, agent, reference), numbers and actions.
//!
//! The form posts without a script. `ecom-editor.js` adds the Koenig canvas,
//! the photo manager, the search preview and "Unsaved changes";
//! `properties-editor.js` adds the sale/rent switch, repeatable floor-plan
//! and open-house rows with uploads, and the map preview. Neither is the
//! authority: `PropertyInput::from_form` rebuilds everything that arrives.

use super::*;

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

fn area(name: &str, label: &str, value: &str, rows: u8, extra: &str) -> String {
    format!(r#"<label class="field"><textarea name="{name}" rows="{rows}" placeholder=" "{extra}>{v}</textarea><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

fn card(title: &str, icon: &str, tint: &str, body: &str, id: &str) -> String {
    format!(r#"<section class="card padded ec-card" id="{id}"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title))
}

fn check(name: &str, value: &str, label: &str, on: bool) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="{v}"{c}> {l}</label>"#, v = esc(value), c = if on { " checked" } else { "" }, l = esc(label))
}

fn select(name: &str, label: &str, opts: &[(String, String)], current: &str) -> String {
    let o: String = opts.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), s = if v == current { " selected" } else { "" }, l = esc(l))).collect();
    format!(r#"<label class="field fixed"><select name="{name}">{o}</select><span>{l}</span>{SELECT_CARET}</label>"#, l = esc(label))
}

fn money(cents: i64) -> String {
    if cents <= 0 {
        String::new()
    } else if cents % 100 == 0 {
        (cents / 100).to_string()
    } else {
        format!("{}.{:02}", cents / 100, cents % 100)
    }
}

fn num(n: i64) -> String {
    if n > 0 { n.to_string() } else { String::new() }
}

pub(crate) async fn listing_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn listing_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That property could not be found.");
    }
    let Some(p) = repo::find(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That property could not be found.");
    };
    editor(&state, &actor, Some(p), &q).await
}

/// A floor-plan row: a picture (uploaded or typed), its name and its size.
fn plan_row(p: Option<&rules::FloorPlan>) -> String {
    let img = p.map(|p| p.image.as_str()).unwrap_or("");
    format!(
        r#"<div class="pr-plan" data-row>
<div class="pr-plan-pic">{preview}</div>
<div class="pr-plan-fields"><div class="ec-grid2">{t}{s}</div>
<div class="ec-row"><input class="ec-in pr-plan-url" name="fp_image" value="{img}" placeholder="Picture: https://… or upload" maxlength="500" aria-label="Floor plan picture">
<label class="btn tonal small pr-upload">{up} Upload<input type="file" accept="image/*" hidden></label>
<button type="button" class="icon-btn" data-remove-row title="Remove this floor plan">{del}</button></div></div></div>"#,
        preview = if img.is_empty() { String::new() } else { format!(r#"<img src="{}" alt="">"#, esc(img)) },
        t = field("fp_title", "Name — e.g. Ground floor", p.map(|p| p.title.as_str()).unwrap_or(""), "text", r#" maxlength="80""#),
        s = field("fp_size", "Size — e.g. 85 m²", p.map(|p| p.size.as_str()).unwrap_or(""), "text", r#" maxlength="40""#),
        img = esc(img),
        up = svg("upload"),
        del = svg("delete"),
    )
}

fn open_row(o: Option<&rules::OpenHouse>) -> String {
    format!(
        r#"<div class="pr-open" data-row>{d}{s}{e}<button type="button" class="icon-btn" data-remove-row title="Remove this open house">{del}</button></div>"#,
        d = field("oh_date", "Date", o.map(|o| o.date.as_str()).unwrap_or(""), "date", ""),
        s = field("oh_start", "From", o.map(|o| o.start.as_str()).unwrap_or(""), "time", ""),
        e = field("oh_end", "Until", o.map(|o| o.end.as_str()).unwrap_or(""), "time", ""),
        del = svg("delete"),
    )
}

async fn editor(state: &AppState, actor: &Actor, p: Option<repo::Property>, q: &HashMap<String, String>) -> Response {
    let s = property_settings(state);
    let types = repo::types(&state.db).await;
    let agents = repo::agents(&state.db, false).await;
    let get = |f: fn(&repo::Property) -> &str| p.as_ref().map(f).unwrap_or("").to_string();
    let n = |f: fn(&repo::Property) -> i64| p.as_ref().map(f).unwrap_or(0);
    let purpose = p.as_ref().map(|p| p.purpose()).unwrap_or(Purpose::Sale);
    let market = p.as_ref().map(|p| p.market()).unwrap_or(Market::Available);
    let status = Status::from_key(p.as_ref().map(|p| p.status.as_str()).unwrap_or("draft"));
    let action = match &p {
        Some(p) => format!("{LISTINGS}{}/", esc(&p.id)),
        None => format!("{LISTINGS}new/"),
    };
    let unit = s.area_unit.label();

    let lexical = get(|p| &p.description_lexical);
    let legacy_html = p.as_ref().map(|p| p.description_markup()).unwrap_or_default();
    let initial = if !lexical.trim().is_empty() { Some(lexical) } else { vuuut_render::lexical::wrap_existing(Some(""), Some(&legacy_html)) };
    let initial = initial.map(|x| vuuut_core::url::base_assets(&vuuut_core::url::unbase_assets(&x)));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": "Describe the property — the light, the layout, the street, what is nearby…",
        "siteUrl": state.site_url(),
        "siteTitle": state.settings.get_str("title").unwrap_or_default(),
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false,
        "stripeEnabled": false,
        "basePath": vuuut_core::url::subdir(),
        "unsplash": serde_json::Value::Null,
        "pinturaConfig": serde_json::Value::Null,
    });

    // Photos.
    let images = p.as_ref().map(|p| p.images()).unwrap_or_default();
    let photos = format!(
        r#"<div class="ec-media pr-media" id="ec-media">{items}<label class="ec-media-add" title="Upload photos"><input type="file" accept="image/*" id="ec-upload" multiple>{plus}<span>Add photos</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>
<p class="muted ec-hint">Drag to reorder. The first photo is the cover on cards and at the top of the page; the rest make the gallery. Landscape photos at 1600px or wider look best.</p>"#,
        items = images.iter().map(|u| crate::ecom::media_item(u)).collect::<String>(),
        plus = svg("add"),
    );

    // Facts.
    let energy: Vec<(String, String)> = std::iter::once((String::new(), "Not rated".to_string())).chain(rules::ENERGY_RATINGS.iter().map(|r| (r.to_string(), r.to_string()))).collect();
    let facts = format!(
        r#"<div class="pr-grid4">{beds}{baths}{half}{rooms}</div>
<div class="pr-grid4">{area}{lot}{parking}{floors}</div>
<div class="pr-grid4">{year}{energy}{avail}<span></span></div>
<p class="muted ec-hint">Sizes are in {unit} (change the unit in Settings). Leave anything you do not know blank — it is simply not shown.</p>"#,
        beds = field("bedrooms", "Bedrooms", &num(n(|p| p.bedrooms)), "number", r#" min="0" max="99""#),
        baths = field("bathrooms", "Bathrooms", &num(n(|p| p.bathrooms)), "number", r#" min="0" max="99""#),
        half = field("half_baths", "Half baths / WCs", &num(n(|p| p.half_baths)), "number", r#" min="0" max="20""#),
        rooms = field("rooms", "Rooms in all", &num(n(|p| p.rooms)), "number", r#" min="0" max="999""#),
        area = field("area", &format!("Floor area ({unit})"), &num(n(|p| p.area)), "number", r#" min="0""#),
        lot = field("lot_area", &format!("Plot / lot ({unit})"), &num(n(|p| p.lot_area)), "number", r#" min="0""#),
        parking = field("parking", "Parking spaces", &num(n(|p| p.parking)), "number", r#" min="0" max="999""#),
        floors = field("floors", "Storeys", &num(n(|p| p.floors)), "number", r#" min="0" max="200""#),
        year = field("year_built", "Year built", &num(n(|p| p.year_built)), "number", r#" min="1000" max="2100""#),
        energy = select("energy_rating", "Energy rating", &energy, &get(|p| &p.energy_rating)),
        avail = field("available_from", "Available from", &get(|p| &p.available_from), "date", ""),
    );

    // Location.
    let coords = p.as_ref().and_then(|p| Some((p.latitude.parse::<f64>().ok()?, p.longitude.parse::<f64>().ok()?)));
    let location = format!(
        r#"{addr}
<div class="ec-grid2">{hood}{city}</div>
<div class="pr-grid3">{region}{postal}{country}</div>
{hide}
<div class="pr-map-wrap"><div class="ec-grid2">{lat}{lng}</div>
<p class="muted ec-hint">The pin on the map. <a href="https://www.openstreetmap.org/" id="pr-find-coords" target="_blank" rel="noopener">Find the address on OpenStreetMap</a>, right-click the spot, choose “Show address”, and copy the two numbers here.</p>
<div class="pr-map" id="pr-map">{map}</div></div>"#,
        addr = field("address", "Street address", &get(|p| &p.address), "text", r#" maxlength="120" id="pr-address""#),
        hood = field("neighborhood", "Neighbourhood / area", &get(|p| &p.neighborhood), "text", r#" maxlength="80""#),
        city = field("city", "Town or city", &get(|p| &p.city), "text", r#" maxlength="80" id="pr-city" list="pr-cities""#),
        region = field("region", "County / state", &get(|p| &p.region), "text", r#" maxlength="80""#),
        postal = field("postal_code", "Postcode / ZIP", &get(|p| &p.postal_code), "text", r#" maxlength="20""#),
        country = field("country", "Country", &get(|p| &p.country), "text", r#" maxlength="60" id="pr-country""#),
        hide = check("hide_address", "1", "Hide the street address on the site — show the area only, with an approximate pin", n(|p| p.hide_address) != 0),
        lat = field("latitude", "Latitude", &get(|p| &p.latitude), "text", r#" inputmode="decimal" maxlength="20" id="pr-lat" placeholder="51.5072""#),
        lng = field("longitude", "Longitude", &get(|p| &p.longitude), "text", r#" inputmode="decimal" maxlength="20" id="pr-lng" placeholder="-0.1276""#),
        map = coords.map(|(lat, lng)| format!(r#"<iframe title="Map" loading="lazy" src="{}"></iframe>"#, esc(&repo::map_embed(lat, lng, true)))).unwrap_or_else(|| r#"<span class="muted">The map appears when the pin is set.</span>"#.to_string()),
    );

    // Features.
    let chosen = p.as_ref().map(|p| p.feature_keys()).unwrap_or_default();
    let mut groups: Vec<&str> = Vec::new();
    for f in rules::FEATURES {
        if !groups.contains(&f.2) {
            groups.push(f.2);
        }
    }
    let features = format!(
        r#"{groups}{extra}"#,
        groups = groups
            .iter()
            .map(|g| format!(
                r#"<div class="ec-sublabel">{g}</div><div class="dr-checks pr-checks">{}</div>"#,
                rules::FEATURES.iter().filter(|f| f.2 == *g).map(|f| check("feature", f.0, f.1, chosen.contains(&f.0))).collect::<String>()
            ))
            .collect::<String>(),
        extra = area("extra_features", "Anything else — one per line (e.g. “South-facing garden”, “Walk to the station”)", &get(|p| &p.extra_features), 3, r#" maxlength="2000""#),
    );

    // Floor plans and open houses.
    let plans = p.as_ref().map(|p| p.floor_plan_list()).unwrap_or_default();
    let floor_plans = format!(
        r#"<div class="pr-rows" id="pr-plans">{rows}</div><template id="pr-plan-tpl">{tpl}</template>
<button type="button" class="btn tonal small" data-add-row="pr-plans">{add} Add a floor plan</button>"#,
        rows = if plans.is_empty() { plan_row(None) } else { plans.iter().map(|x| plan_row(Some(x))).collect() },
        tpl = plan_row(None),
        add = svg("add"),
    );
    let opens = p.as_ref().map(|p| p.open_house_list()).unwrap_or_default();
    let open_houses = format!(
        r#"<div class="pr-rows" id="pr-opens">{rows}</div><template id="pr-open-tpl">{tpl}</template>
<button type="button" class="btn tonal small" data-add-row="pr-opens">{add} Add an open house</button>
<p class="muted ec-hint">Upcoming times are listed on the property page and its card; past ones drop off by themselves.</p>"#,
        rows = if opens.is_empty() { open_row(None) } else { opens.iter().map(|x| open_row(Some(x))).collect() },
        tpl = open_row(None),
        add = svg("add"),
    );

    let media = format!(
        "{video}{tour}<div class=\"ec-row pr-brochure\">{brochure}<label class=\"btn tonal small pr-upload\" data-target=\"pr-brochure\">{up} Upload PDF<input type=\"file\" accept=\"application/pdf\" hidden></label></div><p class=\"muted ec-hint\">Video: YouTube, Vimeo, Loom or Wistia. Virtual tour: Matterport, Kuula, Momento360 or a YouTube 360 link is shown in the page; any other address becomes a button.</p>",
        video = field("video_url", "Video tour", &get(|p| &p.video_url), "url", r#" maxlength="500" placeholder="https://""#),
        tour = field("tour_url", "3D / virtual tour", &get(|p| &p.tour_url), "url", r#" maxlength="500" placeholder="https://""#),
        brochure = field("brochure_url", "Brochure (PDF)", &get(|p| &p.brochure_url), "text", r#" maxlength="500" id="pr-brochure" placeholder="https://… or upload""#),
        up = svg("upload"),
    );

    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}{prefix}<b>{slug}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>
{t}{d}{h}{kw}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        prefix = esc(&s.paths.public_path("/properties/")),
        slug = esc(&get(|p| &p.slug)),
        st = esc(&{ let t = get(|p| &p.seo_title); if t.is_empty() { get(|p| &p.title) } else { t } }),
        sd = esc(&{ let d = get(|p| &p.seo_description); if d.is_empty() { get(|p| &p.summary) } else { d } }),
        t = field("seo_title", "Page title", &get(|p| &p.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = area("seo_description", "Meta description", &get(|p| &p.seo_description), 2, r#" maxlength="320" id="ec-seo-desc""#),
        h = field("slug", "Address on the site", &get(|p| &p.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*" id="ec-handle""#),
        kw = field("keywords", "Keywords for search (not shown)", &get(|p| &p.keywords), "text", r#" maxlength="200""#),
    );

    // Side: status.
    let status_opts: Vec<(String, String)> = Status::ALL.iter().map(|x| (x.key().to_string(), x.label().to_string())).collect();
    let market_opts: Vec<(String, String)> = Market::ALL.iter().map(|x| (x.key().to_string(), x.label().to_string())).collect();
    let view = p.as_ref().filter(|p| p.is_published() && s.enabled).map(|p| format!(r#"<a class="btn text small" href="{}" target="_blank" rel="noopener">{} View on the site</a>"#, public_href(&s, &format!("/properties/{}/", p.slug)), svg("open"))).unwrap_or_default();
    let status_card = card("Status", "eye", "green", &format!(
        r#"{chips}{sel}{mk}<p class="muted ec-hint">Only published properties are on the site{off}. Sold and let ones stay, marked{sold}.</p>{feat}{view}"#,
        chips = p.as_ref().map(|p| { let (k, l) = status_chip(&p.status); format!(r#"<p style="margin:0 0 10px"><span class="chip {k}">{l}</span> {}</p>"#, market_chip(p)) }).unwrap_or_default(),
        sel = select("status", "Status", &status_opts, status.key()),
        mk = select("market", "On the market", &market_opts, market.key()),
        off = if s.enabled { "" } else { " — and the catalogue itself is switched off in Settings" },
        sold = if s.show_sold { "" } else { " (switched off in Settings: they leave the site)" },
        feat = check("featured", "1", "Featured — shown first and marked", n(|p| p.featured) != 0),
    ), "pr-status");

    // Side: price.
    let kinds: String = Purpose::ALL.iter().map(|x| format!(
        r#"<label class="dr-kind pr-kind"><input type="radio" name="purpose" value="{k}"{c}><b>{l}</b></label>"#,
        k = x.key(), c = if *x == purpose { " checked" } else { "" }, l = esc(x.label()),
    )).collect();
    let prefixes: Vec<(String, String)> = ["", "Offers over", "Offers in the region of", "Guide price", "From", "Fixed price", "Starting at"].iter().map(|x| (x.to_string(), if x.is_empty() { "No wording".to_string() } else { x.to_string() })).collect();
    let periods: Vec<(String, String)> = rules::RENT_PERIODS.iter().map(|(k, l, _)| (k.to_string(), l.to_string())).collect();
    let price = card("Price", "sell", "amber", &format!(
        r#"<div class="pr-kinds">{kinds}</div>
<div class="ec-grid2">{price}<span class="pr-only-rent">{period}</span><span class="pr-only-sale">{prefix}</span></div>
{poa}
<div class="pr-only-rent">{deposit}</div>
<div class="ec-grid2">{service}{tax}</div>
<p class="muted ec-hint">In {cur} (set in Payments). Service charge is a month; property tax a year.</p>"#,
        price = field("price", "Price", &money(n(|p| p.price_cents)), "text", r#" inputmode="decimal" id="pr-price""#),
        period = select("rent_period", "Per", &periods, &{ let r = get(|p| &p.rent_period); if r.is_empty() { "month".into() } else { r } }),
        prefix = select("price_prefix", "Wording", &prefixes, &get(|p| &p.price_prefix)),
        poa = check("price_poa", "1", "Show “Price on application” instead", n(|p| p.price_poa) != 0),
        deposit = field("deposit", "Deposit", &money(n(|p| p.deposit_cents)), "text", r#" inputmode="decimal""#),
        service = field("service_charge", "Service charge / HOA", &money(n(|p| p.service_charge_cents)), "text", r#" inputmode="decimal""#),
        tax = field("tax", "Property tax", &money(n(|p| p.tax_cents)), "text", r#" inputmode="decimal""#),
        cur = esc(&s.currency),
    ), "pr-price-card");

    // Side: organisation.
    let mut type_opts = vec![(String::new(), "No type".to_string())];
    type_opts.extend(types.iter().map(|t| (t.id.clone(), t.name.clone())));
    let mut agent_opts = vec![(String::new(), "No agent shown".to_string())];
    agent_opts.extend(agents.iter().map(|a| (a.id.clone(), if a.is_active() { a.name.clone() } else { format!("{} (hidden)", a.name) })));
    let org = card("Organisation", "folder", "blue", &format!(
        "{t}{a}{r}{links}",
        t = select("type_id", "Type", &type_opts, &get(|p| p.type_id.as_deref().unwrap_or(""))),
        a = select("agent_id", "Agent", &agent_opts, &get(|p| p.agent_id.as_deref().unwrap_or(""))),
        r = field("reference", "Reference / MLS number", &get(|p| &p.reference), "text", r#" maxlength="40""#),
        links = if actor.role.is_admin() && types.is_empty() {
            r#"<p class="muted ec-hint"><a href="/ghost/properties/types/">Add types</a> — House, Apartment, Land… — to group properties.</p>"#.to_string()
        } else if agents.is_empty() {
            format!(r#"<p class="muted ec-hint"><a href="{AGENTS}new/">Add an agent</a> to show who to contact.</p>"#)
        } else {
            String::new()
        },
    ), "pr-org");

    let numbers = match &p {
        Some(prop) => card("Numbers", "chart", "green", &format!(
            r#"<div class="ec-kpis lm-kpis"><div><b>{views}</b><span class="muted">page views</span></div><div><b>{enq}</b><span class="muted">enquiries</span></div></div>
<p class="muted ec-hint"><a href="{ENQUIRIES}?property={id}">See its enquiries</a> · listed {created}{closed}</p>"#,
            views = prop.views,
            enq = prop.enquiries,
            id = esc(&prop.id),
            created = date(&prop.created_at),
            closed = prop.closed_at.as_deref().map(|c| format!(" · {} {}", if prop.purpose() == Purpose::Rent { "let" } else { "sold" }, date(c))).unwrap_or_default(),
        ), "pr-numbers"),
        None => String::new(),
    };
    let actions = match &p {
        Some(_) => format!(
            r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text" type="submit" form="pr-duplicate">{dup} Duplicate property</button><button class="btn text danger" type="submit" form="pr-delete">{del} Delete property</button></div></section>"#,
            dup = svg("copy"),
            del = svg("delete"),
        ),
        None => String::new(),
    };

    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{summary}
<div class="ec-sublabel">Description</div>
<div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="description_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback"><label class="field"><textarea name="description" rows="8" placeholder=" " maxlength="20000">{plain}</textarea><span>Description (plain text)</span></label></div>
</section>
{photos}{facts}{location}{features}{plans}{opens}{media}{seo}"#,
        title = field("title", "Title — e.g. Bright 3-bed townhouse with garden", &get(|p| &p.title), "text", r#" required maxlength="160" id="ec-title""#),
        summary = field("summary", "One line for cards and search results", &get(|p| &p.summary), "text", r#" maxlength="300""#),
        config = esc(&config.to_string()),
        plain = esc(&get(|p| &p.description)),
        photos = card("Photos", "image", "pink", &photos, "pr-photos"),
        facts = card("Details", "house", "blue", &facts, "pr-facts"),
        location = card("Location", "pin", "red", &location, "pr-location"),
        features = card("Features", "check", "green", &features, "pr-features"),
        plans = card("Floor plans", "layout", "purple", &floor_plans, "pr-plans-card"),
        opens = card("Open houses", "calendar", "teal", &open_houses, "pr-opens-card"),
        media = card("Video, virtual tour & brochure", "play", "amber", &media, "pr-media-card"),
        seo = card("Search engine listing", "search", "slate", &seo, "pr-seo"),
    );
    let side = format!("{status_card}{price}{org}{numbers}{actions}");

    let forms = match &p {
        Some(prop) => format!(
            r#"<form method="post" action="{LISTINGS}{id}/delete/" id="pr-delete" data-confirm="Delete “{t}”|Its photos stay in the media library and its enquiries stay in Enquiries. It cannot be undone."></form>
<form method="post" action="{LISTINGS}{id}/duplicate/" id="pr-duplicate"></form>"#,
            id = esc(&prop.id),
            t = esc(&prop.title),
        ),
        None => String::new(),
    };
    let cities: String = repo::cities(&state.db).await.iter().map(|(c, _)| format!(r#"<option value="{}">"#, esc(c))).collect();
    let heading = p.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| "New property".into());
    let sub = match &p {
        Some(prop) => [prop.price_label(&s.currency), prop.short_location(), specs(&s, prop)].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
        None => "Photos, price, rooms, the address and pin — then publish.".into(),
    };
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{LISTINGS}" title="Back to properties">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pr-property">{check} {save}</button></div></header>
{banner}{forms}<datalist id="pr-cities">{cities}</datalist>
<form id="pr-property" class="ec-edit pr-edit pr-purpose-{purpose}" method="post" action="{action}">
<div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script>
<script src="{ASSETS}/react-jsx.js" defer></script>
<script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script>
<script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script>
<script src="{ASSETS}/properties-editor.js?v={hash}" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&sub),
        check = svg("check"),
        save = if p.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        purpose = purpose.key(),
        hash = editor_script_hash(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

fn editor_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../../assets/properties-editor.js")));
    &HASH
}

/// The property form, rebuilt: the fields through `PropertyInput::from_form`,
/// the description through the post renderer.
async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::PropertyInput, String> {
    let types = repo::types(&state.db).await;
    let agents = repo::agents(&state.db, false).await;
    let mut input = rules::PropertyInput::from_form(pairs, |kind, id| match kind {
        "type" => types.iter().any(|t| t.id == id),
        "agent" => agents.iter().any(|a| a.id == id),
        _ => false,
    })?;
    let currency = property_settings(state).currency;
    for price in [input.price_cents, input.deposit_cents, input.service_charge_cents, input.tax_cents] {
        if vuuut_core::currency::fixed_price_to_minor(price, &currency).is_none() {
            return Err("Enter whole prices for this currency.".into());
        }
    }
    let lexical = pairs.iter().find(|(k, _)| k == "description_lexical").map(|(_, v)| v.clone()).unwrap_or_default();
    if lexical.trim().starts_with('{') {
        let lexical = vuuut_core::url::unbase_assets(&lexical);
        let html = vuuut_render::lexical::to_html(&lexical);
        if lexical.len() > rules::MAX_RICH || html.len() > rules::MAX_RICH {
            return Err("The description is too long.".into());
        }
        let plain = vuuut_render::markdown::html_to_plaintext(&html);
        input.description = vuuut_core::directory::clean_text(&plain, rules::MAX_PLAIN);
        let empty = plain.trim().is_empty() && !html.contains("<img") && !html.contains("<iframe");
        input.description_html = if empty { String::new() } else { html };
        input.description_lexical = if empty { String::new() } else { lexical };
    }
    Ok(input)
}

pub(crate) async fn listing_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{LISTINGS}new/"), "error", &m),
    };
    match repo::insert(&state.db, &input).await {
        Ok(id) => flash_to(&format!("{LISTINGS}{id}/"), "ok", if input.status == "published" { "Property created and published." } else { "Property created as a draft." }),
        Err(e) => {
            tracing::error!("properties: creating a property failed: {}", e.message());
            flash_to(&format!("{LISTINGS}new/"), "error", "The property could not be saved.")
        }
    }
}

pub(crate) async fn listing_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) || repo::find(&state.db, &id).await.is_none() {
        return flash_to(LISTINGS, "error", "That property could not be found.");
    }
    let back = format!("{LISTINGS}{id}/");
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::update(&state.db, &id, &input).await {
        Ok(()) => flash_to(&back, "ok", "Saved."),
        Err(e) => {
            tracing::error!("properties: saving {id} failed: {}", e.message());
            flash_to(&back, "error", "The property could not be saved.")
        }
    }
}

pub(crate) async fn listing_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LISTINGS, "error", "That property could not be found.");
    }
    match repo::delete(&state.db, &id).await {
        Ok(()) => flash_to(LISTINGS, "ok", "Property deleted."),
        Err(_) => flash_to(LISTINGS, "error", "That could not be deleted."),
    }
}

pub(crate) async fn listing_duplicate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LISTINGS, "error", "That property could not be found.");
    }
    match repo::duplicate(&state.db, &id).await {
        Ok(Some(new_id)) => flash_to(&format!("{LISTINGS}{new_id}/"), "ok", "Duplicated as a draft."),
        _ => flash_to(&format!("{LISTINGS}{id}/"), "error", "That could not be duplicated."),
    }
}
