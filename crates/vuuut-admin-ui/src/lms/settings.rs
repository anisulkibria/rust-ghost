//! LMS settings: whether the catalogue is public and where, how it lists
//! courses, reviews and certificates. Administrator and above. Saved
//! through `Settings::read`, the reader the public catalogue uses.

use super::*;
use vuuut_core::lms as l;

const SETTINGS: &str = "/ghost/lms/settings/";

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing dr-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

pub(crate) async fn lms_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = lms_settings(&state);
    let site = state.site_url();
    page_for_role(
        actor.role,
        "LMS settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="lm-settings">
<div class="section">The catalogue</div>
<div class="card rows rise" style="--i:2">
{enabled}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{t_key}" value="{title}" maxlength="60" placeholder="{t_default}"><span>Title</span></label>
<label class="field" style="margin:10px 0 0"><textarea name="{i_key}" rows="2" maxlength="400" placeholder=" ">{intro}</textarea><span>Introduction, above the courses</span></label></div></div>
{per}
{students}
{currency}
</div>
<div class="section">Address</div>
<div class="card padded rise" style="--i:3">
<label class="ec-urlfield"><span class="ec-urlfield-label">Catalogue</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site}/</span><input name="{b_key}" value="{base}" placeholder="courses" maxlength="80" spellcheck="false" autocomplete="off"><span class="ec-urlfield-site">/course-name/</span></span></label>
<p class="muted" style="margin:8px 0 0">Courses, lessons (…/course-name/learn/lesson/), My learning (…/dashboard/) and certificates all live under this address.</p>
</div>
<div class="section">Reviews & certificates</div>
<div class="card rows rise" style="--i:3">
{reviews}
{moderate}
{certs}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{s_key}" value="{signer}" maxlength="80" placeholder=" "><span>Name signed on certificates (blank: the site's title)</span></label></div></div>
</div>
</form>"#,
            head = crate::tools::tool_head_back("lms", BASE, Some("lm-settings"), ""),
            tabs = tabs_for(&state, "settings", &actor).await,
            banner = flash_banner(&q),
            enabled = row("globe", if s.enabled { "green" } else { "slate" }, "Show the catalogue on the site", &format!("Public address {}", s.paths.public_path("/courses/")), &switch(l::SETTING_ENABLED, s.enabled)),
            t_key = l::SETTING_TITLE,
            title = esc(&state.settings.get_str(l::SETTING_TITLE).unwrap_or_default().trim_matches('"').to_string()),
            t_default = l::DEFAULT_TITLE,
            i_key = l::SETTING_INTRO,
            intro = esc(&s.intro),
            per = row("layout", "blue", "Courses per page", "How many the catalogue shows at once.", &format!(
                r#"<label class="field" style="margin:0"><input name="{k}" type="number" min="1" max="96" value="{v}" aria-label="Courses per page"><span>Show</span></label>"#,
                k = l::SETTING_PER_PAGE, v = s.per_page,
            )),
            students = row("group", "purple", "Show student numbers", "“1,204 students” on cards and course pages.", &switch(l::SETTING_SHOW_STUDENTS, s.show_students)),
            currency = row("tags", "amber", "Currency", "Set in Payments.", &format!(r#"<a href="/ghost/settings/payments/#currency">{}</a>"#, esc(&s.currency))),
            b_key = l::SETTING_BASE_PATH,
            base = esc(&s.paths.base),
            site = esc(site.trim_end_matches('/')),
            reviews = row("star", "amber", "Course reviews", "Enrolled students can rate a course and write a review.", &switch(l::SETTING_REVIEWS, s.reviews)),
            moderate = row("check", "green", "Approve reviews first", "New reviews wait in Reviews until an editor approves them.", &switch(l::SETTING_MODERATE_REVIEWS, s.moderate_reviews)),
            certs = row("flag", "teal", "Certificates", "Finishing a course issues a certificate with a public verification address.", &switch(l::SETTING_CERTIFICATES, s.certificates)),
            s_key = l::SETTING_SIGNER,
            signer = esc(&s.signer),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn lms_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let current = lms_settings(&state);
    let Some(paths) = l::Paths::new(form.get(l::SETTING_BASE_PATH).map(String::as_str).unwrap_or(&current.paths.base)) else {
        return flash_to(SETTINGS, "error", "Enter an available address of up to three parts, using letters, numbers and dashes.");
    };
    if paths != current.paths {
        let admin = state.admin_slug();
        if vuuut_core::admin_address::suffix(&paths.base, &admin).is_some() {
            return flash_to(SETTINGS, "error", "Choose an address outside the admin.");
        }
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let per_page = form.get(l::SETTING_PER_PAGE).and_then(|v| v.trim().parse::<i64>().ok()).filter(|n| (1..=96).contains(n)).unwrap_or(l::DEFAULT_PER_PAGE);
    let writes = [
        (l::SETTING_ENABLED, serde_json::json!(flag(l::SETTING_ENABLED))),
        (l::SETTING_TITLE, serde_json::json!(vuuut_core::directory::clean(form.get(l::SETTING_TITLE).map(String::as_str).unwrap_or(""), 60))),
        (l::SETTING_INTRO, serde_json::json!(vuuut_core::directory::clean(form.get(l::SETTING_INTRO).map(String::as_str).unwrap_or(""), 400))),
        (l::SETTING_PER_PAGE, serde_json::json!(per_page)),
        (l::SETTING_SHOW_STUDENTS, serde_json::json!(flag(l::SETTING_SHOW_STUDENTS))),
        (l::SETTING_BASE_PATH, serde_json::json!(paths.base)),
        (l::SETTING_REVIEWS, serde_json::json!(flag(l::SETTING_REVIEWS))),
        (l::SETTING_MODERATE_REVIEWS, serde_json::json!(flag(l::SETTING_MODERATE_REVIEWS))),
        (l::SETTING_CERTIFICATES, serde_json::json!(flag(l::SETTING_CERTIFICATES))),
        (l::SETTING_SIGNER, serde_json::json!(vuuut_core::directory::clean(form.get(l::SETTING_SIGNER).map(String::as_str).unwrap_or(""), 80))),
    ];
    if let Err(e) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("lms: saving settings failed: {}", e.message());
        return flash_to(SETTINGS, "error", "Saving failed. Check the logs.");
    }
    crate::tools::tool_saved(&state, SETTINGS, "LMS settings saved.")
}
