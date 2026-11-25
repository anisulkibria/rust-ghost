//! Tools → E-commerce & Online Stores: what both the admin and the
//! storefront need that touches mail or Stripe.
//!
//! * [`settings`] — the store's settings, read once through the rules.
//! * [`stripe`] — the site's Stripe keys, when card payments can work.
//! * [`receipt`], [`notify_staff`], [`shipped`] — the order emails. Each is
//!   sent in the background: a slow mail server must never hold a buyer on
//!   the checkout page, and a failed send is logged, not shown to them.

use crate::AppState;
use vuuut_core::ecom::{self as rules, Settings};
use vuuut_db::repos::ecom as repo;

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

/// The store's public addresses, from its settings.
pub fn paths(state: &AppState) -> rules::Paths {
    rules::Paths::from_settings(&settings(state))
}

/// The Stripe configuration, if the site has keys.
pub fn stripe(state: &AppState) -> Option<vuuut_stripe::StripeConfig> {
    vuuut_stripe::StripeConfig::from_settings(&state.settings)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The public address of an order's status page.
pub fn order_url(state: &AppState, o: &repo::Order) -> String {
    vuuut_core::url::absolute(&state.site_url(), &vuuut_core::url::base(&paths(state).public_path(&format!("/shop/orders/{}/", o.token))))
}

fn summary_html(o: &repo::Order, items: &[repo::OrderItem]) -> String {
    let money = |c: i64| esc(&rules::format_money(c, &o.currency));
    let rows: String = items
        .iter()
        .map(|i| {
            format!(
                r#"<tr><td style="padding:8px 0;border-bottom:1px solid #eee">{t}{v} × {q}</td><td style="padding:8px 0;border-bottom:1px solid #eee;text-align:right">{p}</td></tr>"#,
                t = esc(&i.title),
                v = if i.variant_title.is_empty() || i.variant_title == rules::DEFAULT_VARIANT { String::new() } else { format!(" <span style=\"color:#777\">({})</span>", esc(&i.variant_title)) },
                q = i.quantity,
                p = money(i.total_cents),
            )
        })
        .collect();
    let line = |label: &str, c: i64| format!(r#"<tr><td style="padding:4px 0;color:#555">{label}</td><td style="padding:4px 0;text-align:right">{}</td></tr>"#, money(c));
    let mut totals = line("Subtotal", o.subtotal_cents);
    if o.discount_cents > 0 {
        totals.push_str(&line(&format!("Discount {}", esc(&o.discount_code)), -o.discount_cents));
    }
    totals.push_str(&line("Shipping", o.shipping_cents));
    if o.tax_cents > 0 {
        totals.push_str(&line(if o.tax_included != 0 { "Tax (included)" } else { "Tax" }, o.tax_cents));
    }
    format!(
        r#"<table style="width:100%;border-collapse:collapse;font-size:14px">{rows}{totals}
<tr><td style="padding:10px 0;font-weight:700;border-top:2px solid #222">Total</td><td style="padding:10px 0;font-weight:700;text-align:right;border-top:2px solid #222">{total}</td></tr></table>"#,
        total = money(o.total_cents),
    )
}

fn wrap(store: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html><body style="margin:0;background:#f6f6f6;font-family:-apple-system,Segoe UI,Roboto,sans-serif;color:#222">
<div style="max-width:560px;margin:24px auto;background:#fff;border-radius:14px;padding:28px">
<p style="margin:0 0 18px;font-weight:700;font-size:18px">{store}</p>{body}</div></body></html>"#,
        store = esc(store),
    )
}

fn spawn_send(state: &AppState, to: String, subject: String, html: String, text: String) {
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let cfg = vuuut_mail::MailConfig::from_settings(&state.settings, &site_title, &state.site_url());
    tokio::spawn(async move {
        let msg = vuuut_mail::Message { to: to.clone(), subject, html, text };
        if let Err(e) = vuuut_mail::send(&cfg, &msg).await {
            tracing::warn!("ecom: mail to {to} failed: {}", e.message());
        }
    });
}

/// The buyer's receipt, when receipts are on.
pub async fn receipt(state: &AppState, o: &repo::Order) {
    let s = settings(state);
    if !s.send_receipts || o.email.is_empty() {
        return;
    }
    let title = state.settings.get_str("title").unwrap_or_default();
    let store = s.name(&title).to_string();
    let items = repo::order_items(&state.db, &o.id).await;
    let pay = match o.payment_method.as_str() {
        "bank" if !o.is_paid() => format!(
            r#"<p style="background:#fff7e6;border-radius:10px;padding:12px 14px;font-size:14px"><b>Pay by bank transfer</b><br>{}<br>Use <b>#{}</b> as the reference.</p>"#,
            esc(&s.bank_details).replace('\n', "<br>"),
            o.number
        ),
        "manual" if !o.is_paid() => "<p style=\"font-size:14px\">You will pay when your order is delivered.</p>".to_string(),
        _ => String::new(),
    };
    let url = order_url(state, o);
    let body = format!(
        r#"<h1 style="font-size:22px;margin:0 0 6px">Thank you for your order</h1>
<p style="color:#555;margin:0 0 18px">Order #{n} · {date}</p>{pay}{summary}
<p style="margin:22px 0 0"><a href="{url}" style="display:inline-block;background:#222;color:#fff;text-decoration:none;padding:11px 18px;border-radius:999px">View your order</a></p>"#,
        n = o.number,
        date = esc(o.created_at.get(..10).unwrap_or(&o.created_at)),
        summary = summary_html(o, &items),
        url = esc(&url),
    );
    let text = format!("Thank you for your order #{} at {store}.\n\nTotal: {}\n\nView it: {url}\n", o.number, rules::format_money(o.total_cents, &o.currency));
    spawn_send(state, o.email.clone(), format!("Order #{} confirmed — {store}", o.number), wrap(&store, &body), text);
}

/// The store's own notice of a new order, to the address in Settings.
pub async fn notify_staff(state: &AppState, o: &repo::Order) {
    let s = settings(state);
    if s.notify_email.is_empty() {
        return;
    }
    let title = state.settings.get_str("title").unwrap_or_default();
    let store = s.name(&title).to_string();
    let items = repo::order_items(&state.db, &o.id).await;
    let admin = format!("{}/ecom/orders/{}/", state.admin_url(), o.id);
    let body = format!(
        r#"<h1 style="font-size:20px;margin:0 0 6px">New order #{n}</h1><p style="color:#555;margin:0 0 18px">{email} · {pay}</p>{summary}
<p style="margin:22px 0 0"><a href="{admin}">Open it in the admin</a></p>"#,
        n = o.number,
        email = esc(&o.email),
        pay = esc(rules::payment_label(&o.payment_method)),
        summary = summary_html(o, &items),
        admin = esc(&admin),
    );
    let text = format!("New order #{} from {} — {}\n{admin}\n", o.number, o.email, rules::format_money(o.total_cents, &o.currency));
    spawn_send(state, s.notify_email.clone(), format!("[{store}] New order #{} — {}", o.number, rules::format_money(o.total_cents, &o.currency)), wrap(&store, &body), text);
}

/// "Your order is on its way", with the tracking link when there is one.
pub async fn shipped(state: &AppState, o: &repo::Order) {
    let s = settings(state);
    if !s.send_receipts || o.email.is_empty() {
        return;
    }
    let title = state.settings.get_str("title").unwrap_or_default();
    let store = s.name(&title).to_string();
    let track = if !o.tracking_url.is_empty() {
        format!(r#"<p><a href="{u}" style="display:inline-block;background:#222;color:#fff;text-decoration:none;padding:11px 18px;border-radius:999px">Track your package</a></p>"#, u = esc(&o.tracking_url))
    } else if !o.tracking_number.is_empty() {
        format!("<p>{} tracking number: <b>{}</b></p>", esc(&o.tracking_company), esc(&o.tracking_number))
    } else {
        String::new()
    };
    let body = format!(
        r#"<h1 style="font-size:22px;margin:0 0 6px">Your order is on its way</h1><p style="color:#555;margin:0 0 18px">Order #{n}</p>{track}
<p style="margin:18px 0 0"><a href="{url}">View your order</a></p>"#,
        n = o.number,
        url = esc(&order_url(state, o)),
    );
    let text = format!("Your order #{} from {store} has shipped. {} {}\n", o.number, o.tracking_company, o.tracking_number);
    spawn_send(state, o.email.clone(), format!("Order #{} has shipped — {store}", o.number), wrap(&store, &body), text);
}

/// Start a Stripe Checkout for an order: `(session id, URL)`.
#[allow(clippy::too_many_arguments)]
pub async fn start_card_payment(
    cfg: &vuuut_stripe::StripeConfig,
    order: &repo::Order,
    label: &str,
    success_url: &str,
    cancel_url: &str,
) -> vuuut_core::Result<(String, String)> {
    vuuut_stripe::create_shop_checkout_session(cfg, &order.id, label, order.total_cents, &order.currency, &order.email, success_url, cancel_url).await
}

/// Whether the Checkout Session `session` paid `order` in full, asked of
/// Stripe. `Some(reference)` (the payment intent) when it did.
pub async fn card_payment_settled(cfg: &vuuut_stripe::StripeConfig, order: &repo::Order, session: &str) -> Option<String> {
    let v = vuuut_stripe::get_checkout_session(cfg, session).await.ok()?;
    let paid = v["payment_status"].as_str() == Some("paid");
    let ours = v["client_reference_id"].as_str() == Some(order.id.as_str());
    let amount = v["amount_total"].as_i64() == Some(order.total_cents);
    (paid && ours && amount).then(|| v["payment_intent"].as_str().unwrap_or(session).to_string())
}

/// Refund `amount` of a card-paid order through Stripe. The order's
/// `payment_ref` is the payment intent once paid.
pub async fn refund_card(state: &AppState, order: &repo::Order, amount: i64) -> vuuut_core::Result<()> {
    let Some(cfg) = stripe(state) else { return Err(vuuut_core::GhostError::BadRequest("Stripe is not connected.".into())) };
    if !order.payment_ref.starts_with("pi_") {
        return Err(vuuut_core::GhostError::BadRequest("This order has no Stripe payment to refund.".into()));
    }
    vuuut_stripe::refund_payment_intent(&cfg, &order.payment_ref, amount).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// digital downloads
// ---------------------------------------------------------------------------

/// Where product files live: inside the content directory but outside every
/// route that serves it (`/content/images|media|files/`), so the only way to
/// a file is the store's own download handler.
pub fn files_dir(state: &AppState) -> std::path::PathBuf {
    state.config.content_path().join("private").join("ecom")
}

/// A stored file's path. `stored` is always a row id; anything else is
/// refused, so no request can name a path.
pub fn stored_path(state: &AppState, stored: &str) -> Option<std::path::PathBuf> {
    vuuut_core::ids::is_objectid(stored).then(|| files_dir(state).join(stored))
}

/// The address buyers download from.
pub fn downloads_url(state: &AppState) -> String {
    vuuut_core::url::absolute(&state.site_url(), &vuuut_core::url::base(&paths(state).public_path("/shop/dashboard/")))
}

/// Everything that happens when an order with digital products is paid —
/// online, by staff marking it paid, or because it was free:
///
/// 1. each digital line gets a download grant (once);
/// 2. the buyer's email becomes a site member through the regular signup
///    path, and a new member gets the site's regular signup email;
/// 3. the buyer gets a "your downloads are ready" email whose button is a
///    magic link: it signs them in to that account and opens Downloads;
/// 4. an order that is only digital is marked fulfilled.
///
/// Returns whether the order had anything to download. Safe to call twice:
/// grants are unique per line and a second call only re-sends the email.
pub async fn on_paid(state: &AppState, order: &repo::Order) -> bool {
    let Some(order) = repo::find_order(&state.db, &order.id).await else { return false };
    if !rules::order_unlocks_downloads(&order.status, &order.financial_status) || order.email.is_empty() {
        return false;
    }
    // Courses sold through the store (Tools → LMS & Online Courses).
    crate::lms::on_order_paid(state, &order).await;
    let granted = repo::grant_downloads(&state.db, &order, None).await.unwrap_or(0);
    if granted == 0 {
        return false;
    }
    let name = order.address().name();
    let member = crate::members::ensure_member_for_purchase(state, &order.email, Some(&name)).await;
    if let Some((member_id, created)) = &member {
        let _ = repo::grant_downloads(&state.db, &order, Some(member_id)).await;
        if *created {
            repo::add_event(&state.db, &order.id, "member", &format!("{} became a member of the site.", order.email), "").await;
            if let Some(link) = crate::members::magic_link_for(state, &order.email, "signup", &paths(state).public_path("/shop/dashboard/")).await {
                let (bg, email) = (state.clone(), order.email.clone());
                tokio::spawn(async move {
                    if let Err(e) = crate::members::send_signup_email(&bg, &email, &link).await {
                        tracing::warn!("ecom: signup email to {email} failed: {}", e.message());
                    }
                });
            }
        }
    }
    send_download_email(state, &order).await;
    repo::add_event(&state.db, &order.id, "download", &format!("Downloads unlocked; the link was emailed to {}.", order.email), "").await;
    let items = repo::order_items(&state.db, &order.id).await;
    if order.fulfillment_status != "fulfilled" && items.iter().all(|i| i.requires_shipping == 0) {
        let _ = repo::fulfil(&state.db, &order.id, "", "", "").await;
        repo::add_event(&state.db, &order.id, "fulfilled", "Digital items delivered.", "").await;
    }
    true
}

/// "Your downloads are ready": the files, and a button that signs the buyer
/// in and opens Downloads. The button is a member magic link — single use,
/// 24 hours — and the email says where to get a fresh one.
pub async fn send_download_email(state: &AppState, order: &repo::Order) {
    let grants = repo::grants_for_order(&state.db, &order.id).await;
    if grants.is_empty() || order.email.is_empty() {
        return;
    }
    let Some(link) = crate::members::magic_link_for(state, &order.email, "signin", &paths(state).public_path("/shop/dashboard/")).await else { return };
    let s = settings(state);
    let title = state.settings.get_str("title").unwrap_or_default();
    let store = s.name(&title).to_string();
    let pids: Vec<String> = grants.iter().map(|g| g.product_id.clone()).collect();
    let files = repo::files_for(&state.db, &pids).await;
    let list: String = grants
        .iter()
        .map(|g| {
            let fs = files.get(&g.product_id).cloned().unwrap_or_default();
            format!(
                r#"<li style="margin:0 0 8px"><b>{t}</b><br><span style="color:#666;font-size:13px">{f}</span></li>"#,
                t = esc(&g.title),
                f = fs.iter().map(|f| format!("{} ({})", esc(&f.name), rules::format_bytes(f.size_bytes.max(0) as u64))).collect::<Vec<_>>().join(" · "),
            )
        })
        .collect();
    let page = downloads_url(state);
    let body = format!(
        r#"<h1 style="font-size:22px;margin:0 0 6px">Your downloads are ready</h1>
<p style="color:#555;margin:0 0 18px">Order #{n}. Your files are kept under “My products” in your account on {store}.</p>
<ul style="padding-left:18px;margin:0 0 20px">{list}</ul>
<p style="margin:0 0 18px"><a href="{link}" style="display:inline-block;background:#222;color:#fff;text-decoration:none;padding:12px 20px;border-radius:999px;font-weight:600">Download your files</a></p>
<p style="color:#666;font-size:13px;margin:0">The button signs you in to your account with {email} and opens My products, where your downloads are. It works once, within 24 hours. Afterwards, go to <a href="{page}">{page}</a> and ask for a new sign-in link.</p>"#,
        n = order.number,
        store = esc(&store),
        link = esc(&link),
        email = esc(&order.email),
        page = esc(&page),
    );
    let text = format!("Your downloads from {store} (order #{}) are ready.\n\nSign in and download: {link}\n\nThis link works once, within 24 hours. Afterwards visit {page} for a new one.\n", order.number);
    spawn_send(state, order.email.clone(), format!("Your downloads are ready — {store}"), wrap(&store, &body), text);
}

/// Revoke (or restore) an order's downloads, with a timeline line.
pub async fn set_downloads_revoked(state: &AppState, order: &repo::Order, revoked: bool, why: &str, actor: &str) {
    if repo::grants_for_order(&state.db, &order.id).await.is_empty() {
        return;
    }
    let _ = repo::set_order_downloads_revoked(&state.db, &order.id, revoked).await;
    repo::add_event(&state.db, &order.id, "download", &format!("Downloads {}{}.", if revoked { "revoked" } else { "restored" }, if why.is_empty() { String::new() } else { format!(" — {why}") }), actor).await;
}

/// Copy a remote product image (a Shopify CDN URL, say) into this site's
/// images, returning its new site path. Refuses anything that is not a
/// public http(s) address (the same guard webmentions use), follows no
/// redirects, gives up after 20 s, and stores only what the image upload
/// itself would accept. `None` keeps the original URL.
pub async fn copy_image(state: &AppState, url: &str) -> Option<String> {
    let parsed = crate::mentions::fetchable(url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let resp = client.get(parsed.clone()).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim().to_string();
    if !content_type.starts_with("image/") || resp.content_length().is_some_and(|n| n > 20 * 1024 * 1024) {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > 20 * 1024 * 1024 {
        return None;
    }
    let name = parsed.path_segments().and_then(|mut s| s.next_back()).unwrap_or("image").to_string();
    let stored = crate::images::store(state, vuuut_core::media::Kind::Image, Some(&name), &content_type, &bytes).await.ok()?;
    Some(vuuut_core::url::unbase_assets(&stored.url))
}
