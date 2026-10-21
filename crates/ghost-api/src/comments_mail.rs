//! Sending the comment notifications: who gets told, and what they are told.
//!
//! Ghost's `CommentsServiceEmails` decides the recipients; `ghost-mail`'s
//! `comments` module renders what they receive. This module is the join
//! between them — it resolves the post URL, the recipients and the dates out
//! of the database and settings, then hands a fully-populated
//! [`CommentEmail`] over to be rendered and sent.
//!
//! Every function here is infallible from the caller's point of view. A
//! comment is published whether or not the notification about it goes out:
//! the commenter cannot act on an SMTP failure, and refusing to accept a
//! comment because the owner's mail server is down would be the wrong
//! trade. Failures are logged instead.

use ghost_core::comments as rules;
use ghost_db::repos::comments as repo;
use ghost_mail::comments::{CommentEmail, Kind};

use crate::members::Member;
use crate::AppState;

/// Everything the three emails share, resolved once.
struct Context {
    email: CommentEmail,
}

/// Build the shared part of a notification about `comment`.
///
/// Returns `None` when the comment's post or author is missing, which can
/// only happen if one was deleted between the comment being saved and this
/// running — there is nothing to notify about in that case.
async fn context(state: &AppState, comment: &repo::Comment) -> Option<Context> {
    let site_url = state.site_url();
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    let post = post_summary(state, &comment.post_id).await?;
    // A comment whose member was deleted still has a post and a body; the
    // notification just has nobody to attribute it to.
    let member = match comment.member_id.as_deref() {
        Some(id) => member_summary(state, id).await,
        None => None,
    };

    // Ghost's `getPostUrl`: the post's own permalink plus the fragment that
    // makes the comments widget scroll to — and highlight — this comment.
    let post_url = format!("{}#ghost-comments-{}", post.url, comment.id);
    // A member who never set a name still has to be addressed as somebody.
    let member_name = member
        .as_ref()
        .and_then(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Anonymous".into());

    let html = comment.html.clone().unwrap_or_default();
    Some(Context {
        email: CommentEmail {
            site_title,
            site_url: site_url.clone(),
            site_domain: ghost_mail::site_domain(&site_url),
            post_title: post.title,
            post_url,
            comment_text: rules::to_plaintext(&html),
            comment_html: html,
            comment_date: ghost_core::dates::short_date_in_zone(&comment.created_at, &timezone),
            member_name: member_name.clone(),
            member_email: member.as_ref().map(|m| m.email.clone()).unwrap_or_default(),
            member_expertise: member.as_ref().and_then(|m| m.expertise.clone()),
            member_initials: rules::initials(&member_name),
            accent_color: state.settings.get_str("accent_color").unwrap_or_default(),
            moderation_enabled: true,
            moderation_url: format!("{site_url}/ghost/comments/?id={}", comment.id),
            ..Default::default()
        },
    })
}

struct PostSummary {
    title: String,
    url: String,
}

async fn post_summary(state: &AppState, post_id: &str) -> Option<PostSummary> {
    let post = ghost_db::repos::posts::find_by_id(&state.db, post_id).await.ok().flatten()?;
    let rel = ghost_db::repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
        .await
        .unwrap_or_default();
    // The permalink can depend on the post's primary tag or author
    // (`/:primary_tag/:slug/` routes), so the relations go in before the
    // resolver sees it.
    let json = ghost_core::models::post_to_json(
        &post,
        &rel.authors_for(&post.id),
        &rel.tags_for(&post.id),
        "",
    );
    let resolver = crate::content::resolve(state).await;
    let relative = resolver(&json);
    Some(PostSummary {
        title: post.title.unwrap_or_default(),
        url: ghost_core::url::absolute(&state.site_url(), &relative),
    })
}

async fn member_summary(state: &AppState, member_id: &str) -> Option<repo::CommentMember> {
    sqlx::query_as::<_, repo::CommentMember>(
        "SELECT id, uuid, name, email, expertise, status, commenting, enable_comment_notifications \
         FROM members WHERE id = ?",
    )
    .bind(member_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

async fn send(state: &AppState, email: &CommentEmail, kind: Kind) {
    if email.to_email.trim().is_empty() {
        return;
    }
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.get_str("title").unwrap_or_default(),
        &state.site_url(),
    );
    let msg = match email.message(kind) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("comments: rendering {:?} failed: {}", kind, e.message());
            return;
        }
    };
    if let Err(e) = ghost_mail::send(&cfg, &msg).await {
        tracing::warn!("comments: sending {:?} to {} failed: {}", kind, email.to_email, e.message());
    }
}

// ---------------------------------------------------------------------------
// new comment → the post's authors
// ---------------------------------------------------------------------------

/// Notify every author of the post who has staff comment notifications on.
///
/// `users.comment_notifications` is a per-author opt-out, so a post with
/// three authors can produce anywhere from zero to three emails.
pub async fn notify_post_authors(state: &AppState, comment: &repo::Comment) {
    let Some(ctx) = context(state, comment).await else {
        return;
    };
    let authors: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT u.email, u.slug, u.name FROM posts_authors pa JOIN users u ON u.id = pa.author_id \
         WHERE pa.post_id = ? AND u.comment_notifications = 1 AND u.status = 'active' \
         ORDER BY pa.sort_order",
    )
    .bind(&comment.post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    for (email, slug, _name) in authors {
        // An author does not want to be told about their own comment on
        // their own post — but Ghost does send it, because a staff user and
        // a member are separate identities that merely share an address.
        let e = CommentEmail {
            to_email: email,
            staff_url: staff_notifications_url(state, slug.as_deref()),
            ..ctx.email.clone()
        };
        send(state, &e, Kind::NewComment).await;
    }
}

// ---------------------------------------------------------------------------
// new reply → whoever is being replied to
// ---------------------------------------------------------------------------

/// Notify the author of the comment this one replies to.
///
/// Called twice for a reply that addresses a sibling: once for the thread
/// parent (`in_reply_to = false`) and once for the sibling itself, which is
/// how Ghost makes sure both the person who started the thread and the
/// person actually being answered hear about it.
pub async fn notify_reply_target(state: &AppState, reply: &repo::Comment, in_reply_to: bool) {
    let target_id = if in_reply_to { &reply.in_reply_to_id } else { &reply.parent_id };
    let Some(target_id) = target_id.as_deref() else {
        return;
    };
    let Ok(Some(target)) = repo::find_raw(&state.db, target_id).await else {
        return;
    };
    // A hidden or deleted comment's author is not notified: they are not
    // part of the conversation any more.
    if target.status != "published" {
        return;
    }
    let Some(target_member_id) = target.member_id.as_deref() else {
        return;
    };
    // Replying to yourself does not generate an email.
    if Some(target_member_id) == reply.member_id.as_deref() {
        return;
    }
    let Some(recipient) = member_summary(state, target_member_id).await else {
        return;
    };
    if !recipient.enable_comment_notifications {
        return;
    }
    let Some(ctx) = context(state, reply).await else {
        return;
    };

    let e = CommentEmail {
        to_email: recipient.email.clone(),
        profile_url: unsubscribe_url(state, recipient.uuid.as_deref().unwrap_or_default()),
        ..ctx.email
    };
    send(state, &e, Kind::NewCommentReply).await;
}

// ---------------------------------------------------------------------------
// report → the site owner
// ---------------------------------------------------------------------------

/// Tell the owner a comment has been reported.
///
/// Only the owner, as in Ghost: a report is a request for a decision, and
/// mailing every administrator would make it ambiguous who should act.
pub async fn notify_report(state: &AppState, comment: &repo::Comment, reporter: &Member) {
    let Some(ctx) = context(state, comment).await else {
        return;
    };
    let owner: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT u.email, u.slug FROM users u \
         JOIN roles_users ru ON ru.user_id = u.id JOIN roles r ON r.id = ru.role_id \
         WHERE r.name = 'Owner' LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((email, slug)) = owner else {
        tracing::warn!("comments: a comment was reported but the site has no owner to tell");
        return;
    };

    let e = CommentEmail {
        to_email: email,
        staff_url: staff_notifications_url(state, slug.as_deref()),
        reporter_name: reporter.name.clone(),
        reporter_email: reporter.email.clone(),
        ..ctx.email
    };
    send(state, &e, Kind::Report).await;
}

// ---------------------------------------------------------------------------
// the "turn this off" links
// ---------------------------------------------------------------------------

/// Where a staff user turns their own comment notifications off.
fn staff_notifications_url(state: &AppState, slug: Option<&str>) -> String {
    let base = state.admin_url();
    match slug {
        Some(slug) => format!("{base}/settings/staff/{slug}/"),
        None => format!("{base}/settings/staff/"),
    }
}

/// Where a member turns comment-reply notifications off. `comments=1` is
/// what makes the unsubscribe page land on the comments preference rather
/// than on newsletters.
fn unsubscribe_url(state: &AppState, uuid: &str) -> String {
    let key = newsletter_key(state, uuid);
    format!("{}/unsubscribe/?uuid={uuid}&key={key}&comments=1", state.site_url())
}

/// The same HMAC the unsubscribe links in newsletters carry, so one
/// implementation validates both.
fn newsletter_key(state: &AppState, uuid: &str) -> String {
    crate::members::newsletter_key(state, uuid)
}
