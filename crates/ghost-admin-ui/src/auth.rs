//! Getting in: the setup wizard, sign-in, the emailed code, and signing out.

use super::*;

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
pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/setup/").into_response();
    }
    match require_session(&state, &headers).await {
        Ok(_) => Redirect::to("/ghost/posts/").into_response(),
        Err(r) => r.into_response(),
    }
}

pub(crate) async fn setup_page(State(state): State<AppState>) -> Response {
    if repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
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
<label class="field"><input name="email" type="email" placeholder=" " required autocomplete="email"><span>Email</span></label>
<label class="field"><input name="password" type="password" minlength="10" required autocomplete="new-password"><span>Password · min 10 characters</span></label>
<button class="btn" type="submit">Create account</button>
</form></div>
</div></div>"#,
            logo = BRAND_GLYPH,
            title = esc(&title),
            title_v = esc(&title),
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
    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return page_u("Setup", "", None, "<div class='banner error'>Password hashing failed</div>", false).into_response(),
    };
    if form.password.len() < 10 {
        return page_u(
            "Setup",
            "",
            None,
            "<div class='banner error'>Password must be at least 10 characters.</div>",
            false,
        )
        .into_response();
    }
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
        return Redirect::to("/ghost/posts/").into_response();
    }
    let err = if q.contains_key("throttled") {
        // Deliberately does not say how long: naming the wait tells an
        // attacker exactly when to resume, and a person who is locked out
        // needs to know to stop rather than to schedule.
        r#"<div class="banner error"><svg class="i" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="10"/></svg>Too many sign-in attempts. Wait a while before trying again, or reset your password.</div>"#
    } else if q.contains_key("invalid") {
        r#"<div class="banner error"><svg class="i" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="10"/></svg>Invalid email or password</div>"#
    } else {
        ""
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
<label class="field"><input name="email" type="email" placeholder=" " required autofocus autocomplete="username"><span>Email</span></label>
<label class="field"><input name="password" type="password" placeholder=" " required autocomplete="current-password"><span>Password</span></label>
<button class="btn" type="submit">Sign in</button>
<a class="btn text" href="/ghost/forgot-password/">Forgot password?</a>
</form></div>
</div></div>"#,
            logo = BRAND_GLYPH,
            title = esc(&state.settings.get_str("title").unwrap_or_else(|| "Ghost".into())),
            err = err
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
            "ghost-admin-api-session={sid}; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age={}{}",
            ghost_core::url::subdir(),
            60 * 60 * 24 * 7,
            ghost_api::admin::cookie_secure(state)
        )
        .parse()
        .unwrap(),
    );
}

pub(crate) async fn login_submit(
    State(state): State<AppState>,
    ghost_api::brute::ClientAddr(peer): ghost_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    // The HTML sign-in form is a second door onto the same password check as
    // `POST /session`, so it needs the same budget — otherwise rate-limiting
    // the API endpoint just moves the guessing here. The key is shared with
    // that endpoint deliberately: it is one allowance per address, not one
    // per door.
    let ip = ghost_api::brute::client_ip(&headers, peer);
    let login_key = format!("user_login:{ip}");
    if ghost_api::brute::check(&state.db, &login_key, &ghost_api::brute::USER_LOGIN).await.is_err() {
        return Redirect::to("/ghost/signin/?throttled=1").into_response();
    }
    let Ok(Some(user)) = repos::users::verify_password(&state.db, &form.email, &form.password).await
    else {
        return Redirect::to("/ghost/signin/?invalid=1").into_response();
    };
    ghost_api::brute::reset(&state.db, &login_key).await;

    // With 2FA off the password is the whole login, as before. A first-ever
    // sign-in also goes straight through: the person only has a password
    // because they followed an emailed invite or setup link, which is the
    // same proof a code would ask for. Ghost's session endpoint takes this
    // shortcut too (`skipVerification` when `!user.hasLoggedIn()`).
    if !state.settings.get_bool("require_email_mfa") || user.last_seen.is_none() {
        return match repos::sessions::create(&state.db, &user.id).await {
            Ok(sid) => {
                let mut resp = Redirect::to("/ghost/posts/").into_response();
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
pub(crate) async fn issue_signin_code(state: &AppState, sid: &str, user: &ghost_core::models::User) -> bool {
    let Some(email) = user.email.clone() else {
        tracing::error!("user {} has no email address to send a sign-in code to", user.id);
        return false;
    };
    // Six digits, zero-padded, off the same CSPRNG that mints session ids —
    // the shape of code Ghost emails.
    let code = ghost_core::ids::random_code(6);
    if repos::sessions::set_auth_code(&state.db, sid, &code).await.is_err() {
        return false;
    }

    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_url = state.site_url();
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let msg = ghost_mail::Message {
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
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("could not send the sign-in code to {}: {}", email, e.message());
            false
        }
    }
}

/// The session id on the request, verified or not.
pub(crate) fn session_id_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())?
        .split(';')
        .find_map(|c| c.trim().strip_prefix("ghost-admin-api-session="))
        .map(str::to_string)
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
        Ok(false) => return Redirect::to("/ghost/posts/").into_response(),
        Err(_) => return Redirect::to("/ghost/signin/").into_response(),
    }

    let banner = match q.get("send").map(String::as_str) {
        Some("failed") => r#"<div class="banner error">Could not send the code — check the mail settings, or ask another administrator.</div>"#,
        Some("resent") => r#"<div class="banner ok">A new code is on its way.</div>"#,
        _ => match q.get("error").map(String::as_str) {
            Some("incorrect") => r#"<div class="banner error">That code is not right. Check the email and try again.</div>"#,
            Some("expired") => r#"<div class="banner error">That code has expired. Send yourself a new one.</div>"#,
            _ => "",
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

pub(crate) async fn verify_submit(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<VerifyForm>) -> Response {
    use ghost_db::repos::sessions::CodeCheck;
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    match repos::sessions::verify_auth_code(&state.db, &sid, &form.code).await {
        Ok(CodeCheck::Verified) => {
            if let Ok(Some(user_id)) = repos::sessions::get_user(&state.db, &sid).await {
                let _ = repos::users::update_last_seen(&state.db, &user_id).await;
            }
            Redirect::to("/ghost/posts/").into_response()
        }
        Ok(CodeCheck::Incorrect) => Redirect::to("/ghost/signin/verify/?error=incorrect").into_response(),
        _ => Redirect::to("/ghost/signin/verify/?error=expired").into_response(),
    }
}

/// Issue a replacement code for a session still waiting on one.
pub(crate) async fn verify_resend(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
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
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
        if let Some(sid) = cookie.split(';').find_map(|c| c.trim().strip_prefix("ghost-admin-api-session=")) {
            let _ = repos::sessions::destroy(&state.db, sid).await;
        }
    }
    let mut resp = Redirect::to("/ghost/signin/").into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!("ghost-admin-api-session=; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age=0{}", ghost_core::url::subdir(), ghost_api::admin::cookie_secure(&state)).parse().unwrap(),
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

pub(crate) async fn forgot_password_page() -> Response {
    page_u("Reset password", "", None, r#"<div class="auth-box"><h1>Reset password</h1><div class="card padded"><form id="password-recovery"><div id="recovery-fields"></div><button class="btn" type="submit">Continue</button><p role="status" id="recovery-message"></p></form><a href="/ghost/signin/">Back to sign in</a></div></div>"#,false).into_response()
}
