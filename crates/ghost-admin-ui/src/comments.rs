//! Settings → Comments: the moderation screen.
//!
//! Ghost's equivalent is a React route at `/ghost/#/comments`. This is the
//! same job in the port's own idiom: a flat list of every comment on the
//! site, newest first, filterable by what a moderator actually goes looking
//! for — reported comments, hidden ones, or everything by one member.
//!
//! The two actions are hide and restore, and neither destroys anything.
//! Editing a comment's text is not offered, here or in Ghost: a comment
//! carries its author's name, and rewriting it would make that attribution a
//! lie. The reversible, visible action is hiding it.
//!
//! Reader-facing deletion (a member retracting their own comment) is a
//! separate status that only the member can set, through the widget — so a
//! `deleted` comment shows here as removed-by-its-author and cannot be
//! brought back by staff.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use ghost_api::AppState;
use ghost_db::repos::comments as repo;
use std::collections::HashMap;

use crate::{esc, flash_banner, page_for_role, require_role, svg, Role, SELECT_CARET};

/// What the filter dropdown offers, as `(query value, label, NQL)`.
///
/// `reported` is first because it is the only one that implies something
/// needs doing.
const FILTERS: &[(&str, &str, &str)] = &[
    ("reported", "Reported", "count.reports:>0"),
    ("all", "All comments", ""),
    ("published", "Visible", "status:published"),
    ("hidden", "Hidden", "status:hidden"),
    ("deleted", "Deleted by author", "status:deleted"),
];

const PER_PAGE: i64 = 50;

pub async fn comments_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };

    // Default to "all" rather than "reported": an empty reported list on a
    // site with comments reads as though the screen is broken.
    let selected = q.get("filter").map(String::as_str).unwrap_or("all");
    let selected = FILTERS.iter().find(|(v, _, _)| *v == selected).unwrap_or(&FILTERS[1]);
    let page = q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).max(1);

    // `?id=` is how the report notification email deep-links straight to the
    // comment that was reported.
    let focus = q.get("id").map(String::as_str).filter(|s| !s.is_empty());
    let member_filter = q.get("member").map(String::as_str).filter(|s| !s.is_empty());

    let mut clauses: Vec<String> = Vec::new();
    if !selected.2.is_empty() {
        clauses.push(selected.2.to_string());
    }
    if let Some(id) = focus {
        clauses.push(format!("id:'{}'", id.replace('\'', "")));
    }
    if let Some(member) = member_filter {
        clauses.push(format!("member_id:'{}'", member.replace('\'', "")));
    }
    // A filter that will not parse is dropped rather than refused: it is
    // built from this screen's own dropdown and query string, so a failure
    // means a bug here, and showing every comment beats showing an error.
    let filter = match clauses.join("+") {
        f if f.is_empty() => None,
        f => ghost_nql::parse(&f).ok(),
    };

    let browse = repo::BrowseComments {
        filter,
        order: Some("created_at desc".into()),
        limit: PER_PAGE,
        page,
        // No parent constraint: a reply needs moderating whether or not its
        // parent does, so it appears as its own row.
        parent_id: None,
        is_admin: true,
        browse_all: true,
        member_id: None,
        with_replies: false,
        with_post: true,
    };
    let result = match repo::browse(&state.db, &browse).await {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.message()).into_response();
        }
    };
    let (visible, hidden, reported) = repo::moderation_totals(&state.db).await.unwrap_or((0, 0, 0));

    let rows: String = result.comments.iter().map(|c| comment_row(c, focus)).collect();
    let list = if result.comments.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3;text-align:center"><p class="muted" style="margin:8px 0">{}</p></div>"#,
            match selected.0 {
                "reported" => "No comments have been reported.",
                "hidden" => "Nothing is hidden.",
                "deleted" => "No comments have been deleted by their authors.",
                _ => "No comments yet. They appear here as soon as readers start posting.",
            }
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:3">{rows}</div>"#)
    };

    let enabled = state.settings.get_str("comments_enabled").unwrap_or_else(|| "off".into());
    let off_notice = if enabled == "off" {
        format!(
            r#"<div class="banner error rise" style="--i:1">{flag}Commenting is switched off, so no new comments can be posted. Existing ones are still shown here and on the site. <a href="/ghost/settings/membership/">Change this in Membership</a>.</div>"#,
            flag = svg("flag"),
        )
    } else {
        String::new()
    };

    let focus_notice = focus
        .map(|id| {
            format!(
                r#"<div class="banner ok rise" style="--i:1">{check}Showing one comment. <a href="/ghost/comments/">Show all comments</a> <span class="muted">({id})</span></div>"#,
                check = svg("check"),
                id = esc(id),
            )
        })
        .unwrap_or_default();

    let member_notice = member_filter
        .map(|id| {
            format!(
                r#"<div class="banner ok rise" style="--i:1">{check}Showing one member's comments. <a href="/ghost/comments/">Show all comments</a> · <a href="/ghost/members/{id}/">Open their profile</a></div>"#,
                check = svg("check"),
                id = esc(id),
            )
        })
        .unwrap_or_default();

    page_for_role(
        actor_role,
        "Comments",
        "comments",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Comments</h1><p class="sub">{visible} visible · {hidden} hidden · {reported} reported</p></div></header>
{banner}{off_notice}{focus_notice}{member_notice}
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-purple">{comment_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:260px">
<select onchange="location.href='/ghost/comments/?filter='+this.value">{filter_opts}</select><span>Show</span>{car}</label></div></div>
</div>
{list}
{pager}"#,
            visible = visible,
            hidden = hidden,
            reported = reported,
            banner = flash_banner(&q),
            off_notice = off_notice,
            focus_notice = focus_notice,
            member_notice = member_notice,
            comment_ic = svg("comment"),
            car = SELECT_CARET,
            filter_opts = crate::select_options(
                &FILTERS.iter().map(|(v, l, _)| (*v, *l)).collect::<Vec<_>>(),
                selected.0,
            ),
            list = list,
            pager = pager(selected.0, page, result.total),
        ),
        true,
    )
    .into_response()
}

/// One comment as a row: who said it, on what, and what can be done about it.
fn comment_row(c: &repo::CommentWithRelations, focus: Option<&str>) -> String {
    let member_name = c
        .member
        .as_ref()
        .and_then(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| c.member.as_ref().map(|m| m.email.clone()))
        // A comment whose member was deleted keeps its text; the FK is
        // `ON DELETE SET NULL` precisely so the thread survives.
        .unwrap_or_else(|| "Deleted member".into());
    let initial = member_name.chars().next().unwrap_or('?').to_uppercase().to_string();

    // Deleted comments have no body to show at all — not even to staff, since
    // the author retracted it.
    let body = match c.comment.status.as_str() {
        "deleted" => r#"<em class="muted">Deleted by its author</em>"#.to_string(),
        _ => ghost_core::comments::snippet(c.comment.html.as_deref().unwrap_or_default()),
    };
    let body = if body.chars().count() > 280 {
        format!("{}…", body.chars().take(280).collect::<String>())
    } else {
        body
    };

    let (chip_class, chip) = match c.comment.status.as_str() {
        "published" => ("published", "visible"),
        "hidden" => ("draft", "hidden"),
        _ => ("draft", "deleted"),
    };

    // The action offered is whichever one this comment is not already in.
    // A deleted comment gets neither: staff did not remove it.
    let action = match c.comment.status.as_str() {
        "published" => format!(
            r#"<form method="post" action="/ghost/comments/{id}/hide/">
<button class="btn tonal small" type="submit" title="Hide from readers">{ic} Hide</button></form>"#,
            id = esc(&c.comment.id),
            ic = svg("hide"),
        ),
        "hidden" => format!(
            r#"<form method="post" action="/ghost/comments/{id}/show/">
<button class="btn tonal small" type="submit" title="Show to readers">{ic} Restore</button></form>"#,
            id = esc(&c.comment.id),
            ic = svg("check"),
        ),
        _ => String::new(),
    };

    let reports = if c.counts.reports > 0 {
        format!(
            r#"<span class="chip draft" title="Reported by {n} {word}">{flag} {n}</span>"#,
            n = c.counts.reports,
            word = if c.counts.reports == 1 { "reader" } else { "readers" },
            flag = svg("flag"),
        )
    } else {
        String::new()
    };

    let post_link = match &c.post {
        Some(p) => format!(
            r#"<a href="/{slug}/#ghost-comments-{id}" target="_blank" rel="noopener">{title}</a>"#,
            slug = esc(p.slug.as_deref().unwrap_or_default()),
            id = esc(&c.comment.id),
            title = esc(p.title.as_deref().unwrap_or("(untitled)")),
        ),
        None => "(post deleted)".to_string(),
    };

    // A reply says so, and says what it is answering — a reported reply read
    // without its parent is usually impossible to judge.
    let context = match (&c.comment.parent_id, &c.in_reply_to, &c.parent) {
        (Some(_), Some(target), _) | (Some(_), None, Some(target)) => format!(
            r#"<p class="muted" style="margin:6px 0 0;font-size:13px">↪ reply to “{}”</p>"#,
            esc(&truncate(&ghost_core::comments::snippet(target.html.as_deref().unwrap_or_default()), 90)),
        ),
        (Some(_), _, _) => r#"<p class="muted" style="margin:6px 0 0;font-size:13px">↪ reply</p>"#.to_string(),
        _ => String::new(),
    };

    let member_link = c
        .member
        .as_ref()
        .map(|m| {
            format!(
                r#" · <a href="/ghost/comments/?member={id}">their comments</a>"#,
                id = esc(&m.id)
            )
        })
        .unwrap_or_default();

    let suspended = c
        .member
        .as_ref()
        .map(|m| ghost_api::comments::Commenting::parse(m.commenting.as_deref()))
        .filter(|c| !c.can_comment())
        .map(|_| r#"<span class="chip draft" title="This member's commenting is suspended">suspended</span>"#)
        .unwrap_or_default();

    format!(
        r#"<div class="row" style="align-items:flex-start"{highlight}>
<span class="avatar">{initial}</span>
<div class="body" style="width:100%">
<span class="title">{member_name}</span>
<span class="detail">{date} · on {post_link}{member_link}</span>
<p style="margin:8px 0 0">{body}</p>
{context}
</div>
<div class="trailing" style="align-items:flex-start">{suspended}{reports}<span class="chip {chip_class}">{chip}</span>{action}</div>
</div>"#,
        // the deep-linked comment is marked so it is findable in a long list
        highlight = if focus == Some(c.comment.id.as_str()) {
            r#" style="align-items:flex-start;background:var(--secondary-container)""#
        } else {
            ""
        },
        initial = esc(&initial),
        member_name = esc(&member_name),
        date = esc(&c.comment.created_at),
        post_link = post_link,
        member_link = member_link,
        body = esc(&body),
        context = context,
        suspended = suspended,
        reports = reports,
        chip_class = chip_class,
        chip = chip,
        action = action,
    )
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    format!("{}…", s.chars().take(n).collect::<String>())
}

fn pager(filter: &str, page: i64, total: i64) -> String {
    let pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    if pages <= 1 {
        return String::new();
    }
    let link = |p: i64, label: &str| {
        format!(r#"<a class="btn tonal small" href="/ghost/comments/?filter={filter}&page={p}">{label}</a>"#)
    };
    format!(
        r#"<div style="display:flex;gap:10px;align-items:center;padding:14px 4px">
{prev}<span class="muted">Page {page} of {pages}</span>{next}</div>"#,
        prev = if page > 1 { link(page - 1, "← Newer") } else { String::new() },
        next = if page < pages { link(page + 1, "Older →") } else { String::new() },
        page = page,
        pages = pages,
    )
}

// ---------------------------------------------------------------------------
// actions
// ---------------------------------------------------------------------------

pub async fn comment_hide(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    set_status(state, headers, &id, "hidden", "Comment hidden from readers.").await
}

pub async fn comment_show(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    set_status(state, headers, &id, "published", "Comment is visible again.").await
}

async fn set_status(
    state: AppState,
    headers: HeaderMap,
    id: &str,
    status: &str,
    message: &str,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // A comment its author deleted is not staff's to restore.
    match repo::find_raw(&state.db, id).await {
        Ok(Some(c)) if c.status == "deleted" => {
            return flash("error", "That comment was deleted by its author.")
        }
        Ok(None) => return flash("error", "That comment no longer exists."),
        Err(e) => return flash("error", &e.message()),
        Ok(Some(_)) => {}
    }
    match repo::set_status(&state.db, id, status).await {
        Ok(_) => flash("ok", message),
        Err(e) => flash("error", &e.message()),
    }
}

fn flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/comments/?{encoded}")).into_response()
}

// ---------------------------------------------------------------------------
// per-member commenting suspension, from the member's own page
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct SuspendForm {
    reason: Option<String>,
    hide_existing: Option<String>,
}

/// Suspend a member's commenting.
///
/// The reason is required because the member is shown it in place of the
/// comment form — a suspension they cannot see the cause of just looks like
/// the site is broken.
pub async fn member_commenting_disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<SuspendForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let reason = form.reason.unwrap_or_default().trim().to_string();
    if reason.is_empty() {
        return member_flash(&id, "error", "A reason is required when suspending commenting.");
    }
    if reason.chars().count() > 2000 {
        return member_flash(&id, "error", "That reason is longer than 2000 characters.");
    }

    let commenting = ghost_api::comments::Commenting::disabled(&reason, None);
    let res = sqlx::query("UPDATE members SET commenting = ?, updated_at = ? WHERE id = ?")
        .bind(commenting.to_storage())
        .bind(ghost_core::dates::now_db())
        .bind(&id)
        .execute(&state.db)
        .await;
    if res.map(|r| r.rows_affected() == 0).unwrap_or(true) {
        return member_flash(&id, "error", "Saving failed — check the logs.");
    }

    let mut message = "Commenting suspended for this member.".to_string();
    if form.hide_existing.as_deref() == Some("1") {
        match repo::hide_all_by_member(&state.db, &id).await {
            Ok(n) => {
                message = format!(
                    "Commenting suspended, and {n} existing {} hidden.",
                    if n == 1 { "comment" } else { "comments" }
                )
            }
            Err(e) => return member_flash(&id, "error", &e.message()),
        }
    }
    member_flash(&id, "ok", &message)
}

pub async fn member_commenting_enable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let res = sqlx::query("UPDATE members SET commenting = ?, updated_at = ? WHERE id = ?")
        .bind(ghost_api::comments::Commenting::default().to_storage())
        .bind(ghost_core::dates::now_db())
        .bind(&id)
        .execute(&state.db)
        .await;
    if res.map(|r| r.rows_affected() == 0).unwrap_or(true) {
        return member_flash(&id, "error", "Saving failed — check the logs.");
    }
    // Comments hidden while they were suspended stay hidden: restoring them
    // wholesale would also un-hide anything hidden on its own merits.
    member_flash(&id, "ok", "Commenting restored for this member.")
}

fn member_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/members/{id}/?{encoded}")).into_response()
}

/// The suspension controls for one member, rendered into their profile page.
pub fn member_commenting_card(
    member_id: &str,
    commenting: &ghost_api::comments::Commenting,
    comment_count: i64,
) -> String {
    let counted = format!(
        r#"<a href="/ghost/comments/?member={id}">{n} {word}</a>"#,
        id = esc(member_id),
        n = comment_count,
        word = if comment_count == 1 { "comment" } else { "comments" },
    );
    if commenting.can_comment() {
        return format!(
            r#"<div class="section">Commenting</div>
<div class="card padded rise" style="--i:7">
<p class="muted" style="margin:0 0 12px">This member can comment, and has posted {counted}.</p>
<form method="post" action="/ghost/members/{id}/commenting/disable/">
<label class="field"><input name="reason" placeholder=" " required maxlength="2000"><span>Reason · shown to the member</span></label>
<label style="display:flex;gap:8px;align-items:center;margin:0 0 12px">
<input type="checkbox" name="hide_existing" value="1"><span class="muted">Also hide their existing comments</span></label>
<button class="btn tonal small danger" type="submit">{hide} Suspend commenting</button>
</form></div>"#,
            counted = counted,
            id = esc(member_id),
            hide = svg("hide"),
        );
    }
    format!(
        r#"<div class="section">Commenting</div>
<div class="card padded rise" style="--i:7">
<p class="muted" style="margin:0 0 6px"><b>Commenting is suspended.</b> They have posted {counted}.</p>
<p class="muted" style="margin:0 0 12px">Reason shown to them: “{reason}”</p>
<form method="post" action="/ghost/members/{id}/commenting/enable/">
<button class="btn tonal small" type="submit">{check} Restore commenting</button>
</form></div>"#,
        counted = counted,
        reason = esc(commenting.disabled_reason.as_deref().unwrap_or("")),
        id = esc(member_id),
        check = svg("check"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filter_compiles_against_the_comments_resource() {
        for (value, label, nql) in FILTERS {
            assert!(!label.is_empty(), "{value} has no label");
            if nql.is_empty() {
                continue;
            }
            let expr = ghost_nql::parse(nql).unwrap_or_else(|e| panic!("{value}: {}", e.message()));
            ghost_nql::compile(&expr, "comments")
                .unwrap_or_else(|e| panic!("{value}: {}", e.message()));
        }
    }

    #[test]
    fn the_pager_only_appears_when_there_is_more_than_one_page() {
        assert_eq!(pager("all", 1, 10), "");
        assert_eq!(pager("all", 1, PER_PAGE), "");
        let two = pager("all", 1, PER_PAGE + 1);
        assert!(two.contains("Page 1 of 2"), "{two}");
        assert!(two.contains("Older"), "{two}");
        assert!(!two.contains("Newer"), "{two}");
        let last = pager("all", 2, PER_PAGE + 1);
        assert!(last.contains("Newer"), "{last}");
        assert!(!last.contains("Older"), "{last}");
    }

    #[test]
    fn truncation_is_on_characters_not_bytes() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
        // a multi-byte string must not be cut mid-character
        assert_eq!(truncate("ééééé", 2), "éé…");
    }
}
