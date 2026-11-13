//! Settings -> Staff: the team list, invitations, roles, and one person's own profile.

use super::*;

// ---------------------------------------------------------------------------
// staff profile (Ghost's Settings → Staff → <user>, which the account block
// at the foot of the rail opens for the signed-in user)
// ---------------------------------------------------------------------------

// --- Settings → Staff: the team list, role changes and invitations ---

pub(crate) fn staff_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/staff/?{encoded}")).into_response()
}

/// Whether `user` may administer other staff. Ghost allows Owner and
/// Administrator; everyone else can edit only their own profile.
pub(crate) async fn can_manage_staff(state: &AppState, user: &ghost_core::models::User) -> bool {
    role_of(state, user).await.can_manage_staff()
}

pub(crate) async fn role_of(state: &AppState, user: &ghost_core::models::User) -> Role {
    let name = repos::users::role_name(&state.db, &user.id).await.ok().flatten().unwrap_or_default();
    Role::from_name(&name)
}

/// Which roles `actor` may hand out.
///
/// Ghost lets Editors invite and manage Authors and Contributors, but not
/// other Editors or Administrators — an Editor must not be able to promote
/// themselves a peer with settings access, or grant a role above their own.
pub(crate) fn assignable_by(actor: Role) -> Vec<&'static (&'static str, &'static str)> {
    repos::staff::ASSIGNABLE_ROLES
        .iter()
        .filter(|(name, _)| {
            let r = Role::from_name(name);
            if actor.is_admin() { r <= Role::Administrator } else { r < Role::Editor }
        })
        .collect()
}

/// Whether `actor` may act on a staff member currently holding `target`.
pub(crate) fn may_act_on(actor: Role, target: Role) -> bool {
    if target == Role::Owner {
        return false;
    }
    if actor.is_admin() {
        return true;
    }
    // Editors reach only below themselves.
    actor.can_manage_staff() && target < Role::Editor
}

/// `<option>`s for the role picker, with each role's description as a
/// tooltip so the difference between them is discoverable.
pub(crate) fn role_options_for(actor: Role, selected: &str) -> String {
    assignable_by(actor)
        .into_iter()
        .map(|(name, desc)| {
            format!(
                r#"<option value="{n}"{sel} title="{d}">{n}</option>"#,
                n = esc(name),
                d = esc(desc),
                sel = if *name == selected { " selected" } else { "" },
            )
        })
        .collect()
}

pub(crate) async fn staff_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    let manage = actor_role.can_manage_staff();
    let staff = repos::staff::list_staff(&state.db).await.unwrap_or_default();
    let invites = if manage { repos::staff::list_invites(&state.db).await.unwrap_or_default() } else { vec![] };

    let rows: String = staff
        .iter()
        .map(|m| {
            let initial = m.name.as_deref().unwrap_or("?").chars().next().unwrap_or('?').to_uppercase().to_string();
            let suspended = m.status.as_deref() == Some("inactive");
            let is_owner = m.role == "Owner";
            let is_me = m.id == user.id;
            // You can open your own profile, or anyone's if you manage staff.
            let openable = manage || is_me;
            let body = format!(
                r#"<span class="title">{name}{you}</span><span class="detail">{email}</span>"#,
                name = esc(m.name.as_deref().unwrap_or("")),
                you = if is_me { r#" <span class="muted">· you</span>"# } else { "" },
                email = esc(m.email.as_deref().unwrap_or("")),
            );
            let body = if openable {
                format!(
                    r#"<a class="body" href="/ghost/settings/staff/{slug}/">{body}</a>"#,
                    slug = esc(m.slug.as_deref().unwrap_or("")),
                    body = body,
                )
            } else {
                format!(r#"<div class="body">{body}</div>"#)
            };
            format!(
                r#"<div class="row" data-filterable><span class="avatar">{initial}</span>{body}
<div class="trailing">{suspended_chip}<span class="chip {chip}">{role}</span>{chev}</div></div>"#,
                initial = esc(&initial),
                body = body,
                suspended_chip = if suspended { r#"<span class="chip draft">suspended</span>"# } else { "" },
                chip = if is_owner { "published" } else { "tonal" },
                role = esc(&m.role),
                chev = if openable { svg("chevron") } else { String::new() },
            )
        })
        .collect();

    let invite_rows: String = invites
        .iter()
        .map(|i| {
            let expired = i.is_expired();
            format!(
                r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{email}</span><span class="detail">Invited as {role} · {state_label}</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/staff/invite/{id}/resend/"><button class="btn tonal small" type="submit">Resend</button></form>
<form method="post" action="/ghost/settings/staff/invite/{id}/revoke/" data-confirm="Revoke invitation|The link emailed to {email} will stop working.">
<button class="icon-btn danger" title="Revoke" aria-label="Revoke">{del}</button></form></div></div>"#,
                ic = svg("mail"),
                email = esc(&i.email),
                role = esc(&i.role),
                state_label = if expired { "expired".to_string() } else { format!("{} · expires in a week", i.status) },
                id = esc(&i.id),
                del = svg("delete"),
            )
        })
        .collect();

    let invite_card = if manage {
        format!(
            r#"<div class="section">Invite people</div>
<div class="card padded rise" style="--i:2">
<form method="post" action="/ghost/settings/staff/invite/" class="inline-form">
<label class="field" style="flex:1;min-width:220px;margin:0"><input name="email" type="email" placeholder=" " required><span>Email address</span></label>
<label class="field fixed" style="min-width:190px;margin:0"><select name="role">{roles}</select><span>Role</span>{car}</label>
<button class="btn tonal" type="submit">{add} Send invitation</button>
</form>
<p class="muted" style="margin:12px 0 0">{roles_help}</p>
</div>
{invite_list}"#,
            roles = role_options_for(actor_role, "Contributor"),
            car = SELECT_CARET,
            add = svg("mail"),
            roles_help = repos::staff::ASSIGNABLE_ROLES
                .iter()
                .map(|(n, d)| format!("<strong>{}</strong> — {}", esc(n), esc(d)))
                .collect::<Vec<_>>()
                .join("<br>"),
            invite_list = if invites.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<div class="section">Pending invitations</div><div class="card rows rise" style="--i:3">{invite_rows}</div>"#
                )
            },
        )
    } else {
        format!(
            r#"<div class="banner info rise" style="--i:2">{ic}<span class="msg">Only the owner and administrators can invite people or change roles.</span></div>"#,
            ic = svg("ghost"),
        )
    };

    // Only the owner and administrators can change how everyone else signs in.
    let security_card = if manage {
        format!(
            r#"<div class="section">Security settings</div>
<div class="card rows rise" style="--i:4">
<form method="post" action="/ghost/settings/staff/security/" id="staff-security">
<div class="row"><span class="ic tint-amber">{lock}</span>
<div class="body"><span class="title">Require email 2FA codes on staff logins</span>
<span class="detail">Require email 2FA codes to be used on all staff logins</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="require_email_mfa" value="1" {checked} onchange="this.form.requestSubmit()"><span class="track"><span class="thumb"></span></span></label></div></div>
</form></div>
<p class="muted" style="margin:10px 2px 0">With this on, a correct password is not enough: everyone gets a six-digit
code by email and has five minutes to enter it. Sessions already signed in stay signed in.</p>"#,
            lock = svg("lock"),
            checked = if state.settings.get_bool("require_email_mfa") { "checked" } else { "" },
        )
    } else {
        String::new()
    };

    page_for(
        "Staff",
        "settings",
        Some(&user),
        actor_role,
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Staff</h1><p class="sub">{count} {word}</p></div></header>
{banner}
<div class="section">Active people</div>
<div class="card rows rise" style="--i:1">{rows}</div>
{invite_card}
{security_card}"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            count = staff.len(),
            word = if staff.len() == 1 { "person" } else { "people" },
            rows = rows,
            invite_card = invite_card,
            security_card = security_card,
        ),
        true,
    )
    .into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct InviteForm {
    email: String,
    role: String,
}

/// Build the invitation link and email it. Returns the link so the caller can
/// surface it when there is no mail transport to carry it.
pub(crate) async fn send_invite_email(
    state: &AppState,
    invite: &repos::staff::Invite,
    inviter: &ghost_core::models::User,
) -> (String, Option<String>) {
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let link = format!("{site_url}/ghost/invite/{}/", invite.token);
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let (subject, html, text) = ghost_mail::templates::staff_invite(
        &title,
        &site_url,
        inviter.name.as_deref().unwrap_or("A colleague"),
        &invite.role,
        &link,
    );
    let msg = ghost_mail::Message { to: invite.email.clone(), subject, html, text };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => {
            let _ = repos::staff::mark_invite_sent(&state.db, &invite.id).await;
            (link, None)
        }
        Err(e) => (link, Some(e.message())),
    }
}

/// Turn the email-2FA requirement on or off.
///
/// Administrators only, like the rest of the staff page's controls — this
/// decides how every other person on the site signs in.
pub(crate) async fn staff_security_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = form.get("require_email_mfa").map(|v| v == "1").unwrap_or(false);
    let saved = state
        .settings
        .set(&state.db, "require_email_mfa", &serde_json::Value::Bool(on))
        .await
        .is_ok();
    let message = match (saved, on) {
        (false, _) => "Saving failed — check the logs.",
        (true, true) => "Staff logins now require an emailed 2FA code.",
        (true, false) => "Staff logins no longer require a 2FA code.",
    };
    let kind = if saved { "ok" } else { "error" };
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("/ghost/settings/staff/?{encoded}")).into_response()
}

pub(crate) async fn staff_invite(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<InviteForm>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return staff_flash("error", "Only editors and administrators can invite people.");
    }
    // An Editor must not be able to invite a peer or an administrator —
    // that would be a route to the settings they cannot reach themselves.
    if !assignable_by(actor_role).iter().any(|(n, _)| *n == form.role) {
        return staff_flash("error", &format!("As {}, you cannot invite someone as {}.", actor_role.label(), form.role));
    }
    let email = form.email.trim().to_lowercase();
    if !email.contains('@') {
        return staff_flash("error", "Enter a valid email address.");
    }
    // The host's staff ceiling, which counts pending invites as well as
    // users. Contributors are exempt, as in Ghost.
    if form.role != "Contributor" {
        if let Err(e) = state
            .limits
            .error_if_would_go_over(&state.db, "staff", ghost_db::limits::Check::default())
            .await
        {
            return staff_flash("error", &e.message());
        }
    }
    let invite = match repos::staff::create_invite(&state.db, &email, &form.role).await {
        Ok(i) => i,
        Err(e) => return staff_flash("error", &e.message()),
    };
    match send_invite_email(&state, &invite, &user).await {
        (_, None) => staff_flash("ok", &format!("Invitation sent to {email}.")),
        // The invite is valid either way, so say so and hand over the link
        // rather than leaving the inviter thinking nothing happened.
        (link, Some(err)) => {
            tracing::warn!("staff: invite email to {email} failed: {err}");
            staff_flash(
                "error",
                &format!("Invitation created, but the email could not be sent ({err}). Share this link instead: {link}"),
            )
        }
    }
}

pub(crate) async fn staff_invite_resend(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if !can_manage_staff(&state, &user).await {
        return staff_flash("error", "Only the owner and administrators can manage invitations.");
    }
    let Some(old) = repos::staff::list_invites(&state.db).await.unwrap_or_default().into_iter().find(|i| i.id == id) else {
        return staff_flash("error", "That invitation no longer exists.");
    };
    // Reissue rather than resend: a fresh token means an old forwarded email
    // cannot still be redeemed.
    let invite = match repos::staff::create_invite(&state.db, &old.email, &old.role).await {
        Ok(i) => i,
        Err(e) => return staff_flash("error", &e.message()),
    };
    match send_invite_email(&state, &invite, &user).await {
        (_, None) => staff_flash("ok", &format!("Invitation resent to {}.", invite.email)),
        (link, Some(err)) => staff_flash(
            "error",
            &format!("Could not send the email ({err}). Share this link instead: {link}"),
        ),
    }
}

pub(crate) async fn staff_invite_revoke(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if !can_manage_staff(&state, &user).await {
        return staff_flash("error", "Only the owner and administrators can manage invitations.");
    }
    match repos::staff::revoke_invite(&state.db, &id).await {
        Ok(()) => staff_flash("ok", "Invitation revoked."),
        Err(e) => staff_flash("error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RoleForm {
    role: String,
}

pub(crate) async fn staff_set_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<RoleForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return profile_flash(&slug, "error", "Only editors and administrators can change roles.");
    }
    let Ok(Some(target)) = repos::users::find_by_slug(&state.db, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let current = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    if current == "Owner" {
        return profile_flash(&slug, "error", "The owner's role cannot be changed.");
    }
    if !may_act_on(actor_role, Role::from_name(&current))
        || !assignable_by(actor_role).iter().any(|(n, _)| *n == form.role)
    {
        return profile_flash(&slug, "error", &format!("As {}, you cannot set that role.", actor_role.label()));
    }
    // Demoting the last administrator would leave nobody able to administer
    // the site, including reversing the change.
    if current == "Administrator"
        && form.role != "Administrator"
        && repos::staff::admin_count(&state.db).await.unwrap_or(0) <= 1
    {
        return profile_flash(&slug, "error", "This is the last administrator — promote someone else first.");
    }
    match repos::staff::set_role(&state.db, &target.id, &form.role).await {
        Ok(()) => profile_flash(&slug, "ok", &format!("Role changed to {}.", form.role)),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct StatusForm {
    status: String,
}

pub(crate) async fn staff_set_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<StatusForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return profile_flash(&slug, "error", "Only editors and administrators can suspend people.");
    }
    let Ok(Some(target)) = repos::users::find_by_slug(&state.db, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    if target.id == user.id {
        return profile_flash(&slug, "error", "You cannot suspend your own account.");
    }
    let role = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    if !may_act_on(actor_role, Role::from_name(&role)) {
        return profile_flash(&slug, "error", "You cannot suspend that person.");
    }
    match repos::staff::set_user_status(&state.db, &target.id, &form.status).await {
        Ok(()) => profile_flash(
            &slug,
            "ok",
            if form.status == "inactive" { "Account suspended." } else { "Account reactivated." },
        ),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

pub(crate) async fn staff_delete(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return profile_flash(&slug, "error", "Only editors and administrators can remove people.");
    }
    let Ok(Some(target)) = repos::users::find_by_slug(&state.db, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    if target.id == user.id {
        return profile_flash(&slug, "error", "You cannot delete your own account.");
    }
    let role = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    if !may_act_on(actor_role, Role::from_name(&role)) {
        return profile_flash(&slug, "error", "You cannot remove that person.");
    }
    match repos::staff::delete_staff(&state.db, &target.id).await {
        Ok(()) => staff_flash("ok", "Staff member removed."),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

// --- redeeming an invitation (signed out) ---

pub(crate) async fn invite_accept_page(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let invite = repos::staff::find_invite_by_token(&state.db, &token).await.ok().flatten();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let body = match &invite {
        Some(i) if !i.is_expired() => format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Join {title}</h1>
<p class="sub">You were invited as {role}. Choose a name and password to finish.</p>
<div class="card"><form method="post" action="/ghost/invite/{token}/">
<label class="field"><input value="{email}" disabled><span>Email</span></label>
<label class="field"><input name="name" placeholder=" " required autocomplete="name" autofocus><span>Your name</span></label>
<label class="field"><input name="password" type="password" minlength="10" required autocomplete="new-password"><span>Password · min 10 characters</span></label>
<button class="btn" type="submit">Create account</button>
</form></div></div></div>"#,
            logo = BRAND_GLYPH,
            title = esc(&title),
            role = esc(&i.role),
            token = esc(&token),
            email = esc(&i.email),
        ),
        _ => format!(
            r#"<div class="auth"><div class="auth-box"><div class="auth-logo">{logo}</div>
<h1>Invitation not valid</h1>
<p class="sub">This invitation has expired or has already been used. Ask whoever invited you to send a new one.</p>
<div class="card"><a class="btn tonal" href="/ghost/signin/">Go to sign in</a></div></div></div>"#,
            logo = BRAND_GLYPH,
        ),
    };
    page_u("Accept invitation", "", None, &body, false).into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct AcceptInviteForm {
    name: String,
    password: String,
}

pub(crate) async fn invite_accept_submit(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Form(form): Form<AcceptInviteForm>,
) -> Response {
    let Some(invite) = repos::staff::find_invite_by_token(&state.db, &token).await.ok().flatten() else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    if invite.is_expired() {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let name = form.name.trim();
    if name.is_empty() || form.password.chars().count() < 10 {
        return Redirect::to(&format!("/ghost/invite/{token}/")).into_response();
    }
    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return Redirect::to(&format!("/ghost/invite/{token}/")).into_response(),
    };
    let created = repos::users::insert_staff(&state.db, name, &invite.email, &hash, &invite.role).await;
    match created {
        Ok(user) => {
            // Single use: the invite is spent whether or not signin succeeds.
            let _ = repos::staff::delete_invite_by_email(&state.db, &invite.email).await;
            match repos::sessions::create(&state.db, &user.id).await {
                Ok(sid) => {
                    let mut resp = Redirect::to("/ghost/posts/").into_response();
                    if let Ok(v) = format!(
                        "ghost-admin-api-session={sid}; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age={}{}",
                        ghost_core::url::subdir(),
                        60 * 60 * 24 * 7,
                        ghost_api::admin::cookie_secure(&state)
                    )
                    .parse()
                    {
                        resp.headers_mut().insert(header::SET_COOKIE, v);
                    }
                    resp
                }
                Err(_) => Redirect::to("/ghost/signin/").into_response(),
            }
        }
        Err(e) => {
            tracing::error!("staff: could not create invited user {}: {}", invite.email, e.message());
            Redirect::to(&format!("/ghost/invite/{token}/")).into_response()
        }
    }
}

/// The editable profile fields, in Ghost's own order on that screen.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct ProfileForm {
    name: String,
    slug: String,
    email: String,
    location: String,
    bio: String,
    profile_image: String,
    cover_image: String,
    meta_title: String,
    meta_description: String,
}

/// The Social Links tab posts the website plus every social handle. All inputs
/// always render, so an empty field means "cleared", never "untouched".
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct SocialForm {
    website: String,
    twitter: String,
    facebook: String,
    linkedin: String,
    bluesky: String,
    threads: String,
    mastodon: String,
    tiktok: String,
    youtube: String,
    instagram: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

/// The staff user page's tabs, keyed by the `?tab=` the seg-bar carries.
/// Ghost's own user screen splits Profile / Social Links / Email Notifications
/// the same way.
pub(crate) const PROFILE_TABS: &[(&str, &str)] = &[
    ("profile", "Profile"),
    ("social-links", "Social Links"),
    ("email-notifications", "Email Notifications"),
];

pub(crate) fn profile_tab(q: &HashMap<String, String>) -> &'static str {
    let tab = q.get("tab").map(String::as_str).unwrap_or("profile");
    PROFILE_TABS
        .iter()
        .find(|(key, _)| *key == tab)
        .map(|(key, _)| *key)
        .unwrap_or("profile")
}

pub(crate) fn profile_flash(slug: &str, kind: &str, message: &str) -> Response {
    profile_flash_tab(slug, "profile", kind, message)
}

pub(crate) fn profile_flash_tab(slug: &str, tab: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/staff/{slug}/?tab={tab}&{encoded}")).into_response()
}

/// Ghost lets an owner/admin open any staff member's profile, and everyone
/// open their own. There are no non-owner roles wired up here yet, so the
/// rule is simply: your own profile, or any profile if you are the owner.
pub(crate) async fn profile_target(
    state: &AppState,
    viewer: &ghost_core::models::User,
    slug: &str,
) -> Option<ghost_core::models::User> {
    let target = repos::users::find_by_slug(&state.db, slug).await.ok().flatten()?;
    if target.id == viewer.id {
        return Some(target);
    }
    let role = repos::users::role_name(&state.db, &viewer.id).await.ok().flatten().unwrap_or_default();
    matches!(role.as_str(), "Owner" | "Administrator").then_some(target)
}

pub(crate) async fn profile_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let Some(target) = profile_target(&state, &user, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let is_self = target.id == user.id;
    let role = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    let v = |s: Option<&str>| esc(s.unwrap_or(""));
    let initial = target.name.as_deref().unwrap_or("?").chars().next().unwrap_or('?').to_uppercase().to_string();
    let avatar = match target.profile_image.as_deref().filter(|s| !s.is_empty()) {
        Some(img) => format!(
            r#"<img class="avatar lg" src="{}" alt="" onerror="this.replaceWith(Object.assign(document.createElement('span'),{{className:'avatar lg',textContent:'{}'}}))">"#,
            esc(img),
            esc(&initial)
        ),
        None => format!(r#"<span class="avatar lg">{}</span>"#, esc(&initial)),
    };

    // Role, suspension and removal are staff administration, so they are
    // gated on the viewer rather than on whose profile this is — and the
    // owner is exempt from all three.
    let viewer_role = role_of(&state, &user).await;
    let manage = may_act_on(viewer_role, Role::from_name(&role));
    let tab = profile_tab(&q);
    // Ghost's Email notifications tab gates the growth toggles on the edited
    // user being an administrator, and half of them on Stripe being set up.
    let target_is_admin = Role::from_name(&role).is_admin();
    let stripe_on = ghost_stripe::StripeConfig::from_settings(&state.settings).is_some();
    let admin_card = if manage && role != "Owner" {
        let suspended = target.status.as_deref() == Some("inactive");
        let self_note = if is_self {
            r#"<p class="muted" style="margin:10px 0 0">You cannot suspend or remove your own account.</p>"#
        } else {
            ""
        };
        format!(
            r#"<div class="section">Role &amp; access</div>
<div class="card rows rise" style="--i:5">
<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{people}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/staff/{slug}/role/" class="inline-form">
<label class="field fixed" style="min-width:200px;margin:0"><select name="role">{role_opts}</select><span>Role</span>{car}</label>
<button class="btn tonal small" type="submit">Change role</button>
</form>
<p class="muted" style="margin:10px 0 0">{role_help}</p>
</div></div>
{status_row}
<div class="row"><span class="ic tint-red">{del}</span>
<div class="body"><span class="title">Remove from the team</span><span class="detail">Their posts stay; the account and its sessions are deleted.</span></div>
<div class="trailing">{delete_btn}</div></div>
</div>{self_note}"#,
            people = svg("ghost"),
            slug = esc(&slug),
            role_opts = role_options_for(viewer_role, &role),
            car = SELECT_CARET,
            role_help = repos::staff::ASSIGNABLE_ROLES
                .iter()
                .find(|(n, _)| *n == role)
                .map(|(_, d)| esc(d))
                .unwrap_or_default(),
            del = svg("delete"),
            status_row = if is_self {
                String::new()
            } else {
                format!(
                    r#"<div class="row"><span class="ic tint-amber">{flag}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><form method="post" action="/ghost/settings/staff/{slug}/status/">
<input type="hidden" name="status" value="{next}">
<button class="btn tonal small" type="submit">{label}</button></form></div></div>"#,
                    flag = svg("flag"),
                    slug = esc(&slug),
                    title = if suspended { "Reactivate account" } else { "Suspend account" },
                    detail = if suspended {
                        "They can sign in again."
                    } else {
                        "Blocks sign in without deleting anything."
                    },
                    next = if suspended { "active" } else { "inactive" },
                    label = if suspended { "Reactivate" } else { "Suspend" },
                )
            },
            delete_btn = if is_self {
                r#"<button class="btn tonal small" disabled>Delete</button>"#.to_string()
            } else {
                format!(
                    r#"<form method="post" action="/ghost/settings/staff/{slug}/delete/" data-confirm="Remove staff member|&#8220;{name}&#8221; will lose access to this site.">
<button class="btn tonal small danger" type="submit">Delete</button></form>"#,
                    slug = esc(&slug),
                    name = esc(target.name.as_deref().unwrap_or("")),
                )
            },
            self_note = self_note,
        )
    } else {
        String::new()
    };

    // Changing your own password needs the current one; an owner or admin
    // resetting somebody else's does not — Ghost only asks the old password
    // of the account it belongs to.
    let password_card = if is_self || manage {
        format!(
            r#"<div class="section">Password</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{lock}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/staff/{slug}/password/" class="grid2" style="gap:12px">
{current_row}
<label class="field" style="margin:0"><input type="password" name="new_password" placeholder=" " autocomplete="new-password" minlength="10" required><span>New password · min 10 characters</span></label>
<label class="field" style="margin:0"><input type="password" name="confirm_password" placeholder=" " autocomplete="new-password" minlength="10" required><span>Confirm new password</span></label>
<div><button class="btn tonal small" type="submit">{label}</button></div>
</form>
<p class="muted" style="margin:10px 0 0">{note}</p>
</div></div>
</div>"#,
            lock = svg("logout"),
            slug = esc(&slug),
            current_row = if is_self {
                r#"<label class="field" style="margin:0"><input type="password" name="current_password" placeholder=" " autocomplete="current-password" required><span>Current password</span></label>
<span></span>"#
            } else {
                r#"<span></span><span></span>"#
            },
            label = if is_self { "Change password" } else { "Set new password" },
            note = if is_self {
                "Changing it signs this account out of every other session."
            } else {
                "Their sessions are all ended and they sign in with the new password."
            },
        )
    } else {
        String::new()
    };

    // --- the tab bar (Ghost splits this screen the same three ways) -------
    let tab_bar = format!(
        r#"<div class="seg-bar">{}</div>"#,
        PROFILE_TABS
            .iter()
            .map(|(key, label)| {
                format!(
                    r#"<a class="seg{on}" href="/ghost/settings/staff/{slug}/?tab={key}">{label}</a>"#,
                    on = if *key == tab { " on" } else { "" },
                    slug = esc(&slug),
                    key = key,
                    label = label,
                )
            })
            .collect::<String>()
    );
    let save_form = match tab {
        "social-links" => "social-form",
        "email-notifications" => "notify-form",
        _ => "profile-form",
    };

    // --- Profile: Ghost's field order (name, slug, email, location, bio) --
    let profile_body = if tab == "profile" {
        format!(
            r#"<form method="post" action="/ghost/settings/staff/{slug}/" id="profile-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{person}</span>
<div class="body" style="width:100%">
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " maxlength="191" required><span>Full name</span></label>
<label class="field" style="margin:0"><input name="slug" value="{slug_v}" placeholder=" " maxlength="191"><span>Slug</span></label>
<label class="field" style="margin:0"><input name="email" type="email" value="{email_v}" placeholder=" " maxlength="191" required><span>Email · used for notifications</span></label>
<label class="field" style="margin:0"><input name="location" value="{location}" placeholder=" " maxlength="150"><span>Location</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="bio" rows="4" placeholder=" " maxlength="2000">{bio}</textarea><span>Bio</span></label>
<p class="muted" style="margin:6px 0 0">Recommended: 250 characters. You&rsquo;ve used {bio_len}. Themes read these as <code>{{{{author.bio}}}}</code> and friends.</p>
</div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{image}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="profile_image" value="{profile_image}" placeholder=" "><span>Profile image URL</span></label>
<label class="field" style="margin:0"><input name="cover_image" value="{cover_image}" placeholder=" "><span>Cover image URL</span></label>
</div></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-slate">{seo}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="meta_title" value="{meta_title}" placeholder=" " maxlength="300"><span>Meta title</span></label>
<label class="field" style="margin:0"><input name="meta_description" value="{meta_description}" placeholder=" " maxlength="500"><span>Meta description</span></label>
</div></div></div>
</div>
</form>
<p class="muted" style="margin:14px 0 0 16px">Public author page: <a href="{author_url}" target="_blank" rel="noopener">{author_url}</a></p>
{password_card}
{admin_card}"#,
            slug = esc(&slug),
            name_v = v(target.name.as_deref()),
            slug_v = v(target.slug.as_deref()),
            email_v = v(target.email.as_deref()),
            location = v(target.location.as_deref()),
            bio = v(target.bio.as_deref()),
            bio_len = target.bio.as_deref().map(|b| b.chars().count()).unwrap_or(0),
            profile_image = v(target.profile_image.as_deref()),
            cover_image = v(target.cover_image.as_deref()),
            meta_title = v(target.meta_title.as_deref()),
            meta_description = v(target.meta_description.as_deref()),
            person = svg("ghost"),
            image = svg("image"),
            seo = svg("history"),
            author_url = esc(&author_archive_url(&state, &target).await),
            password_card = password_card,
            admin_card = admin_card,
        )
    } else {
        String::new()
    };

    // --- Social Links: Website plus the platforms Ghost offers, in its order
    let social_body = if tab == "social-links" {
        let social = |name: &str, label: &str, placeholder: &str, value: &str| {
            format!(
                r#"<label class="field" style="margin:0"><input name="{name}" value="{value}" placeholder="{placeholder}" maxlength="191"><span>{label}</span></label>"#,
                name = name,
                value = esc(value),
                placeholder = esc(placeholder),
                label = label,
            )
        };
        format!(
            r#"<form method="post" action="/ghost/settings/staff/{slug}/social/" id="social-form">
<div class="section">Social links</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{link}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><input name="website" value="{website}" placeholder="https://example.com" maxlength="2000"><span>Website</span></label>
<div class="grid2" style="gap:12px;margin-top:12px">
{twitter}
{facebook}
{linkedin}
{bluesky}
{threads}
{mastodon}
{tiktok}
{youtube}
{instagram}
</div>
<p class="muted" style="margin:12px 0 0">Shown on your public author page. Full profile URLs, or just the username.</p>
</div></div>
</div>
</form>"#,
            slug = esc(&slug),
            link = svg("link"),
            website = v(target.website.as_deref()),
            twitter = social("twitter", "X", "https://x.com/username", target.twitter.as_deref().unwrap_or("")),
            facebook = social("facebook", "Facebook", "https://www.facebook.com/username", target.facebook.as_deref().unwrap_or("")),
            linkedin = social("linkedin", "LinkedIn", "https://www.linkedin.com/in/username", target.linkedin.as_deref().unwrap_or("")),
            bluesky = social("bluesky", "Bluesky", "https://bsky.app/profile/username", target.bluesky.as_deref().unwrap_or("")),
            threads = social("threads", "Threads", "https://threads.net/@username", target.threads.as_deref().unwrap_or("")),
            mastodon = social("mastodon", "Mastodon", "https://mastodon.social/@username", target.mastodon.as_deref().unwrap_or("")),
            tiktok = social("tiktok", "TikTok", "https://www.tiktok.com/@username", target.tiktok.as_deref().unwrap_or("")),
            youtube = social("youtube", "YouTube", "https://www.youtube.com/@channel", target.youtube.as_deref().unwrap_or("")),
            instagram = social("instagram", "Instagram", "https://www.instagram.com/username", target.instagram.as_deref().unwrap_or("")),
        )
    } else {
        String::new()
    };

    // --- Email Notifications: the same groups and wordings as Ghost's screen.
    // `shown` markers tell the save handler which switches were actually
    // offered, so a toggle hidden by role or Stripe stays untouched.
    let notifications_body = if tab == "email-notifications" {
        let toggle = |key: &str, title: &str, detail: &str, on: bool| {
            format!(
                r#"{row}<input type="hidden" name="shown" value="{key}">"#,
                row = toggle_row(key, title, detail, on),
                key = key,
            )
        };
        let engagement = format!(
            r#"<div class="section">Engagement</div>
<div class="card rows rise" style="--i:2">{comments}{recommendations}</div>"#,
            comments = toggle(
                "comment_notifications",
                "Comments",
                "Every time a member comments on one of your posts",
                target.comment_notifications,
            ),
            recommendations = if target_is_admin {
                toggle(
                    "recommendation_notifications",
                    "Recommendations",
                    "Every time another publisher recommends you to their audience",
                    target.recommendation_notifications,
                )
            } else {
                String::new()
            },
        );
        let growth = if target_is_admin {
            format!(
                r#"<div class="section">Members</div>
<div class="card rows rise" style="--i:3">{signups}{paid_started}{paid_canceled}</div>
<div class="section">Revenue</div>
<div class="card rows rise" style="--i:4">{milestones}{donations}{gifts}</div>"#,
                signups = toggle(
                    "free_member_signup_notification",
                    "New signups",
                    "Every time a new free member signs up",
                    target.free_member_signup_notification,
                ),
                paid_started = if stripe_on {
                    toggle(
                        "paid_subscription_started_notification",
                        "New paid members",
                        "Every time a member starts a new paid subscription",
                        target.paid_subscription_started_notification,
                    )
                } else {
                    String::new()
                },
                paid_canceled = if stripe_on {
                    toggle(
                        "paid_subscription_canceled_notification",
                        "Paid member cancellations",
                        "Every time a member cancels their paid subscription",
                        target.paid_subscription_canceled_notification,
                    )
                } else {
                    String::new()
                },
                milestones = toggle(
                    "milestone_notifications",
                    "Milestones",
                    if stripe_on {
                        "Occasional summaries of your audience & revenue growth"
                    } else {
                        "Occasional summaries of your audience growth"
                    },
                    target.milestone_notifications,
                ),
                donations = if stripe_on {
                    toggle(
                        "donation_notifications",
                        "Tips & donations",
                        "Every time you receive a one-time payment",
                        target.donation_notifications,
                    )
                } else {
                    String::new()
                },
                gifts = if stripe_on {
                    toggle(
                        "gift_subscription_notifications",
                        "Gift subscriptions",
                        "Every time someone purchases or redeems a gift subscription",
                        target.gift_subscription_notifications,
                    )
                } else {
                    String::new()
                },
            )
        } else {
            String::new()
        };
        format!(
            r#"<form method="post" action="/ghost/settings/staff/{slug}/notifications/" id="notify-form">{engagement}{growth}</form>"#,
            slug = esc(&slug),
            engagement = engagement,
            growth = growth,
        )
    } else {
        String::new()
    };

    page_for(
        "Profile",
        "settings",
        Some(&user),
        viewer_role,
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/staff/" title="Back to staff">{back}</a>
<div><h1>{name}</h1><p class="sub">{role_label}</p></div>
<div class="actions"><button class="btn" type="submit" form="{save_form}">{check} Save</button></div></header>
{banner}
<div class="card padded rise" style="--i:1;display:flex;align-items:center;gap:16px;flex-wrap:wrap">
{avatar}
<div style="min-width:0"><b style="font-size:17px;display:block">{name}</b>
<span class="muted">{email}</span></div>
<div class="trailing"><span class="chip tonal">{role_label}</span></div>
</div>
{tab_bar}
{profile_body}
{social_body}
{notifications_body}"#,
            back = svg("chevron"),
            check = svg("check"),
            save_form = save_form,
            banner = flash_banner(&q),
            avatar = avatar,
            name = v(target.name.as_deref()),
            email = v(target.email.as_deref()),
            role_label = esc(&role),
            tab_bar = tab_bar,
            profile_body = profile_body,
            social_body = social_body,
            notifications_body = notifications_body,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn profile_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let Some(target) = profile_target(&state, &user, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let name = form.name.trim();
    if name.is_empty() {
        return profile_flash(&slug, "error", "Name cannot be empty.");
    }
    let email = form.email.trim().to_lowercase();
    if !email.contains('@') {
        return profile_flash(&slug, "error", "Please enter a valid email address.");
    }
    // Emails identify a staff account at signin, so a collision would lock
    // somebody out of theirs.
    if let Ok(Some(other)) = repos::users::find_by_email(&state.db, &email).await {
        if other.id != target.id {
            return profile_flash(&slug, "error", "Another staff member already uses that email.");
        }
    }

    let changes = repos::users::UserChanges {
        name: Some(name.to_string()),
        email: Some(email),
        bio: Some(form.bio.trim().to_string()),
        location: Some(form.location.trim().to_string()),
        profile_image: Some(form.profile_image.trim().to_string()),
        cover_image: Some(form.cover_image.trim().to_string()),
        meta_title: Some(form.meta_title.trim().to_string()),
        meta_description: Some(form.meta_description.trim().to_string()),
        slug: Some(form.slug.trim().to_string()),
        ..Default::default()
    };
    match repos::users::update(&state.db, &target.id, &changes).await {
        // Editing the slug moves the user, so redirect to wherever they now
        // live rather than to a URL that no longer resolves. A rename alone
        // leaves the slug (and this URL) as it was.
        Ok(updated) => profile_flash_tab(
            updated.slug.as_deref().unwrap_or(&slug),
            "profile",
            "ok",
            "Profile saved.",
        ),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

/// The Social Links tab — every field always renders there, so each one that
/// posts is meant, and an empty value clears it.
pub(crate) async fn profile_social_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<SocialForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if profile_target(&state, &user, &slug).await.is_none() {
        return not_found_page(&user, "That staff member could not be found.");
    }
    let t = |s: &str| Some(s.trim().to_string());
    let changes = repos::users::UserChanges {
        website: t(&form.website),
        twitter: t(&form.twitter),
        facebook: t(&form.facebook),
        linkedin: t(&form.linkedin),
        bluesky: t(&form.bluesky),
        threads: t(&form.threads),
        mastodon: t(&form.mastodon),
        tiktok: t(&form.tiktok),
        youtube: t(&form.youtube),
        instagram: t(&form.instagram),
        ..Default::default()
    };
    match repos::users::update_by_slug(&state.db, &slug, &changes).await {
        Ok(_) => profile_flash_tab(&slug, "social-links", "ok", "Social links saved."),
        Err(e) => profile_flash_tab(&slug, "social-links", "error", &e.message()),
    }
}

/// The Email Notifications tab. Unchecked switches post nothing, so each
/// rendered toggle carries a `shown` marker and only those are written back —
/// the ones a role or a missing Stripe setup hid keep their value.
pub(crate) async fn profile_notifications(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<Vec<(String, String)>>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if profile_target(&state, &user, &slug).await.is_none() {
        return not_found_page(&user, "That staff member could not be found.");
    }
    let shown: Vec<&str> =
        form.iter().filter(|(k, _)| k == "shown").map(|(_, v)| v.as_str()).collect();
    let on = |key: &str| form.iter().any(|(k, v)| k == key && v == "1");
    let flag = |key: &str| shown.contains(&key).then(|| on(key));
    let changes = repos::users::UserChanges {
        comment_notifications: flag("comment_notifications"),
        free_member_signup_notification: flag("free_member_signup_notification"),
        paid_subscription_started_notification: flag("paid_subscription_started_notification"),
        paid_subscription_canceled_notification: flag("paid_subscription_canceled_notification"),
        recommendation_notifications: flag("recommendation_notifications"),
        milestone_notifications: flag("milestone_notifications"),
        donation_notifications: flag("donation_notifications"),
        gift_subscription_notifications: flag("gift_subscription_notifications"),
        ..Default::default()
    };
    match repos::users::update_by_slug(&state.db, &slug, &changes).await {
        Ok(_) => profile_flash_tab(&slug, "email-notifications", "ok", "Email notifications saved."),
        Err(e) => profile_flash_tab(&slug, "email-notifications", "error", &e.message()),
    }
}

pub(crate) async fn profile_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<PasswordForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let Some(target) = profile_target(&state, &user, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let is_self = target.id == user.id;
    // Ghost's password endpoint asks the old password only of the account it
    // belongs to; an administrator resetting someone else's just needs edit
    // rights — which profile_target has already checked.
    if !is_self && !role_of(&state, &user).await.is_admin() {
        return profile_flash(&slug, "error", "Only you can change your own password.");
    }
    if form.new_password != form.confirm_password {
        return profile_flash(&slug, "error", "The new passwords do not match.");
    }
    if form.new_password.chars().count() < 10 {
        return profile_flash(&slug, "error", "Password must be at least 10 characters.");
    }
    if is_self {
        let email = user.email.clone().unwrap_or_default();
        match repos::users::verify_password(&state.db, &email, &form.current_password).await {
            Ok(Some(_)) => {}
            _ => return profile_flash(&slug, "error", "Current password is incorrect."),
        }
    }
    let pw = form.new_password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return profile_flash(&slug, "error", "Could not set that password."),
    };
    let changes = repos::users::UserChanges { password_hash: Some(hash), ..Default::default() };
    match repos::users::update(&state.db, &target.id, &changes).await {
        Ok(_) => {
            // A password change invalidates every session the account holds —
            // a cookie cloned before the change stops working. This session
            // survives its own change; nothing else does.
            let keep = is_self.then(|| session_id_from(&headers)).flatten();
            let _ = repos::sessions::destroy_for_user(&state.db, &target.id, keep.as_deref()).await;
            profile_flash(
                &slug,
                "ok",
                if is_self { "Password changed." } else { "Password set — their other sessions are signed out." },
            )
        }
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}
