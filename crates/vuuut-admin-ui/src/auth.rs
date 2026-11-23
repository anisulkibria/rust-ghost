//! Getting in: the setup wizard, sign-in, the emailed code, and signing out.

use super::*;

// ---------------------------------------------------------------------------
// shared pieces of the signed-out screens
// ---------------------------------------------------------------------------

/// The banner an auth screen shows when something went wrong.
///
/// `role="alert"` so a screen reader announces it on arrival: the page has
/// just reloaded and the only thing that changed is this sentence, which a
/// sighted person sees and nobody else would.
///
/// The message is always one of this module's own literals — never anything
/// from the query string — but it goes through `esc` anyway, so that stays
/// true if somebody later passes something that is not.
pub(crate) fn auth_error(message: &str) -> String {
    format!(
        r#"<div class="banner error" role="alert">{icon}<span class="msg">{message}</span></div>"#,
        icon = svg("warning"),
        message = esc(message),
    )
}

/// A password input with a button that reveals what has been typed.
///
/// Typing a long password blind into a form that answers "that does not
/// match" is the worst loop on this screen, and the reveal is the cheapest
/// way out of it. The button is `aria-pressed`, so its state is announced
/// rather than only drawn, and it is hidden outright without scripting —
/// see the `.no-js` rules in the stylesheet.
///
/// `autocomplete` differs by screen: `current-password` on sign-in lets a
/// manager fill it, `new-password` on setup and invite stops it offering an
/// existing one for a brand-new account.
///
/// `min` is the site's own minimum length on a screen that sets a password,
/// and `None` on the sign-in screen, which must not advertise a length at all
/// — a `minlength` there would refuse to submit an existing shorter password
/// and so lock out exactly the account that needs to sign in and change it.
pub(crate) fn password_field(label: &str, autocomplete: &str, min: Option<usize>) -> String {
    format!(
        r#"<div class="field auth-pw"><input name="password" type="password" placeholder=" " required autocomplete="{autocomplete}"{min}><span>{label}</span>
<button type="button" class="auth-reveal" aria-pressed="false" aria-label="Show password" title="Show password">{eye}{hide}</button></div>"#,
        autocomplete = esc(autocomplete),
        // The server enforces this too; the attribute only moves the
        // complaint to before the round trip.
        min = min.map(|n| format!(r#" minlength="{n}""#)).unwrap_or_default(),
        label = esc(label),
        eye = svg_class("eye", "on"),
        hide = svg_class("hide", "off"),
    )
}

/// The reveal button's script, cache-busted by its own content.
pub(crate) fn auth_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/auth.js")));
    &HASH
}

// ---------------------------------------------------------------------------
// index / setup / login
// ---------------------------------------------------------------------------

/// The admin landing handler, routed inside this crate at the nest root and
/// re-exported so the app can also bind it at `/ghost/`.
///
/// `Router::nest("/ghost", …)` maps an inner `/` route to `/ghost` only, not to
/// `/ghost/`, so without that second binding the canonical admin URL falls
/// through to the site's catch-all — which bounces every `/ghost`-prefixed
/// path straight back to `/ghost/` and loops.
///
/// A session lands on [`ADMIN_HOME`], the dashboard, rather than on the posts
/// list: the first screen of a session should say what is waiting, and the
/// posts list is one of the things it says it about. Every other way in
/// redirects to the same constant, so "home" is one screen however you arrive.
pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/setup/").into_response();
    }
    match require_session(&state, &headers).await {
        Ok(_) => Redirect::to(ADMIN_HOME).into_response(),
        Err(r) => r.into_response(),
    }
}

pub(crate) async fn setup_page(State(state): State<AppState>) -> Response {
    if repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let title = state.settings.get_str("title").unwrap_or_else(|| vuuut_core::DEFAULT_SITE_TITLE.into());
    // The site's own minimum, so a site that has raised it on the Security
    // screen does not have a setup form still promising ten.
    let min_len = vuuut_api::security_settings::min_password_length(&state);
    let min_label = format!("Password · min {min_len} characters");
    page_u(
        "Setup",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Create your site</h1>
<p class="sub">Set up the owner account for {title}</p>
<div class="card"><form method="post" action="/ghost/setup/">
<label class="field"><input name="blog_title" value="{title_v}" placeholder=" " required><span>Site title</span></label>
<label class="field"><input name="name" placeholder=" " required autocomplete="name"><span>Your name</span></label>
<label class="field"><input name="email" type="email" placeholder=" " required autocomplete="email" autocapitalize="none" spellcheck="false"><span>Email</span></label>
{password}
<button class="btn" type="submit">Create account</button>
</form></div>
</div></div>
<script src="{assets}/auth.js?v={auth_hash}" defer></script>"#,
            logo = BRAND_GLYPH,
            title = esc(&title),
            title_v = esc(&title),
            password = password_field(&min_label, "new-password", Some(min_len)),
            assets = ASSETS,
            auth_hash = auth_script_hash(),
        ),
        false,
    )
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct SetupForm {
    name: String,
    email: String,
    password: String,
    blog_title: String,
}

pub(crate) async fn setup_submit(State(state): State<AppState>, Form(form): Form<SetupForm>) -> Response {
    if repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    // Checked before hashing, not after: bcrypt at cost 12 is deliberately
    // expensive, and spending it on a password that is about to be refused
    // made a rejected setup form as slow as an accepted one.
    //
    // The minimum is the site's own, so raising it on the Security screen
    // raises it here too — see `vuuut_api::security_settings`.
    if form.password.chars().count() < vuuut_api::security_settings::min_password_length(&state) {
        return page_u(
            "Setup",
            "",
            None,
            &format!(
                "<div class='banner error'>{}</div>",
                esc(&vuuut_api::security_settings::password_too_short(&state)),
            ),
            false,
        )
        .into_response();
    }
    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return page_u("Setup", "", None, "<div class='banner error'>Password hashing failed</div>", false).into_response(),
    };
    match repos::users::setup_owner(&state.db, &form.name, &form.email, &hash).await {
        Ok(_) => {
            let seeded_title = state.settings.get_str("title").unwrap_or_default();
            let _ = state.settings.set(&state.db, "title", &serde_json::json!(form.blog_title)).await;
            // Only here: the seeded newsletter takes the site's real name at
            // setup and keeps whatever it is called after that.
            let _ = repos::newsletters::name_after_site(&state.db, &seeded_title, &form.blog_title).await;
            let _ = sqlx::query("UPDATE posts SET title = ? WHERE slug = 'coming-soon'").bind(&form.blog_title).execute(&state.db).await;
            state.refresh_setup_flag().await;
            Redirect::to("/ghost/signin/").into_response()
        }
        Err(e) => page_u("Setup", "", None, &format!("<div class='banner error'>{}</div>", esc(&e.message())), false).into_response(),
    }
}

pub(crate) async fn login_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if !repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/setup/").into_response();
    }
    if current_user(&state, &headers).await.is_some() {
        return Redirect::to(ADMIN_HOME).into_response();
    }
    let err = if q.contains_key("throttled") {
        // Deliberately does not say how long: naming the wait tells an
        // attacker exactly when to resume, and a person who is locked out
        // needs to know to stop rather than to schedule.
        auth_error("Too many sign-in attempts. Wait a while before trying again, or reset your password.")
    } else if q.contains_key("invalid") {
        // One message for both halves. Saying which of the two was wrong tells
        // an attacker which addresses have accounts.
        auth_error("That email and password do not match an account.")
    } else {
        String::new()
    };
    page_u(
        "Sign in",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>{title}</h1>
<p class="sub">Sign in to the admin panel</p>
<div class="card"><form method="post" action="/ghost/signin/">{err}
<label class="field"><input name="email" type="email" placeholder=" " required autofocus autocomplete="username" autocapitalize="none" spellcheck="false"><span>Email</span></label>
{password}
<button class="btn" type="submit">Sign in</button>
</form></div>
<div class="auth-foot">
<a href="/ghost/forgot-password/">Forgot your password?</a>
<a href="/">&larr; Back to {title}</a>
</div>
</div></div>
<script src="{assets}/auth.js?v={auth_hash}" defer></script>"#,
            logo = BRAND_GLYPH,
            title = esc(&state.settings.get_str("title").unwrap_or_else(|| vuuut_core::DEFAULT_SITE_TITLE.into())),
            err = err,
            password = password_field("Password", "current-password", None),
            assets = ASSETS,
            auth_hash = auth_script_hash(),
        ),
        false,
    )
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct LoginForm {
    email: String,
    password: String,
}

/// Attach a freshly minted session cookie to a response.
///
/// `Secure` follows the site's own scheme — see `admin::cookie_secure`. An
/// admin session sent over cleartext is the whole site, so the flag matters
/// here more than anywhere else the admin sets a cookie.
pub(crate) fn set_session_cookie(state: &AppState, resp: &mut Response, sid: &str) {
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{name}={sid}; Path={}; HttpOnly; SameSite=Lax; Max-Age={}{}",
            state.admin_path(),
            60 * 60 * 24 * 7,
            vuuut_api::admin::cookie_secure(state),
            name = vuuut_api::admin::SESSION_COOKIE,
        )
        .parse()
        .unwrap(),
    );
}

pub(crate) async fn login_submit(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    // The HTML sign-in form is a second door onto the same password check as
    // `POST /session`, so it needs the same budget — otherwise rate-limiting
    // the API endpoint just moves the guessing here. The key is shared with
    // that endpoint deliberately: it is one allowance per address, not one
    // per door.
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    let login_key = format!("user_login:{ip}");
    if vuuut_api::brute::check(&state.db, &login_key, &vuuut_api::brute::USER_LOGIN).await.is_err() {
        return Redirect::to("/ghost/signin/?throttled=1").into_response();
    }
    let Ok(Some(user)) = repos::users::verify_password(&state.db, &form.email, &form.password).await
    else {
        return Redirect::to("/ghost/signin/?invalid=1").into_response();
    };
    vuuut_api::brute::reset(&state.db, &login_key).await;

    // With 2FA off the password is the whole login, as before. A first-ever
    // sign-in also goes straight through: the person only has a password
    // because they followed an emailed invite or setup link, which is the
    // same proof a code would ask for. Ghost's session endpoint takes this
    // shortcut too (`skipVerification` when `!user.hasLoggedIn()`).
    if !state.settings.get_bool("require_email_mfa") || user.last_seen.is_none() {
        return match repos::sessions::create(&state.db, &user.id).await {
            Ok(sid) => {
                let mut resp = Redirect::to(ADMIN_HOME).into_response();
                set_session_cookie(&state, &mut resp, &sid);
                resp
            }
            Err(_) => Redirect::to("/ghost/signin/?invalid=1").into_response(),
        };
    }

    // With it on, the password only names the user: the session is created
    // unverified, so none of the admin guards will resolve it until the code
    // that just went out by email comes back.
    let Ok(sid) = repos::sessions::create_unverified(&state.db, &user.id).await else {
        return Redirect::to("/ghost/signin/?invalid=1").into_response();
    };
    let sent = issue_signin_code(&state, &sid, &user).await;
    let mut resp = Redirect::to(if sent { "/ghost/signin/verify/" } else { "/ghost/signin/verify/?send=failed" })
        .into_response();
    set_session_cookie(&state, &mut resp, &sid);
    resp
}

/// Generate a sign-in code, store its digest on the session and email it.
/// Returns whether the mail actually went out.
pub(crate) async fn issue_signin_code(state: &AppState, sid: &str, user: &vuuut_core::models::User) -> bool {
    let Some(email) = user.email.clone() else {
        tracing::error!("user {} has no email address to send a sign-in code to", user.id);
        return false;
    };
    // Six digits, zero-padded, off the same CSPRNG that mints session ids —
    // the shape of code Ghost emails.
    let code = vuuut_core::ids::random_code(6);
    if repos::sessions::set_auth_code(&state.db, sid, &code).await.is_err() {
        return false;
    }

    let title = state.settings.get_str("title").unwrap_or_else(|| vuuut_core::DEFAULT_SITE_TITLE.into());
    let site_url = state.site_url();
    let cfg = vuuut_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let msg = vuuut_mail::Message {
        to: email.clone(),
        subject: format!("{code} is your sign-in verification code"),
        html: format!(
            r#"<div style="font-family:-apple-system,Segoe UI,sans-serif;font-size:16px;line-height:1.6;color:#15171a">
<p>Hey{name},</p>
<p>Here is the verification code you need to sign in to <strong>{title}</strong>:</p>
<p style="font-size:30px;font-weight:700;letter-spacing:6px;margin:28px 0">{code}</p>
<p style="color:#626d79;font-size:14px">The code is good for five minutes. If you didn't try to sign in, you can ignore this email — someone knows your password, so it is worth changing it.</p>
</div>"#,
            name = user.name.as_deref().filter(|n| !n.is_empty()).map(|n| format!(" {}", esc(n))).unwrap_or_default(),
            title = esc(&title),
            code = code,
        ),
        text: format!(
            "Your verification code for {title} is {code}.\n\nIt is good for five minutes. \
             If you didn't try to sign in, someone knows your password and it is worth changing."
        ),
    };
    match vuuut_mail::send(&cfg, &msg).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("could not send the sign-in code to {}: {}", email, e.message());
            false
        }
    }
}

/// The session id on the request, verified or not.
///
/// Delegates rather than re-reading the header, so there is exactly one piece
/// of code that knows what the admin session cookie is called. This used to
/// spell the name out, and a rename had to find all eight copies.
pub(crate) fn session_id_from(headers: &HeaderMap) -> Option<String> {
    vuuut_api::admin::session_id_from(headers)
}

/// The code-entry screen, shown between password and session when email 2FA
/// is on. Reached with an unverified session cookie already set.
pub(crate) async fn verify_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    // A verified session has nothing to do here; an unknown one has to start over.
    match repos::sessions::is_unverified(&state.db, &sid).await {
        Ok(true) => {}
        Ok(false) => return Redirect::to(ADMIN_HOME).into_response(),
        Err(_) => return Redirect::to("/ghost/signin/").into_response(),
    }

    // Announced, not just drawn: the page has reloaded and this sentence is
    // the only thing that changed, which a sighted person sees and nobody
    // else would. Errors are `alert`; the confirmation is a quieter `status`,
    // because it does not interrupt anything.
    let banner = match q.get("send").map(String::as_str) {
        Some("failed") => {
            auth_error("Could not send the code — check the mail settings, or ask another administrator.")
        }
        Some("resent") => format!(
            r#"<div class="banner ok" role="status">{icon}<span class="msg">A new code is on its way.</span></div>"#,
            icon = svg("check"),
        ),
        _ => match q.get("error").map(String::as_str) {
            Some("incorrect") => auth_error("That code is not right. Check the email and try again."),
            Some("expired") => auth_error("That code has expired. Send yourself a new one."),
            // As on the sign-in screen, deliberately silent about how long:
            // naming the wait tells a guesser exactly when to resume.
            Some("throttled") => auth_error(
                "Too many attempts. Wait a while before trying again, or start signing in afresh.",
            ),
            _ => String::new(),
        },
    };

    page_u(
        "Verify sign-in",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Check your email</h1>
<p class="sub">We sent a six-digit code to your inbox. Enter it to finish signing in.</p>
<div class="card"><form method="post" action="/ghost/signin/verify/">{banner}
<label class="field"><input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]*" maxlength="6" placeholder=" " required autofocus><span>Verification code</span></label>
<button class="btn" type="submit">Verify</button>
</form>
<form method="post" action="/ghost/signin/resend/" style="margin-top:10px">
<button class="btn text" type="submit">Send a new code</button>
</form>
<form method="post" action="/ghost/signout/" style="margin-top:2px">
<button class="btn text" type="submit">Sign in as someone else</button>
</form></div>
</div></div>"#,
            logo = BRAND_GLYPH,
            banner = banner,
        ),
        false,
    )
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct VerifyForm {
    code: String,
}

/// Finish a sign-in with the emailed code.
///
/// Rate-limited per address against `USER_VERIFICATION`, on the same argument
/// [`login_submit`] makes about the password: this HTML form is a second door
/// onto the same check as `PUT /session/verify`, so budgeting only the JSON
/// endpoint moves the guessing here rather than stopping it. The key is
/// shared with that endpoint deliberately — one allowance per address, not
/// one per door.
///
/// The per-session five-try cap in
/// [`vuuut_db::repos::sessions::verify_auth_code`] is not enough on its own:
/// it expires the code after five wrong guesses, and the resend button issues
/// a fresh one, so five-then-resend repeated without a budget is an unbounded
/// search of a six-digit space.
pub(crate) async fn verify_submit(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<VerifyForm>,
) -> Response {
    use vuuut_db::repos::sessions::CodeCheck;
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    let verify_key = format!("user_verification:{ip}");
    if vuuut_api::brute::check(&state.db, &verify_key, &vuuut_api::brute::USER_VERIFICATION)
        .await
        .is_err()
    {
        return Redirect::to("/ghost/signin/verify/?error=throttled").into_response();
    }
    match repos::sessions::verify_auth_code(&state.db, &sid, &form.code).await {
        Ok(CodeCheck::Verified) => {
            // The right code is not a guess, so it does not count against the
            // budget: somebody who mistyped twice first is not then locked
            // out of their own admin.
            vuuut_api::brute::reset(&state.db, &verify_key).await;
            if let Ok(Some(user_id)) = repos::sessions::get_user(&state.db, &sid).await {
                let _ = repos::users::update_last_seen(&state.db, &user_id).await;
            }
            Redirect::to(ADMIN_HOME).into_response()
        }
        Ok(CodeCheck::Incorrect) => Redirect::to("/ghost/signin/verify/?error=incorrect").into_response(),
        _ => Redirect::to("/ghost/signin/verify/?error=expired").into_response(),
    }
}

/// Issue a replacement code for a session still waiting on one.
///
/// Budgeted per address against `SEND_VERIFICATION_CODE`, sharing its key
/// with `POST /session/verify`. Two reasons, and the second is the one that
/// matters: unbudgeted, this is a button that posts a stream of mail to a
/// staff address, *and* it is the refill behind the code-guessing described
/// on [`verify_submit`] — the five-try cap on a code means nothing if a sixth
/// try is one click away.
pub(crate) async fn verify_resend(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
) -> Response {
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(
        &state.db,
        &format!("send_verification_code:{ip}"),
        &vuuut_api::brute::SEND_VERIFICATION_CODE,
    )
    .await
    .is_err()
    {
        return Redirect::to("/ghost/signin/verify/?error=throttled").into_response();
    }
    if !repos::sessions::is_unverified(&state.db, &sid).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let Ok(Some(user_id)) = repos::sessions::get_pending_user(&state.db, &sid).await else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    let Ok(Some(user)) = repos::users::find_by_id(&state.db, &user_id).await else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    let sent = issue_signin_code(&state, &sid, &user).await;
    Redirect::to(if sent { "/ghost/signin/verify/?send=resent" } else { "/ghost/signin/verify/?send=failed" })
        .into_response()
}

pub(crate) async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(sid) = session_id_from(&headers) {
        let _ = repos::sessions::destroy(&state.db, &sid).await;
    }
    let mut resp = Redirect::to("/ghost/signin/").into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        // Name, path and `Secure` must match the cookie that was set or the
        // browser keeps the original alongside this one and the sign-out
        // does nothing.
        format!(
            "{name}=; Path={}; HttpOnly; SameSite=Lax; Max-Age=0{}",
            state.admin_path(),
            vuuut_api::admin::cookie_secure(&state),
            name = vuuut_api::admin::SESSION_COOKIE,
        )
        .parse()
        .unwrap(),
    );
    resp
}

/// Saves the Appearance choice from the rail — Ghost's night shift. Every
/// signed-in staff user carries their own, like Ghost's, so this needs a
/// session but no role above the entry one. The JSON body is `{"theme":
/// "light" | "dark" | "system"}`; anything else is a 400 rather than being
/// guessed at, because a bad value would sit in the JSON silently colouring
/// every future page wrong.
pub(crate) async fn appearance_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(theme) = body["theme"].as_str().filter(|t| ["light", "dark", "system"].contains(t)) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"errors":[{"message":"theme must be light, dark or system","errorType":"ValidationError"}]})),
        )
            .into_response();
    };
    let merged = merge_night_shift(actor.user.accessibility.as_deref(), theme);
    match repos::users::set_accessibility(&state.db, &actor.user.id, &merged).await {
        Ok(()) => (StatusCode::OK, axum::Json(serde_json::json!({"theme": theme}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"errors":[{"message": e.to_string(), "errorType": "InternalServerError"}]})),
        )
            .into_response(),
    }
}

/// Ask for a reset link, or set a new password when arriving with a token.
///
/// The form itself is built by `publishing-workflows.js`, which reads the
/// `?token=` in the URL to decide whether to ask for an email address or for
/// a new password, and posts to the password-reset endpoint. The ids
/// `password-recovery`, `recovery-fields` and `recovery-message` are that
/// script's contract and must not be renamed here.
///
/// Because the whole form is script-built, there is a `<noscript>`: without
/// it this page is an empty card with a Continue button that does nothing,
/// which looks broken rather than unavailable.
pub(crate) async fn forgot_password_page() -> Response {
    page_u(
        "Reset password",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Reset your password</h1>
<p class="sub">We will email you a link to set a new one.</p>
<div class="card">
<noscript><div class="banner error" role="alert">{warn}<span class="msg">Resetting a password needs JavaScript. Turn it on and reload, or ask another administrator to send you a new invitation.</span></div></noscript>
<form id="password-recovery"><div id="recovery-fields"></div>
<button class="btn" type="submit">Continue</button>
<p role="status" id="recovery-message" class="auth-status"></p></form>
</div>
<div class="auth-foot"><a href="/ghost/signin/">&larr; Back to sign in</a></div>
</div></div>"#,
            logo = BRAND_GLYPH,
            warn = svg("warning"),
        ),
        false,
    )
    .into_response()
}
