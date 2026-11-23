//! Post history and Page history — what this entry used to say, and putting
//! it back.
//!
//! Ghost calls these "Post history" and "Page history" and reaches them from
//! the editor's settings menu. This port reaches them from a panel in the
//! editor's settings column, which is where everything else about an entry is
//! set, and the panel links through to the screen below.
//!
//! ## There was nothing to build on the server
//!
//! A full snapshot of an entry — body, excerpt, feature image, meta fields,
//! code injection, tags, authors and tiers — has been written to
//! `post_revisions` on **every save** since the table existed
//! ([`vuuut_db::repos::posts::update`]), and
//! [`vuuut_db::repos::publishing::restore`] has been able to put one back for
//! just as long. Both were unreachable: no screen in the admin called either.
//! So this module is almost entirely presentation, and the one thing it adds
//! to the data is `post_revisions.author_id`, which the insert was not
//! binding — a history that cannot say who changed something is half a
//! history.
//!
//! ## What restoring does and does not touch
//!
//! Content, metadata, tags, authors and tiers come back. **Status, publish
//! date and slug do not** — that is [`restore`]'s own rule in the repo, and it
//! is the right one: restoring the text of a published post must not silently
//! unpublish it or move its URL out from under the links pointing at it.
//!
//! ## Who may use it
//!
//! Whoever may edit the entry, decided by the same [`crate::may_edit_post`]
//! check the editor itself uses — so a Contributor sees the history of their
//! own drafts and nothing else. Restoring is additionally refused to anyone
//! below Editor when the snapshot carries a different author list or code
//! injection, mirroring what the Admin API's own restore endpoint does: those
//! two fields are a privilege boundary, and a snapshot is a way to write them
//! without ever having been allowed to type them.
//!
//! Restoring is a POST, origin-checked, and redirects back to the editor.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use vuuut_api::AppState;
use vuuut_db::repos;
use vuuut_db::repos::publishing::Revision;
use std::collections::HashMap;

use crate::{esc, flash_banner, page_for, require_actor, svg, Role};

/// The most revisions one screen lists. Matches the Admin API's own ceiling
/// for the same endpoint, so the two cannot disagree about what exists.
pub(crate) const LIMIT: i64 = 100;

/// How many the editor's own panel shows before "See all".
pub(crate) const PANEL_LIMIT: usize = 3;

/// `/ghost/posts/` or `/ghost/pages/`, from the post's own type — never from
/// the URL that asked, so a page's history cannot be reached at a post's
/// address and come back linking to the wrong editor.
fn base_for(post_type: &str) -> &'static str {
    if post_type == "page" {
        "/ghost/pages/"
    } else {
        "/ghost/posts/"
    }
}

fn title_for(post_type: &str) -> &'static str {
    if post_type == "page" {
        "Page history"
    } else {
        "Post history"
    }
}

fn flash_to_editor(base: &str, id: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("{base}{id}/?{encoded}")).into_response()
}

/// The date a revision carries, in the site's timezone.
///
/// Stored UTC and shown local, like every other date in the admin: a history
/// is read against "when was I working on this", and that is a wall clock.
fn when(stored: &str, timezone: &str) -> String {
    let date = vuuut_core::dates::short_date_in_zone(stored, timezone);
    let time = vuuut_core::dates::clock_in_zone(stored, timezone);
    if date.is_empty() {
        return stored.to_string();
    }
    format!("{date} at {time}")
}

/// One row of the revision list, on the screen or in the editor's panel.
fn row(base: &str, post_id: &str, r: &Revision, timezone: &str, selected: bool, compact: bool) -> String {
    let title = r.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("(untitled)");
    let by = match r.author_name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!(" · {}", esc(name)),
        // The scheduler, or a revision from before the author was recorded.
        None => String::new(),
    };
    format!(
        r#"<a class="ph-rev{on}{compact}" href="{base}{post_id}/history/?revision={rev}">
<span class="ph-when">{when}</span>
<span class="ph-title">{title}</span>
<span class="ph-by">{by}</span></a>"#,
        on = if selected { " on" } else { "" },
        compact = if compact { " compact" } else { "" },
        base = base,
        post_id = esc(post_id),
        rev = esc(&r.id),
        when = esc(&when(&r.created_at, timezone)),
        title = esc(title),
        by = by,
    )
}

/// The panel the editor's settings column shows, and the only entry point to
/// this screen.
///
/// Rendered here rather than in `editor.rs` so the module owns its own
/// markup: `editor.rs` gets one call, the way the rest of the module pattern
/// has it.
pub(crate) fn editor_panel(
    post_type: &str,
    post_id: &str,
    revisions: &[Revision],
    total: i64,
    timezone: &str,
) -> String {
    let base = base_for(post_type);
    let body = if revisions.is_empty() {
        format!(
            r#"<p class="hint">No earlier versions yet. One is kept each time this {singular} is saved.</p>"#,
            singular = esc(if post_type == "page" { "page" } else { "post" }),
        )
    } else {
        let rows: String = revisions
            .iter()
            .take(PANEL_LIMIT)
            .map(|r| row(base, post_id, r, timezone, false, true))
            .collect();
        format!(
            r#"<div class="ph-list compact">{rows}</div>
<a class="btn tonal small" href="{base}{id}/history/">{open} {all}</a>"#,
            base = base,
            id = esc(post_id),
            open = svg("history"),
            all = esc(&match total {
                n if n > PANEL_LIMIT as i64 => format!("See all {n} versions"),
                1 => "See the earlier version".to_string(),
                n => format!("See all {n} versions"),
            }),
        )
    };
    crate::editor::panel("history", "slate", title_for(post_type), &body, false)
}

// ---------------------------------------------------------------------------
// the screen
// ---------------------------------------------------------------------------

pub async fn history_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    // An id straight from the URL, reaching a query and an href.
    if !vuuut_core::ids::is_objectid(&id) || !crate::may_edit_post(&state, &actor, &id).await {
        return crate::forbidden_page(&actor, Role::Editor);
    }
    let Some(post) = repos::posts::find_by_id(&state.db, &id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let base = base_for(&post.post_type);
    let nav = if post.post_type == "page" { "pages" } else { "posts" };
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    let revisions = repos::publishing::revisions(&state.db, &id, LIMIT).await.unwrap_or_default();
    // `?revision=` comes from a link on this page, but it is still a query
    // parameter: it is only used once it matches a revision this post really
    // has, so a forged one shows the newest instead of reaching the query.
    let selected = q
        .get("revision")
        .and_then(|want| revisions.iter().find(|r| &r.id == want))
        .or_else(|| revisions.first());

    let list: String = revisions
        .iter()
        .map(|r| row(base, &id, r, &timezone, selected.is_some_and(|s| s.id == r.id), false))
        .collect();

    let post_title = post.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("(untitled)");

    let detail = match selected {
        None => format!(
            r#"<div class="card padded" style="text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">No earlier versions</p>
<p class="muted" style="margin:0">A version is kept each time this {singular} is saved, so the first one appears after the next save.</p></div>"#,
            singular = esc(if post.post_type == "page" { "page" } else { "post" }),
        ),
        Some(r) => {
            let snapshot = repos::publishing::revision_snapshot(&state.db, &id, &r.id).await.unwrap_or(None);
            // The stored body carries no base path, and this is a browser
            // output boundary, so images in it resolve only once it is added.
            let body = snapshot
                .as_ref()
                .and_then(|s| s.post.html.as_deref())
                .map(vuuut_core::url::rewrite_html)
                .filter(|html| !html.trim().is_empty())
                .unwrap_or_else(|| {
                    r#"<p class="muted">This version has no rendered body stored. Restoring it still returns its title, metadata, tags and authors.</p>"#.to_string()
                });
            let image = r
                .feature_image
                .as_deref()
                .filter(|i| !i.is_empty())
                .map(|i| {
                    format!(
                        r#"<img class="ph-image" src="{}" alt="">"#,
                        esc(&vuuut_core::url::base_asset(i))
                    )
                })
                .unwrap_or_default();
            let revision_title =
                r.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("(untitled)");
            format!(
                r#"<div class="card padded ph-detail">
<div class="ph-detail-head">
<div><p class="ph-when">{when}{by}</p><h2>{title}</h2></div>
<form method="post" action="{base}{id}/history/{rev}/restore/" class="ph-restore"
      data-confirm="Restore this version|The current text of &#8220;{current}&#8221; will be replaced. Its status, publish date and URL stay as they are — and the current version is kept as a version of its own, so this can be undone.">
<button class="btn" type="submit">{repeat} Restore this version</button></form>
</div>
{image}
<div class="ph-body">{body}</div>
</div>"#,
                when = esc(&when(&r.created_at, &timezone)),
                by = r
                    .author_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(|n| format!(" · saved by {}", esc(n)))
                    .unwrap_or_default(),
                title = esc(revision_title),
                base = base,
                id = esc(&id),
                rev = esc(&r.id),
                current = esc(post_title),
                repeat = svg("repeat"),
                image = image,
                body = body,
            )
        }
    };

    page_for(
        title_for(&post.post_type),
        nav,
        Some(&actor.user),
        actor.role,
        &format!(
            r#"{banner}<header class="page-head rise">
<a class="icon-btn" style="transform:rotate(180deg)" href="{base}{id}/" title="Back to the editor">{back}</a>
<div><h1>{heading}</h1><p class="sub">{title}</p></div>
<div class="actions"><a class="btn tonal" href="{base}{id}/">{edit} Back to the editor</a></div></header>
<div class="ph-split rise" style="--i:1">
<aside class="card ph-list">{list}</aside>
<div class="ph-pane">{detail}</div>
</div>
<p class="muted rise" style="--i:2;margin:16px 0 0">Restoring returns the text, excerpt, feature image, metadata, tags and authors of a version. It leaves the status, the publish date and the URL exactly as they are.</p>"#,
            banner = flash_banner(&q),
            base = base,
            id = esc(&id),
            back = svg("chevron"),
            edit = svg("edit"),
            heading = esc(title_for(&post.post_type)),
            title = esc(post_title),
            list = if list.is_empty() {
                r#"<p class="hint" style="padding:16px">Nothing yet.</p>"#.to_string()
            } else {
                list
            },
            detail = detail,
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// restoring
// ---------------------------------------------------------------------------

pub async fn restore(
    State(state): State<AppState>,
    Path((id, revision)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    // A POST that rewrites a post's content: the session cookie is
    // `SameSite=Lax` already, and this is the explicit check so that does not
    // rest on one browser behaviour.
    if !vuuut_api::forms::same_origin(&headers, &state.site_url()) {
        return (StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response();
    }
    if !vuuut_core::ids::is_objectid(&id)
        || !vuuut_core::ids::is_objectid(&revision)
        || !crate::may_edit_post(&state, &actor, &id).await
    {
        return crate::forbidden_page(&actor, Role::Editor);
    }
    let Some(post) = repos::posts::find_by_id(&state.db, &id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let base = base_for(&post.post_type);

    // The privilege boundary. A snapshot carries an author list and code
    // injection, and restoring is a way to write both without ever having
    // been able to type them — so below Editor, a snapshot that would change
    // either is refused rather than quietly applied. Mirrors the Admin API's
    // own restore endpoint.
    if !actor.role.can_edit_any_post() {
        let Some(snapshot) = repos::publishing::revision_snapshot(&state.db, &id, &revision).await.unwrap_or(None)
        else {
            return flash_to_editor(base, &id, "error", "That version no longer exists.");
        };
        let current = repos::posts::load_relations(&state.db, std::slice::from_ref(&post)).await.unwrap_or_default();
        let mut current_authors: Vec<String> =
            current.authors_for(&post.id).into_iter().map(|a| a.id).collect();
        let mut snapshot_authors = snapshot.author_ids.clone();
        current_authors.sort();
        snapshot_authors.sort();
        let injection_differs = snapshot.post.codeinjection_head != post.codeinjection_head
            || snapshot.post.codeinjection_foot != post.codeinjection_foot;
        if current_authors != snapshot_authors || injection_differs {
            return flash_to_editor(
                base,
                &id,
                "error",
                "That version changes the authors or the code injection, which needs an Editor.",
            );
        }
    }

    match repos::publishing::restore(&state.db, &id, &revision).await {
        Ok(_) => {
            // Recorded the way the Admin API records it, so Settings →
            // History shows a restore beside the ordinary edits.
            let _ = repos::actions::record(&state.db, &post.post_type, &id, &actor.user.id, "restored").await;
            flash_to_editor(
                base,
                &id,
                "ok",
                "That version has been restored. The version you replaced was kept, so this can be undone.",
            )
        }
        Err(e) => {
            tracing::error!("history restore: {}", e.message());
            flash_to_editor(base, &id, "error", "That version could not be restored — check the logs.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(id: &str, title: Option<&str>, author: Option<&str>) -> Revision {
        Revision {
            id: id.into(),
            created_at: "2026-11-14 09:30:00".into(),
            title: title.map(str::to_string),
            author_name: author.map(str::to_string),
            feature_image: None,
        }
    }

    #[test]
    fn the_editor_it_links_back_to_comes_from_the_post_not_the_url() {
        // A page's history reached at a post's address must not come back
        // linking into the posts editor, where the id does not resolve.
        assert_eq!(base_for("page"), "/ghost/pages/");
        assert_eq!(base_for("post"), "/ghost/posts/");
        // Anything unexpected is treated as a post, which is the type that
        // every id in this table has unless it says otherwise.
        assert_eq!(base_for(""), "/ghost/posts/");
        assert_eq!(title_for("page"), "Page history");
        assert_eq!(title_for("post"), "Post history");
    }

    #[test]
    fn a_hostile_title_or_author_name_is_escaped_in_the_list() {
        let r = revision(
            "6553f7a1b2c3d4e5f6a7b8c9",
            Some(r#"<img src=x onerror="alert(1)">"#),
            Some("<script>alert(2)</script>"),
        );
        let html = row("/ghost/posts/", "6553f7a1b2c3d4e5f6a7b8ca", &r, "Etc/UTC", false, false);
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("<script"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }

    #[test]
    fn a_version_with_no_title_is_still_listed() {
        // A draft saved before it was titled is exactly the version somebody
        // wants back, so it cannot be dropped for having no title.
        let html = row("/ghost/posts/", "6553f7a1b2c3d4e5f6a7b8c9", &revision("r1", None, None), "Etc/UTC", false, false);
        assert!(html.contains("(untitled)"), "{html}");
        // An empty-string title reads the same as none.
        let html = row("/ghost/posts/", "6553f7a1b2c3d4e5f6a7b8c9", &revision("r1", Some("  "), None), "Etc/UTC", false, false);
        assert!(html.contains("(untitled)"), "{html}");
    }

    #[test]
    fn a_revision_with_no_recorded_author_says_nothing_rather_than_unknown() {
        // The scheduler has no human behind it, and revisions written before
        // `author_id` was bound have none either. "Unknown" would read as a
        // fault; saying nothing reads as what it is.
        let html = row("/ghost/posts/", "6553f7a1b2c3d4e5f6a7b8c9", &revision("r1", Some("A post"), None), "Etc/UTC", false, false);
        assert!(!html.to_lowercase().contains("unknown"), "{html}");
        let with = row("/ghost/posts/", "6553f7a1b2c3d4e5f6a7b8c9", &revision("r1", Some("A post"), Some("Ada")), "Etc/UTC", false, false);
        assert!(with.contains("Ada"), "{with}");
    }

    #[test]
    fn the_panel_says_how_many_versions_there_are_in_words_that_agree_with_the_count() {
        let id = "6553f7a1b2c3d4e5f6a7b8c9";
        // Nothing yet: no link, because there is nothing to open.
        let empty = editor_panel("post", id, &[], 0, "Etc/UTC");
        assert!(empty.contains("No earlier versions yet"), "{empty}");
        assert!(!empty.contains("/history/"), "{empty}");

        let one = editor_panel("post", id, &[revision("r1", Some("A post"), None)], 1, "Etc/UTC");
        assert!(one.contains("See the earlier version"), "{one}");
        assert!(one.contains(&format!("/ghost/posts/{id}/history/")), "{one}");

        let many: Vec<Revision> = (0..8).map(|i| revision(&format!("r{i}"), Some("A post"), None)).collect();
        let panel = editor_panel("post", id, &many, 8, "Etc/UTC");
        assert!(panel.contains("See all 8 versions"), "{panel}");
        // Only the first few are listed inline; the rest are behind the link.
        assert_eq!(panel.matches("ph-rev").count(), PANEL_LIMIT, "{panel}");

        // And a page's panel is titled for a page.
        assert!(editor_panel("page", id, &[], 0, "Etc/UTC").contains("Page history"));
    }

    #[test]
    fn a_date_is_shown_in_the_sites_timezone() {
        // 23:30 UTC is already the next day in Dhaka, and a history read
        // against the wrong day is a history nobody can navigate.
        assert_eq!(when("2026-11-14 23:30:00", "Etc/UTC"), "14 Nov 2026 at 23:30");
        assert_eq!(when("2026-11-14 23:30:00", "Asia/Dhaka"), "15 Nov 2026 at 05:30");
        // An unparseable stamp is shown as stored rather than as an empty row.
        assert_eq!(when("not a date", "Etc/UTC"), "not a date");
    }
}
