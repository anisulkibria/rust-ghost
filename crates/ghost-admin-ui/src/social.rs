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

use crate::*;
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

// -------------------------------------------------------------------------
// The social web screens: the feed, threads, accounts and the notes composer.
// -------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// social web — Ghost's Fediverse/ActivityPub admin
// ---------------------------------------------------------------------------


/// Back to a social web section, carrying a one-shot banner.
pub(crate) fn social_flash(tab: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair("tab", tab)
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/social/?{encoded}")).into_response()
}

/// Back to wherever the form was submitted from.
///
/// An action taken from a thread should land back in that thread, not at the
/// top of the feed — losing your place is the quickest way to make a feed
/// unusable.
pub(crate) fn social_return(form: &HashMap<String, String>, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    match form.get("return").map(String::as_str).filter(|r| r.starts_with("/ghost/social/")) {
        Some(back) => {
            let join = if back.contains('?') { '&' } else { '?' };
            Redirect::to(&format!("{back}{join}{encoded}")).into_response()
        }
        None => Redirect::to(&format!("/ghost/social/?{encoded}")).into_response(),
    }
}

/// The publication's handle and actor URL, for the identity card.
pub(crate) async fn social_identity(state: &AppState) -> (String, String) {
    let site = reqwest::Url::parse(&state.site_url()).ok();
    let host = site
        .as_ref()
        .and_then(|url| match url.port() {
            Some(port) => url.host_str().map(|h| format!("{h}:{port}")),
            None => url.host_str().map(str::to_string),
        })
        .unwrap_or_default();
    (
        format!("@publication@{host}"),
        format!("{}/.ghost/activitypub/actor", state.site_url()),
    )
}

/// The shell every social screen shares: the identity card and the tab bar.
pub(crate) async fn social_shell(state: &AppState, tab: &str, body: &str, flash: &str) -> String {
    let (handle, actor) = social_identity(state).await;
    let (followers, following, posts) = ops::summary(&state.db).await;
    let unread = ops::unread(&state.db).await;
    let enabled = ghost_api::federation::enabled(state);

    let tabs: String = social::TABS
        .iter()
        .map(|(slug, label)| {
            // The unread count rides the tab it belongs to, which is the one
            // place it is useful without being nagging.
            let badge = if *slug == "notifications" && unread > 0 {
                format!(r#"<span class="seg-count">{unread}</span>"#)
            } else {
                String::new()
            };
            format!(
                r#"<a class="seg{on}" href="/ghost/social/?tab={slug}">{label}{badge}</a>"#,
                on = if *slug == tab { " on" } else { "" },
                slug = slug,
                label = esc(label),
                badge = badge,
            )
        })
        .collect();

    // Federation needs a public HTTPS address to be reachable at all. Saying
    // so once, at the top, beats every action failing with its own message.
    let notice = if !enabled {
        format!(
            r#"<div class="banner info rise">{icon}<span class="msg">The social web is off. Your publication is not federating, and nothing here will be delivered until it is enabled.</span></div>"#,
            icon = svg("ghost"),
        )
    } else if !state.site_url().starts_with("https://") {
        format!(
            r#"<div class="banner info rise">{icon}<span class="msg">This site's URL is not HTTPS, so other servers cannot reach it. Notes and follows are recorded here but will not be delivered.</span></div>"#,
            icon = svg("ghost"),
        )
    } else {
        String::new()
    };

    format!(
        r#"<header class="page-head rise"><div><h1>Social web</h1><p class="sub">Your publication on the fediverse</p></div></header>
{flash}{notice}
{profile}
<div class="seg-bar">{tabs}</div>
{body}"#,
        flash = flash,
        notice = notice,
        profile = social::publication_header(&handle, &actor, followers, following, posts, enabled),
        tabs = tabs,
        body = body,
    )
}

pub(crate) async fn social_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Editor).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let tab = q.get("tab").map(String::as_str).unwrap_or("feed");
    let tab = social::TABS
        .iter()
        .find(|(slug, _)| *slug == tab)
        .map(|(slug, _)| *slug)
        .unwrap_or("feed");
    let page = q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).clamp(1, 10000);
    const PER_PAGE: i64 = 20;

    let body = match tab {
        "notes" | "feed" => {
            let feed = if tab == "notes" { ops::Feed::Mine } else { ops::Feed::Reader };
            let notes = ops::notes(&state.db, feed, None, page, PER_PAGE).await;
            let own = ops::own_interactions(&state.db).await;
            // Where this page's forms come back to, so an action taken on
            // page three of the feed does not land on page one.
            let feed_return = format!("/ghost/social/?tab={tab}&page={page}");
            // Names for whoever we have not met yet, after this page is
            // built: a feed must not wait on a remote fetch per author.
            social_warm_profiles(&state, notes.iter().map(|n| n.author.actor.clone()).collect());
            let cards: String = notes
                .iter()
                .map(|note| social::note_card(note, &own, &feed_return))
                .collect();
            let list = if notes.is_empty() {
                if tab == "notes" {
                    social::empty("No notes yet", "Anything you publish here goes to every follower.")
                } else {
                    social::empty(
                        "Nothing in your feed yet",
                        "Follow some accounts and their notes will appear here.",
                    )
                }
            } else {
                cards
            };
            format!(
                "{composer}{list}{pager}",
                composer = social::composer(None),
                list = list,
                pager = social_pager(tab, page, notes.len() as i64, PER_PAGE),
            )
        }
        "explore" => {
            let (_, actor_url) = social_identity(&state).await;
            let found = ops::suggestions(&state.db, &actor_url, 24).await;
            social_warm_profiles(&state, found.iter().map(|s| s.profile.actor.clone()).collect());
            let cards: String = found.iter().map(social::suggestion_card).collect();
            let hosts = ops::recommended_hosts(&state.db).await;
            format!(
                r#"{search}
{suggestions}
{recommended}"#,
                // Explore is where you go looking, so the lookup box belongs
                // here as well as on Following.
                search = {
                    let query = q.get("q").map(String::as_str).unwrap_or("");
                    if query.trim().is_empty() {
                        social::search_box("", None, None)
                    } else {
                        match ops::preview(&state, query).await {
                            Ok(profile) => social::search_box(query, Some(&profile), None),
                            Err(e) => social::search_box(query, None, Some(&e.message())),
                        }
                    }
                },
                suggestions = if found.is_empty() {
                    social::empty(
                        "No suggestions yet",
                        "Accounts that follow you, or whose notes reach your feed, are suggested here.",
                    )
                } else {
                    format!(
                        r#"<div class="section">Suggested for you</div><div class="sw-grid">{cards}</div>"#
                    )
                },
                recommended = social::recommended_publications(&hosts),
            )
        }
        "notifications" => {
            let kind = q.get("kind").map(String::as_str).unwrap_or("all");
            let kind = social::NOTIFICATION_KINDS
                .iter()
                .find(|(slug, _, _)| *slug == kind)
                .copied()
                .unwrap_or(social::NOTIFICATION_KINDS[0]);
            let items = ops::notifications(&state.db, kind.2, 100).await;
            social_warm_profiles(&state, items.iter().map(|n| n.actor.actor.clone()).collect());
            let filters: String = social::NOTIFICATION_KINDS
                .iter()
                .map(|(slug, label, _)| {
                    format!(
                        r#"<a class="seg{on}" href="/ghost/social/?tab=notifications&kind={slug}">{label}</a>"#,
                        on = if *slug == kind.0 { " on" } else { "" },
                        slug = slug,
                        label = esc(label),
                    )
                })
                .collect();
            let rows: String = items.iter().map(social::notification_row).collect();
            let unread = ops::unread(&state.db).await;
            format!(
                r#"<div class="seg-bar sub">{filters}</div>
{mark}{list}"#,
                filters = filters,
                mark = if unread > 0 {
                    format!(
                        r#"<form method="post" action="/ghost/social/read/" class="sw-markread"><button class="btn tonal small" type="submit">Mark all as read</button></form>"#
                    )
                } else {
                    String::new()
                },
                list = if items.is_empty() {
                    social::empty(
                        "Nothing here yet",
                        "Follows, replies, likes and boosts from other servers show up here.",
                    )
                } else {
                    format!(r#"<div class="card rows rise">{rows}</div>"#)
                },
            )
        }
        "following" | "followers" | "blocked" => {
            let relation = match tab {
                "followers" => ops::Relation::Followers,
                "blocked" => ops::Relation::Blocked,
                _ => ops::Relation::Following,
            };
            let accounts = ops::accounts(&state.db, relation, page, PER_PAGE).await;
            social_warm_profiles(
                &state,
                accounts.iter().map(|a| a.profile.actor.clone()).collect(),
            );
            let rows: String = accounts
                .iter()
                .map(|account| social::account_row(account, relation))
                .collect();
            // Account search belongs with Following: it is the screen you are
            // on when you want another one.
            let search = if tab == "following" {
                let query = q.get("q").map(String::as_str).unwrap_or("");
                if query.trim().is_empty() {
                    social::search_box("", None, None)
                } else {
                    match ops::preview(&state, query).await {
                        Ok(found) => social::search_box(query, Some(&found), None),
                        Err(e) => social::search_box(query, None, Some(&e.message())),
                    }
                }
            } else {
                String::new()
            };
            let (empty_title, empty_detail) = match tab {
                "followers" => ("No followers yet", "Accounts that follow your publication appear here."),
                "blocked" => ("Nobody is blocked", "Blocking an account drops the follow in both directions."),
                _ => ("Not following anyone yet", "Look an account up above to follow it."),
            };
            format!(
                "{search}{list}{pager}",
                search = search,
                list = if accounts.is_empty() {
                    social::empty(empty_title, empty_detail)
                } else {
                    format!(r#"<div class="card rows rise">{rows}</div>"#)
                },
                pager = social_pager(tab, page, accounts.len() as i64, PER_PAGE),
            )
        }
        _ => String::new(),
    };

    page_for_role(
        actor.role,
        "Social web",
        "modules",
        Some(&actor.user),
        &social_shell(&state, tab, &body, &flash_banner(&q)).await,
        true,
    )
    .into_response()
}

/// Previous/next, shown only where there is somewhere to go.
pub(crate) fn social_pager(tab: &str, page: i64, shown: i64, per_page: i64) -> String {
    let mut links = String::new();
    if page > 1 {
        links.push_str(&format!(
            r#"<a class="btn tonal small" href="/ghost/social/?tab={tab}&page={}">Previous</a>"#,
            page - 1
        ));
    }
    if shown == per_page {
        links.push_str(&format!(
            r#"<a class="btn tonal small" href="/ghost/social/?tab={tab}&page={}">Next</a>"#,
            page + 1
        ));
    }
    if links.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="sw-pager">{links}</div>"#)
    }
}

/// Fetch missing profiles in the background.
///
/// Detached on purpose: the page this serves has already been rendered from
/// whatever was cached, so a slow or dead remote server delays nothing. The
/// names appear on the next load.
pub(crate) fn social_warm_profiles(state: &AppState, actors: Vec<String>) {
    let db = state.db.clone();
    // Resolved before detaching, so the background fetch carries the
    // publication's identity and reaches authorised-fetch instances.
    let state = state.clone();
    tokio::spawn(async move {
        let signer = ghost_api::federation::signer_for(&state).await;
        ghost_api::federation::profiles::warm(&db, actors, signer).await;
    });
}

/// One note and the conversation under it.
pub(crate) async fn social_thread_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Editor).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let id = q.get("id").map(String::as_str).unwrap_or("");
    let Some(note) = ops::note(&state.db, id).await else {
        return social_flash("feed", "error", "That note is no longer here.");
    };
    let own = ops::own_interactions(&state.db).await;
    let thread_return = format!("/ghost/social/thread/?id={}", url_value(&note.id));
    let replies = ops::notes(&state.db, ops::Feed::Replies, Some(&note.id), 1, 100).await;

    // The note it answers, when it answers one, so a thread opened from the
    // middle still has its top.
    let parent = match note.in_reply_to.as_deref() {
        Some(parent) => match ops::note(&state.db, parent).await {
            Some(parent) => format!(
                r#"<div class="section">In reply to</div>{}"#,
                social::note_card(&parent, &own, &thread_return)
            ),
            None => String::new(),
        },
        None => String::new(),
    };

    let reply_cards: String = replies
        .iter()
        .map(|reply| social::note_card(reply, &own, &thread_return))
        .collect();

    let body = format!(
        r#"<p class="sw-back"><a href="/ghost/social/">{icon} Back to the feed</a></p>
{parent}
<div class="section">Note</div>
{note}
<div class="section">{count}</div>
{composer}
{replies}"#,
        icon = svg("chevron"),
        parent = parent,
        note = social::note_card(&note, &own, &thread_return),
        count = if replies.is_empty() {
            "No replies yet".to_string()
        } else if replies.len() == 1 {
            "1 reply".to_string()
        } else {
            format!("{} replies", replies.len())
        },
        composer = social::composer(Some(&note)),
        replies = reply_cards,
    );

    page_for_role(
        actor.role,
        "Social web",
        "modules",
        Some(&actor.user),
        &format!("{}{}", flash_banner(&q), body),
        true,
    )
    .into_response()
}

/// A remote account's page: who they are, and what we hold of theirs.
pub(crate) async fn social_account_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let user = match require_role(&state, &headers, Role::Editor).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let target = q.get("actor").map(String::as_str).unwrap_or("");
    if target.is_empty() {
        return social_flash("feed", "error", "No account was named.");
    }
    // One deliberate fetch, because this page *is* the profile — unlike a
    // list, where it would be fifty.
    let signer = ghost_api::federation::signer_for(&state).await;
    let profile =
        ghost_api::federation::profiles::refresh(&state.db, target, signer.as_ref()).await;

    let following: bool =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM federation_following WHERE actor=?)")
            .bind(target)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0) != 0;
    let blocked: bool =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM federation_blocks WHERE actor=?)")
            .bind(target)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0) != 0;

    let notes = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM federation_objects WHERE actor=? AND deleted=0 \
         ORDER BY published_at DESC LIMIT 20",
    )
    .bind(target)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let own = ops::own_interactions(&state.db).await;
    let account_return = format!("/ghost/social/account/?actor={}", url_value(target));
    let mut cards = String::new();
    for (id,) in &notes {
        if let Some(note) = ops::note(&state.db, id).await {
            cards.push_str(&social::note_card(&note, &own, &account_return));
        }
    }

    page_for_role(
        user.role,
        "Social web",
        "modules",
        Some(&user.user),
        &format!(
            r#"{flash}<p class="sw-back"><a href="/ghost/social/">{icon} Back to the feed</a></p>
{card}
<div class="section">Notes we have seen</div>
{notes}"#,
            flash = flash_banner(&q),
            icon = svg("chevron"),
            card = social::account_card(&profile, following, blocked),
            notes = if cards.is_empty() {
                social::empty(
                    "Nothing from this account yet",
                    "Notes appear here once they reach this server.",
                )
            } else {
                cards
            },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn social_note_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    let reply = form.get("in_reply_to").map(String::as_str).filter(|r| !r.is_empty());
    let result = ops::publish_note(
        &state,
        form.get("text").map(String::as_str).unwrap_or(""),
        reply,
        form.get("reply_actor").map(String::as_str),
    )
    .await;
    // A reply belongs back in its thread; a note belongs in your notes.
    match (result, reply) {
        (Ok(_), Some(parent)) => {
            let encoded: String = form_urlencoded::Serializer::new(String::new())
                .append_pair("id", parent)
                .append_pair("ok", "Reply published.")
                .finish();
            Redirect::to(&format!("/ghost/social/thread/?{encoded}")).into_response()
        }
        (Ok(_), None) => social_flash("notes", "ok", "Note published."),
        (Err(e), Some(parent)) => {
            let encoded: String = form_urlencoded::Serializer::new(String::new())
                .append_pair("id", parent)
                .append_pair("error", &e.message())
                .finish();
            Redirect::to(&format!("/ghost/social/thread/?{encoded}")).into_response()
        }
        (Err(e), None) => social_flash("notes", "error", &e.message()),
    }
}

pub(crate) async fn social_note_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    // The screens carry the full object id; only its last segment is ours to
    // act on, and rebuilding the URL from it is what stops a crafted id
    // naming something on another server.
    let id = form.get("id").map(String::as_str).unwrap_or("");
    let uuid = id.rsplit('/').next().unwrap_or("");
    match ops::delete_note(&state, uuid).await {
        Ok(()) => social_return(&form, "ok", "Note deleted."),
        Err(e) => social_return(&form, "error", &e.message()),
    }
}

pub(crate) async fn social_follow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    let target = form.get("actor").map(String::as_str).unwrap_or("");
    if form.get("unfollow").is_some() {
        return match ops::unfollow(&state, target).await {
            Ok(()) => social_return(&form, "ok", "Unfollowed."),
            Err(e) => social_return(&form, "error", &e.message()),
        };
    }
    match ops::follow(&state, target).await {
        Ok(profile) => social_flash(
            "following",
            "ok",
            &format!("Follow request sent to {}.", profile.handle),
        ),
        Err(e) => social_flash("following", "error", &e.message()),
    }
}

pub(crate) async fn social_interact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    let kind = form.get("type").map(String::as_str).unwrap_or("");
    let result = ops::interact(
        &state,
        kind,
        form.get("actor").map(String::as_str).unwrap_or(""),
        form.get("object").map(String::as_str).unwrap_or(""),
    )
    .await;
    match result {
        Ok(_) => social_return(
            &form,
            "ok",
            match kind {
                "Like" => "Liked.",
                "Announce" => "Boosted.",
                _ => "Withdrawn.",
            },
        ),
        Err(e) => social_return(&form, "error", &e.message()),
    }
}

pub(crate) async fn social_block(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let unblock = form.get("unblock").is_some();
    match ops::set_block(
        &state,
        form.get("actor").map(String::as_str).unwrap_or(""),
        !unblock,
    )
    .await
    {
        Ok(()) => social_return(
            &form,
            "ok",
            if unblock { "Account unblocked." } else { "Account blocked." },
        ),
        Err(e) => social_return(&form, "error", &e.message()),
    }
}

pub(crate) async fn social_set_enabled(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let enable = form.get("enabled").map(String::as_str) == Some("1");
    if enable && state.settings.get_bool("is_private") {
        return social_flash(
            "feed",
            "error",
            "A private publication cannot federate. Turn off private mode first.",
        );
    }
    if let Err(e) = state
        .settings
        .set(&state.db, "social_web_enabled", &serde_json::json!(enable))
        .await
    {
        return social_flash("feed", "error", &e.message());
    }
    social_flash(
        "feed",
        "ok",
        if enable { "The social web is on." } else { "The social web is off." },
    )
}

pub(crate) async fn social_mark_read(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    let _ = ops::mark_read(&state).await;
    social_flash("notifications", "ok", "Notifications marked as read.")
}
