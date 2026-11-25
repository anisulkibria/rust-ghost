//! Tools → E-commerce & Online Stores: the admin.
//!
//! The rules are [`vuuut_core::ecom`], the queries
//! [`vuuut_db::repos::ecom`], the mail and Stripe calls [`vuuut_api::ecom`],
//! and the storefront at `/shop/` is `vuuut_front::shop`. What is here:
//!
//! * **Home** (`/ghost/ecom/`) — today's and this month's sales, a 30-day
//!   chart, what needs doing (orders to ship, unpaid, low stock), and the
//!   store's switch.
//! * **Orders** — `ecom/orders.rs`: list, detail, pay, fulfil, refund,
//!   cancel, archive, staff-created orders, abandoned checkouts, CSV.
//! * **Products** — `ecom/products.rs`: the catalogue and the product
//!   editor, whose description is the post editor's Koenig canvas.
//! * **Collections** — `ecom/collections.rs`: manual and smart.
//! * **Inventory** — `ecom/inventory.rs`: every variant's stock, editable.
//! * **Customers** — `ecom/customers.rs`.
//! * **Discounts** — `ecom/discounts.rs`.
//! * **Analytics** — sales over time, best sellers, averages.
//! * **Settings** — `ecom/settings.rs`: store, payments, shipping, taxes,
//!   checkout, notifications, policies.
//!
//! ## The security contract
//!
//! Editor and above for the catalogue, orders and customers; Administrator
//! for discounts and settings. Every write is origin-checked; every value is
//! escaped where it is shown; ids are validated before they reach a query.
//! Customer details are personal data and appear only here.

use super::*;
use vuuut_core::ecom::{self as rules, Settings};
use vuuut_db::repos::ecom as repo;

mod collections;
mod customers;
mod discounts;
mod inventory;
mod orders;
mod products;
mod settings;
mod transfer;
pub(crate) use collections::*;
pub(crate) use customers::*;
pub(crate) use discounts::*;
pub(crate) use inventory::*;
pub(crate) use orders::*;
pub(crate) use products::*;
pub(crate) use settings::*;
pub(crate) use transfer::*;

pub(crate) const BASE: &str = "/ghost/ecom/";

pub(crate) async fn guard(state: &AppState, headers: &HeaderMap, role: Role, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, role).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

pub(crate) fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    // The message goes in the query, which has to come before any `#anchor`.
    let (path, anchor) = path.split_once('#').map(|(p, a)| (p, format!("#{a}"))).unwrap_or((path, String::new()));
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}{anchor}")).into_response()
}

pub(crate) fn store_settings(state: &AppState) -> Settings {
    vuuut_api::ecom::settings(state)
}

pub(crate) fn money(c: i64, s: &Settings) -> String {
    esc(&rules::format_money(c, &s.currency))
}

pub(crate) fn valid_id(id: &str) -> bool {
    vuuut_core::ids::is_objectid(id)
}

/// The tool's tabs. Discounts and Settings only for Administrators.
pub(crate) fn tabs(active: &str, admin: bool) -> String {
    let items: [(&str, &str, &str, bool); 9] = [
        ("home", "Home", "/ghost/ecom/", true),
        ("orders", "Orders", "/ghost/ecom/orders/", true),
        ("products", "Products", "/ghost/ecom/products/", true),
        ("collections", "Collections", "/ghost/ecom/collections/", true),
        ("inventory", "Inventory", "/ghost/ecom/inventory/", true),
        ("customers", "Customers", "/ghost/ecom/customers/", true),
        ("discounts", "Discounts", "/ghost/ecom/discounts/", admin),
        ("analytics", "Analytics", "/ghost/ecom/analytics/", true),
        ("settings", "Settings", "/ghost/ecom/settings/", admin),
    ];
    format!(
        r#"<div class="seg-bar ec-tabs rise" style="--i:1">{}</div>"#,
        items
            .iter()
            .filter(|i| i.3)
            .map(|(k, l, h, _)| format!(r#"<a class="seg{on}" href="{h}">{l}</a>"#, on = if *k == active { " on" } else { "" }))
            .collect::<String>()
    )
}

/// A page of the tool, with its header, tabs and flash banner.
pub(crate) fn ecom_page(actor: &Actor, title: &str, tab: &str, q: &HashMap<String, String>, extra_actions: &str, back: Option<&str>, body: &str) -> Response {
    let head = match back {
        Some(b) => crate::tools::tool_head_back("ecom", b, None, extra_actions),
        None => crate::tools::tool_head("ecom", None, extra_actions),
    };
    page_for_role(
        actor.role,
        title,
        "tools",
        Some(&actor.user),
        &format!(r#"<div class="ec">{head}{tabs}{banner}{body}</div>"#, tabs = tabs(tab, actor.role.is_admin()), banner = flash_banner(q)),
        true,
    )
    .into_response()
}

pub(crate) fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{ic}</span><span class="ec-tile-value">{value}</span><span class="ec-tile-label">{label}</span><span class="ec-tile-detail">{detail}</span>"#,
        ic = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(h) => format!(r#"<a class="card padded ec-tile" href="{h}">{body}</a>"#),
        None => format!(r#"<div class="card padded ec-tile">{body}</div>"#),
    }
}

pub(crate) fn chip_for_financial(k: &str) -> String {
    let cls = match k {
        "paid" => "published",
        "pending" => "scheduled",
        "refunded" | "voided" => "draft",
        _ => "tonal",
    };
    format!(r#"<span class="chip {cls}">{}</span>"#, rules::financial_label(k))
}

pub(crate) fn chip_for_fulfillment(k: &str) -> String {
    if k == "fulfilled" { r#"<span class="chip published">Fulfilled</span>"#.into() } else { r#"<span class="chip scheduled">Unfulfilled</span>"#.into() }
}

/// Thumbnail of a product or line, or a tinted icon.
pub(crate) fn thumb(url: &str) -> String {
    if url.is_empty() {
        format!(r#"<span class="ic tint-pink ec-thumb-empty">{}</span>"#, svg("bag"))
    } else {
        format!(r#"<img class="ec-thumb" src="{}" alt="" loading="lazy">"#, esc(url))
    }
}

pub(crate) fn date(d: &str) -> String {
    esc(d.get(..16).unwrap_or(d))
}

/// `(day label, cents, orders)` for the last `days` days, oldest first.
fn daily(rows: &[repo::SaleRow], days: i64) -> Vec<(String, i64, i64)> {
    let today = chrono::Utc::now().date_naive();
    (0..days)
        .rev()
        .map(|back| {
            let d = today - chrono::Duration::days(back);
            let key = d.format("%Y-%m-%d").to_string();
            let mut cents = 0;
            let mut n = 0;
            for r in rows.iter().filter(|r| r.created_at.starts_with(&key)) {
                n += 1;
                if r.financial_status != "voided" {
                    cents += r.total_cents - r.refunded_cents;
                }
            }
            (key, cents, n)
        })
        .collect()
}

/// A single-series bar chart of sales per day: one hue, thin bars with
/// rounded data-ends on the baseline, a 2px gap, a recessive axis, a hover
/// tooltip on each bar and a table for anyone who would rather read.
pub(crate) fn sales_chart(points: &[(String, i64, i64)], s: &Settings) -> String {
    let max = points.iter().map(|p| p.1).max().unwrap_or(0).max(1);
    let n = points.len().max(1) as f64;
    let (w, h, pad_b) = (720.0, 180.0, 22.0);
    let slot = w / n;
    let bar = (slot - 2.0).clamp(2.0, 18.0);
    let bars: String = points
        .iter()
        .enumerate()
        .map(|(i, (day, cents, orders))| {
            let bh = if *cents <= 0 { 0.0 } else { ((*cents as f64 / max as f64) * (h - pad_b - 8.0)).max(3.0) };
            let x = i as f64 * slot + (slot - bar) / 2.0;
            let y = h - pad_b - bh;
            let r = (bar / 2.0).min(4.0).min(bh);
            let label = format!("{day}: {} · {orders} order{}", rules::format_money(*cents, &s.currency), if *orders == 1 { "" } else { "s" });
            format!(
                r#"<g class="ec-bar" tabindex="0"><rect class="ec-hit" x="{hx:.1}" y="0" width="{slot:.1}" height="{hb:.1}"/>{shape}<title>{t}</title></g>"#,
                hx = i as f64 * slot,
                hb = h - pad_b,
                shape = if bh > 0.0 {
                    // Rounded top, square foot on the baseline.
                    format!(
                        r#"<path d="M{x:.1},{base:.1} V{yr:.1} Q{x:.1},{y:.1} {xr:.1},{y:.1} H{xl:.1} Q{x2:.1},{y:.1} {x2:.1},{yr:.1} V{base:.1} Z"/>"#,
                        base = h - pad_b,
                        yr = y + r,
                        xr = x + r,
                        xl = x + bar - r,
                        x2 = x + bar,
                    )
                } else {
                    String::new()
                },
                t = esc(&label),
            )
        })
        .collect();
    let ticks: String = points
        .iter()
        .enumerate()
        .filter(|(i, _)| (i % 7 == 0 && points.len() - 1 - i >= 4) || *i == points.len() - 1)
        .map(|(i, (day, _, _))| format!(r#"<text x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#, i as f64 * slot + slot / 2.0, h - 6.0, esc(day.get(5..).unwrap_or(day))))
        .collect();
    let table: String = points.iter().rev().map(|(d, c, o)| format!("<tr><td>{}</td><td>{o}</td><td>{}</td></tr>", esc(d), money(*c, s))).collect();
    format!(
        r#"<figure class="ec-chart"><svg viewBox="0 0 {w} {h}" role="img" aria-label="Sales per day, last {n} days" preserveAspectRatio="none">
<line class="ec-axis" x1="0" x2="{w}" y1="{base}" y2="{base}"/>{bars}{ticks}</svg>
<details class="ec-chart-table"><summary>Show as table</summary><table class="ec-table"><thead><tr><th>Day</th><th>Orders</th><th>Sales</th></tr></thead><tbody>{table}</tbody></table></details></figure>"#,
        base = h - pad_b,
        n = points.len(),
    )
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

fn home_empty(icon: &str, tint: &str, message: &str) -> String {
    format!(
        r#"<div class="ec-home-empty"><span class="ic tint-{tint}" aria-hidden="true">{icon}</span><p>{message}</p></div>"#,
        icon = svg(icon),
        message = esc(message),
    )
}

pub(crate) async fn ecom_home(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let from = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(30)));
    let rows = repo::sales_since(&state.db, &from, &s.currency).await;
    let points = daily(&rows, 30);
    let today = points.last().map(|p| (p.1, p.2)).unwrap_or((0, 0));
    let month: i64 = points.iter().map(|p| p.1).sum();
    let month_orders: i64 = points.iter().map(|p| p.2).sum();
    let counts = repo::order_counts(&state.db).await;
    let pc = repo::product_counts(&state.db).await;
    let (_, out, low) = repo::stock_summary(&state.db, s.low_stock).await;
    let n = |k: &str| counts.get(k).copied().unwrap_or(0);
    let (recent, _) = repo::browse_orders(&state.db, &repo::OrderBrowse { view: "all", limit: 6, ..Default::default() }).await;
    let top = repo::top_products(&state.db, &from, &s.currency, 5).await;

    let banner = if s.enabled {
        format!(
            r#"<div class="card padded ec-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>Your store is open</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{public}</a> · {active} active products · {methods}</p></div>
<a class="btn tonal" href="{href}" target="_blank" rel="noopener">{open} View store</a></div>"#,
            ic = svg("store"),
            href = vuuut_core::url::base(&vuuut_api::ecom::paths(&state).public_path("/shop/")),
            public = esc(&vuuut_api::ecom::paths(&state).public_path("/shop/")),
            site = esc(state.site_url().trim_end_matches('/')),
            active = pc.get("active").copied().unwrap_or(0),
            methods = esc(&{
                let m = s.payment_methods(vuuut_api::ecom::stripe(&state).is_some());
                if m.is_empty() { "no payment method on".to_string() } else { m.iter().map(|x| x.label()).collect::<Vec<_>>().join(", ") }
            }),
            open = svg("open"),
        )
    } else {
        format!(
            r#"<div class="card padded ec-live ec-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>Your store is not open yet.</b>
<p class="muted">Add products, set up payments and shipping, then open the store and it appears at {public} inside your theme.</p></div>{btn}</div>"#,
            public = esc(&vuuut_api::ecom::paths(&state).public_path("/shop/")),
            ic = svg("hide"),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/ecom/settings/">{} Set up</a>"#, svg("settings")) } else { String::new() },
        )
    };
    let setup = {
        let steps = [
            (pc.values().sum::<i64>() > 0, "Add your first product", "/ghost/ecom/products/new/"),
            (!s.payment_methods(vuuut_api::ecom::stripe(&state).is_some()).is_empty(), "Choose how customers pay", "/ghost/ecom/settings/#payments"),
            (!repo::shipping_rates(&state.db).await.is_empty(), "Set your shipping rates", "/ghost/ecom/settings/#shipping"),
            (s.enabled, "Open the store", "/ghost/ecom/settings/"),
        ];
        let done = steps.iter().filter(|x| x.0).count();
        if done == steps.len() {
            String::new()
        } else {
            format!(
                r#"<div class="card padded ec-setup rise" style="--i:3"><div class="ec-setup-head"><b>Get your store ready</b><span class="muted">{done} of {all} done</span></div><div class="ec-progress"><span style="width:{pct}%"></span></div>
<ol>{items}</ol></div>"#,
                all = steps.len(),
                pct = done * 100 / steps.len(),
                items = steps.iter().map(|(ok, l, h)| format!(r#"<li class="{c}"><a href="{h}">{ic} {l}</a></li>"#, c = if *ok { "done" } else { "" }, ic = svg(if *ok { "check" } else { "add" }))).collect::<String>(),
            )
        }
    };
    let todo: String = [
        (n("unfulfilled"), "order", "to fulfil", "/ghost/ecom/orders/?view=unfulfilled", "truck", "blue"),
        (n("unpaid"), "order", "awaiting payment", "/ghost/ecom/orders/?view=unpaid", "card", "amber"),
        (out, "variant", "out of stock", "/ghost/ecom/inventory/?low=1", "box", "red"),
        (low, "variant", "low on stock", "/ghost/ecom/inventory/?low=1", "warning", "amber"),
    ]
    .into_iter()
    .filter(|task| task.0 > 0)
    .map(|(count, noun, label, href, icon, tint)| {
        format!(
            r#"<a class="row ec-home-row ec-todo-row" href="{href}"><span class="ic tint-{tint}" aria-hidden="true">{icon}</span>
<div class="body"><span class="title">{count} {label}</span></div><span class="chev" aria-hidden="true">{chevron}</span></a>"#,
            count = esc(&countable(count, noun)),
            icon = svg(icon),
            chevron = svg("chevron"),
        )
    })
    .collect();
    let recent_html: String = recent
        .iter()
        .map(|o| {
            let name = o.address().name();
            let customer = if name.is_empty() { &o.email } else { &name };
            format!(
                r#"<a class="row ec-home-row ec-recent-row" href="/ghost/ecom/orders/{id}/">
<div class="body"><span class="title">#{number}{customer}</span><span class="detail">{date}</span></div>
<span class="ec-order-summary">{status}<b class="ec-home-amount">{total}</b></span></a>"#,
                id = esc(&o.id),
                number = o.number,
                customer = if customer.is_empty() { String::new() } else { format!(" · {}", esc(customer)) },
                date = date(&o.created_at),
                status = chip_for_financial(&o.financial_status),
                total = esc(&rules::format_money(o.total_cents, &o.currency)),
            )
        })
        .collect();
    let top_html: String = top
        .iter()
        .map(|(id, title, units, revenue)| {
            let row = format!(
                r#"<div class="body"><span class="title">{title}</span><span class="detail">{units} sold</span></div><b class="ec-home-amount">{revenue}</b>"#,
                title = esc(title),
                revenue = money(*revenue, &s),
            );
            if valid_id(id) {
                format!(r#"<a class="row ec-home-row ec-seller-row" href="/ghost/ecom/products/{id}/">{row}</a>"#)
            } else {
                format!(r#"<div class="row ec-home-row ec-seller-row">{row}</div>"#)
            }
        })
        .collect();
    let body = format!(
        r#"{banner}{setup}
<div class="ec-tiles rise" style="--i:3">{a}{b}{c}{d}</div>
<div class="ec-home-grid">
<section class="card ec-home-panel rise" aria-labelledby="ec-home-sales" style="--i:4">
<header class="ec-home-panel-head"><div class="ec-home-heading"><h2 id="ec-home-sales">Sales</h2><span class="ec-home-period">Last 30 days · {currency}</span></div></header>
{chart}</section>
<section class="card rows ec-home-panel rise" aria-labelledby="ec-home-todo" style="--i:4">
<header class="ec-home-panel-head"><h2 id="ec-home-todo">To do</h2></header>
{todo}</section>
<section class="card rows ec-home-panel rise" aria-labelledby="ec-home-recent" style="--i:5">
<header class="ec-home-panel-head"><h2 id="ec-home-recent">Recent orders</h2><a class="btn text small" href="/ghost/ecom/orders/">View orders</a></header>
{recent}</section>
<section class="card rows ec-home-panel rise" aria-labelledby="ec-home-sellers" style="--i:5">
<header class="ec-home-panel-head"><div class="ec-home-heading"><h2 id="ec-home-sellers">Best sellers</h2><span class="ec-home-period">Last 30 days</span></div><a class="btn text small" href="/ghost/ecom/analytics/?days=30">View analytics</a></header>
{top}</section>
</div>"#,
        currency = esc(&s.currency),
        a = tile("Today", &rules::format_money(today.0, &s.currency), &countable(today.1, "order"), Some("/ghost/ecom/orders/"), "chart", "green"),
        b = tile("30 days", &rules::format_money(month, &s.currency), &countable(month_orders, "order"), Some("/ghost/ecom/analytics/"), "trending", "blue"),
        c = tile("To fulfil", &n("unfulfilled").to_string(), "Open orders not yet shipped", Some("/ghost/ecom/orders/?view=unfulfilled"), "truck", if n("unfulfilled") > 0 { "amber" } else { "slate" }),
        d = tile("Products", &pc.get("active").copied().unwrap_or(0).to_string(), &format!("Active · {} drafts", pc.get("draft").copied().unwrap_or(0)), Some("/ghost/ecom/products/"), "bag", "pink"),
        chart = if month_orders == 0 {
            home_empty("chart", "green", "No orders in the last 30 days.")
        } else {
            format!(r#"<div class="ec-home-chart">{}</div>"#, sales_chart(&points, &s))
        },
        recent = if recent_html.is_empty() { home_empty("receipt", "blue", "No orders yet.") } else { recent_html },
        todo = if todo.is_empty() { home_empty("check", "green", "Nothing needs your attention.") } else { todo },
        top = if top_html.is_empty() { home_empty("trending", "purple", "No product sales in the last 30 days.") } else { top_html },
    );
    ecom_page(
        &actor,
        "E-commerce & Online Stores",
        "home",
        &q,
        &format!(r#"<a class="btn text" href="/ghost/ecom/orders/new/">{} Create order</a><a class="btn" href="/ghost/ecom/products/new/">{} Add product</a>"#, svg("receipt"), svg("add")),
        None,
        &body,
    )
}

// ---------------------------------------------------------------------------
// Analytics
// ---------------------------------------------------------------------------

pub(crate) async fn ecom_analytics(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let days: i64 = match q.get("days").map(String::as_str) {
        Some("7") => 7,
        Some("90") => 90,
        _ => 30,
    };
    let from = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(days)));
    let rows = repo::sales_since(&state.db, &from, &s.currency).await;
    let points = daily(&rows, days);
    let gross: i64 = rows.iter().filter(|r| r.financial_status != "voided").map(|r| r.total_cents).sum();
    let refunds: i64 = rows.iter().map(|r| r.refunded_cents).sum();
    let discounts: i64 = rows.iter().map(|r| r.discount_cents).sum();
    let tax: i64 = rows.iter().map(|r| r.tax_cents).sum();
    let shipping: i64 = rows.iter().map(|r| r.shipping_cents).sum();
    let orders = rows.len() as i64;
    let aov = if orders > 0 { gross / orders } else { 0 };
    let mut customers: Vec<&str> = rows.iter().filter_map(|r| r.customer_id.as_deref()).collect();
    customers.sort();
    customers.dedup();
    let top = repo::top_products(&state.db, &from, &s.currency, 10).await;
    let range = [(7, "7 days"), (30, "30 days"), (90, "90 days")]
        .iter()
        .map(|(d, l)| format!(r#"<a class="chip{on}" href="?days={d}">{l}</a>"#, on = if *d == days { " tonal" } else { "" }))
        .collect::<String>();
    let top_rows: String = top
        .iter()
        .enumerate()
        .map(|(i, (_, t, u, r))| format!("<tr><td>{}</td><td>{}</td><td>{u}</td><td>{}</td></tr>", i + 1, esc(t), money(*r, &s)))
        .collect();
    let body = format!(
        r#"<div class="ec-range rise" style="--i:2">{range}</div>
<div class="ec-tiles rise" style="--i:3">{a}{b}{c}{d}</div>
<div class="section">Net sales per day</div><div class="card padded rise" style="--i:4">{chart}</div>
<div class="ec-split">
<div><div class="section">Breakdown</div><div class="card padded rise" style="--i:5"><table class="ec-table ec-kv">
<tr><td>Gross sales</td><td>{gross}</td></tr><tr><td>Discounts</td><td>−{disc}</td></tr><tr><td>Refunds</td><td>−{refunds}</td></tr>
<tr><td>Shipping charged</td><td>{ship}</td></tr><tr><td>Taxes</td><td>{tax}</td></tr><tr class="ec-strong"><td>Net sales</td><td>{net}</td></tr></table></div></div>
<div><div class="section">Best-selling products</div><div class="card padded rise" style="--i:5">{top}</div></div>
</div>"#,
        a = tile("Net sales", &rules::format_money(gross - refunds, &s.currency), &format!("Last {days} days"), None, "chart", "green"),
        b = tile("Orders", &orders.to_string(), "Not counting cancelled", Some("/ghost/ecom/orders/"), "receipt", "blue"),
        c = tile("Average order", &rules::format_money(aov, &s.currency), "Gross ÷ orders", None, "trending", "purple"),
        d = tile("Customers", &customers.len().to_string(), "Who ordered in this period", Some("/ghost/ecom/customers/"), "group", "teal"),
        chart = sales_chart(&points, &s),
        gross = money(gross, &s),
        disc = money(discounts, &s),
        refunds = money(refunds, &s),
        ship = money(shipping, &s),
        tax = money(tax, &s),
        net = money(gross - refunds, &s),
        top = if top_rows.is_empty() { r#"<p class="muted" style="margin:0">No sales in this period.</p>"#.to_string() } else { format!(r#"<table class="ec-table"><thead><tr><th>#</th><th>Product</th><th>Units</th><th>Sales</th></tr></thead><tbody>{top_rows}</tbody></table>"#) },
    );
    ecom_page(&actor, "Store analytics", "analytics", &q, "", Some(BASE), &body)
}
