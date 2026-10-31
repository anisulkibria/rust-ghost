//! The social web screens — Ghost's Fediverse/ActivityPub admin.
//!
//! This replaces a script that built unstyled DOM nodes into a bare card.
//! Everything it offered is here, in the admin's own Material 3 language, and
//! quite a lot it did not: real identities instead of actor URLs, followers
//! as well as following, threads, interaction counts, account search before
//! you commit to a follow, and notifications split by kind.
//!
//! Server-rendered, like the rest of this admin. Every action is a form POST
//! that redirects with a flash, so the screens work without JavaScript and
//! there is no second copy of the rules living in a browser.

use crate::{esc, svg};
use ghost_api::federation::{ops, profiles::Profile};

/// The sections the social web is split into.
///
/// Ghost's own ordering: what other people said, then what you said, then
/// what happened to it, then who is on each side of the relationship.
pub const TABS: &[(&str, &str)] = &[
    ("feed", "Feed"),
    ("explore", "Explore"),
    ("notes", "Your notes"),
    ("notifications", "Notifications"),
    ("following", "Following"),
    ("followers", "Followers"),
    ("blocked", "Blocked"),
];

/// The notification filters, as (query value, label, activity type).
pub const NOTIFICATION_KINDS: &[(&str, &str, Option<&str>)] = &[
    ("all", "Everything", None),
    ("Create", "Replies", Some("Create")),
    ("Like", "Likes", Some("Like")),
    ("Announce", "Boosts", Some("Announce")),
    ("Follow", "Follows", Some("Follow")),
];

/// Someone's avatar, or their initial when they have none.
///
/// Remote avatars are hotlinked, as every fediverse client does — proxying
/// them would mean this server fetching an image for every row of every list.
/// `loading="lazy"` keeps a long feed from opening fifty connections at once,
/// and `referrerpolicy` keeps the admin's URL out of the other server's logs.
fn avatar(profile: &Profile, large: bool) -> String {
    let class = if large { "avatar lg" } else { "avatar" };
    match profile.avatar.as_deref() {
        Some(url) => format!(
            r#"<img class="{class}" src="{url}" alt="" loading="lazy" referrerpolicy="no-referrer" decoding="async">"#,
            class = class,
            url = esc(url),
        ),
        None => format!(
            r#"<span class="{class}">{initial}</span>"#,
            class = class,
            initial = esc(
                &profile
                    .display()
                    .trim_start_matches('@')
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string()
            ),
        ),
    }
}

/// A name and handle, linking to the account's own page here.
fn identity(profile: &Profile) -> String {
    format!(
        r#"<a class="sw-who" href="/ghost/social/account/?actor={q}">{avatar}
<span class="sw-names"><b>{name}</b><span class="sw-handle">{handle}</span></span></a>"#,
        q = esc(&url_value(&profile.actor)),
        avatar = avatar(profile, false),
        name = esc(profile.display()),
        handle = esc(&profile.handle),
    )
}

/// Percent-encode one query value.
fn url_value(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// `2026-10-30 09:00:00` as something a person reads.
///
/// Relative for the first day, because "3 hours ago" is what a feed is for,
/// then the date, because "47 days ago" is not.
fn when(stamp: &str) -> String {
    let Some(then) = ghost_core::dates::from_db_string(stamp) else {
        return esc(stamp);
    };
    let minutes = (chrono::Utc::now() - then).num_minutes();
    match minutes {
        m if m < 1 => "just now".to_string(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m if m < 60 * 24 * 7 => format!("{}d ago", m / (60 * 24)),
        _ => then.format("%-d %b %Y").to_string(),
    }
}

/// One note, as a card with its author, text and what it has collected.
///
/// `own` maps `(object, kind)` to the activity id of our own standing Like or
/// Announce, so a note already liked offers to unlike rather than to like
/// again — a button that silently does nothing the second time is worse than
/// no button.
///
/// `return_to` is where this card's forms come back to. A like sent from
/// inside a thread has to land back in that thread: bouncing to the top of
/// the feed loses the reader's place, which on a feed is most of what they
/// had.
pub fn note_card(
    note: &ops::Note,
    own: &std::collections::HashMap<(String, String), String>,
    return_to: &str,
) -> String {
    let liked = own.get(&(note.id.clone(), "Like".to_string()));
    let boosted = own.get(&(note.id.clone(), "Announce".to_string()));
    let back = format!(
        r#"<input type="hidden" name="return" value="{}">"#,
        esc(return_to)
    );

    // A note of ours is ours to withdraw; a note of theirs is one to react to.
    let actions = if note.local {
        format!(
            r#"<form method="post" action="/ghost/social/notes/delete/" class="sw-act"
 data-confirm="Delete this note|It will be withdrawn from every server it reached.">
<input type="hidden" name="id" value="{id}">{back}
<button class="sw-btn danger" type="submit">{icon} Delete</button></form>"#,
            id = esc(&note.id),
            back = back,
            icon = svg("delete"),
        )
    } else {
        format!(
            r#"{like}{boost}
<form method="post" action="/ghost/social/block/" class="sw-act"
 data-confirm="Block {handle}|Their posts stop appearing here, and the follow is dropped in both directions.">
<input type="hidden" name="actor" value="{actor}">{back}
<button class="sw-btn danger" type="submit">{block_icon} Block</button></form>"#,
            handle = esc(&note.author.handle),
            actor = esc(&note.author.actor),
            back = back,
            block_icon = svg("close"),
            like = interaction_form(note, "Like", liked, "Like", "Liked", &back),
            boost = interaction_form(note, "Announce", boosted, "Boost", "Boosted", &back),
        )
    };

    let reply_to = match &note.in_reply_to {
        Some(parent) => format!(
            r#"<a class="sw-replyto" href="/ghost/social/thread/?id={q}">{icon} In reply to a note</a>"#,
            q = esc(&url_value(parent)),
            icon = svg("chevron"),
        ),
        _ => String::new(),
    };

    let counts = format!(
        r#"<a class="sw-count" href="/ghost/social/thread/?id={q}" title="Replies">{reply_icon}{replies}</a>
<span class="sw-count" title="Likes">{heart}{likes}</span>
<span class="sw-count" title="Boosts">{repeat}{boosts}</span>"#,
        q = esc(&url_value(&note.id)),
        reply_icon = svg("comment"),
        replies = note.replies,
        heart = svg("heart"),
        likes = note.likes,
        repeat = svg("repeat"),
        boosts = note.boosts,
    );

    format!(
        r#"<article class="card padded sw-note rise">
<header class="sw-note-head">{identity}<span class="sw-time">{when}</span></header>
{reply_to}
<div class="sw-text">{text}</div>
<footer class="sw-note-foot"><div class="sw-counts">{counts}</div>
<div class="sw-actions">{reply}{actions}{source}</div></footer>
</article>"#,
        identity = identity(&note.author),
        when = esc(&when(&note.published_at)),
        reply_to = reply_to,
        // Plain text, escaped: note bodies arrive as HTML from servers we do
        // not control, so they are flattened to text upstream and never put
        // back into the page as markup.
        text = esc(&note.text),
        counts = counts,
        reply = format!(
            r#"<a class="sw-btn" href="/ghost/social/thread/?id={q}">{icon} Reply</a>"#,
            q = esc(&url_value(&note.id)),
            icon = svg("comment"),
        ),
        actions = actions,
        source = format!(
            r#"<a class="sw-btn" href="{url}" target="_blank" rel="noopener noreferrer">{icon} Open</a>"#,
            url = esc(&note.url),
            icon = svg("open"),
        ),
    )
}

/// The Like/Boost button, in whichever of its two states applies.
fn interaction_form(
    note: &ops::Note,
    kind: &str,
    standing: Option<&String>,
    verb: &str,
    done: &str,
    back: &str,
) -> String {
    match standing {
        // Already done: the button undoes it, and says so. An Undo names the
        // activity that is being withdrawn, not the note — which is why the
        // standing activity id has to be carried here at all.
        Some(activity) => format!(
            r#"<form method="post" action="/ghost/social/interact/" class="sw-act">
<input type="hidden" name="type" value="Undo">
<input type="hidden" name="actor" value="{actor}">
<input type="hidden" name="object" value="{activity}">{back}
<button class="sw-btn on" type="submit">{icon} {done}</button></form>"#,
            actor = esc(&note.author.actor),
            activity = esc(activity),
            back = back,
            icon = if kind == "Like" { svg("heart") } else { svg("repeat") },
            done = esc(done),
        ),
        None => format!(
            r#"<form method="post" action="/ghost/social/interact/" class="sw-act">
<input type="hidden" name="type" value="{kind}">
<input type="hidden" name="actor" value="{actor}">
<input type="hidden" name="object" value="{object}">{back}
<button class="sw-btn" type="submit">{icon} {verb}</button></form>"#,
            kind = esc(kind),
            actor = esc(&note.author.actor),
            object = esc(&note.id),
            back = back,
            icon = if kind == "Like" { svg("heart") } else { svg("repeat") },
            verb = esc(verb),
        ),
    }
}

/// The composer. Used for a new note and for a reply, which differ only in
/// what they carry and what the button says.
pub fn composer(reply_to: Option<&ops::Note>) -> String {
    let (action_label, hidden, placeholder) = match reply_to {
        Some(note) => (
            "Publish reply",
            format!(
                r#"<input type="hidden" name="in_reply_to" value="{id}"><input type="hidden" name="reply_actor" value="{actor}">"#,
                id = esc(&note.id),
                actor = esc(&note.author.actor),
            ),
            format!("Reply to {}", note.author.display()),
        ),
        None => (
            "Publish note",
            String::new(),
            "Share something with the fediverse".to_string(),
        ),
    };
    format!(
        r#"<form method="post" action="/ghost/social/notes/" class="card padded sw-composer rise">
{hidden}
<label class="field" style="margin:0"><textarea name="text" rows="3" maxlength="10000" placeholder=" " required></textarea><span>{placeholder}</span></label>
<div class="sw-composer-foot"><p class="muted">Markdown is supported. Notes are public and go to every follower.</p>
<button class="btn" type="submit">{icon} {label}</button></div>
</form>"#,
        hidden = hidden,
        placeholder = esc(&placeholder),
        icon = svg("send"),
        label = esc(action_label),
    )
}

/// One row of a relationship list.
pub fn account_row(account: &ops::Account, relation: ops::Relation) -> String {
    let actor = esc(&account.profile.actor);
    let action = match relation {
        ops::Relation::Following => format!(
            r#"<form method="post" action="/ghost/social/follow/" data-confirm="Unfollow {handle}|Their notes stop appearing in your feed.">
<input type="hidden" name="actor" value="{actor}"><input type="hidden" name="unfollow" value="1">
<button class="btn tonal small" type="submit">Unfollow</button></form>"#,
            handle = esc(&account.profile.handle),
            actor = actor,
        ),
        ops::Relation::Blocked => format!(
            r#"<form method="post" action="/ghost/social/block/">
<input type="hidden" name="actor" value="{actor}"><input type="hidden" name="unblock" value="1">
<button class="btn tonal small" type="submit">Unblock</button></form>"#,
            actor = actor,
        ),
        ops::Relation::Followers => format!(
            r#"<form method="post" action="/ghost/social/follow/">
<input type="hidden" name="actor" value="{actor}">
<button class="btn tonal small" type="submit">Follow back</button></form>"#,
            actor = actor,
        ),
    };
    // A follow is a handshake: we send Follow, their server sends Accept
    // back. Until it does, the follow is pending — so the row says which kind
    // of pending it is, because "it failed" and "give it a moment" look
    // identical otherwise.
    let detail = match relation {
        ops::Relation::Following if !account.accepted => match &account.delivery_error {
            Some(error) => format!(
                r#"<span class="chip draft" title="{title}">Not delivered</span>"#,
                title = esc(&format!(
                    "Their server did not accept the follow: {error}. \
                     A follow is only completed when the other server can reach this site back, \
                     which needs a public HTTPS address."
                )),
            ),
            None => r#"<span class="chip draft" title="Sent. Their server completes the follow by sending an Accept back, which usually takes a few seconds.">Awaiting acceptance</span>"#.to_string(),
        },
        _ => format!(
            r#"<span class="sw-since">Since {}</span>"#,
            esc(&when(&account.since))
        ),
    };
    format!(
        r#"<div class="row sw-row">{identity}
<div class="trailing sw-row-end">{detail}{action}</div></div>"#,
        identity = identity(&account.profile),
        detail = detail,
        action = action,
    )
}

/// One notification.
pub fn notification_row(item: &ops::Notification) -> String {
    let (icon, tint, said) = match item.kind.as_str() {
        "Follow" => (svg("ghost"), "blue", "followed your publication"),
        "Like" => (svg("heart"), "pink", "liked your note"),
        "Announce" => (svg("repeat"), "green", "boosted your note"),
        _ => (svg("comment"), "amber", "replied to you"),
    };
    let body = if item.text.is_empty() {
        String::new()
    } else {
        format!(
            r#"<span class="detail sw-quote">{}</span>"#,
            esc(&item.text.chars().take(280).collect::<String>())
        )
    };
    let link = match &item.object {
        Some(object) => format!(
            r#"<a class="btn tonal small" href="/ghost/social/thread/?id={q}">View</a>"#,
            q = esc(&url_value(object)),
        ),
        None => String::new(),
    };
    format!(
        r#"<div class="row sw-notification{unread}"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{name} <span class="sw-said">{said}</span></span>{handle}{body}</div>
<div class="trailing"><span class="sw-since">{when}</span>{link}</div></div>"#,
        unread = if item.read { "" } else { " unread" },
        tint = tint,
        icon = icon,
        name = esc(item.actor.display()),
        said = esc(said),
        handle = format!(
            r#"<span class="detail">{}</span>"#,
            esc(&item.actor.handle)
        ),
        body = body,
        when = esc(&when(&item.created_at)),
        link = link,
    )
}

/// The search box that finds an account before you follow it.
pub fn search_box(query: &str, found: Option<&Profile>, error: Option<&str>) -> String {
    let result = match (found, error) {
        (_, Some(message)) => format!(
            r#"<p class="sw-search-miss">{}</p>"#,
            esc(message)
        ),
        (Some(profile), _) => format!(
            r#"<div class="sw-found">{identity}
<div class="sw-found-end">{counts}
<form method="post" action="/ghost/social/follow/"><input type="hidden" name="actor" value="{actor}">
<button class="btn" type="submit">{icon} Follow</button></form></div>
{summary}</div>"#,
            identity = identity(profile),
            actor = esc(&profile.actor),
            icon = svg("add"),
            counts = profile_counts(profile),
            summary = match profile.summary.as_deref() {
                Some(text) if !text.is_empty() =>
                    format!(r#"<p class="sw-bio">{}</p>"#, esc(text)),
                _ => String::new(),
            },
        ),
        _ => String::new(),
    };
    format!(
        r#"<form method="get" action="/ghost/social/" class="card padded sw-search rise">
<input type="hidden" name="tab" value="following">
<label class="field" style="margin:0"><input name="q" value="{query}" placeholder=" " autocomplete="off"><span>Find an account · @name@server or an actor URL</span></label>
<button class="btn tonal" type="submit">{icon} Look up</button>
</form>{result}"#,
        query = esc(query),
        icon = svg("search"),
        result = result,
    )
}

/// Followers / following / posts, when the other server reported them.
fn profile_counts(profile: &Profile) -> String {
    let part = |n: Option<i64>, one: &str, many: &str| match n {
        Some(n) => format!(
            r#"<span><b>{}</b> {}</span>"#,
            crate::compact(n),
            if n == 1 { one } else { many }
        ),
        None => String::new(),
    };
    let body = format!(
        "{}{}{}",
        part(profile.posts, "post", "posts"),
        part(profile.followers, "follower", "followers"),
        part(profile.following, "following", "following"),
    );
    if body.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="sw-stats">{body}</div>"#)
    }
}

/// The publication's own identity card, at the top of every section.
pub fn publication_header(
    handle: &str,
    actor: &str,
    followers: i64,
    following: i64,
    posts: i64,
    enabled: bool,
) -> String {
    let toggle = format!(
        r#"<form method="post" action="/ghost/social/enabled/"{confirm}>
<input type="hidden" name="enabled" value="{next}">
<button class="btn {style} small" type="submit">{label}</button></form>"#,
        confirm = if enabled {
            r#" data-confirm="Turn off the social web|Your publication stops federating. Followers keep their follow, but nothing new reaches them.""#
        } else {
            ""
        },
        next = if enabled { "0" } else { "1" },
        style = if enabled { "tonal" } else { "" },
        label = if enabled { "Disable" } else { "Enable social web" },
    );
    format!(
        r#"<div class="card padded sw-profile rise">
<span class="avatar lg">{initial}</span>
<div class="sw-profile-main"><b>{handle}</b>
<div class="sw-stats"><span><b>{posts}</b> {posts_word}</span><span><b>{followers}</b> {followers_word}</span><span><b>{following}</b> following</span></div></div>
<div class="sw-profile-end">{toggle}
<a class="btn text small" href="{actor}" target="_blank" rel="noopener noreferrer">{icon} Actor</a></div>
</div>"#,
        initial = esc(
            &handle
                .trim_start_matches('@')
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string()
        ),
        handle = esc(handle),
        posts = crate::compact(posts),
        posts_word = if posts == 1 { "note" } else { "notes" },
        followers = crate::compact(followers),
        followers_word = if followers == 1 { "follower" } else { "followers" },
        following = crate::compact(following),
        toggle = toggle,
        actor = esc(actor),
        icon = svg("open"),
    )
}

/// A remote account's own page header: who they are and what we can do
/// about them.
///
/// Blocked wins over following in what it offers: an account you have blocked
/// is not one you are deciding whether to follow, and showing both reads as a
/// contradiction.
pub fn account_card(profile: &Profile, following: bool, blocked: bool) -> String {
    let action = if blocked {
        format!(
            r#"<form method="post" action="/ghost/social/block/"><input type="hidden" name="actor" value="{actor}">
<input type="hidden" name="unblock" value="1"><input type="hidden" name="return" value="/ghost/social/account/?actor={q}">
<button class="btn tonal" type="submit">Unblock</button></form>"#,
            actor = esc(&profile.actor),
            q = esc(&url_value(&profile.actor)),
        )
    } else {
        format!(
            r#"<form method="post" action="/ghost/social/follow/"{confirm}>
<input type="hidden" name="actor" value="{actor}">{unfollow}
<input type="hidden" name="return" value="/ghost/social/account/?actor={q}">
<button class="btn{style}" type="submit">{label}</button></form>
<form method="post" action="/ghost/social/block/" data-confirm="Block {handle}|Their posts stop appearing here, and the follow is dropped in both directions.">
<input type="hidden" name="actor" value="{actor}">
<button class="btn text danger" type="submit">Block</button></form>"#,
            confirm = if following {
                r#" data-confirm="Unfollow this account|Their notes stop appearing in your feed.""#
            } else {
                ""
            },
            actor = esc(&profile.actor),
            q = esc(&url_value(&profile.actor)),
            unfollow = if following {
                r#"<input type="hidden" name="unfollow" value="1">"#
            } else {
                ""
            },
            style = if following { " tonal" } else { "" },
            label = if following { "Unfollow" } else { "Follow" },
            handle = esc(&profile.handle),
        )
    };
    format!(
        r#"<div class="card padded sw-profile rise">{avatar}
<div class="sw-profile-main"><b>{name}</b><span class="sw-handle">{handle}</span>
{counts}{bio}</div>
<div class="sw-profile-end">{action}
<a class="btn text small" href="{url}" target="_blank" rel="noopener noreferrer">{icon} Open profile</a></div>
</div>"#,
        avatar = avatar(profile, true),
        name = esc(profile.display()),
        handle = esc(&profile.handle),
        counts = profile_counts(profile),
        bio = match profile.summary.as_deref() {
            Some(text) if !text.is_empty() => format!(r#"<p class="sw-bio">{}</p>"#, esc(text)),
            _ => String::new(),
        },
        action = action,
        url = esc(profile.url.as_deref().unwrap_or(&profile.actor)),
        icon = svg("open"),
    )
}

/// One suggested account, with the reason it is being suggested.
pub fn suggestion_card(item: &ops::Suggestion) -> String {
    format!(
        r#"<div class="card padded sw-suggest rise">{identity}
<p class="sw-reason">{reason}</p>{bio}
<div class="sw-suggest-end">
<form method="post" action="/ghost/social/follow/">
<input type="hidden" name="actor" value="{actor}">
<input type="hidden" name="return" value="/ghost/social/?tab=explore">
<button class="btn small" type="submit">{icon} Follow</button></form>
<a class="btn text small" href="/ghost/social/account/?actor={q}">View</a></div></div>"#,
        identity = identity(&item.profile),
        reason = esc(&item.reason),
        bio = match item.profile.summary.as_deref() {
            Some(text) if !text.is_empty() => format!(
                r#"<p class="sw-bio">{}</p>"#,
                esc(&text.chars().take(200).collect::<String>())
            ),
            _ => String::new(),
        },
        actor = esc(&item.profile.actor),
        q = esc(&url_value(&item.profile.actor)),
        icon = svg("add"),
    )
}

/// The publications this site recommends, offered as accounts to look up.
///
/// A Ghost publication answers to `@index@<host>`, so a recommendation is one
/// webfinger call away from being a followable account. The call happens when
/// the button is pressed rather than on load — a dozen recommendations would
/// otherwise be a dozen remote fetches every time this tab is opened.
pub fn recommended_publications(hosts: &[(String, String)]) -> String {
    if hosts.is_empty() {
        return String::new();
    }
    let rows: String = hosts
        .iter()
        .map(|(title, host)| {
            format!(
                r#"<div class="row sw-row"><span class="ic tint-teal">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">@index@{host}</span></div>
<div class="trailing"><form method="get" action="/ghost/social/">
<input type="hidden" name="tab" value="following">
<input type="hidden" name="q" value="@index@{host}">
<button class="btn tonal small" type="submit">Look up</button></form></div></div>"#,
                icon = svg("globe"),
                title = esc(title),
                host = esc(host),
            )
        })
        .collect();
    format!(
        r#"<div class="section">Publications you recommend</div>
<p class="sw-section-note">A Ghost publication federates as <code>@index@its-domain</code>. These are the sites from Settings &rsaquo; Growth &rsaquo; Recommendations.</p>
<div class="card rows rise">{rows}</div>"#
    )
}

/// An empty section, saying what would fill it.
pub fn empty(title: &str, detail: &str) -> String {
    format!(
        r#"<div class="card padded rise" style="text-align:center;padding:44px 20px">
<p style="margin:0 0 4px;font-weight:600;font-size:17px">{}</p>
<p class="muted" style="margin:0">{}</p></div>"#,
        esc(title),
        esc(detail),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_api::federation::ops::Note;
    use std::collections::HashMap;

    fn profile(actor: &str, name: Option<&str>) -> Profile {
        Profile {
            actor: actor.into(),
            handle: "@alice@remote.test".into(),
            name: name.map(str::to_string),
            ..Default::default()
        }
    }

    fn note(local: bool) -> Note {
        Note {
            id: "https://remote.test/notes/1".into(),
            url: "https://remote.test/@alice/1".into(),
            text: "Hello".into(),
            published_at: "2026-10-30 09:00:00".into(),
            local,
            in_reply_to: None,
            author: profile("https://remote.test/users/alice", Some("Alice")),
            likes: 2,
            boosts: 1,
            replies: 3,
        }
    }

    /// A note already liked has to offer to undo that like, carrying the
    /// activity id the undo needs. Offering "Like" again would post a second
    /// Like that the receiving server simply ignores — a button that looks
    /// like it worked and did nothing.
    #[test]
    fn an_already_liked_note_offers_to_undo_rather_than_like_again() {
        let note = note(false);
        let mut own = HashMap::new();
        own.insert(
            (note.id.clone(), "Like".to_string()),
            "https://local.test/activities/abc".to_string(),
        );
        let html = note_card(&note, &own, "/ghost/social/");
        assert!(html.contains(r#"value="Undo""#), "{html}");
        assert!(html.contains("https://local.test/activities/abc"), "{html}");
        assert!(html.contains("Liked"), "{html}");
        // The boost is untouched: it was not the thing that was done.
        assert!(html.contains(r#"value="Announce""#), "{html}");

        // With nothing standing, it offers the plain Like.
        let fresh = note_card(&note, &HashMap::new(), "/ghost/social/");
        assert!(fresh.contains(r#"value="Like""#), "{fresh}");
        assert!(!fresh.contains(r#"value="Undo""#), "{fresh}");
    }

    /// Our own notes are ours to withdraw, and are never offered a Like,
    /// Boost or Block button aimed at ourselves.
    #[test]
    fn a_local_note_offers_deletion_and_no_self_interaction() {
        let html = note_card(&note(true), &HashMap::new(), "/ghost/social/");
        assert!(html.contains("/ghost/social/notes/delete/"), "{html}");
        assert!(!html.contains("/ghost/social/block/"), "{html}");
        assert!(!html.contains(r#"value="Like""#), "{html}");
    }

    /// Note bodies come from servers we do not control, so they are text in
    /// the page, never markup.
    #[test]
    fn remote_note_text_is_escaped() {
        let mut evil = note(false);
        evil.text = "<script>alert(1)</script>".into();
        evil.author = profile("https://remote.test/users/alice", Some("<b>Bad</b>"));
        let html = note_card(&evil, &HashMap::new(), "/ghost/social/");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(!html.contains("<b>Bad</b>"), "{html}");
    }

    /// An account that has not accepted yet says so, rather than appearing
    /// as a settled follow.
    #[test]
    fn a_pending_follow_is_marked_as_pending() {
        let account = ops::Account {
            profile: profile("https://remote.test/users/alice", None),
            accepted: false,
            delivery_error: None,
            since: "2026-10-30 09:00:00".into(),
        };
        let html = account_row(&account, ops::Relation::Following);
        assert!(html.contains("Awaiting acceptance"), "{html}");

        let settled = ops::Account { accepted: true, ..account };
        let html = account_row(&settled, ops::Relation::Following);
        assert!(!html.contains("Awaiting acceptance"), "{html}");
        assert!(html.contains("Unfollow"), "{html}");
    }

    /// Timestamps read as a feed near the top and as dates further down.
    #[test]
    fn times_are_relative_while_that_is_still_useful() {
        let now = chrono::Utc::now();
        let ago = |minutes: i64| {
            when(&(now - chrono::Duration::minutes(minutes))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string())
        };
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(5), "5m ago");
        assert_eq!(ago(120), "2h ago");
        assert_eq!(ago(60 * 24 * 3), "3d ago");
        // Past a week a relative figure stops meaning anything.
        assert!(ago(60 * 24 * 40).contains("20"), "{}", ago(60 * 24 * 40));
    }
}
