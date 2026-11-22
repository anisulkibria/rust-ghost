//! The Memberships module's rules: the arithmetic of a membership funnel, and
//! the judgement about what is missing from one.
//!
//! The Memberships module is an **organiser**, not a new store. Every number
//! it shows is already in the database — members, tiers, offers, newsletters,
//! subscriptions — and every switch it links to already has a settings
//! screen. What did not exist before was anywhere that put them beside each
//! other and said whether, taken together, they add up to a membership
//! business a visitor can actually join and pay for.
//!
//! That judgement is what lives here, and it lives here rather than in the
//! admin screen for the usual two reasons: it can be wrong, and it has to be
//! unit-testable without a database or a browser. A funnel can be broken in
//! ways no single settings page can see — paid tiers with Stripe
//! disconnected, Stripe connected with every tier hidden from the portal, a
//! portal offering no plan at all — and each of those is invisible from
//! inside the screen that owns half of it.
//!
//! ## This is not a trust boundary
//!
//! Nothing here parses anything a visitor supplied. [`Funnel`] is assembled
//! from counts and from settings an administrator wrote, so unlike
//! [`crate::forms`] or [`crate::firewall`] this module decides nothing about
//! who is allowed to do what. What it owns is the *order* gaps are shown in
//! and the *thresholds* at which one appears — which is exactly the argument
//! [`crate::dashboard`] makes for its rankings, and the same reason it is
//! tested here rather than inspected in a rendered page.
//!
//! The one piece that does take something from a request is
//! [`window_days`], which reduces a query parameter to one of the three
//! windows the screen offers. It is here so that the reduction and the list
//! of offered values cannot drift apart.
//!
//! ## What the arithmetic is careful about
//!
//! Money is in **minor units** throughout (cents), as it is everywhere else
//! in this codebase, and MRR arrives as a `REAL` from the subscriptions table
//! — see `ghost_db::repos::memberships::funnel`. Rates are returned as
//! `f64` percentages and are **never** computed when the denominator is zero:
//! a site with no members has no conversion rate, and rendering `0%` for one
//! would read as a failure rather than as an absence.

use std::fmt::Write as _;

/// How badly a gap in the funnel matters.
///
/// Three levels rather than two because the useful distinction is not
/// "serious or not" but *who it stops*: a `Blocking` gap means somebody
/// trying to join or pay right now cannot, an `Opportunity` means the
/// machinery works and is not being used, and a `Note` is worth knowing and
/// may well be deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Somebody trying to sign up or pay is stopped by this.
    Blocking,
    /// Nothing is broken; something that would grow the membership is unused.
    Opportunity,
    /// Worth knowing. Often a deliberate choice.
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

    /// The shared `.ic.tint-*` / `.chip` colour this severity reads as.
    pub fn tint(self) -> &'static str {
        match self {
            Severity::Blocking => "red",
            Severity::Opportunity => "amber",
            Severity::Note => "blue",
        }
    }
}

/// One thing that is missing from, or broken about, the membership funnel.
///
/// `fix` is a label and a path into the screen that owns the setting. It is
/// `Option` because two of the gaps ("no members yet", "nobody has paid") are
/// observations with no switch behind them, and a Fix button that led to a
/// page where nothing could be changed would be worse than no button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// Stable identifier, used as the row's anchor so a link can point at one
    /// gap. Never shown.
    pub code: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    /// A sentence or two naming the actual numbers, built here rather than in
    /// the page so it can be asserted on.
    pub detail: String,
    pub fix: Option<(&'static str, &'static str)>,
}

/// The state of the membership funnel, as counted at the moment a page is
/// asked for.
///
/// Assembled in one place — see `ghost_db::repos::memberships::funnel` — so
/// that [`audit`] stays a pure function of facts and the screen cannot judge
/// something it forgot to look up.
#[derive(Debug, Clone, Default)]
pub struct Funnel {
    // --- who is here ---
    pub members: i64,
    pub free: i64,
    pub paid: i64,
    pub comped: i64,
    /// Members whose email delivery has been switched off after bouncing.
    pub email_disabled: i64,

    // --- what happened in the window ---
    /// Members created inside the window.
    pub joined: i64,
    /// Members created inside the window immediately before this one, for the
    /// trend arrow. Counted rather than derived, so a member deleted since
    /// does not silently change the past.
    pub joined_before: i64,
    /// Members who became paid inside the window.
    pub upgraded: i64,
    /// Subscription cancellations inside the window.
    pub cancelled: i64,

    // --- money ---
    /// Monthly recurring revenue in minor units.
    pub mrr_cents: i64,
    /// The currency MRR is in, lowercase as Stripe stores it.
    pub currency: String,

    // --- what is on offer ---
    /// Active tiers of type `paid`.
    pub paid_tiers: i64,
    /// Active paid tiers a visitor can actually reach: `visibility <> 'none'`.
    pub visible_paid_tiers: i64,
    /// Offers with `active = 1`.
    pub active_offers: i64,
    pub newsletters: i64,

    // --- settings ---
    /// `members_signup_access`: `all`, `invite` or `none`.
    pub signup_access: String,
    /// `portal_plans`, as stored.
    pub portal_plans: Vec<String>,
    pub stripe_connected: bool,
    /// `comments_enabled`: `off`, `all` or `paid`.
    pub comments_enabled: String,
    /// A suggested donation amount has been set, so tips are on.
    pub donations_on: bool,
    /// `default_content_visibility`: `public`, `members`, `paid` or `tiers`.
    pub default_visibility: String,
}

impl Funnel {
    /// Members who are paying or have been given a paid tier.
    pub fn supported(&self) -> i64 {
        self.paid + self.comped
    }

    /// Whether this site is trying to sell anything at all.
    ///
    /// Both halves, as the dashboard's checklist has it: a site with signups
    /// switched off, or with no paid tier a visitor can reach, is not a site
    /// with a broken shop — it is a site with no shop, and telling its owner
    /// about Stripe would be noise.
    pub fn sells(&self) -> bool {
        self.signup_access != "none" && self.paid_tiers > 0
    }

    /// Whether the signup portal offers a paid plan.
    pub fn portal_sells(&self) -> bool {
        self.portal_plans.iter().any(|p| p == "monthly" || p == "yearly")
    }
}

/// Every gap in the funnel, worst first.
///
/// Ordering is by severity and then by the order the checks are written in,
/// which is roughly the order a visitor meets them: can they sign up, is
/// there anything to buy, can they pay, will they hear from you again. A
/// stable sort keeps that intact inside each severity, so the list does not
/// reshuffle between two page loads that found the same problems.
pub fn audit(f: &Funnel) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = Vec::new();
    let mut gap = |code, severity, title, detail: String, fix| {
        gaps.push(Gap { code, severity, title, detail, fix });
    };

    const ACCESS: (&str, &str) = ("Open signups", "/ghost/settings/memberships/#access");
    const PORTAL: (&str, &str) = ("Signup portal", "/ghost/settings/memberships/#portal");
    const STRIPE: (&str, &str) = ("Connect Stripe", "/ghost/settings/memberships/#stripe");
    const TIERS: (&str, &str) = ("Tiers", "/ghost/memberships/tiers/");
    const OFFERS: (&str, &str) = ("Offers", "/ghost/memberships/offers/");

    // 1. The door.
    match f.signup_access.as_str() {
        "none" => gap(
            "signups-closed",
            Severity::Blocking,
            "Nobody can sign up",
            "Membership signups are switched off, so the portal will not accept a new member \
             however they arrive. Existing members can still sign in."
                .into(),
            Some(ACCESS),
        ),
        "invite" => gap(
            "invite-only",
            Severity::Note,
            "Signups are invite-only",
            "Only somebody you have invited can become a member. Deliberate on a private \
             publication; a dead end on a public one."
                .into(),
            Some(ACCESS),
        ),
        _ => {}
    }

    // 2. Is there anything to buy, and can the portal show it?
    if f.portal_plans.is_empty() && f.signup_access != "none" {
        gap(
            "portal-no-plans",
            Severity::Blocking,
            "The signup portal offers no plan",
            "Signups are open but the portal has no plan selected — free, monthly or yearly — \
             so there is nothing for a visitor to choose."
                .into(),
            Some(PORTAL),
        );
    }
    if f.paid_tiers > 0 && f.visible_paid_tiers == 0 {
        gap(
            "tiers-hidden",
            Severity::Blocking,
            "Every paid tier is hidden",
            format!(
                "{} active, and none of them is visible in the portal, so a visitor is never \
                 shown a way to pay.",
                countable(f.paid_tiers, "paid tier"),
            ),
            Some(TIERS),
        );
    }

    // 3. Can they pay?
    if f.sells() && !f.stripe_connected {
        gap(
            "stripe-missing",
            Severity::Blocking,
            "Paid tiers exist but Stripe is not connected",
            format!(
                "{} on offer and no payment processor behind them. A visitor who picks one \
                 reaches a checkout that cannot charge them.",
                countable(f.paid_tiers, "paid tier"),
            ),
            Some(STRIPE),
        );
    }
    if f.stripe_connected && f.paid_tiers == 0 {
        gap(
            "stripe-idle",
            Severity::Opportunity,
            "Stripe is connected but nothing is for sale",
            "Payments are set up and there is no active paid tier to charge for. One tier is \
             enough to start."
                .into(),
            Some(TIERS),
        );
    }
    if f.stripe_connected && f.paid_tiers > 0 && !f.portal_sells() {
        gap(
            "portal-free-only",
            Severity::Opportunity,
            "The portal only offers the free plan",
            "Paid tiers exist and Stripe is connected, but the portal shows the free plan \
             alone — the paid prices are never put in front of a visitor."
                .into(),
            Some(PORTAL),
        );
    }

    // 4. Will they hear from you again?
    if f.newsletters == 0 {
        gap(
            "no-newsletter",
            Severity::Opportunity,
            "There is no newsletter",
            "Members have no way to be sent anything, so a signup is the end of the \
             relationship rather than the start of it."
                .into(),
            Some(("Newsletters", "/ghost/settings/newsletters/")),
        );
    }
    if f.email_disabled > 0 {
        gap(
            "email-disabled",
            Severity::Note,
            "Some members receive no email",
            format!(
                "{} had email switched off after messages to them failed. They stay members \
                 and stop hearing from you.",
                countable(f.email_disabled, "member"),
            ),
            Some(("Members", "/ghost/members/?filter=all")),
        );
    }

    // 5. Growth that is set up and unused.
    if f.sells() && f.stripe_connected && f.active_offers == 0 {
        gap(
            "no-offers",
            Severity::Note,
            "No offers are running",
            "A discount or a trial gives a reason to decide today rather than later. There is \
             no active offer."
                .into(),
            Some(OFFERS),
        );
    }
    if f.stripe_connected && !f.donations_on {
        gap(
            "no-tips",
            Severity::Note,
            "Tips and donations are off",
            "Stripe is connected, so one-off support costs nothing to switch on and reaches \
             readers who will not take a subscription."
                .into(),
            Some(("Tips & donations", "/ghost/settings/memberships/#tips")),
        );
    }
    if f.comments_enabled == "off" {
        gap(
            "comments-off",
            Severity::Note,
            "Comments are off",
            "Members cannot reply to anything they are sent, which is one of the few reasons \
             a free member comes back."
                .into(),
            Some(("Comments", "/ghost/settings/memberships/#commenting")),
        );
    }

    // 6. Observations with no switch behind them. Thresholds rather than
    //    "> 0" so a site in its first week is not told off for being new.
    if f.members == 0 {
        gap(
            "no-members",
            Severity::Note,
            "No members yet",
            "Nothing here is wrong — there is simply nobody to count. The numbers above start \
             moving with the first signup."
                .into(),
            None,
        );
    } else if f.supported() == 0 && f.members >= PAYING_EXPECTED_FROM && f.sells() && f.stripe_connected {
        gap(
            "none-paying",
            Severity::Opportunity,
            "Nobody has paid yet",
            format!(
                "{} and no paid subscription. The machinery works; what is missing is a \
                 reason, which is usually a tier page or an offer rather than a setting.",
                countable(f.members, "member"),
            ),
            Some(OFFERS),
        );
    }
    if f.cancelled > 0 && f.cancelled >= f.upgraded {
        gap(
            "churn",
            Severity::Opportunity,
            "Cancellations are outpacing upgrades",
            format!(
                "{} cancelled against {} in this window. Churn above new business shrinks \
                 revenue even while the member count grows.",
                countable(f.cancelled, "subscription"),
                countable(f.upgraded, "upgrade"),
            ),
            None,
        );
    }

    gaps.sort_by_key(|g| g.severity);
    gaps
}

/// How many members a site needs before "nobody is paying" is a finding
/// rather than a fact about a new site.
///
/// Twenty is a judgement, and it is the kind of judgement that belongs in a
/// named constant in a tested file: below it the sample is too small to mean
/// anything, and a report that scolds a site with three members teaches its
/// owner to ignore the report.
pub const PAYING_EXPECTED_FROM: i64 = 20;

/// "1 member" / "3 members", with an `s` on the noun.
///
/// A local copy rather than a call into `ghost-admin-ui`'s `countable`: this
/// crate is below that one, the gap details are built here so they can be
/// asserted here, and the alternative is returning a format string for the
/// admin to fill in.
fn countable(n: i64, noun: &str) -> String {
    let mut out = String::new();
    let _ = write!(out, "{n} {noun}");
    if n != 1 {
        out.push('s');
    }
    out
}

// ---------------------------------------------------------------------------
// the arithmetic
// ---------------------------------------------------------------------------

/// Paid conversion as a percentage, or `None` when there is nothing to divide
/// by.
///
/// `None` rather than `0.0` on purpose. A site with no members has no
/// conversion rate; printing `0%` says it tried and failed, which is a
/// different and wrong statement.
pub fn conversion_percent(supported: i64, members: i64) -> Option<f64> {
    if members <= 0 {
        return None;
    }
    Some(supported as f64 * 100.0 / members as f64)
}

/// Average revenue per paying member, in minor units.
pub fn arpu_cents(mrr_cents: i64, paying: i64) -> Option<i64> {
    if paying <= 0 {
        return None;
    }
    Some(mrr_cents / paying)
}

/// MRR × 12, in minor units: the annual run rate.
pub fn annual_run_rate_cents(mrr_cents: i64) -> i64 {
    mrr_cents.saturating_mul(12)
}

/// Change between this window and the one before it, as a percentage.
///
/// `None` when the previous window was empty: a rise from nothing is not a
/// percentage, and "+∞%" or "+100%" would both be inventions. The screen
/// shows the count without an arrow in that case.
pub fn growth_percent(now: i64, before: i64) -> Option<f64> {
    if before <= 0 {
        return None;
    }
    Some((now - before) as f64 * 100.0 / before as f64)
}

/// The reporting window, under this module's own names.
///
/// The reducer itself is [`crate::windows`], shared with the Newsletters
/// module: the offered list and the reduction of `?days=` have to be the same
/// list, and two copies would be two chances for a forged value to be
/// believed in one of them. Re-exported rather than wrapped so there is no
/// second function to keep in step.
pub use crate::windows::{days as window_days, key as window_key, OFFERED as WINDOWS};

/// What `members_signup_access` means, in words.
pub fn signup_access_label(value: &str) -> &'static str {
    match value {
        "none" => "Closed",
        "invite" => "Invite only",
        _ => "Open to all",
    }
}

/// What a tier's `visibility` means for a visitor.
///
/// `products.visibility` is Ghost's enum and the only two values it writes
/// are `public` and `none`. Anything else arrived through an import or a hand
/// edit, and reads as hidden — which is the safe direction, because claiming
/// a tier is visible when nothing shows it is the mistake that costs money.
pub fn visibility_label(value: &str) -> &'static str {
    if value == "public" {
        "Visible"
    } else {
        "Hidden"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A funnel with everything in place, as the baseline the tests perturb.
    fn healthy() -> Funnel {
        Funnel {
            members: 400,
            free: 340,
            paid: 55,
            comped: 5,
            email_disabled: 0,
            joined: 40,
            joined_before: 30,
            upgraded: 6,
            cancelled: 2,
            mrr_cents: 275_000,
            currency: "usd".into(),
            paid_tiers: 2,
            visible_paid_tiers: 2,
            active_offers: 1,
            newsletters: 1,
            signup_access: "all".into(),
            portal_plans: vec!["free".into(), "monthly".into(), "yearly".into()],
            stripe_connected: true,
            comments_enabled: "all".into(),
            donations_on: true,
            default_visibility: "public".into(),
        }
    }

    fn codes(f: &Funnel) -> Vec<&'static str> {
        audit(f).into_iter().map(|g| g.code).collect()
    }

    #[test]
    fn a_funnel_with_everything_in_place_has_nothing_to_report() {
        assert!(audit(&healthy()).is_empty(), "{:?}", codes(&healthy()));
    }

    #[test]
    fn closed_signups_are_blocking_and_come_first() {
        let mut f = healthy();
        f.signup_access = "none".into();
        let gaps = audit(&f);
        assert_eq!(gaps[0].code, "signups-closed");
        assert_eq!(gaps[0].severity, Severity::Blocking);
    }

    #[test]
    fn switching_signups_off_does_not_also_complain_about_stripe() {
        // The site is not selling, so nothing about the shop is a finding.
        // Without `sells()` this reported a missing processor on a site that
        // has deliberately closed its doors.
        let mut f = healthy();
        f.signup_access = "none".into();
        f.stripe_connected = false;
        let codes = codes(&f);
        assert!(codes.contains(&"signups-closed"));
        assert!(!codes.contains(&"stripe-missing"), "{codes:?}");
    }

    #[test]
    fn paid_tiers_with_no_stripe_are_blocking_and_name_the_count() {
        let mut f = healthy();
        f.stripe_connected = false;
        f.active_offers = 0;
        f.donations_on = false;
        let gaps = audit(&f);
        let stripe = gaps.iter().find(|g| g.code == "stripe-missing").expect("no stripe finding");
        assert_eq!(stripe.severity, Severity::Blocking);
        assert!(stripe.detail.starts_with("2 paid tiers"), "{}", stripe.detail);
        // And the two notes that only make sense with Stripe connected are
        // not raised: offering a discount through a processor that is not
        // there is not advice.
        let codes: Vec<_> = gaps.iter().map(|g| g.code).collect();
        assert!(!codes.contains(&"no-offers"), "{codes:?}");
        assert!(!codes.contains(&"no-tips"), "{codes:?}");
    }

    #[test]
    fn tiers_that_exist_but_are_hidden_are_blocking() {
        let mut f = healthy();
        f.visible_paid_tiers = 0;
        let gaps = audit(&f);
        let hidden = gaps.iter().find(|g| g.code == "tiers-hidden").expect("no hidden finding");
        assert_eq!(hidden.severity, Severity::Blocking);
    }

    #[test]
    fn a_portal_with_no_plan_is_blocking_but_only_while_signups_are_open() {
        let mut f = healthy();
        f.portal_plans = vec![];
        assert!(codes(&f).contains(&"portal-no-plans"));

        f.signup_access = "none".into();
        assert!(!codes(&f).contains(&"portal-no-plans"));
    }

    #[test]
    fn a_portal_offering_only_the_free_plan_is_an_opportunity_not_a_blockage() {
        // Nothing is broken: a visitor can join. What is missing is the
        // chance to pay, and that reading matters because the fix is a
        // choice rather than a repair.
        let mut f = healthy();
        f.portal_plans = vec!["free".into()];
        let gaps = audit(&f);
        let g = gaps.iter().find(|g| g.code == "portal-free-only").expect("no portal finding");
        assert_eq!(g.severity, Severity::Opportunity);
    }

    #[test]
    fn the_gaps_come_back_worst_first() {
        let mut f = healthy();
        f.signup_access = "invite".into(); // Note
        f.visible_paid_tiers = 0; // Blocking
        f.newsletters = 0; // Opportunity
        let severities: Vec<_> = audit(&f).into_iter().map(|g| g.severity).collect();
        let mut sorted = severities.clone();
        sorted.sort();
        assert_eq!(severities, sorted, "{severities:?}");
    }

    #[test]
    fn a_brand_new_site_is_told_it_is_new_rather_than_told_off() {
        let f = Funnel {
            members: 0,
            signup_access: "all".into(),
            portal_plans: vec!["free".into()],
            newsletters: 1,
            comments_enabled: "all".into(),
            ..Funnel::default()
        };
        let codes = codes(&f);
        assert!(codes.contains(&"no-members"), "{codes:?}");
        assert!(!codes.contains(&"none-paying"), "{codes:?}");
    }

    #[test]
    fn nobody_paying_is_only_a_finding_once_there_are_enough_members_to_mean_it() {
        let mut f = healthy();
        f.paid = 0;
        f.comped = 0;
        f.members = PAYING_EXPECTED_FROM - 1;
        assert!(!codes(&f).contains(&"none-paying"));

        f.members = PAYING_EXPECTED_FROM;
        assert!(codes(&f).contains(&"none-paying"));
    }

    #[test]
    fn churn_is_reported_when_it_matches_or_beats_the_upgrades() {
        let mut f = healthy();
        f.cancelled = 6;
        f.upgraded = 6;
        assert!(codes(&f).contains(&"churn"));

        f.upgraded = 7;
        assert!(!codes(&f).contains(&"churn"));

        // And a quiet window with no cancellations at all is not churn.
        f.cancelled = 0;
        f.upgraded = 0;
        assert!(!codes(&f).contains(&"churn"));
    }

    #[test]
    fn every_gap_has_a_unique_code_so_an_anchor_points_at_one_row() {
        // Perturb everything at once and check the codes are still distinct —
        // the codes are rendered as element ids.
        let f = Funnel {
            members: 500,
            email_disabled: 3,
            cancelled: 9,
            upgraded: 1,
            paid_tiers: 2,
            visible_paid_tiers: 0,
            active_offers: 0,
            newsletters: 0,
            signup_access: "invite".into(),
            portal_plans: vec![],
            stripe_connected: false,
            comments_enabled: "off".into(),
            donations_on: false,
            ..Funnel::default()
        };
        let codes = codes(&f);
        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(codes.len(), unique.len(), "{codes:?}");
        assert!(codes.len() >= 6, "expected a pile of findings, got {codes:?}");
    }

    #[test]
    fn a_fix_link_always_points_inside_the_admin() {
        // Rule 8: never build a link out of anything but a literal. These are
        // all literals; this test is here so a later edit that interpolates
        // one is caught.
        let f = Funnel {
            members: 500,
            email_disabled: 3,
            paid_tiers: 2,
            visible_paid_tiers: 0,
            newsletters: 0,
            signup_access: "invite".into(),
            portal_plans: vec![],
            comments_enabled: "off".into(),
            ..Funnel::default()
        };
        for gap in audit(&f) {
            if let Some((_, path)) = gap.fix {
                assert!(path.starts_with("/ghost/"), "{} links to {path}", gap.code);
                assert!(!path.starts_with("//"), "{} links to {path}", gap.code);
            }
        }
    }

    #[test]
    fn a_rate_with_nothing_to_divide_by_is_absent_rather_than_zero() {
        assert_eq!(conversion_percent(0, 0), None);
        assert_eq!(arpu_cents(1_000, 0), None);
        assert_eq!(growth_percent(10, 0), None);
    }

    #[test]
    fn the_rates_are_the_obvious_arithmetic() {
        assert_eq!(conversion_percent(25, 100), Some(25.0));
        assert_eq!(arpu_cents(100_000, 20), Some(5_000));
        assert_eq!(growth_percent(150, 100), Some(50.0));
        assert_eq!(growth_percent(50, 100), Some(-50.0));
        assert_eq!(annual_run_rate_cents(275_000), 3_300_000);
    }

    #[test]
    fn a_forged_window_collapses_to_the_default() {
        assert_eq!(window_days(Some("7")), 7);
        assert_eq!(window_days(Some("90")), 90);
        assert_eq!(window_days(Some(" 30 ")), 30);
        assert_eq!(window_days(Some("3650")), 30);
        assert_eq!(window_days(Some("'; DROP TABLE members; --")), 30);
        assert_eq!(window_days(None), 30);
        // And every offered window round-trips through its key, or the
        // segmented bar would mark the wrong tab active.
        for (key, days, _) in WINDOWS {
            assert_eq!(window_key(*days), *key);
        }
    }

    #[test]
    fn an_unknown_tier_visibility_reads_as_hidden() {
        assert_eq!(visibility_label("public"), "Visible");
        assert_eq!(visibility_label("none"), "Hidden");
        assert_eq!(visibility_label("members"), "Hidden");
        assert_eq!(visibility_label(""), "Hidden");
    }

    #[test]
    fn countable_puts_the_s_on_everything_but_one() {
        assert_eq!(countable(1, "paid tier"), "1 paid tier");
        assert_eq!(countable(0, "paid tier"), "0 paid tiers");
        assert_eq!(countable(2, "paid tier"), "2 paid tiers");
    }
}
