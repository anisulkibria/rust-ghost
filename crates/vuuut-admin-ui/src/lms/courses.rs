//! The course editor: the same two-column editor as a store product or a
//! directory listing — the description in Koenig, cards down the middle,
//! status, access and settings down the side — plus the curriculum builder:
//! sections and their lessons, reordered by dragging (or the arrow
//! buttons), renamed in place, and lessons, quizzes and assignments added
//! to a section in one line.

use super::*;
use vuuut_core::lms::{Level, LessonKind};

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

fn area(name: &str, label: &str, value: &str, rows: u8, extra: &str) -> String {
    format!(r#"<label class="field"><textarea name="{name}" rows="{rows}" placeholder=" "{extra}>{v}</textarea><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

pub(super) fn card(title: &str, icon: &str, tint: &str, body: &str, id: &str) -> String {
    format!(r#"<section class="card padded ec-card" id="{id}"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title))
}

pub(super) fn check(name: &str, value: &str, label: &str, on: bool) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="{v}"{c}> {l}</label>"#, v = esc(value), c = if on { " checked" } else { "" }, l = esc(label))
}

fn select(name: &str, label: &str, opts: &[(String, String)], current: &str) -> String {
    let o: String = opts.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), s = if v == current { " selected" } else { "" }, l = esc(l))).collect();
    format!(r#"<label class="field fixed"><select name="{name}">{o}</select><span>{l}</span>{SELECT_CARET}</label>"#, l = esc(label))
}

fn kind_icon(kind: &str) -> &'static str {
    match LessonKind::from_key(kind) {
        LessonKind::Quiz => "check",
        LessonKind::Assignment => "edit",
        LessonKind::Lesson => "notes",
    }
}

pub(crate) async fn course_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn course_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That course could not be found.");
    }
    let Some(c) = repo::find_course(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That course could not be found.");
    };
    editor(&state, &actor, Some(c), &q).await
}

/// The curriculum: sections with their lessons, each lesson a row that
/// links to its editor. Section titles are inputs of the course form; the
/// add-lesson and remove-section controls belong to small forms outside it.
fn curriculum(_c: &repo::Course, sections: &[repo::Section], lessons: &[repo::Lesson], questions: &HashMap<String, i64>) -> String {
    let lesson_row = |l: &repo::Lesson| {
        let meta = match l.kind() {
            LessonKind::Quiz => countable(questions.get(&l.id).copied().unwrap_or(0), "question"),
            LessonKind::Assignment => format!("{} points", l.points),
            LessonKind::Lesson => {
                let mut bits = Vec::new();
                if !l.video_url.is_empty() {
                    bits.push("Video".to_string());
                }
                let d = rules::duration_label(l.duration_minutes);
                if !d.is_empty() {
                    bits.push(d);
                }
                if bits.is_empty() { "Text".into() } else { bits.join(" · ") }
            }
        };
        format!(
            r#"<li class="lm-les" data-id="{id}" draggable="true"><span class="lm-grip" aria-hidden="true">⋮⋮</span><span class="ic tint-{tint} lm-les-ic">{ic}</span>
<a class="lm-les-title" href="{LESSONS}{id}/">{title}</a><span class="lm-les-meta">{kind} · {meta}{preview}{drip}{draft}</span>
<span class="lm-move"><button type="button" class="icon-btn" data-move="up" title="Move up">{up}</button><button type="button" class="icon-btn" data-move="down" title="Move down">{down}</button></span></li>"#,
            id = esc(&l.id),
            tint = match l.kind() { LessonKind::Quiz => "amber", LessonKind::Assignment => "pink", LessonKind::Lesson => "blue" },
            ic = svg(kind_icon(&l.kind)),
            title = esc(&l.title),
            kind = l.kind().label(),
            meta = esc(&meta),
            preview = if l.is_preview() { r#" · <b class="lm-tag">Free preview</b>"# } else { "" },
            drip = if l.drip_days > 0 { format!(" · opens day {}", l.drip_days) } else { String::new() },
            draft = if l.status == "draft" { r#" · <b class="lm-tag lm-tag-muted">Draft</b>"# } else { "" },
            up = svg("up"),
            down = svg("down"),
        )
    };
    let add_row = |form: &str| {
        format!(
            r#"<div class="lm-add"><input class="ec-in" name="title" form="{form}" placeholder="New lesson, quiz or assignment title" maxlength="160" aria-label="Title">
<select class="ec-in" name="kind" form="{form}" aria-label="Kind"><option value="lesson">Lesson</option><option value="quiz">Quiz</option><option value="assignment">Assignment</option></select>
<button class="btn tonal small" type="submit" form="{form}">{add} Add</button></div>"#,
            add = svg("add"),
        )
    };
    let mut out = String::new();
    for s in sections {
        let inside: Vec<&repo::Lesson> = lessons.iter().filter(|l| l.section_id.as_deref() == Some(s.id.as_str())).collect();
        out.push_str(&format!(
            r#"<div class="lm-sec" data-id="{id}"><div class="lm-sec-head"><span class="lm-grip" aria-hidden="true">⋮⋮</span>
<input class="lm-sec-title" name="sec_title_{id}" value="{title}" maxlength="160" aria-label="Section title">
<span class="lm-sec-count muted">{n}</span>
<span class="lm-move"><button type="button" class="icon-btn" data-move="up" title="Move section up">{up}</button><button type="button" class="icon-btn" data-move="down" title="Move section down">{down}</button>
<button type="submit" class="icon-btn" form="lm-delsec-{id}" title="Remove section">{del}</button></span></div>
<ol class="lm-lessons">{rows}</ol>{add}</div>"#,
            id = esc(&s.id),
            title = esc(&s.title),
            n = countable(inside.len() as i64, "item"),
            up = svg("up"),
            down = svg("down"),
            del = svg("delete"),
            rows = inside.iter().map(|l| lesson_row(l)).collect::<String>(),
            add = add_row(&format!("lm-add-{}", s.id)),
        ));
    }
    let loose: Vec<&repo::Lesson> = lessons.iter().filter(|l| l.section_id.as_ref().is_none_or(|sid| !sections.iter().any(|s| &s.id == sid))).collect();
    if !loose.is_empty() || sections.is_empty() {
        out.push_str(&format!(
            r#"<div class="lm-sec lm-sec-loose" data-id=""><div class="lm-sec-head"><b>{label}</b></div><ol class="lm-lessons">{rows}</ol>{add}</div>"#,
            label = if sections.is_empty() { "Lessons" } else { "Not in a section" },
            rows = loose.iter().map(|l| lesson_row(l)).collect::<String>(),
            add = add_row("lm-add-loose"),
        ));
    }
    let total_minutes: i64 = lessons.iter().map(|l| l.duration_minutes).sum();
    format!(
        r#"<p class="muted ec-hint lm-cur-sum">{sections} · {lessons} · {quizzes} · {mins}. Drag to reorder; changes are saved with the course.</p>
<div class="lm-cur" id="lm-cur">{out}</div>
<input type="hidden" name="curriculum" id="lm-curriculum" value="">
<div class="lm-add lm-add-sec"><input class="ec-in" name="title" form="lm-add-section" placeholder="New section — e.g. “Getting started”" maxlength="160" aria-label="Section title">
<button class="btn small" type="submit" form="lm-add-section">{add} Add section</button></div>"#,
        sections = countable(sections.len() as i64, "section"),
        lessons = countable(lessons.iter().filter(|l| l.kind() == LessonKind::Lesson).count() as i64, "lesson"),
        quizzes = { let n = lessons.iter().filter(|l| l.kind() != LessonKind::Lesson).count(); if n == 1 { "1 quiz or assignment".to_string() } else { format!("{n} quizzes & assignments") } },
        mins = { let d = rules::duration_label(total_minutes); if d.is_empty() { "no timings yet".into() } else { format!("{d} of lessons") } },
        add = svg("add"),
    )
}

async fn editor(state: &AppState, actor: &Actor, c: Option<repo::Course>, q: &HashMap<String, String>) -> Response {
    let s = lms_settings(state);
    let x = repo::extras(&state.db).await;
    let get = |f: fn(&repo::Course) -> &str| c.as_ref().map(f).unwrap_or("").to_string();
    let num = |f: fn(&repo::Course) -> i64| c.as_ref().map(f).unwrap_or(0);
    let access = c.as_ref().map(|c| c.access()).unwrap_or(Access::Free);
    let status = CourseStatus::from_key(c.as_ref().map(|c| c.status.as_str()).unwrap_or("draft"));
    let action = match &c {
        Some(c) => format!("{COURSES}{}/", esc(&c.id)),
        None => format!("{COURSES}new/"),
    };

    let lexical = get(|c| &c.description_lexical);
    let legacy_html = c.as_ref().map(|c| c.description_markup()).unwrap_or_default();
    let initial = if !lexical.trim().is_empty() { Some(lexical) } else { vuuut_render::lexical::wrap_existing(Some(""), Some(&legacy_html)) };
    let initial = initial.map(|x| vuuut_core::url::base_assets(&vuuut_core::url::unbase_assets(&x)));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": "Describe the course — who it is for, what they will make, and why it works…",
        "siteUrl": state.site_url(),
        "siteTitle": state.settings.get_str("title").unwrap_or_default(),
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false,
        "stripeEnabled": false,
        "basePath": vuuut_core::url::subdir(),
        "unsplash": serde_json::Value::Null,
        "pinturaConfig": serde_json::Value::Null,
    });

    // Curriculum.
    let curriculum_card = match &c {
        Some(course) => {
            let sections = repo::sections(&state.db, &course.id).await;
            let lessons = repo::lessons(&state.db, &course.id).await;
            let qc = repo::question_counts(&state.db, &course.id).await;
            card("Curriculum", "list", "purple", &curriculum(course, &sections, &lessons, &qc), "lm-curriculum-card")
        }
        None => card("Curriculum", "list", "purple", r#"<p class="muted" style="margin:0">Create the course first; then add its sections, lessons, quizzes and assignments here.</p>"#, "lm-curriculum-card"),
    };

    let cover = get(|c| &c.image);
    let media = format!(
        r#"<div class="ec-media lm-cover" id="ec-media">{items}<label class="ec-media-add" title="Upload a cover"><input type="file" accept="image/*" id="ec-upload">{plus}<span>Cover image</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>
<p class="muted ec-hint">A 16:9 picture works best. It is the card in the catalogue and the top of the course page.</p>
{video}"#,
        items = if cover.is_empty() { String::new() } else { crate::ecom::media_item(&cover) },
        plus = svg("add"),
        video = field("intro_video", "Introduction video — YouTube, Vimeo, Loom or Wistia link", &get(|c| &c.intro_video), "url", r#" maxlength="500" placeholder="https://""#),
    );

    let promises = format!(
        r#"{o}{r}{a}<p class="muted ec-hint">One item per line. They become the “What you'll learn”, “Requirements” and “Who it's for” lists on the course page.</p>"#,
        o = area("outcomes", "What students will learn", &get(|c| &c.outcomes), 5, r#" maxlength="6000""#),
        r = area("requirements", "Requirements", &get(|c| &c.requirements), 3, r#" maxlength="6000""#),
        a = area("audience", "Who this course is for", &get(|c| &c.audience), 3, r#" maxlength="6000""#),
    );

    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}{prefix}<b>{slug}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>
{t}{d}{h}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        prefix = esc(&s.paths.public_path("/courses/")),
        slug = esc(&get(|c| &c.slug)),
        st = esc(&{ let t = get(|c| &c.seo_title); if t.is_empty() { get(|c| &c.title) } else { t } }),
        sd = esc(&{ let d = get(|c| &c.seo_description); if d.is_empty() { get(|c| &c.subtitle) } else { d } }),
        t = field("seo_title", "Page title", &get(|c| &c.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = area("seo_description", "Meta description", &get(|c| &c.seo_description), 2, r#" maxlength="320" id="ec-seo-desc""#),
        h = field("slug", "Address on the site", &get(|c| &c.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*" id="ec-handle""#),
    );

    // Side: status.
    let status_opts: Vec<(String, String)> = CourseStatus::ALL.iter().map(|x| (x.key().to_string(), x.label().to_string())).collect();
    let view = c.as_ref().filter(|c| c.is_published() && s.enabled).map(|c| format!(r#"<a class="btn text small" href="{}" target="_blank" rel="noopener">{} View on the site</a>"#, public_href(&s, &format!("/courses/{}/", c.slug)), svg("open"))).unwrap_or_default();
    let status_card = card("Status", "eye", "green", &format!(
        r#"{chip}{sel}<p class="muted ec-hint">Only published courses are in the catalogue{off}.</p>{feat}{view}"#,
        chip = c.as_ref().map(|c| { let (k, l) = status_chip(&c.status); format!(r#"<p style="margin:0 0 10px"><span class="chip {k}">{l}</span></p>"#) }).unwrap_or_default(),
        sel = select("status", "Status", &status_opts, status.key()),
        off = if s.enabled { "" } else { " — and the catalogue itself is switched off in Settings" },
        feat = check("featured", "1", "Featured — shown first and marked", num(|c| c.featured) != 0),
    ), "lm-status");

    // Side: access & pricing.
    let kinds: String = Access::ALL.iter().map(|a| format!(
        r#"<label class="dr-kind lm-kind"><input type="radio" name="access" value="{k}"{c}><b>{l}</b><span>{h}</span></label>"#,
        k = a.key(), c = if *a == access { " checked" } else { "" }, l = esc(a.label()), h = esc(a.hint()),
    )).collect();
    let tiers = repo::tiers(&state.db).await;
    let chosen = c.as_ref().map(|c| c.tiers()).unwrap_or_default();
    let tier_boxes: String = tiers.iter().map(|(id, name)| check("tier", id, name, chosen.contains(id))).collect();
    let products = repo::store_products(&state.db).await;
    let mut prod_opts = vec![(String::new(), "Not linked yet".to_string())];
    prod_opts.extend(products.iter().map(|(id, t, st)| (id.clone(), if st == "active" { t.clone() } else { format!("{t} ({st})") })));
    let money = |cents: i64| if cents > 0 { format!("{}.{:02}", cents / 100, cents % 100) } else { String::new() };
    let pricing = card("Access & pricing", "lock", "amber", &format!(
        r#"<div class="lm-kinds">{kinds}</div>
<div class="lm-only lm-only-paid"><div class="ec-grid2">{price}{compare}</div>
{product}
{make}
<p class="muted ec-hint">Buying the linked product enrols the buyer, and a plain product's price follows the course's. Prices are in {cur}.</p></div>
<div class="lm-only lm-only-members">{paid_only}</div>
<div class="lm-only lm-only-tiers"><div class="ec-sublabel">Tiers that can enrol</div><div class="dr-checks">{tier_boxes}</div></div>"#,
        price = field("price", "Price", &money(num(|c| c.price_cents)), "text", r#" inputmode="decimal""#),
        compare = field("compare_price", "Regular price (shown crossed out)", &money(num(|c| c.compare_cents)), "text", r#" inputmode="decimal""#),
        product = select("product_id", "Sold as store product", &prod_opts, &get(|c| c.product_id.as_deref().unwrap_or(""))),
        make = match &c {
            Some(course) if course.product_id.is_none() => format!(r#"<button class="btn tonal small" type="submit" form="lm-make-product">{} Create a store product for this course</button>"#, svg("store")),
            _ => String::new(),
        },
        cur = esc(&s.currency),
        paid_only = check("tier", "paid", "Paid members only", chosen.iter().any(|t| t == "paid")),
        tier_boxes = if tiers.is_empty() { r#"<p class="muted" style="margin:0">No paid tiers yet — add them in Memberships.</p>"#.to_string() } else { tier_boxes },
    ), "lm-access");

    // Side: organisation.
    let mut cat_opts = vec![(String::new(), "No category".to_string())];
    cat_opts.extend(x.categories.iter().map(|k| (k.id.clone(), k.name.clone())));
    let mut ins_opts = vec![(String::new(), "Nobody shown".to_string())];
    ins_opts.extend(x.instructors.iter().map(|i| (i.id.clone(), i.name.clone())));
    let current_instructor = c.as_ref().map(|c| c.instructor_id.clone().unwrap_or_default()).unwrap_or_else(|| actor.user.id.clone());
    let levels: Vec<(String, String)> = Level::ALL.iter().map(|l| (l.key().to_string(), l.label().to_string())).collect();
    let org = card("Organisation", "folder", "blue", &format!(
        "{cat}{ins}<div class=\"ec-grid2\">{lvl}{lang}</div>{dur}{kw}",
        cat = select("category_id", "Category", &cat_opts, &get(|c| c.category_id.as_deref().unwrap_or(""))),
        ins = select("instructor_id", "Instructor", &ins_opts, &current_instructor),
        lvl = select("level", "Level", &levels, c.as_ref().map(|c| c.level.as_str()).unwrap_or("all")),
        lang = field("language", "Language", &get(|c| &c.language), "text", r#" maxlength="40" placeholder="English""#),
        dur = field("duration_minutes", "Total length in minutes (blank: add up the lessons)", &{ let d = num(|c| c.duration_minutes); if d > 0 { d.to_string() } else { String::new() } }, "number", r#" min="0" max="100000""#),
        kw = field("keywords", "Keywords for search", &get(|c| &c.keywords), "text", r#" maxlength="200""#),
    ), "lm-org");

    let date_value = |v: Option<&str>| v.map(|d| d.get(..16).unwrap_or(d).replace(' ', "T")).unwrap_or_default();
    let settings_card = card("Course settings", "tune", "teal", &format!(
        r#"{seq}{cert}
{cap}{days}
<div class="ec-sublabel">Enrolment window</div>
{opens}{closes}
<p class="muted ec-hint">Leave blank to keep enrolment open. Access days count from each student's enrolment; blank is lifetime access.</p>"#,
        seq = check("sequential", "1", "Lessons unlock in order", num(|c| c.sequential) != 0),
        cert = check("certificate", "1", "Certificate on completion", c.as_ref().map(|c| c.certificate != 0).unwrap_or(true)),
        cap = field("capacity", "Seats (blank: unlimited)", &{ let v = num(|c| c.capacity); if v > 0 { v.to_string() } else { String::new() } }, "number", r#" min="0""#),
        days = field("access_days", "Access for … days", &{ let v = num(|c| c.access_days); if v > 0 { v.to_string() } else { String::new() } }, "number", r#" min="0""#),
        opens = field("opens_at", "Opens", &date_value(c.as_ref().and_then(|c| c.opens_at.as_deref())), "datetime-local", ""),
        closes = field("closes_at", "Closes", &date_value(c.as_ref().and_then(|c| c.closes_at.as_deref())), "datetime-local", ""),
    ), "lm-settings");

    let numbers = match &c {
        Some(course) => {
            let students = x.students.get(&course.id).copied().unwrap_or(0);
            let (done_rows, done) = repo::browse_enrollments(&state.db, &repo::EnrollmentQuery { course_id: Some(&course.id), status: Some("completed"), limit: 1, ..Default::default() }).await;
            let _ = done_rows;
            let rating = x.ratings.get(&course.id).map(|(r, n)| format!("{}.{} ★ ({n})", r / 10, r % 10)).unwrap_or_else(|| "—".into());
            card("Numbers", "chart", "green", &format!(
                r#"<div class="ec-kpis lm-kpis"><div><b>{students}</b><span class="muted">students</span></div><div><b>{done}</b><span class="muted">finished</span></div><div><b>{rating}</b><span class="muted">rating</span></div><div><b>{views}</b><span class="muted">page views</span></div></div>
<p class="muted ec-hint"><a href="{STUDENTS}?course={id}">See the students</a> · created {created}</p>"#,
                rating = esc(&rating),
                views = course.views,
                id = esc(&course.id),
                created = date(&course.created_at),
            ), "lm-numbers")
        }
        None => String::new(),
    };
    let actions = match &c {
        Some(_) => format!(
            r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text" type="submit" form="lm-duplicate">{dup} Duplicate course</button><button class="btn text danger" type="submit" form="lm-delete">{del} Delete course</button></div></section>"#,
            dup = svg("copy"),
            del = svg("delete"),
        ),
        None => String::new(),
    };

    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{subtitle}
<div class="ec-sublabel">Description</div>
<div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="description_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback"><label class="field"><textarea name="description" rows="8" placeholder=" " maxlength="20000">{plain}</textarea><span>Description (plain text)</span></label></div>
</section>
{curriculum_card}{media}{promises}{seo}"#,
        title = field("title", "Course title", &get(|c| &c.title), "text", r#" required maxlength="160" id="ec-title""#),
        subtitle = field("subtitle", "Subtitle — one line under the title and on cards", &get(|c| &c.subtitle), "text", r#" maxlength="240""#),
        config = esc(&config.to_string()),
        plain = esc(&get(|c| &c.description)),
        media = card("Cover & introduction", "image", "pink", &media, "lm-media"),
        promises = card("What students get", "star", "amber", &promises, "lm-promises"),
        seo = card("Search engine listing", "search", "slate", &seo, "lm-seo"),
    );
    let side = format!("{status_card}{pricing}{org}{settings_card}{numbers}{actions}");

    let forms = match &c {
        Some(course) => {
            let id = esc(&course.id);
            let mut f = format!(
                r#"<form method="post" action="{COURSES}{id}/delete/" id="lm-delete" data-confirm="Delete “{t}”|Its lessons, quizzes, students' progress, certificates and reviews are deleted too. It cannot be undone."></form>
<form method="post" action="{COURSES}{id}/duplicate/" id="lm-duplicate"></form>
<form method="post" action="{COURSES}{id}/product/" id="lm-make-product"></form>
<form method="post" action="{COURSES}{id}/sections/" id="lm-add-section"></form>
<form method="post" action="{COURSES}{id}/lessons/" id="lm-add-loose"><input type="hidden" name="section_id" value=""></form>"#,
                t = esc(&course.title),
            );
            for sec in repo::sections(&state.db, &course.id).await {
                let sid = esc(&sec.id);
                f.push_str(&format!(
                    r#"<form method="post" action="{COURSES}{id}/lessons/" id="lm-add-{sid}"><input type="hidden" name="section_id" value="{sid}"></form>
<form method="post" action="{COURSES}{id}/sections/{sid}/delete/" id="lm-delsec-{sid}" data-confirm="Remove the section “{t}”|Its lessons stay in the course, outside any section."></form>"#,
                    t = esc(&sec.title),
                ));
            }
            f
        }
        None => String::new(),
    };
    let heading = c.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| "New course".into());
    let sub = match &c {
        Some(course) => format!("{} · {}", status.label().split(" —").next().unwrap_or(""), price_label(&s, course)),
        None => "Write the course, then build its curriculum.".into(),
    };
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{COURSES}" title="Back to courses">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="lm-course">{check} {save}</button></div></header>
{banner}{forms}
<form id="lm-course" class="ec-edit lm-edit lm-access-{access}" method="post" action="{action}">
<div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script>
<script src="{ASSETS}/react-jsx.js" defer></script>
<script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script>
<script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script>
<script src="{ASSETS}/lms-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&sub),
        check = svg("check"),
        save = if c.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        access = access.key(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

/// The course form, rebuilt: the fields through `CourseInput::from_form`,
/// the description through the post renderer.
async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::CourseInput, String> {
    let x = repo::extras(&state.db).await;
    let products = repo::store_products(&state.db).await;
    let tiers = repo::tiers(&state.db).await;
    let image = pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default();
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image").cloned().collect();
    all.push(("image".into(), image));
    let mut input = rules::CourseInput::from_form(&all, |kind, id| match kind {
        "category" => x.categories.iter().any(|c| c.id == id),
        "instructor" => x.instructors.iter().any(|i| i.id == id),
        "product" => products.iter().any(|p| p.0 == id),
        "tier" => tiers.iter().any(|t| t.0 == id),
        _ => false,
    })?;
    let store_currency = crate::ecom::store_settings(state).currency;
    let course_currency = lms_settings(state).currency;
    if [input.price_cents, input.compare_cents].into_iter().any(|price| {
        vuuut_core::currency::fixed_price_to_minor(price, &course_currency).is_none()
            || (input.product_id.is_some() && vuuut_core::currency::fixed_price_to_minor(price, &store_currency).is_none())
    }) {
        return Err("Enter whole prices for this currency.".into());
    }
    let lexical = pairs.iter().find(|(k, _)| k == "description_lexical").map(|(_, v)| v.clone()).unwrap_or_default();
    if lexical.trim().starts_with('{') {
        let lexical = vuuut_core::url::unbase_assets(&lexical);
        let html = vuuut_render::lexical::to_html(&lexical);
        if lexical.len() > rules::MAX_RICH || html.len() > rules::MAX_RICH {
            return Err("The description is too long.".into());
        }
        let plain = vuuut_render::markdown::html_to_plaintext(&html);
        input.description = vuuut_core::directory::clean_text(&plain, rules::MAX_PLAIN);
        let empty = plain.trim().is_empty() && !html.contains("<img") && !html.contains("<iframe");
        input.description_html = if empty { String::new() } else { html };
        input.description_lexical = if empty { String::new() } else { lexical };
    }
    Ok(input)
}

/// `curriculum` JSON from the builder: `{"sections":[{"id","title","lessons":[…]}],"loose":[…]}`.
/// Without the script, the section titles still arrive as `sec_title_<id>`.
async fn save_plan(state: &AppState, course_id: &str, pairs: &[(String, String)]) -> vuuut_core::Result<()> {
    let sections = repo::sections(&state.db, course_id).await;
    let lessons = repo::lessons(&state.db, course_id).await;
    let title_of = |sid: &str| pairs.iter().find(|(k, _)| k == &format!("sec_title_{sid}")).map(|(_, v)| rules::lines(v).into_iter().next().unwrap_or_default());
    let raw = pairs.iter().find(|(k, _)| k == "curriculum").map(|(_, v)| v.as_str()).unwrap_or("");
    let parsed: Option<serde_json::Value> = serde_json::from_str(raw).ok();
    let known_lesson = |id: &str| lessons.iter().any(|l| l.id == id);
    let (plan, loose): (Vec<(String, String, Vec<String>)>, Vec<String>) = match parsed {
        Some(v) => {
            let plan = v["sections"].as_array().map(|a| a.iter().filter_map(|s| {
                let id = s["id"].as_str()?.to_string();
                let sec = sections.iter().find(|x| x.id == id)?;
                let title = vuuut_core::directory::clean(s["title"].as_str().unwrap_or(""), 160);
                let ls = s["lessons"].as_array().map(|l| l.iter().filter_map(|x| x.as_str()).filter(|x| known_lesson(x)).map(str::to_string).collect()).unwrap_or_default();
                Some((id, if title.is_empty() { sec.title.clone() } else { title }, ls))
            }).collect()).unwrap_or_default();
            let loose = v["loose"].as_array().map(|l| l.iter().filter_map(|x| x.as_str()).filter(|x| known_lesson(x)).map(str::to_string).collect()).unwrap_or_default();
            (plan, loose)
        }
        None => {
            let plan = sections.iter().map(|sec| {
                let t = title_of(&sec.id).filter(|t| !t.is_empty()).unwrap_or_else(|| sec.title.clone());
                (sec.id.clone(), t, lessons.iter().filter(|l| l.section_id.as_deref() == Some(sec.id.as_str())).map(|l| l.id.clone()).collect())
            }).collect();
            let loose = lessons.iter().filter(|l| l.section_id.is_none()).map(|l| l.id.clone()).collect();
            (plan, loose)
        }
    };
    repo::save_curriculum(&state.db, course_id, &plan, &loose).await
}

pub(crate) async fn course_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{COURSES}new/"), "error", &m),
    };
    match repo::insert_course(&state.db, &input).await {
        Ok(id) => flash_to(&format!("{COURSES}{id}/"), "ok", "Course created. Now add its first section and lessons."),
        Err(e) => {
            tracing::error!("lms: creating a course failed: {}", e.message());
            flash_to(&format!("{COURSES}new/"), "error", "The course could not be saved.")
        }
    }
}

pub(crate) async fn course_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) || repo::find_course(&state.db, &id).await.is_none() {
        return flash_to(COURSES, "error", "That course could not be found.");
    }
    let back = format!("{COURSES}{id}/");
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    if let Err(e) = repo::update_course(&state.db, &id, &input).await {
        tracing::error!("lms: saving {id} failed: {}", e.message());
        return flash_to(&back, "error", "The course could not be saved.");
    }
    if let Err(e) = save_plan(&state, &id, &form).await {
        tracing::error!("lms: saving the curriculum of {id} failed: {}", e.message());
        return flash_to(&back, "error", "The course was saved, but not its curriculum order.");
    }
    if let (Some(p), "paid") = (&input.product_id, input.access.as_str()) {
        let currency = crate::ecom::store_settings(&state).currency;
        let minor = |price| vuuut_core::currency::fixed_price_to_minor(price, &currency);
        if let (Some(price), Some(compare)) = (minor(input.price_cents), minor(input.compare_cents)) {
            repo::sync_product_price(&state.db, p, price, compare).await;
        }
    }
    flash_to(&back, "ok", "Saved.")
}

pub(crate) async fn course_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(COURSES, "error", "That course could not be found.");
    }
    match repo::delete_course(&state.db, &id).await {
        Ok(()) => flash_to(COURSES, "ok", "Course deleted."),
        Err(_) => flash_to(COURSES, "error", "That could not be deleted."),
    }
}

pub(crate) async fn course_duplicate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(COURSES, "error", "That course could not be found.");
    }
    match repo::duplicate_course(&state.db, &id).await {
        Ok(Some(new_id)) => flash_to(&format!("{COURSES}{new_id}/"), "ok", "Duplicated as a draft, with its curriculum and no students."),
        _ => flash_to(&format!("{COURSES}{id}/"), "error", "That could not be duplicated."),
    }
}

/// A digital product in the store, priced like the course, linked to it,
/// with the course switched to Paid.
pub(crate) async fn course_make_product(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(COURSES, "error", "That course could not be found.");
    }
    let Some(c) = repo::find_course(&state.db, &id).await else { return flash_to(COURSES, "error", "That course could not be found.") };
    let back = format!("{COURSES}{id}/");
    let price = if c.price_cents > 0 { c.price_cents } else { 4900 };
    let p = vuuut_db::repos::ecom::ProductInput {
        handle: vuuut_core::directory::slugify(&format!("{} course", c.title)),
        title: c.title.clone(),
        status: "active".into(),
        description_lexical: c.description_lexical.clone(),
        description_html: c.description_html.clone(),
        excerpt: c.subtitle.clone(),
        product_type: "Online course".into(),
        tags: vec!["course".into()],
        images: if c.image.is_empty() { Vec::new() } else { vec![c.image.clone()] },
        taxable: true,
        ..Default::default()
    };
    let currency = crate::ecom::store_settings(&state).currency;
    let minor = |amount| vuuut_core::currency::fixed_price_to_minor(amount, &currency);
    let (Some(store_price), Some(store_compare)) = (minor(price), minor(c.compare_cents)) else {
        return flash_to(&back, "error", "Enter whole prices for the store currency.");
    };
    let v = vuuut_db::repos::ecom::VariantInput { price_cents: store_price, compare_at_cents: store_compare, ..Default::default() };
    match vuuut_db::repos::ecom::save_product(&state.db, None, &p, &[v], &[]).await {
        Ok(pid) => {
            let mut input = repo::input_of(&c, &c.title);
            input.slug = c.slug.clone();
            input.status = c.status.clone();
            input.featured = c.featured != 0;
            input.opens_at = c.opens_at.clone();
            input.closes_at = c.closes_at.clone();
            input.access = "paid".into();
            input.price_cents = price;
            input.product_id = Some(pid);
            match repo::update_course(&state.db, &id, &input).await {
                Ok(()) => flash_to(&back, "ok", "A store product was created and linked. Buying it enrols the buyer."),
                Err(_) => flash_to(&back, "error", "The product was created but could not be linked."),
            }
        }
        Err(e) => {
            tracing::error!("lms: creating a product for {id} failed: {}", e.message());
            flash_to(&back, "error", "The store product could not be created.")
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct TitleForm {
    title: String,
    kind: String,
    section_id: String,
}

pub(crate) async fn section_add(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<TitleForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) || repo::find_course(&state.db, &id).await.is_none() {
        return flash_to(COURSES, "error", "That course could not be found.");
    }
    let back = format!("{COURSES}{id}/");
    let title = vuuut_core::directory::clean(&f.title, 160);
    let n = repo::sections(&state.db, &id).await.len();
    let title = if title.is_empty() { format!("Section {}", n + 1) } else { title };
    match repo::add_section(&state.db, &id, &title).await {
        Ok(_) => flash_to(&format!("{back}#lm-curriculum-card"), "ok", &format!("Section “{title}” added.")),
        Err(_) => flash_to(&back, "error", "The section could not be added."),
    }
}

pub(crate) async fn section_delete(State(state): State<AppState>, headers: HeaderMap, Path((id, sid)): Path<(String, String)>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) || !vuuut_core::ids::is_objectid(&sid) {
        return flash_to(COURSES, "error", "That section could not be found.");
    }
    match repo::delete_section(&state.db, &id, &sid).await {
        Ok(()) => flash_to(&format!("{COURSES}{id}/"), "ok", "Section removed. Its lessons are now outside any section."),
        Err(_) => flash_to(&format!("{COURSES}{id}/"), "error", "That could not be removed."),
    }
}

pub(crate) async fn lesson_add(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<TitleForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) || repo::find_course(&state.db, &id).await.is_none() {
        return flash_to(COURSES, "error", "That course could not be found.");
    }
    let back = format!("{COURSES}{id}/");
    let kind = LessonKind::from_key(&f.kind);
    let title = vuuut_core::directory::clean(&f.title, 160);
    let title = if title.chars().count() < 2 { format!("New {}", kind.label().to_lowercase()) } else { title };
    let sections = repo::sections(&state.db, &id).await;
    let section_id = Some(f.section_id.clone()).filter(|s| sections.iter().any(|x| &x.id == s));
    let input = rules::LessonInput {
        slug: vuuut_core::directory::slugify(&title),
        title,
        kind: kind.key().into(),
        status: "published".into(),
        section_id,
        pass_mark: 70,
        reveal: true,
        points: 100,
        ..Default::default()
    };
    match repo::insert_lesson(&state.db, &id, &input).await {
        Ok(lid) => flash_to(&format!("{LESSONS}{lid}/"), "ok", &format!("{} added. Write it here.", kind.label())),
        Err(e) => {
            tracing::error!("lms: adding a lesson to {id} failed: {}", e.message());
            flash_to(&back, "error", "That could not be added.")
        }
    }
}
