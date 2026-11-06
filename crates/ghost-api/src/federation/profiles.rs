//! Remote actor profiles, cached so a list can name people at list speed.
//!
//! Every screen on the social web names accounts — the feed, the follower
//! lists, notifications, a thread. What there is to name them with lives on
//! their own server, behind an HTTPS fetch that takes as long as that server
//! takes. Rendering fifty rows cannot mean fifty fetches, so a profile is
//! stored the first time it is seen and read from the database after that.
//!
//! A row is never *required*: anything that has not been fetched yet falls
//! back to the actor URL, which is ugly but true. That is what keeps the
//! cache off the critical path of receiving an activity — a fetch that fails
//! must not make an inbox delivery fail with it.

use super::*;

/// How long a cached profile is considered current.
///
/// A display name and an avatar change, but not often, and a stale name is a
/// far smaller problem than hammering someone's server every time a list is
/// drawn. A day is the same order Mastodon uses for its own actor cache.
const FRESH_FOR_HOURS: i64 = 24;

/// What the admin shows about an account.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Profile {
    pub actor: String,
    pub handle: String,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub summary: Option<String>,
    pub url: Option<String>,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub posts: Option<i64>,
}

impl Profile {
    /// The name to show: their display name, else their handle.
    pub fn display(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.handle)
    }

    /// A profile we know nothing about beyond where it lives.
    ///
    /// The handle falls back to `@user@host` parsed out of the URL where the
    /// shape allows it, because `@alice@mastodon.social` is readable and
    /// `https://mastodon.social/users/alice` is not.
    pub fn unknown(actor: &str) -> Self {
        Profile {
            actor: actor.to_string(),
            handle: handle_from_url(actor),
            url: Some(actor.to_string()),
            ..Default::default()
        }
    }
}

/// Guess `@user@host` from an actor URL.
///
/// Most implementations put the username last (`/users/alice`,
/// `/@alice`, `/actor/alice`), so the last non-empty segment is a good guess
/// and a much better label than the whole URL. It is only ever a fallback:
/// once the actor document has been fetched, the real `preferredUsername`
/// replaces it.
fn handle_from_url(actor: &str) -> String {
    let Ok(url) = reqwest::Url::parse(actor) else {
        return actor.to_string();
    };
    let Some(host) = url.host_str() else {
        return actor.to_string();
    };
    let last = url
        .path_segments()
        .and_then(|segments| segments.filter(|s| !s.is_empty()).next_back())
        .unwrap_or("");
    let name = last.trim_start_matches('@');
    if name.is_empty() {
        return actor.to_string();
    }
    format!("@{name}@{host}")
}

/// Read cached profiles for a set of actors, in one query.
///
/// Actors with no cached row come back as `Profile::unknown`, so the caller
/// always gets one profile per actor and never has to handle a gap.
pub async fn lookup(pool: &ghost_db::Db, actors: &[String]) -> HashMap<String, Profile> {
    let mut out: HashMap<String, Profile> = actors
        .iter()
        .map(|actor| (actor.clone(), Profile::unknown(actor)))
        .collect();
    if actors.is_empty() {
        return out;
    }
    let placeholders = std::iter::repeat_n("?", actors.len()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT actor,handle,name,avatar,summary,url,followers,following,posts \
         FROM federation_actors WHERE actor IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ),
    >(&sql);
    for actor in actors {
        query = query.bind(actor);
    }
    for (actor, handle, name, avatar, summary, url, followers, following, posts) in
        query.fetch_all(pool).await.unwrap_or_default()
    {
        out.insert(
            actor.clone(),
            Profile { actor, handle, name, avatar, summary, url, followers, following, posts },
        );
    }
    out
}

/// One profile, cached or unknown.
pub async fn one(pool: &ghost_db::Db, actor: &str) -> Profile {
    lookup(pool, std::slice::from_ref(&actor.to_string()))
        .await
        .remove(actor)
        .unwrap_or_else(|| Profile::unknown(actor))
}

/// Store what an actor document said about itself.
pub async fn store(pool: &ghost_db::Db, actor: &str, profile: &network::Profile) {
    let _ = sqlx::query(
        ghost_db::dialect::sql("INSERT INTO federation_actors (actor,handle,name,avatar,summary,url,followers,following,posts,fetched_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(actor) DO UPDATE SET handle=excluded.handle,name=excluded.name,\
         avatar=excluded.avatar,summary=excluded.summary,url=excluded.url,\
         followers=excluded.followers,following=excluded.following,posts=excluded.posts,\
         fetched_at=excluded.fetched_at"),
    )
    .bind(actor)
    .bind(&profile.handle)
    .bind(profile.name.as_deref())
    .bind(profile.avatar.as_deref())
    .bind(profile.summary.as_deref())
    .bind(profile.url.as_deref())
    .bind(profile.followers)
    .bind(profile.following)
    .bind(profile.posts)
    .bind(dates::now_db())
    .execute(pool)
    .await;
}

/// Fetch and cache a profile, unless a current one is already held.
///
/// Returns the cached profile either way, so a caller that only wants
/// something to render never has to care whether a fetch happened.
/// Re-fetch one profile from its home server.
///
/// `as_who` is the publication's signing identity; without it an instance in
/// authorised-fetch mode refuses the read and the profile stays blank.
pub async fn refresh(
    pool: &ghost_db::Db,
    actor: &str,
    as_who: Option<&network::Signer>,
) -> Profile {
    let stale: bool = sqlx::query_scalar::<_, i64>(
        "SELECT NOT EXISTS(SELECT 1 FROM federation_actors WHERE actor=? \
         AND fetched_at > ?)",
    )
    .bind(actor)
    // The cutoff as an absolute instant: SQLite takes a modifier as a
    // parameter, MySQL cannot, and the comparison is portable this way.
    .bind(ghost_core::dates::to_db_string(
        &(chrono::Utc::now() - chrono::Duration::hours(FRESH_FOR_HOURS)),
    ))
    .fetch_one(pool)
    .await
    .unwrap_or(1) != 0;
    if stale {
        if let Ok(fetched) = network::actor_as(actor, as_who).await {
            store(pool, actor, &fetched.profile).await;
        }
    }
    one(pool, actor).await
}

/// Fill the cache for whichever of `actors` have no current row.
///
/// Bounded, and run after the page it serves has already been rendered from
/// whatever was cached: a feed must not take one remote fetch per unknown
/// author before it can be shown. The next load has the names.
pub async fn warm(
    pool: &ghost_db::Db,
    actors: Vec<String>,
    as_who: Option<network::Signer>,
) {
    const MAX_PER_PASS: usize = 8;
    let mut done = 0;
    for actor in actors {
        if done >= MAX_PER_PASS {
            break;
        }
        let known: bool = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM federation_actors WHERE actor=? \
             AND fetched_at > ?)",
        )
        .bind(&actor)
        // The cutoff as an absolute instant: SQLite takes a modifier as a
    // parameter, MySQL cannot, and the comparison is portable this way.
    .bind(ghost_core::dates::to_db_string(
        &(chrono::Utc::now() - chrono::Duration::hours(FRESH_FOR_HOURS)),
    ))
        .fetch_one(pool)
        .await
        .unwrap_or(1) != 0;
        if known {
            continue;
        }
        if let Ok(fetched) = network::actor_as(&actor, as_who.as_ref()).await {
            store(pool, &actor, &fetched.profile).await;
        }
        done += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An uncached actor still gets a readable label rather than a URL.
    #[test]
    fn an_unknown_actor_is_labelled_from_its_url() {
        for (url, expected) in [
            ("https://mastodon.social/users/alice", "@alice@mastodon.social"),
            ("https://example.test/@bob", "@bob@example.test"),
            ("https://example.test/actor/carol/", "@carol@example.test"),
        ] {
            assert_eq!(Profile::unknown(url).handle, expected, "{url}");
        }
        // Nothing to make a handle out of: the URL itself, unchanged.
        assert_eq!(Profile::unknown("https://example.test/").handle, "https://example.test/");
        assert_eq!(Profile::unknown("not a url").handle, "not a url");
    }

    /// The display name falls back through name, then handle — never to an
    /// empty string, which would render as a blank row.
    #[test]
    fn a_profile_always_has_something_to_display() {
        let mut p = Profile::unknown("https://example.test/users/dave");
        assert_eq!(p.display(), "@dave@example.test");
        p.name = Some("Dave".into());
        assert_eq!(p.display(), "Dave");
        // An empty name is not a name.
        p.name = Some(String::new());
        assert_eq!(p.display(), "@dave@example.test");
    }
}
