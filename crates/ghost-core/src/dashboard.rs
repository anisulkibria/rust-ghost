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
//! * what time of day it is, in words ([`greeting`]).
//!
//! All three are pure functions over plain counts and flags, so each can be
//! tested without a database, an HTTP request or a logged-in user — which is
//! the point, because every one of them is a *ranking* or a *threshold* and
//! those are the parts that go quietly wrong. The admin module turns the
//! returned values into rows, links and icons; nothing here knows a URL.
//!
//! This module is not a trust boundary. Nothing it reads comes from a visitor:
//! the counts are produced by `ghost_db::repos::dashboard` from the site's own
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
    /// that does not sell anything is not missing Stripe.
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
    if site.sells_memberships && !site.stripe_connected {
        steps.push(Setup::ConnectStripe);
    }
    if !site.analytics_on {
        steps.push(Setup::TurnOnAnalytics);
    }
    steps
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
        };
        assert!(setup(&site).is_empty());
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
