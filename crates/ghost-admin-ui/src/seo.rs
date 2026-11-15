//! The SEO module: what a search engine sees, and what is stopping it.
//!
//! Two screens under `/ghost/seo/`:
//!
//! * **Overview** — the site's own setup, checked one line at a time, with a
//!   link to the thing that fixes each line; then the findings across all the
//!   content, counted and ordered by how much they cost.
//! * **Content** — every post and page with its score and its findings,
//!   filterable by finding, so "the eleven posts with no meta description" is
//!   a list you can work through rather than a number.
//!
//! The site-wide switches live under Settings → SEO, with the other settings
//! ([`settings_page`]).
//!
//! ## There is no SEO data
//!
//! This module stores nothing. Every number on both screens is computed from
//! the posts as they are at the moment the page is asked for — see
//! [`ghost_db::repos::seo`] for why that is the only honest arrangement, and
//! [`ghost_core::seo`] for the rules themselves. A report cannot disagree
//! with the editor, go stale, or need invalidating, and there is no migration
//! for the module at all.
//!
//! The one thing it does write is the six settings its own page offers, into
//! `settings` with the other site settings. None of them is a secret, so none
//! of them needs a config table of its own.
//!
//! ## Who may see it
//!
//! Administrators, like every other module. The report is derived entirely
//! from content the site publishes — there is no personal data in it — but it
//! names draft and scheduled entries, and the settings it offers can change
//! how the whole site is indexed.
//!
//! Everything interpolated into these pages goes through `esc`, including the
//! findings, whose messages are built from entry titles and user-chosen
//! lengths. The one query parameter that comes back as a link (`issue`) is
//! reduced to a code from [`ghost_core::seo::CHECKS`] before it is used, so a
//! forged one collapses to "everything" rather than reaching the page.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Form;
use ghost_api::AppState;
use ghost_core::seo as rules;
use ghost_core::seo::{Entry, Finding, Severity};
use ghost_db::repos::seo as repo;
use std::collections::HashMap;

use crate::{esc, flash_banner, page_for_role, require_role, select_options, svg, Actor, Role, SELECT_CARET};

/// The most rows the content screen shows at once. The audit itself is
/// bounded by [`repo::ENTRY_LIMIT`]; this is just how much of it is painted.
const PER_PAGE: usize = 100;

/// The role check, plus — for anything that changes state — the origin check.
async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn server_error(e: &ghost_core::GhostError) -> Response {
    tracing::error!("seo admin: {}", e.message());
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

// ---------------------------------------------------------------------------
// the site's own setup
// ---------------------------------------------------------------------------

/// One line of the Overview's first card.
pub(crate) struct SiteCheck {
    pub(crate) ok: bool,
    pub(crate) title: &'static str,
    pub(crate) detail: String,
    /// Where the thing that fixes it lives. `None` for a line that is only
    /// ever good news.
    pub(crate) fix: Option<(&'static str, &'static str)>,
}

/// The settings the site-level checks read, lifted out of `AppState` so the
/// rules below are a pure function of them and can be tested without a
/// server. Every field is already trimmed.
#[derive(Default)]
pub(crate) struct SiteSnapshot {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) meta_description: String,
    pub(crate) icon: String,
    pub(crate) logo: String,
    pub(crate) social_image: String,
    pub(crate) socials: usize,
    pub(crate) site_url: String,
    pub(crate) structured_data: bool,
    pub(crate) llms: bool,
    pub(crate) indexnow: bool,
    pub(crate) is_private: bool,
    pub(crate) custom_robots: bool,
    pub(crate) noindex_archives: bool,
}

/// The checks about the site rather than about one entry.
///
/// Ordered by what it costs to get wrong: a private site is not indexed at
/// all, so that comes first and says so plainly rather than being reported as
/// a mistake — a site can be private on purpose, and the report's job is to
/// tell you what is true, not to argue.
pub(crate) fn site_checks(s: &SiteSnapshot) -> Vec<SiteCheck> {
    let mut out = Vec::new();
    let mut add = |ok: bool, title: &'static str, detail: String, fix: Option<(&'static str, &'static str)>| {
        out.push(SiteCheck { ok, title, detail, fix });
    };

    if s.is_private {
        add(
            false,
            "This site is private",
            "Every page answers `noindex` and robots.txt disallows everything, so nothing below will be indexed while that is on.".into(),
            Some(("Membership settings", "/ghost/settings/membership/")),
        );
    }
    add(
        !s.title.is_empty(),
        "Site title",
        if s.title.is_empty() {
            "The site has no title, so archives and the home page have nothing to be titled with.".into()
        } else {
            format!("“{}”", s.title)
        },
        Some(("General settings", "/ghost/settings/general/")),
    );
    let description = if s.meta_description.is_empty() { &s.description } else { &s.meta_description };
    add(
        !description.is_empty(),
        "Home page description",
        if description.is_empty() {
            "No site description or SEO description, so the home page's search snippet is whatever a crawler picks.".into()
        } else {
            format!("{} characters", description.chars().count())
        },
        Some(("General settings", "/ghost/settings/general/")),
    );
    add(
        !s.icon.is_empty(),
        "Publication icon",
        if s.icon.is_empty() {
            "No icon, so no `rel=icon` is published and search results fall back to a generic mark.".into()
        } else {
            "Published as this site's icon in a browser tab and in search results.".into()
        },
        Some(("Brand settings", "/ghost/settings/design/")),
    );
    add(
        !s.social_image.is_empty(),
        "Default social image",
        if s.social_image.is_empty() {
            "Nothing to show when a page with no feature image of its own is shared.".into()
        } else {
            "Used by any page that has no image of its own.".into()
        },
        Some(("Meta data", "/ghost/settings/general/")),
    );
    add(
        !s.logo.is_empty(),
        "Publisher logo",
        if s.logo.is_empty() {
            "No logo, so the structured data names this site as a publisher with no mark. The icon is used if there is one.".into()
        } else {
            "Carried in the schema.org publisher block.".into()
        },
        Some(("Brand settings", "/ghost/settings/design/")),
    );
    add(
        s.socials > 0,
        "Social accounts",
        if s.socials == 0 {
            "No accounts linked, so `article:publisher` and `twitter:site` are not published.".into()
        } else {
            format!("{} linked", crate::countable(s.socials as i64, "account"))
        },
        Some(("Social accounts", "/ghost/settings/general/")),
    );
    add(
        s.structured_data,
        "Structured data",
        if s.structured_data {
            "Open Graph, Twitter cards and schema.org JSON-LD are published on every page.".into()
        } else {
            "Switched off, so no page carries Open Graph tags, a Twitter card or a schema.org block.".into()
        },
        Some(("SEO settings", "/ghost/settings/seo/")),
    );
    add(
        s.site_url.starts_with("https://"),
        "Canonical URLs are HTTPS",
        if s.site_url.starts_with("https://") {
            format!("Every canonical and social URL is built from {}", s.site_url)
        } else {
            format!("The site URL is {} — every canonical URL on the site points at plain HTTP.", s.site_url)
        },
        None,
    );
    add(
        true,
        "robots.txt",
        if s.custom_robots {
            "Serving a custom file from Settings → SEO.".into()
        } else {
            "Serving the default, which names the sitemap and blocks only the admin and the machinery.".into()
        },
        Some(("Open robots.txt", "/robots.txt")),
    );
    add(true, "Sitemaps", "Posts, pages, tags and authors, each dated and listed in the index.".into(), Some(("Open the sitemap", "/sitemap.xml")));
    add(
        s.indexnow,
        "IndexNow",
        if s.indexnow {
            "Published pages are submitted to Bing, Yandex and the other participating engines as soon as they go out.".into()
        } else {
            "No key, so nothing is submitted; engines find new posts on their own schedule.".into()
        },
        Some(("SEO settings", "/ghost/settings/seo/")),
    );
    add(
        s.llms,
        "Markdown for AI tools",
        if s.llms {
            "Every post advertises a Markdown source, and /llms.txt lists them.".into()
        } else {
            "Off, so there is no `llms.txt` and no Markdown alternate on a post.".into()
        },
        Some(("Meta data", "/ghost/settings/general/")),
    );
    if s.noindex_archives {
        add(
            false,
            "Some archives are hidden",
            "A tag, author or paged archive is being told not to index itself. That is a choice, but it is one worth remembering.".into(),
            Some(("SEO settings", "/ghost/settings/seo/")),
        );
    }
    out
}

fn snapshot(state: &AppState) -> SiteSnapshot {
    let s = |key: &str| state.settings.get_str(key).map(|v| v.trim().to_string()).unwrap_or_default();
    // The SEO flags are read through the same function the renderer uses, so
    // the report cannot claim one thing while the pages do another.
    let flags = serde_json::json!({
        "seo_structured_data": state.settings.get("seo_structured_data"),
        "seo_noindex_tag_pages": state.settings.get("seo_noindex_tag_pages"),
        "seo_noindex_author_pages": state.settings.get("seo_noindex_author_pages"),
        "seo_noindex_paged_pages": state.settings.get("seo_noindex_paged_pages"),
    });
    let socials = ghost_render::social::SOCIAL_PLATFORMS
        .iter()
        .filter(|(_, key, _)| !s(key).is_empty())
        .count();
    SiteSnapshot {
        title: s("title"),
        description: s("description"),
        meta_description: s("meta_description"),
        icon: s("icon"),
        logo: s("logo"),
        social_image: if s("og_image").is_empty() { s("cover_image") } else { s("og_image") },
        socials,
        site_url: state.site_url(),
        structured_data: rules::flag(&flags, "seo_structured_data"),
        llms: crate::settings::llms_enabled(state),
        indexnow: ghost_api::announce::indexnow_key(state).is_some(),
        is_private: state.settings.get_bool("is_private"),
        custom_robots: !s("seo_robots_txt").is_empty(),
        noindex_archives: ["seo_noindex_tag_pages", "seo_noindex_author_pages", "seo_noindex_paged_pages"]
            .iter()
            .any(|key| rules::flag(&flags, key)),
    }
}

// ---------------------------------------------------------------------------
// shared rendering
// ---------------------------------------------------------------------------

fn tabs(active: &str, issues: usize) -> String {
    let tab = |key: &str, href: &str, label: &str, count: Option<usize>| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{count}</a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
            count = count
                .map(|n| format!(r#"<span class="seg-count">{n}</span>"#))
                .unwrap_or_else(|| r#"<span style="width:8px"></span>"#.into()),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{overview}{content}</div>"#,
        overview = tab("overview", "/ghost/seo/", "Overview", None),
        content = tab("content", "/ghost/seo/content/", "Content", Some(issues)),
    )
}

/// The page head, with the re-run control.
///
/// There is no "re-run" in the sense of starting a job, because there is no
/// job: every number on these screens is computed from the posts during the
/// request that renders them. The button is a link back to the same screen,
/// and the line beside it says so — without it the screens look like a cached
/// report with no way to refresh, which is exactly how it read to the first
/// person who opened one.
fn head(here: &str, actions: &str) -> String {
    format!(
        r#"<header class="page-head rise"><div><h1>SEO</h1>
<p class="sub">What a search engine sees on this site, and what is stopping it.</p></div>
<div class="actions"><a class="btn tonal" href="{here}">{refresh} Re-run audit</a>{actions}</div></header>"#,
        here = esc(here),
        refresh = svg("repeat"),
    )
}

/// The line under the tiles that explains what the numbers are.
///
/// `at` is the clock time the audit ran, in the site's timezone — the same
/// timezone every other date in the admin is shown in.
fn freshness(state: &AppState, audited: usize, total: i64) -> String {
    let tz = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let now = ghost_core::dates::clock_in_zone(&ghost_core::dates::now_db(), &tz);
    let scope = if total > audited as i64 {
        format!(
            " It covers the {} most recent of {}; older entries are not audited.",
            audited, total
        )
    } else {
        String::new()
    };
    format!(
        r#"<p class="muted rise" style="--i:3;margin:-4px 0 18px">Checked at {now} — nothing here is stored or cached, so the audit runs again every time this page is opened.{scope}</p>"#,
        now = esc(&now),
        scope = esc(&scope),
    )
}

/// The score ring's colour band. Three bands rather than a gradient: the
/// number is an estimate, and a colour that changes at every point implies a
/// precision it does not have.
fn band(score: u8) -> &'static str {
    match score {
        90..=100 => "green",
        60..=89 => "amber",
        _ => "red",
    }
}

fn severity_chip(severity: Severity, label: &str) -> String {
    let class = match severity {
        Severity::Error => "error",
        Severity::Warning => "draft",
    };
    format!(r#"<span class="chip {class}">{}</span>"#, esc(label))
}

fn tile(label: &str, value: &str, detail: &str) -> String {
    format!(
        r#"<div class="tile"><div class="label">{label}</div><div class="value">{value}</div>
<div class="delta">{detail}</div></div>"#,
        label = esc(label),
        value = esc(value),
        detail = esc(detail),
    )
}

/// The audit, loaded and run. Both screens need all of it: the overview
/// counts the findings and the content screen lists them.
async fn report(state: &AppState) -> Result<(Vec<Entry>, Vec<Vec<Finding>>, i64), ghost_core::GhostError> {
    let mut entries = repo::entries(&state.db, repo::ENTRY_LIMIT).await?;
    // NULL is the default language, which only the settings know; filled in
    // so the audit compares an English post stored as NULL with one stored
    // as `en`, and never with a Bengali one.
    let locales = crate::settings::site_locales(state);
    for entry in &mut entries {
        entry.locale = Some(locales.resolve(entry.locale.as_deref()));
    }
    let total = repo::total(&state.db).await?;
    let findings: Vec<Vec<Finding>> = rules::audit_all(&mut entries).into_iter().map(|(_, f)| f).collect();
    Ok((entries, findings, total))
}

// ---------------------------------------------------------------------------
// overview
// ---------------------------------------------------------------------------

pub async fn seo_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (entries, findings, total) = match report(&state).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };

    let audited = entries.len();
    let flagged = findings.iter().filter(|f| !f.is_empty()).count();
    let errors: usize = findings.iter().flatten().filter(|f| f.severity == Severity::Error).count();
    let warnings: usize = findings.iter().flatten().filter(|f| f.severity == Severity::Warning).count();
    // The mean of the per-entry scores, which is the only average that means
    // anything here: a total would just be a restatement of how much has been
    // published.
    let average = if audited == 0 {
        100
    } else {
        (findings.iter().map(|f| rules::score(f) as usize).sum::<usize>() / audited) as u8
    };

    let setup = snapshot(&state);
    let checks = site_checks(&setup);
    let setup_rows: String = checks
        .iter()
        .map(|c| {
            let (icon, tint, chip) = if c.ok {
                ("check", "green", r#"<span class="chip published">good</span>"#.to_string())
            } else {
                ("warning", "amber", r#"<span class="chip draft">worth a look</span>"#.to_string())
            };
            let action = c
                .fix
                .map(|(label, href)| {
                    format!(
                        r#"<a class="btn tonal small" href="{href}"{target}>{label}</a>"#,
                        href = esc(href),
                        // The two links that leave the admin are the public
                        // files, and they are the only ones opened in a tab.
                        target = if href.starts_with("/ghost/") { "" } else { r#" target="_blank" rel="noopener""# },
                        label = esc(label),
                    )
                })
                .unwrap_or_default();
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing">{chip}{action}</div></div>"#,
                icon = svg(icon),
                title = esc(c.title),
                detail = esc(&c.detail),
            )
        })
        .collect();

    // What to fix first: each check that fired, with how many entries it
    // fired on, in the order `CHECKS` lists them — which is the order of how
    // much each one costs.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for finding in findings.iter().flatten() {
        *counts.entry(finding.code).or_insert(0) += 1;
    }
    let issue_rows: String = rules::CHECKS
        .iter()
        .filter_map(|(code, severity, label)| {
            let n = *counts.get(code)?;
            if n == 0 {
                return None;
            }
            Some(format!(
                r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><a class="title" style="color:inherit;text-decoration:none" href="/ghost/seo/content/?issue={code}">{label}</a>
<span class="detail">{count}</span></div>
<div class="trailing">{chip}<a class="btn tonal small" href="/ghost/seo/content/?issue={code}">Show them</a></div></div>"#,
                tint = if *severity == Severity::Error { "red" } else { "amber" },
                icon = svg(if *severity == Severity::Error { "warning" } else { "excerpt" }),
                code = esc(code),
                label = esc(label),
                count = esc(&crate::countable(n as i64, "entry").replace("entrys", "entries")),
                chip = severity_chip(*severity, severity.as_str()),
            ))
        })
        .collect();

    let issues_card = if issue_rows.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:4;text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">Nothing to fix</p>
<p class="muted" style="margin:0 0 12px">Every one of the {n} checked has a title, a description and an image.</p></div>"#,
            n = esc(&crate::countable(audited as i64, "entry").replace("entrys", "entries")),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:4">{issue_rows}</div>"#)
    };


    page_for_role(
        actor.role,
        "SEO",
        "seo",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}
<div class="tiles rise" style="--i:2">{tiles}</div>
{freshness}
<div class="section">Site setup</div>
<div class="card rows rise" style="--i:3">{setup_rows}</div>
<div class="section">What to fix first</div>
{issues_card}"#,
            head = head(
                "/ghost/seo/",
                &format!(
                    r#"<a class="btn tonal" href="/ghost/settings/seo/">{gear} SEO settings</a>"#,
                    gear = svg("settings")
                )
            ),
            freshness = freshness(&state, audited, total),
            tabs = tabs("overview", flagged),
            tiles = format!(
                "{}{}{}{}",
                tile("Average score", &average.to_string(), match band(average) {
                    "green" => "Nothing much to do",
                    "amber" => "Some pages need a look",
                    _ => "Most pages need a look",
                }),
                tile("Audited", &audited.to_string(), "Posts and pages, newest first"),
                tile("Errors", &errors.to_string(), "Things a crawler cannot work around"),
                tile("Warnings", &warnings.to_string(), "Things it will handle less well"),
            ),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// content
// ---------------------------------------------------------------------------

/// The query parameters, reduced to values this module offers.
///
/// `issue` becomes a link and a filter, so it is matched against
/// [`rules::CHECKS`] rather than trusted; `kind` and `state` likewise. A
/// forged value collapses to "everything", never into the page.
fn filters(q: &HashMap<String, String>) -> (Option<&'static str>, &'static str, &'static str) {
    let issue = q
        .get("issue")
        .and_then(|raw| rules::CHECKS.iter().find(|(code, _, _)| code == raw).map(|(code, _, _)| *code));
    let kind = match q.get("type").map(String::as_str) {
        Some("post") => "post",
        Some("page") => "page",
        _ => "all",
    };
    let published = match q.get("state").map(String::as_str) {
        Some("published") => "published",
        Some("unpublished") => "unpublished",
        _ => "all",
    };
    (issue, kind, published)
}

pub async fn content_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (entries, findings, total) = match report(&state).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };
    let (issue, kind, published) = filters(&q);
    let locales = crate::settings::site_locales(&state);
    // Re-running from this screen keeps the filters, which is why the link is
    // rebuilt from the reduced values rather than from the query string as it
    // arrived — a forged parameter must not survive into an href.
    let here = format!(
        "/ghost/seo/content/?issue={}&type={}&state={}",
        issue.unwrap_or(""),
        kind,
        published
    );
    let flagged = findings.iter().filter(|f| !f.is_empty()).count();

    let mut rows: Vec<(usize, &Entry, &Vec<Finding>)> = entries
        .iter()
        .zip(findings.iter())
        .enumerate()
        .filter(|(_, (entry, found))| {
            (kind == "all" || entry.kind == kind)
                && (published == "all"
                    || (published == "published") == entry.is_published())
                && issue.is_none_or(|code| found.iter().any(|f| f.code == code))
        })
        .map(|(i, (entry, found))| (i, entry, found))
        .collect();
    // Worst first: the point of the screen is the next thing to fix.
    rows.sort_by_key(|(i, _, found)| (rules::score(found), *i));
    let matched = rows.len();
    rows.truncate(PER_PAGE);

    let list: String = rows
        .iter()
        .map(|(_, entry, found)| {
            let score = rules::score(found);
            let chips: String = found
                .iter()
                .map(|f| {
                    format!(
                        r#"<span class="chip {class}" title="{why}">{label}</span>"#,
                        class = if f.severity == Severity::Error { "error" } else { "draft" },
                        why = esc(&f.message),
                        label = esc(rules::check_label(f.code)),
                    )
                })
                .collect();
            let chips = if chips.is_empty() {
                r#"<span class="chip published">nothing to fix</span>"#.to_string()
            } else {
                chips
            };
            let title = if entry.title.trim().is_empty() { "(untitled)" } else { entry.title.trim() };
            // The editor is at /ghost/posts/{id}/ or /ghost/pages/{id}/, and
            // the id is only linked once it is the shape this install issues.
            let href = ghost_core::ids::is_objectid(&entry.id)
                .then(|| format!("/ghost/{}s/{}/", esc(&entry.kind), esc(&entry.id)));
            let linked = match &href {
                Some(href) => format!(
                    r#"<a class="title" style="color:inherit;text-decoration:none" href="{href}">{title}</a>"#,
                    title = esc(title)
                ),
                None => format!(r#"<span class="title">{}</span>"#, esc(title)),
            };
            let edit = href
                .as_deref()
                .map(|href| {
                    format!(
                        r#"<a class="icon-btn" href="{href}" title="Edit" aria-label="Edit">{}</a>"#,
                        svg("edit")
                    )
                })
                .unwrap_or_default();
            format!(
                r#"<div class="row"><span class="ic seo-score tint-{tint}" title="SEO score">{score}</span>
<div class="body">{linked}<span class="detail">{lang}{kind} · {prefix}/{slug}/ · {status}</span>
<div class="chips seo-findings">{chips}</div></div>
<div class="trailing">{edit}</div></div>"#,
                tint = band(score),
                score = score,
                kind = esc(if entry.kind == "page" { "Page" } else { "Post" }),
                slug = esc(&entry.slug),
                status = esc(&entry.status),
                // On a multilingual site, which language — and the prefix
                // its URL really carries.
                lang = match entry.locale.as_deref().filter(|_| locales.multilingual()) {
                    Some(code) => format!(r#"<span class="chip tonal">{}</span> "#, esc(code)),
                    None => String::new(),
                },
                prefix = esc(&entry.locale.as_deref().map(|c| locales.prefix(c)).unwrap_or_default()),
            )
        })
        .collect();

    let list = if rows.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3;text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">Nothing matches</p>
<p class="muted" style="margin:0">{what}</p></div>"#,
            what = match issue {
                Some(code) => format!("No entry has “{}”.", esc(rules::check_label(code))),
                None => "There is nothing published or drafted yet.".to_string(),
            },
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:3">{list}</div>"#)
    };

    let issue_options: Vec<(&str, &str)> = std::iter::once(("", "Any issue"))
        .chain(rules::CHECKS.iter().map(|(code, _, label)| (*code, *label)))
        .collect();
    let bar = format!(
        r#"<form class="seo-bar rise" style="--i:2" method="get" action="/ghost/seo/content/">
<label class="field fixed"><select name="issue" onchange="this.form.submit()">{issues}</select><span>Issue</span>{car}</label>
<label class="field fixed"><select name="type" onchange="this.form.submit()">{kinds}</select><span>Type</span>{car}</label>
<label class="field fixed"><select name="state" onchange="this.form.submit()">{states}</select><span>Status</span>{car}</label>
<noscript><button class="btn tonal" type="submit">Filter</button></noscript>
</form>"#,
        issues = select_options(&issue_options, issue.unwrap_or("")),
        kinds = select_options(&[("all", "Posts and pages"), ("post", "Posts"), ("page", "Pages")], kind),
        states = select_options(
            &[("all", "Any status"), ("published", "Published"), ("unpublished", "Draft or scheduled")],
            published
        ),
        car = SELECT_CARET,
    );

    let more = if matched > PER_PAGE {
        format!(
            r#"<p class="muted rise" style="--i:4;margin:14px 0 0">Showing the {PER_PAGE} that need the most work, of {matched} matching.</p>"#,
        )
    } else {
        String::new()
    };
    let freshness = freshness(&state, entries.len(), total);

    page_for_role(
        actor.role,
        "SEO",
        "seo",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{freshness}{bar}{list}{more}"#,
            head = head(
                &here,
                &format!(
                    r#"<a class="btn tonal" href="/ghost/settings/seo/">{gear} SEO settings</a>"#,
                    gear = svg("settings")
                )
            ),
            tabs = tabs("content", flagged),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub async fn settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let group = crate::SETTINGS_GROUPS.iter().find(|g| g.slug == "seo").expect("seo settings group");
    let flags = serde_json::json!({
        "seo_structured_data": state.settings.get("seo_structured_data"),
        "seo_append_site_title": state.settings.get("seo_append_site_title"),
        "seo_sitemap_images": state.settings.get("seo_sitemap_images"),
        "seo_noindex_tag_pages": state.settings.get("seo_noindex_tag_pages"),
        "seo_noindex_author_pages": state.settings.get("seo_noindex_author_pages"),
        "seo_noindex_paged_pages": state.settings.get("seo_noindex_paged_pages"),
        "seo_referrer_policy": state.settings.get("seo_referrer_policy"),
    });
    let on = |key: &str| rules::flag(&flags, key);
    let switch = |name: &str, icon: &str, tint: &str, title: &str, detail: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="{name}" value="1"{ck} form="seo-settings"><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            icon = svg(icon),
            title = esc(title),
            detail = esc(detail),
            name = esc(name),
            ck = if on(name) { " checked" } else { "" },
        )
    };
    let robots = state.settings.get_str("seo_robots_txt").unwrap_or_default();
    let indexnow = state.settings.get_str("indexnow_api_key").unwrap_or_default();

    page_for_role(
        actor.role,
        "SEO",
        "settings",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<p class="rise" style="--i:1"><a class="btn tonal" href="/ghost/seo/">{chart} Open the SEO report</a></p>
<form method="post" action="/ghost/settings/seo/" id="seo-settings">
<div class="section">Search appearance</div>
<div class="card rows rise" style="--i:2">
{structured}
{append}
<div class="row"><span class="ic tint-slate">{lock}</span>
<div class="body"><span class="title">Referrer policy</span><span class="detail">What this site tells other sites about where a visitor came from</span></div>
<div class="trailing"><div class="selbox"><select name="seo_referrer_policy" aria-label="Referrer policy" form="seo-settings">{policies}</select>{car}</div></div></div>
</div>

<div class="section">Crawling</div>
<div class="card rows rise" style="--i:3">
{sitemap_images}
{noindex_tags}
{noindex_authors}
{noindex_paged}
</div>

<div class="section">robots.txt</div>
<div class="card padded rise" style="--i:4">
<label class="field" style="margin:0 0 8px"><textarea name="seo_robots_txt" rows="8" placeholder=" " maxlength="{robots_max}" spellcheck="false" style="font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px">{robots}</textarea><span>Custom robots.txt</span></label>
<p class="muted" style="margin:0 0 6px">Leave this empty to serve the default, which names the sitemap and blocks only <code>/ghost/</code> and the machinery. Anything written here <b>replaces the default completely</b>, including the <code>Sitemap:</code> line.</p>
<p class="muted" style="margin:0"><a href="/robots.txt" target="_blank" rel="noopener">Open the file this site is serving now</a> · <a href="/sitemap.xml" target="_blank" rel="noopener">Open the sitemap</a></p>
</div>

<div class="section">IndexNow</div>
<div class="card padded rise" style="--i:5">
<label class="field" style="margin:0 0 8px"><input name="indexnow_api_key" value="{indexnow}" placeholder=" " maxlength="128" spellcheck="false" autocomplete="off"><span>IndexNow key</span></label>
<p class="muted" style="margin:0 0 6px">With a key here, every post is submitted to Bing, Yandex and the other participating engines the moment it is published, instead of waiting to be crawled. 8–128 letters, numbers and hyphens; the site serves it back at <code>/&lt;key&gt;.txt</code>, which is how an engine checks that the key is yours.</p>
<p class="muted" style="margin:0">Leave it empty to submit nothing.{current_key}</p>
</div>
</form>"#,
            head = crate::group_head(group, Some("seo-settings")),
            banner = flash_banner(&q),
            chart = svg("chart"),
            lock = svg("lock"),
            car = SELECT_CARET,
            robots_max = rules::ROBOTS_MAX,
            robots = esc(&robots),
            indexnow = esc(&indexnow),
            current_key = if indexnow.is_empty() {
                String::new()
            } else {
                format!(
                    r#" This site is serving <a href="{path}" target="_blank" rel="noopener">{path}</a>."#,
                    path = esc(&ghost_api::announce::key_file_path(&indexnow))
                )
            },
            policies = select_options(rules::REFERRER_POLICIES, rules::referrer_policy(&flags)),
            structured = switch(
                "seo_structured_data",
                "share",
                "green",
                "Structured data",
                "Open Graph, Twitter cards and schema.org JSON-LD on every page",
            ),
            append = switch(
                "seo_append_site_title",
                "title",
                "blue",
                "Append the site title to page titles",
                "“A post - My Site” instead of “A post”. Archives always carry it.",
            ),
            sitemap_images = switch(
                "seo_sitemap_images",
                "image",
                "purple",
                "List images in the sitemap",
                "Feature images, so a site's pictures reach image search",
            ),
            noindex_tags = switch(
                "seo_noindex_tag_pages",
                "tags",
                "amber",
                "Hide tag archives from search",
                "Adds noindex to /tag/… pages. Links on them are still followed.",
            ),
            noindex_authors = switch(
                "seo_noindex_author_pages",
                "person",
                "amber",
                "Hide author archives from search",
                "Adds noindex to /author/… pages. Links on them are still followed.",
            ),
            noindex_paged = switch(
                "seo_noindex_paged_pages",
                "pages",
                "amber",
                "Hide page 2 and beyond from search",
                "Adds noindex to /page/2/ and onwards. The posts on them stay indexable.",
            ),
        ),
        true,
    )
    .into_response()
}

/// Bound and clean a pasted robots.txt.
///
/// Rebuilt rather than stored as sent, which is rule 2 of the module pattern:
/// control characters are dropped, lines are trimmed of trailing space, and
/// the whole thing is cut to [`rules::ROBOTS_MAX`]. The file is written out as
/// `text/plain` with no interpolation, so nothing in it can become markup —
/// but a stray `\r` or a NUL in a directive is still a directive that no
/// crawler will honour, and silently serving one is worse than refusing it.
pub(crate) fn clean_robots(raw: &str) -> String {
    let cleaned: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|line| line.chars().filter(|c| !c.is_control()).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    cleaned.chars().take(rules::ROBOTS_MAX).collect::<String>().trim().to_string()
}

pub async fn settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let mut ok = true;
    // A checkbox that is off is absent from the post, so every switch is
    // written on every save rather than only the ones that arrived.
    for (key, _) in rules::BOOL_SETTINGS {
        let value = posted.get(*key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(value)).await.is_ok();
    }

    // Only a policy we offered. An unknown one would reach every page.
    let policy = posted.get("seo_referrer_policy").map(String::as_str).unwrap_or("");
    let policy = rules::referrer_policy(&serde_json::json!({"seo_referrer_policy": policy}));
    ok &= state.settings.set(&state.db, "seo_referrer_policy", &serde_json::json!(policy)).await.is_ok();

    let robots = clean_robots(posted.get("seo_robots_txt").map(String::as_str).unwrap_or(""));
    ok &= state.settings.set(&state.db, "seo_robots_txt", &serde_json::json!(robots)).await.is_ok();

    // The IndexNow key is also a URL path on this site, so it is accepted
    // only in the shape `announce::indexnow_key` will later agree to use —
    // otherwise the field would appear saved while nothing was submitted.
    let key = posted.get("indexnow_api_key").map(|k| k.trim().to_string()).unwrap_or_default();
    if !key.is_empty() && !((8..=128).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        return crate::settings_flash(
            "seo",
            "error",
            "An IndexNow key must be 8–128 letters, numbers or hyphens. Nothing was saved.",
        );
    }
    ok &= state.settings.set(&state.db, "indexnow_api_key", &serde_json::json!(key)).await.is_ok();

    if ok {
        crate::settings_flash("seo", "ok", "SEO settings saved.")
    } else {
        crate::settings_flash("seo", "error", "Saving failed — check the logs.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, description: Option<&str>) -> Entry {
        Entry {
            id: "6553f7a1b2c3d4e5f6a7b8c9".into(),
            title: title.into(),
            slug: "a-post".into(),
            kind: "post".into(),
            status: "published".into(),
            visibility: "public".into(),
            meta_description: description.map(str::to_string),
            feature_image: Some("/content/images/x.jpg".into()),
            feature_image_alt: Some("Alt".into()),
            word_count: 800,
            tag_count: 1,
            ..Entry::default()
        }
    }

    #[test]
    fn a_forged_filter_collapses_to_everything() {
        // `issue` comes back as a link and as a filter. Anything that is not
        // a known check must not survive to either.
        for hostile in [r#"" onload="alert(1)"#, "../../etc/passwd", "no_description'; DROP TABLE"] {
            let q = HashMap::from([("issue".to_string(), hostile.to_string())]);
            assert_eq!(filters(&q).0, None, "{hostile}");
        }
        let q = HashMap::from([("issue".to_string(), "no_description".to_string())]);
        assert_eq!(filters(&q).0, Some("no_description"));

        // And so do the other two.
        let q = HashMap::from([
            ("type".to_string(), "../x".to_string()),
            ("state".to_string(), "deleted".to_string()),
        ]);
        assert_eq!(filters(&q), (None, "all", "all"));
        let q = HashMap::from([
            ("type".to_string(), "page".to_string()),
            ("state".to_string(), "published".to_string()),
        ]);
        assert_eq!(filters(&q), (None, "page", "published"));
    }

    #[test]
    fn a_site_with_nothing_filled_in_is_told_what_to_fill_in() {
        let checks = site_checks(&SiteSnapshot { site_url: "http://localhost:1971".into(), ..Default::default() });
        let failing: Vec<&str> = checks.iter().filter(|c| !c.ok).map(|c| c.title).collect();
        for expected in [
            "Site title",
            "Home page description",
            "Publication icon",
            "Default social image",
            "Publisher logo",
            "Social accounts",
            "Structured data",
            "Canonical URLs are HTTPS",
            "IndexNow",
        ] {
            assert!(failing.contains(&expected), "{expected} should be reported: {failing:?}");
        }
        // The two that are always true now that they are served at all.
        assert!(checks.iter().any(|c| c.title == "robots.txt" && c.ok));
        assert!(checks.iter().any(|c| c.title == "Sitemaps" && c.ok));
    }

    #[test]
    fn a_private_site_is_told_plainly_that_nothing_is_indexed() {
        let checks = site_checks(&SiteSnapshot { is_private: true, ..Default::default() });
        assert_eq!(checks[0].title, "This site is private", "it has to be the first line");
        assert!(!checks[0].ok);
        // and it is not reported on a site that is not private
        let public = site_checks(&SiteSnapshot::default());
        assert!(!public.iter().any(|c| c.title == "This site is private"));
    }

    #[test]
    fn a_complete_site_has_nothing_to_report() {
        let checks = site_checks(&SiteSnapshot {
            title: "My Site".into(),
            description: "Words".into(),
            icon: "/content/images/icon.png".into(),
            logo: "/content/images/logo.png".into(),
            social_image: "/content/images/cover.jpg".into(),
            socials: 2,
            site_url: "https://example.com".into(),
            structured_data: true,
            llms: true,
            indexnow: true,
            ..Default::default()
        });
        let failing: Vec<&str> = checks.iter().filter(|c| !c.ok).map(|c| c.title).collect();
        assert!(failing.is_empty(), "{failing:?}");
    }

    #[test]
    fn hiding_an_archive_is_reported_as_a_choice_that_was_made() {
        // Not a failure — but a switch nobody remembers setting is exactly
        // what makes a site quietly disappear from a tag search.
        let checks = site_checks(&SiteSnapshot { noindex_archives: true, ..Default::default() });
        assert!(checks.iter().any(|c| c.title == "Some archives are hidden"));
    }

    #[test]
    fn the_score_bands_do_not_overlap_or_leave_a_gap() {
        assert_eq!(band(100), "green");
        assert_eq!(band(90), "green");
        assert_eq!(band(89), "amber");
        assert_eq!(band(60), "amber");
        assert_eq!(band(59), "red");
        assert_eq!(band(0), "red");
    }

    #[test]
    fn a_hostile_title_and_finding_come_back_escaped() {
        // The findings' messages carry entry text, and they are rendered into
        // a `title=` attribute, which is the easiest place to forget.
        let e = entry(r#"<img src=x onerror="alert(1)">"#, None);
        let findings = rules::audit(&e);
        let html = format!(
            r#"<span class="chip" title="{}">{}</span>"#,
            esc(&findings[0].message),
            esc(&e.title)
        );
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }

    #[test]
    fn a_pasted_robots_file_is_rebuilt_rather_than_stored_as_sent() {
        assert_eq!(clean_robots("User-agent: *\r\nDisallow: /x  \r\n"), "User-agent: *\nDisallow: /x");
        // Control characters cannot survive into a directive.
        assert_eq!(clean_robots("User-agent: *\u{0}\u{1}"), "User-agent: *");
        // Bounded, so the setting cannot be used as storage.
        let huge = "Disallow: /a\n".repeat(2000);
        assert_eq!(clean_robots(&huge).chars().count(), rules::ROBOTS_MAX);
        // An empty file stays empty, which is what selects the default.
        assert_eq!(clean_robots("   \n  \n"), "");
    }

    #[test]
    fn an_entry_id_that_is_not_an_object_id_is_never_linked() {
        // The id goes straight into an href. `is_objectid` is the only reason
        // that is safe.
        assert!(ghost_core::ids::is_objectid("6553f7a1b2c3d4e5f6a7b8c9"));
        for bad in ["../../settings", "6553f7a1b2c3d4e5f6a7b8c9/x", "", "\" onclick=\"x"] {
            assert!(!ghost_core::ids::is_objectid(bad), "{bad}");
        }
    }

    #[test]
    fn the_worst_entries_are_listed_first() {
        // The screen's whole purpose is "what next", so the order is the
        // score and nothing else.
        let mut entries = vec![
            entry("A complete post with a good long title", Some(&"d".repeat(80))),
            entry("Bad", None),
            entry("A middling post with a reasonable title", Some("short")),
        ];
        let findings: Vec<Vec<Finding>> =
            rules::audit_all(&mut entries).into_iter().map(|(_, f)| f).collect();
        let mut rows: Vec<(usize, u8)> =
            findings.iter().enumerate().map(|(i, f)| (i, rules::score(f))).collect();
        rows.sort_by_key(|(i, score)| (*score, *i));
        assert_eq!(rows[0].0, 1, "the entry with no description comes first");
        assert_eq!(rows[2].0, 0, "the complete one comes last");
    }
}
