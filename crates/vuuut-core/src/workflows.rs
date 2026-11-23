//! Workflow & Editorial Management: the rules.
//!
//! An editorial pipeline laid over the site's own posts and pages. A draft
//! sits in one of the site's **stages** (Idea → Writing → In review →
//! Approved by default, all renameable); scheduled and published entries are
//! shown in two columns that are not stages at all but the post's own
//! status, so the board can never disagree with the post about whether it is
//! live.
//!
//! Pure — no database, no HTTP. What lives here, each a decision that can be
//! wrong and is therefore unit-tested:
//!
//! * [`Settings`] — the stages, the approval gate, the checklist, each read
//!   with a safe default. A stored stage list is rebuilt rather than
//!   trusted: keys are slugs, unique, never the two reserved column names,
//!   between two and ten of them.
//! * [`may_move`] — who may move what where. Editors and above move
//!   anything; writers (Authors, Contributors) move their own work, and
//!   never into or out of the approval stage — submitting is theirs,
//!   approving is not.
//! * [`publish_gate`] — whether publishing is allowed for an entry in a
//!   given stage. The editor's save path asks this before anything goes
//!   live; it is off by default, so installing the tool changes nothing.
//! * [`due`] and [`auto_checks`] — what the board flags.
//! * [`month_grid`] — the calendar's weeks.

use chrono::{Datelike, Duration, NaiveDate};

// ---------------------------------------------------------------------------
// stages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stage {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub label: String,
    /// One of [`COLORS`].
    #[serde(default)]
    pub color: String,
    /// Work-in-progress limit; 0 is none. Exceeding it is a warning on the
    /// board, never a refusal — a limit that blocks work gets switched off.
    #[serde(default)]
    pub limit: i64,
}

/// The tints the admin already draws with.
pub const COLORS: [&str; 8] = ["slate", "blue", "teal", "green", "amber", "purple", "pink", "red"];
/// Columns that are a post's own status, not a stage. No stage may use them.
pub const RESERVED: [&str; 2] = ["scheduled", "published"];
pub const MIN_STAGES: usize = 2;
pub const MAX_STAGES: usize = 10;
pub const MAX_LABEL: usize = 40;

pub fn default_stages() -> Vec<Stage> {
    let s = |key: &str, label: &str, color: &str| Stage { key: key.into(), label: label.into(), color: color.into(), limit: 0 };
    vec![
        s("idea", "Ideas", "slate"),
        s("writing", "Writing", "blue"),
        s("review", "In review", "amber"),
        s("ready", "Approved", "green"),
    ]
}

pub fn slug(raw: &str) -> String {
    let s: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-").chars().take(30).collect()
}

/// Rebuild a stage list from what was stored or posted. Bad entries are
/// dropped, labels bounded, colours reduced to the palette, keys made
/// unique. Fewer than two usable stages is not a pipeline — the defaults
/// come back.
pub fn normalize_stages(stages: Vec<Stage>) -> Vec<Stage> {
    let mut out: Vec<Stage> = Vec::new();
    for st in stages {
        let label: String = st.label.trim().chars().filter(|c| !c.is_control()).take(MAX_LABEL).collect();
        if label.is_empty() {
            continue;
        }
        let mut key = slug(if st.key.trim().is_empty() { &label } else { &st.key });
        if key.is_empty() || RESERVED.contains(&key.as_str()) {
            key = format!("stage-{}", out.len() + 1);
        }
        let mut unique = key.clone();
        let mut n = 2;
        while out.iter().any(|o| o.key == unique) {
            unique = format!("{key}-{n}");
            n += 1;
        }
        let color = if COLORS.contains(&st.color.as_str()) { st.color } else { COLORS[out.len() % COLORS.len()].to_string() };
        out.push(Stage { key: unique, label, color, limit: st.limit.clamp(0, 999) });
        if out.len() == MAX_STAGES {
            break;
        }
    }
    if out.len() < MIN_STAGES { default_stages() } else { out }
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_STAGES: &str = "workflow_stages";
pub const SETTING_REQUIRE_APPROVAL: &str = "workflow_require_approval";
pub const SETTING_APPROVAL_STAGE: &str = "workflow_approval_stage";
pub const SETTING_ADMINS_BYPASS: &str = "workflow_admins_bypass";
pub const SETTING_CHECKLIST: &str = "workflow_checklist";

pub const MAX_CHECKS: usize = 12;
pub const MAX_CHECK: usize = 80;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub stages: Vec<Stage>,
    /// Publishing needs the entry to have reached the approval stage.
    pub require_approval: bool,
    /// The stage that counts as approved. Always one of `stages`.
    pub approval_stage: String,
    /// Administrators and the Owner may publish without approval.
    pub admins_bypass: bool,
    /// The manual checklist every entry carries, one item per line.
    pub checklist: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        let stages = get(SETTING_STAGES)
            .and_then(|raw| serde_json::from_str::<Vec<Stage>>(raw.trim()).ok())
            .map(normalize_stages)
            .unwrap_or_else(default_stages);
        let wanted = get(SETTING_APPROVAL_STAGE).map(|s| s.trim().trim_matches('"').to_string()).unwrap_or_default();
        // An approval stage that no longer exists falls back to the last
        // stage — the one nearest publication — never to "nothing".
        let approval_stage = if stages.iter().any(|s| s.key == wanted) {
            wanted
        } else {
            stages.last().map(|s| s.key.clone()).unwrap_or_default()
        };
        Settings {
            approval_stage,
            stages,
            require_approval: flag(SETTING_REQUIRE_APPROVAL, false),
            admins_bypass: flag(SETTING_ADMINS_BYPASS, true),
            checklist: parse_checklist(&get(SETTING_CHECKLIST).unwrap_or_else(|| DEFAULT_CHECKLIST.into())),
        }
    }

    pub fn stage(&self, key: &str) -> Option<&Stage> {
        self.stages.iter().find(|s| s.key == key)
    }

    pub fn index(&self, key: &str) -> Option<usize> {
        self.stages.iter().position(|s| s.key == key)
    }

    /// Where a draft with no workflow record sits: the second stage
    /// (writing), because a draft that exists has been started. Ideas are
    /// things somebody put on the board on purpose.
    pub fn default_stage(&self) -> &str {
        self.stages.get(1).or(self.stages.first()).map(|s| s.key.as_str()).unwrap_or("")
    }

    /// A stored key that is no longer a stage reads as the default.
    pub fn resolve<'a>(&'a self, key: Option<&'a str>) -> &'a str {
        match key {
            Some(k) if self.stage(k).is_some() => k,
            _ => self.default_stage(),
        }
    }
}

pub const DEFAULT_CHECKLIST: &str = "Facts checked\nLinks checked\nRead aloud once";

pub fn parse_checklist(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.replace("\\n", "\n").lines() {
        let l: String = line.trim().chars().filter(|c| !c.is_control()).take(MAX_CHECK).collect();
        if !l.is_empty() && !out.contains(&l) {
            out.push(l);
        }
        if out.len() == MAX_CHECKS {
            break;
        }
    }
    out
}

/// A checklist item's stable key, so renaming "Facts checked" to "Facts
/// verified" resets that tick rather than silently carrying it over.
pub fn check_key(label: &str) -> String {
    slug(label)
}

// ---------------------------------------------------------------------------
// who may do what
// ---------------------------------------------------------------------------

/// The actor, as far as these rules care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Who {
    /// Editor, Administrator or Owner.
    pub editor: bool,
    /// Administrator or Owner.
    pub admin: bool,
    /// The entry is theirs (they are one of its authors, or it is assigned
    /// to them).
    pub owns: bool,
}

/// Whether `who` may move an entry from one stage to another.
pub fn may_move(s: &Settings, who: Who, from: &str, to: &str) -> Result<(), &'static str> {
    let Some(to_i) = s.index(to) else { return Err("That is not one of this site's stages.") };
    if who.editor {
        return Ok(());
    }
    if !who.owns {
        return Err("Only an editor can move somebody else's work.");
    }
    let approval = s.index(&s.approval_stage).unwrap_or(s.stages.len());
    let from_i = s.index(from).unwrap_or(0);
    if to_i >= approval {
        return Err("Approving is for editors. Move it to review and an editor will take it from there.");
    }
    if from_i >= approval {
        return Err("This has been approved. Ask an editor to send it back.");
    }
    Ok(())
}

/// Assigning work, setting due dates and changing priority are editors'
/// decisions; a writer may only update their own entry's due date.
pub fn may_assign(who: Who) -> bool {
    who.editor
}

/// Whether publishing an entry in `stage` is allowed. `None` means allowed;
/// `Some(message)` is what the editor shows instead of publishing.
pub fn publish_gate(s: &Settings, stage: Option<&str>, admin: bool) -> Option<String> {
    if !s.require_approval || (admin && s.admins_bypass) {
        return None;
    }
    let approval = s.index(&s.approval_stage)?;
    let at = stage.and_then(|k| s.index(k));
    match at {
        Some(i) if i >= approval => None,
        _ => {
            let label = s.stage(&s.approval_stage).map(|st| st.label.as_str()).unwrap_or("approved");
            Some(format!(
                "This needs approval before it can be published. Move it to “{label}” on the editorial board first."
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// priorities, due dates, checks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Normal,
    High,
    Urgent,
}

impl Priority {
    pub const ALL: [Priority; 4] = [Priority::Urgent, Priority::High, Priority::Normal, Priority::Low];
    pub fn key(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }
    pub fn from_key(k: &str) -> Priority {
        match k.trim() {
            "low" => Priority::Low,
            "high" => Priority::High,
            "urgent" => Priority::Urgent,
            _ => Priority::Normal,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Priority::Low => "Low",
            Priority::Normal => "Normal",
            Priority::High => "High",
            Priority::Urgent => "Urgent",
        }
    }
    pub fn tint(self) -> &'static str {
        match self {
            Priority::Low => "slate",
            Priority::Normal => "blue",
            Priority::High => "amber",
            Priority::Urgent => "red",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Overdue,
    Today,
    Soon,
    Later,
}

impl Due {
    pub fn label(self) -> &'static str {
        match self {
            Due::Overdue => "Overdue",
            Due::Today => "Due today",
            Due::Soon => "Due soon",
            Due::Later => "Due",
        }
    }
}

/// How a due date (`YYYY-MM-DD`) stands against today. "Soon" is within
/// three days.
pub fn due(due_on: &str, today: NaiveDate) -> Option<Due> {
    let d = NaiveDate::parse_from_str(due_on.get(..10)?, "%Y-%m-%d").ok()?;
    Some(match (d - today).num_days() {
        n if n < 0 => Due::Overdue,
        0 => Due::Today,
        1..=3 => Due::Soon,
        _ => Due::Later,
    })
}

/// A due date from a form: a real `YYYY-MM-DD`, or nothing.
pub fn parse_date(raw: &str) -> Option<String> {
    let v = raw.trim().get(..10)?;
    NaiveDate::parse_from_str(v, "%Y-%m-%d").ok().map(|d| d.format("%Y-%m-%d").to_string())
}

/// What is known about an entry without anybody ticking anything.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub has_title: bool,
    pub words: i64,
    pub has_feature_image: bool,
    pub has_excerpt: bool,
    pub has_meta_description: bool,
    pub tags: i64,
}

/// The automatic half of the checklist: each a label and whether it holds.
pub fn auto_checks(f: &Facts) -> Vec<(&'static str, bool)> {
    vec![
        ("Has a title", f.has_title),
        ("At least 300 words", f.words >= 300),
        ("Feature image", f.has_feature_image),
        ("Excerpt or meta description", f.has_excerpt || f.has_meta_description),
        ("Tagged", f.tags > 0),
    ]
}

// ---------------------------------------------------------------------------
// the calendar
// ---------------------------------------------------------------------------

/// `YYYY-MM` from a query, or the month containing `today`.
pub fn month_of(raw: Option<&str>, today: NaiveDate) -> NaiveDate {
    raw.and_then(|m| NaiveDate::parse_from_str(&format!("{}-01", m.trim().get(..7)?), "%Y-%m-%d").ok())
        .filter(|d| (1970..=2200).contains(&d.year()))
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today))
}

/// The weeks shown for a month, Monday first, padded with the neighbouring
/// months' days so every row has seven.
pub fn month_grid(first: NaiveDate) -> Vec<[NaiveDate; 7]> {
    let start = first - Duration::days(first.weekday().num_days_from_monday() as i64);
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    }
    .unwrap_or(first);
    let mut weeks = Vec::new();
    let mut day = start;
    while day < next || weeks.is_empty() {
        let mut week = [day; 7];
        for (i, slot) in week.iter_mut().enumerate() {
            *slot = day + Duration::days(i as i64);
        }
        weeks.push(week);
        day += Duration::days(7);
    }
    weeks
}

pub fn shift_month(first: NaiveDate, by: i32) -> NaiveDate {
    let total = first.year() * 12 + first.month0() as i32 + by;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).unwrap_or(first)
}

pub const MAX_NOTE: usize = 2_000;
pub const MAX_IDEA: usize = 200;

pub fn clean(raw: &str, max: usize) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        Settings::read(|k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string()))
    }
    const WRITER: Who = Who { editor: false, admin: false, owns: true };
    const STRANGER: Who = Who { editor: false, admin: false, owns: false };
    const EDITOR: Who = Who { editor: true, admin: false, owns: false };

    #[test]
    fn the_defaults_change_nothing_about_publishing() {
        let s = Settings::default();
        assert_eq!(s.stages.len(), 4);
        assert_eq!(s.approval_stage, "ready");
        assert!(!s.require_approval && s.admins_bypass);
        assert_eq!(publish_gate(&s, None, false), None, "the gate is off by default");
        assert_eq!(s.default_stage(), "writing");
        assert_eq!(s.checklist.len(), 3);
    }

    #[test]
    fn a_stored_stage_list_is_rebuilt_not_trusted() {
        let raw = r#"[{"key":"published","label":"Live","color":"neon"},{"key":"x","label":"  "},
            {"key":"Draft Copy","label":"Drafting","color":"blue","limit":-4},{"key":"draft-copy","label":"Again"}]"#;
        let s = settings(&[(SETTING_STAGES, raw), (SETTING_APPROVAL_STAGE, "gone")]);
        let keys: Vec<&str> = s.stages.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["stage-1", "draft-copy", "draft-copy-2"]);
        assert!(COLORS.contains(&s.stages[0].color.as_str()));
        assert_eq!(s.stages[1].limit, 0);
        assert_eq!(s.approval_stage, "draft-copy-2", "a missing approval stage falls back to the last");
        let one = settings(&[(SETTING_STAGES, r#"[{"key":"a","label":"Only"}]"#)]);
        assert_eq!(one.stages, default_stages(), "one stage is not a pipeline");
        assert_eq!(settings(&[(SETTING_STAGES, "not json")]).stages, default_stages());
    }

    #[test]
    fn writers_submit_and_editors_approve() {
        let s = Settings::default();
        assert!(may_move(&s, WRITER, "writing", "review").is_ok());
        assert!(may_move(&s, WRITER, "review", "idea").is_ok());
        assert!(may_move(&s, WRITER, "review", "ready").is_err(), "approving is not a writer's");
        assert!(may_move(&s, WRITER, "ready", "writing").is_err(), "un-approving is not either");
        assert!(may_move(&s, STRANGER, "writing", "review").is_err());
        assert!(may_move(&s, EDITOR, "idea", "ready").is_ok());
        assert!(may_move(&s, EDITOR, "idea", "published").is_err(), "a status is not a stage");
        assert!(!may_assign(WRITER) && may_assign(EDITOR));
    }

    #[test]
    fn the_gate_holds_until_approval_unless_an_admin_may_bypass() {
        let s = settings(&[(SETTING_REQUIRE_APPROVAL, "true")]);
        assert!(publish_gate(&s, Some("review"), false).unwrap().contains("Approved"));
        assert!(publish_gate(&s, None, false).is_some(), "no record is not approved");
        assert_eq!(publish_gate(&s, Some("ready"), false), None);
        assert_eq!(publish_gate(&s, Some("review"), true), None, "admins bypass by default");
        let strict = settings(&[(SETTING_REQUIRE_APPROVAL, "true"), (SETTING_ADMINS_BYPASS, "false")]);
        assert!(publish_gate(&strict, Some("review"), true).is_some());
    }

    #[test]
    fn due_dates_are_judged_against_today() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        assert_eq!(due("2026-11-20", today), Some(Due::Overdue));
        assert_eq!(due("2026-11-21", today), Some(Due::Today));
        assert_eq!(due("2026-11-24", today), Some(Due::Soon));
        assert_eq!(due("2026-12-24", today), Some(Due::Later));
        assert_eq!(due("nonsense", today), None);
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date(" 2026-02-28 "), Some("2026-02-28".into()));
    }

    #[test]
    fn a_month_is_whole_weeks_starting_monday() {
        let nov = month_of(Some("2026-11"), NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());
        let weeks = month_grid(nov);
        assert_eq!(weeks[0][0], NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
        assert_eq!(weeks.len(), 6);
        assert!(weeks.iter().flatten().any(|d| *d == NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()));
        assert_eq!(shift_month(nov, 2), NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        assert_eq!(shift_month(nov, -11), NaiveDate::from_ymd_opt(2025, 12, 1).unwrap());
        assert_eq!(month_of(Some("<script>"), NaiveDate::from_ymd_opt(2026, 11, 21).unwrap()), nov);
    }

    #[test]
    fn checks_and_checklists_are_bounded() {
        let f = Facts { has_title: true, words: 320, has_meta_description: true, ..Facts::default() };
        let checks = auto_checks(&f);
        assert_eq!(checks.iter().filter(|c| c.1).count(), 3);
        assert_eq!(parse_checklist("a\na\n\n b \n").len(), 2);
        assert_eq!(check_key("Facts checked!"), "facts-checked");
    }
}
