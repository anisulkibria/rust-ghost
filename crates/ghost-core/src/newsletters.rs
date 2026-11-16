//! The Newsletters module's rules: the arithmetic of a sending programme, and
//! the judgement about what is wrong with one.
//!
//! Like [`crate::memberships`], the Newsletters module is an **organiser**.
//! Newsletters, their designs, the transport that carries them and the record
//! of every send already exist; what did not exist was anywhere that read
//! them together and said whether, taken together, a post written today would
//! actually reach anybody.
//!
//! That judgement lives here for the usual two reasons: it can be wrong, and
//! it has to be testable without a database, a browser or an SMTP server. And
//! a sending programme is broken in ways no single screen can see. Settings →
//! Newsletters knows whether sending is switched on; Settings → Email knows
//! whether a relay is configured; Settings → Analytics knows whether opens are
//! tracked; the `emails` table knows that the last three sends failed. Each of
//! those screens can be individually correct while a post goes nowhere.
//!
//! ## This is not a trust boundary
//!
//! Nothing here parses anything a visitor supplied. [`Programme`] is built
//! from counts and from settings an administrator wrote. What this module owns
//! is the *order* problems are reported in and the *thresholds* at which one
//! is worth reporting — the same argument [`crate::dashboard`] makes for its
//! rankings.
//!
//! ## What the arithmetic is careful about
//!
//! Every rate is `Option<f64>` and is **never** computed when the denominator
//! is zero: a programme that has sent nothing has no open rate, and `0%`
//! would say it was sent and ignored. More particularly, **an open rate is
//! meaningless when open tracking is off**, which is the trap this module
//! exists to avoid — `email_track_opens` off makes every `opened_at` NULL, so
//! the honest answer is "not measured", not "0%". [`Programme::open_rate`]
//! returns `None` in that case and the screen says why.

use std::fmt::Write as _;

pub use crate::windows::{days as window_days, key as window_key, OFFERED as WINDOWS};

/// How badly a problem with the sending programme matters.
///
/// The distinction is who it stops: `Blocking` means a post written today
/// reaches nobody, or reached nobody and could be made to; `Opportunity`
/// means sending works and is not being used well; `Note` is worth knowing
/// and may be deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Blocking,
    Opportunity,
    Note,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Blocking => "Blocking",
            Severity::Opportunity => "Opportunity",
            Severity::Note => "Note",
        }
    }

    /// The shared `.ic.tint-*` colour this severity reads as.
    pub fn tint(self) -> &'static str {
        match self {
            Severity::Blocking => "red",
            Severity::Opportunity => "amber",
            Severity::Note => "blue",
        }
    }
}

/// One thing wrong with, or missing from, the sending programme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// Stable identifier, used as the row's anchor. Never shown.
    pub code: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    pub detail: String,
    pub fix: Option<(&'static str, &'static str)>,
}

/// The state of the sending programme, as counted when a page is asked for.
#[derive(Debug, Clone, Default)]
pub struct Programme {
    // --- the newsletters themselves ---
    pub newsletters: i64,
    /// Newsletters with `status = 'active'`, which is Ghost's "sending".
    pub sending_newsletters: i64,
    /// Distinct members with a confirmed subscription to at least one.
    pub subscribers: i64,
    /// Confirmed subscription rows: a member on three newsletters is three.
    pub subscriptions: i64,
    pub members: i64,

    // --- the editions ---
    /// Every `emails` row, which is one per post ever emailed.
    pub editions: i64,
    /// Editions created inside the window.
    pub editions_in_window: i64,
    /// Editions whose own status is `failed` — the send as a whole came to
    /// nothing, as opposed to individual recipients bouncing.
    pub failed_editions: i64,
    /// Editions still `pending` or `submitted`. Non-zero for a long time
    /// means the send loop is not running.
    pub pending_editions: i64,
    /// The last edition's `created_at`, as a `'YYYY-MM-DD …'` string.
    pub last_sent: Option<String>,
    /// Days since the last edition, worked out by the caller from its own
    /// clock rather than from the database's.
    pub days_since_last: Option<i64>,

    // --- the recipients, inside the window ---
    /// Recipient rows written in the window: messages this site tried to
    /// deliver.
    pub attempted: i64,
    /// Recipient rows that reached somebody.
    pub delivered: i64,
    /// Recipient rows with an `opened_at`.
    pub opened: i64,
    /// Recipient rows that failed.
    pub failed: i64,
    /// Members who clicked a link in an edition in the window.
    pub clicked: i64,

    // --- the settings ---
    /// `editor_default_email_recipients` is not `disabled`.
    pub sending_on: bool,
    /// What carries the newsletters — "Mailgun", "Amazon SES", a relay name —
    /// or `None` with the reason, when nothing is configured.
    pub transport: Option<String>,
    /// Why there is no transport. Only meaningful when `transport` is `None`.
    pub transport_error: Option<String>,
    /// `email_track_opens`.
    pub track_opens: bool,
    /// `email_track_clicks`.
    pub track_clicks: bool,
    /// `members_signup_access == "none"` — nobody new can subscribe.
    pub signups_closed: bool,
    /// The sending allowance for the period, when the host enforces one.
    pub allowance: Option<i64>,
    /// Messages already sent against that allowance.
    pub allowance_used: i64,
}

impl Programme {
    /// Whether anything has ever gone out.
    pub fn has_sent(&self) -> bool {
        self.editions > 0
    }

    /// Share of delivered messages that were opened.
    ///
    /// `None` when open tracking is off, which is the distinction that
    /// matters: with `email_track_opens` off every `opened_at` is NULL, so
    /// "0%" would be a measurement that was never taken reported as a result.
    pub fn open_rate(&self) -> Option<f64> {
        if !self.track_opens {
            return None;
        }
        rate(self.opened, self.delivered)
    }

    /// Share of delivered messages whose links were clicked. `None` when
    /// click tracking is off, for the same reason as [`Self::open_rate`].
    pub fn click_rate(&self) -> Option<f64> {
        if !self.track_clicks {
            return None;
        }
        rate(self.clicked, self.delivered)
    }

    /// Share of attempted messages that arrived.
    pub fn delivery_rate(&self) -> Option<f64> {
        rate(self.delivered, self.attempted)
    }

    /// Share of attempted messages that failed.
    pub fn failure_rate(&self) -> Option<f64> {
        rate(self.failed, self.attempted)
    }

    /// Share of the membership that is subscribed to anything.
    pub fn reach(&self) -> Option<f64> {
        rate(self.subscribers, self.members)
    }

    /// Share of the sending allowance used, when there is one.
    pub fn allowance_used_percent(&self) -> Option<f64> {
        self.allowance.filter(|a| *a > 0).and_then(|a| rate(self.allowance_used, a))
    }
}

/// A percentage, or `None` when there is nothing to divide by.
///
/// `None` rather than `0.0`: a programme that has sent nothing has no open
/// rate, and printing `0%` says it was sent and ignored.
pub fn rate(part: i64, whole: i64) -> Option<f64> {
    if whole <= 0 {
        return None;
    }
    Some(part as f64 * 100.0 / whole as f64)
}

/// How many messages have to have been attempted before a delivery or open
/// rate means anything.
///
/// Twenty, and it is a judgement that belongs in a named constant: below it
/// one bounce reads as a five-per-cent failure rate, and a report that cries
/// deliverability at a site with four members teaches its owner to ignore the
/// report.
pub const RATES_MEANINGFUL_FROM: i64 = 20;

/// The share of attempted messages that may fail before it is a problem.
///
/// Two per cent. Mailgun and SES both start warning around there, and a
/// sustained five per cent is how a sending domain gets blocked.
pub const FAILURE_RATE_LIMIT: f64 = 2.0;

/// An open rate below this is worth mentioning once there is enough volume to
/// mean it. Twenty per cent is a weak newsletter by any published benchmark.
pub const LOW_OPEN_RATE: f64 = 20.0;

/// How long a programme may be quiet before the screen mentions it.
pub const QUIET_DAYS: i64 = 60;

/// The share of the allowance at which it is worth warning.
pub const ALLOWANCE_WARN_PERCENT: f64 = 80.0;

/// Everything wrong with the sending programme, worst first.
///
/// Within a severity the order is the order a message meets them: is sending
/// on, is there something to send with, is there a newsletter, is anybody
/// subscribed, did the last send work, and was it any good. A stable sort
/// keeps that intact, so the list does not reshuffle between two page loads
/// that found the same problems.
pub fn audit(p: &Programme) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = Vec::new();
    let mut gap = |code, severity, title, detail: String, fix| {
        gaps.push(Gap { code, severity, title, detail, fix });
    };

    const SENDING: (&str, &str) = ("Newsletter settings", "/ghost/settings/newsletters/");
    const EMAIL: (&str, &str) = ("Email settings", "/ghost/settings/email/");
    const TRACKING: (&str, &str) = ("Analytics settings", "/ghost/settings/analytics/");
    const EDITIONS: (&str, &str) = ("Editions", "/ghost/newsletters/editions/");
    const AUDIENCE: (&str, &str) = ("Membership settings", "/ghost/settings/membership/#access");

    // 1. Is sending even on, and is there anything to send with?
    if !p.sending_on {
        gap(
            "sending-off",
            Severity::Blocking,
            "Newsletter sending is off",
            "Publishing a post will not email anybody, whatever the post says about its \
             recipients."
                .into(),
            Some(SENDING),
        );
    }
    if p.transport.is_none() {
        gap(
            "no-transport",
            Severity::Blocking,
            "Nothing is configured to send newsletters",
            match p.transport_error.as_deref() {
                // The reason comes from the transport resolver, which knows
                // which of the three arrangements is half-finished. Repeating
                // it beats a generic sentence that sends somebody to the
                // wrong screen.
                Some(reason) if !reason.trim().is_empty() => reason.to_string(),
                _ => "Set up Mailgun, Amazon SES or an SMTP relay before publishing to a \
                      newsletter."
                    .into(),
            },
            Some(SENDING),
        );
    }

    // 2. Is there a newsletter, and is anybody on it?
    if p.newsletters == 0 {
        gap(
            "no-newsletter",
            Severity::Blocking,
            "There is no newsletter",
            "A post has nowhere to be emailed to. One newsletter is enough.".into(),
            Some(SENDING),
        );
    } else if p.sending_newsletters == 0 {
        gap(
            "all-archived",
            Severity::Blocking,
            "Every newsletter is archived",
            format!(
                "{} exist and none of them is sending, so a post cannot be emailed to any of \
                 them.",
                countable(p.newsletters, "newsletter"),
            ),
            Some(SENDING),
        );
    }
    if p.subscribers == 0 && p.members > 0 {
        gap(
            "nobody-subscribed",
            Severity::Blocking,
            "No member is subscribed to anything",
            format!(
                "{} and not one confirmed newsletter subscription. Check that the newsletter \
                 has \"subscribe on signup\" on, or the next send goes to nobody.",
                countable(p.members, "member"),
            ),
            Some(SENDING),
        );
    } else if let Some(reach) = p.reach().filter(|_| p.members >= RATES_MEANINGFUL_FROM) {
        if reach < 60.0 {
            gap(
                "reach-gap",
                Severity::Opportunity,
                "Most members are not subscribed",
                format!(
                    "{} of {} — {:.0}% of the membership — take a newsletter. The rest signed \
                     up and hear nothing.",
                    p.subscribers,
                    countable(p.members, "member"),
                    reach,
                ),
                Some(SENDING),
            );
        }
    }
    if p.signups_closed {
        gap(
            "signups-closed",
            Severity::Note,
            "Nobody new can subscribe",
            "Membership signups are closed, so the subscriber list can only shrink."
                .into(),
            Some(AUDIENCE),
        );
    }

    // 3. Did the sending work?
    if p.failed_editions > 0 {
        gap(
            "failed-editions",
            Severity::Blocking,
            "An edition failed to send",
            format!(
                "{} came to nothing. A failed send can be retried without mailing anybody \
                 twice — only the recipients it could not reach are tried again.",
                countable(p.failed_editions, "edition"),
            ),
            Some(EDITIONS),
        );
    }
    if p.pending_editions > 0 {
        gap(
            "pending-editions",
            Severity::Opportunity,
            "An edition is still waiting to go out",
            format!(
                "{} has been accepted and not finished. If that does not clear, the send loop \
                 is not running.",
                countable(p.pending_editions, "edition"),
            ),
            Some(EDITIONS),
        );
    }
    if let Some(failure) = p.failure_rate().filter(|_| p.attempted >= RATES_MEANINGFUL_FROM) {
        if failure > FAILURE_RATE_LIMIT {
            gap(
                "delivery-failing",
                Severity::Blocking,
                "Too many messages are bouncing",
                format!(
                    "{:.1}% of {} failed. Above about {:.0}% a sending domain starts to be \
                     treated as a spammer, which costs the messages that would have arrived.",
                    failure,
                    countable(p.attempted, "message"),
                    FAILURE_RATE_LIMIT,
                ),
                Some(EMAIL),
            );
        }
    }

    // 4. Is any of it measured, and was it any good?
    if !p.track_opens {
        gap(
            "opens-untracked",
            Severity::Note,
            "Opens are not tracked",
            "Every open rate on this screen reads as \"not measured\" rather than as a \
             number, because nothing is recording them."
                .into(),
            Some(TRACKING),
        );
    } else if let Some(open) = p.open_rate().filter(|_| p.delivered >= RATES_MEANINGFUL_FROM) {
        if open < LOW_OPEN_RATE {
            gap(
                "low-opens",
                Severity::Opportunity,
                "Few people are opening",
                format!(
                    "{:.0}% of {} delivered were opened. Below about {:.0}% the usual causes \
                     are the subject line and the sending address, not the writing.",
                    open,
                    countable(p.delivered, "message"),
                    LOW_OPEN_RATE,
                ),
                None,
            );
        }
    }
    if !p.track_clicks {
        gap(
            "clicks-untracked",
            Severity::Note,
            "Clicks are not tracked",
            "No link in a newsletter is counted, so there is no way to tell what people read \
             after opening."
                .into(),
            Some(TRACKING),
        );
    }

    // 5. Observations about the programme as a whole.
    if !p.has_sent() {
        gap(
            "never-sent",
            Severity::Opportunity,
            "No edition has gone out yet",
            "Nothing here is wrong — there is simply nothing to report on. Publishing a post \
             to a newsletter fills this screen in."
                .into(),
            None,
        );
    } else if let Some(quiet) = p.days_since_last.filter(|d| *d >= QUIET_DAYS) {
        gap(
            "gone-quiet",
            Severity::Opportunity,
            "Nothing has gone out for a while",
            format!(
                "The last edition was {}. A list that hears nothing for months forgets who \
                 you are, and opens fall when you come back.",
                countable(quiet, "day ago").replace("day agos", "days ago"),
            ),
            None,
        );
    }
    if let Some(used) = p.allowance_used_percent() {
        if used >= ALLOWANCE_WARN_PERCENT {
            gap(
                "allowance",
                if used >= 100.0 { Severity::Blocking } else { Severity::Note },
                "The sending allowance is nearly used up",
                format!(
                    "{} of {} messages for this period, {:.0}% of the allowance. A send that \
                     would go over it is refused outright.",
                    p.allowance_used,
                    p.allowance.unwrap_or(0),
                    used,
                ),
                None,
            );
        }
    }

    gaps.sort_by_key(|g| g.severity);
    gaps
}

/// "1 edition" / "3 editions", with an `s` on the noun.
///
/// A local copy rather than a call into `ghost-admin-ui`'s `countable`: this
/// crate is below that one, and the gap details are built here so they can be
/// asserted here.
fn countable(n: i64, noun: &str) -> String {
    let mut out = String::new();
    let _ = write!(out, "{n} {noun}");
    if n != 1 {
        out.push('s');
    }
    out
}

/// What an edition's `status` means, in words.
///
/// `emails.status` is Ghost's enum. Anything unrecognised reads as the
/// cautious answer rather than as success: an edition in an unknown state has
/// not been shown to have arrived.
pub fn edition_status(status: &str) -> (&'static str, &'static str) {
    match status {
        // (label, the shared `.chip` class)
        "submitted" => ("Sent", "published"),
        "sent" => ("Sent", "published"),
        "failed" => ("Failed", "error"),
        "pending" => ("Pending", "draft"),
        _ => ("Unknown", "tonal"),
    }
}

/// Whether an edition in this state can be retried.
///
/// Only a failed one. Retrying a send that worked would be a way to mail
/// everybody a second time, and retrying a pending one would race the send
/// loop that is still working on it.
pub fn is_retryable(status: &str) -> bool {
    status == "failed"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A programme with everything in place, as the baseline the tests
    /// perturb.
    fn healthy() -> Programme {
        Programme {
            newsletters: 1,
            sending_newsletters: 1,
            subscribers: 180,
            subscriptions: 180,
            members: 200,
            editions: 12,
            editions_in_window: 4,
            failed_editions: 0,
            pending_editions: 0,
            last_sent: Some("2026-11-14 09:00:00".into()),
            days_since_last: Some(2),
            attempted: 720,
            delivered: 715,
            opened: 340,
            failed: 5,
            clicked: 90,
            sending_on: true,
            transport: Some("Mailgun".into()),
            transport_error: None,
            track_opens: true,
            track_clicks: true,
            signups_closed: false,
            allowance: None,
            allowance_used: 0,
        }
    }

    fn codes(p: &Programme) -> Vec<&'static str> {
        audit(p).into_iter().map(|g| g.code).collect()
    }

    #[test]
    fn a_programme_with_everything_in_place_has_nothing_to_report() {
        assert!(audit(&healthy()).is_empty(), "{:?}", codes(&healthy()));
    }

    #[test]
    fn sending_switched_off_is_blocking_and_comes_first() {
        let mut p = healthy();
        p.sending_on = false;
        let gaps = audit(&p);
        assert_eq!(gaps[0].code, "sending-off");
        assert_eq!(gaps[0].severity, Severity::Blocking);
    }

    #[test]
    fn a_missing_transport_repeats_the_resolvers_own_reason() {
        // Three arrangements can be half-finished and the resolver knows
        // which. A generic sentence would send somebody to the wrong screen.
        let mut p = healthy();
        p.transport = None;
        p.transport_error = Some("Configure Mailgun in Settings \u{2192} Newsletters.".into());
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "no-transport").expect("no transport finding");
        assert_eq!(g.severity, Severity::Blocking);
        assert!(g.detail.contains("Configure Mailgun"), "{}", g.detail);
    }

    #[test]
    fn a_missing_transport_with_no_reason_still_says_something_useful() {
        let mut p = healthy();
        p.transport = None;
        p.transport_error = Some("   ".into());
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "no-transport").unwrap();
        assert!(g.detail.contains("Mailgun, Amazon SES or an SMTP relay"), "{}", g.detail);
    }

    #[test]
    fn archived_newsletters_are_reported_separately_from_having_none() {
        let mut p = healthy();
        p.sending_newsletters = 0;
        let archived = codes(&p);
        assert!(archived.contains(&"all-archived"), "{archived:?}");
        assert!(!archived.contains(&"no-newsletter"), "{archived:?}");

        p.newsletters = 0;
        let none = codes(&p);
        assert!(none.contains(&"no-newsletter"), "{none:?}");
        assert!(!none.contains(&"all-archived"), "both were reported at once: {none:?}");
    }

    #[test]
    fn nobody_subscribed_is_blocking_but_only_once_there_are_members_to_subscribe() {
        let mut p = healthy();
        p.subscribers = 0;
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "nobody-subscribed").expect("not reported");
        assert_eq!(g.severity, Severity::Blocking);
        // And it does not also complain that the reach is low, which would be
        // the same fact stated twice.
        assert!(!gaps.iter().any(|g| g.code == "reach-gap"));

        // A site with no members at all is not a site with a subscription
        // problem.
        p.members = 0;
        assert!(!codes(&p).contains(&"nobody-subscribed"));
    }

    #[test]
    fn a_low_reach_is_only_reported_once_the_membership_is_big_enough_to_mean_it() {
        let mut p = healthy();
        p.members = RATES_MEANINGFUL_FROM - 1;
        p.subscribers = 1;
        assert!(!codes(&p).contains(&"reach-gap"));

        p.members = 100;
        p.subscribers = 59;
        assert!(codes(&p).contains(&"reach-gap"));
        // At the threshold it is fine.
        p.subscribers = 60;
        assert!(!codes(&p).contains(&"reach-gap"));
    }

    #[test]
    fn a_failed_edition_is_blocking_and_says_a_retry_is_safe() {
        // The sentence matters: somebody who thinks a retry mails everybody
        // again will never press it, and the failed send stays failed.
        let mut p = healthy();
        p.failed_editions = 2;
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "failed-editions").unwrap();
        assert_eq!(g.severity, Severity::Blocking);
        assert!(g.detail.contains("without mailing anybody twice"), "{}", g.detail);
        assert_eq!(g.fix, Some(("Editions", "/ghost/newsletters/editions/")));
    }

    #[test]
    fn a_bounce_rate_over_the_limit_is_blocking_and_a_small_sample_is_not() {
        let mut p = healthy();
        // 3 of 100 is over the 2% limit.
        p.attempted = 100;
        p.delivered = 97;
        p.failed = 3;
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "delivery-failing").expect("not reported");
        assert_eq!(g.severity, Severity::Blocking);

        // 1 of 10 is ten per cent and means nothing.
        p.attempted = 10;
        p.delivered = 9;
        p.failed = 1;
        assert!(!codes(&p).contains(&"delivery-failing"), "a sample of ten was judged");
    }

    #[test]
    fn an_open_rate_is_not_measured_when_nothing_is_measuring_it() {
        // The trap this module exists for: with tracking off every opened_at
        // is NULL, so "0%" would report a measurement nobody took.
        let mut p = healthy();
        p.track_opens = false;
        p.opened = 0;
        assert_eq!(p.open_rate(), None);
        let codes = codes(&p);
        assert!(codes.contains(&"opens-untracked"), "{codes:?}");
        // And it must not then also claim the opens are low.
        assert!(!codes.contains(&"low-opens"), "a 0% reading was treated as a result");
    }

    #[test]
    fn a_low_open_rate_is_reported_once_there_is_volume_behind_it() {
        let mut p = healthy();
        p.delivered = 100;
        p.opened = 15;
        assert!(codes(&p).contains(&"low-opens"));
        p.opened = 25;
        assert!(!codes(&p).contains(&"low-opens"));
        // Four opens of nineteen is 21% and still means nothing.
        p.delivered = RATES_MEANINGFUL_FROM - 1;
        p.opened = 1;
        assert!(!codes(&p).contains(&"low-opens"));
    }

    #[test]
    fn click_tracking_being_off_is_its_own_note() {
        let mut p = healthy();
        p.track_clicks = false;
        assert_eq!(p.click_rate(), None);
        assert!(codes(&p).contains(&"clicks-untracked"));
    }

    #[test]
    fn a_programme_that_has_sent_nothing_is_told_it_is_new_rather_than_told_off() {
        let p = Programme {
            newsletters: 1,
            sending_newsletters: 1,
            subscribers: 0,
            members: 0,
            sending_on: true,
            transport: Some("Mailgun".into()),
            track_opens: true,
            track_clicks: true,
            ..Programme::default()
        };
        let codes = codes(&p);
        assert_eq!(codes, vec!["never-sent"], "{codes:?}");
        assert_eq!(audit(&p)[0].severity, Severity::Opportunity);
    }

    #[test]
    fn a_quiet_programme_is_mentioned_only_after_it_has_actually_sent_something() {
        let mut p = healthy();
        p.days_since_last = Some(QUIET_DAYS);
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "gone-quiet").expect("not reported");
        assert!(g.detail.contains("60 days ago"), "{}", g.detail);

        p.days_since_last = Some(QUIET_DAYS - 1);
        assert!(!codes(&p).contains(&"gone-quiet"));

        // Never sent and quiet are the same fact; only one is reported.
        p.editions = 0;
        p.days_since_last = None;
        let codes = codes(&p);
        assert!(codes.contains(&"never-sent"));
        assert!(!codes.contains(&"gone-quiet"));
    }

    #[test]
    fn one_day_of_quiet_is_not_pluralised() {
        // `countable` would make "1 day agos" of it.
        let mut p = healthy();
        p.days_since_last = Some(1);
        // Below the threshold, so nothing is reported — but the helper the
        // sentence uses is still asserted, because the day it does fire the
        // mistake would be in production prose.
        assert!(!codes(&p).contains(&"gone-quiet"));
        assert_eq!(countable(1, "day ago").replace("day agos", "days ago"), "1 day ago");
        assert_eq!(countable(60, "day ago").replace("day agos", "days ago"), "60 days ago");
    }

    #[test]
    fn the_allowance_warns_at_four_fifths_and_blocks_when_it_is_gone() {
        let mut p = healthy();
        p.allowance = Some(1_000);
        p.allowance_used = 790;
        assert!(!codes(&p).contains(&"allowance"));

        p.allowance_used = 800;
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "allowance").expect("not reported");
        assert_eq!(g.severity, Severity::Note);

        p.allowance_used = 1_000;
        let gaps = audit(&p);
        let g = gaps.iter().find(|g| g.code == "allowance").unwrap();
        assert_eq!(g.severity, Severity::Blocking);

        // No allowance means no finding, not a division by zero.
        p.allowance = None;
        assert_eq!(p.allowance_used_percent(), None);
        assert!(!codes(&p).contains(&"allowance"));
        p.allowance = Some(0);
        assert_eq!(p.allowance_used_percent(), None);
    }

    #[test]
    fn the_gaps_come_back_worst_first() {
        let mut p = healthy();
        p.sending_on = false; // Blocking
        p.track_clicks = false; // Note
        p.pending_editions = 1; // Opportunity
        let severities: Vec<_> = audit(&p).into_iter().map(|g| g.severity).collect();
        let mut sorted = severities.clone();
        sorted.sort();
        assert_eq!(severities, sorted, "{severities:?}");
    }

    #[test]
    fn every_gap_has_a_unique_code_so_an_anchor_points_at_one_row() {
        let p = Programme {
            newsletters: 2,
            sending_newsletters: 0,
            subscribers: 0,
            members: 400,
            editions: 9,
            failed_editions: 1,
            pending_editions: 1,
            days_since_last: Some(200),
            attempted: 400,
            delivered: 300,
            failed: 100,
            sending_on: false,
            transport: None,
            transport_error: Some("nothing configured".into()),
            track_opens: false,
            track_clicks: false,
            signups_closed: true,
            allowance: Some(100),
            allowance_used: 100,
            ..Programme::default()
        };
        let codes = codes(&p);
        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(codes.len(), unique.len(), "{codes:?}");
        assert!(codes.len() >= 9, "expected a pile of findings, got {codes:?}");
    }

    #[test]
    fn a_fix_link_always_points_inside_the_admin() {
        let p = Programme {
            newsletters: 0,
            members: 100,
            editions: 3,
            failed_editions: 1,
            signups_closed: true,
            sending_on: false,
            transport: None,
            ..Programme::default()
        };
        for gap in audit(&p) {
            if let Some((_, path)) = gap.fix {
                assert!(path.starts_with("/ghost/"), "{} links to {path}", gap.code);
                assert!(!path.starts_with("//"), "{} links to {path}", gap.code);
            }
        }
    }

    #[test]
    fn a_rate_with_nothing_to_divide_by_is_absent_rather_than_zero() {
        assert_eq!(rate(0, 0), None);
        assert_eq!(rate(5, -1), None);
        assert_eq!(rate(1, 4), Some(25.0));
        let empty = Programme::default();
        assert_eq!(empty.delivery_rate(), None);
        assert_eq!(empty.reach(), None);
    }

    #[test]
    fn an_unknown_edition_status_is_not_reported_as_a_success() {
        assert_eq!(edition_status("submitted"), ("Sent", "published"));
        assert_eq!(edition_status("failed"), ("Failed", "error"));
        assert_eq!(edition_status("pending"), ("Pending", "draft"));
        assert_eq!(edition_status("whatever"), ("Unknown", "tonal"));
    }

    #[test]
    fn only_a_failed_edition_can_be_retried() {
        // Retrying a send that worked is a way to mail everybody twice;
        // retrying a pending one races the send loop still working on it.
        assert!(is_retryable("failed"));
        for status in ["submitted", "sent", "pending", "", "FAILED"] {
            assert!(!is_retryable(status), "{status} was offered a retry");
        }
    }
}
