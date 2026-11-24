//! Settings → Backups: whole-site backups (automatic and on demand) that can
//! be restored, and the exports that used to sit beside the importer.
//!
//! The backup machinery itself is `vuuut_api::backups`; this is its screen.

use super::*;
use vuuut_api::backups::{self, Backup, Kind, Schedule, MAX_BACKUPS};

fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_073_741_824.0 {
        format!("{:.1} GB", b / 1_073_741_824.0)
    } else if b >= 1_048_576.0 {
        format!("{:.1} MB", b / 1_048_576.0)
    } else {
        format!("{:.0} KB", (b / 1024.0).max(1.0))
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn kind_chip(kind: Kind) -> String {
    let cls = match kind {
        Kind::Manual => "tonal",
        Kind::Daily | Kind::Weekly | Kind::Monthly => "published",
        Kind::PreRestore => "scheduled",
        Kind::Uploaded => "draft",
    };
    format!(r#"<span class="chip {cls}">{}</span>"#, esc(kind.label()))
}

fn backup_row(state: &AppState, b: &Backup, newest: bool) -> String {
    let when = backups::local_time(state, b.taken_at);
    let uploaded = if b.kind == Kind::Uploaded {
        format!(" · uploaded {}", esc(&backups::local_time(state, b.stored_at)))
    } else {
        String::new()
    };
    let from = if b.site.is_empty() { String::new() } else { format!(" · {}", esc(&b.site)) };
    format!(
        r#"<div class="row bk-row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{when} {chip}{latest}</span>
<span class="detail">{size} · {rows} rows in {tables} tables · {files} files · Vuuut {version}{from}{uploaded}</span></div>
<div class="trailing">
<a class="icon-btn" href="/ghost/backups/download/{name}" title="Download" aria-label="Download this backup">{download}</a>
<form method="post" action="/ghost/backups/restore/" data-confirm="Restore this backup?|Posts, pages, members, settings, staff, themes and uploaded files all go back to how they were at {when_attr}. A backup of the site as it is right now is taken first, so you can undo this.">
<input type="hidden" name="name" value="{name}"><button class="btn tonal small" type="submit">{restore} Restore</button></form>
<form method="post" action="/ghost/backups/delete/" data-confirm="Delete this backup?|The backup from {when_attr} is deleted for good.">
<input type="hidden" name="name" value="{name}"><button class="icon-btn danger" type="submit" title="Delete" aria-label="Delete this backup">{del}</button></form>
</div></div>"#,
        tint = if b.kind.is_automatic() { "green" } else if b.kind == Kind::PreRestore { "blue" } else { "slate" },
        ic = svg("history"),
        when = esc(&when),
        when_attr = esc(&when),
        chip = kind_chip(b.kind),
        latest = if newest { r#" <span class="chip free">Latest</span>"# } else { "" },
        size = human_size(b.size),
        rows = thousands(b.rows),
        tables = b.tables,
        files = thousands(b.files),
        version = esc(&b.version),
        from = from,
        uploaded = uploaded,
        name = esc(&b.name),
        download = svg("import"),
        restore = svg("repeat"),
        del = svg("delete"),
    )
}

pub(crate) async fn backups_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let list = backups::list(&state).await;
    let schedule = Schedule::from_settings(&state);

    let next = match backups::next_due(schedule, &list) {
        None => "Automatic backups are off. Take one by hand whenever you like.".to_string(),
        Some(at) if at <= chrono::Utc::now() => "The first one runs within the next ten minutes.".to_string(),
        Some(at) => format!("Next one: {}.", backups::local_time(&state, at)),
    };
    let rows: String = if list.is_empty() {
        format!(
            r#"<div class="row"><span class="ic tint-slate">{ic}</span><div class="body"><span class="title">No backups yet</span>
<span class="detail">Take one now, or switch on automatic backups above.</span></div></div>"#,
            ic = svg("history"),
        )
    } else {
        list.iter().enumerate().map(|(i, b)| backup_row(&state, b, i == 0)).collect()
    };
    let used = list.len().min(MAX_BACKUPS);
    let meter = format!(
        r#"<div class="bk-meter" role="img" aria-label="{used} of {MAX_BACKUPS} backups kept">{cells}</div>"#,
        cells = (0..MAX_BACKUPS).map(|i| format!(r#"<i class="{}"></i>"#, if i < used { "on" } else { "" })).collect::<String>(),
    );

    // what an export would contain, so the checkbox list says how much
    let count = |sql: &'static str| {
        let db = state.db.clone();
        async move { sqlx::query_scalar::<_, i64>(sql).fetch_one(&db).await.unwrap_or(0) }
    };
    let members = count("SELECT COUNT(*) FROM members").await;
    let posts = count("SELECT COUNT(*) FROM posts WHERE type = 'post'").await;
    let pages = count("SELECT COUNT(*) FROM posts WHERE type = 'page'").await;
    let themes = vuuut_api::themes::list(&state).len();
    let components: String = EXPORT_COMPONENTS
        .iter()
        .map(|(key, label, detail, default_on)| {
            format!(
                r#"<label class="fchip exp"><input type="checkbox" name="{key}" value="1"{on}><span class="ck">{check}</span>
<span class="expl"><b>{label}</b><span>{detail}</span></span></label>"#,
                key = esc(key),
                on = if *default_on { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
                detail = esc(detail),
            )
        })
        .collect();

    page_for_role(
        actor_role,
        "Backups",
        "tools",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="section">Automatic backups</div>
<form method="post" action="/ghost/backups/schedule/" class="card padded rise bk-schedule" style="--i:1">
<div class="bk-sched-row">{seg}<button class="btn tonal small" type="submit">{check} Save schedule</button></div>
<p class="muted" style="margin:12px 0 0">{next} The {max} most recent backups are kept — automatic, manual and uploaded alike — and older ones are deleted as new ones are made.</p>
</form>

<div class="section bk-head"><span>Backups</span>{meter}<span class="bk-count">{used} of {max}</span></div>
<div class="card rows rise" style="--i:2">
<div class="row bk-actions">
<form method="post" action="/ghost/backups/create/"><button class="btn" type="submit">{history} Back up now</button></form>
<form method="post" action="/ghost/backups/upload/" enctype="multipart/form-data" class="bk-upload" data-confirm="Add this backup?|It joins the list below, where you can restore it. If {max} backups are already kept, the oldest is deleted.">
<input type="file" name="file" accept=".zip,application/zip" required class="filefield">
<button class="btn tonal small" type="submit">{upload} Upload a backup</button></form>
</div>
{rows}
</div>
<p class="muted" style="margin:10px 2px 0">A backup is the whole site: the database — posts, pages, members, settings, staff, API keys — and the
images, media, files and themes it uses. Restoring puts all of it back and keeps you signed in. Backups hold passwords and keys, so keep downloaded copies somewhere safe.</p>

<div class="section">Export</div>
<form class="card padded rise" style="--i:3" method="get" action="/ghost/backups/export/">
<p class="muted" style="margin:0 0 14px">A portable archive for moving to another Ghost-compatible site. Unlike a backup it carries no
credentials, and importing it adds content rather than replacing the site.</p>
<div class="expgrid">{components}</div>
<div style="margin-top:16px"><button class="btn" type="submit">{download} Download archive</button></div>
</form>
<div class="section">Individual files</div>
<div class="card rows rise" style="--i:4">
<div class="row"><span class="ic tint-blue">{notes}</span>
<div class="body"><span class="title">Content &amp; settings</span><span class="detail">{posts} posts and {pages} pages, with their tags · JSON</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/backups/export/content/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{ghost_ic}</span>
<div class="body"><span class="title">Export for Ghost</span><span class="detail">The same content and settings, trimmed to what Ghost {ghost_version} accepts — import it in Ghost under Settings → Import/Export · JSON</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/backups/export/ghost/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{people}</span>
<div class="body"><span class="title">Members</span><span class="detail">{members} members with labels, tiers and Stripe ids · CSV</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/members/export/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{chart}</span>
<div class="body"><span class="title">Post analytics</span><span class="detail">Sends, opens, views and feedback, one row per post · CSV</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/backups/export/analytics/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{themes_ic}</span>
<div class="body"><span class="title">Themes</span><span class="detail">{themes} installed, including anything edited in the browser · ZIP</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/backups/export/themes/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{link}</span>
<div class="body"><span class="title">Redirects &amp; routes</span><span class="detail">The two YAML files, each downloadable from its tool</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/redirects/">Redirects</a><a class="btn tonal small" href="/ghost/routes/">Routes</a></div></div>
</div>
<p class="muted" style="margin:10px 2px 0">Bringing content in from another site is in <a href="/ghost/migrations/">Tools → Migrations</a>.</p>"#,
            head = crate::tools::tool_head("backups", None, ""),
            banner = flash_banner(&q),
            seg = segmented("schedule", &[("off", "Off"), ("daily", "Daily"), ("weekly", "Weekly"), ("monthly", "Monthly")], schedule.as_str()),
            check = svg("check"),
            next = esc(&next),
            max = MAX_BACKUPS,
            meter = meter,
            used = used,
            history = svg("history"),
            upload = svg("upload"),
            rows = rows,
            components = components,
            download = svg("import"),
            notes = svg("notes"),
            ghost_ic = svg("upload"),
            ghost_version = esc(vuuut_db::ghost_compat::schema_version()),
            people = svg("person"),
            chart = svg("chart"),
            themes_ic = svg("themes"),
            link = svg("link"),
            posts = posts,
            pages = pages,
            members = members,
            themes = themes,
        ),
        true,
    )
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct ScheduleForm {
    #[serde(default)]
    schedule: String,
}

pub(crate) async fn backups_schedule_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<ScheduleForm>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let schedule = Schedule::parse(&form.schedule);
    match state.settings.set(&state.db, "backup_schedule", &serde_json::json!(schedule.as_str())).await {
        Ok(()) => crate::tools::tool_flash(
            "/ghost/backups/",
            "ok",
            match schedule {
                Schedule::Off => "Automatic backups switched off.",
                Schedule::Daily => "The site will back itself up every day.",
                Schedule::Weekly => "The site will back itself up every week.",
                Schedule::Monthly => "The site will back itself up every month.",
            },
        ),
        Err(e) => crate::tools::tool_flash("/ghost/backups/", "error", &e.message()),
    }
}

pub(crate) async fn backups_create(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match backups::create(&state, Kind::Manual).await {
        Ok(b) => crate::tools::tool_flash("/ghost/backups/", "ok", &format!("Backup taken — {}.", human_size(b.size))),
        Err(e) => crate::tools::tool_flash("/ghost/backups/", "error", &format!("The backup failed: {}", e.message())),
    }
}

#[derive(Deserialize)]
pub(crate) struct NameForm {
    #[serde(default)]
    name: String,
}

pub(crate) async fn backups_restore(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<NameForm>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match backups::restore(&state, &form.name).await {
        Ok(r) => {
            let mut msg = format!(
                "Restored — {} rows and {} files are back. The site as it was a moment ago is saved as the “Before a restore” backup.",
                thousands(r.rows),
                thousands(r.files)
            );
            for w in &r.warnings {
                msg.push(' ');
                msg.push_str(w);
            }
            fire_site_changed(&state);
            crate::tools::tool_flash("/ghost/backups/", if r.warnings.is_empty() { "ok" } else { "error" }, &msg)
        }
        Err(e) => crate::tools::tool_flash("/ghost/backups/", "error", &format!("Nothing was restored: {}", e.message())),
    }
}

pub(crate) async fn backups_delete(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<NameForm>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match backups::delete(&state, &form.name).await {
        Ok(()) => crate::tools::tool_flash("/ghost/backups/", "ok", "Backup deleted."),
        Err(e) => crate::tools::tool_flash("/ghost/backups/", "error", &e.message()),
    }
}

pub(crate) async fn backups_download(State(state): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(path) = backups::path_of(&state, &name) else {
        return (StatusCode::NOT_FOUND, "No such backup").into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(body) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/zip".to_string()),
                (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            ],
            body,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "No such backup").into_response(),
    }
}

pub(crate) async fn backups_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let bytes = match field.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => return crate::tools::tool_flash("/ghost/backups/", "error", &format!("Upload failed: {e}")),
        };
        return match backups::store_upload(&state, bytes).await {
            Ok(b) => crate::tools::tool_flash("/ghost/backups/", "ok", &format!("Backup added — {}. Restore it from the list when you are ready.", human_size(b.size))),
            Err(e) => crate::tools::tool_flash("/ghost/backups/", "error", &e.message()),
        };
    }
    crate::tools::tool_flash("/ghost/backups/", "error", "No file was uploaded.")
}
