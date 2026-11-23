//! Automation domain rules: what an automation may contain, and when each of
//! its steps becomes due.
//!
//! An automation is a trigger plus an ordered chain of actions. Ghost stores
//! the chain as a directed graph but accepts only one shape today — a single
//! linear path with no branches and no cycles — and
//! [`validate_graph`] is the check that enforces it. Keeping the storage
//! general and the validation strict is Ghost's own choice: the editor draws
//! a flow, and the day branches arrive nothing about the tables has to change.
//!
//! Mirrors `core/server/services/automations/` in Ghost 6.72 —
//! `automations-schemas.ts` for the bounds, `automations-api.ts` for the
//! graph rules, and `util/get-ready-at-for-action.ts` for the timing.

use crate::{GhostError, Result};
use std::collections::{HashMap, HashSet};

/// Ghost's `MAX_AUTOMATION_ACTIONS`.
pub const MAX_ACTIONS: usize = 50;
/// Ghost's `MAX_WAIT_HOURS` — 30 days. The editor presents this in days.
pub const MAX_WAIT_HOURS: i64 = 720;
/// Ghost's soft cap on how many automations a site may have.
pub const MAX_AUTOMATIONS: i64 = 50;
/// `name` is `varchar(191)` in Ghost's schema.
pub const MAX_NAME_LEN: usize = 191;
pub const MAX_DESCRIPTION_LEN: usize = 2000;
/// `email_subject` is `varchar(300)`.
pub const MAX_SUBJECT_LEN: usize = 300;

/// Whether an automation runs, is paused, or has been put away.
///
/// `archived` is not deletion: Ghost keeps the automation and its run history
/// and only takes it off the default list, because the history is the record
/// of emails real people were sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Inactive,
    Archived,
}

impl Status {
    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "active" => Some(Status::Active),
            "inactive" => Some(Status::Inactive),
            "archived" => Some(Status::Archived),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Inactive => "inactive",
            Status::Archived => "archived",
        }
    }
}

/// Which members a signup-triggered automation applies to.
///
/// Ghost's only trigger today is "membership started", so the scope is
/// entirely about *which* membership: free signups, any paid signup, or
/// signups on particular tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerScope {
    Free,
    AllPaid,
    SelectedPaid,
}

impl TriggerScope {
    pub fn parse(s: &str) -> Option<TriggerScope> {
        match s {
            "free" => Some(TriggerScope::Free),
            "all_paid" => Some(TriggerScope::AllPaid),
            "selected_paid" => Some(TriggerScope::SelectedPaid),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TriggerScope::Free => "free",
            TriggerScope::AllPaid => "all_paid",
            TriggerScope::SelectedPaid => "selected_paid",
        }
    }

    /// The member status this scope fires for. Ghost's
    /// `MEMBER_STATUS_BY_TRIGGER_TIER_SCOPE`.
    pub fn member_status(self) -> &'static str {
        match self {
            TriggerScope::Free => "free",
            TriggerScope::AllPaid | TriggerScope::SelectedPaid => "paid",
        }
    }

    /// Human copy, as the editor's trigger node reads.
    pub fn label(self) -> &'static str {
        match self {
            TriggerScope::Free => "Free member signed up",
            TriggerScope::AllPaid => "Paid subscription started",
            TriggerScope::SelectedPaid => "Paid subscription started on selected tiers",
        }
    }
}

/// Ghost's `DEFAULT_WELCOME_EMAIL_AUTOMATIONS`: the two member welcome flows
/// present on every install, seeded paused and empty. The member service
/// recognises them by their reserved slugs (`MEMBER_WELCOME_EMAIL_SLUGS`);
/// user-made automations carry no slug at all.
pub const DEFAULT_WELCOME_AUTOMATIONS: &[(&str, &str, &str, &str)] = &[
    // (name, description, slug, trigger scope)
    (
        "Free member welcome flow",
        "Welcome new free members after they sign up.",
        "member-welcome-email-free",
        "free",
    ),
    (
        "Paid member welcome flow",
        "Welcome new paid members after they start their subscription.",
        "member-welcome-email-paid",
        "all_paid",
    ),
];

/// Whether `slug` belongs to one of the built-in welcome flows — the one
/// automation kind that may not be deleted, because the seeder would bring
/// it back on the next boot.
pub fn is_welcome_email_slug(slug: Option<&str>) -> bool {
    slug.is_some_and(|s| DEFAULT_WELCOME_AUTOMATIONS.iter().any(|(_, _, slug, _)| *slug == s))
}

/// Member statuses that still count as matching a scope at send time.
///
/// Ghost's `MEMBER_WELCOME_EMAIL_ELIGIBLE_STATUSES`: a `comped` or `gift`
/// member counts as paid, so a complimentary subscription does not silently
/// drop someone out of a paid automation halfway through.
pub fn eligible_statuses(member_status: &str) -> &'static [&'static str] {
    match member_status {
        "free" => &["free"],
        _ => &["paid", "comped", "gift"],
    }
}

/// One node in the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    /// Hold the member here for `wait_hours` before moving on.
    Wait { wait_hours: i64 },
    /// Send one email, then move on immediately.
    SendEmail { subject: String, lexical: String },
}

impl ActionKind {
    pub fn type_str(&self) -> &'static str {
        match self {
            ActionKind::Wait { .. } => "wait",
            ActionKind::SendEmail { .. } => "send_email",
        }
    }
}

/// An action as submitted by the editor: a caller-chosen id plus its contents.
///
/// The id comes from the client so that `edges` can reference nodes that do
/// not exist in the database yet — a newly drawn chain is saved in one
/// request, nodes and links together.
#[derive(Debug, Clone)]
pub struct Action {
    pub id: String,
    pub kind: ActionKind,
}

/// A link from one action to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub source_action_id: String,
    pub target_action_id: String,
}

fn invalid(message: &str, property: &str) -> GhostError {
    GhostError::ValidationError {
        message: message.to_string(),
        context: Some(property.to_string()),
    }
}

/// Validate a whole submitted automation, as Ghost's `validateAutomationData`
/// does: shape first, then the stricter rules that only apply once it is live.
pub fn validate(
    status: Status,
    name: &str,
    description: &str,
    actions: &[Action],
    edges: &[Edge],
) -> Result<()> {
    if name.trim().is_empty() {
        return Err(invalid("Automation name is required.", "name"));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(invalid(
            "Automation name must be 191 characters or less.",
            "name",
        ));
    }
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(invalid(
            "Automation description must be 2000 characters or less.",
            "description",
        ));
    }
    if actions.len() > MAX_ACTIONS {
        return Err(invalid(
            "Automations cannot have more than 50 actions.",
            "actions",
        ));
    }
    for action in actions {
        validate_action(action)?;
    }
    // An active automation with no actions would enrol members into nothing
    // and never let them out.
    if status == Status::Active && actions.is_empty() {
        return Err(invalid(
            "Active automations require at least one action.",
            "actions",
        ));
    }
    validate_graph(actions, edges)?;
    validate_active_emails(status, actions)
}

fn validate_action(action: &Action) -> Result<()> {
    match &action.kind {
        ActionKind::Wait { wait_hours } => {
            if *wait_hours <= 0 || *wait_hours > MAX_WAIT_HOURS {
                return Err(invalid("Enter a wait between 1 and 30 days.", "actions"));
            }
        }
        ActionKind::SendEmail { subject, lexical } => {
            if subject.chars().count() > MAX_SUBJECT_LEN {
                return Err(invalid(
                    "Email subject cannot be longer than 300 characters.",
                    "actions",
                ));
            }
            // An empty document is valid draft state, but malformed JSON is
            // not — it would fail at send time, with nobody watching.
            if !lexical.trim().is_empty() && serde_json::from_str::<serde_json::Value>(lexical).is_err() {
                return Err(invalid(
                    "Email lexical must be a well-formed Lexical document.",
                    "actions",
                ));
            }
        }
    }
    Ok(())
}

/// A draft may hold a half-written email; an active automation may not, since
/// the next signup would send it.
fn validate_active_emails(status: Status, actions: &[Action]) -> Result<()> {
    if status != Status::Active {
        return Ok(());
    }
    for action in actions {
        if let ActionKind::SendEmail { subject, lexical } = &action.kind {
            if subject.trim().is_empty() {
                return Err(invalid(
                    "Active automations require a subject line for every email.",
                    "actions",
                ));
            }
            if is_empty_lexical(lexical) {
                return Err(invalid(
                    "Active automations require a body for every email.",
                    "actions",
                ));
            }
        }
    }
    Ok(())
}

/// Whether a Lexical document has no content.
///
/// Ghost treats both "no children" and "one empty paragraph" as empty,
/// because that is what the editor leaves behind when you clear it. Anything
/// unparseable counts as empty too — there is nothing to send either way.
pub fn is_empty_lexical(lexical: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(lexical) else {
        return true;
    };
    let Some(children) = parsed.get("root").and_then(|r| r.get("children")).and_then(|c| c.as_array())
    else {
        return true;
    };
    if children.is_empty() {
        return true;
    }
    if children.len() != 1 || children[0].get("type").and_then(|t| t.as_str()) != Some("paragraph") {
        return false;
    }
    children[0]
        .get("children")
        .and_then(|c| c.as_array())
        .map(|c| c.is_empty())
        .unwrap_or(false)
}

/// Ghost's `validateGraph` + `validateLinearGraph`.
///
/// The chain must be one path: every action reachable, each with at most one
/// successor and one predecessor, exactly one start and one end, and no way
/// back to where you came from. A cycle would walk a member round forever,
/// and a branch has no defined meaning yet.
pub fn validate_graph(actions: &[Action], edges: &[Edge]) -> Result<()> {
    let mut identities: HashSet<&str> = HashSet::new();
    for action in actions {
        // Edges name nodes by id, so a duplicate id makes an edge ambiguous.
        if !identities.insert(action.id.as_str()) {
            return Err(invalid(
                "Automation action identifiers must be unique.",
                "actions",
            ));
        }
    }

    let mut seen_edges: HashSet<(&str, &str)> = HashSet::new();
    let mut outgoing: HashMap<&str, &str> = HashMap::new();
    let mut incoming: HashMap<&str, &str> = HashMap::new();

    for edge in edges {
        let (source, target) = (edge.source_action_id.as_str(), edge.target_action_id.as_str());
        if !identities.contains(source) || !identities.contains(target) {
            return Err(invalid(
                "Automation edges must reference actions in the submitted graph.",
                "edges",
            ));
        }
        if source == target {
            return Err(invalid(
                "Automation edges cannot connect an action to itself.",
                "edges",
            ));
        }
        if !seen_edges.insert((source, target)) {
            return Err(invalid("Automation edges must be unique.", "edges"));
        }
        // A second edge out of one node, or into one node, is a branch.
        if outgoing.contains_key(source) || incoming.contains_key(target) {
            return Err(invalid(
                "Automation graph must be a single linear path without branches or cycles.",
                "edges",
            ));
        }
        outgoing.insert(source, target);
        incoming.insert(target, source);
    }

    validate_linear(&identities, &outgoing, &incoming)
}

fn validate_linear(
    identities: &HashSet<&str>,
    outgoing: &HashMap<&str, &str>,
    incoming: &HashMap<&str, &str>,
) -> Result<()> {
    let shape_error = || {
        invalid(
            "Automation graph must be a single linear path without branches or cycles.",
            "edges",
        )
    };

    // Nothing to connect: a lone action, or none at all.
    if identities.len() <= 1 {
        if outgoing.is_empty() && incoming.is_empty() {
            return Ok(());
        }
        return Err(shape_error());
    }
    // A path over N nodes has exactly N-1 links.
    if outgoing.len() != identities.len() - 1 {
        return Err(shape_error());
    }

    let heads: Vec<&str> = identities.iter().copied().filter(|id| !incoming.contains_key(id)).collect();
    let tails: Vec<&str> = identities.iter().copied().filter(|id| !outgoing.contains_key(id)).collect();
    if heads.len() != 1 || tails.len() != 1 {
        return Err(shape_error());
    }

    // Walk it. Revisiting a node is a cycle; stopping early means the rest of
    // the submitted actions are floating loose.
    let mut visited: HashSet<&str> = HashSet::new();
    let mut cursor = Some(heads[0]);
    while let Some(id) = cursor {
        if !visited.insert(id) {
            return Err(shape_error());
        }
        cursor = outgoing.get(id).copied();
    }
    if visited.len() != identities.len() {
        return Err(shape_error());
    }
    Ok(())
}

/// Put `actions` in the order the chain runs them.
///
/// The graph is already known to be a single path by the time this is used,
/// so the walk is total; an unexpected shape falls back to the submitted
/// order rather than dropping nodes.
pub fn ordered_actions<'a>(actions: &'a [Action], edges: &[Edge]) -> Vec<&'a Action> {
    let by_id: HashMap<&str, &Action> = actions.iter().map(|a| (a.id.as_str(), a)).collect();
    let outgoing: HashMap<&str, &str> = edges
        .iter()
        .map(|e| (e.source_action_id.as_str(), e.target_action_id.as_str()))
        .collect();
    let has_incoming: HashSet<&str> = edges.iter().map(|e| e.target_action_id.as_str()).collect();

    let Some(head) = actions.iter().find(|a| !has_incoming.contains(a.id.as_str())) else {
        return actions.iter().collect();
    };
    let mut out = Vec::with_capacity(actions.len());
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cursor = Some(head.id.as_str());
    while let Some(id) = cursor {
        if !seen.insert(id) {
            break;
        }
        match by_id.get(id) {
            Some(action) => out.push(*action),
            None => break,
        }
        cursor = outgoing.get(id).copied();
    }
    if out.len() == actions.len() {
        out
    } else {
        actions.iter().collect()
    }
}

/// Build the edge list for a chain given in running order.
///
/// The admin UI edits a list, not a graph; this is the translation.
pub fn edges_for_chain(actions: &[Action]) -> Vec<Edge> {
    actions
        .windows(2)
        .map(|pair| Edge {
            source_action_id: pair[0].id.clone(),
            target_action_id: pair[1].id.clone(),
        })
        .collect()
}

/// Seconds from now until an action is due. Ghost's `getReadyAtForAction`.
///
/// A `send_email` is due immediately — the wait before it is its own node,
/// which is what lets the editor show the delay as a step you can edit.
pub fn ready_in_seconds(kind: &ActionKind) -> i64 {
    match kind {
        ActionKind::Wait { wait_hours } => wait_hours * 3600,
        ActionKind::SendEmail { .. } => 0,
    }
}

/// How the editor labels a wait. Ghost's UI works in days; the column is in
/// hours, so a sub-day wait still has to read sensibly.
pub fn wait_label(wait_hours: i64) -> String {
    if wait_hours % 24 == 0 {
        let days = wait_hours / 24;
        return format!("{days} day{}", if days == 1 { "" } else { "s" });
    }
    format!("{wait_hours} hour{}", if wait_hours == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(id: &str, hours: i64) -> Action {
        Action { id: id.into(), kind: ActionKind::Wait { wait_hours: hours } }
    }

    fn email(id: &str) -> Action {
        Action {
            id: id.into(),
            kind: ActionKind::SendEmail {
                subject: "Hello".into(),
                lexical: r#"{"root":{"children":[{"type":"paragraph","children":[{"type":"text","text":"hi"}]}]}}"#.into(),
            },
        }
    }

    fn chain(actions: &[Action]) -> Vec<Edge> {
        edges_for_chain(actions)
    }

    #[test]
    fn accepts_a_single_linear_path() {
        let actions = vec![email("a"), wait("b", 24), email("c")];
        assert!(validate_graph(&actions, &chain(&actions)).is_ok());
        // and the degenerate cases either side of it
        assert!(validate_graph(&[], &[]).is_ok());
        assert!(validate_graph(&[email("a")], &[]).is_ok());
    }

    #[test]
    fn rejects_a_branch() {
        let actions = vec![email("a"), email("b"), email("c")];
        // one node with two successors
        let branch = vec![
            Edge { source_action_id: "a".into(), target_action_id: "b".into() },
            Edge { source_action_id: "a".into(), target_action_id: "c".into() },
        ];
        assert!(validate_graph(&actions, &branch).is_err());
        // and one node with two predecessors
        let merge = vec![
            Edge { source_action_id: "a".into(), target_action_id: "c".into() },
            Edge { source_action_id: "b".into(), target_action_id: "c".into() },
        ];
        assert!(validate_graph(&actions, &merge).is_err());
    }

    #[test]
    fn rejects_a_cycle() {
        let actions = vec![email("a"), email("b")];
        let cycle = vec![
            Edge { source_action_id: "a".into(), target_action_id: "b".into() },
            Edge { source_action_id: "b".into(), target_action_id: "a".into() },
        ];
        // no head and no tail, so this fails before the walk
        assert!(validate_graph(&actions, &cycle).is_err());
        // a self-link is called out separately
        let loopback = vec![Edge { source_action_id: "a".into(), target_action_id: "a".into() }];
        assert!(validate_graph(&actions, &loopback).is_err());
    }

    #[test]
    fn rejects_a_disconnected_graph() {
        // two actions, no edge between them: neither is reachable from the other
        let actions = vec![email("a"), email("b")];
        assert!(validate_graph(&actions, &[]).is_err());
        // four actions wired as two separate pairs
        let actions = vec![email("a"), email("b"), email("c"), email("d")];
        let two_pairs = vec![
            Edge { source_action_id: "a".into(), target_action_id: "b".into() },
            Edge { source_action_id: "c".into(), target_action_id: "d".into() },
        ];
        assert!(validate_graph(&actions, &two_pairs).is_err());
    }

    #[test]
    fn rejects_edges_that_name_unknown_or_duplicate_nodes() {
        let actions = vec![email("a"), email("b")];
        let dangling = vec![Edge { source_action_id: "a".into(), target_action_id: "zz".into() }];
        assert!(validate_graph(&actions, &dangling).is_err());

        let dupe = vec![email("a"), email("a")];
        assert!(validate_graph(&dupe, &[]).is_err());

        let repeated = vec![
            Edge { source_action_id: "a".into(), target_action_id: "b".into() },
            Edge { source_action_id: "a".into(), target_action_id: "b".into() },
        ];
        assert!(validate_graph(&actions, &repeated).is_err());
    }

    #[test]
    fn active_automations_must_be_complete() {
        let blank = Action {
            id: "a".into(),
            kind: ActionKind::SendEmail { subject: "   ".into(), lexical: "{}".into() },
        };
        // a draft may hold a half-written email
        assert!(validate(Status::Inactive, "Welcome", "", &[blank.clone()], &[]).is_ok());
        // turning it on may not
        assert!(validate(Status::Active, "Welcome", "", &[blank], &[]).is_err());
        // nor may an active automation with nothing in it
        assert!(validate(Status::Active, "Welcome", "", &[], &[]).is_err());
        assert!(validate(Status::Inactive, "Welcome", "", &[], &[]).is_ok());

        let complete = vec![email("a")];
        assert!(validate(Status::Active, "Welcome", "", &complete, &[]).is_ok());
    }

    #[test]
    fn empty_lexical_matches_ghosts_definition() {
        assert!(is_empty_lexical(r#"{"root":{"children":[]}}"#));
        assert!(is_empty_lexical(r#"{"root":{"children":[{"type":"paragraph","children":[]}]}}"#));
        assert!(is_empty_lexical("not json"));
        assert!(is_empty_lexical(""));
        assert!(!is_empty_lexical(
            r#"{"root":{"children":[{"type":"paragraph","children":[{"type":"text","text":"hi"}]}]}}"#
        ));
        // two empty paragraphs is content as far as Ghost is concerned
        assert!(!is_empty_lexical(
            r#"{"root":{"children":[{"type":"paragraph","children":[]},{"type":"paragraph","children":[]}]}}"#
        ));
    }

    #[test]
    fn wait_bounds_match_ghost() {
        assert!(validate(Status::Inactive, "n", "", &[wait("a", 1)], &[]).is_ok());
        assert!(validate(Status::Inactive, "n", "", &[wait("a", MAX_WAIT_HOURS)], &[]).is_ok());
        assert!(validate(Status::Inactive, "n", "", &[wait("a", 0)], &[]).is_err());
        assert!(validate(Status::Inactive, "n", "", &[wait("a", MAX_WAIT_HOURS + 1)], &[]).is_err());
    }

    #[test]
    fn a_name_is_required_and_bounded() {
        assert!(validate(Status::Inactive, "  ", "", &[], &[]).is_err());
        let long = "x".repeat(MAX_NAME_LEN + 1);
        assert!(validate(Status::Inactive, &long, "", &[], &[]).is_err());
    }

    #[test]
    fn ordering_follows_the_edges_not_the_submitted_order() {
        let actions = vec![email("c"), email("a"), email("b")];
        let edges = vec![
            Edge { source_action_id: "a".into(), target_action_id: "b".into() },
            Edge { source_action_id: "b".into(), target_action_id: "c".into() },
        ];
        let order: Vec<&str> = ordered_actions(&actions, &edges).iter().map(|a| a.id.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn a_chain_round_trips_through_its_edges() {
        let actions = vec![email("a"), wait("b", 48), email("c")];
        let edges = edges_for_chain(&actions);
        assert_eq!(edges.len(), 2);
        let order: Vec<&str> = ordered_actions(&actions, &edges).iter().map(|a| a.id.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn timing_matches_ghost() {
        assert_eq!(ready_in_seconds(&ActionKind::Wait { wait_hours: 48 }), 172_800);
        assert_eq!(
            ready_in_seconds(&ActionKind::SendEmail { subject: String::new(), lexical: String::new() }),
            0
        );
    }

    #[test]
    fn wait_labels_read_in_days_where_they_can() {
        assert_eq!(wait_label(24), "1 day");
        assert_eq!(wait_label(48), "2 days");
        assert_eq!(wait_label(1), "1 hour");
        assert_eq!(wait_label(5), "5 hours");
    }

    #[test]
    fn paid_scopes_keep_comped_and_gift_members() {
        assert_eq!(eligible_statuses("free"), &["free"]);
        assert!(eligible_statuses("paid").contains(&"comped"));
        assert!(eligible_statuses("paid").contains(&"gift"));
        assert!(!eligible_statuses("paid").contains(&"free"));
    }

    #[test]
    fn welcome_email_slugs_recognise_only_the_builtin_flows() {
        assert!(is_welcome_email_slug(Some("member-welcome-email-free")));
        assert!(is_welcome_email_slug(Some("member-welcome-email-paid")));
        // user-made automations have no slug; a near-miss is not reserved
        assert!(!is_welcome_email_slug(None));
        assert!(!is_welcome_email_slug(Some("member-welcome-email-paid ")));
        assert!(!is_welcome_email_slug(Some("my-flow")));
    }
}
