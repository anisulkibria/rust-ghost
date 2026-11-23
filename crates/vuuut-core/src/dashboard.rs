//! What the admin home decides to say, with no database and no HTML in it.
//!
//! The dashboard at `/ghost/dashboard/` is the first screen a signed-in staff
//! member sees. It answers one question — *what should I do next?* — and
//! almost all of it is a direct reading of counts. The judgements that are not
//! direct live here:
//!
//! * which queues are actually waiting on somebody ([`pending`]), and in what
//!   order they deserve attention;
//! * which parts of setting the site up are still undone ([`setup`]);
//! * which of the modules that audit themselves are reporting that somebody is
//!   stopped, and which of those matters most ([`health`]);
//! * what time of day it is, in words ([`greeting`]).
//!
//! All of them are pure functions over plain counts and flags, so each can be
//! tested without a database, an HTTP request or a logged-in user — which is
//! the point, because every one of them is a *ranking* or a *threshold* and
//! those are the parts that go quietly wrong. The admin module turns the
//! returned values into rows, links and icons; nothing here knows a URL.
//!
//! This module is not a trust boundary. Nothing it reads comes from a visitor:
//! the counts are produced by `vuuut_db::repos::dashboard` from the site's own
//! tables, and the flags from the site's own settings.

/// Everything the home screen's queues are derived from.
///
/// Counts, not rows: deciding whether a queue needs attention never requires
/// the contents of it, and reading only counts is what lets the whole screen
/// be one database round trip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Queues {
    /// Comments a reader has reported and nobody has acted on.
    pub reported_comments: i64,
    /// Form submissions still in the `new` state.
    pub new_responses: i64,
    /// Newsletter sends that ended in `failed`.
    pub failed_emails: i64,
    /// Posts still `scheduled` whose publish time has already passed. A
    /// non-zero count here means the scheduler is not running, which is the
    /// most serious thing this screen can report.
    pub overdue_scheduled: i64,
}

/// One queue that is waiting on a person, carrying how many items are in it.
///
/// An enum rather than a label and a link, so the ordering below can be tested
/// without asserting on prose, and so the admin owns every word the reader
/// sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// Scheduled posts whose time has passed — the scheduler has stopped.
    OverdueScheduled(i64),
    /// Newsletter sends that failed.
    FailedEmails(i64),
    /// Reported comments awaiting moderation.
    ReportedComments(i64),
    /// Unread form responses.
    NewResponses(i64),
}

impl Pending {
    /// How many items the queue holds.
    pub fn count(self) -> i64 {
        match self {
            Pending::OverdueScheduled(n)
            | Pending::FailedEmails(n)
            | Pending::ReportedComments(n)
            | Pending::NewResponses(n) => n,
        }
    }

    /// Whether this is a fault to fix rather than work to do.
    ///
    /// The distinction drives the colour and the icon on the screen, and it is
    /// a real one: an overdue post or a failed send means the site is not
    /// doing what it was told, while a reported comment or a new response
    /// means the site worked and somebody now has reading to do. Showing both
    /// in alarm red would make the alarm meaningless.
    pub fn is_fault(self) -> bool {
        matches!(self, Pending::OverdueScheduled(_) | Pending::FailedEmails(_))
    }
}

/// The queues with anything in them, most urgent first.
///
/// Empty queues are dropped rather than listed as zero. A list of four
/// "0 waiting" rows is noise on every well-run site, and noise is what stops
/// the one row that matters from being noticed.
///
/// The order is faults before work, and within faults the one that means the
/// site is broken comes first. It is fixed rather than computed from the
/// counts: three reported comments are not more urgent than one post that
/// failed to go out, however the numbers compare.
pub fn pending(queues: &Queues) -> Vec<Pending> {
    [
        Pending::OverdueScheduled(queues.overdue_scheduled),
        Pending::FailedEmails(queues.failed_emails),
        Pending::ReportedComments(queues.reported_comments),
        Pending::NewResponses(queues.new_responses),
    ]
    .into_iter()
    .filter(|item| item.count() > 0)
    .collect()
}

/// The site's own state, as far as the setup checklist is concerned.
///
/// Every field is a flag rather than a value: the checklist only ever asks
/// "has this been done", and passing the values themselves would invite it to
/// start judging them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Site {
    /// At least one post is published. A site with only drafts is not live.
    pub has_published: bool,
    /// `description` is set, which is what search results and the theme's
    /// header show under the title.
    pub has_description: bool,
    /// `icon` is set — the favicon and the apple-touch icon.
    pub has_icon: bool,
    /// Paid signups are offered (`members_signup_access` is not `none` and a
    /// paid tier exists).
    pub sells_memberships: bool,
    /// A Stripe account is connected.
    pub stripe_connected: bool,
    /// `web_analytics` is on, so visits are being recorded at all.
    pub analytics_on: bool,
    /// Whether the Memberships module is already reporting blocking gaps of
    /// its own, which it does on the same page a few rows further up.
    ///
    /// Not a fact about the site like the fields above it, and here anyway
    /// because the decision it drives is this function's: when the module that
    /// *owns* the funnel is saying "three things stop somebody paying, and
    /// here they are", this checklist's one-line guess at the same thing is
    /// noise. `setup` drops [`Setup::ConnectStripe`] rather than the module
    /// dropping its gap — the module's version names the other two.
    pub funnel_reported_blocking: bool,
}

/// One thing about the site that has not been set up yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    /// Nothing is published.
    FirstPost,
    /// No site description.
    Describe,
    /// No site icon.
    Icon,
    /// Memberships are being sold with no Stripe account behind them, so
    /// nobody can actually pay. Listed only when both halves are true — a site
    /// that does not sell anything is not missing Stripe — and only while the
    /// Memberships module is not reporting the same thing in more detail; see
    /// [`Site::funnel_reported_blocking`].
    ConnectStripe,
    /// Web analytics is off, so the figures elsewhere on this screen will stay
    /// at zero however much traffic arrives.
    TurnOnAnalytics,
}

/// What is still undone, in the order worth doing it.
///
/// Publishing comes first because nothing else matters on a site with nothing
/// on it; the two cosmetic items follow because they are a minute's work each;
/// Stripe comes next, and only when memberships are actually being sold; and
/// analytics comes last because it changes nothing a reader sees.
///
/// An empty result is the normal state of a running site, and the screen shows
/// nothing at all rather than a completed checklist — a finished list is a
/// trophy, and this is a work surface.
pub fn setup(site: &Site) -> Vec<Setup> {
    let mut steps = Vec::new();
    if !site.has_published {
        steps.push(Setup::FirstPost);
    }
    if !site.has_description {
        steps.push(Setup::Describe);
    }
    if !site.has_icon {
        steps.push(Setup::Icon);
    }
    if site.sells_memberships && !site.stripe_connected && !site.funnel_reported_blocking {
        steps.push(Setup::ConnectStripe);
    }
    if !site.analytics_on {
        steps.push(Setup::TurnOnAnalytics);
    }
    steps
}

// ---------------------------------------------------------------------------
// what the self-auditing modules report
// ---------------------------------------------------------------------------

/// What the security report found, reduced to the three numbers the home
/// screen can act on.
///
/// Counts of findings rather than the findings themselves: the home screen
/// never reproduces another module's report, it says how much of one is
/// waiting and links to it. Reproducing it would mean two screens that have to
/// agree about wording, and the one with less room would lose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Security {
    /// [`crate::security::score`] over the findings.
    pub score: i64,
    /// Findings at [`crate::security::Severity::Critical`] — a defence that is
    /// absent or bypassable.
    pub critical: i64,
    /// Findings at [`crate::security::Severity::Warning`].
    pub warnings: i64,
    /// Addresses currently serving a brute-force lockout.
    ///
    /// Not part of the audit: a lockout is a defence *working*, and the report
    /// shows it as a figure rather than as a finding. It is here because it is
    /// the one security fact that is about right now rather than about how the
    /// site is configured.
    pub lockouts: i64,
}

/// What the modules that audit themselves say, as the home screen reads them.
///
/// Only the counts that mean *somebody is stopped*: `Blocking` from
/// [`crate::memberships`] and [`crate::newsletters`], `Critical` from
/// [`crate::security`]. Their `Opportunity` and `Note` levels are growth
/// advice and deliberate choices, which belong on the module's own screen —
/// a home screen that listed them would be a list nobody finishes, and that
/// is how the rows that matter stop being read.
///
/// SEO audits itself too and is deliberately absent: its findings are
/// per-entry rather than per-site, so a count would mean reading every post on
/// every visit to the home screen, and its site-level checks mix things that
/// are missing with things that were chosen.
///
/// [`Default`] is *not* a healthy site — it is a site nothing has been
/// measured on, which is why `security` is an [`Option`]. A failed measurement
/// reports nothing rather than reporting a zero score as "At risk".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Audits {
    /// `None` when the posture could not be measured.
    pub security: Option<Security>,
    /// Membership gaps at `Blocking`: somebody trying to join or pay is
    /// stopped by each one.
    pub membership_blocking: i64,
    /// Sending gaps at `Blocking`: a post emailed today reaches nobody.
    pub newsletter_blocking: i64,
}

/// One module reporting that something is wrong with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Security findings at `Critical`.
    SecurityCritical { critical: i64, score: i64 },
    /// Addresses locked out of signing in at this moment.
    Lockouts(i64),
    /// The newsletter cannot reach anybody.
    Newsletters(i64),
    /// Nobody can join or pay.
    Memberships(i64),
    /// No critical finding, but the posture is not what it should be.
    SecurityPosture { score: i64, warnings: i64 },
}

impl Health {
    /// Whether this is the site failing at something rather than a defence
    /// doing its job or a score worth improving.
    ///
    /// The same distinction [`Pending::is_fault`] draws, and it drives the
    /// same colours: an open door, a newsletter that reaches nobody and a shop
    /// that cannot take money are all the site not doing what it was told. A
    /// lockout and a middling score are not.
    pub fn is_fault(self) -> bool {
        matches!(
            self,
            Health::SecurityCritical { .. } | Health::Newsletters(_) | Health::Memberships(_)
        )
    }
}

/// Whether a score is below the band [`crate::security::grade`] calls "Strong".
///
/// Asked of `grade` rather than compared against a number here, so the home
/// screen's idea of "good enough" cannot drift from the security report's.
fn posture_is_short(score: i64) -> bool {
    crate::security::grade(score).0 != "Strong"
}

/// What the modules are reporting, worst first.
///
/// The order is fixed rather than computed, for the reason [`pending`]'s is:
/// these are different kinds of wrong and no arithmetic over their counts
/// compares them. Reading down —
///
/// 1. a critical security finding: a defence the site relies on is not there,
///    and everything else on this list assumes it is;
/// 2. an address locked out: either somebody is guessing at a password right
///    now, or a colleague cannot get in — both are about this minute;
/// 3. a newsletter that reaches nobody: a post published today is lost, and
///    unlike the two below it the loss is not recoverable later;
/// 4. a shop that cannot take money: a visitor who wanted to pay has left;
/// 5. the posture score, last, because a site with no critical finding and a
///    middling score is a site to improve rather than a site to fix.
///
/// Security appears at either the top or the bottom and never both: a site
/// with a critical finding is told *that*, and the score it implies is on the
/// row already.
pub fn health(audits: &Audits) -> Vec<Health> {
    let mut out = Vec::new();
    let security = audits.security.unwrap_or_default();
    let critical = audits.security.is_some() && security.critical > 0;

    if critical {
        out.push(Health::SecurityCritical { critical: security.critical, score: security.score });
    }
    if security.lockouts > 0 {
        out.push(Health::Lockouts(security.lockouts));
    }
    if audits.newsletter_blocking > 0 {
        out.push(Health::Newsletters(audits.newsletter_blocking));
    }
    if audits.membership_blocking > 0 {
        out.push(Health::Memberships(audits.membership_blocking));
    }
    if !critical && audits.security.is_some() && posture_is_short(security.score) {
        out.push(Health::SecurityPosture { score: security.score, warnings: security.warnings });
    }
    out
}

/// "Good morning" / "Good afternoon" / "Good evening" for an hour of the day.
///
/// Boundaries at 05, 12 and 18 in the *site's* timezone — see
/// [`hour_in_zone`]. The small hours get "Good evening" rather than a fourth
/// phrase: somebody working at 03:00 is finishing a long evening, not starting
/// a morning, and "Good night" reads as a farewell on a screen they have just
/// opened.
pub fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// The hour of the day at `now`, in `timezone`.
///
/// The site's timezone, not UTC, for the same reason the rest of the admin
/// uses it: a greeting that says "Good evening" over breakfast is worse than
/// no greeting. An unparseable zone falls back to UTC rather than failing,
/// which is how [`crate::dates::short_date_in_zone`] treats it too.
pub fn hour_in_zone(now: chrono::DateTime<chrono::Utc>, timezone: &str) -> u32 {
    use chrono::Timelike;
    now.with_timezone(&zone(timezone)).hour()
}

/// "Sunday 16 November" at `now`, in `timezone`.
///
/// No year: the home screen is a screen about today, and a year on it is a
/// word nobody reads. Dates further in the past or future are rendered by
/// [`crate::dates::short_date_in_zone`], which does carry one.
///
/// Here rather than in the admin because the timezone database lives in this
/// crate's dependencies, and because the day a screen calls "today" should be
/// decided in exactly one place.
pub fn today_in_zone(now: chrono::DateTime<chrono::Utc>, timezone: &str) -> String {
    now.with_timezone(&zone(timezone)).format("%A %-d %B").to_string()
}

/// A timezone name, or UTC if it is not one.
///
/// Falling back rather than failing, the same way
/// [`crate::dates::short_date_in_zone`] does: a greeting an hour out is a
/// small wrong, and a home screen that will not render is a large one.
fn zone(timezone: &str) -> chrono_tz::Tz {
    timezone.parse().unwrap_or(chrono_tz::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-run site shows no queues at all, rather than four zeroes.
    #[test]
    fn nothing_waiting_lists_nothing() {
        assert!(pending(&Queues::default()).is_empty());
    }

    /// Faults come before work, whatever the counts are — this is the ranking
    /// the whole section rests on, so it is asserted with the numbers stacked
    /// against it.
    #[test]
    fn a_broken_scheduler_outranks_a_hundred_unread_responses() {
        let order = pending(&Queues {
            new_responses: 100,
            reported_comments: 50,
            failed_emails: 1,
            overdue_scheduled: 1,
        });
        assert_eq!(
            order,
            vec![
                Pending::OverdueScheduled(1),
                Pending::FailedEmails(1),
                Pending::ReportedComments(50),
                Pending::NewResponses(100),
            ]
        );
    }

    /// Only the non-empty queues survive, and they keep their relative order.
    #[test]
    fn empty_queues_are_dropped_without_disturbing_the_rest() {
        let order = pending(&Queues { new_responses: 3, overdue_scheduled: 2, ..Queues::default() });
        assert_eq!(order, vec![Pending::OverdueScheduled(2), Pending::NewResponses(3)]);
    }

    /// The fault/work split is what the screen colours by, so it is pinned.
    #[test]
    fn only_a_site_misbehaving_counts_as_a_fault() {
        assert!(Pending::OverdueScheduled(1).is_fault());
        assert!(Pending::FailedEmails(1).is_fault());
        assert!(!Pending::ReportedComments(1).is_fault());
        assert!(!Pending::NewResponses(1).is_fault());
    }

    /// A finished site shows no checklist.
    #[test]
    fn a_fully_set_up_site_has_no_steps() {
        let site = Site {
            has_published: true,
            has_description: true,
            has_icon: true,
            sells_memberships: true,
            stripe_connected: true,
            analytics_on: true,
            funnel_reported_blocking: false,
        };
        assert!(setup(&site).is_empty());
    }

    /// When Memberships is already reporting blocking gaps, the checklist does
    /// not repeat the one it can see — the module's rows name the other two,
    /// and two sections of the same page asking for Stripe is how a checklist
    /// starts reading as noise.
    #[test]
    fn stripe_is_left_to_the_memberships_module_when_it_is_reporting() {
        let selling = Site { sells_memberships: true, ..Site::default() };
        assert!(setup(&selling).contains(&Setup::ConnectStripe));

        let also_reported = Site { funnel_reported_blocking: true, ..selling };
        assert!(!setup(&also_reported).contains(&Setup::ConnectStripe));
        // And nothing else about the checklist changes.
        assert!(setup(&also_reported).contains(&Setup::FirstPost));
    }

    /// A brand-new site that sells nothing is not nagged about Stripe — the
    /// negative case matters more than the positive one, because an
    /// irrelevant step is what teaches people to ignore the list.
    #[test]
    fn stripe_is_only_asked_for_when_something_is_being_sold() {
        let blank = Site::default();
        assert!(!setup(&blank).contains(&Setup::ConnectStripe));

        let selling = Site { sells_memberships: true, ..Site::default() };
        assert!(setup(&selling).contains(&Setup::ConnectStripe));

        let selling_and_connected =
            Site { sells_memberships: true, stripe_connected: true, ..Site::default() };
        assert!(!setup(&selling_and_connected).contains(&Setup::ConnectStripe));
    }

    /// Publishing leads and analytics trails, on a site with everything undone.
    #[test]
    fn the_checklist_starts_with_publishing_and_ends_with_analytics() {
        let steps = setup(&Site { sells_memberships: true, ..Site::default() });
        assert_eq!(
            steps,
            vec![
                Setup::FirstPost,
                Setup::Describe,
                Setup::Icon,
                Setup::ConnectStripe,
                Setup::TurnOnAnalytics,
            ]
        );
    }

    /// A site nothing has been measured on reports nothing. The trap this
    /// pins is real: a zero score grades as "At risk", so a failed
    /// measurement read as a `Default` would put a red row on every home
    /// screen it touched.
    #[test]
    fn an_unmeasured_site_reports_no_module_at_all() {
        assert!(health(&Audits::default()).is_empty());
        assert!(health(&Audits { security: None, ..Audits::default() }).is_empty());
    }

    /// A healthy, fully measured site shows no health rows either — the same
    /// rule the queues and the checklist keep.
    #[test]
    fn a_healthy_site_shows_nothing() {
        let audits = Audits {
            security: Some(Security { score: 100, critical: 0, warnings: 0, lockouts: 0 }),
            membership_blocking: 0,
            newsletter_blocking: 0,
        };
        assert!(health(&audits).is_empty());
    }

    /// The ranking the section rests on, asserted with the counts stacked
    /// against it: one critical finding outranks any number of gaps.
    #[test]
    fn a_critical_finding_leads_and_the_score_trails() {
        let order = health(&Audits {
            security: Some(Security { score: 40, critical: 2, warnings: 3, lockouts: 1 }),
            membership_blocking: 9,
            newsletter_blocking: 9,
        });
        assert_eq!(
            order,
            vec![
                Health::SecurityCritical { critical: 2, score: 40 },
                Health::Lockouts(1),
                Health::Newsletters(9),
                Health::Memberships(9),
            ]
        );
        // Security is at the top or the bottom, never both.
        assert!(!order.iter().any(|h| matches!(h, Health::SecurityPosture { .. })));
    }

    /// With nothing critical, the posture row comes last rather than first —
    /// a site to improve, not a site to fix.
    #[test]
    fn a_middling_posture_is_reported_after_the_things_that_are_broken() {
        let order = health(&Audits {
            security: Some(Security { score: 80, critical: 0, warnings: 2, lockouts: 0 }),
            membership_blocking: 1,
            newsletter_blocking: 0,
        });
        assert_eq!(
            order,
            vec![Health::Memberships(1), Health::SecurityPosture { score: 80, warnings: 2 }]
        );
    }

    /// "Good enough" is the security report's own band, so the two screens
    /// cannot disagree about whether a score is worth mentioning.
    #[test]
    fn the_posture_row_appears_exactly_below_the_strong_band() {
        let at = |score| {
            health(&Audits {
                security: Some(Security { score, ..Security::default() }),
                ..Audits::default()
            })
        };
        assert!(at(90).is_empty());
        assert!(at(100).is_empty());
        assert_eq!(at(89), vec![Health::SecurityPosture { score: 89, warnings: 0 }]);
        assert_eq!(crate::security::grade(90).0, "Strong");
        assert_ne!(crate::security::grade(89).0, "Strong");
    }

    /// A lockout is reported even on an otherwise perfect site: it is the one
    /// row here about this minute rather than about configuration.
    #[test]
    fn a_live_lockout_is_reported_on_a_hardened_site() {
        let order = health(&Audits {
            security: Some(Security { score: 100, critical: 0, warnings: 0, lockouts: 3 }),
            ..Audits::default()
        });
        assert_eq!(order, vec![Health::Lockouts(3)]);
    }

    /// The fault/work split, which the section colours by — a defence doing
    /// its job is not a fault, and neither is a score.
    #[test]
    fn only_the_site_failing_counts_as_a_fault() {
        assert!(Health::SecurityCritical { critical: 1, score: 50 }.is_fault());
        assert!(Health::Newsletters(1).is_fault());
        assert!(Health::Memberships(1).is_fault());
        assert!(!Health::Lockouts(1).is_fault());
        assert!(!Health::SecurityPosture { score: 80, warnings: 1 }.is_fault());
    }

    #[test]
    fn the_day_is_divided_at_five_twelve_and_eighteen() {
        assert_eq!(greeting(5), "Good morning");
        assert_eq!(greeting(11), "Good morning");
        assert_eq!(greeting(12), "Good afternoon");
        assert_eq!(greeting(17), "Good afternoon");
        assert_eq!(greeting(18), "Good evening");
        // The small hours are an evening that has not ended, not a morning.
        assert_eq!(greeting(23), "Good evening");
        assert_eq!(greeting(0), "Good evening");
        assert_eq!(greeting(4), "Good evening");
    }

    /// The greeting follows the site's clock, not the server's.
    #[test]
    fn the_hour_is_read_in_the_sites_own_zone() {
        let noon_utc = chrono::DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(hour_in_zone(noon_utc, "Etc/UTC"), 12);
        // Dhaka is UTC+6, so the same instant is early evening there.
        assert_eq!(hour_in_zone(noon_utc, "Asia/Dhaka"), 18);
        assert_eq!(greeting(hour_in_zone(noon_utc, "Asia/Dhaka")), "Good evening");
        // An unknown zone falls back to UTC rather than panicking.
        assert_eq!(hour_in_zone(noon_utc, "Mars/Olympus"), 12);
    }

    /// Today's date reads as a day and a month, in the site's zone, with no
    /// year — and the zone can carry it over into the next day.
    #[test]
    fn today_is_the_sites_day_not_the_servers() {
        let late = chrono::DateTime::parse_from_rfc3339("2026-11-15T20:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(today_in_zone(late, "Etc/UTC"), "Sunday 15 November");
        // 20:00 UTC is already 02:00 the next day in Dhaka.
        assert_eq!(today_in_zone(late, "Asia/Dhaka"), "Monday 16 November");
        assert!(!today_in_zone(late, "Etc/UTC").contains("2026"));
    }
}
