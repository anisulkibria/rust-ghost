//! Settings -> Membership: tiers, Stripe, the signup portal and member custom fields.

use super::*;

// --- membership: access, tiers, signup portal, welcome emails, newsletters ---

/// Ghost's price display: cents with whole-dollar collapsing ("$5").
pub(crate) fn tier_price(cents: Option<i64>) -> String {
    match cents {
        None => "—".into(),
        Some(c) if c % 100 == 0 => format!("${}", c / 100),
        Some(c) => format!("${:.2}", c as f64 / 100.0),
    }
}

// --- membership tiers ---

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct TierForm {
    name: String,
    description: String,
    /// Entered in whole currency units; Stripe works in the smallest unit.
    monthly_price: String,
    yearly_price: String,
    currency: String,
    welcome_page_url: String,
    trial_days: String,
    visibility: String,
    active: Option<String>,
    /// One per line.
    benefits: String,
}

pub(crate) fn tier_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/membership/tiers/{id}/?{encoded}")).into_response()
}

/// Parse a price typed as "5", "5.00" or "£5" into cents. Returns `None` for
/// an empty field, which means "this interval is not offered".
pub(crate) fn parse_price_cents(raw: &str) -> Option<i64> {
    let cleaned: String = raw.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().map(|v| (v * 100.0).round() as i64).filter(|c| *c > 0)
}

pub(crate) fn format_price_input(cents: Option<i64>) -> String {
    match cents {
        Some(c) if c > 0 => format!("{:.2}", c as f64 / 100.0),
        _ => String::new(),
    }
}

pub(crate) async fn tier_benefits(state: &AppState, tier_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT b.name FROM benefits b JOIN products_benefits pb ON pb.benefit_id = b.id \
         WHERE pb.product_id = ? ORDER BY pb.sort_order",
    )
    .bind(tier_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}

/// Replace a tier's benefits with the lines given, creating any that are new.
pub(crate) async fn set_tier_benefits(state: &AppState, tier_id: &str, lines: &str) {
    let _ = sqlx::query("DELETE FROM products_benefits WHERE product_id = ?")
        .bind(tier_id)
        .execute(&state.db)
        .await;
    let now = ghost_core::dates::now_db();
    for (i, raw) in lines.lines().enumerate() {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM benefits WHERE name = ?")
            .bind(name)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
        let bid = match existing {
            Some(b) => b,
            None => {
                let bid = ghost_core::ids::new_id();
                // benefits.slug is unique, so suffix with the id to avoid a
                // collision between two similarly-named benefits
                let slug = format!("{}-{}", ghost_core::slug::safe_string(name), &bid[..6]);
                if sqlx::query("INSERT INTO benefits (id, name, slug, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
                    .bind(&bid).bind(name).bind(&slug).bind(&now).bind(&now)
                    .execute(&state.db).await.is_err()
                {
                    continue;
                }
                bid
            }
        };
        let _ = sqlx::query("INSERT INTO products_benefits (id, product_id, benefit_id, sort_order) VALUES (?, ?, ?, ?)")
            .bind(ghost_core::ids::new_id()).bind(tier_id).bind(&bid).bind(i as i64)
            .execute(&state.db).await;
    }
}

pub(crate) async fn tier_create(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let id = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
    let slug = format!("tier-{}", &id[..8]);
    let res = sqlx::query(
        "INSERT INTO products (id, name, slug, active, visibility, type, currency, created_at, updated_at) \
         VALUES (?, 'New tier', ?, 0, 'public', 'paid', 'usd', ?, ?)",
    )
    .bind(&id).bind(&slug).bind(&now).bind(&now)
    .execute(&state.db).await;
    match res {
        Ok(_) => Redirect::to(&format!("/ghost/settings/membership/tiers/{id}/")).into_response(),
        Err(e) => membership_flash("error", &format!("Could not create the tier: {e}")),
    }
}

pub(crate) type TierRowData = (String, String, Option<String>, String, bool, Option<String>, Option<i64>, Option<i64>, Option<String>, i64, Option<String>);

pub(crate) async fn load_tier(state: &AppState, id: &str) -> Option<TierRowData> {
    sqlx::query_as::<_, TierRowData>(
        "SELECT id, name, description, type, active, currency, monthly_price, yearly_price, \
         welcome_page_url, trial_days, visibility FROM products WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

pub(crate) async fn tier_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Some((tid, name, description, ttype, active, currency, monthly, yearly, welcome, trial, visibility)) =
        load_tier(&state, &id).await
    else {
        return not_found_page(&user, "That tier could not be found.");
    };
    let is_free = ttype == "free";
    let benefits = tier_benefits(&state, &tid).await.join("\n");
    let stripe_on = ghost_stripe::StripeConfig::from_settings(&state.settings).is_some();

    // A paid tier is only sellable once Stripe can take the money.
    let price_note = if is_free {
        "The free tier has no price. Its benefits are what members see when they sign up for free.".to_string()
    } else if stripe_on {
        "Saving a changed price creates a new price in Stripe. Existing subscribers stay on the price they signed up at, which is how Stripe works.".to_string()
    } else {
        "Connect Stripe on the Membership screen before setting prices — without it a paid tier cannot be sold.".to_string()
    };

    let price_fields = if is_free {
        String::new()
    } else {
        format!(
            r#"<div class="section">Pricing</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{coin}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="monthly_price" value="{monthly}" placeholder=" " inputmode="decimal"><span>Monthly price</span></label>
<label class="field" style="margin:0"><input name="yearly_price" value="{yearly}" placeholder=" " inputmode="decimal"><span>Yearly price</span></label>
<label class="field fixed" style="margin:0"><select name="currency">{cur_opts}</select><span>Currency</span>{car}</label>
<label class="field" style="margin:0"><input name="trial_days" value="{trial}" placeholder=" " inputmode="numeric"><span>Free trial · days, 0 for none</span></label>
</div>
<p class="muted" style="margin:10px 0 0">{price_note}</p>
</div></div>
</div>"#,
            coin = svg("chart"),
            monthly = esc(&format_price_input(monthly)),
            yearly = esc(&format_price_input(yearly)),
            cur_opts = select_options(
                &[("usd", "USD $"), ("eur", "EUR €"), ("gbp", "GBP £"), ("aud", "AUD $"), ("cad", "CAD $"), ("inr", "INR ₹")],
                &currency.clone().unwrap_or_else(|| "usd".into())
            ),
            car = SELECT_CARET,
            trial = trial,
            price_note = price_note,
        )
    };

    page_for_role(actor_role, 
        "Tier",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/membership/" title="Back to membership">{back}</a>
<div><h1>{name}</h1><p class="sub">{sub}</p></div>
<div class="actions"><button class="btn" type="submit" form="tier-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/settings/membership/tiers/{id}/" id="tier-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{ghost_ic}</span>
<div class="body" style="width:100%">
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " required><span>Name</span></label>
<label class="field fixed" style="margin:0"><select name="visibility">{vis_opts}</select><span>Visibility</span>{car}</label>
</div>
<label class="field" style="margin:12px 0 0"><input name="description" value="{description}" placeholder=" "><span>Description · shown in Portal</span></label>
<label class="field" style="margin:12px 0 0"><input name="welcome_page_url" value="{welcome}" placeholder=" "><span>Welcome page URL · where new members land</span></label>
<label class="check" style="display:flex;align-items:center;gap:10px;margin:14px 0 0">
<label class="switch"><input type="checkbox" name="active" value="1" {act}><span class="track"><span class="thumb"></span></span></label>
<span class="muted">Active · inactive tiers are hidden from Portal</span></label>
</div></div>
</div>
{price_fields}
<div class="section">Benefits</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{check_ic}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><textarea name="benefits" rows="5" placeholder=" ">{benefits}</textarea><span>One benefit per line</span></label>
<p class="muted" style="margin:10px 0 0">Listed under the tier in Portal's signup window.</p>
</div></div>
</div>
</form>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&tid),
            ghost_ic = svg("ghost"),
            check_ic = svg("check"),
            car = SELECT_CARET,
            name = esc(&name),
            name_v = esc(&name),
            sub = if is_free { "Free tier".to_string() } else { "Paid tier".to_string() },
            description = esc(description.as_deref().unwrap_or("")),
            welcome = esc(welcome.as_deref().unwrap_or("")),
            vis_opts = select_options(&[("public", "Public"), ("none", "Hidden")], visibility.as_deref().unwrap_or("public")),
            act = if active { "checked" } else { "" },
            price_fields = price_fields,
            benefits = esc(&benefits),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn tier_edit_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<TierForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some((tid, _name, _d, ttype, _a, old_currency, old_monthly, old_yearly, _w, _t, _v)) =
        load_tier(&state, &id).await
    else {
        return membership_flash("error", "That tier could not be found.");
    };
    let name = form.name.trim();
    if name.is_empty() {
        return tier_flash(&tid, "error", "A tier needs a name.");
    }
    let is_free = ttype == "free";
    let currency = if form.currency.trim().is_empty() { "usd".to_string() } else { form.currency.trim().to_lowercase() };
    let monthly = if is_free { None } else { parse_price_cents(&form.monthly_price) };
    let yearly = if is_free { None } else { parse_price_cents(&form.yearly_price) };
    let trial: i64 = form.trial_days.trim().parse().unwrap_or(0).max(0);

    let now = ghost_core::dates::now_db();
    let res = sqlx::query(
        "UPDATE products SET name = ?, description = ?, welcome_page_url = ?, visibility = ?, \
         active = ?, currency = ?, monthly_price = ?, yearly_price = ?, trial_days = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(form.description.trim())
    .bind(form.welcome_page_url.trim())
    .bind(if form.visibility == "none" { "none" } else { "public" })
    .bind(form.active.is_some())
    .bind(&currency)
    .bind(monthly)
    .bind(yearly)
    .bind(trial)
    .bind(&now)
    .bind(&tid)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        return tier_flash(&tid, "error", &format!("Could not save: {e}"));
    }
    if is_free {
        let mut plans: Vec<String> = state.settings.get("portal_plans").and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default();
        plans.retain(|plan| plan != "free");
        if form.active.is_some() && form.visibility != "none" { plans.insert(0, "free".into()); }
        if let Err(error) = state.settings.set(&state.db, "portal_plans", &serde_json::json!(plans)).await {
            return tier_flash(&tid, "error", &error.message());
        }
    }
    set_tier_benefits(&state, &tid, &form.benefits).await;

    // Mirror the prices into Stripe. Only when something actually changed —
    // Stripe prices are immutable, so every save would otherwise pile up new
    // ones.
    if !is_free {
        let changed = monthly != old_monthly
            || yearly != old_yearly
            || currency != old_currency.unwrap_or_default();
        let missing_prices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ? AND ((monthly_price > 0 AND monthly_price_id IS NULL) OR (yearly_price > 0 AND yearly_price_id IS NULL))")
            .bind(&tid).fetch_one(&state.db).await.unwrap_or(0);
        if changed || missing_prices > 0 {
            if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
                if let Err(e) = sync_tier_to_stripe(&state, &cfg, &tid, name, &currency, monthly, yearly).await {
                    return tier_flash(&tid, "error", &format!("Saved, but Stripe was not updated: {}", e.message()));
                }
            }
        }
    }
    tier_flash(&tid, "ok", "Tier saved.")
}

/// Create the Stripe product and prices for a tier and record their ids.
pub(crate) async fn sync_tier_to_stripe(
    state: &AppState, cfg: &ghost_stripe::StripeConfig, tier_id: &str, name: &str,
    currency: &str, monthly: Option<i64>, yearly: Option<i64>,
) -> ghost_core::Result<()> {
    ghost_api::stripe_tiers::sync_tier(state, cfg, tier_id, name, currency, monthly, yearly).await
}

// --- Stripe connection (Ghost's "Connect to Stripe", self-hosted flavour) ---

pub(crate) fn membership_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/membership/?{encoded}")).into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct StripeConnectForm {
    secret_key: String,
    publishable_key: String,
}

/// Start Connect: mint a state onto this session and send the admin to Stripe.
///
/// The URL, the nonce and its window all live in [`ghost_api::stripe_connect`],
/// shared with the Admin API's `GET /members/stripe_connect`. This handler is
/// only the difference between the two: a form post that reports failure as a
/// flash message on the membership screen.
pub(crate) async fn stripe_oauth_start(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(response) = require_role(&state, &headers, Role::Administrator).await { return response; }
    if ghost_api::stripe_connect::can_connect(&state.config.env, &state.site_url()).is_err() {
        return membership_flash("error", "Stripe Connect requires HTTPS in production.");
    }
    let Some(sid) = session_id_from(&headers) else { return membership_flash("error", "Sign in again before connecting Stripe."); };
    let mode = ghost_api::stripe_connect::mode_from(form.get("mode").map(String::as_str)).unwrap_or("live");
    match ghost_api::stripe_connect::mint_state(&state.db, &sid, mode).await {
        Some(minted) => Redirect::to(&ghost_api::stripe_connect::oauth_url(mode, &minted)).into_response(),
        None => membership_flash("error", "Could not start the Stripe connection. Sign in again and retry."),
    }
}

#[derive(Deserialize)]
pub(crate) struct StripeSecureKeyForm { secure_key: String }

pub(crate) async fn stripe_oauth_finish(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<StripeSecureKeyForm>) -> Response {
    if let Err(response) = require_role(&state, &headers, Role::Administrator).await { return response; }
    let Some(sid) = session_id_from(&headers) else { return membership_flash("error", "Sign in again before connecting Stripe."); };
    let Some(expected) = ghost_api::stripe_connect::pending_state(&state.db, &sid).await else {
        return membership_flash("error", "Start Connect with Stripe before pasting its secure key, and within thirty minutes of doing so.");
    };
    let key = match ghost_api::stripe_connect::decode_secure_key(&form.secure_key, &expected) {
        Ok(key) => key,
        Err(ghost_api::stripe_connect::INCORRECT_STATE) => {
            return membership_flash("error", "The Stripe connection belongs to another session. Start the connection again.");
        }
        Err(message) => return membership_flash("error", message),
    };
    // Spend the nonce before adopting the keys, not after: a blob that got as
    // far as a Stripe call must not stay replayable because that call failed.
    if !ghost_api::stripe_connect::clear_state(&state.db, &sid).await {
        return membership_flash("error", "Could not complete the Stripe connection. Sign in again and retry.");
    }
    stripe_connect(State(state), headers, Form(StripeConnectForm {secret_key: key.secret_key, publishable_key: key.publishable_key})).await
}

/// Store Stripe keys after proving they work.
///
/// Ghost(Pro) uses Stripe Connect's OAuth; a self-hosted site pastes its own
/// keys, and both arrive here. The work — verifying the key against
/// `/v1/account`, clearing price ids that belonged to another account,
/// registering the webhook — is [`ghost_api::stripe_connect::adopt_keys`],
/// shared with the Admin API's Connect token so the two cannot drift. What is
/// left in this handler is how the membership screen says what happened.
pub(crate) async fn stripe_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<StripeConnectForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::stripe_connect::adopt_keys(&state, &form.secret_key, &form.publishable_key).await {
        Ok(adopted) => {
            let mode = if adopted.livemode { "live" } else { "test" };
            let notes = adopted.notes.iter().map(|note| format!(" {note}")).collect::<String>();
            membership_flash(
                "ok",
                &format!("Connected to Stripe account “{}” in {mode} mode.{notes}", adopted.account),
            )
        }
        Err(message) => membership_flash("error", &message),
    }
}

/// The webhook signing-secret form, shown both when nothing is set and when
/// rotating an existing secret.
pub(crate) fn stripe_webhook_secret_form(site_url: &str) -> String {
    format!(
        r#"<form method="post" action="/ghost/settings/membership/stripe/webhook/" class="inline-form">
<label class="field" style="flex:1;min-width:240px;margin:0"><input name="webhook_secret" type="password" placeholder=" " autocomplete="off" required><span>Webhook signing secret · whsec_…</span></label>
<button class="btn tonal small" type="submit">Save secret</button></form>
<p class="muted" style="margin:10px 0 0">Testing locally? Run this and paste the <code>whsec_…</code> it prints:</p>
<p class="muted" style="margin:6px 0 0"><code style="display:block;white-space:pre-wrap;word-break:break-all">stripe listen --events {events} --forward-to {url}/members/webhooks/stripe/</code></p>
<p class="muted" style="margin:6px 0 0">Recent Stripe CLI versions require <code>--events</code> (or <code>--all-snapshot</code>).</p>"#,
        events = ghost_stripe::WEBHOOK_EVENTS.join(","),
        url = esc(site_url),
    )
}

/// Direct API key setup, shown when no account is connected.
/// Hosted onboarding stays hidden until we provide our own Stripe Connect flow.
pub(crate) fn stripe_key_form(heading: &str, blurb: &str, button: &str) -> String {
    format!(
        r#"<span class="title" style="display:block;margin-bottom:6px">{heading}</span>
<p class="muted" style="margin:0 0 12px">{blurb}</p>
<form method="post" action="/ghost/settings/membership/stripe/connect/">
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
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let secret = form.webhook_secret.trim().to_string();
    if !secret.is_empty() && !secret.starts_with("whsec_") {
        return membership_flash("error", "A webhook signing secret starts with whsec_.");
    }
    match state.settings.set(&state.db, "members_stripe_webhook_secret", &serde_json::json!(secret.clone())).await {
        Ok(()) if secret.is_empty() => membership_flash("ok", "Webhook secret cleared. Webhooks will be refused until one is set."),
        Ok(()) => membership_flash("ok", "Webhook signing secret saved."),
        Err(e) => membership_flash("error", &e.message()),
    }
}

/// Turn the webhook off: drop the signing secret, and remove the endpoint at
/// Stripe when we registered one.
///
/// Clearing the secret is what actually deactivates it — every incoming
/// webhook is refused without one — so this is also the safe way to retire a
/// secret that has leaked, without touching the Stripe connection itself.
pub(crate) async fn stripe_deactivate_webhook(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut note = String::new();
    if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
        if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|i| !i.is_empty()) {
            if let Err(e) = ghost_stripe::delete_webhook(&cfg, &id).await {
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
    membership_flash("ok", &format!("Webhook deactivated. Subscription changes will not be accepted until a new secret is set.{note}"))
}

/// Disconnect. The tiers and their Stripe ids are left alone: reconnecting
/// the same account should not mean rebuilding every price, and deleting
/// local price ids would orphan live subscriptions.
pub(crate) async fn stripe_disconnect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // Upstream's guard, and this screen needs it as much as the API does: the
    // keys are what reconciles a subscription, so clearing them while members
    // still hold one leaves them paying Stripe for access this site can no
    // longer confirm, with nothing in the admin able to show it.
    if ghost_api::stripe_connect::has_paid_members(&state.db).await {
        return membership_flash("error", ghost_api::stripe_connect::HAS_SUBSCRIPTIONS);
    }
    // Remove the registered webhook first, while the key still works.
    if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
        if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|i| !i.is_empty()) {
            let _ = ghost_stripe::delete_webhook(&cfg, &id).await;
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
    membership_flash("ok", "Disconnected from Stripe. Existing subscriptions are untouched on Stripe's side.")
}

/// One row of the Portal links reference: a name, and the same destination in
/// both of the spellings Portal accepts.
///
/// `link` is shown first because that is the form a theme author reaches for;
/// the attribute is the one to use on an element that is not an anchor, or
/// where the href has to stay a real URL for visitors without JavaScript. Both
/// are rendered into `data-` attributes and the visible one is chosen in the
/// browser, so flipping the switch costs no round trip.
pub(crate) fn portal_link_row(name: &str, link: &str, attr: &str) -> String {
    format!(
        r#"<div class="plink" data-plink data-link="{link}" data-attr="{attr}">
<span class="name">{name}</span>
<span class="key"><code>{link}</code><button type="button" data-copy="{link}" title="Copy" aria-label="Copy {name}">{copy}</button></span></div>"#,
        name = esc(name),
        link = esc(link),
        attr = esc(attr),
        copy = svg("copy"),
    )
}

/// Every Portal destination the site answers, as a link and as a data
/// attribute — Ghost's Settings → Portal → Links, which this port had nowhere
/// to show.
///
/// Rows are gated on what the site can actually do: with no Stripe connection
/// there are no paid signups, no gifts and no tips, so listing those links
/// would advertise screens Portal refuses to open. `paid_tiers` drives the
/// picker, since a tier-specific signup URL needs a real tier id to work.
pub(crate) fn portal_links_card(
    site_url: &str,
    paid_tiers: &[(String, String)],
    paid_on: bool,
    recommendations_on: bool,
) -> String {
    let base = site_url.trim_end_matches('/');
    let row = |name: &str, path: &str| {
        let attr = path.trim_start_matches('/');
        let attr = if attr.is_empty() {
            "data-portal".to_string()
        } else {
            format!(r#"data-portal="{attr}""#)
        };
        portal_link_row(name, &format!("{base}/#/portal{path}"), &attr)
    };

    let mut generic = String::new();
    generic.push_str(&row("Default", ""));
    generic.push_str(&row("Sign in", "/signin"));
    generic.push_str(&row("Sign up", "/signup"));
    generic.push_str(&row("Sign up / Free", "/signup/free"));
    if paid_on {
        generic.push_str(&row("Sign up / Monthly", "/signup/monthly"));
        generic.push_str(&row("Sign up / Yearly", "/signup/yearly"));
        generic.push_str(&row("Gift subscriptions", "/gift"));
        generic.push_str(&row("Tips & donations", "/support"));
    }
    if recommendations_on {
        generic.push_str(&row("Recommendations", "/recommendations"));
    }

    // The tier block only makes sense once there is a paid tier to name: the
    // id is part of the path, and `#/portal/signup//monthly` opens nothing.
    let tiers = if paid_on && !paid_tiers.is_empty() {
        let opts: String = paid_tiers
            .iter()
            .enumerate()
            .map(|(i, (id, name))| {
                format!(
                    r#"<option value="{id}"{sel}>{name}</option>"#,
                    id = esc(id),
                    name = esc(name),
                    sel = if i == 0 { " selected" } else { "" },
                )
            })
            .collect();
        format!(
            r#"<h3>Tiers</h3>
<div class="plink"><span class="name">Tier</span>
<label class="field fixed" style="margin:0;max-width:260px"><select data-plink-tier>{opts}</select><span>Tier</span>{car}</label></div>
{monthly}{yearly}"#,
            opts = opts,
            car = SELECT_CARET,
            monthly = row("Signup / Monthly", "/signup/{tier}/monthly"),
            yearly = row("Signup / Yearly", "/signup/{tier}/yearly"),
        )
    } else {
        String::new()
    };

    let mut account = String::new();
    account.push_str(&row("Account", "/account"));
    if paid_on {
        account.push_str(&row("Account / Plans", "/account/plans"));
    }
    account.push_str(&row("Account / Profile", "/account/profile"));
    account.push_str(&row("Account / Newsletters", "/account/newsletters"));
    account.push_str(&row("Account / Newsletter help", "/account/newsletters/help"));
    account.push_str(&row("Account / Email disabled", "/account/newsletters/disabled"));

    format!(
        r#"<div class="section">Portal links</div>
<div class="card padded rise plinks" style="--i:7" data-plinks>
<div class="inline-form" style="justify-content:space-between;align-items:center;margin-bottom:6px">
<p class="muted" style="margin:0;max-width:460px">Use these <span data-plink-note>links</span> anywhere in your theme, your navigation or an email to open a page of Portal. Copy a row to use it as it is.</p>
{seg}
</div>
<h3>Generic</h3>
{generic}
{tiers}
<h3>Account</h3>
{account}
<p class="muted" style="margin:18px 0 0">An offer has a link of its own, <code>{base}/&lt;offer code&gt;</code>, which sends the visitor to <code>#/portal/offers/&lt;id&gt;</code> — find it on the offer in <a href="/ghost/settings/growth/">Settings → Growth</a>.{gift_note}</p>
</div>"#,
        seg = segmented(
            "portal_link_mode",
            &[("links", "Links"), ("attrs", "Data attributes")],
            "links",
        ),
        generic = generic,
        tiers = tiers,
        account = account,
        base = esc(base),
        // A redemption link only exists once gifts can be bought, which needs
        // the same Stripe connection as the Gift row above.
        gift_note = if paid_on {
            " A gift email carries <code>#/portal/gift/redeem/&lt;token&gt;</code>, which is minted per gift."
        } else {
            ""
        },
    )
}

pub(crate) async fn membership_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let get = |k: &str, d: &str| state.settings.get_str(k).unwrap_or_else(|| d.to_string());

    let mut tier_rows = String::new();
    // The paid, active tiers also feed the Portal links reference below, whose
    // tier-specific signup URLs name a tier by id.
    let mut paid_tiers: Vec<(String, String)> = Vec::new();
    if let Ok(tiers) = sqlx::query_as::<_, (String, String, String, Option<i64>, Option<i64>, i64)>(
        "SELECT id, name, type, monthly_price, yearly_price, active FROM products ORDER BY type DESC, created_at",
    )
    .fetch_all(&state.db)
    .await
    {
        for (id, name, tier_type, monthly, yearly, active) in tiers {
            if active != 0 && tier_type != "free" {
                paid_tiers.push((id.clone(), name.clone()));
            }
            let price = if tier_type == "free" {
                "Free".to_string()
            } else {
                format!("{} / month · {} / year", tier_price(monthly), tier_price(yearly))
            };
            tier_rows.push_str(&format!(
                r#"<a class="row" href="/ghost/settings/membership/tiers/{id}/"><span class="ic tint-teal">{ghost}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{price}</span></div>
<div class="trailing">{inactive}<span class="chip tonal">{typ}</span>{chev}</div></a>"#,
                id = esc(&id),
                ghost = svg("ghost"),
                name = esc(&name),
                price = esc(&price),
                inactive = if active != 0 { "" } else { r#"<span class="chip draft">inactive</span>"# },
                typ = esc(&tier_type),
                chev = svg("chevron"),
            ));
        }
    }

    let mut newsletter_rows = String::new();
    // the primary newsletter's editable fields: (id, name, description, sender_name, sender_email)
    let mut newsletter_edit: Option<(String, String, String, String, String)> = None;
    if let Ok(rows) = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, name, sender_name, sender_email, status, description FROM newsletters ORDER BY sort_order, created_at",
    )
    .fetch_all(&state.db)
    .await
    {
        for (i, (id, name, sender_name, sender_email, status, description)) in rows.iter().enumerate() {
            if i == 0 {
                newsletter_edit = Some((
                    id.clone(),
                    name.clone(),
                    description.clone().unwrap_or_default(),
                    sender_name.clone().unwrap_or_default(),
                    sender_email.clone().unwrap_or_default(),
                ));
            }
            let edit = if i == 0 {
                r#"<span class="chip published">editable below</span>"#.to_string()
            } else {
                String::new()
            };
            let sender = [sender_name.clone().unwrap_or_default(), sender_email.clone().unwrap_or_default()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" <");
            newsletter_rows.push_str(&format!(
                r#"<div class="row"><span class="ic tint-teal">{mail}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{sender}{status}</span></div>
<div class="trailing">{edit}<span class="chip tonal">{id_short}</span></div></div>"#,
                mail = svg("notes"),
                name = esc(name),
                sender = if sender.is_empty() { String::new() } else { format!("{} · ", esc(&sender)) },
                status = esc(status.as_deref().unwrap_or("active")),
                edit = edit,
                id_short = esc(&id[..8.min(id.len())]),
            ));
        }
    }

    // Stripe connection card. Ghost(Pro) does this over Stripe Connect OAuth;
    // a self-hosted site pastes its own keys, which is what this offers.
    let stripe_card = match ghost_stripe::StripeConfig::from_settings(&state.settings) {
        Some(cfg) => {
            let who = state.settings.get_str("stripe_connect_display_name").unwrap_or_default();
            let account = state.settings.get_str("stripe_connect_account_id").unwrap_or_default();
            let live = cfg.is_live();
            let hook = state.settings.get_str("members_stripe_webhook_secret").unwrap_or_default();
            format!(
                r#"<div class="section">Stripe</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-green">{check_ic}</span>
<div class="body"><span class="title">Connected{named}</span><span class="detail">{account} · {mode} mode</span></div>
<div class="trailing"><span class="chip {chip}">{mode}</span>
<form method="post" action="/ghost/settings/membership/stripe/disconnect/" data-confirm="Disconnect from Stripe|New paid signups will stop. Existing subscriptions stay active on Stripe and resume if you reconnect the same account.">
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
<form method="post" action="/ghost/settings/membership/stripe/webhook/deactivate/" data-confirm="Deactivate webhook|Subscription changes will stop being accepted until a new signing secret is set. Payments still go through at Stripe.">
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
            r#"<div class="section">Stripe</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{coin}</span>
<div class="body" style="width:100%">{key_form}</div></div>
</div>"#,
            coin = svg("chart"),
            key_form = stripe_key_form(
                "Connect Stripe",
                "Enter your Stripe secret and publishable keys to offer paid tiers. Use test keys until you are ready to accept payments.",
                "Save API keys",
            ),
        ),
    };

    let toggle = |key: &str, title: &str, detail: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-teal">{ghost}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" form="membership-form" name="{key}" value="1" {ck}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            ghost = svg("ghost"),
            title = esc(title),
            detail = esc(detail),
            key = esc(key),
            ck = if state.settings.get_bool(key) { "checked" } else { "" },
        )
    };

    // The access code only matters while the site is private, so Ghost only
    // shows it then. It is deliberately readable rather than masked: it is
    // shared with readers, not a credential anyone types in private.
    let private_code = if state.settings.get_bool("is_private") {
        let site_url = state.site_url();
        format!(
            r#"<div style="display:flex;gap:10px;align-items:flex-end;flex-wrap:wrap;margin-top:12px">
<label class="field" style="margin:0;max-width:320px;flex:1"><input name="password" form="membership-form" value="{code}" placeholder=" " maxlength="191"><span>Access code</span></label>
<button class="btn tonal small" type="submit" form="membership-form" name="regenerate_code" value="1" title="Generate a new code">{refresh} Regenerate</button>
</div>
<p class="muted" style="margin:6px 0 0">Everyone who visits <a href="{url}/private/" target="_blank" rel="noopener">{url}/private/</a> needs this.
Changing it — including regenerating — signs out everyone who had the old one.</p>"#,
            code = s("password"),
            refresh = svg("history"),
            url = esc(site_url.trim_end_matches('/')),
        )
    } else {
        String::new()
    };

    // Ghost's fourth default-access option: specific tiers. The chips only
    // mean anything while that option is chosen, so they are rendered with the
    // row rather than as a separate setting.
    let visibility_now = get("default_content_visibility", "public");
    let tier_options: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM products WHERE active = 1 ORDER BY type DESC, created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect();
    let selected_tiers: Vec<String> = match state.settings.get("default_content_visibility_tiers") {
        Some(serde_json::Value::Array(a)) => {
            a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        Some(serde_json::Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => Vec::new(),
    };
    let visibility_tiers = if visibility_now == "tiers" {
        format!(
            r#"<div style="margin-top:10px">{chips}</div>
<p class="muted" style="margin:8px 0 0">New posts are visible to members on any of the ticked tiers.</p>"#,
            chips = chip_set(
                "default_content_visibility_tiers",
                &tier_options,
                &selected_tiers,
                "No tiers yet — add one under Tiers below.",
            ),
        )
    } else {
        String::new()
    };

    // --- welcome emails -----------------------------------------------------
    //
    // These are the two built-in welcome automations, seeded on every install.
    // Ghost surfaces them here as well as under Automations, because this is
    // where a publisher looks for them.
    let welcome_rows = {
        let flows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
            "SELECT id, name, status, slug FROM automations WHERE slug IN (?, ?) ORDER BY slug",
        )
        .bind("member-welcome-email-free")
        .bind("member-welcome-email-paid")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let mut rows = String::new();
        for (id, name, status, slug) in &flows {
            let active = status == "active";
            let is_free = slug.as_deref() == Some("member-welcome-email-free");
            // An empty flow cannot be turned on — the automation engine
            // refuses it — so offer the editor instead of a switch that is
            // guaranteed to bounce.
            let written: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM automation_actions WHERE automation_id = ?")
                    .bind(id)
                    .fetch_one(&state.db)
                    .await
                    .unwrap_or(0);
            let trailing = if written == 0 {
                format!(
                    r#"<span class="chip draft">not written</span><a class="btn tonal small" href="/ghost/automations/{id}/">Write it</a>"#,
                    id = esc(id),
                )
            } else {
                format!(
                    r#"<span class="chip {chip}">{state_label}</span>
<form method="post" action="/ghost/automations/{id}/status/">
<input type="hidden" name="status" value="{next}">
<button class="btn tonal small" type="submit">{button}</button></form>"#,
                    id = esc(id),
                    chip = if active { "published" } else { "draft" },
                    state_label = if active { "on" } else { "off" },
                    next = if active { "inactive" } else { "active" },
                    button = if active { "Turn off" } else { "Turn on" },
                )
            };
            rows.push_str(&format!(
                r#"<div class="row"><span class="ic tint-teal">{ic}</span>
<div class="body"><a class="title" href="/ghost/automations/{id}/">{title}</a><span class="detail">{detail}</span></div>
<div class="trailing">{trailing}</div></div>"#,
                ic = svg("mail"),
                id = esc(id),
                title = esc(name),
                detail = if is_free {
                    "Sent when someone signs up for a free account"
                } else {
                    "Sent when someone starts a paid subscription"
                },
                trailing = trailing,
            ));
        }
        if rows.is_empty() {
            rows = r#"<div class="row"><div class="body"><span class="muted">The welcome flows are seeded at startup and will appear here.</span></div></div>"#.into();
        }
        format!(
            r#"<div class="card rows rise" style="--i:4">{rows}</div>
<p class="muted" style="margin:10px 0 0 16px">Open one to write it. Signup and signin magic links are separate — they always send, using the
transport in <a href="/ghost/settings/email/">Settings → Email</a>.</p>"#
        )
    };

    // --- custom fields: Ghost's Membership → Custom fields ------------------
    //
    // Each row is its own form: a field's key and type are fixed once it
    // exists (values reference the key and are shaped by the type), so only
    // the name and the member's access can be edited here.
    let fields = repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let last = fields.len().saturating_sub(1);
    let mut field_rows = String::new();
    for (i, f) in fields.iter().enumerate() {
        let held = repos::metafields::value_count(&state.db, &f.key).await;
        let type_label = match f.field_type.as_str() {
            "short_text" => "Short text",
            "long_text" => "Long text",
            "address" => "Address",
            other => other,
        };
        field_rows.push_str(&format!(
            r#"<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{ic}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/membership/fields/{id}/" class="inline-form" style="align-items:flex-end">
<label class="field" style="flex:1;min-width:180px;margin:0"><input name="name" value="{name}" placeholder=" " maxlength="191" required><span>Name</span></label>
<label class="field fixed" style="min-width:190px;margin:0"><select name="member_access">{access_opts}</select><span>Member access</span>{car}</label>
<label class="field fixed" style="min-width:150px;margin:0"><select name="status">{status_opts}</select><span>Status</span>{car}</label>
<button class="btn tonal small" type="submit">Save</button>
</form>
<p class="muted" style="margin:8px 0 0"><code>{identity}</code> · {type_label} · {held} member{plural} with a value</p>
</div>
<div class="trailing" style="align-items:flex-start">
<form method="post" action="/ghost/settings/membership/fields/{id}/move/"><input type="hidden" name="dir" value="up">
<button class="icon-btn" style="transform:rotate(-90deg)" title="Move up" aria-label="Move up"{up_dis}>{up}</button></form>
<form method="post" action="/ghost/settings/membership/fields/{id}/move/"><input type="hidden" name="dir" value="down">
<button class="icon-btn" style="transform:rotate(90deg)" title="Move down" aria-label="Move down"{down_dis}>{down}</button></form>
<form method="post" action="/ghost/settings/membership/fields/{id}/delete/" data-confirm="Delete custom field|&#8220;{name}&#8221; and the values {held} member{plural} hold for it will be deleted.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>
</div></div>"#,
            ic = svg("notes"),
            id = esc(&f.id),
            name = esc(&f.name),
            identity = esc(&f.identity()),
            type_label = type_label,
            held = held,
            plural = if held == 1 { "" } else { "s" },
            car = SELECT_CARET,
            access_opts = select_options(
                &[
                    ("none", "Staff only"),
                    ("read", "Members can view"),
                    ("write", "Members can edit"),
                ],
                &f.member_access,
            ),
            status_opts = select_options(&[("active", "Active"), ("archived", "Archived")], &f.status),
            up = svg("chevron"),
            down = svg("chevron"),
            up_dis = if i == 0 { " disabled" } else { "" },
            down_dis = if i == last { " disabled" } else { "" },
            del = svg("delete"),
        ));
    }
    if field_rows.is_empty() {
        field_rows = r#"<div class="row"><div class="body"><span class="muted">No custom fields yet — add one to collect something extra about your members.</span></div></div>"#.into();
    }
    let fields_card = format!(
        r#"<div class="section">Custom fields</div>
<div class="card rows rise" style="--i:7">{field_rows}</div>
<div class="card padded rise" style="--i:7">
<form method="post" action="/ghost/settings/membership/fields/new/" class="inline-form">
<label class="field" style="flex:1;min-width:200px;margin:0"><input name="name" placeholder=" " maxlength="191" required><span>Field name</span></label>
<label class="field fixed" style="min-width:170px;margin:0"><select name="type">{type_opts}</select><span>Type</span>{car}</label>
<label class="field fixed" style="min-width:190px;margin:0"><select name="member_access">{new_access_opts}</select><span>Member access</span>{car}</label>
<button class="btn tonal" type="submit">{add} Add field</button>
</form>
<p class="muted" style="margin:12px 0 0">The key is taken from the name and cannot change afterwards — values are stored against it.
An <b>address</b> collects six parts (line 1 and 2, city, state, postal code, country).
<b>Staff only</b> keeps a field out of the member's own account page entirely.</p>
</div>"#,
        field_rows = field_rows,
        car = SELECT_CARET,
        add = svg("add"),
        type_opts = select_options(
            &[("short_text", "Short text"), ("long_text", "Long text"), ("address", "Address")],
            "short_text",
        ),
        new_access_opts = select_options(
            &[("none", "Staff only"), ("read", "Members can view"), ("write", "Members can edit")],
            "none",
        ),
    );

    // --- spam filters: Ghost puts this immediately after Access ------------
    // `blocked_email_domains` is stored as a JSON array; the textarea is the
    // readable form of it, one domain per line.
    let blocked_domains = state.settings.blocked_email_domains().join("\n");
    let spam_card = format!(
        r#"<div class="section">Spam filters</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{shield}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><textarea name="blocked_email_domains" form="membership-form" rows="3" placeholder=" ">{domains}</textarea><span>Blocked email domains</span></label>
<p class="muted" style="margin:6px 0 0">Protect your member signups from spam. One domain per line, e.g. <code>spam.xyz</code> to block
addresses like <code>hello@spam.xyz</code>. Members who already signed up are not removed.</p>
</div></div>
</div>"#,
        shield = svg("flag"),
        domains = esc(&blocked_domains),
    );

    // Stripe is what unlocks paid members, and gifts and tips both bill
    // through it — Ghost hides each of these sections until it is connected.
    let stripe_on = ghost_stripe::StripeConfig::from_settings(&state.settings).is_some();
    let site_url = state.site_url();
    let site_url = site_url.trim_end_matches('/');

    let gift_card = if stripe_on {
        format!(
            r#"<div class="section">Gift subscriptions</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{gift}</span>
<div class="body" style="width:100%">
<span class="title">Shareable link</span>
<p class="muted" style="margin:6px 0 10px">Allow your readers to share your work by purchasing a gift subscription for a friend or colleague.</p>
{link}
</div></div>
</div>"#,
            gift = svg("ghost"),
            link = key_chip("Gift", &format!("{site_url}/#/portal/gift")),
        )
    } else {
        String::new()
    };

    // Ghost stores the suggested tip in the currency's minor unit (cents), but
    // asks for it in the major one — so it is divided here and multiplied back
    // on save.
    let tips_card = if stripe_on {
        let cents = state.settings.get_str("donations_suggested_amount").unwrap_or_default();
        let cents: i64 = cents.trim().parse().unwrap_or(0);
        format!(
            r#"<div class="section">Tips &amp; donations</div>
<div class="card rows rise" style="--i:6">
<div class="row"><span class="ic tint-green">{coin}</span>
<div class="body"><div class="grid2" style="gap:12px">
<label class="field fixed" style="margin:0"><select name="donations_currency" form="membership-form">{currency_opts}</select><span>Currency</span>{car}</label>
<label class="field" style="margin:0"><input name="donations_suggested_amount" form="membership-form" type="number" min="0" max="10000" step="1" value="{amount}" placeholder=" "><span>Suggested amount</span></label>
</div>
<p class="muted" style="margin:8px 0 0">One-time payments, no account needed. Stripe caps a preset amount at 10,000.</p></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{linkic}</span>
<div class="body" style="width:100%"><span class="title">Shareable link</span>
<p class="muted" style="margin:6px 0 10px">Send readers straight to the tip page, or add it to your navigation.</p>
{link}</div></div>
</div>"#,
            coin = svg("ghost"),
            linkic = svg("link"),
            car = SELECT_CARET,
            currency_opts = select_options(CURRENCIES, &get("donations_currency", "USD")),
            amount = cents / 100,
            link = key_chip("Tips", &format!("{site_url}/#/portal/support")),
        )
    } else {
        String::new()
    };

    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "membership").unwrap();
    page_for_role(actor_role, 
        "Membership",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/membership/" id="membership-form"></form>
<div class="section">Checkout</div>
<div class="card rows rise" style="--i:0"><a class="row" href="/ghost/settings/checkout/"><span class="ic tint-pink">{palette}</span>
<div class="body"><span class="title">Checkout design</span><span class="detail">Colours, typography and button corners for paid signups</span></div>
<div class="trailing">{chev}</div></a></div>
<div class="section">Access</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-amber">{private_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="is_private" form="membership-form">{private_opts}</select><span>Site access</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Who is allowed to browse the site at all. A private site is hidden from search
engines and asks every visitor for one shared access code.</p>
{private_code}</div></div>
<div class="row"><span class="ic tint-teal">{key}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="members_signup_access" form="membership-form">{access_opts}</select><span>Signup access</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Who is allowed to create an account on your site.</p></div></div>
<div class="row"><span class="ic tint-teal">{lock}</span>
<div class="body" style="width:100%"><label class="field fixed" style="margin:0;max-width:320px"><select name="default_content_visibility" form="membership-form" onchange="this.form.requestSubmit()">{visibility_opts}</select><span>Default post access</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Access used by new posts when none is set in the editor.</p>{visibility_tiers}</div></div>
<div class="row"><span class="ic tint-teal">{comment_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="comments_enabled" form="membership-form">{comment_opts}</select><span>Commenting</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Who can comment on posts. Readers always need an account to comment. Existing comments stay visible when you turn commenting off — <a href="/ghost/comments/">moderate them here</a>.</p></div></div>
</div>
{spam_card}
{stripe_card}
<div class="section">Tiers</div>
<div class="card rows rise" style="--i:2">{tier_rows}</div>
<div style="padding:2px 4px 8px"><form method="post" action="/ghost/settings/membership/tiers/new/">
<button class="btn tonal small" type="submit">{add} Add tier</button></form></div>
<p class="muted" style="margin:4px 0 0 16px">Open a tier to change its name, price, benefits and visibility. Paid tiers need Stripe connected above.</p>
<div class="section">Signup portal</div>
<div class="card rows rise" style="--i:3">
{portal_button}
<div class="row" style="align-items:center"><span class="ic tint-teal">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="portal_button_signup_text" form="membership-form" value="{signup_text}" placeholder=" "><span>Portal button signup text</span></label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-teal">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="portal_button_icon" form="membership-form" value="{icon}" placeholder=" "><span>Portal button icon</span></label></div></div>
<div class="row"><span class="ic tint-teal">{palette}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:260px"><select name="portal_button_style" form="membership-form">{style_opts}</select><span>Portal button style</span>{car}</label></div></div>
{portal_name}
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{notes}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="portal_signup_terms_html" form="membership-form" rows="3" placeholder=" ">{terms}</textarea><span>Signup terms · HTML shown on signup</span></label></div></div>
{terms_required}
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{plans_ic}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:10px">Plans shown at signup</span>
{plan_chips}
<div style="max-width:260px;margin-top:12px"><label class="field fixed" style="margin:0"><select name="portal_default_plan" form="membership-form">{default_plan_opts}</select><span>Default price at signup</span>{car}</label></div>
<p class="muted" style="margin:8px 0 0">Which prices the portal offers, and which one is preselected. A default that is not on offer falls back to one that is.</p>
</div></div>
<div class="row" style="align-items:center"><span class="ic tint-teal">{mail_ic}</span>
<div class="body"><label class="field" style="margin:0;max-width:360px"><input name="members_support_address" form="membership-form" type="email" value="{support_address}" placeholder=" "><span>Support email address</span></label>
<p class="muted" style="margin:6px 0 0">Shown on the member's account page and used as the reply-to on member emails. Blank uses the sender address from <a href="/ghost/settings/email/">Settings → Email</a>.</p></div></div>
{gift_signup}
{gift_account}
</div>
{gift_card}
<div class="section">Welcome emails</div>
{welcome_deferred}
{tips_card}
{fields_card}
<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:5">{newsletter_rows}</div>
{newsletter_edit}
<p class="muted" style="margin:4px 0 0 16px">The design of each newsletter — colours, typography, header and footer — is set in <a href="/ghost/settings/newsletters/">Settings → Newsletters</a>.</p>
{portal_links}
"#,
            head = group_head(group, Some("membership-form")),
            banner = flash_banner(&q),
            spam_card = spam_card,
            gift_card = gift_card,
            tips_card = tips_card,
            fields_card = fields_card,
            private_ic = svg("lock"),
            private_opts = select_options(
                &[("false", "Public"), ("true", "Private")],
                if state.settings.get_bool("is_private") { "true" } else { "false" },
            ),
            private_code = private_code,
            key = svg("ghost"),
            lock = svg("flag"),
            car = SELECT_CARET,
            access_opts = select_options(&[("all", "Anybody can sign up"), ("invite", "People can only be invited"), ("none", "Nobody can sign up")], &get("members_signup_access", "all")),
            visibility_opts = select_options(
                &[
                    ("public", "Public"),
                    ("members", "Members only"),
                    ("paid", "Paid-members only"),
                    ("tiers", "Specific tiers"),
                ],
                &visibility_now,
            ),
            visibility_tiers = visibility_tiers,
            comment_ic = svg("comment"),
            // Ghost's three levels, same values: `off`, `all`, `paid`.
            comment_opts = select_options(&[("off", "Nobody"), ("all", "All members"), ("paid", "Paid members only")], &get("comments_enabled", "off")),
            tier_rows = tier_rows,
            stripe_card = stripe_card,
            add = svg("add"),
            portal_button = toggle("portal_button", "Show portal button", "Display the floating portal button on the site"),
            portal_name = toggle("portal_name", "Show portal name", "Show the site name inside the portal"),
            terms_required = toggle("portal_signup_checkbox_required", "Require signup terms", "Members must tick the terms checkbox to sign up"),
            plans_ic = svg("tags"),
            mail_ic = svg("mail"),
            plan_chips = {
                // Ghost stores this as a JSON array of plan names.
                let current: Vec<String> = match state.settings.get("portal_plans") {
                    Some(serde_json::Value::Array(a)) => {
                        a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
                    }
                    Some(serde_json::Value::String(s)) => {
                        serde_json::from_str(&s).unwrap_or_default()
                    }
                    _ => vec!["free".to_string()],
                };
                chip_set(
                    "portal_plans",
                    &[
                        ("free".to_string(), "Free".to_string()),
                        ("monthly".to_string(), "Monthly".to_string()),
                        ("yearly".to_string(), "Yearly".to_string()),
                    ],
                    &current,
                    "",
                )
            },
            default_plan_opts = select_options(
                &[("monthly", "Monthly"), ("yearly", "Yearly")],
                &get("portal_default_plan", "monthly"),
            ),
            support_address = s("members_support_address"),
            gift_signup = toggle(
                "portal_signup_gift_promotion",
                "Promote gifts at signup",
                "Offer a gift subscription on the signup screen",
            ),
            gift_account = toggle(
                "portal_account_gift_promotion",
                "Promote gifts on the account page",
                "Offer a gift subscription from a member's own account",
            ),
            title_ic = svg("title"),
            palette = svg("themes"),
            chev = svg("chevron"),
            notes = svg("notes"),
            signup_text = s("portal_button_signup_text"),
            icon = s("portal_button_icon"),
            style_opts = select_options(&[("icon-and-text", "Icon and text"), ("icon", "Icon only"), ("text", "Text only")], &get("portal_button_style", "icon-and-text")),
            terms = s("portal_signup_terms_html"),
            welcome_deferred = welcome_rows,
            newsletter_rows = newsletter_rows,
            newsletter_edit = newsletter_edit
                .map(|(id, name, description, sender_name, sender_email)| {
                    format!(
                        r#"<div class="card padded rise" style="--i:6">
<p class="muted" style="margin:0 0 12px">Edit <b>{name}</b></p>
<label class="field"><input name="newsletter_name" form="membership-form" value="{name}" placeholder=" "><span>Newsletter name</span></label>
<label class="field"><input name="newsletter_description" form="membership-form" value="{description}" placeholder=" "><span>Description · optional</span></label>
<div class="grid2">
<label class="field"><input name="newsletter_sender_name" form="membership-form" value="{sender_name}" placeholder=" "><span>Sender name</span></label>
<label class="field"><input name="newsletter_sender_email" form="membership-form" value="{sender_email}" placeholder=" "><span>Sender email</span></label>
</div>
<input type="hidden" name="newsletter_id" form="membership-form" value="{id}">
</div>"#,
                        name = esc(&name),
                        description = esc(&description),
                        sender_name = esc(&sender_name),
                        sender_email = esc(&sender_email),
                        id = esc(&id),
                    )
                })
                .unwrap_or_default(),
            // Gated on the same connection as the Gift and Tips cards above,
            // so the page never offers a link to a screen it has just said is
            // unavailable.
            portal_links = portal_links_card(
                site_url,
                &paid_tiers,
                stripe_on,
                state.settings.get_bool("recommendations_enabled"),
            ),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn membership_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // Checkbox chips repeat under one name, so the body is read as pairs; the
    // map is only for the fields that appear once.
    let many = |k: &str| -> Vec<String> {
        pairs.iter().filter(|(n, _)| n == k).map(|(_, v)| v.clone()).collect()
    };
    let form: HashMap<String, String> = pairs.iter().cloned().collect();
    let get = |k: &str| form.get(k).cloned().unwrap_or_default();
    let one_of = |v: String, allowed: &[&str], fallback: String| {
        if allowed.contains(&v.as_str()) { v } else { fallback }
    };
    let access = one_of(get("members_signup_access"), &["all", "invite", "none"], "all".into());
    let visibility = one_of(
        get("default_content_visibility"),
        &["public", "members", "paid", "tiers"],
        "public".into(),
    );
    let style = one_of(get("portal_button_style"), &["icon-and-text", "icon", "text"], "icon-and-text".into());
    let commenting = one_of(get("comments_enabled"), &["off", "all", "paid"], "off".into());

    let mut ok = true;
    ok &= state.settings.set(&state.db, "members_signup_access", &serde_json::json!(access)).await.is_ok();
    ok &= state.settings.set(&state.db, "default_content_visibility", &serde_json::json!(visibility)).await.is_ok();
    ok &= state.settings.set(&state.db, "comments_enabled", &serde_json::json!(commenting)).await.is_ok();
    ok &= state.settings.set(&state.db, "portal_button_style", &serde_json::json!(style)).await.is_ok();
    for key in ["portal_button_signup_text", "portal_button_icon", "portal_signup_terms_html"] {
        ok &= state.settings.set(&state.db, key, &serde_json::json!(get(key))).await.is_ok();
    }
    for key in [
        "portal_button",
        "portal_name",
        "portal_signup_checkbox_required",
        "portal_signup_gift_promotion",
        "portal_account_gift_promotion",
    ] {
        let on = form.get(key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(on)).await.is_ok();
    }
    ok &= state
        .settings
        .set(&state.db, "members_support_address", &serde_json::json!(get("members_support_address").trim()))
        .await
        .is_ok();

    // Which tiers a "Specific tiers" default applies to. Only meaningful at
    // that setting, so it is cleared otherwise rather than left to go stale.
    let tiers = if visibility == "tiers" { many("default_content_visibility_tiers") } else { vec![] };
    ok &= state
        .settings
        .set(&state.db, "default_content_visibility_tiers", &serde_json::json!(tiers))
        .await
        .is_ok();

    // Portal plans. Ghost keeps a JSON array, and guarantees the default is
    // one of the offered prices — a default nobody can pick would leave the
    // portal preselecting a price it does not show.
    let plans: Vec<String> = many("portal_plans")
        .into_iter()
        .filter(|p| ["free", "monthly", "yearly"].contains(&p.as_str()))
        .collect();
    let default_plan = one_of(get("portal_default_plan"), &["monthly", "yearly"], "monthly".into());
    let default_plan = if plans.contains(&default_plan) {
        default_plan
    } else if plans.iter().any(|p| p == "monthly") {
        "monthly".to_string()
    } else if plans.iter().any(|p| p == "yearly") {
        "yearly".to_string()
    } else {
        default_plan
    };
    ok &= ghost_db::repos::tiers::set_free_signup_visibility(&state.db, plans.iter().any(|plan| plan == "free")).await.is_ok();
    ok &= state.settings.set(&state.db, "portal_plans", &serde_json::json!(plans)).await.is_ok();
    ok &= state
        .settings
        .set(&state.db, "portal_default_plan", &serde_json::json!(default_plan))
        .await
        .is_ok();

    // Private site. Turning it on with no code yet generates one, the way
    // Ghost does — a private site with an empty code would be one nobody,
    // including its owner, could get into.
    let private = get("is_private") == "true";
    ok &= state.settings.set(&state.db, "is_private", &serde_json::Value::Bool(private)).await.is_ok();
    if private {
        // "Regenerate" wins over whatever is in the box: the button means
        // "give me a new one", so a stale typed value must not override it.
        let regenerate = get("regenerate_code") == "1";
        let typed = get("password");
        let code = if regenerate {
            generate_access_code()
        } else {
            match typed.trim() {
                "" => state
                    .settings
                    .get_str("password")
                    .filter(|c| !c.trim().is_empty())
                    .unwrap_or_else(generate_access_code),
                t => t.to_string(),
            }
        };
        ok &= state.settings.set(&state.db, "password", &serde_json::json!(code)).await.is_ok();
    }

    // Spam filters. Stored as a JSON array, so the textarea is parsed rather
    // than saved verbatim.
    let domains = ghost_db::settings_cache::parse_blocked_email_domains(&get("blocked_email_domains"));
    ok &= state
        .settings
        .set(&state.db, "blocked_email_domains", &serde_json::json!(domains))
        .await
        .is_ok();

    // Tips & donations. Only written when the section was on screen — it is
    // hidden until Stripe is connected, and a hidden form posts nothing, which
    // would otherwise read back as "clear the amount".
    if form.contains_key("donations_currency") {
        let currency = one_of(
            get("donations_currency"),
            &CURRENCIES.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            "USD".into(),
        );
        // Shown in the major unit, stored in the minor one, and capped where
        // Stripe caps a preset amount.
        let amount = get("donations_suggested_amount").trim().parse::<i64>().unwrap_or(0);
        let cents = amount.clamp(0, 10_000) * 100;
        ok &= state.settings.set(&state.db, "donations_currency", &serde_json::json!(currency)).await.is_ok();
        ok &= state
            .settings
            .set(&state.db, "donations_suggested_amount", &serde_json::json!(cents))
            .await
            .is_ok();
    }

    // the primary newsletter's editable fields
    let nid = get("newsletter_id");
    if !nid.is_empty() {
        let now = ghost_core::dates::now_db();
        let res = sqlx::query(
            "UPDATE newsletters SET name = ?, description = ?, sender_name = ?, sender_email = ?, updated_at = ? WHERE id = ?",
        )
        .bind(get("newsletter_name"))
        .bind(get("newsletter_description"))
        .bind(get("newsletter_sender_name"))
        .bind(get("newsletter_sender_email"))
        .bind(&now)
        .bind(&nid)
        .execute(&state.db)
        .await;
        ok &= res.map(|r| r.rows_affected() > 0).unwrap_or(false);
    }

    if ok {
        settings_flash("membership", "ok", "Membership settings saved.")
    } else {
        settings_flash("membership", "error", "Saving failed — check the logs.")
    }
}

// --- custom fields (Ghost's member metafields) ------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct MetafieldNewForm {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
    member_access: String,
}

pub(crate) async fn metafield_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<MetafieldNewForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return settings_flash("membership", "error", "A custom field needs a name.");
    }
    // The key is minted from the name and then frozen: values are stored
    // against it, so it cannot follow a later rename.
    let key = ghost_core::metafields::key_from_name(&name);
    if !ghost_core::metafields::is_valid_key(&key) {
        return settings_flash(
            "membership",
            "error",
            "That name has no letters or digits to build a key from.",
        );
    }
    if !ghost_core::metafields::is_known_type(&form.field_type) {
        return settings_flash("membership", "error", "Choose a field type.");
    }
    let access = if ghost_core::metafields::is_member_access(&form.member_access) {
        form.member_access.as_str()
    } else {
        "none"
    };
    match repos::metafields::definition_conflict(&state.db, &key, &name, None).await {
        Ok(Some("key")) => {
            return settings_flash(
                "membership",
                "error",
                &format!("Another field already uses the key “{key}” — pick a different name."),
            )
        }
        Ok(Some(_)) => {
            return settings_flash("membership", "error", "Another field already has that name.")
        }
        Err(e) => return settings_flash("membership", "error", &e.message()),
        Ok(None) => {}
    }
    match repos::metafields::create_definition(&state.db, &key, &name, &form.field_type, access)
        .await
    {
        Ok(f) => settings_flash("membership", "ok", &format!("Added “{}”.", f.name)),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct MetafieldSaveForm {
    name: String,
    member_access: String,
    status: String,
}

pub(crate) async fn metafield_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<MetafieldSaveForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(_)) = repos::metafields::find_definition(&state.db, &id).await else {
        return settings_flash("membership", "error", "That custom field no longer exists.");
    };
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return settings_flash("membership", "error", "A custom field needs a name.");
    }
    if let Ok(Some(_)) =
        repos::metafields::definition_conflict(&state.db, "", &name, Some(&id)).await
    {
        return settings_flash("membership", "error", "Another field already has that name.");
    }
    let access = if ghost_core::metafields::is_member_access(&form.member_access) {
        form.member_access.as_str()
    } else {
        "none"
    };
    let status = if form.status == "archived" { "archived" } else { "active" };
    match repos::metafields::update_definition(&state.db, &id, &name, access, status).await {
        Ok(f) => settings_flash("membership", "ok", &format!("Saved “{}”.", f.name)),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

pub(crate) async fn metafield_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::metafields::delete_definition(&state.db, &id).await {
        // The values cascade with it, which the confirmation said.
        Ok(()) => settings_flash("membership", "ok", "Custom field deleted."),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct MetafieldMoveForm {
    dir: String,
}

pub(crate) async fn metafield_move(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<MetafieldMoveForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::metafields::reorder_definition(&state.db, &id, form.dir == "up").await {
        Ok(()) => settings_flash("membership", "ok", "Order saved."),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}
