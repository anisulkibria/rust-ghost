//! The publisher for scheduled posts.
//!
//! A post with `status = 'scheduled'` is finished writing and waiting for its
//! date. Nothing in a request path can publish it — by definition nobody is
//! asking — so a task on a timer does the work the author would otherwise
//! have come back to do: flip the status, tell Slack, and send the newsletter
//! if that was the plan when it was scheduled.
//!
//! It lives in this crate because the publish path does: `editor_create_impl`
//! and this module do the same three things, and keeping them side by side is
//! what stops them drifting apart.
//!
//! One process, one task. A multi-process deployment would need a lock in the
//! table (Ghost uses a job row for this); the port serves a site from a single
//! binary, so the loop is the whole scheduler.

use ghost_api::AppState;
use ghost_core::dates;
use ghost_db::repos;
use std::time::Duration;

/// How often to look. A minute is the granularity the editor offers — the
/// date field has no seconds — so checking more often would only burn wakeups
/// for a post that is already late by less than it takes to read the title.
const TICK: Duration = Duration::from_secs(30);

/// Start the publisher. Returns immediately; the work happens on its own task
/// for the life of the process.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            publish_due(&state).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Publish everything whose time has come.
///
/// Errors are logged and the loop goes on: one post whose newsletter could
/// not go out must not keep the next one from being published, and the
/// scheduler has no user in front of it to tell.
pub async fn publish_due(state: &AppState) {
    let now = dates::now_db();
    let due = match repos::posts::due_scheduled(&state.db, &now).await {
        Ok(due) => due,
        Err(e) => {
            tracing::warn!("scheduler: could not read scheduled posts: {e}");
            return;
        }
    };

    for post in due {
        publish_one(state, &post).await;
    }
}

async fn publish_one(state: &AppState, due: &repos::posts::DuePost) {
    let status = due.delivery.status();
    let changes = repos::posts::PostChanges {
        status: Some(status.to_string()),
        // Everything else stays as written, and `published_at` in particular:
        // the scheduled moment is the post's publication date, not the moment
        // this loop happened to notice it.
        ..Default::default()
    };
    let post = match repos::posts::update(&state.db, &due.id, &changes).await {
        Ok(post) => post,
        Err(e) => {
            tracing::warn!("scheduler: could not publish {}: {e}", due.id);
            return;
        }
    };
    tracing::info!(
        "scheduler: published {} ({status}) scheduled for {}",
        post.slug.as_deref().unwrap_or(&post.id),
        post.published_at.as_deref().unwrap_or("?"),
    );

    // The author is not here to be told, so the audit log carries it instead.
    // There is no user to attribute it to; the post's own author is the one
    // who asked for this, which is what Ghost records too.
    if let Some(author) = crate::post_author_id(state, &post.id).await {
        let _ = repos::actions::record(&state.db, &post.post_type, &post.id, &author, "published").await;
    }

    if status == "published" {
        crate::notify_slack_published(state, &post).await;
        crate::announce_published(state, &post).await;
    }

    if due.delivery.email {
        match crate::send_post_as_newsletter(state, &post.id).await {
            Ok(n) => tracing::info!("scheduler: emailed {} to {n} subscriber(s)", post.id),
            // Worth a warning rather than a retry: a second attempt would hit
            // the same missing Mailgun key or empty recipient list, and a
            // newsletter nobody can unsend is not something to loop on.
            Err(e) => tracing::warn!("scheduler: {} was published but not emailed: {e}", post.id),
        }
    }
}
