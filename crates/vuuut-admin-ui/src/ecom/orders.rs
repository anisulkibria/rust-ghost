//! Orders: the list, one order, everything done to an order, orders typed
//! in by staff, and abandoned checkouts.
//!
//! Every action writes a line to the order's timeline with who did it.
//! Refunds never exceed what was paid and never go below zero; a card
//! refund goes through Stripe first and is recorded only if Stripe agrees.

use super::*;

const ORDERS: &str = "/ghost/ecom/orders/";

const VIEWS: &[(&str, &str)] = &[("all", "All"), ("unfulfilled", "Unfulfilled"), ("unpaid", "Unpaid"), ("open", "Open"), ("archived", "Archived"), ("cancelled", "Cancelled")];

pub(crate) async fn orders_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let view = q.get("view").map(String::as_str).filter(|v| VIEWS.iter().any(|(k, _)| k == v)).unwrap_or("all");
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let page_no: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).clamp(1, 10_000);
    let per = 50;
    let (orders, total) = repo::browse_orders(&state.db, &repo::OrderBrowse { terms: &terms, view, limit: per, offset: (page_no - 1) * per, ..Default::default() }).await;
    let counts = repo::order_counts(&state.db).await;
    let mut item_counts: HashMap<String, i64> = HashMap::new();
    for o in &orders {
        item_counts.insert(o.id.clone(), repo::order_items(&state.db, &o.id).await.iter().map(|i| i.quantity).sum());
    }
    let rows: String = orders
        .iter()
        .map(|o| {
            let a = o.address();
            let who = if a.name().is_empty() { o.email.clone() } else { a.name() };
            format!(
                r#"<tr class="{cls}"><td><a href="{ORDERS}{id}/"><b>#{n}</b></a></td><td>{d}</td><td>{who}</td><td class="ec-num">{t}</td><td>{fin}</td><td>{ful}</td><td>{items}</td><td>{pay}</td></tr>"#,
                cls = if o.is_cancelled() { "ec-cancelled" } else { "" },
                id = esc(&o.id),
                n = o.number,
                d = date(&o.created_at),
                who = esc(&who),
                t = esc(&rules::format_money(o.total_cents, &o.currency)),
                fin = if o.is_cancelled() { r#"<span class="chip draft">Cancelled</span>"#.to_string() } else { chip_for_financial(&o.financial_status) },
                ful = chip_for_fulfillment(&o.fulfillment_status),
                items = countable(item_counts.get(&o.id).copied().unwrap_or(0), "item"),
                pay = esc(rules::payment_label(&o.payment_method)),
            )
        })
        .collect();
    let tabs_html: String = VIEWS
        .iter()
        .map(|(k, l)| format!(r#"<a class="ec-subtab{on}" href="{ORDERS}?view={k}">{l} <span>{n}</span></a>"#, on = if *k == view { " on" } else { "" }, n = counts.get(k).copied().unwrap_or(0)))
        .collect();
    let pages = (total + per - 1) / per;
    let pager = if pages > 1 {
        let mk = |n: i64| format!("?view={view}&amp;q={}&amp;page={n}", vuuut_core::social_media::encode(&query));
        format!(
            r#"<nav class="ec-pager">{}<span>Page {page_no} of {pages}</span>{}</nav>"#,
            if page_no > 1 { format!(r#"<a class="btn text" href="{}">Previous</a>"#, mk(page_no - 1)) } else { String::new() },
            if page_no < pages { format!(r#"<a class="btn text" href="{}">Next</a>"#, mk(page_no + 1)) } else { String::new() }
        )
    } else {
        String::new()
    };
    let body = format!(
        r#"<div class="ec-subtabs rise" style="--i:2">{tabs_html}<a class="ec-subtab" href="{ORDERS}abandoned/">Abandoned checkouts</a></div>
<form class="ec-filter rise" style="--i:2" method="get" action="{ORDERS}"><input type="hidden" name="view" value="{view}">
<label class="field" style="margin:0;flex:1"><input type="search" name="q" value="{qv}" placeholder=" " maxlength="100"><span>Search by order number, email, name, product or SKU</span></label>
<button class="btn tonal" type="submit">{search} Search</button></form>
<div class="card ec-table-card rise" style="--i:3">{table}</div>{pager}"#,
        qv = esc(&query),
        search = svg("search"),
        table = if orders.is_empty() {
            format!(r#"<div class="ec-empty">{}<p><b>No orders here</b></p><p class="muted">Orders from your store appear here as they come in. You can also create one for a customer by hand.</p></div>"#, svg("receipt"))
        } else {
            format!(r#"<table class="ec-table ec-list"><thead><tr><th>Order</th><th>Date</th><th>Customer</th><th class="ec-num">Total</th><th>Payment</th><th>Fulfillment</th><th>Items</th><th>Method</th></tr></thead><tbody>{rows}</tbody></table>"#)
        },
    );
    ecom_page(
        &actor,
        "Orders",
        "orders",
        &q,
        &format!(r#"<a class="btn text" href="{ORDERS}export.csv?view={view}">{} Export</a><a class="btn" href="{ORDERS}new/">{} Create order</a>"#, svg("import"), svg("add")),
        Some(BASE),
        &body,
    )
}

pub(crate) async fn orders_export(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let view = q.get("view").map(String::as_str).filter(|v| VIEWS.iter().any(|(k, _)| k == v)).unwrap_or("all");
    let (orders, _) = repo::browse_orders(&state.db, &repo::OrderBrowse { view, limit: 10_000, ..Default::default() }).await;
    let mut out = String::from("Number,Date,Email,Name,Status,Payment,Fulfillment,Currency,Subtotal,Discount,Shipping,Tax,Total,Refunded,Discount code,Shipping method,Payment method,Country,Lines\n");
    let cell = |v: &str| format!("\"{}\"", rules::csv_cell(v).replace('"', "\"\""));
    for o in &orders {
        let a = o.address();
        let items = repo::order_items(&state.db, &o.id).await;
        let lines = items.iter().map(|i| format!("{} x{}", if i.variant_title == rules::DEFAULT_VARIANT { i.title.clone() } else { format!("{} ({})", i.title, i.variant_title) }, i.quantity)).collect::<Vec<_>>().join("; ");
        let d = |c: i64| rules::decimal(c, &o.currency);
        let row = [
            o.number.to_string(), o.created_at.clone(), o.email.clone(), a.name(), o.status.clone(), o.financial_status.clone(), o.fulfillment_status.clone(), o.currency.clone(),
            d(o.subtotal_cents), d(o.discount_cents), d(o.shipping_cents), d(o.tax_cents), d(o.total_cents), d(o.refunded_cents), o.discount_code.clone(), o.shipping_method.clone(),
            o.payment_method.clone(), a.country.clone(), lines,
        ];
        out.push_str(&row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"orders.csv\"")], out).into_response()
}

// ---------------------------------------------------------------------------
// one order
// ---------------------------------------------------------------------------

pub(crate) async fn order_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(o) = valid_id(&id).then_some(()).and(repo::find_order(&state.db, &id).await) else {
        return not_found_page_for(actor.role, &actor.user, "That order could not be found.");
    };
    let items = repo::order_items(&state.db, &o.id).await;
    let events = repo::order_events(&state.db, &o.id).await;
    let customer = match &o.customer_id {
        Some(c) => repo::find_customer(&state.db, c).await,
        None => None,
    };
    let stats = match &customer {
        Some(c) => repo::customer_stats(&state.db, std::slice::from_ref(&c.id)).await.remove(&c.id),
        None => None,
    };
    let m = |c: i64| esc(&rules::format_money(c, &o.currency));
    let act = format!("{ORDERS}{}/", esc(&o.id));
    let line_rows: String = items
        .iter()
        .map(|i| {
            format!(
                r#"<div class="row ec-line">{thumb}<div class="body"><span class="title">{t}</span><span class="detail">{v}{sku}</span></div><span class="ec-line-q">{u} × {q}</span><b class="ec-num">{tot}</b></div>"#,
                thumb = thumb(&i.image),
                t = match &i.product_id {
                    Some(pid) if valid_id(pid) => format!(r#"<a href="/ghost/ecom/products/{}/">{}</a>"#, esc(pid), esc(&i.title)),
                    _ => esc(&i.title),
                },
                v = if i.variant_title == rules::DEFAULT_VARIANT { String::new() } else { esc(&i.variant_title) },
                sku = if i.sku.is_empty() { String::new() } else { format!(" · SKU {}", esc(&i.sku)) },
                u = m(i.unit_cents),
                q = i.quantity,
                tot = m(i.total_cents),
            )
        })
        .collect();
    let fulfil_box = if o.is_cancelled() {
        String::new()
    } else if o.fulfillment_status == "fulfilled" {
        format!(
            r#"<div class="ec-fulfilled"><p>{ic} Fulfilled {when}{track}</p><form method="post" action="{act}unfulfil/"><button class="btn text small" type="submit">Mark as unfulfilled</button></form></div>"#,
            ic = svg("check"),
            when = o.fulfilled_at.as_deref().map(|d| format!("on {}", esc(d.get(..10).unwrap_or(d)))).unwrap_or_default(),
            track = if o.tracking_number.is_empty() { String::new() } else {
                format!(
                    r#" · {} <b>{}</b>{}"#,
                    esc(&o.tracking_company),
                    esc(&o.tracking_number),
                    if o.tracking_url.is_empty() { String::new() } else { format!(r#" · <a href="{}" target="_blank" rel="noopener nofollow">Track</a>"#, esc(&o.tracking_url)) }
                )
            },
        )
    } else {
        let carriers: String = rules::CARRIERS.iter().map(|c| format!(r#"<option value="{c}">{c}</option>"#)).collect();
        format!(
            r#"<details class="ec-fulfil"><summary class="btn">{truck} Fulfil items</summary>
<form method="post" action="{act}fulfil/" class="ec-fulfil-form"><div class="ec-grid3">
<label class="field fixed"><select name="company">{carriers}</select><span>Carrier</span>{SELECT_CARET}</label>
<label class="field"><input name="number" placeholder=" " maxlength="120"><span>Tracking number</span></label>
<label class="field"><input name="url" placeholder=" " maxlength="500"><span>Tracking URL (optional)</span></label></div>
<label class="ec-check"><input type="checkbox" name="notify" value="1" checked> Email the customer that it has shipped</label>
<button class="btn" type="submit">{check} Mark as fulfilled</button></form></details>"#,
            truck = svg("truck"),
            check = svg("check"),
        )
    };
    let pay_box = if o.is_cancelled() {
        String::new()
    } else if !o.is_paid() {
        format!(
            r#"<form method="post" action="{act}paid/" class="ec-row"><label class="field" style="margin:0;flex:1"><input name="reference" placeholder=" " maxlength="200"><span>Payment reference (optional)</span></label><button class="btn" type="submit">{} Mark as paid</button></form>"#,
            svg("card")
        )
    } else if o.refunded_cents < o.total_cents {
        format!(
            r#"<details class="ec-refund"><summary class="btn text">Refund</summary><form method="post" action="{act}refund/" class="ec-refund-form">
<label class="field"><input name="amount" value="{max}" placeholder=" " inputmode="decimal"><span>Amount to refund (up to {maxf})</span></label>
<label class="field"><input name="reason" placeholder=" " maxlength="300"><span>Reason (only you see this)</span></label>
<label class="ec-check"><input type="checkbox" name="restock" value="1"{rs}> Put the items back in stock</label>
{card}<button class="btn" type="submit">Refund</button></form></details>"#,
            max = esc(&rules::money_input(o.total_cents - o.refunded_cents, &o.currency)),
            maxf = m(o.total_cents - o.refunded_cents),
            rs = if o.restocked != 0 { " disabled" } else { "" },
            card = if o.payment_method == "stripe" && o.payment_ref.starts_with("pi_") { r#"<p class="muted ec-hint">The money goes back to the customer's card through Stripe.</p>"# } else { r#"<p class="muted ec-hint">Records the refund; return the money the way it was paid.</p>"# },
        )
    } else {
        String::new()
    };
    let addr = o.address();
    let side = format!(
        r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-blue">{person}</span>Customer</h2>
{cust}<div class="ec-sublabel">Contact</div><p class="ec-p"><a href="mailto:{email}">{email}</a>{phone}</p>
<div class="ec-sublabel">Shipping address</div><p class="ec-p">{addr}</p>
<details class="ec-more"><summary>Edit contact and address</summary>{edit}</details></section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-slate">{notes}</span>Notes</h2>
{cnote}<form method="post" action="{act}note/"><label class="field"><textarea name="note" rows="3" placeholder=" " maxlength="2000">{snote}</textarea><span>Staff note</span></label><button class="btn tonal small" type="submit">Save note</button></form></section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-amber">{tune}</span>More actions</h2><div class="ec-side-actions">
<a class="btn text" href="{status_url}" target="_blank" rel="noopener">{open} Customer's order page</a>
{resend}{archive}{cancel}{delete}</div></section>"#,
        person = svg("person"),
        cust = match (&customer, &stats) {
            (Some(c), st) => format!(
                r#"<p class="ec-p"><a href="/ghost/ecom/customers/{id}/"><b>{n}</b></a><br><span class="muted">{orders}{spent}</span></p>"#,
                id = esc(&c.id),
                n = esc(&c.name()),
                orders = countable(st.as_ref().map(|s| s.orders).unwrap_or(0), "order"),
                spent = st.as_ref().map(|s| format!(" · {} spent", esc(&super::customers::spending(s, &o.currency, false)))).unwrap_or_default(),
            ),
            _ => r#"<p class="muted ec-p">Guest</p>"#.to_string(),
        },
        email = esc(&o.email),
        phone = if o.phone.is_empty() { String::new() } else { format!("<br>{}", esc(&o.phone)) },
        addr = if addr.is_empty() { "No shipping address".into() } else { addr.lines().iter().map(|l| esc(l)).collect::<Vec<_>>().join("<br>") },
        edit = contact_form(&act, &o, &addr),
        notes = svg("notes"),
        cnote = if o.note.is_empty() { String::new() } else { format!(r#"<p class="ec-p ec-cnote"><span class="muted">From the customer:</span><br>{}</p>"#, esc(&o.note).replace('\n', "<br>")) },
        snote = esc(&o.staff_note),
        tune = svg("tune"),
        status_url = esc(&vuuut_api::ecom::order_url(&state, &o)),
        open = svg("open"),
        resend = format!(r#"<form method="post" action="{act}resend/"><button class="btn text" type="submit">{} Resend receipt</button></form>"#, svg("send")),
        archive = if o.is_cancelled() { String::new() } else if o.status == "archived" {
            format!(r#"<form method="post" action="{act}archive/"><input type="hidden" name="to" value="open"><button class="btn text" type="submit">{} Unarchive</button></form>"#, svg("archive"))
        } else {
            format!(r#"<form method="post" action="{act}archive/"><input type="hidden" name="to" value="archived"><button class="btn text" type="submit">{} Archive</button></form>"#, svg("archive"))
        },
        cancel = if o.is_cancelled() { String::new() } else {
            format!(
                r#"<details class="ec-more"><summary class="btn text danger">{x} Cancel order</summary><form method="post" action="{act}cancel/" class="ec-cancel-form">
<label class="ec-check"><input type="checkbox" name="restock" value="1"{rs}> Put the items back in stock</label>
<label class="field"><input name="reason" placeholder=" " maxlength="300"><span>Reason</span></label>
<button class="btn danger" type="submit">Cancel order</button></form></details>"#,
                x = svg("close"),
                rs = if o.restocked != 0 { " disabled" } else { " checked" },
            )
        },
        delete = if o.is_cancelled() || o.status == "archived" {
            format!(r#"<form method="post" action="{act}delete/" data-confirm="Delete order|Order #{n} and its timeline will be permanently deleted."><button class="btn text danger" type="submit">{d} Delete</button></form>"#, n = o.number, d = svg("delete"))
        } else {
            String::new()
        },
    );
    let timeline: String = events
        .iter()
        .map(|e| format!(r#"<li><span class="ec-dot ec-dot-{k}"></span><div><p>{msg}</p><small class="muted">{when}{who}</small></div></li>"#, k = esc(&e.kind), msg = esc(&e.message), when = date(&e.created_at), who = if e.actor.is_empty() { String::new() } else { format!(" · {}", esc(&e.actor)) }))
        .collect();
    let main = format!(
        r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-amber">{box_ic}</span>{fstat} <span class="muted">· {method}</span></h2>
<div class="rows">{line_rows}</div>{fulfil_box}</section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-green">{card_ic}</span>{pstat}</h2>
<table class="ec-table ec-kv"><tr><td>Subtotal</td><td>{sub}</td></tr>{disc}<tr><td>Shipping{smeth}</td><td>{ship}</td></tr>{tax}<tr class="ec-strong"><td>Total</td><td>{total}</td></tr>
{refund}<tr><td>Paid by customer</td><td>{paid}</td></tr></table>
<p class="muted ec-hint">{pm}{pref}</p>{pay_box}</section>
{downloads}
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-slate">{hist}</span>Timeline</h2>
<form method="post" action="{act}comment/" class="ec-row"><label class="field" style="margin:0;flex:1"><input name="message" placeholder=" " maxlength="1000" required><span>Leave a comment…</span></label><button class="btn tonal" type="submit">Post</button></form>
<ol class="ec-timeline">{timeline}</ol></section>"#,
        box_ic = svg("box"),
        fstat = rules::fulfillment_label(&o.fulfillment_status),
        method = if o.shipping_method.is_empty() { "No shipping needed".to_string() } else { esc(&o.shipping_method) },
        card_ic = svg("card"),
        pstat = if o.is_cancelled() { "Cancelled".to_string() } else { rules::financial_label(&o.financial_status).to_string() },
        sub = m(o.subtotal_cents),
        disc = if o.discount_cents > 0 { format!(r#"<tr><td>Discount <span class="chip tonal">{}</span></td><td>−{}</td></tr>"#, esc(&o.discount_code), m(o.discount_cents)) } else { String::new() },
        smeth = if o.shipping_method.is_empty() { String::new() } else { format!(r#" <span class="muted">({})</span>"#, esc(&o.shipping_method)) },
        ship = m(o.shipping_cents),
        tax = if o.tax_cents > 0 { format!("<tr><td>{}</td><td>{}</td></tr>", if o.tax_included != 0 { "Tax (included)" } else { "Tax" }, m(o.tax_cents)) } else { String::new() },
        total = m(o.total_cents),
        refund = if o.refunded_cents > 0 { format!("<tr><td>Refunded</td><td>−{}</td></tr><tr class=\"ec-strong\"><td>Net</td><td>{}</td></tr>", m(o.refunded_cents), m(o.net())) } else { String::new() },
        paid = if o.is_paid() { m(o.total_cents) } else { m(0) },
        pm = esc(rules::payment_label(&o.payment_method)),
        pref = if o.payment_ref.is_empty() { String::new() } else { format!(" · ref {}", esc(&o.payment_ref)) },
        hist = svg("history"),
        downloads = downloads_card(&state, &o, &act).await,
    );
    let heading = format!("Order #{}", o.number);
    let body = format!(
        r#"<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{ORDERS}" title="Back to orders">{back}</a>
<div><h1>{h} {fin} {ful}</h1><p class="sub">{when} · from {src}</p></div><div class="actions">{prev}</div></header>{banner}
<div class="ec-edit"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></div>"#,
        back = svg("chevron"),
        h = esc(&heading),
        fin = if o.is_cancelled() { r#"<span class="chip draft">Cancelled</span>"#.to_string() } else { chip_for_financial(&o.financial_status) },
        ful = chip_for_fulfillment(&o.fulfillment_status),
        when = date(&o.created_at),
        src = if o.source == "admin" { "the admin" } else { "the online store" },
        prev = format!(r#"<a class="btn text" href="{act}print/" target="_blank" rel="noopener">{} Packing slip</a>"#, svg("file")),
        banner = flash_banner(&q),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

fn contact_form(act: &str, o: &repo::Order, a: &rules::Address) -> String {
    let f = |n: &str, l: &str, v: &str| format!(r#"<label class="field"><input name="{n}" value="{}" placeholder=" " maxlength="120"><span>{l}</span></label>"#, esc(v));
    format!(
        r#"<form method="post" action="{act}contact/" class="ec-contact">{email}<div class="ec-grid2">{fname}{lname}</div>{company}{l1}{l2}<div class="ec-grid3">{city}{region}{postal}</div><div class="ec-grid2">{country}{phone}</div><button class="btn tonal small" type="submit">Save</button></form>"#,
        email = f("email", "Email", &o.email),
        fname = f("first_name", "First name", &a.first_name),
        lname = f("last_name", "Last name", &a.last_name),
        company = f("company", "Company", &a.company),
        l1 = f("line1", "Address", &a.line1),
        l2 = f("line2", "Apartment, suite", &a.line2),
        city = f("city", "City", &a.city),
        region = f("region", "State / region", &a.region),
        postal = f("postal", "Postal code", &a.postal),
        country = f("country", "Country code (US, GB…)", &a.country),
        phone = f("phone", "Phone", &a.phone),
    )
}

/// A printable packing slip.
pub(crate) async fn order_print(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let Some(o) = valid_id(&id).then_some(()).and(repo::find_order(&state.db, &id).await) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let s = store_settings(&state);
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let items = repo::order_items(&state.db, &o.id).await;
    let rows: String = items
        .iter()
        .filter(|i| i.requires_shipping != 0)
        .map(|i| format!("<tr><td>{}{}</td><td>{}</td><td style=\"text-align:right\">{}</td></tr>", esc(&i.title), if i.variant_title == rules::DEFAULT_VARIANT { String::new() } else { format!(" — {}", esc(&i.variant_title)) }, esc(&i.sku), i.quantity))
        .collect();
    Html(format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>Packing slip #{n}</title><style>
body{{font-family:-apple-system,Segoe UI,Roboto,sans-serif;max-width:720px;margin:32px auto;color:#111}}h1{{font-size:22px}}table{{width:100%;border-collapse:collapse}}
td,th{{padding:8px 4px;border-bottom:1px solid #ddd;text-align:left}}.cols{{display:flex;justify-content:space-between;gap:24px;margin:24px 0}}@media print{{button{{display:none}}}}</style></head>
<body><button onclick="print()">Print</button><h1>{store}</h1><p>Order #{n} · {d}</p>
<div class="cols"><div><b>Ship to</b><br>{addr}</div><div><b>Shipping</b><br>{method}</div></div>
<table><thead><tr><th>Item</th><th>SKU</th><th style="text-align:right">Qty</th></tr></thead><tbody>{rows}</tbody></table>
{note}<p style="margin-top:32px;color:#666">Thank you for shopping with us.</p></body></html>"#,
        n = o.number,
        store = esc(s.name(&site_title)),
        d = esc(o.created_at.get(..10).unwrap_or(&o.created_at)),
        addr = o.address().lines().iter().map(|l| esc(l)).collect::<Vec<_>>().join("<br>"),
        method = esc(&o.shipping_method),
        note = if o.note.is_empty() { String::new() } else { format!("<p><b>Note:</b> {}</p>", esc(&o.note)) },
    ))
    .into_response()
}

async fn load(state: &AppState, headers: &HeaderMap, id: &str) -> Result<(Actor, repo::Order), Response> {
    let actor = guard(state, headers, Role::Editor, true).await?;
    match valid_id(id).then_some(()).and(repo::find_order(&state.db, id).await) {
        Some(o) => Ok((actor, o)),
        None => Err(flash_to(ORDERS, "error", "That order no longer exists.")),
    }
}

fn back(o: &repo::Order) -> String {
    format!("{ORDERS}{}/", o.id)
}

fn who(a: &Actor) -> String {
    a.user.name.clone().or_else(|| a.user.email.clone()).unwrap_or_default()
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct ActionForm {
    reference: String,
    company: String,
    number: String,
    url: String,
    notify: Option<String>,
    amount: String,
    reason: String,
    restock: Option<String>,
    note: String,
    message: String,
    to: String,
}

pub(crate) async fn order_paid(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if o.is_paid() || o.is_cancelled() {
        return Redirect::to(&back(&o)).into_response();
    }
    let reference = rules::clean(&f.reference, 200);
    if let Err(e) = repo::mark_paid(&state.db, &o.id, &reference).await {
        return flash_to(&back(&o), "error", &e.to_string());
    }
    repo::add_event(&state.db, &o.id, "paid", &format!("Marked as paid: {}{}.", rules::format_money(o.total_cents, &o.currency), if reference.is_empty() { String::new() } else { format!(" (ref {reference})") }), &who(&actor)).await;
    let digital = vuuut_api::ecom::on_paid(&state, &o).await;
    flash_to(&back(&o), "ok", if digital { "Marked as paid. Downloads unlocked and emailed to the buyer." } else { "Marked as paid." })
}

pub(crate) async fn order_fulfil(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if o.is_cancelled() {
        return flash_to(&back(&o), "error", "A cancelled order cannot be fulfilled.");
    }
    let company = rules::clean(&f.company, 80);
    let number = rules::clean(&f.number, 120);
    let typed = rules::clean_images(std::iter::once(f.url.trim())).into_iter().next().filter(|u| u.starts_with("http"));
    let url = typed.or_else(|| rules::tracking_link(&company, &number)).unwrap_or_default();
    if let Err(e) = repo::fulfil(&state.db, &o.id, &company, &number, &url).await {
        return flash_to(&back(&o), "error", &e.to_string());
    }
    repo::add_event(&state.db, &o.id, "fulfilled", &if number.is_empty() { "Items fulfilled.".to_string() } else { format!("Items fulfilled — {company} {number}.") }, &who(&actor)).await;
    if f.notify.is_some() {
        if let Some(fresh) = repo::find_order(&state.db, &o.id).await {
            vuuut_api::ecom::shipped(&state, &fresh).await;
            repo::add_event(&state.db, &o.id, "email", &format!("Shipping confirmation sent to {}.", o.email), "").await;
        }
    }
    flash_to(&back(&o), "ok", "Marked as fulfilled.")
}

pub(crate) async fn order_unfulfil(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let _ = repo::unfulfil(&state.db, &o.id).await;
    repo::add_event(&state.db, &o.id, "fulfilled", "Marked as unfulfilled.", &who(&actor)).await;
    flash_to(&back(&o), "ok", "Marked as unfulfilled.")
}

pub(crate) async fn order_refund(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !o.is_paid() {
        return flash_to(&back(&o), "error", "Only a paid order can be refunded.");
    }
    let left = o.total_cents - o.refunded_cents;
    let Some(amount) = rules::parse_money(&f.amount, &o.currency).filter(|a| *a > 0 && *a <= left) else {
        return flash_to(&back(&o), "error", &format!("Enter an amount between {} and {}.", rules::format_money(1, &o.currency), rules::format_money(left, &o.currency)));
    };
    if o.payment_method == "stripe" && o.payment_ref.starts_with("pi_") {
        if let Err(e) = vuuut_api::ecom::refund_card(&state, &o, amount).await {
            return flash_to(&back(&o), "error", &format!("Stripe refused the refund: {e}"));
        }
    }
    if let Err(e) = repo::refund(&state.db, &o, amount).await {
        return flash_to(&back(&o), "error", &e.to_string());
    }
    if f.restock.is_some() {
        let _ = repo::restock(&state.db, &o).await;
    }
    if amount >= left {
        vuuut_api::ecom::set_downloads_revoked(&state, &o, true, "fully refunded", &who(&actor)).await;
    }
    let reason = rules::clean(&f.reason, 300);
    repo::add_event(
        &state.db,
        &o.id,
        "refund",
        &format!("Refunded {}{}{}.", rules::format_money(amount, &o.currency), if reason.is_empty() { String::new() } else { format!(" — {reason}") }, if f.restock.is_some() && o.restocked == 0 { "; items restocked" } else { "" }),
        &who(&actor),
    )
    .await;
    flash_to(&back(&o), "ok", "Refund recorded.")
}

pub(crate) async fn order_cancel(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if o.is_cancelled() {
        return Redirect::to(&back(&o)).into_response();
    }
    if let Err(e) = repo::cancel(&state.db, &o).await {
        return flash_to(&back(&o), "error", &e.to_string());
    }
    if f.restock.is_some() {
        let _ = repo::restock(&state.db, &o).await;
    }
    vuuut_api::ecom::set_downloads_revoked(&state, &o, true, "order cancelled", &who(&actor)).await;
    let reason = rules::clean(&f.reason, 300);
    repo::add_event(&state.db, &o.id, "cancelled", &format!("Order cancelled{}.", if reason.is_empty() { String::new() } else { format!(" — {reason}") }), &who(&actor)).await;
    flash_to(&back(&o), "ok", if o.is_paid() { "Order cancelled. It was paid — refund it if the money should go back." } else { "Order cancelled." })
}

pub(crate) async fn order_archive(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if o.is_cancelled() {
        return Redirect::to(&back(&o)).into_response();
    }
    let to = if f.to == "open" { "open" } else { "archived" };
    let _ = repo::set_order_status(&state.db, &o.id, to).await;
    repo::add_event(&state.db, &o.id, "status", if to == "open" { "Order unarchived." } else { "Order archived." }, &who(&actor)).await;
    flash_to(&back(&o), "ok", if to == "open" { "Unarchived." } else { "Archived." })
}

pub(crate) async fn order_note(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (_, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let _ = repo::set_staff_note(&state.db, &o.id, &rules::clean_multiline(&f.note, 2000)).await;
    flash_to(&back(&o), "ok", "Note saved.")
}

pub(crate) async fn order_comment(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let msg = rules::clean(&f.message, 1000);
    if !msg.is_empty() {
        repo::add_event(&state.db, &o.id, "comment", &msg, &who(&actor)).await;
    }
    Redirect::to(&back(&o)).into_response()
}

pub(crate) async fn order_contact(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<HashMap<String, String>>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let email = f.get("email").map(|e| e.trim().to_lowercase()).unwrap_or_default();
    if !rules::is_email(&email) {
        return flash_to(&back(&o), "error", "Enter a valid email address.");
    }
    let a = rules::Address::from_form(|k| f.get(k).cloned());
    let _ = repo::set_order_contact(&state.db, &o.id, &email, &a.phone, &a).await;
    repo::add_event(&state.db, &o.id, "edit", "Contact details or address changed.", &who(&actor)).await;
    flash_to(&back(&o), "ok", "Saved.")
}

pub(crate) async fn order_resend(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    vuuut_api::ecom::receipt(&state, &o).await;
    repo::add_event(&state.db, &o.id, "email", &format!("Receipt sent to {}.", o.email), &who(&actor)).await;
    flash_to(&back(&o), "ok", "Receipt sent (if receipts are on in Settings).")
}

pub(crate) async fn order_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let (_, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !(o.is_cancelled() || o.status == "archived") {
        return flash_to(&back(&o), "error", "Only cancelled or archived orders can be deleted.");
    }
    let _ = repo::delete_order(&state.db, &o.id).await;
    flash_to(ORDERS, "ok", &format!("Order #{} deleted.", o.number))
}

// ---------------------------------------------------------------------------
// staff-created orders
// ---------------------------------------------------------------------------

pub(crate) async fn order_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let (products, _) = repo::browse_products(&state.db, &repo::ProductBrowse { sort: "title", limit: 1000, ..Default::default() }).await.unwrap_or_default();
    let ids: Vec<String> = products.iter().map(|p| p.id.clone()).collect();
    let vs = repo::variants_for(&state.db, &ids).await;
    let currency = s.currency.clone();
    let currency = &currency;
    let options: String = products
        .iter()
        .flat_map(|p| {
            vs.get(&p.id).cloned().unwrap_or_default().into_iter().map(move |v| {
                format!(
                    r#"<option value="{id}">{t}{vt} — {price}{stock}</option>"#,
                    id = esc(&v.id),
                    t = esc(&p.title),
                    vt = if v.title == rules::DEFAULT_VARIANT { String::new() } else { format!(" ({})", esc(&v.title)) },
                    price = esc(&rules::format_money(v.price_cents, currency)),
                    stock = if v.tracked() { format!(" · {} in stock", v.inventory) } else { String::new() },
                )
            })
        })
        .collect();
    let preset_email = q.get("email").cloned().unwrap_or_default();
    let customer = if rules::is_email(&preset_email) { repo::customer_by_email(&state.db, &preset_email).await } else { None };
    let a = customer.as_ref().map(|c| c.address()).unwrap_or_default();
    let line = |i: usize| {
        format!(
            r#"<div class="ec-row ec-newline"><label class="field fixed" style="margin:0;flex:3"><select name="variant"><option value="">Choose a product…</option>{options}</select><span>Product {n}</span>{SELECT_CARET}</label>
<label class="field" style="margin:0;width:110px"><input name="qty" value="1" type="number" min="1" placeholder=" "><span>Qty</span></label></div>"#,
            n = i + 1
        )
    };
    let f = |n: &str, l: &str, v: &str| format!(r#"<label class="field"><input name="{n}" value="{}" placeholder=" " maxlength="120"><span>{l}</span></label>"#, esc(v));
    let body = format!(
        r#"<form method="post" action="{ORDERS}new/" class="ec-edit"><div class="ec-edit-main">
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-pink">{bag}</span>Products</h2><div id="ec-newlines">{lines}</div>
<button class="btn text small" type="button" id="ec-add-line">{add} Add another product</button>
<p class="muted ec-hint">Stock is taken for tracked variants, as with an online order.</p></section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-green">{card}</span>Payment</h2>
<div class="ec-grid2"><label class="field"><input name="shipping" placeholder=" " inputmode="decimal"><span>Shipping ({cur})</span></label>
<label class="field"><input name="shipping_name" value="Shipping" placeholder=" " maxlength="120"><span>Shipping label</span></label></div>
<div class="ec-grid2"><label class="field"><input name="code" placeholder=" " maxlength="40"><span>Discount code (optional)</span></label>
<label class="field fixed"><select name="paid"><option value="0">Payment pending</option><option value="1">Already paid</option></select><span>Payment</span>{SELECT_CARET}</label></div>
<label class="field"><textarea name="note" rows="2" placeholder=" " maxlength="1000"></textarea><span>Note</span></label></section>
</div><aside class="ec-edit-side">
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-blue">{person}</span>Customer</h2>
{email}<div class="ec-grid2">{fname}{lname}</div>{l1}{l2}<div class="ec-grid2">{city}{postal}</div><div class="ec-grid2">{country}{phone}</div>
<label class="ec-check"><input type="checkbox" name="receipt" value="1"> Email the customer a receipt</label></section>
<button class="btn ec-wide" type="submit">{check} Create order</button></aside></form>
<template id="ec-line-template">{tpl}</template><script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        bag = svg("bag"),
        lines = (0..3).map(line).collect::<String>(),
        tpl = line(3),
        add = svg("add"),
        card = svg("card"),
        cur = esc(&s.currency),
        person = svg("person"),
        email = f("email", "Email", &preset_email),
        fname = f("first_name", "First name", customer.as_ref().map(|c| c.first_name.as_str()).unwrap_or(&a.first_name)),
        lname = f("last_name", "Last name", customer.as_ref().map(|c| c.last_name.as_str()).unwrap_or(&a.last_name)),
        l1 = f("line1", "Address", &a.line1),
        l2 = f("line2", "Apartment, suite", &a.line2),
        city = f("city", "City", &a.city),
        postal = f("postal", "Postal code", &a.postal),
        country = f("country", "Country code (US, GB…)", &a.country),
        phone = f("phone", "Phone", &a.phone),
        check = svg("check"),
    );
    ecom_page(&actor, "Create order", "orders", &q, "", Some(ORDERS), &body)
}

pub(crate) async fn order_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<Vec<(String, String)>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let variants: Vec<String> = f.iter().filter(|(k, _)| k == "variant").map(|(_, v)| v.clone()).collect();
    let qtys: Vec<String> = f.iter().filter(|(k, _)| k == "qty").map(|(_, v)| v.clone()).collect();
    let mut lines: Vec<rules::CartLine> = Vec::new();
    for (i, v) in variants.iter().enumerate() {
        if valid_id(v) {
            lines.push(rules::CartLine { variant_id: v.clone(), qty: qtys.get(i).and_then(|q| q.trim().parse().ok()).unwrap_or(1) });
        }
    }
    let lines = rules::normalise_cart(lines);
    let back_to = format!("{ORDERS}new/");
    if lines.is_empty() {
        return flash_to(&back_to, "error", "Add at least one product.");
    }
    let email = get("email").trim().to_lowercase();
    if !rules::is_email(&email) {
        return flash_to(&back_to, "error", "Enter the customer's email.");
    }
    let ids: Vec<String> = lines.iter().map(|l| l.variant_id.clone()).collect();
    let vs = repo::variants_by_ids(&state.db, &ids).await;
    let ps = repo::products_by_ids(&state.db, &vs.iter().map(|v| v.product_id.clone()).collect::<Vec<_>>()).await;
    let mut new_lines = Vec::new();
    let (mut subtotal, mut taxable, mut ships) = (0, 0, false);
    for l in &lines {
        let Some(v) = vs.iter().find(|v| v.id == l.variant_id) else { continue };
        let Some(p) = ps.iter().find(|p| p.id == v.product_id) else { continue };
        subtotal += v.price_cents * l.qty;
        if p.taxable != 0 {
            taxable += v.price_cents * l.qty;
        }
        ships |= p.requires_shipping != 0;
        new_lines.push(repo::NewLine {
            product_id: p.id.clone(),
            variant_id: v.id.clone(),
            title: p.title.clone(),
            variant_title: v.title.clone(),
            sku: v.sku.clone(),
            image: if v.image.is_empty() { p.first_image().unwrap_or_default() } else { v.image.clone() },
            unit_cents: v.price_cents,
            quantity: l.qty,
            requires_shipping: p.requires_shipping != 0,
            track: v.tracked(),
            backorder: true,
        });
    }
    if new_lines.is_empty() {
        return flash_to(&back_to, "error", "Those products no longer exist.");
    }
    let shipping = rules::money_or_zero(&get("shipping"), &s.currency);
    let code = rules::normalise_code(&get("code"));
    let (mut item_off, mut ship_off, mut used_code) = (0, 0, String::new());
    if !code.is_empty() {
        match repo::discount_by_code(&state.db, &code).await {
            Some(d) => {
                let (a, b) = rules::discount_amount(rules::DiscountKind::from_key(&d.kind), d.value, subtotal, shipping);
                item_off = a;
                ship_off = b;
                used_code = d.code;
            }
            None => return flash_to(&back_to, "error", "That discount code does not exist."),
        }
    }
    let totals = rules::totals(subtotal, taxable, item_off, shipping, ship_off, s.tax_rate_bp, s.tax_included, s.tax_shipping);
    let address = rules::Address::from_form(|k| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()));
    let paid = get("paid") == "1";
    let order = repo::place_order(
        &state.db,
        &repo::NewOrder {
            email: email.clone(),
            phone: address.phone.clone(),
            first_name: address.first_name.clone(),
            last_name: address.last_name.clone(),
            accepts_marketing: false,
            currency: s.currency.clone(),
            lines: new_lines,
            totals,
            tax_included: s.tax_included,
            discount_code: used_code,
            shipping_method: if ships { rules::clean(&get("shipping_name"), 120) } else { String::new() },
            address,
            payment_method: "admin".into(),
            provider_product: None,
            financial_status: if paid || totals.total == 0 { "paid".into() } else { "pending".into() },
            note: rules::clean_multiline(&get("note"), 1000),
            source: "admin".into(),
            cart_id: None,
            actor: who(&actor),
        },
    )
    .await;
    match order {
        Ok(o) => {
            if f.iter().any(|(k, _)| k == "receipt") {
                vuuut_api::ecom::receipt(&state, &o).await;
            }
            if o.is_paid() {
                vuuut_api::ecom::on_paid(&state, &o).await;
            }
            flash_to(&format!("{ORDERS}{}/", o.id), "ok", &format!("Order #{} created.", o.number))
        }
        Err(e) => flash_to(&back_to, "error", &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// abandoned checkouts
// ---------------------------------------------------------------------------

pub(crate) async fn abandoned_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let cutoff = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::hours(1)));
    let carts = repo::abandoned(&state.db, &cutoff, 200).await;
    let mut rows = String::new();
    for c in &carts {
        let lines = c.lines();
        let vs = repo::variants_by_ids(&state.db, &lines.iter().map(|l| l.variant_id.clone()).collect::<Vec<_>>()).await;
        let ps = repo::products_by_ids(&state.db, &vs.iter().map(|v| v.product_id.clone()).collect::<Vec<_>>()).await;
        let value: i64 = lines.iter().filter_map(|l| vs.iter().find(|v| v.id == l.variant_id).map(|v| v.price_cents * l.qty)).sum();
        let what = lines
            .iter()
            .filter_map(|l| vs.iter().find(|v| v.id == l.variant_id).and_then(|v| ps.iter().find(|p| p.id == v.product_id)).map(|p| format!("{} × {}", p.title, l.qty)))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push_str(&format!(
            r#"<tr><td><a href="mailto:{e}">{e}</a></td><td>{d}</td><td>{w}</td><td class="ec-num">{v}</td><td><a class="btn text small" href="{ORDERS}new/?email={eu}">Create order</a></td></tr>"#,
            e = esc(&c.email),
            eu = vuuut_core::social_media::encode(&c.email),
            d = date(&c.updated_at),
            w = esc(&what),
            v = money(value, &s),
        ));
    }
    let body = format!(
        r#"<p class="muted rise" style="--i:2">Checkouts where the shopper gave an email, then left without ordering, more than an hour ago. Carts untouched for 60 days are forgotten.</p>
<div class="card ec-table-card rise" style="--i:3">{}</div>"#,
        if carts.is_empty() { format!(r#"<div class="ec-empty">{}<p><b>No abandoned checkouts</b></p></div>"#, svg("cart")) } else { format!(r#"<table class="ec-table ec-list"><thead><tr><th>Email</th><th>Last active</th><th>Cart</th><th class="ec-num">Value</th><th></th></tr></thead><tbody>{rows}</tbody></table>"#) }
    );
    let _ = repo::purge_carts(&state.db, &vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(60)))).await;
    ecom_page(&actor, "Abandoned checkouts", "orders", &q, "", Some(ORDERS), &body)
}

// ---------------------------------------------------------------------------
// downloads
// ---------------------------------------------------------------------------

async fn downloads_card(state: &AppState, o: &repo::Order, act: &str) -> String {
    let grants = repo::grants_for_order(&state.db, &o.id).await;
    if grants.is_empty() {
        let items = repo::order_items(&state.db, &o.id).await;
        let pids: Vec<String> = items.iter().filter_map(|i| i.product_id.clone()).collect();
        if repo::digital_ids(&state.db, &pids).await.is_empty() || o.is_cancelled() {
            return String::new();
        }
        return format!(
            r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-purple">{}</span>Downloads</h2><p class="muted ec-p">Locked until the order is paid. Marking it paid unlocks the files, makes {} a member and emails the download link.</p></section>"#,
            svg("file"),
            esc(&o.email)
        );
    }
    let log = repo::download_log_for_order(&state.db, &o.id, 8).await;
    let rows: String = grants
        .iter()
        .map(|g| {
            format!(
                r#"<div class="row"><div class="body"><span class="title">{t}{rv}</span><span class="detail">{used}{exp}{last}</span></div>
<div class="trailing"><form method="post" action="{act}downloads/"><input type="hidden" name="op" value="reset"><input type="hidden" name="grant" value="{id}"><button class="btn text small" type="submit"{dis}>Reset count</button></form></div></div>"#,
                t = esc(&g.title),
                rv = if g.revoked != 0 { r#" <span class="chip error">Revoked</span>"# } else { "" },
                used = if g.download_limit > 0 { format!("{} of {} downloads used", g.download_count, g.download_limit) } else { format!("{} downloaded", countable(g.download_count, "time")) },
                exp = g.expires_at.as_deref().map(|e| format!(" · until {}", esc(e.get(..10).unwrap_or(e)))).unwrap_or_default(),
                last = g.last_download_at.as_deref().map(|e| format!(" · last {}", date(e))).unwrap_or_default(),
                id = esc(&g.id),
                dis = if g.download_count == 0 { " disabled" } else { "" },
            )
        })
        .collect();
    let revoked = grants.iter().all(|g| g.revoked != 0);
    format!(
        r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-purple">{ic}</span>Downloads <span class="muted">· for {email}</span></h2>
<div class="rows">{rows}</div>
<div class="ec-row" style="flex-wrap:wrap">
<form method="post" action="{act}downloads/"><input type="hidden" name="op" value="resend"><button class="btn tonal small" type="submit"{rdis}>{send} Resend download email</button></form>
<form method="post" action="{act}downloads/"><input type="hidden" name="op" value="{op}"><button class="btn text small{danger}" type="submit">{label}</button></form></div>
{log}</section>"#,
        ic = svg("file"),
        email = esc(&o.email),
        rdis = if revoked { " disabled" } else { "" },
        send = svg("send"),
        op = if revoked { "restore" } else { "revoke" },
        danger = if revoked { "" } else { " danger" },
        label = if revoked { "Restore access" } else { "Revoke access" },
        log = if log.is_empty() {
            r#"<p class="muted ec-hint">Not downloaded yet.</p>"#.to_string()
        } else {
            format!(
                r#"<details class="ec-more"><summary>Download log</summary><table class="ec-table">{}</table></details>"#,
                log.iter().map(|(when, f, ip)| format!("<tr><td>{}</td><td>{}</td><td class=\"muted\">{}</td></tr>", date(when), esc(f), esc(ip))).collect::<String>()
            )
        },
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct DownloadsForm {
    op: String,
    grant: String,
}

pub(crate) async fn order_downloads(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<DownloadsForm>) -> Response {
    let (actor, o) = match load(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    match f.op.as_str() {
        "resend" => {
            if !rules::order_unlocks_downloads(&o.status, &o.financial_status) {
                return flash_to(&back(&o), "error", "The order is not paid, so there is nothing to send.");
            }
            vuuut_api::ecom::send_download_email(&state, &o).await;
            repo::add_event(&state.db, &o.id, "email", &format!("Download link sent to {}.", o.email), &who(&actor)).await;
            flash_to(&back(&o), "ok", "Download email sent.")
        }
        "revoke" | "restore" => {
            vuuut_api::ecom::set_downloads_revoked(&state, &o, f.op == "revoke", "", &who(&actor)).await;
            flash_to(&back(&o), "ok", if f.op == "revoke" { "Downloads revoked." } else { "Downloads restored." })
        }
        "reset" => {
            match repo::find_grant(&state.db, &f.grant).await.filter(|g| g.order_id == o.id) {
                Some(g) => {
                    let _ = repo::reset_download_count(&state.db, &g.id).await;
                    repo::add_event(&state.db, &o.id, "download", &format!("Download count reset for {}.", g.title), &who(&actor)).await;
                    flash_to(&back(&o), "ok", "Download count reset.")
                }
                None => flash_to(&back(&o), "error", "That download no longer exists."),
            }
        }
        _ => Redirect::to(&back(&o)).into_response(),
    }
}
