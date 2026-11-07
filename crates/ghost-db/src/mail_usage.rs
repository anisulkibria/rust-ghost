//! Persistent send-time reservations: concurrent sends share one monthly budget.
use crate::Db;
use chrono::{DateTime, Months, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use ghost_core::{GhostError, Result};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct Meter {
    pool: Db,
    host: Value,
}

pub struct Reservation {
    pool: Db,
    id: String,
    period: String,
    count: i64,
}

fn db(error: sqlx::Error) -> GhostError {
    GhostError::Db(error.to_string())
}
fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Monthly anniversaries, including clamped month ends and the original time of day.
pub fn period_bounds(anchor: &str, now: DateTime<Utc>) -> (String, String) {
    use chrono::Datelike;
    let anchor = DateTime::parse_from_rfc3339(anchor)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(anchor, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|d| d.and_utc())
        })
        .or_else(|| {
            NaiveDate::parse_from_str(anchor, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|d| d.and_utc())
        })
        .unwrap_or_else(|| {
            now.with_day(1)
                .unwrap()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
        });
    let mut months = ((now.year() - anchor.year()) * 12 + now.month() as i32
        - anchor.month() as i32)
        .max(0) as u32;
    let candidate = anchor
        .checked_add_months(Months::new(months))
        .unwrap_or(anchor);
    if candidate > now && months > 0 {
        months -= 1;
    }
    (
        timestamp(
            anchor
                .checked_add_months(Months::new(months))
                .unwrap_or(anchor),
        ),
        timestamp(
            anchor
                .checked_add_months(Months::new(months + 1))
                .unwrap_or(anchor),
        ),
    )
}

impl Meter {
    pub fn new(pool: Db, host: Value) -> Self {
        Self { pool, host }
    }
    fn bounds(&self) -> (String, String) {
        period_bounds(
            self.host
                .pointer("/subscription/start")
                .and_then(Value::as_str)
                .unwrap_or(""),
            Utc::now(),
        )
    }
    fn suspended(&self) -> bool {
        self.host
            .pointer("/limits/publicSiteAccess/disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn limit(&self) -> Option<i64> {
        if self.suspended() {
            return Some(0);
        }
        self.host
            .pointer("/limits/emails/maxPeriodic")
            .and_then(Value::as_i64)
            .map(|n| n.max(0))
    }
    pub fn enforced(&self) -> bool {
        self.limit().is_some()
            && (self.suspended()
                || self
                    .host
                    .pointer("/emailQuota/autoStop")
                    .and_then(Value::as_bool)
                    .unwrap_or(true))
    }

    pub async fn reserve(&self, kind: &str, provider: &str, count: i64) -> Result<Reservation> {
        if count <= 0 {
            return Err(GhostError::BadRequest("A send must have recipients".into()));
        }
        let (period, end) = self.bounds();
        let now = timestamp(Utc::now());
        let id = ghost_core::ids::new_id();
        let mut tx = self.pool.begin().await.map_err(db)?;
        // INSERT serializes initialization; UPDATE makes the ceiling atomic on both engines.
        sqlx::query(crate::dialect::sql("INSERT INTO mail_send_periods(period_start,sent,reserved) SELECT ?,CAST(COALESCE(SUM(CASE WHEN status='sent' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='reserved' THEN recipient_count ELSE 0 END),0) AS SIGNED) FROM mail_send_events WHERE created_at>=? AND created_at<? ON CONFLICT(period_start) DO NOTHING"))
            .bind(&period).bind(&period).bind(&end).execute(&mut *tx).await.map_err(db)?;
        let cap = self.limit().unwrap_or(i64::MAX);
        let changed = sqlx::query("UPDATE mail_send_periods SET reserved=reserved+? WHERE period_start=? AND (?=0 OR sent+reserved<=?-?)")
            .bind(count).bind(&period).bind(if self.enforced() {1i64}else{0}).bind(cap).bind(count)
            .execute(&mut *tx).await.map_err(db)?.rows_affected();
        let status = if changed == 1 { "reserved" } else { "blocked" };
        sqlx::query("INSERT INTO mail_send_events(id,period_start,kind,provider,recipient_count,status,created_at) VALUES(?,?,?,?,?,?,?)")
            .bind(&id).bind(&period).bind(kind).bind(provider).bind(count).bind(status).bind(&now)
            .execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        if changed == 0 {
            return Err(GhostError::HostLimitError {
                message: format!("Monthly email allowance reached ({cap} emails). Sending is paused until {end}, or your host increases the allowance."),
                details: json!({"name":"emails","limit":cap,"period_end":end}),
                help: self.host.pointer("/billing/url").and_then(Value::as_str).map(str::to_string),
            });
        }
        Ok(Reservation {
            pool: self.pool.clone(),
            id,
            period,
            count,
        })
    }

    pub async fn report(&self) -> Result<Value> {
        let (period, end) = self.bounds();
        let row: (i64,i64,i64,i64,i64,i64) = sqlx::query_as("SELECT CAST(COALESCE(SUM(CASE WHEN status='sent' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='reserved' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='failed' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='blocked' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='sent' AND kind='newsletter' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='sent' AND kind='transactional' THEN recipient_count ELSE 0 END),0) AS SIGNED) FROM mail_send_events WHERE created_at>=? AND created_at<?")
            .bind(&period).bind(&end).fetch_one(&self.pool).await.map_err(db)?;
        let total: i64 = sqlx::query_scalar("SELECT CAST(COALESCE(SUM(recipient_count),0) AS SIGNED) FROM mail_send_events WHERE status='sent'").fetch_one(&self.pool).await.map_err(db)?;
        let daily: Vec<(String,i64,i64,i64)> = sqlx::query_as("SELECT substr(created_at,1,10),CAST(COALESCE(SUM(CASE WHEN status='sent' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='failed' THEN recipient_count ELSE 0 END),0) AS SIGNED),CAST(COALESCE(SUM(CASE WHEN status='blocked' THEN recipient_count ELSE 0 END),0) AS SIGNED) FROM mail_send_events WHERE created_at>=? AND created_at<? GROUP BY substr(created_at,1,10) ORDER BY substr(created_at,1,10) DESC LIMIT 31")
            .bind(&period).bind(&end).fetch_all(&self.pool).await.map_err(db)?;
        let providers: Vec<(String,i64)> = sqlx::query_as("SELECT provider,CAST(COALESCE(SUM(recipient_count),0) AS SIGNED) FROM mail_send_events WHERE status='sent' AND created_at>=? AND created_at<? GROUP BY provider ORDER BY provider")
            .bind(&period).bind(&end).fetch_all(&self.pool).await.map_err(db)?;
        Ok(
            json!({"period_start":period,"period_end":end,"limit":self.limit(),"auto_stop":self.enforced(),
            "sent":row.0,"pending":row.1,"failed":row.2,"blocked":row.3,"newsletter_sent":row.4,"transactional_sent":row.5,
            "total_sent":total,"remaining":self.limit().map(|n|(n-row.0-row.1).max(0)),"stopped":self.enforced() && self.limit().is_some_and(|n|row.0+row.1>=n),
            "daily":daily.into_iter().map(|r|json!({"date":r.0,"sent":r.1,"failed":r.2,"blocked":r.3})).collect::<Vec<_>>(),
            "providers":providers.into_iter().map(|r|json!({"provider":r.0,"sent":r.1})).collect::<Vec<_>>(),
            "history_note":"Known historical newsletter sends are included; transactional history starts when volume tracking is enabled. Sent means accepted by the provider, not confirmed inbox delivery."}),
        )
    }
}

impl Reservation {
    pub async fn finish(self, accepted: bool) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let changed =
            sqlx::query("UPDATE mail_send_events SET status=? WHERE id=? AND status='reserved'")
                .bind(if accepted { "sent" } else { "failed" })
                .bind(&self.id)
                .execute(&mut *tx)
                .await
                .map_err(db)?
                .rows_affected();
        if changed == 1 {
            sqlx::query(
                "UPDATE mail_send_periods SET reserved=reserved-?,sent=sent+? WHERE period_start=?",
            )
            .bind(self.count)
            .bind(if accepted { self.count } else { 0 })
            .bind(&self.period)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture(limit: i64, stop: bool) -> (crate::TestDb, Meter) {
        let database = crate::pool::connect_test().await.unwrap();
        let host = json!({"subscription":{"start":"2020-01-31T10:00:00Z"},"limits":{"emails":{"maxPeriodic":limit}},"emailQuota":{"autoStop":stop}});
        let meter = Meter::new(database.pool(), host);
        (database, meter)
    }
    #[test]
    fn monthly_anniversaries_clamp_and_include_the_time_of_day() {
        let at = |date: &str| {
            DateTime::parse_from_rfc3339(date)
                .unwrap()
                .with_timezone(&Utc)
        };
        assert_eq!(
            period_bounds("2020-01-31T10:00:00Z", at("2026-02-28T10:00:00Z")),
            ("2026-02-28T10:00:00Z".into(), "2026-03-31T10:00:00Z".into())
        );
        assert_eq!(
            period_bounds("2020-01-31T10:00:00Z", at("2026-02-28T09:59:59Z")),
            ("2026-01-31T10:00:00Z".into(), "2026-02-28T10:00:00Z".into())
        );
        assert_eq!(
            period_bounds("2020-01-10T00:00:00Z", at("2026-11-12T00:00:00Z")),
            ("2026-11-10T00:00:00Z".into(), "2026-12-10T00:00:00Z".into())
        );
    }
    #[tokio::test]
    async fn accepted_sends_share_the_budget_and_failed_sends_release_it() {
        let (_database, meter) = fixture(2, true).await;
        meter
            .reserve("transactional", "ses", 1)
            .await
            .unwrap()
            .finish(true)
            .await
            .unwrap();
        meter
            .reserve("newsletter", "mailgun", 1)
            .await
            .unwrap()
            .finish(false)
            .await
            .unwrap();
        meter
            .reserve("newsletter", "mailgun", 1)
            .await
            .unwrap()
            .finish(true)
            .await
            .unwrap();
        assert!(meter.reserve("transactional", "smtp", 1).await.is_err());
        let report = meter.report().await.unwrap();
        assert_eq!(report["sent"], 2);
        assert_eq!(report["failed"], 1);
        assert_eq!(report["blocked"], 1);
        assert_eq!(report["remaining"], 0);
        assert_eq!(report["newsletter_sent"], 1);
        assert_eq!(report["transactional_sent"], 1);
        assert_eq!(report["stopped"], true);
        assert_eq!(report["pending"], 0);
    }
    #[tokio::test]
    async fn simultaneous_reservations_never_exceed_the_limit() {
        let (_database, meter) = fixture(3, true).await;
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let meter = meter.clone();
            tasks.spawn(async move {
                match meter.reserve("newsletter", "ses", 1).await {
                    Ok(reservation) => {
                        reservation.finish(true).await.unwrap();
                        true
                    }
                    Err(GhostError::HostLimitError { .. }) => false,
                    Err(error) => panic!("unexpected reservation error: {error}"),
                }
            });
        }
        let mut sent = 0;
        while let Some(result) = tasks.join_next().await {
            if result.unwrap() {
                sent += 1;
            }
        }
        assert_eq!(sent, 3);
        assert_eq!(meter.report().await.unwrap()["sent"], 3);
    }
    #[tokio::test]
    async fn disabled_enforcement_counts_overages_and_reenabling_blocks() {
        let (database, meter) = fixture(1, false).await;
        meter
            .reserve("newsletter", "ses", 2)
            .await
            .unwrap()
            .finish(true)
            .await
            .unwrap();
        let report = meter.report().await.unwrap();
        assert_eq!(report["sent"], 2);
        assert_eq!(report["stopped"], false);
        let mut host = meter.host.clone();
        host["emailQuota"]["autoStop"] = json!(true);
        assert!(Meter::new(database.pool(), host)
            .reserve("newsletter", "ses", 1)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn expired_periods_do_not_consume_this_month() {
        let (_database, meter) = fixture(1, true).await;
        sqlx::query("INSERT INTO mail_send_events(id,period_start,kind,provider,recipient_count,status,created_at) VALUES('old','old','newsletter','legacy',9999,'sent','2020-01-01T00:00:00Z')").execute(&meter.pool).await.unwrap();
        meter
            .reserve("transactional", "ses", 1)
            .await
            .unwrap()
            .finish(true)
            .await
            .unwrap();
        let report = meter.report().await.unwrap();
        assert_eq!(report["sent"], 1);
        assert_eq!(report["total_sent"], 10000);
    }
    #[tokio::test]
    async fn pending_reservations_survive_process_recreation() {
        let (_database, meter) = fixture(1, true).await;
        let reservation = meter.reserve("newsletter", "ses", 1).await.unwrap();
        let fresh = Meter::new(meter.pool.clone(), meter.host.clone());
        assert!(fresh.reserve("transactional", "ses", 1).await.is_err());
        assert_eq!(fresh.report().await.unwrap()["pending"], 1);
        reservation.finish(true).await.unwrap();
        assert_eq!(fresh.report().await.unwrap()["sent"], 1);
    }
    #[tokio::test]
    async fn suspended_sites_cannot_bypass_the_stop_with_an_overage_override() {
        let (_database, mut meter) = fixture(100, false).await;
        meter.host["limits"]["publicSiteAccess"] = json!({"disabled":true});
        assert!(meter.reserve("transactional", "ses", 1).await.is_err());
        assert_eq!(meter.report().await.unwrap()["limit"], 0);
    }
}
