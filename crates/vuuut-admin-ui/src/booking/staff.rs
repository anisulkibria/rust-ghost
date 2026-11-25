//! Staff — the people (or rooms, chairs, courts) that are booked — with
//! their weekly hours and the services they give; and Time off, for one
//! person or the whole business.

use super::services::{card, check, select};
use super::*;
use vuuut_core::booking::TINTS;
use vuuut_core::directory::{Day, DAYS_OF_WEEK};

const STAFF: &str = "/ghost/booking/staff/";
const OFF: &str = "/ghost/booking/time-off/";

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

/// One day of the weekly hours (the same controls as a directory listing).
pub(crate) fn hours_row(key: &str, name: &str, day: &Day, unset_label: &str) -> String {
    let (state, shifts) = match day {
        Day::Unknown => ("", Vec::new()),
        Day::Closed => ("closed", Vec::new()),
        Day::AllDay => ("24h", Vec::new()),
        Day::Open(r) => ("open", r.clone()),
    };
    let t = |m: Option<&(u16, u16)>, close: bool| m.map(|(a, b)| { let v = if close { *b } else { *a }; format!("{:02}:{:02}", (v / 60) % 24, v % 60) }).unwrap_or_default();
    let opts: String = [("", unset_label), ("open", "Open"), ("closed", "Closed"), ("24h", "All day")]
        .iter()
        .map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if *v == state { " selected" } else { "" }))
        .collect();
    let second = shifts.get(1);
    format!(
        r#"<div class="dr-day" data-day="{key}" data-state="{state}"><b>{name}</b>
<select class="ec-in" name="hours_{key}" aria-label="{name}">{opts}</select>
<span class="dr-times"><input class="ec-in" type="time" name="hours_{key}_open" value="{o1}" aria-label="{name} starts"><span>–</span><input class="ec-in" type="time" name="hours_{key}_close" value="{c1}" aria-label="{name} ends"></span>
<span class="dr-times dr-shift2"{hide2}><input class="ec-in" type="time" name="hours_{key}_open2" value="{o2}" aria-label="{name} starts again"><span>–</span><input class="ec-in" type="time" name="hours_{key}_close2" value="{c2}" aria-label="{name} ends again"></span>
<button class="btn text small dr-add-shift" type="button"{hide_add}>+ Split</button></div>"#,
        o1 = t(shifts.first(), false),
        c1 = t(shifts.first(), true),
        o2 = t(second, false),
        c2 = t(second, true),
        hide2 = if second.is_none() { " hidden" } else { "" },
        hide_add = if second.is_some() { " hidden" } else { "" },
    )
}

pub(crate) fn week_editor(hours: &vuuut_core::directory::Hours, unset_label: &str) -> String {
    format!(
        r#"<div class="dr-week">{}</div><div class="dr-week-tools"><button class="btn text small" type="button" id="dr-copy-weekdays">Copy Monday to Tuesday–Friday</button><button class="btn text small" type="button" id="dr-copy-all">Copy Monday to every day</button></div>"#,
        DAYS_OF_WEEK.iter().enumerate().map(|(i, (k, n, _))| hours_row(k, n, &hours.0[i], unset_label)).collect::<String>()
    )
}

pub(crate) async fn team_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let list = repo::staff(&state.db, false).await;
    let links = repo::links(&state.db).await;
    let services = repo::services(&state.db, false).await;
    let rows: String = list
        .iter()
        .map(|p| {
            let gives: Vec<&str> = links.iter().filter(|(_, st)| st == &p.id).filter_map(|(sv, _)| services.iter().find(|x| &x.id == sv).map(|x| x.name.as_str())).collect();
            let hours = p.hours();
            let open: Vec<&str> = DAYS_OF_WEEK.iter().enumerate().filter(|(i, _)| { let d = if hours.0[*i] == Day::Unknown { &s.hours.0[*i] } else { &hours.0[*i] }; matches!(d, Day::Open(_) | Day::AllDay) }).map(|(_, (_, _, short))| *short).collect();
            format!(
                r#"<div class="row dr-listing">{face}<div class="body"><span class="title"><a href="{STAFF}{id}/">{name}</a>{inactive}</span><span class="detail">{title}{days} · {gives}</span></div><div class="trailing"><a class="icon-btn" href="{STAFF}{id}/" title="Edit">{edit}</a></div></div>"#,
                face = if p.photo.is_empty() { format!(r#"<span class="lm-avatar tint-{}">{}</span>"#, esc(&p.tint), esc(&p.initials())) } else { format!(r#"<img class="dr-thumb bk-face" src="{}" alt="">"#, esc(&p.photo)) },
                id = esc(&p.id),
                name = esc(&p.name),
                inactive = if p.status == "active" { "" } else { r#" <span class="chip draft">Not bookable</span>"# },
                title = if p.title.is_empty() { String::new() } else { format!("{} · ", esc(&p.title)) },
                days = if open.is_empty() { "no hours".into() } else { open.join(" ") },
                gives = if gives.is_empty() { "every service with nobody chosen".to_string() } else { esc(&gives.join(", ")) },
                edit = svg("edit"),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Staff",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead" style="margin-top:16px">{count}. Staff can be people, or rooms, chairs, courts — anything booked by the hour. With nobody here, bookings use the business hours.</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("booking", BASE, None, &format!(r#"<a class="btn" href="{STAFF}new/">{} Add staff</a>"#, svg("add"))),
            tabs = tabs_for(&state, "staff", &actor).await,
            banner = flash_banner(&q),
            count = esc(&countable(list.len() as i64, "member")),
            body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">Nobody yet.</p><a class="btn" href="{STAFF}new/">{} Add staff</a></div>"#, svg("add")) } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn team_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    staff_editor(&state, &actor, None, &q).await
}

pub(crate) async fn team_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(p) = repo::find_staff(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That person could not be found.") };
    staff_editor(&state, &actor, Some(p), &q).await
}

async fn staff_editor(state: &AppState, actor: &Actor, p: Option<repo::Staff>, q: &HashMap<String, String>) -> Response {
    let get = |f: fn(&repo::Staff) -> &str| p.as_ref().map(f).unwrap_or("").to_string();
    let action = match &p { Some(p) => format!("{STAFF}{}/", esc(&p.id)), None => format!("{STAFF}new/") };
    let services = repo::services(&state.db, false).await;
    let linked: Vec<String> = match &p { Some(p) => repo::links(&state.db).await.into_iter().filter(|(_, s)| s == &p.id).map(|(sv, _)| sv).collect(), None => Vec::new() };
    let photo = get(|p| &p.photo);
    let tints: Vec<(String, String)> = TINTS.iter().map(|t| (t.to_string(), t[..1].to_uppercase() + &t[1..])).collect();
    let hours = p.as_ref().map(|p| p.hours()).unwrap_or_default();
    let tz = zone(state);
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard"><div class="ec-grid2">{name}{title}</div>{bio}</section>
{photo_card}{hours_card}"#,
        name = field("name", "Name", &get(|p| &p.name), "text", r#" required maxlength="120" id="ec-title""#),
        title = field("title", "Role — e.g. Senior therapist, Room 2", &get(|p| &p.title), "text", r#" maxlength="120""#),
        bio = format!(r#"<label class="field"><textarea name="bio" rows="5" placeholder=" " maxlength="4000">{}</textarea><span>About — shown on the booking page</span></label>"#, esc(&get(|p| &p.bio))),
        photo_card = card("Photo", "image", "pink", &format!(
            r#"<div class="ec-media lm-cover" id="ec-media">{items}<label class="ec-media-add" title="Upload a photo"><input type="file" accept="image/*" id="ec-upload">{plus}<span>Photo</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>
<div id="koenig-mount" hidden data-config="{{}}"></div>"#,
            items = if photo.is_empty() { String::new() } else { crate::ecom::media_item(&photo) },
            plus = svg("add"),
        ), "bk-photo"),
        hours_card = card("Working hours", "history", "purple", &format!(
            r#"{week}<p class="muted ec-hint">In {tz}. Days left as “Business hours” follow the hours in Settings. Bookable times are cut from these hours, minus bookings and time off.</p>"#,
            week = week_editor(&hours, "Business hours"),
            tz = esc(tz.name()),
        ), "bk-hours"),
    );
    let svc_boxes: String = services.iter().map(|x| check("service", &x.id, &x.name, linked.contains(&x.id))).collect();
    let side = format!(
        "{contact}{gives}{org}{del}",
        contact = card("Contact", "mail", "blue", &format!(
            r#"{e}{ph}<p class="muted ec-hint">New bookings are emailed here. Not shown on the site.</p>"#,
            e = field("email", "Email", &get(|p| &p.email), "email", r#" maxlength="191""#),
            ph = field("phone", "Phone", &get(|p| &p.phone), "tel", r#" maxlength="40""#),
        ), "bk-contact"),
        gives = card("Services", "calendar", "teal", &if services.is_empty() { r#"<p class="muted" style="margin:0">No services yet.</p>"#.to_string() } else { format!(r#"<div class="dr-checks bk-checks">{svc_boxes}</div>"#) }, "bk-gives"),
        org = card("Display", "tune", "amber", &format!(
            "{status}{tint}{order}{slug}",
            status = select("status", "Bookable", &[("active".into(), "Yes — takes bookings".into()), ("inactive".into(), "No — hidden for now".into())], p.as_ref().map(|p| p.status.as_str()).unwrap_or("active")),
            tint = select("tint", "Colour", &tints, p.as_ref().map(|p| p.tint.as_str()).unwrap_or("blue")),
            order = field("sort_order", "Order", &p.as_ref().map(|p| p.sort_order).unwrap_or(0).to_string(), "number", r#" min="0" max="999""#),
            slug = field("slug", "Address", &get(|p| &p.slug), "text", r#" maxlength="100""#),
        ), "bk-display"),
        del = if p.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><a class="btn text" href="{OFF}">{} Time off</a><button class="btn text danger" type="submit" form="bk-delete">{} Remove</button></div></section>"#, svg("history"), svg("delete")) } else { String::new() },
    );
    let heading = p.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Add staff".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{STAFF}" title="Back to staff">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="bk-staff">{check} {save}</button></div></header>
{banner}{del}
<form id="bk-staff" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/ecom-editor.js" defer></script><script src="{ASSETS}/booking-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&p.as_ref().map(|p| if p.title.is_empty() { "Staff".to_string() } else { p.title.clone() }).unwrap_or_else(|| "A person, or a room, chair or court.".into())),
        check = svg("check"),
        save = if p.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        del = p.as_ref().map(|p| format!(r#"<form method="post" action="{STAFF}{}/delete/" id="bk-delete" data-confirm="Remove {}|Their bookings stay, with nobody assigned."></form>"#, esc(&p.id), esc(&p.name))).unwrap_or_default(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

async fn read_staff(state: &AppState, pairs: &[(String, String)]) -> Result<rules::StaffInput, String> {
    let services = repo::services(&state.db, false).await;
    let photo = pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default();
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image").cloned().collect();
    all.push(("photo".into(), photo));
    rules::StaffInput::from_form(&all, |id| services.iter().any(|x| x.id == id))
}

pub(crate) async fn team_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_staff(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{STAFF}new/"), "error", &m),
    };
    match repo::save_staff(&state.db, None, &input).await {
        Ok(id) => flash_to(&format!("{STAFF}{id}/"), "ok", "Added."),
        Err(_) => flash_to(&format!("{STAFF}new/"), "error", "That could not be saved."),
    }
}

pub(crate) async fn team_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{STAFF}{id}/");
    let input = match read_staff(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_staff(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn team_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_staff(&state.db, &id).await {
        Ok(()) => flash_to(STAFF, "ok", "Removed."),
        Err(_) => flash_to(STAFF, "error", "That could not be removed."),
    }
}

// ---------------------------------------------------------------------------
// time off
// ---------------------------------------------------------------------------

pub(crate) async fn time_off_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let tz = zone(&state);
    let staff = repo::staff(&state.db, false).await;
    let list = repo::time_off(&state.db, &repo::db_time(Utc::now() - Duration::days(1)), "9999").await;
    let rows: String = list
        .iter()
        .map(|o| {
            let (s, e) = (repo::parse_time(&o.starts_at).unwrap_or_else(Utc::now), repo::parse_time(&o.ends_at).unwrap_or_else(Utc::now));
            let who = o.staff_id.as_deref().and_then(|id| staff.iter().find(|p| p.id == id)).map(|p| p.name.clone()).unwrap_or_else(|| "Everyone — the business is closed".into());
            format!(
                r#"<div class="row dr-listing"><span class="ic tint-amber">{ic}</span><div class="body"><span class="title">{who}</span><span class="detail">{from} → {to}{reason}</span></div>
<div class="trailing"><form method="post" action="{OFF}{id}/delete/"><button class="icon-btn" type="submit" title="Remove">{del}</button></form></div></div>"#,
                ic = svg("history"),
                who = esc(&who),
                from = esc(&s.with_timezone(&tz).format("%a %-d %b %Y, %H:%M").to_string()),
                to = esc(&e.with_timezone(&tz).format("%a %-d %b %Y, %H:%M").to_string()),
                reason = if o.reason.is_empty() { String::new() } else { format!(" · {}", esc(&o.reason)) },
                id = esc(&o.id),
                del = svg("delete"),
            )
        })
        .collect();
    let staff_opts: String = std::iter::once(r#"<option value="">Everyone (holiday, closure)</option>"#.to_string()).chain(staff.iter().map(|p| format!(r#"<option value="{}">{}</option>"#, esc(&p.id), esc(&p.name)))).collect();
    let today = Utc::now().with_timezone(&tz).format("%Y-%m-%d").to_string();
    page_for_role(
        actor.role,
        "Time off",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="dr-split"><div><div class="section">Coming up</div><div class="rise" style="--i:2">{body}</div></div>
<div><div class="section">Add time off</div><form class="card padded rise" style="--i:3" method="post" action="{OFF}">
<label class="field fixed"><select name="staff_id">{staff_opts}</select><span>Who</span>{SELECT_CARET}</label>
<div class="ec-grid2"><label class="field"><input type="date" name="from" value="{today}" required><span>From</span></label><label class="field"><input type="time" name="from_time" step="300"><span>Time (blank: all day)</span></label></div>
<div class="ec-grid2"><label class="field"><input type="date" name="to" value="{today}" required><span>Until</span></label><label class="field"><input type="time" name="to_time" step="300"><span>Time (blank: end of day)</span></label></div>
<label class="field"><input name="reason" maxlength="200" placeholder=" "><span>Reason (only staff see it)</span></label>
<div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{add} Add</button></div>
<p class="muted ec-hint">Times in {tzn}. Existing bookings in that time stay; check them in the Calendar.</p></form></div></div>"#,
            head = crate::tools::tool_head_back("booking", BASE, None, ""),
            tabs = tabs_for(&state, "time-off", &actor).await,
            banner = flash_banner(&q),
            body = if rows.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No time off planned.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{rows}</div>"#) },
            add = svg("add"),
            tzn = esc(tz.name()),
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct OffForm {
    staff_id: String,
    from: String,
    from_time: String,
    to: String,
    to_time: String,
    reason: String,
}

pub(crate) async fn time_off_add(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<OffForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let tz = zone(&state);
    let (Ok(d1), Ok(d2)) = (NaiveDate::parse_from_str(f.from.trim(), "%Y-%m-%d"), NaiveDate::parse_from_str(f.to.trim(), "%Y-%m-%d")) else {
        return flash_to(OFF, "error", "Choose the dates.");
    };
    let m1 = vuuut_core::directory::parse_clock(&f.from_time).map(i64::from).unwrap_or(0);
    let m2 = vuuut_core::directory::parse_clock(&f.to_time).map(i64::from).unwrap_or(24 * 60);
    let (Some(s), Some(e)) = (rules::local(tz, d1, m1), rules::local(tz, d2, m2)) else { return flash_to(OFF, "error", "Those times do not exist (a clock change).") };
    if e <= s {
        return flash_to(OFF, "error", "The end must be after the start.");
    }
    let staff = Some(f.staff_id.trim()).filter(|x| vuuut_core::ids::is_objectid(x));
    match repo::add_time_off(&state.db, staff, &repo::db_time(s), &repo::db_time(e), &vuuut_core::directory::clean(&f.reason, 200)).await {
        Ok(()) => flash_to(OFF, "ok", "Time off added. Those times are no longer bookable."),
        Err(_) => flash_to(OFF, "error", "That could not be saved."),
    }
}

pub(crate) async fn time_off_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_time_off(&state.db, &id).await {
        Ok(()) => flash_to(OFF, "ok", "Removed."),
        Err(_) => flash_to(OFF, "error", "That could not be removed."),
    }
}
