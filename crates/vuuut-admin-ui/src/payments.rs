//! Payment provider settings and checkout methods for the storefront.
use super::*;

const BASE: &str = "/ghost/settings/payments/";

pub(crate) async fn legacy(uri: axum::http::Uri) -> Redirect {
    let path = uri.path();
    let suffix = path.strip_prefix("/payments/")
        .or_else(|| path.strip_prefix("/memberships/settings/"))
        .unwrap_or_default();
    let query = uri.query().map(|query| format!("?{query}")).unwrap_or_default();
    Redirect::permanent(&format!("{BASE}{suffix}{query}"))
}

async fn guard_write(state: &AppState, headers: &HeaderMap) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn stripe_flash(kind: &str, message: &str) -> Response {
    crate::ecom::flash_to("/ghost/settings/payments/#stripe", kind, message)
}

fn stripe_card(state: &AppState) -> String {
    // Stripe connection card. Ghost(Pro) does this over Stripe Connect OAuth;
    // a self-hosted site pastes its own keys, which is what this offers.
    match vuuut_stripe::StripeConfig::from_settings(&state.settings) {
        Some(cfg) => {
            let who = state.settings.get_str("stripe_connect_display_name").unwrap_or_default();
            let account = state.settings.get_str("stripe_connect_account_id").unwrap_or_default();
            let live = cfg.is_live();
            let hook = state.settings.get_str("members_stripe_webhook_secret").unwrap_or_default();
            format!(
                r#"<div class="section" id="stripe">Stripe</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-green">{check_ic}</span>
<div class="body"><span class="title">Connected{named}</span><span class="detail">{account} · {mode} mode</span></div>
<div class="trailing"><span class="chip {chip}">{mode}</span>
<form method="post" action="/ghost/settings/payments/stripe/disconnect/" data-confirm="Disconnect from Stripe|New paid signups will stop. Existing subscriptions stay active on Stripe and resume if you reconnect the same account.">
<button class="btn tonal small danger" type="submit">Disconnect from Stripe</button></form></div></div>
{checkout_notice}
{hook_row}
</div>"#,
                checkout_notice = {
                    let error = state.settings.get_str("stripe_checkout_setup_error").unwrap_or_default();
                    if error.is_empty() { String::new() } else {
                        format!(r#"<div class="row"><span class="ic tint-amber">{}</span><div class="body"><span class="title">Checkout setup required</span><span class="detail">{}</span><a href="https://dashboard.stripe.com/settings/business-details" target="_blank" rel="noopener">Open Stripe business details</a></div></div>"#, svg("flag"), esc(&error))
                    }
                },
                check_ic = svg("check"),
                named = if who.is_empty() { String::new() } else { format!(" · {}", esc(&who)) },
                account = esc(&account),
                mode = if live { "live" } else { "test" },
                chip = if live { "published" } else { "draft" },
                hook_row = if !hook.trim().is_empty() {
                    // Active: deactivating comes first, the way disconnecting
                    // precedes connecting above. Setting a replacement is
                    // still possible directly, for rotating a secret.
                    format!(
                        r#"<div class="row"><span class="ic tint-green">{link_ic}</span>
<div class="body"><span class="title">Webhooks active</span><span class="detail">Subscription changes update members automatically.</span></div>
<div class="trailing"><span class="chip published">verified</span>
<form method="post" action="/ghost/settings/payments/stripe/webhook/deactivate/" data-confirm="Deactivate webhook|Subscription changes will stop being accepted until a new signing secret is set. Payments still go through at Stripe.">
<button class="btn tonal small danger" type="submit">Deactivate webhook</button></form></div></div>"#,
                        link_ic = svg("link"),
                    )
                } else {
                    format!(
                        r#"<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{flag}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:4px">Webhooks not active</span>
<p class="muted" style="margin:0 0 10px">Without a signing secret every incoming webhook is refused, so subscriptions will never update. Stripe could not register one automatically — that needs a publicly reachable site.</p>
{secret_form}
</div></div>"#,
                        flag = svg("flag"),
                        secret_form = stripe_webhook_secret_form(&state.site_url()),
                    )
                },
            )
        }
        None => format!(
            r#"<div class="section" id="stripe">Stripe</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{coin}</span>
<div class="body" style="width:100%">{key_form}</div></div>
</div>"#,
            coin = svg("chart"),
            key_form = stripe_key_form(
                "Connect Stripe",
                "Enter your Stripe secret and publishable keys to accept card payments and offer paid tiers. Use test keys until you are ready to accept payments.",
                "Save API keys",
            ),
        ),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct StripeConnectForm {
    secret_key: String,
    publishable_key: String,
}

/// Start Connect: mint a state onto this session and send the admin to Stripe.
///
/// The URL, the nonce and its window all live in [`vuuut_api::stripe_connect`],
/// shared with the Admin API's `GET /members/stripe_connect`. This handler is
/// only the difference between the two: a form post that reports failure as a
/// flash message on the Payments screen.
pub(crate) async fn stripe_oauth_start(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    if vuuut_api::stripe_connect::can_connect(&state.config.env, &state.site_url()).is_err() {
        return stripe_flash("error", "Stripe Connect requires HTTPS in production.");
    }
    let Some(sid) = session_id_from(&headers) else { return stripe_flash("error", "Sign in again before connecting Stripe."); };
    let mode = vuuut_api::stripe_connect::mode_from(form.get("mode").map(String::as_str)).unwrap_or("live");
    match vuuut_api::stripe_connect::mint_state(&state.db, &sid, mode).await {
        Some(minted) => Redirect::to(&vuuut_api::stripe_connect::oauth_url(mode, &minted)).into_response(),
        None => stripe_flash("error", "Could not start the Stripe connection. Sign in again and retry."),
    }
}

#[derive(Deserialize)]
pub(crate) struct StripeSecureKeyForm { secure_key: String }

pub(crate) async fn stripe_oauth_finish(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<StripeSecureKeyForm>) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    let Some(sid) = session_id_from(&headers) else { return stripe_flash("error", "Sign in again before connecting Stripe."); };
    let Some(expected) = vuuut_api::stripe_connect::pending_state(&state.db, &sid).await else {
        return stripe_flash("error", "Start Connect with Stripe before pasting its secure key, and within thirty minutes of doing so.");
    };
    let key = match vuuut_api::stripe_connect::decode_secure_key(&form.secure_key, &expected) {
        Ok(key) => key,
        Err(vuuut_api::stripe_connect::INCORRECT_STATE) => {
            return stripe_flash("error", "The Stripe connection belongs to another session. Start the connection again.");
        }
        Err(message) => return stripe_flash("error", message),
    };
    // Spend the nonce before adopting the keys, not after: a blob that got as
    // far as a Stripe call must not stay replayable because that call failed.
    if !vuuut_api::stripe_connect::clear_state(&state.db, &sid).await {
        return stripe_flash("error", "Could not complete the Stripe connection. Sign in again and retry.");
    }
    stripe_connect(State(state), headers, Form(StripeConnectForm {secret_key: key.secret_key, publishable_key: key.publishable_key})).await
}

/// Store Stripe keys after proving they work.
///
/// Ghost(Pro) uses Stripe Connect's OAuth; a self-hosted site pastes its own
/// keys, and both arrive here. The work — verifying the key against
/// `/v1/account`, clearing price ids that belonged to another account,
/// registering the webhook — is [`vuuut_api::stripe_connect::adopt_keys`],
/// shared with the Admin API's Connect token so the two cannot drift. What is
/// left in this handler is how the Payments screen says what happened.
pub(crate) async fn stripe_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<StripeConnectForm>,
) -> Response {
    if let Err(r) = guard_write(&state, &headers).await {
        return r;
    }
    match vuuut_api::stripe_connect::adopt_keys(&state, &form.secret_key, &form.publishable_key).await {
        Ok(adopted) => {
            let mode = if adopted.livemode { "live" } else { "test" };
            let notes = adopted.notes.iter().map(|note| format!(" {note}")).collect::<String>();
            stripe_flash(
                "ok",
                &format!("Connected to Stripe account “{}” in {mode} mode.{notes}", adopted.account),
            )
        }
        Err(message) => stripe_flash("error", &message),
    }
}

/// The webhook signing-secret form, shown both when nothing is set and when
/// rotating an existing secret.
pub(crate) fn stripe_webhook_secret_form(site_url: &str) -> String {
    format!(
        r#"<form method="post" action="/ghost/settings/payments/stripe/webhook/" class="inline-form">
<label class="field" style="flex:1;min-width:240px;margin:0"><input name="webhook_secret" type="password" placeholder=" " autocomplete="off" required><span>Webhook signing secret · whsec_…</span></label>
<button class="btn tonal small" type="submit">Save secret</button></form>
<p class="muted" style="margin:10px 0 0">Testing locally? Run this and paste the <code>whsec_…</code> it prints:</p>
<p class="muted" style="margin:6px 0 0"><code style="display:block;white-space:pre-wrap;word-break:break-all">stripe listen --events {events} --forward-to {url}/members/webhooks/stripe/</code></p>
<p class="muted" style="margin:6px 0 0">Recent Stripe CLI versions require <code>--events</code> (or <code>--all-snapshot</code>).</p>"#,
        events = vuuut_stripe::WEBHOOK_EVENTS.join(","),
        url = esc(site_url),
    )
}

/// Direct API key setup, shown when no account is connected.
/// Hosted onboarding stays hidden until we provide our own Stripe Connect flow.
pub(crate) fn stripe_key_form(heading: &str, blurb: &str, button: &str) -> String {
    format!(
        r#"<span class="title" style="display:block;margin-bottom:6px">{heading}</span>
<p class="muted" style="margin:0 0 12px">{blurb}</p>
<form method="post" action="/ghost/settings/payments/stripe/connect/">
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="secret_key" type="password" placeholder=" " autocomplete="off" required><span>Secret key · sk_live_… or sk_test_…</span></label>
<label class="field" style="margin:0"><input name="publishable_key" placeholder=" " autocomplete="off"><span>Publishable key · pk_…</span></label>
</div>
<button class="btn tonal" type="submit" style="margin-top:12px">{coin} {button}</button>
</form>
<p class="muted" style="margin:12px 0 0">The key is checked against Stripe before it is stored, and is never shown again afterwards. Stripe Checkout still requires the account business details to be complete.</p>"#,
        heading = heading,
        blurb = blurb,
        button = button,
        coin = svg("chart"),
    )
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct WebhookSecretForm {
    webhook_secret: String,
}

/// Set the webhook signing secret by hand.
///
/// Auto-registration only works when Stripe can reach the site, so a local
/// install never gets a secret — and without one every webhook is refused,
/// which makes paid membership impossible to exercise. `stripe listen` prints
/// a `whsec_…` for exactly this case.
pub(crate) async fn stripe_set_webhook_secret(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<WebhookSecretForm>,
) -> Response {
    if let Err(r) = guard_write(&state, &headers).await {
        return r;
    }
    let secret = form.webhook_secret.trim().to_string();
    if !secret.is_empty() && !secret.starts_with("whsec_") {
        return stripe_flash("error", "A webhook signing secret starts with whsec_.");
    }
    match state.settings.set(&state.db, "members_stripe_webhook_secret", &serde_json::json!(secret.clone())).await {
        Ok(()) if secret.is_empty() => stripe_flash("ok", "Webhook secret cleared. Webhooks will be refused until one is set."),
        Ok(()) => stripe_flash("ok", "Webhook signing secret saved."),
        Err(e) => stripe_flash("error", &e.message()),
    }
}

/// Turn the webhook off: drop the signing secret, and remove the endpoint at
/// Stripe when we registered one.
///
/// Clearing the secret is what actually deactivates it — every incoming
/// webhook is refused without one — so this is also the safe way to retire a
/// secret that has leaked, without touching the Stripe connection itself.
pub(crate) async fn stripe_deactivate_webhook(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard_write(&state, &headers).await {
        return r;
    }
    let mut note = String::new();
    if let Some(cfg) = vuuut_stripe::StripeConfig::from_settings(&state.settings) {
        if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|i| !i.is_empty()) {
            if let Err(e) = vuuut_stripe::delete_webhook(&cfg, &id).await {
                // Local-only secrets (from `stripe listen`) have no endpoint
                // to delete, and a deleted one is already gone — neither is
                // worth failing over, but say so rather than implying the
                // Stripe side was tidied up.
                note = format!(" The endpoint at Stripe could not be removed ({}) — delete it in the dashboard if it still exists.", e.message());
            }
        }
    }
    for k in ["members_stripe_webhook_id", "members_stripe_webhook_secret"] {
        let _ = state.settings.set(&state.db, k, &serde_json::json!("")).await;
    }
    stripe_flash("ok", &format!("Webhook deactivated. Subscription changes will not be accepted until a new secret is set.{note}"))
}

/// Disconnect. The tiers and their Stripe ids are left alone: reconnecting
/// the same account should not mean rebuilding every price, and deleting
/// local price ids would orphan live subscriptions.
pub(crate) async fn stripe_disconnect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard_write(&state, &headers).await {
        return r;
    }
    // Upstream's guard, and this screen needs it as much as the API does: the
    // keys are what reconciles a subscription, so clearing them while members
    // still hold one leaves them paying Stripe for access this site can no
    // longer confirm, with nothing in the admin able to show it.
    if vuuut_api::stripe_connect::has_paid_members(&state.db).await {
        return stripe_flash("error", vuuut_api::stripe_connect::HAS_SUBSCRIPTIONS);
    }
    // Remove the registered webhook first, while the key still works.
    if let Some(cfg) = vuuut_stripe::StripeConfig::from_settings(&state.settings) {
        if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|i| !i.is_empty()) {
            let _ = vuuut_stripe::delete_webhook(&cfg, &id).await;
        }
    }
    for k in [
        "stripe_secret_key", "stripe_publishable_key", "stripe_connect_account_id",
        "stripe_connect_display_name", "stripe_connect_livemode",
        "members_stripe_webhook_id", "members_stripe_webhook_secret",
    ] {
        let _ = state.settings.set(&state.db, k, &serde_json::json!("")).await;
    }
    let _ = state.settings.set(&state.db, "paid_members_enabled", &serde_json::json!(false)).await;
    stripe_flash("ok", "Disconnected from Stripe. Existing subscriptions are untouched on Stripe's side.")
}


use vuuut_api::payments as providers;
use vuuut_core::ecom::PaymentMethod;
use vuuut_db::repos::{ecom as store, payments as repo};

pub(crate) async fn page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };

    let methods = vuuut_api::ecom::settings(&state);
    let requested = query.get("product").map(String::as_str).unwrap_or("");
    let products = repo::digital_products(&state.db).await.unwrap_or_default();
    let options: String = products.iter().map(|(id, title)| {
        format!(r#"<option value="{}"{}>{}</option>"#,
            esc(id), if id == requested { " selected" } else { "" }, esc(title))
    }).collect();
    let mapping = if products.iter().any(|(id, _)| id == requested) {
        let mapping = repo::mapping(&state.db, requested).await.unwrap_or_default();
        format!(r#"<form method="post" action="{BASE}products/">
<input type="hidden" name="product" value="{product}">
<div class="ec-grid2">
<label class="field"><input name="polar_product" value="{polar}" placeholder=" " maxlength="36"><span>Polar product ID</span></label>
<label class="field"><input name="gumroad_product" value="{gumroad}" placeholder=" " maxlength="128"><span>Gumroad product ID</span></label>
</div><button class="btn" type="submit">Save product links</button></form>"#,
            product = esc(requested), polar = esc(&mapping.polar_product), gumroad = esc(&mapping.gumroad_product))
    } else {
        String::new()
    };

    let polar_status = provider_status(&state, PaymentMethod::Polar);
    let gumroad_status = provider_status(&state, PaymentMethod::Gumroad);
    let webhook = |name: &str| esc(&format!("{}/members/webhooks/{name}/", state.site_url()));
    let body = format!(r#"{head}{banner}{stripe}
<div class="section" id="polar">Polar</div>
<form class="card padded" method="post" action="{BASE}polar/">
<p>{polar_status}</p>
<p class="muted">Use an organization access token with checkout write access. Subscribe to <code>order.paid</code> and paste the endpoint’s signing secret.</p>
<p><code>{polar_webhook}</code></p>
<div class="ec-grid2">{polar_key}{polar_secret}</div>
{sandbox}{polar_enabled}
<button class="btn" type="submit">Save Polar settings</button>
</form>
<div class="section" id="gumroad">Gumroad</div>
<form class="card padded" method="post" action="{BASE}gumroad/">
<p>{gumroad_status}</p>
<p class="muted">Use an access token with product and sales read access. Set your Gumroad Ping URL to the address below. Add a required text field named <b>Vuuut order</b> to each linked Gumroad product.</p>
<p><code>{gumroad_webhook}</code></p>
{gumroad_key}{gumroad_enabled}
<button class="btn" type="submit">Save Gumroad settings</button>
</form>
<div class="section" id="products">Provider product links</div>
<div class="card padded">
<p class="muted">Polar and Gumroad appear for orders with one linked digital item and no store-applied tax or discount. Use one fixed, one-time provider price in the same currency as the Vuuut product. Leave an ID blank to remove its link.</p>
<form method="get" action="{BASE}#products">
<label class="field fixed"><select name="product" required><option value="">Choose a digital product</option>{options}</select><span>Vuuut product</span>{caret}</label>
<button class="btn tonal" type="submit">Open product links</button>
</form>{mapping}
</div>
<div class="section" id="store">Store payment methods</div>
<form class="card padded" method="post" action="{BASE}store/">
{card}{cod}{transfer}
<label class="field"><textarea name="bank_details" rows="3" maxlength="600" placeholder=" ">{bank_details}</textarea><span>Bank transfer instructions</span></label>
<button class="btn" type="submit">Save store payment methods</button>
</form>
{tips}"#,
        head = group_head(SETTINGS_GROUPS.iter().find(|group| group.slug == "payments").expect("Payments settings group"), None),
        banner = flash_banner(&query),
        stripe = stripe_card(&state),
        polar_webhook = webhook("polar"), gumroad_webhook = webhook("gumroad"),
        polar_key = secret_field(&state, providers::POLAR_KEY, "Organization access token"),
        polar_secret = secret_field(&state, providers::POLAR_SECRET, "Webhook signing secret"),
        gumroad_key = secret_field(&state, providers::GUMROAD_KEY, "Access token"),
        sandbox = checkbox("sandbox", "Use Polar sandbox", state.settings.get_bool(providers::POLAR_SANDBOX)),
        polar_enabled = checkbox("enabled", "Offer Polar at checkout", state.settings.get_bool(providers::POLAR_ENABLED)),
        gumroad_enabled = checkbox("enabled", "Offer Gumroad at checkout", state.settings.get_bool(providers::GUMROAD_ENABLED)),
        caret = SELECT_CARET,
        card = checkbox("stripe", "Card payments through Stripe", methods.pay_stripe),
        cod = checkbox("manual", "Cash on delivery", methods.pay_manual),
        transfer = checkbox("bank", "Bank transfer", methods.pay_bank),
        bank_details = esc(&methods.bank_details),
        tips = tips_card(&state),
    );
    page_for_role(actor.role, "Payments", "settings", Some(&actor.user), &body, true).into_response()
}

fn tips_card(state: &AppState) -> String {
    let currency = state.settings.get_str("donations_currency").unwrap_or_else(|| "USD".into());
    let cents = state.settings.get_str("donations_suggested_amount")
        .and_then(|value| value.parse::<i64>().ok()).unwrap_or_default();
    let link = esc(&format!("{}/#/portal/support", state.site_url()));
    format!(r#"<div class="section" id="tips">Tips &amp; donations</div>
<form class="card padded" method="post" action="{BASE}tips/">
<p class="muted">Tips and donations use Stripe.</p>
<div class="ec-grid2">
<label class="field fixed"><select name="currency">{currencies}</select><span>Currency</span>{SELECT_CARET}</label>
<label class="field"><input name="amount" type="number" min="0" max="10000" step="1" value="{amount}" placeholder=" "><span>Suggested amount</span></label>
</div>
<button class="btn" type="submit">Save tip settings</button>
<a class="btn tonal" href="{link}" target="_blank" rel="noopener">Open tips page</a>
</form>"#, currencies = select_options(CURRENCIES, &currency), amount = cents / 100)
}

pub(crate) async fn save_tips(
    State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    let currency = form.get("currency").map(String::as_str).unwrap_or("");
    let amount = form.get("amount").map(|value| value.trim()).unwrap_or("0");
    let amount = if amount.is_empty() { Some(0) } else { amount.parse::<i64>().ok() };
    let Some(amount) = amount.filter(|value| (0..=10_000).contains(value)) else {
        return crate::ecom::flash_to("/ghost/settings/payments/#tips", "error", "Enter a suggested amount from 0 to 10,000.");
    };
    if !CURRENCIES.iter().any(|(code, _)| *code == currency) {
        return crate::ecom::flash_to("/ghost/settings/payments/#tips", "error", "Choose a currency.");
    }
    write_settings(&state, "tips", &[
        ("donations_currency", serde_json::json!(currency)),
        ("donations_suggested_amount", serde_json::json!(amount * 100)),
    ]).await
}

fn provider_status(state: &AppState, method: PaymentMethod) -> &'static str {
    if providers::enabled(state, method) { "Enabled for eligible store orders" }
    else if providers::configured(state, method) { "Configured" }
    else { "Setup required" }
}

fn checkbox(name: &str, label: &str, checked: bool) -> String {
    let checked = if checked { " checked" } else { "" };
    format!(r#"<p><label><input name="{name}" type="checkbox" value="1"{checked}> {label}</label></p>"#)
}

fn secret_field(state: &AppState, key: &str, label: &str) -> String {
    let hint = if state.settings.get_str(key).is_some_and(|value| !value.is_empty()) {
        "Saved. Enter a value to replace it."
    } else {
        " "
    };
    format!(r#"<label class="field fixed"><input name="{key}" type="password" placeholder="{hint}" maxlength="512" autocomplete="new-password"><span>{label}</span></label>"#)
}

fn saved_secret(state: &AppState, form: &HashMap<String, String>, name: &str) -> Result<String, &'static str> {
    let candidate = form.get(name).map(|value| value.trim()).unwrap_or_default();
    if candidate.len() > 512 || candidate.bytes().any(|byte| !byte.is_ascii_graphic()) {
        return Err("Enter a valid token or signing secret.");
    }
    Ok(if candidate.is_empty() { state.settings.get_str(name).unwrap_or_default() } else { candidate.into() })
}

async fn write_settings(state: &AppState, section: &str, writes: &[(&str, serde_json::Value)]) -> Response {
    let path = format!("{BASE}#{section}");
    if let Err(error) = state.settings.set_many(&state.db, writes).await {
        tracing::error!("saving {section} payment settings: {error}");
        return crate::ecom::flash_to(&path, "error", "Payment settings could not be saved.");
    }
    crate::ecom::flash_to(&path, "ok", "Payment settings saved.")
}

pub(crate) async fn save_polar(
    State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    let error = |message| crate::ecom::flash_to("/ghost/settings/payments/#polar", "error", message);
    let key = match saved_secret(&state, &form, providers::POLAR_KEY) { Ok(value) => value, Err(message) => return error(message) };
    let secret = match saved_secret(&state, &form, providers::POLAR_SECRET) { Ok(value) => value, Err(message) => return error(message) };
    let enabled = form.contains_key("enabled");
    if enabled && (key.is_empty() || secret.is_empty()) {
        return error("Save an access token and a webhook signing secret before enabling Polar.");
    }
    write_settings(&state, "polar", &[
        (providers::POLAR_KEY, serde_json::json!(key)),
        (providers::POLAR_SECRET, serde_json::json!(secret)),
        (providers::POLAR_SANDBOX, serde_json::json!(form.contains_key("sandbox"))),
        (providers::POLAR_ENABLED, serde_json::json!(enabled)),
    ]).await
}

pub(crate) async fn save_gumroad(
    State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    let error = |message| crate::ecom::flash_to("/ghost/settings/payments/#gumroad", "error", message);
    let key = match saved_secret(&state, &form, providers::GUMROAD_KEY) { Ok(value) => value, Err(message) => return error(message) };
    let enabled = form.contains_key("enabled");
    if enabled && key.is_empty() {
        return error("Save an access token before enabling Gumroad.");
    }
    write_settings(&state, "gumroad", &[
        (providers::GUMROAD_KEY, serde_json::json!(key)),
        (providers::GUMROAD_ENABLED, serde_json::json!(enabled)),
    ]).await
}

pub(crate) async fn save_store(
    State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    let details = vuuut_core::ecom::clean_multiline(form.get("bank_details").map(String::as_str).unwrap_or(""), 600);
    if form.contains_key("bank") && details.is_empty() {
        return crate::ecom::flash_to("/ghost/settings/payments/#store", "error", "Enter the bank transfer instructions.");
    }
    write_settings(&state, "store", &[
        ("ecom_pay_stripe", serde_json::json!(form.contains_key("stripe"))),
        ("ecom_pay_manual", serde_json::json!(form.contains_key("manual"))),
        ("ecom_pay_bank", serde_json::json!(form.contains_key("bank"))),
        ("ecom_bank_details", serde_json::json!(details)),
    ]).await
}

#[derive(Deserialize)]
pub(crate) struct ProductForm {
    product: String,
    polar_product: String,
    gumroad_product: String,
}

pub(crate) async fn save_product(
    State(state): State<AppState>, headers: HeaderMap, Form(form): Form<ProductForm>,
) -> Response {
    if let Err(response) = guard_write(&state, &headers).await { return response; }
    if !vuuut_core::ids::is_objectid(&form.product)
        || !store::find_product(&state.db, &form.product).await.is_ok_and(|row| row.is_some_and(|product| product.requires_shipping == 0))
    {
        return crate::ecom::flash_to("/ghost/settings/payments/#products", "error", "Choose a digital product.");
    }
    let path = format!("{BASE}?product={}#products", form.product);
    let mapping = repo::ProductMapping {
        polar_product: form.polar_product.trim().to_owned(),
        gumroad_product: form.gumroad_product.trim().to_owned(),
    };
    if (!mapping.polar_product.is_empty() && !providers::polar_id(&mapping.polar_product))
        || (!mapping.gumroad_product.is_empty() && !providers::gumroad_id(&mapping.gumroad_product))
    {
        return crate::ecom::flash_to(&path, "error", "Enter valid product IDs from your providers.");
    }
    match repo::save_mapping(&state.db, &form.product, &mapping).await {
        Ok(()) => crate::ecom::flash_to(&path, "ok", "Product links saved."),
        Err(_) => crate::ecom::flash_to(&path, "error", "Product links could not be saved."),
    }
}
