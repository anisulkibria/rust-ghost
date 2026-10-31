//! Brute-force protection for the authentication endpoints.
//!
//! Ported from Ghost's `express-brute` setup — `web/shared/middleware/brute.js`
//! and `api/spam-prevention.js` — with the budgets from `config/defaults.json`.
//! State lives in the `brute` table, which this port already carried in
//! upstream's shape and nothing used.
//!
//! The algorithm is express-brute's, and the shape of it matters:
//!
//! * `free_retries` attempts cost nothing.
//! * Each failure after that doubles the wait, starting at `min_wait` and
//!   stopping at `max_wait`. Five bad passwords buys a ten-minute lockout; a
//!   persistent attacker is quickly waiting a week.
//! * `lifetime` is how long the record survives without a new attempt, so a
//!   legitimate user who mistypes twice today is not penalised next month.
//!
//! Budgets are per **key**, and the key decides what is being protected. A
//! login key of `user_login` plus the caller's IP limits one network's
//! guessing; `<email>reset` limits requests against one account from
//! anywhere. Both are needed: neither alone stops both attack shapes.
use sqlx::SqlitePool;

/// One endpoint's budget. Waits are milliseconds, lifetime seconds, matching
/// upstream's config so the numbers can be compared directly.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub free_retries: i64,
    pub min_wait: i64,
    pub max_wait: i64,
    pub lifetime: i64,
}

/// `spam.user_login` — per IP, across every staff account.
pub const USER_LOGIN: Budget =
    Budget { free_retries: 4, min_wait: 600_000, max_wait: 604_800_000, lifetime: 3600 };

/// `spam.global_block` — per route per IP. Deliberately generous: a whole
/// office behind one address must not be locked out by one person's typo.
pub const GLOBAL_BLOCK: Budget =
    Budget { free_retries: 99, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.global_reset` — per route per IP, for password-reset requests.
pub const GLOBAL_RESET: Budget =
    Budget { free_retries: 4, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.user_reset` — per account, so rotating IPs cannot flood one inbox.
pub const USER_RESET: Budget =
    Budget { free_retries: 4, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.send_verification_code` — per IP.
pub const SEND_VERIFICATION_CODE: Budget =
    Budget { free_retries: 4, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.user_verification` — per IP, for submitting a code.
pub const USER_VERIFICATION: Budget =
    Budget { free_retries: 4, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.member_login` — per email address, ignoring IP.
pub const MEMBER_LOGIN: Budget =
    Budget { free_retries: 8, min_wait: 600_000, max_wait: 43_200_000, lifetime: 43_200 };

/// `spam.content_api_key` — per IP, reset on an authorized request so only
/// rejected keys count.
pub const CONTENT_API_KEY: Budget =
    Budget { free_retries: 99, min_wait: 3_600_000, max_wait: 86_400_000, lifetime: 3600 };

/// `spam.private_block` — the password gate on a private site, per IP.
pub const PRIVATE_BLOCK: Budget =
    Budget { free_retries: 99, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.email_preview_block` — ten test sends an hour, globally.
pub const EMAIL_PREVIEW_BLOCK: Budget =
    Budget { free_retries: 10, min_wait: 360_000, max_wait: 360_000, lifetime: 3600 };

/// `spam.support_request_block` — ten support requests an hour, globally.
pub const SUPPORT_REQUEST_BLOCK: Budget =
    Budget { free_retries: 10, min_wait: 3_600_000, max_wait: 3_600_000, lifetime: 3600 };

/// `spam.webmentions_block` — per IP.
pub const WEBMENTIONS_BLOCK: Budget =
    Budget { free_retries: 100, min_wait: 10, max_wait: 100, lifetime: 1000 };

/// How long a caller must wait, when they must wait at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blocked {
    /// Milliseconds until the next attempt is allowed.
    pub retry_in_ms: i64,
}

impl Blocked {
    /// Whole seconds for a `Retry-After` header, rounded up — rounding down
    /// would advertise a time that is still blocked.
    pub fn retry_after_secs(&self) -> i64 {
        self.retry_in_ms.div_euclid(1000) + i64::from(self.retry_in_ms.rem_euclid(1000) > 0)
    }

    /// Upstream's wording, which the admin surfaces verbatim.
    pub fn message(&self) -> String {
        format!("Too many attempts try again in {}.", humanise(self.retry_in_ms))
    }
}

/// Round a wait the way `moment().fromNow(true)` does for these messages.
fn humanise(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let minutes = seconds.div_euclid(60);
    let hours = minutes.div_euclid(60);
    let days = hours.div_euclid(24);
    if seconds < 45 {
        return "a few seconds".into();
    }
    if minutes < 2 {
        return "a minute".into();
    }
    if minutes < 45 {
        return format!("{minutes} minutes");
    }
    if hours < 2 {
        return "an hour".into();
    }
    if hours < 22 {
        return format!("{hours} hours");
    }
    if days < 2 {
        return "a day".into();
    }
    format!("{days} days")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// How long the caller must wait after `count` recorded attempts.
///
/// Zero while they are still inside the free allowance. Exposed for the tests,
/// which is where the doubling is easiest to read.
fn wait_for(budget: &Budget, count: i64) -> i64 {
    if count <= budget.free_retries {
        return 0;
    }
    let steps = (count - budget.free_retries - 1).min(62) as u32;
    budget.min_wait.saturating_mul(2i64.saturating_pow(steps)).min(budget.max_wait)
}

/// Count one attempt against `key`, and say whether it is allowed.
///
/// Call this *before* doing the work the attempt asks for. An attempt that
/// turns out to be legitimate should then call [`reset`], which is what keeps
/// a correct password from counting towards a lockout.
pub async fn check(pool: &SqlitePool, key: &str, budget: &Budget) -> Result<(), Blocked> {
    let now = now_ms();
    let existing = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>)>(
        "SELECT firstRequest, lastRequest, count FROM brute WHERE key = ?",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // A record nobody has touched for `lifetime` is gone: an honest user who
    // mistyped their password last month starts clean.
    let live = existing.filter(|(_, last, _)| {
        last.is_some_and(|last| now.saturating_sub(last) < budget.lifetime.saturating_mul(1000))
    });

    if let Some((_, Some(last), Some(count))) = live {
        let wait = wait_for(budget, count);
        if wait > 0 {
            let ready_at = last.saturating_add(wait);
            if now < ready_at {
                // Still serving a wait. Deliberately *not* counted again:
                // hammering the endpoint while blocked must not extend the
                // lockout forever, which would turn a short block permanent.
                return Err(Blocked { retry_in_ms: ready_at - now });
            }
        }
        let _ = sqlx::query("UPDATE brute SET count = count + 1, lastRequest = ? WHERE key = ?")
            .bind(now)
            .bind(key)
            .execute(pool)
            .await;
        return Ok(());
    }

    // No live record: start, or restart, the window.
    let _ = sqlx::query(
        "INSERT INTO brute (key, firstRequest, lastRequest, lifetime, count) VALUES (?, ?, ?, ?, 1) \
         ON CONFLICT(key) DO UPDATE SET firstRequest = ?, lastRequest = ?, lifetime = ?, count = 1",
    )
    .bind(key)
    .bind(now)
    .bind(now)
    .bind(budget.lifetime)
    .bind(now)
    .bind(now)
    .bind(budget.lifetime)
    .execute(pool)
    .await;
    Ok(())
}

/// Clear a key's record, after an attempt that turned out to be legitimate.
pub async fn reset(pool: &SqlitePool, key: &str) {
    let _ = sqlx::query("DELETE FROM brute WHERE key = ?").bind(key).execute(pool).await;
}

/// Is this key currently blocked? Reads only; counts nothing.
///
/// For a budget that should only charge for *failures* — the Content API key
/// being the one, where counting every request would put two writes on the
/// hot path of a public, cacheable API that a busy site hits constantly, and
/// a site serving its own front end would be paying them forever. Pair with
/// [`record_failure`].
///
/// `Ok(true)` means there is a record to clear if the request turns out to be
/// legitimate, so the caller can skip the delete when there is nothing there —
/// which is the overwhelmingly common case.
pub async fn peek(pool: &SqlitePool, key: &str, budget: &Budget) -> Result<bool, Blocked> {
    let now = now_ms();
    let row = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT lastRequest, count FROM brute WHERE key = ?",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((Some(last), Some(count))) = row else { return Ok(false) };
    if now.saturating_sub(last) >= budget.lifetime.saturating_mul(1000) {
        return Ok(true);
    }
    let wait = wait_for(budget, count);
    let ready_at = last.saturating_add(wait);
    if wait > 0 && now < ready_at {
        return Err(Blocked { retry_in_ms: ready_at - now });
    }
    Ok(true)
}

/// Charge one failure against `key`.
pub async fn record_failure(pool: &SqlitePool, key: &str, budget: &Budget) {
    let now = now_ms();
    let fresh: Option<Option<i64>> =
        sqlx::query_scalar("SELECT lastRequest FROM brute WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let live = fresh
        .flatten()
        .is_some_and(|last| now.saturating_sub(last) < budget.lifetime.saturating_mul(1000));
    if live {
        let _ = sqlx::query("UPDATE brute SET count = count + 1, lastRequest = ? WHERE key = ?")
            .bind(now)
            .bind(key)
            .execute(pool)
            .await;
        return;
    }
    let _ = sqlx::query(
        "INSERT INTO brute (key, firstRequest, lastRequest, lifetime, count) VALUES (?, ?, ?, ?, 1) \
         ON CONFLICT(key) DO UPDATE SET firstRequest = ?, lastRequest = ?, lifetime = ?, count = 1",
    )
    .bind(key)
    .bind(now)
    .bind(now)
    .bind(budget.lifetime)
    .bind(now)
    .bind(now)
    .bind(budget.lifetime)
    .execute(pool)
    .await;
}

/// Drop expired records, so the table does not grow without bound.
pub async fn sweep(pool: &SqlitePool) {
    let _ = sqlx::query(
        "DELETE FROM brute WHERE lastRequest IS NOT NULL AND lifetime IS NOT NULL \
           AND lastRequest + (lifetime * 1000) < ?",
    )
    .bind(now_ms())
    .execute(pool)
    .await;
}

/// The caller's address, for keys that are scoped per IP.
///
/// Reads the proxy headers first, because Ghost runs with Express'
/// `trust proxy` on and this port is normally behind one too; the socket
/// address of a reverse proxy is the same for every visitor, which would make
/// one budget shared by the whole internet. Only the left-most entry of
/// `X-Forwarded-For` is used, as that is the original client.
pub fn client_ip(headers: &axum::http::HeaderMap, socket: Option<std::net::IpAddr>) -> String {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(forwarded) = header("x-forwarded-for") {
        if let Some(first) = forwarded.split(',').next().map(str::trim).filter(|s| !s.is_empty()) {
            return first.to_string();
        }
    }
    if let Some(real) = header("x-real-ip").filter(|s| !s.is_empty()) {
        return real.to_string();
    }
    socket.map(|ip| ip.to_string()).unwrap_or_else(|| "unknown".into())
}

/// The caller's address as an extractor.
///
/// `ConnectInfo` itself is not optional in axum 0.8, and these handlers have
/// to keep working when there is no socket behind the request — unit tests
/// and the in-process routers call them directly. This reads the same
/// extension the server inserts and shrugs when it is absent.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientAddr(pub Option<std::net::IpAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(ClientAddr(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|info| info.0.ip()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE brute (key TEXT PRIMARY KEY NOT NULL, firstRequest BIGINT, \
             lastRequest BIGINT, lifetime BIGINT, count INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn the_free_allowance_costs_nothing() {
        for count in 1..=USER_LOGIN.free_retries {
            assert_eq!(wait_for(&USER_LOGIN, count), 0, "attempt {count} is free");
        }
    }

    #[test]
    fn each_failure_past_the_allowance_doubles_the_wait() {
        // Ghost's login budget: four free tries, then ten minutes, twenty, …
        assert_eq!(wait_for(&USER_LOGIN, 5), 600_000);
        assert_eq!(wait_for(&USER_LOGIN, 6), 1_200_000);
        assert_eq!(wait_for(&USER_LOGIN, 7), 2_400_000);
    }

    #[test]
    fn the_wait_stops_doubling_at_the_maximum() {
        // A week, and no further, however long the attack runs.
        assert_eq!(wait_for(&USER_LOGIN, 40), USER_LOGIN.max_wait);
        // and the exponent cannot overflow on an absurd count
        assert_eq!(wait_for(&USER_LOGIN, i64::MAX), USER_LOGIN.max_wait);
    }

    #[tokio::test]
    async fn five_bad_logins_buy_a_lockout() {
        let pool = pool().await;
        // Four free attempts.
        for attempt in 1..=4 {
            assert!(check(&pool, "user_login:1.2.3.4", &USER_LOGIN).await.is_ok(), "attempt {attempt}");
        }
        // The fifth is recorded and still allowed — the budget is spent *after*
        // it, which is why the sixth is the one refused.
        assert!(check(&pool, "user_login:1.2.3.4", &USER_LOGIN).await.is_ok());
        let blocked = check(&pool, "user_login:1.2.3.4", &USER_LOGIN).await.expect_err("locked out");
        assert!(blocked.retry_in_ms > 0);
        assert!(blocked.retry_in_ms <= USER_LOGIN.min_wait);
    }

    #[tokio::test]
    async fn a_successful_attempt_clears_the_count() {
        let pool = pool().await;
        for _ in 0..5 {
            let _ = check(&pool, "user_login:1.2.3.4", &USER_LOGIN).await;
        }
        // Signing in correctly must not leave the next person on that address
        // one typo away from an hour's wait.
        reset(&pool, "user_login:1.2.3.4").await;
        assert!(check(&pool, "user_login:1.2.3.4", &USER_LOGIN).await.is_ok());
    }

    #[tokio::test]
    async fn keys_are_counted_separately() {
        let pool = pool().await;
        for _ in 0..6 {
            let _ = check(&pool, "user_login:1.1.1.1", &USER_LOGIN).await;
        }
        assert!(check(&pool, "user_login:1.1.1.1", &USER_LOGIN).await.is_err());
        // Another address has its own budget.
        assert!(check(&pool, "user_login:2.2.2.2", &USER_LOGIN).await.is_ok());
    }

    #[tokio::test]
    async fn an_expired_window_starts_clean() {
        let pool = pool().await;
        for _ in 0..6 {
            let _ = check(&pool, "stale", &USER_LOGIN).await;
        }
        assert!(check(&pool, "stale", &USER_LOGIN).await.is_err());
        // Age the record past its lifetime.
        let long_ago = now_ms() - (USER_LOGIN.lifetime * 1000) - 1;
        sqlx::query("UPDATE brute SET lastRequest = ? WHERE key = 'stale'")
            .bind(long_ago)
            .execute(&pool)
            .await
            .unwrap();
        assert!(check(&pool, "stale", &USER_LOGIN).await.is_ok(), "the window had closed");
    }

    #[tokio::test]
    async fn requests_while_blocked_do_not_extend_the_block() {
        let pool = pool().await;
        for _ in 0..6 {
            let _ = check(&pool, "hammered", &USER_LOGIN).await;
        }
        let first = check(&pool, "hammered", &USER_LOGIN).await.expect_err("blocked");
        for _ in 0..20 {
            let _ = check(&pool, "hammered", &USER_LOGIN).await;
        }
        let later = check(&pool, "hammered", &USER_LOGIN).await.expect_err("still blocked");
        assert!(
            later.retry_in_ms <= first.retry_in_ms,
            "a blocked caller must not be able to lock themselves out for longer"
        );
    }

    #[tokio::test]
    async fn sweeping_drops_only_expired_records() {
        let pool = pool().await;
        let _ = check(&pool, "fresh", &USER_LOGIN).await;
        let _ = check(&pool, "old", &USER_LOGIN).await;
        sqlx::query("UPDATE brute SET lastRequest = ? WHERE key = 'old'")
            .bind(now_ms() - (USER_LOGIN.lifetime * 1000) - 1)
            .execute(&pool)
            .await
            .unwrap();
        sweep(&pool).await;
        let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM brute ORDER BY key")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(keys, vec!["fresh".to_string()]);
    }

    #[test]
    fn retry_after_rounds_up_so_it_is_never_still_blocked() {
        assert_eq!(Blocked { retry_in_ms: 1 }.retry_after_secs(), 1);
        assert_eq!(Blocked { retry_in_ms: 1000 }.retry_after_secs(), 1);
        assert_eq!(Blocked { retry_in_ms: 1001 }.retry_after_secs(), 2);
    }

    #[test]
    fn the_wait_reads_the_way_ghost_words_it() {
        assert_eq!(humanise(10_000), "a few seconds");
        assert_eq!(humanise(60_000), "a minute");
        assert_eq!(humanise(600_000), "10 minutes");
        assert_eq!(humanise(3_600_000), "an hour");
        assert_eq!(humanise(10 * 3_600_000), "10 hours");
        assert_eq!(humanise(604_800_000), "7 days");
        assert!(Blocked { retry_in_ms: 600_000 }.message().contains("10 minutes"));
    }

    #[test]
    fn the_forwarded_client_is_preferred_over_the_proxy_socket() {
        use axum::http::HeaderMap;
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7, 10.0.0.1".parse().unwrap());
        let socket = Some("10.0.0.1".parse().unwrap());
        // Keying on the proxy's own address would give the whole internet one
        // shared login budget.
        assert_eq!(client_ip(&headers, socket), "203.0.113.7");
    }

    #[test]
    fn the_socket_address_is_the_fallback() {
        use axum::http::HeaderMap;
        let socket = Some("198.51.100.4".parse().unwrap());
        assert_eq!(client_ip(&HeaderMap::new(), socket), "198.51.100.4");
        assert_eq!(client_ip(&HeaderMap::new(), None), "unknown");
    }

    #[test]
    fn an_empty_forwarded_header_falls_through() {
        use axum::http::HeaderMap;
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        let socket = Some("198.51.100.4".parse().unwrap());
        assert_eq!(client_ip(&headers, socket), "198.51.100.4");
    }

    #[test]
    fn the_budgets_match_ghosts_configured_defaults() {
        // Transcribed from `core/shared/config/defaults.json`; a drift here is
        // a drift in policy, so it is asserted rather than trusted.
        assert_eq!((USER_LOGIN.free_retries, USER_LOGIN.min_wait, USER_LOGIN.max_wait), (4, 600_000, 604_800_000));
        assert_eq!((USER_RESET.free_retries, USER_RESET.lifetime), (4, 3600));
        assert_eq!((GLOBAL_BLOCK.free_retries, GLOBAL_BLOCK.min_wait), (99, 3_600_000));
        assert_eq!((MEMBER_LOGIN.free_retries, MEMBER_LOGIN.lifetime), (8, 43_200));
        assert_eq!((EMAIL_PREVIEW_BLOCK.free_retries, EMAIL_PREVIEW_BLOCK.lifetime), (10, 3600));
        assert_eq!((CONTENT_API_KEY.free_retries, CONTENT_API_KEY.max_wait), (99, 86_400_000));
    }

    #[tokio::test]
    async fn peek_does_not_charge_for_a_request() {
        let pool = pool().await;
        // The Content API's path: reading must not write, however many times
        // a front end asks.
        for _ in 0..200 {
            assert_eq!(
                peek(&pool, "content_api_key:1.2.3.4", &CONTENT_API_KEY).await,
                Ok(false),
                "nothing to clear, so the caller skips the delete too"
            );
        }
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM brute").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0, "a successful request leaves no record behind");
    }

    #[tokio::test]
    async fn failures_accumulate_until_the_budget_is_spent() {
        let pool = pool().await;
        let key = "content_api_key:1.2.3.4";
        for _ in 0..=CONTENT_API_KEY.free_retries {
            record_failure(&pool, key, &CONTENT_API_KEY).await;
        }
        assert!(peek(&pool, key, &CONTENT_API_KEY).await.is_err(), "a rejected key eventually blocks");
    }

    #[tokio::test]
    async fn a_success_after_failures_clears_the_record() {
        let pool = pool().await;
        let key = "content_api_key:1.2.3.4";
        for _ in 0..=CONTENT_API_KEY.free_retries {
            record_failure(&pool, key, &CONTENT_API_KEY).await;
        }
        reset(&pool, key).await;
        assert!(peek(&pool, key, &CONTENT_API_KEY).await.is_ok());
    }

    #[tokio::test]
    async fn peek_forgets_an_expired_window() {
        let pool = pool().await;
        let key = "expired";
        for _ in 0..=CONTENT_API_KEY.free_retries {
            record_failure(&pool, key, &CONTENT_API_KEY).await;
        }
        assert!(peek(&pool, key, &CONTENT_API_KEY).await.is_err());
        sqlx::query("UPDATE brute SET lastRequest = ? WHERE key = ?")
            .bind(now_ms() - (CONTENT_API_KEY.lifetime * 1000) - 1)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
        assert!(peek(&pool, key, &CONTENT_API_KEY).await.is_ok());
    }
}
