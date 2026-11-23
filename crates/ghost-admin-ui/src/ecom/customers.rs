//! Customers: everyone who has bought, or been added by staff.
//!
//! A customer is keyed on email and linked to the site's member with the
//! same address when there is one. Order counts and totals are computed
//! from the orders each time, never stored.

use super::*;

const CUSTOMERS: &str = "/ghost/ecom/customers/";

pub(crate) async fn customers_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let marketing = q.get("marketing").is_some();
    let page_no: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).clamp(1, 10_000);
    let per = 50;
    let (list, total) = repo::browse_customers(&state.db, &terms, marketing, per, (page_no - 1) * per).await;
    let stats = repo::customer_stats(&state.db, &list.iter().map(|c| c.id.clone()).collect::<Vec<_>>()).await;
    let rows: String = list
        .iter()
        .map(|c| {
            let st = stats.get(&c.id).cloned().unwrap_or((0, 0, String::new()));
            let a = c.address();
            format!(
                r#"<tr><td><a href="{CUSTOMERS}{id}/"><b>{n}</b></a><br><small class="muted">{e}</small></td><td>{mk}</td><td>{loc}</td><td>{o}</td><td class="ec-num">{spent}</td><td>{last}</td></tr>"#,
                id = esc(&c.id),
                n = esc(&c.name()),
                e = esc(&c.email),
                mk = if c.accepts_marketing != 0 { r#"<span class="chip published">Subscribed</span>"# } else { r#"<span class="chip">Not subscribed</span>"# },
                loc = esc(&[a.city.as_str(), rules::country_name(&a.country)].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(", ")),
                o = countable(st.0, "order"),
                spent = money(st.1, &s),
                last = if st.2.is_empty() { "—".to_string() } else { esc(st.2.get(..10).unwrap_or(&st.2)) },
            )
        })
        .collect();
    let pages = (total + per - 1) / per;
    let pager = if pages > 1 {
        let mk = |n: i64| format!("?q={}{}&amp;page={n}", ghost_core::social_media::encode(&query), if marketing { "&amp;marketing=1" } else { "" });
        format!(
            r#"<nav class="ec-pager">{}<span>Page {page_no} of {pages}</span>{}</nav>"#,
            if page_no > 1 { format!(r#"<a class="btn text" href="{}">Previous</a>"#, mk(page_no - 1)) } else { String::new() },
            if page_no < pages { format!(r#"<a class="btn text" href="{}">Next</a>"#, mk(page_no + 1)) } else { String::new() }
        )
    } else {
        String::new()
    };
    let body = format!(
        r#"<form class="ec-filter rise" style="--i:2" method="get" action="{CUSTOMERS}">
<label class="field" style="margin:0;flex:1"><input type="search" name="q" value="{qv}" placeholder=" " maxlength="100"><span>Search name, email, phone or tag</span></label>
<label class="ec-check"><input type="checkbox" name="marketing" value="1"{mk}> Email subscribers only</label>
<button class="btn tonal" type="submit">{search} Search</button></form>
<p class="muted ec-lead">{count}</p>
<div class="card ec-table-card rise" style="--i:3">{table}</div>{pager}"#,
        qv = esc(&query),
        mk = if marketing { " checked" } else { "" },
        search = svg("search"),
        count = esc(&countable(total, "customer")),
        table = if list.is_empty() {
            format!(r#"<div class="ec-empty">{}<p><b>No customers yet</b></p><p class="muted">Everyone who orders becomes a customer here, with their orders and addresses.</p></div>"#, svg("group"))
        } else {
            format!(r#"<table class="ec-table ec-list"><thead><tr><th>Customer</th><th>Email marketing</th><th>Location</th><th>Orders</th><th class="ec-num">Spent</th><th>Last order</th></tr></thead><tbody>{rows}</tbody></table>"#)
        },
    );
    ecom_page(
        &actor,
        "Customers",
        "customers",
        &q,
        &format!(r#"<a class="btn text" href="{CUSTOMERS}export.csv">{} Export</a><a class="btn" href="{CUSTOMERS}new/">{} Add customer</a>"#, svg("import"), svg("add")),
        Some(BASE),
        &body,
    )
}

pub(crate) async fn customers_export(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let (list, _) = repo::browse_customers(&state.db, &[], false, 100_000, 0).await;
    let stats = repo::customer_stats(&state.db, &list.iter().map(|c| c.id.clone()).collect::<Vec<_>>()).await;
    let s = store_settings(&state);
    let cell = |v: &str| format!("\"{}\"", rules::csv_cell(v).replace('"', "\"\""));
    let mut out = String::from("Email,First name,Last name,Phone,Accepts marketing,City,Country,Orders,Spent,Tags,Note\n");
    for c in &list {
        let st = stats.get(&c.id).cloned().unwrap_or((0, 0, String::new()));
        let a = c.address();
        let row = [c.email.clone(), c.first_name.clone(), c.last_name.clone(), c.phone.clone(), if c.accepts_marketing != 0 { "yes".into() } else { "no".into() }, a.city, a.country, st.0.to_string(), rules::decimal(st.1, &s.currency), c.tags.clone(), c.note.clone()];
        out.push_str(&row.iter().map(|x| cell(x)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"customers.csv\"")], out).into_response()
}

pub(crate) async fn customer_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    customer_view(&state, &actor, None, &q).await
}

pub(crate) async fn customer_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(c) = valid_id(&id).then_some(()).and(repo::find_customer(&state.db, &id).await) else {
        return not_found_page_for(actor.role, &actor.user, "That customer could not be found.");
    };
    customer_view(&state, &actor, Some(c), &q).await
}

async fn customer_view(state: &AppState, actor: &Actor, c: Option<repo::Customer>, q: &HashMap<String, String>) -> Response {
    let s = store_settings(state);
    let a = c.as_ref().map(|c| c.address()).unwrap_or_default();
    let get = |f: fn(&repo::Customer) -> &str| c.as_ref().map(f).unwrap_or("").to_string();
    let f = |n: &str, l: &str, v: &str, extra: &str| format!(r#"<label class="field"><input name="{n}" value="{}" placeholder=" " maxlength="120"{extra}><span>{l}</span></label>"#, esc(v));
    let action = match &c {
        Some(c) => format!("{CUSTOMERS}{}/", esc(&c.id)),
        None => format!("{CUSTOMERS}new/"),
    };
    let (orders, stats) = match &c {
        Some(c) => {
            let (o, _) = repo::browse_orders(&state.db, &repo::OrderBrowse { customer_id: Some(&c.id), view: "all", limit: 100, ..Default::default() }).await;
            let st = repo::customer_stats(&state.db, std::slice::from_ref(&c.id)).await.remove(&c.id).unwrap_or((0, 0, String::new()));
            (o, st)
        }
        None => (Vec::new(), (0, 0, String::new())),
    };
    let order_rows: String = orders
        .iter()
        .map(|o| format!(r#"<a class="row" href="/ghost/ecom/orders/{id}/"><div class="body"><span class="title">#{n}</span><span class="detail">{d}</span></div><span class="trailing">{fin} {ful} <b>{t}</b></span></a>"#, id = esc(&o.id), n = o.number, d = date(&o.created_at), fin = chip_for_financial(&o.financial_status), ful = chip_for_fulfillment(&o.fulfillment_status), t = money(o.total_cents, &s)))
        .collect();
    let member_link = match c.as_ref().and_then(|c| c.member_id.clone()) {
        Some(m) => format!(r#"<p class="ec-p"><a href="/ghost/members/{}/">{} Also a member of this site</a></p>"#, esc(&m), svg("person")),
        None => String::new(),
    };
    let main = format!(
        r#"{kpis}<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-amber">{rc}</span>Orders</h2><div class="rows">{rows}</div>{create}</section>"#,
        kpis = if c.is_some() {
            format!(r#"<div class="ec-tiles">{}{}{}</div>"#,
                tile("Orders", &stats.0.to_string(), "Not counting cancelled", None, "receipt", "blue"),
                tile("Amount spent", &rules::format_money(stats.1, &s.currency), "Paid, net of refunds", None, "chart", "green"),
                tile("Average order", &rules::format_money(if stats.0 > 0 { stats.1 / stats.0 } else { 0 }, &s.currency), &if stats.2.is_empty() { "No orders yet".into() } else { format!("Last on {}", stats.2.get(..10).unwrap_or(&stats.2)) }, None, "trending", "purple"))
        } else {
            String::new()
        },
        rc = svg("receipt"),
        rows = if order_rows.is_empty() { r#"<p class="muted">No orders yet.</p>"#.to_string() } else { order_rows },
        create = c.as_ref().map(|c| format!(r#"<a class="btn text" href="/ghost/ecom/orders/new/?email={}">{} Create order</a>"#, ghost_core::social_media::encode(&c.email), svg("add"))).unwrap_or_default(),
    );
    let form = format!(
        r#"<form method="post" action="{action}" id="ec-customer">
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-blue">{person}</span>Customer</h2>{member_link}
<div class="ec-grid2">{fname}{lname}</div>{email}{phone}
<label class="ec-check"><input type="checkbox" name="marketing" value="1"{mk}> Agreed to receive marketing emails</label></section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-teal">{globe}</span>Default address</h2>
{company}{l1}{l2}<div class="ec-grid2">{city}{region}</div><div class="ec-grid2">{postal}{country}</div></section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-slate">{notes}</span>Notes and tags</h2>
<label class="field"><textarea name="note" rows="3" placeholder=" " maxlength="2000">{note}</textarea><span>Note</span></label>{tags}</section>
<button class="btn ec-wide" type="submit">{check} Save customer</button></form>{delete}"#,
        person = svg("person"),
        fname = f("first_name", "First name", &get(|c| &c.first_name), ""),
        lname = f("last_name", "Last name", &get(|c| &c.last_name), ""),
        email = f("email", "Email", &get(|c| &c.email), r#" type="email" required"#),
        phone = f("phone", "Phone", &get(|c| &c.phone), ""),
        mk = if c.as_ref().is_some_and(|c| c.accepts_marketing != 0) { " checked" } else { "" },
        globe = svg("globe"),
        company = f("company", "Company", &a.company, ""),
        l1 = f("line1", "Address", &a.line1, ""),
        l2 = f("line2", "Apartment, suite", &a.line2, ""),
        city = f("city", "City", &a.city, ""),
        region = f("region", "State / region", &a.region, ""),
        postal = f("postal", "Postal code", &a.postal, ""),
        country = f("country", "Country code (US, GB…)", &a.country, ""),
        notes = svg("notes"),
        note = esc(&get(|c| &c.note)),
        tags = f("tags", "Tags, separated by commas", &get(|c| &c.tags), ""),
        check = svg("check"),
        delete = c.as_ref().map(|c| format!(r#"<form method="post" action="{CUSTOMERS}{}/delete/" data-confirm="Delete customer|Their orders stay, without a customer attached."><button class="btn text danger" type="submit">{} Delete customer</button></form>"#, esc(&c.id), svg("delete"))).unwrap_or_default(),
    );
    let title = c.as_ref().map(|c| c.name()).unwrap_or_else(|| "Add customer".into());
    let body = format!(r#"<h2 class="ec-subhead">{}</h2><div class="ec-edit"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{form}</aside></div>"#, esc(&title));
    ecom_page(actor, &title, "customers", q, "", Some(CUSTOMERS), &body)
}

fn read_customer(f: &HashMap<String, String>) -> Result<repo::CustomerInput, String> {
    let get = |k: &str| f.get(k).cloned().unwrap_or_default();
    let email = get("email").trim().to_lowercase();
    if !rules::is_email(&email) {
        return Err("Enter a valid email address.".into());
    }
    Ok(repo::CustomerInput {
        email,
        first_name: rules::clean(&get("first_name"), 80),
        last_name: rules::clean(&get("last_name"), 80),
        phone: rules::clean(&get("phone"), 40),
        accepts_marketing: f.contains_key("marketing"),
        note: rules::clean_multiline(&get("note"), 2000),
        tags: rules::parse_tags(&get("tags")),
        address: rules::Address::from_form(|k| f.get(k).cloned()),
    })
}

pub(crate) async fn customer_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match read_customer(&f) {
        Err(e) => flash_to(&format!("{CUSTOMERS}new/"), "error", &e),
        Ok(c) => match repo::save_customer(&state.db, None, &c).await {
            Ok(id) => flash_to(&format!("{CUSTOMERS}{id}/"), "ok", "Customer added."),
            Err(e) => flash_to(&format!("{CUSTOMERS}new/"), "error", &e.to_string()),
        },
    }
}

pub(crate) async fn customer_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !valid_id(&id) || repo::find_customer(&state.db, &id).await.is_none() {
        return flash_to(CUSTOMERS, "error", "That customer no longer exists.");
    }
    let back = format!("{CUSTOMERS}{id}/");
    match read_customer(&f) {
        Err(e) => flash_to(&back, "error", &e),
        Ok(c) => match repo::save_customer(&state.db, Some(&id), &c).await {
            Ok(_) => flash_to(&back, "ok", "Customer saved."),
            Err(e) => flash_to(&back, "error", &e.to_string()),
        },
    }
}

pub(crate) async fn customer_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if valid_id(&id) {
        let _ = repo::delete_customer(&state.db, &id).await;
    }
    flash_to(CUSTOMERS, "ok", "Customer deleted.")
}
