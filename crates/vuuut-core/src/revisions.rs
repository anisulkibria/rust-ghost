//! When a post revision is kept, and how many are kept — Ghost's rule, with
//! Ghost's two numbers.
//!
//! Ported from `core/server/lib/post-revisions.js` and the two constants in
//! `core/server/models/post.js` that configure it:
//!
//! ```js
//! const POST_REVISIONS_COUNT = 25;
//! const POST_REVISIONS_INTERVAL_MS = 10 * 60 * 1000; // 10 minutes
//! ```
//!
//! ## Why the count is meaningless without the interval
//!
//! The editor autosaves three seconds after typing stops. Before this module
//! existed, every one of those saves wrote a full snapshot, and nothing ever
//! deleted one — so the table grew without bound and a post written over an
//! afternoon carried hundreds of near-identical versions.
//!
//! Capping that at 25 on its own would have made the feature *worse*, not
//! better: twenty-five autosaves is about seventy-five seconds of typing, so
//! the whole history would be the last minute of work and yesterday's draft
//! would be gone. The cap only means anything alongside the rule that decides
//! what fills it, which is why both are here and why [`MAX_REVISIONS`] is not
//! usable without [`should_generate`].
//!
//! ## The rule
//!
//! A revision is always kept when there is no history yet, when a post is
//! published, and when a published post is pulled back to a draft — those are
//! the three moments somebody later wants to return to. Otherwise one is kept
//! only if something a reader would notice has changed (the body, the title,
//! the feature image or the excerpt) *and* either the save was asked for
//! explicitly, or ten minutes have passed since the last one was kept.
//!
//! Note what is deliberately *not* in that list of four: tags, authors, SEO
//! fields, code injection. A snapshot carries them and a restore puts them
//! back, but changing one does not by itself start a new version — that is
//! Ghost's choice, and following it is what keeps the 25 slots holding
//! twenty-five recognisably different drafts.

/// Ghost's `POST_REVISIONS_COUNT`: how many revisions one post keeps.
pub const MAX_REVISIONS: i64 = 25;

/// Ghost's `POST_REVISIONS_INTERVAL_MS`: how long a run of background saves
/// is folded into one revision.
pub const INTERVAL_MS: i64 = 10 * 60 * 1000;

/// Why a revision was kept. The strings are Ghost's own, stored in
/// `post_revisions.reason`, so a database moved between the two reads the
/// same either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The post had no history yet.
    Initial,
    /// A published post was pulled back to a draft.
    Unpublished,
    /// The post was published.
    Published,
    /// Somebody pressed Save (or cmd-S, or left the editor).
    ExplicitSave,
    /// A background save, more than [`INTERVAL_MS`] after the last revision.
    BackgroundSave,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Initial => "initial_revision",
            Reason::Unpublished => "unpublished",
            Reason::Published => "published",
            Reason::ExplicitSave => "explicit_save",
            Reason::BackgroundSave => "background_save",
        }
    }
}

/// What the caller knows about the save being made.
#[derive(Debug, Clone, Copy, Default)]
pub struct Save {
    /// Whether any of the four fields Ghost watches differs from what is
    /// stored: the body, the title, the feature image, the excerpt.
    pub content_changed: bool,
    /// Whether this post has any revision yet.
    pub has_revisions: bool,
    /// When the newest revision was taken, in epoch milliseconds.
    pub last_revision_ms: Option<i64>,
    /// A published post becoming a draft.
    pub unpublishing: bool,
    /// The post becoming published.
    pub publishing: bool,
    /// Somebody asked for this save, rather than it being an autosave.
    ///
    /// Ghost's `forceRevision`. A form submitted without JavaScript is an
    /// explicit save too — there is no autosave without the script.
    pub explicit: bool,
}

/// Whether to keep a revision for this save, and why — Ghost's
/// `shouldGenerateRevision`, in its own order.
pub fn should_generate(save: &Save, now_ms: i64) -> Option<Reason> {
    // Nothing to compare against: the first save of a post always leaves a
    // version, or there would be no way back to how it started.
    if !save.has_revisions {
        return Some(Reason::Initial);
    }
    if save.unpublishing {
        return Some(Reason::Unpublished);
    }
    if save.publishing {
        return Some(Reason::Published);
    }
    if !save.content_changed {
        return None;
    }
    if save.explicit {
        return Some(Reason::ExplicitSave);
    }
    // A run of autosaves is folded into one version: the first one past the
    // interval is kept and the rest are not, so ten minutes of typing costs
    // one slot rather than two hundred.
    match save.last_revision_ms {
        Some(last) if now_ms - last > INTERVAL_MS => Some(Reason::BackgroundSave),
        // No timestamp to measure from — treat it as due rather than silently
        // never keeping another.
        None => Some(Reason::BackgroundSave),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn save() -> Save {
        Save {
            content_changed: true,
            has_revisions: true,
            last_revision_ms: Some(NOW - 1000),
            explicit: false,
            ..Default::default()
        }
    }

    #[test]
    fn the_numbers_are_ghosts_own() {
        // `POST_REVISIONS_COUNT` and `POST_REVISIONS_INTERVAL_MS` in
        // core/server/models/post.js. Pinned so a later edit to either has to
        // be a deliberate divergence rather than a typo.
        assert_eq!(MAX_REVISIONS, 25);
        assert_eq!(INTERVAL_MS, 600_000);
    }

    #[test]
    fn the_first_save_always_leaves_a_version() {
        let first = Save { has_revisions: false, content_changed: false, ..Default::default() };
        assert_eq!(should_generate(&first, NOW), Some(Reason::Initial));
    }

    #[test]
    fn publishing_and_unpublishing_always_leave_one() {
        // These are the two moments somebody comes back looking for, so they
        // are kept even when nothing about the text changed.
        let published = Save { publishing: true, content_changed: false, ..save() };
        assert_eq!(should_generate(&published, NOW), Some(Reason::Published));
        let unpublished = Save { unpublishing: true, content_changed: false, ..save() };
        assert_eq!(should_generate(&unpublished, NOW), Some(Reason::Unpublished));
        // Unpublishing is checked first, matching Ghost, so a save that
        // somehow claimed both reports the one that lost content.
        let both = Save { publishing: true, unpublishing: true, ..save() };
        assert_eq!(should_generate(&both, NOW), Some(Reason::Unpublished));
    }

    #[test]
    fn a_save_that_changes_nothing_visible_leaves_no_version() {
        // Ticking a tag or editing an SEO field does not start a new draft.
        // The snapshot carries those and a restore returns them; they just do
        // not get a slot of their own.
        let unchanged = Save { content_changed: false, explicit: true, ..save() };
        assert_eq!(should_generate(&unchanged, NOW), None);
    }

    #[test]
    fn an_explicit_save_always_keeps_a_changed_draft() {
        let explicit = Save { explicit: true, ..save() };
        assert_eq!(should_generate(&explicit, NOW), Some(Reason::ExplicitSave));
    }

    #[test]
    fn a_run_of_autosaves_costs_one_version_per_ten_minutes() {
        // The point of the whole module. The editor autosaves three seconds
        // after typing stops, so without this an afternoon's writing would
        // fill all 25 slots in about a minute.
        let recent = save();
        assert_eq!(should_generate(&recent, NOW), None, "three seconds later: no new version");

        let just_under = Save { last_revision_ms: Some(NOW - INTERVAL_MS), ..save() };
        assert_eq!(should_generate(&just_under, NOW), None, "exactly ten minutes is not yet past it");

        let past = Save { last_revision_ms: Some(NOW - INTERVAL_MS - 1), ..save() };
        assert_eq!(should_generate(&past, NOW), Some(Reason::BackgroundSave));
    }

    #[test]
    fn a_revision_with_no_timestamp_is_treated_as_due() {
        // Rather than never keeping another one, which is the failure mode
        // that would quietly stop the history dead.
        let no_stamp = Save { last_revision_ms: None, ..save() };
        assert_eq!(should_generate(&no_stamp, NOW), Some(Reason::BackgroundSave));
    }

    #[test]
    fn the_reason_strings_match_ghosts() {
        // Stored in `post_revisions.reason`; a database moved between the two
        // implementations has to read the same.
        assert_eq!(Reason::Initial.as_str(), "initial_revision");
        assert_eq!(Reason::Unpublished.as_str(), "unpublished");
        assert_eq!(Reason::Published.as_str(), "published");
        assert_eq!(Reason::ExplicitSave.as_str(), "explicit_save");
        assert_eq!(Reason::BackgroundSave.as_str(), "background_save");
    }
}
