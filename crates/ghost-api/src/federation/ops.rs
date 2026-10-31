//! The social web's actions and reads, as plain functions.
//!
//! These exist so the JSON admin API and the server-rendered admin screens
//! are the same code. Before this they could not be: every action lived
//! inside an axum handler that took `Json` and returned a `Response`, so the
//! only way to drive them from a form POST was to re-implement them. One
//! implementation means a rule like "unblock before following" cannot hold in
//! one place and be forgotten in the other.

use super::*;

// ---------------------------------------------------------------------------
// reads
// ---------------------------------------------------------------------------

/// A note as the admin shows it: the text, who wrote it, and what has
/// happened to it since.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Note {
    pub id: String,
    pub url: String,
    pub text: String,
    pub published_at: String,
    pub local: bool,
    pub in_reply_to: Option<String>,
    pub author: profiles::Profile,
    pub likes: i64,
    pub boosts: i64,
    pub replies: i64,
}

/// Which slice of notes a feed is showing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Feed {
    /// Everything from the accounts this publication follows, and its own.
    Reader,
    /// Only this publication's own notes.
    Mine,
    /// The replies to one note.
    Replies,
}

/// Read a page of notes, with their authors and interaction counts resolved.
///
/// Counts come from one grouped query rather than three per row: a feed of
/// thirty notes was otherwise ninety round trips to SQLite.
pub async fn notes(
    pool: &sqlx::SqlitePool,
    feed: Feed,
    parent: Option<&str>,
    page: i64,
    per_page: i64,
) -> Vec<Note> {
    let offset = (page - 1).max(0) * per_page;
    let rows = match feed {
        Feed::Replies => {
            let Some(parent) = parent else { return Vec::new() };
            sqlx::query_as::<_, (String, String, String, String, Option<String>, String, bool)>(
                "SELECT id,actor,content,url,in_reply_to,published_at,local FROM federation_objects \
                 WHERE deleted=0 AND in_reply_to=? ORDER BY published_at ASC,id ASC LIMIT ? OFFSET ?",
            )
            .bind(parent)
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
        Feed::Mine => {
            sqlx::query_as(
                "SELECT id,actor,content,url,in_reply_to,published_at,local FROM federation_objects \
                 WHERE deleted=0 AND local=1 ORDER BY published_at DESC,id DESC LIMIT ? OFFSET ?",
            )
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
        Feed::Reader => {
            sqlx::query_as(
                "SELECT id,actor,content,url,in_reply_to,published_at,local FROM federation_objects o \
                 WHERE o.deleted=0 AND (o.local=1 OR EXISTS(SELECT 1 FROM federation_following f \
                   WHERE f.actor=o.actor AND f.accepted=1)) \
                 ORDER BY o.published_at DESC,o.id DESC LIMIT ? OFFSET ?",
            )
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
    }
    .unwrap_or_default();

    if rows.is_empty() {
        return Vec::new();
    }
    let actors: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    let who = profiles::lookup(pool, &actors).await;
    let ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let likes = counts(pool, &ids, "Like").await;
    let boosts = counts(pool, &ids, "Announce").await;
    let replies = reply_counts(pool, &ids).await;

    rows.into_iter()
        .map(|(id, actor, content, url, in_reply_to, published_at, local)| Note {
            likes: likes.get(&id).copied().unwrap_or(0),
            boosts: boosts.get(&id).copied().unwrap_or(0),
            replies: replies.get(&id).copied().unwrap_or(0),
            author: who
                .get(&actor)
                .cloned()
                .unwrap_or_else(|| profiles::Profile::unknown(&actor)),
            text: ghost_core::comments::to_plaintext(&content),
            id,
            url,
            in_reply_to,
            published_at,
            local,
        })
        .collect()
}

/// Likes or boosts per object, for a set of objects.
async fn counts(pool: &sqlx::SqlitePool, ids: &[String], kind: &str) -> HashMap<String, i64> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT object,COUNT(*) FROM federation_interactions \
         WHERE kind=? AND object IN ({placeholders}) GROUP BY object"
    );
    let mut query = sqlx::query_as::<_, (String, i64)>(&sql).bind(kind);
    for id in ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

/// Replies per object.
async fn reply_counts(pool: &sqlx::SqlitePool, ids: &[String]) -> HashMap<String, i64> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT in_reply_to,COUNT(*) FROM federation_objects \
         WHERE deleted=0 AND in_reply_to IN ({placeholders}) GROUP BY in_reply_to"
    );
    let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

/// One note by id, with its author and counts.
pub async fn note(pool: &sqlx::SqlitePool, id: &str) -> Option<Note> {
    let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, bool)>(
        "SELECT id,actor,content,url,in_reply_to,published_at,local FROM federation_objects \
         WHERE id=? AND deleted=0",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    let (id, actor, content, url, in_reply_to, published_at, local) = row;
    Some(Note {
        likes: *counts(pool, &[id.clone()], "Like").await.get(&id).unwrap_or(&0),
        boosts: *counts(pool, &[id.clone()], "Announce").await.get(&id).unwrap_or(&0),
        replies: *reply_counts(pool, &[id.clone()]).await.get(&id).unwrap_or(&0),
        author: profiles::one(pool, &actor).await,
        text: ghost_core::comments::to_plaintext(&content),
        id,
        url,
        in_reply_to,
        published_at,
        local,
    })
}

/// An account in one of the relationship lists.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Account {
    pub profile: profiles::Profile,
    /// For following: whether the other end has accepted yet.
    ///
    /// A follow is a handshake, not a switch — we send `Follow`, they send
    /// `Accept` back — so "not accepted" is the normal state for the first
    /// few seconds, and the permanent state when the handshake cannot
    /// complete.
    pub accepted: bool,
    pub since: String,
    /// Why the follow has not gone through, when the queue knows.
    ///
    /// Without this, a follow that was rejected outright looks identical to
    /// one that is simply in flight, and the screen gives the reader no way
    /// to tell "wait a moment" from "this will never work".
    pub delivery_error: Option<String>,
}

/// Which relationship list to read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Relation {
    Followers,
    Following,
    Blocked,
}

/// One page of a relationship list, with profiles resolved.
pub async fn accounts(
    pool: &sqlx::SqlitePool,
    relation: Relation,
    page: i64,
    per_page: i64,
) -> Vec<Account> {
    let offset = (page - 1).max(0) * per_page;
    let rows: Vec<(String, bool, String)> = match relation {
        Relation::Followers => sqlx::query_as(
            "SELECT actor,1,created_at FROM federation_followers ORDER BY created_at DESC LIMIT ? OFFSET ?",
        ),
        Relation::Following => sqlx::query_as(
            "SELECT actor,accepted,created_at FROM federation_following ORDER BY created_at DESC LIMIT ? OFFSET ?",
        ),
        Relation::Blocked => sqlx::query_as(
            "SELECT actor,1,created_at FROM federation_blocks ORDER BY created_at DESC LIMIT ? OFFSET ?",
        ),
    }
    .bind(per_page)
    .bind(offset)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let actors: Vec<String> = rows.iter().map(|(actor, _, _)| actor.clone()).collect();
    let who = profiles::lookup(pool, &actors).await;

    // Why a pending follow is pending. Only asked for the list that can show
    // it, and only for the follows that have not been accepted — a settled
    // follow has nothing to explain.
    let mut trouble: HashMap<String, String> = HashMap::new();
    if relation == Relation::Following {
        let failed = sqlx::query_as::<_, (String, String)>(
            "SELECT f.actor, d.last_error FROM federation_following f \
             JOIN federation_deliveries d ON d.activity_id = f.follow_id \
             WHERE f.accepted = 0 AND d.delivered_at IS NULL AND d.last_error IS NOT NULL",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        trouble.extend(failed);
    }

    rows.into_iter()
        .map(|(actor, accepted, since)| Account {
            profile: who
                .get(&actor)
                .cloned()
                .unwrap_or_else(|| profiles::Profile::unknown(&actor)),
            delivery_error: if accepted { None } else { trouble.get(&actor).cloned() },
            accepted,
            since,
        })
        .collect()
}

/// Something that happened to this publication.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Notification {
    pub id: String,
    pub kind: String,
    pub actor: profiles::Profile,
    pub text: String,
    /// The note it happened to, when there is one to link to.
    pub object: Option<String>,
    pub read: bool,
    pub created_at: String,
}

/// The notification feed, newest first.
///
/// `kind` filters to one type — Ghost's notification screen has the same
/// tabs, because a hundred likes otherwise bury the one reply worth reading.
pub async fn notifications(
    pool: &sqlx::SqlitePool,
    kind: Option<&str>,
    limit: i64,
) -> Vec<Notification> {
    let allowed = ["Follow", "Like", "Announce", "Create"];
    let filter = kind.filter(|k| allowed.contains(k));
    let sql = format!(
        "SELECT id,actor,activity_type,body,read_at,created_at FROM federation_received \
         WHERE undone=0 AND activity_type IN ('Follow','Like','Announce','Create'){} \
         ORDER BY created_at DESC LIMIT ?",
        if filter.is_some() { " AND activity_type=?" } else { "" }
    );
    let mut query =
        sqlx::query_as::<_, (String, String, String, String, Option<String>, String)>(&sql);
    if let Some(kind) = filter {
        query = query.bind(kind);
    }
    let rows = query.bind(limit).fetch_all(pool).await.unwrap_or_default();

    let actors: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    let who = profiles::lookup(pool, &actors).await;
    rows.into_iter()
        .map(|(id, actor, kind, body, read_at, created_at)| {
            let body = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
            Notification {
                id,
                actor: who
                    .get(&actor)
                    .cloned()
                    .unwrap_or_else(|| profiles::Profile::unknown(&actor)),
                text: ghost_core::comments::to_plaintext(
                    body.pointer("/object/content").and_then(Value::as_str).unwrap_or(""),
                ),
                object: match kind.as_str() {
                    // A like or a boost points at the note it acted on; a
                    // reply points at itself, which is where the thread is.
                    "Like" | "Announce" => {
                        object_id(&body["object"]).map(str::to_string)
                    }
                    "Create" => body.pointer("/object/id").and_then(Value::as_str).map(str::to_string),
                    _ => None,
                },
                read: read_at.is_some(),
                kind,
                created_at,
            }
        })
        .collect()
}

/// How many notifications have not been looked at.
pub async fn unread(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM federation_received WHERE read_at IS NULL AND undone=0 \
         AND activity_type IN ('Follow','Like','Announce','Create')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Counts for the publication's own profile header.
pub async fn summary(pool: &sqlx::SqlitePool) -> (i64, i64, i64) {
    let one = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await.unwrap_or(0)
    };
    (
        one("SELECT COUNT(*) FROM federation_followers").await,
        one("SELECT COUNT(*) FROM federation_following WHERE accepted=1").await,
        one("SELECT COUNT(*) FROM federation_objects WHERE local=1 AND deleted=0").await,
    )
}

// ---------------------------------------------------------------------------
// actions
// ---------------------------------------------------------------------------

/// Look an account up without following it.
///
/// This is the "search" the social web screens offer: type a handle, see who
/// it is, then decide. Following blind — which is all the old screen could
/// do — means the first time you find out you had the wrong Alice is after a
/// Follow has already been delivered to her.
pub async fn preview(state: &AppState, handle: &str) -> Result<profiles::Profile> {
    let handle = handle.trim();
    if handle.is_empty() || handle.len() > 2000 {
        return Err(bad("Enter @name@server or an actor URL"));
    }
    let found = network::discover_as(handle, signer_for(state).await.as_ref()).await?;
    profiles::store(&state.db, &found.id, &found.profile).await;
    Ok(profiles::one(&state.db, &found.id).await)
}

/// Mark every notification as seen.
pub async fn mark_read(state: &AppState) -> Result<()> {
    sqlx::query("UPDATE federation_received SET read_at=? WHERE read_at IS NULL")
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(db_error)?;
    Ok(())
}

/// Publish a note, optionally as a reply.
///
/// Returns the new note's id. Delivery is queued inside the same transaction
/// that stores the note, so a note is never visible locally without its
/// deliveries having been scheduled.
pub async fn publish_note(
    state: &AppState,
    markdown: &str,
    in_reply_to: Option<&str>,
    reply_actor: Option<&str>,
) -> Result<String> {
    if !enabled(state) {
        return Err(bad("Enable the social web first"));
    }
    let markdown = markdown.trim();
    if markdown.is_empty() || markdown.chars().count() > 10000 {
        return Err(bad("Write a note of 1–10000 characters"));
    }
    let reply = in_reply_to.filter(|url| !url.is_empty());
    if reply.is_some_and(|url| {
        url.len() > 2000 || !reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "https")
    }) {
        return Err(bad("Invalid reply target"));
    }
    let content = ghost_render::markdown::to_html(markdown);
    let uuid = ids::uuid_v4();
    let id = endpoint(state, &format!("objects/{uuid}"));
    let url = endpoint(state, &format!("notes/{uuid}"));
    let now = dates::now_db();
    let mut inboxes = follower_inboxes(state).await;
    let mut mentions = Vec::new();
    if let Some(actor) = reply_actor.filter(|a| !a.is_empty()) {
        let actor = network::actor_as(actor, signer_for(state).await.as_ref()).await?;
        inboxes.push(actor.inbox);
        mentions.push(actor.id);
    }
    let object = json!({"id":id,"type":"Note","attributedTo":actor_id(state),"content":content,"inReplyTo":reply,"url":url,"published":chrono::Utc::now().to_rfc3339(),"to":[PUBLIC],"cc":mentions});
    let mut activity = activity_body(state, "Create", object);
    activity["cc"] = json!(mentions);
    let mut tx = state.db.begin().await.map_err(db_error)?;
    sqlx::query("INSERT INTO federation_objects (id,actor,object_type,content,url,in_reply_to,published_at,updated_at,local) VALUES (?,?,'Note',?,?,?,?,?,1)")
        .bind(&id).bind(actor_id(state)).bind(content).bind(&url).bind(reply).bind(&now).bind(&now)
        .execute(&mut *tx).await.map_err(db_error)?;
    enqueue_tx(&mut tx, &activity, &inboxes).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(id)
}

/// Withdraw one of this publication's own notes.
///
/// `uuid` is the trailing segment of the object id, which is what the admin
/// screens carry around — the full URL is rebuilt here so a caller cannot ask
/// for the deletion of something on another server.
pub async fn delete_note(state: &AppState, uuid: &str) -> Result<()> {
    let object = endpoint(state, &format!("objects/{uuid}"));
    let mut tx = state.db.begin().await.map_err(db_error)?;
    let changed = sqlx::query(
        "UPDATE federation_objects SET deleted=1,content='',updated_at=? \
         WHERE id=? AND local=1 AND deleted=0",
    )
    .bind(dates::now_db())
    .bind(&object)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    if changed.rows_affected() != 1 {
        return Err(bad("That note no longer exists"));
    }
    let activity = activity_body(state, "Delete", json!({"id":object,"type":"Tombstone"}));
    let inboxes = sqlx::query_scalar::<_, String>("SELECT DISTINCT inbox FROM federation_followers")
        .fetch_all(&mut *tx)
        .await
        .unwrap_or_default();
    enqueue_tx(&mut tx, &activity, &inboxes).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

/// Follow an account, by handle or actor URL.
pub async fn follow(state: &AppState, handle: &str) -> Result<profiles::Profile> {
    if !enabled(state) {
        return Err(bad("Enable the social web first"));
    }
    if handle.len() > 2000 {
        return Err(bad("Invalid actor"));
    }
    let actor = network::discover_as(handle, signer_for(state).await.as_ref()).await?;
    if actor.id == actor_id(state) {
        return Err(bad("Cannot follow the publication itself"));
    }
    let blocked: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM federation_blocks WHERE actor=?)")
            .bind(&actor.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(true);
    if blocked {
        return Err(bad("Unblock this account before following it"));
    }
    // Who they are is worth keeping whether or not the follow is new: it is
    // what every list will render them with.
    profiles::store(&state.db, &actor.id, &actor.profile).await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM federation_following WHERE actor=?)")
            .bind(&actor.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);
    if exists {
        return Ok(profiles::one(&state.db, &actor.id).await);
    }
    let mut activity = activity_body(state, "Follow", json!(actor.id));
    activity["to"] = json!([actor.id]);
    activity["cc"] = json!([]);
    let mut tx = state.db.begin().await.map_err(db_error)?;
    sqlx::query("INSERT INTO federation_following (actor,inbox,follow_id,created_at) VALUES (?,?,?,?)")
        .bind(&actor.id)
        .bind(&actor.inbox)
        .bind(activity["id"].as_str().unwrap_or_default())
        .bind(dates::now_db())
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    enqueue_tx(&mut tx, &activity, &[actor.inbox.clone()]).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(profiles::one(&state.db, &actor.id).await)
}

/// Stop following an account.
pub async fn unfollow(state: &AppState, actor: &str) -> Result<()> {
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT inbox,follow_id FROM federation_following WHERE actor=?",
    )
    .bind(actor)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((inbox, id)) = existing else {
        return Err(bad("Not following that account"));
    };
    let mut activity = activity_body(
        state,
        "Undo",
        json!({"id":id,"type":"Follow","actor":actor_id(state),"object":actor}),
    );
    activity["to"] = json!([actor]);
    activity["cc"] = json!([]);
    let mut tx = state.db.begin().await.map_err(db_error)?;
    enqueue_tx(&mut tx, &activity, &[inbox]).await?;
    sqlx::query("DELETE FROM federation_following WHERE actor=?")
        .bind(actor)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

/// Like, boost, or undo one of either.
///
/// Returns the id of the activity sent, which is what an Undo later needs.
pub async fn interact(
    state: &AppState,
    kind: &str,
    actor: &str,
    target: &str,
) -> Result<String> {
    if !enabled(state) {
        return Err(bad("Enable the social web first"));
    }
    if !matches!(kind, "Like" | "Announce" | "Undo") {
        return Err(bad("Choose Like, Announce or Undo"));
    }
    if target.len() > 2000 || !reqwest::Url::parse(target).is_ok_and(|url| url.scheme() == "https") {
        return Err(bad("Invalid interaction target"));
    }
    let actor = network::actor_as(actor, signer_for(state).await.as_ref()).await?;
    let object = if kind == "Undo" {
        let original = sqlx::query_scalar::<_, String>(
            "SELECT body FROM federation_activities WHERE id=? \
             AND activity_type IN ('Like','Announce') AND undone=0",
        )
        .bind(target)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        original
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .ok_or_else(|| bad("That interaction has already been withdrawn"))?
    } else {
        json!(target)
    };
    let mut activity = activity_body(state, kind, object);
    activity["cc"] = json!([actor.id]);
    let mut inboxes = vec![actor.inbox];
    if kind == "Announce" || kind == "Undo" {
        inboxes.extend(follower_inboxes(state).await);
    }
    let mut tx = state.db.begin().await.map_err(db_error)?;
    if kind == "Undo" {
        sqlx::query("UPDATE federation_activities SET undone=1 WHERE id=?")
            .bind(target)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    enqueue_tx(&mut tx, &activity, &inboxes).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(activity["id"].as_str().unwrap_or_default().to_string())
}

/// Which of our own Likes and Announces are currently standing, so the feed
/// can show a liked note as liked rather than offering to like it again.
pub async fn own_interactions(pool: &sqlx::SqlitePool) -> HashMap<(String, String), String> {
    sqlx::query_as::<_, (String, String, String)>(
        "SELECT id,activity_type,object_id FROM federation_activities \
         WHERE undone=0 AND activity_type IN ('Like','Announce') AND object_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id, kind, object)| ((object, kind), id))
    .collect()
}

/// Block or unblock an account.
///
/// Blocking also drops the relationship in both directions: a blocked account
/// that stayed in the follower list would keep being delivered to.
pub async fn set_block(state: &AppState, actor: &str, blocked: bool) -> Result<()> {
    if !reqwest::Url::parse(actor).is_ok_and(|url| url.scheme() == "https") {
        return Err(bad("Invalid actor URL"));
    }
    let mut tx = state.db.begin().await.map_err(db_error)?;
    if blocked {
        sqlx::query("INSERT OR IGNORE INTO federation_blocks (actor,created_at) VALUES (?,?)")
            .bind(actor)
            .bind(dates::now_db())
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        for table in ["federation_followers", "federation_following"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE actor=?"))
                .bind(actor)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
    } else {
        sqlx::query("DELETE FROM federation_blocks WHERE actor=?")
            .bind(actor)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

/// An account worth following, and the reason it is being suggested.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Suggestion {
    pub profile: profiles::Profile,
    /// Why this one, in the reader's terms. Shown on the card: a suggestion
    /// with no stated reason is just a list.
    pub reason: String,
}

/// Accounts to explore, drawn from what this site already knows.
///
/// Ghost's own Explore is a curated directory served by its hosted
/// ActivityPub service, which a self-hosted install has no access to. Rather
/// than ship an invented list of strangers, this suggests the accounts the
/// site has a real connection to and is not yet following: people who
/// followed first, and people whose notes have already reached the feed by
/// being replied to or boosted by somebody followed.
pub async fn suggestions(pool: &sqlx::SqlitePool, local_actor: &str, limit: i64) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(local_actor.to_string());

    // Followers who are not followed back. The clearest signal there is.
    let followers = sqlx::query_scalar::<_, String>(
        "SELECT f.actor FROM federation_followers f \
         WHERE NOT EXISTS(SELECT 1 FROM federation_following g WHERE g.actor=f.actor) \
           AND NOT EXISTS(SELECT 1 FROM federation_blocks b WHERE b.actor=f.actor) \
         ORDER BY f.created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    // Authors whose notes are already here but who are not followed —
    // reached by a reply or a boost from somebody who is.
    let in_feed = sqlx::query_scalar::<_, String>(
        "SELECT o.actor FROM federation_objects o \
         WHERE o.local=0 AND o.deleted=0 \
           AND NOT EXISTS(SELECT 1 FROM federation_following g WHERE g.actor=o.actor) \
           AND NOT EXISTS(SELECT 1 FROM federation_blocks b WHERE b.actor=o.actor) \
         GROUP BY o.actor ORDER BY COUNT(*) DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let all: Vec<String> = followers.iter().chain(in_feed.iter()).cloned().collect();
    let who = profiles::lookup(pool, &all).await;

    for (actors, reason) in [
        (followers, "Follows your publication"),
        (in_feed, "Their notes already reach your feed"),
    ] {
        for actor in actors {
            if !seen.insert(actor.clone()) {
                continue;
            }
            out.push(Suggestion {
                profile: who
                    .get(&actor)
                    .cloned()
                    .unwrap_or_else(|| profiles::Profile::unknown(&actor)),
                reason: reason.to_string(),
            });
        }
    }
    out.truncate(limit as usize);
    out
}

/// The publications this site recommends, as hosts to try for a handle.
///
/// Ghost's own publications answer to `@index@<host>`, which is what makes a
/// recommendation list a usable source of fediverse accounts at all. The
/// lookup is deliberately not done here: resolving a dozen of them would be a
/// dozen remote fetches on every page load, so the screen offers each as a
/// button and the fetch happens when one is pressed.
pub async fn recommended_hosts(pool: &sqlx::SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT title,url FROM recommendations ORDER BY featured DESC, created_at DESC LIMIT 24",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(title, url)| {
        let host = reqwest::Url::parse(&url).ok()?.host_str()?.to_string();
        Some((title, host))
    })
    .collect()
}
