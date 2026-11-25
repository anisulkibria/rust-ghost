//! Store settings: the switch, store details, payments, shipping rates,
//! taxes, checkout, notifications and policies. Administrator only.

use super::*;

const SETTINGS: &str = "/ghost/ecom/settings/";
static EDITOR_HASH: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| crate::asset_hash(crate::assets::ECOM_EDITOR_JS));

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="ec-setting"><span class="ic tint-{tint}">{ic}</span>
<div class="ec-setting-text"><b>{title}</b>{detail_html}</div>
<div class="ec-setting-control">{control}</div></div>"#,
        detail_html = if detail.is_empty() { String::new() } else { format!("<span>{detail}</span>") },
        ic = svg(icon),
        title = esc(title),
    )
}

fn delivery_countries(selected: &[String]) -> String {
    let mut choices = rules::COUNTRIES.to_vec();
    choices.extend(selected.iter().filter_map(|code| {
        vuuut_core::countries::country_name(code).is_none().then_some((code.as_str(), code.as_str()))
    }));
    choices.sort_by(|a, b| a.1.cmp(b.1));
    let options: String = choices.iter().map(|(code, name)| {
        let checked = if selected.iter().any(|saved| saved == code) { " checked" } else { "" };
        format!(
            r#"<label class="ec-country"><input type="checkbox" name="delivery_country" value="{code}"{checked}><span>{name}</span><small>{code}</small></label>"#,
            code = esc(code),
            name = esc(name),
        )
    }).collect();
    let summary = match selected.len() {
        0 => "All countries".to_string(),
        1 => "1 country selected".to_string(),
        count => format!("{count} countries selected"),
    };
    format!(
        r#"<div class="ec-set-fields ec-countries">
<input type="hidden" name="delivery_countries_present" value="1">
<details>
<summary>Delivery countries <span id="ec-country-summary">{summary}</span></summary>
<p class="muted ec-hint">Checkout offers the selected countries for physical orders. Leave all countries unchecked to deliver anywhere.</p>
<label class="field" id="ec-country-filter-field" hidden><input id="ec-country-filter" type="search" placeholder=" " autocomplete="off" aria-controls="ec-country-options"><span>Find a country</span></label>
<div class="ec-country-options" id="ec-country-options" role="group" aria-label="Delivery countries">{options}</div>
<p class="muted ec-hint" id="ec-country-no-results" hidden role="status">No matching countries.</p>
</details></div>"#,
    )
}

pub(crate) async fn ecom_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let rates = repo::shipping_rates(&state.db).await;
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let rate_rows: String = rates
        .iter()
        .map(|r| {
            format!(
                r#"<form class="ec-rate" method="post" action="{SETTINGS}shipping/{id}/">
<input class="ec-in" name="name" value="{n}" aria-label="Name" maxlength="120"><input class="ec-in" name="price" value="{p}" aria-label="Price" inputmode="decimal" placeholder="Free">
<input class="ec-in" name="min" value="{min}" aria-label="Minimum subtotal" inputmode="decimal" placeholder="Any"><input class="ec-in" name="max" value="{max}" aria-label="Maximum subtotal" inputmode="decimal" placeholder="Any">
<input class="ec-in" name="countries" value="{c}" aria-label="Countries" placeholder="All countries" maxlength="1000">
<button class="btn tonal small" type="submit">Save</button><button class="icon-btn" type="submit" formaction="{SETTINGS}shipping/{id}/delete/" title="Delete">{del}</button></form>"#,
                id = esc(&r.id),
                n = esc(&r.name),
                p = esc(&rules::money_input(r.price_cents, &s.currency)),
                min = esc(&rules::money_input(r.min_subtotal_cents, &s.currency)),
                max = esc(&rules::money_input(r.max_subtotal_cents, &s.currency)),
                c = esc(&r.countries),
                del = svg("delete"),
            )
        })
        .collect();
    let body = format!(
        r#"<form id="ec-settings" method="post" action="{SETTINGS}">
<div class="section">Store</div>
<div class="card padded ec-set rise" style="--i:2">
{open}
<div class="ec-grid2 ec-set-fields"><label class="field"><input name="ecom_store_name" value="{name}" placeholder=" " maxlength="80"><span>Store name (blank uses “{site}”)</span></label>
<p>Currency <a href="/ghost/settings/payments/#currency">{currency}</a></p></div>
<div class="ec-set-fields"><label class="field"><textarea name="ecom_intro" rows="2" placeholder=" " maxlength="400">{intro}</textarea><span>Welcome line on the store's home</span></label>
<div class="ec-grid2"><label class="field"><input name="ecom_per_page" type="number" min="1" step="1" value="{per}" placeholder="{default_per}"><span>Products per page</span></label>
<label class="field"><input name="ecom_low_stock" value="{low}" type="number" min="0" placeholder=" "><span>“Low stock” at or under</span></label></div></div>
</div>

{addr}
<div class="section" id="payments">Payments</div>
<div class="card rows"><a class="row" href="/ghost/settings/payments/#store"><span class="ic tint-purple">{payment_icon}</span><span class="body"><span class="title">Store payment methods</span><span class="detail">Manage them in Payments</span></span>{payment_chevron}</a></div>

<div class="section">Taxes</div>
<div class="card padded ec-set rise" style="--i:3">
<div class="ec-grid2 ec-set-fields"><label class="field"><input name="ecom_tax_rate" value="{rate}" placeholder=" " inputmode="decimal"><span>Tax rate (%)</span></label><div></div></div>
{incl}{taxship}
</div>

<div class="section">Checkout</div>
<div class="card padded ec-set rise" style="--i:4">
{countries}
{phone}
</div>

<div class="section">Notifications</div>
<div class="card padded ec-set rise" style="--i:4">
{receipts}
<div class="ec-set-fields"><label class="field"><input name="ecom_notify_email" value="{notify}" type="email" placeholder=" " maxlength="191"><span>Email new orders to (blank = nobody)</span></label></div>
</div>

<div class="section">Policies</div>
<div class="card padded ec-set rise" style="--i:5">
<label class="field"><textarea name="ecom_policy_shipping" rows="4" placeholder=" " maxlength="4000">{pship}</textarea><span>Shipping policy</span></label>
<label class="field"><textarea name="ecom_policy_refund" rows="4" placeholder=" " maxlength="4000">{prefund}</textarea><span>Refund policy</span></label>
<p class="muted ec-hint">Shown at {pol_ship} and {pol_refund}, and linked from product pages and checkout.</p>
</div>
<div class="ec-actions"><button class="btn" type="submit">{check} Save settings</button></div>
</form>

<div class="section" id="shipping">Shipping rates</div>
<div class="card padded rise" style="--i:5">
<p class="muted ec-hint">Checkout offers every rate whose countries and subtotal range fit the order. With no rates, shipping is free. A rate with a minimum subtotal is how you offer “free shipping over $50”.</p>
<div class="ec-rate ec-rate-head"><span>Name</span><span>Price</span><span>Min subtotal</span><span>Max subtotal</span><span>Countries</span><span></span><span></span></div>
{rate_rows}
<form class="ec-rate" method="post" action="{SETTINGS}shipping/">
<input class="ec-in" name="name" placeholder="Standard" aria-label="Name" maxlength="120" required><input class="ec-in" name="price" aria-label="Price" inputmode="decimal" placeholder="Free">
<input class="ec-in" name="min" aria-label="Minimum subtotal" inputmode="decimal" placeholder="Any"><input class="ec-in" name="max" aria-label="Maximum subtotal" inputmode="decimal" placeholder="Any">
<input class="ec-in" name="countries" aria-label="Countries" placeholder="All countries" maxlength="1000">
<button class="btn small ec-rate-add" type="submit">{add} Add rate</button></form>
</div>"#,
        open = row("store", if s.enabled { "green" } else { "slate" }, "Open the store", &if s.enabled { format!(r#"Live at <a href="{0}" target="_blank" rel="noopener">{1}</a>, inside your theme."#, vuuut_core::url::base(&vuuut_api::ecom::paths(&state).public_path("/shop/")), esc(&vuuut_api::ecom::paths(&state).public_path("/shop/"))) } else { format!("Off: {} is a 404.", esc(&vuuut_api::ecom::paths(&state).public_path("/shop/"))) }, &switch("ecom_enabled", s.enabled)),
        pol_ship = esc(&vuuut_api::ecom::paths(&state).public_path("/shop/policies/shipping/")),
        pol_refund = esc(&vuuut_api::ecom::paths(&state).public_path("/shop/policies/refund/")),
        addr = address_section(&state, &s),
        name = esc(&s.store_name),
        site = esc(&site_title),
        intro = esc(&s.intro),
        currency = esc(&s.currency),
        per = s.per_page,
        default_per = rules::DEFAULT_PER_PAGE,
        low = s.low_stock,
        payment_icon = svg("card"),
        payment_chevron = svg("chevron"),
        rate = esc(&if s.tax_rate_bp > 0 { rules::format_rate(s.tax_rate_bp).trim_end_matches('%').to_string() } else { String::new() }),
        incl = row("sell", "green", "Prices include tax", "On for VAT/GST-style pricing: the tax is the share already in the price.", &switch("ecom_tax_included", s.tax_included)),
        taxship = row("truck", "blue", "Charge tax on shipping", "", &switch("ecom_tax_shipping", s.tax_shipping)),
        countries = delivery_countries(&s.countries),
        phone = row("person", "teal", "Require a phone number", "For couriers that need one.", &switch("ecom_require_phone", s.require_phone)),
        receipts = row("mail", "pink", "Email buyers their receipt and shipping updates", "Sent through the site's mail settings.", &switch("ecom_send_receipts", s.send_receipts)),
        notify = esc(&s.notify_email),
        pship = esc(&s.policy_shipping),
        prefund = esc(&s.policy_refund),
        check = svg("check"),
        add = svg("add"),
    );
    ecom_page(&actor, "Store settings", "settings", &q, "", Some(BASE), &format!(r#"{body}<script src="{ASSETS}/ecom-editor.js?v={hash}" defer></script>"#, hash = &*EDITOR_HASH))
}

pub(crate) async fn ecom_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let form: HashMap<String, String> = pairs.iter().cloned().collect();
    let current = store_settings(&state);
    let per_page = match form.get("ecom_per_page").map(|value| value.trim()) {
        None => current.per_page,
        Some("") => rules::DEFAULT_PER_PAGE,
        Some(value) => match value.parse::<i64>() {
            Ok(count) if count > 0 => count,
            _ => return flash_to(SETTINGS, "error", "Enter a positive whole number for products per page."),
        },
    };
    let countries = if form.get("delivery_countries_present").is_some_and(|value| value == "1") {
        let selected: Vec<&str> = pairs.iter()
            .filter(|(key, _)| key == "delivery_country")
            .map(|(_, country)| country.as_str())
            .collect();
        if selected.iter().any(|code| {
            !code.bytes().all(|ch| ch.is_ascii_uppercase())
                || (vuuut_core::countries::country_name(code).is_none()
                    && !current.countries.iter().any(|saved| saved == code))
        }) {
            return flash_to(SETTINGS, "error", "Choose delivery countries from the list.");
        }
        // Keep the order of existing countries, including checkout's first choice.
        current.countries.iter().map(String::as_str)
            .filter(|code| selected.contains(code))
            .chain(selected.iter().copied())
            .collect::<Vec<_>>().join(", ")
    } else {
        // Older forms still send the comma-separated setting.
        form.get("ecom_countries").cloned().unwrap_or_else(|| current.countries.join(", "))
    };
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let rate = match form.get("ecom_tax_rate").map(|v| v.trim()).unwrap_or("") {
        "" => 0,
        v => match rules::parse_rate(v) {
            Some(r) => r,
            None => return flash_to(SETTINGS, "error", "The tax rate is a percentage from 0 to 50."),
        },
    };
    // The addresses are checked as typed, so a bad one is said, not dropped.
    let base_raw = form.get("ecom_base_path").cloned().unwrap_or_default();
    let product_raw = form.get("ecom_product_path").cloned().unwrap_or_default();
    let Some(base_path) = rules::clean_path(if base_raw.trim().is_empty() { rules::DEFAULT_BASE } else { &base_raw }) else {
        return flash_to(&format!("{SETTINGS}#address"), "error", "The store address can use letters, numbers and dashes, one to three parts, and not a path the site already uses (ghost, members, content, tag, author…).");
    };
    let product_path = match form.get("ecom_product_style").map(String::as_str) {
        Some("under") => format!("{base_path}/products"),
        Some("direct") => base_path.clone(),
        Some("custom") => match rules::clean_path(&product_raw) {
            Some(p) => p,
            None => return flash_to(&format!("{SETTINGS}#address"), "error", "The product address can use letters, numbers and dashes, one to three parts."),
        },
        // A form without the address section keeps what is set.
        _ => rules::clean_path(&product_raw).unwrap_or_else(|| current.product_path.clone()),
    };
    let base_path = if base_raw.trim().is_empty() && !form.contains_key("ecom_product_style") { current.base_path.clone() } else { base_path };
    if base_path != current.base_path || product_path != current.product_path {
        let admin = state.admin_slug();
        if [&base_path, &product_path].iter().any(|path| vuuut_core::admin_address::suffix(path, &admin).is_some()) {
            return flash_to(SETTINGS, "error", "Choose paths outside the admin address.");
        }
        let directory = vuuut_core::directory::Paths::read(|key| state.settings.get_str(key));
        if directory.overlaps_store(&base_path, &product_path) {
            return flash_to(SETTINGS, "error", "Choose paths outside the directory and its listing addresses.");
        }
    }
    let s = Settings::read(|key| match key {
        "ecom_enabled" | "ecom_tax_included" | "ecom_tax_shipping" | "ecom_send_receipts" | "ecom_require_phone" => Some(flag(key).to_string()),
        "ecom_pay_manual" | "ecom_pay_bank" | "ecom_pay_stripe" if form.contains_key("ecom_bank_details") => Some(flag(key).to_string()),
        "ecom_pay_manual" | "ecom_pay_bank" | "ecom_pay_stripe" | "ecom_bank_details" => {
            form.get(key).cloned().or_else(|| state.settings.get_str(key))
        }
        "ecom_tax_rate" => Some(rate.to_string()),
        "ecom_countries" => Some(countries.clone()),
        _ => form.get(key).cloned(),
    });
    if form.get("ecom_notify_email").is_some_and(|e| !e.trim().is_empty()) && s.notify_email.is_empty() {
        return flash_to(SETTINGS, "error", "The notification address is not a valid email.");
    }
    if s.pay_bank && s.bank_details.is_empty() {
        return flash_to(SETTINGS, "error", "Add your bank details, or turn bank transfer off.");
    }
    let writes = [
        ("ecom_enabled", serde_json::json!(s.enabled)),
        ("ecom_store_name", serde_json::json!(s.store_name)),
        ("ecom_intro", serde_json::json!(s.intro)),
        ("ecom_tax_rate", serde_json::json!(s.tax_rate_bp)),
        ("ecom_tax_included", serde_json::json!(s.tax_included)),
        ("ecom_tax_shipping", serde_json::json!(s.tax_shipping)),
        ("ecom_pay_manual", serde_json::json!(s.pay_manual)),
        ("ecom_pay_bank", serde_json::json!(s.pay_bank)),
        ("ecom_bank_details", serde_json::json!(s.bank_details)),
        ("ecom_pay_stripe", serde_json::json!(s.pay_stripe)),
        ("ecom_per_page", serde_json::json!(per_page)),
        ("ecom_low_stock", serde_json::json!(s.low_stock)),
        ("ecom_notify_email", serde_json::json!(s.notify_email)),
        ("ecom_send_receipts", serde_json::json!(s.send_receipts)),
        ("ecom_countries", serde_json::json!(s.countries.join(", "))),
        ("ecom_require_phone", serde_json::json!(s.require_phone)),
        ("ecom_policy_refund", serde_json::json!(s.policy_refund)),
        ("ecom_policy_shipping", serde_json::json!(s.policy_shipping)),
        ("ecom_base_path", serde_json::json!(base_path)),
        ("ecom_product_path", serde_json::json!(product_path)),
    ];
    for (key, value) in writes {
        // The Payments form owns these fields. Keep accepting older forms
        // that submitted some or all of them with the Ecommerce settings.
        if matches!(key, "ecom_pay_manual" | "ecom_pay_bank" | "ecom_pay_stripe" | "ecom_bank_details")
            && !form.contains_key("ecom_bank_details") && !form.contains_key(key)
        {
            continue;
        }
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("ecom: saving {key} failed: {}", e.message());
            return flash_to(SETTINGS, "error", "Saving failed — check the logs.");
        }
    }
    vuuut_db::content::touch();
    crate::tools::tool_saved(&state, SETTINGS, "Store settings saved.")
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct RateForm {
    name: String,
    price: String,
    min: String,
    max: String,
    countries: String,
}

async fn save_rate(state: &AppState, id: Option<&str>, f: &RateForm) -> Response {
    let s = store_settings(state);
    let name = rules::clean(&f.name, 120);
    if name.is_empty() {
        return flash_to(&format!("{SETTINGS}#shipping"), "error", "Give the rate a name buyers will see.");
    }
    let max = rules::money_or_zero(&f.max, &s.currency);
    let min = rules::money_or_zero(&f.min, &s.currency);
    if max > 0 && max < min {
        return flash_to(SETTINGS, "error", "The maximum subtotal has to be above the minimum.");
    }
    let countries = rules::parse_countries(&f.countries).join(", ");
    match repo::save_shipping_rate(&state.db, id, &name, rules::money_or_zero(&f.price, &s.currency), min, max, &countries).await {
        Ok(_) => flash_to(SETTINGS, "ok", "Shipping rate saved."),
        Err(e) => flash_to(SETTINGS, "error", &e.to_string()),
    }
}

pub(crate) async fn shipping_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<RateForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    save_rate(&state, None, &f).await
}

pub(crate) async fn shipping_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<RateForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !valid_id(&id) {
        return flash_to(SETTINGS, "error", "That rate no longer exists.");
    }
    save_rate(&state, Some(&id), &f).await
}

pub(crate) async fn shipping_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if valid_id(&id) {
        let _ = repo::delete_shipping_rate(&state.db, &id).await;
    }
    flash_to(SETTINGS, "ok", "Shipping rate deleted.")
}

/// "Store address": where the store lives, and how product addresses look —
/// under the store (`/shop/products/hat/`), directly in it
/// (`/themes/hat/`), or at a path of their own.
fn address_section(state: &AppState, s: &Settings) -> String {
    let site = state.site_url();
    let site = site.trim_end_matches('/');
    let style = if s.product_path == s.base_path {
        "direct"
    } else if s.product_path == format!("{}/products", s.base_path) {
        "under"
    } else {
        "custom"
    };
    let radio = |key: &str, title: &str, example: &str| {
        format!(
            r#"<label class="ec-kind ec-url-kind"><input type="radio" name="ecom_product_style" value="{key}"{c}><b>{title}</b><span class="ec-url-example" data-example="{key}">{example}</span></label>"#,
            c = if style == key { " checked" } else { "" },
        )
    };
    format!(
        r##"<div class="section" id="address">Store address</div>
<div class="card padded ec-set rise ec-address" style="--i:3" data-site="{site_e}">
<label class="ec-urlfield"><span class="ec-urlfield-label">Store address</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site_e}/</span><input name="ecom_base_path" id="ec-base-path" value="{base}" maxlength="80" required spellcheck="false" autocomplete="off"><span class="ec-urlfield-site">/</span></span></label>
<p class="muted ec-hint">The store's home page. Its cart, checkout, collections and downloads live under it — for example <code id="ec-base-example">{site_e}/{base}/cart/</code></p>
<div class="ec-sublabel">Product addresses</div>
<div class="ec-kinds ec-kinds-3">{under}{direct}{custom}</div>
<label class="ec-urlfield ec-only-custom"><span class="ec-urlfield-label">Product path</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site_e}/</span><input name="ecom_product_path" id="ec-product-path" value="{product}" maxlength="80" spellcheck="false" autocomplete="off"><span class="ec-urlfield-site">/product-name/</span></span></label>
<p class="muted ec-hint">Old addresses keep working: they redirect (301) to the new ones, so shared links and search results are not lost. With products directly in the store, a product cannot be called cart, checkout, collections, orders, account, downloads, dashboard, policies, assets or products.</p>
</div>"##,
        site_e = esc(site),
        base = esc(&s.base_path),
        product = esc(&s.product_path),
        under = radio("under", "Under “products”", &format!("{}/{}/products/product-name/", esc(site), esc(&s.base_path))),
        direct = radio("direct", "Directly in the store", &format!("{}/{}/product-name/", esc(site), esc(&s.base_path))),
        custom = radio("custom", "A path of their own", &format!("{}/{}/product-name/", esc(site), esc(&s.product_path))),
    )
}
