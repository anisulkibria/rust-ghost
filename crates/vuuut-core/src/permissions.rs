//! What each staff role is allowed to reach: the model, the defaults, and the
//! reading and writing of the one `permissions` setting that holds them.
//!
//! This is the rules half of the Permissions module. It has no database and no
//! HTTP in it, so what a grant *means* can be unit-tested without either, and
//! the admin UI, the middleware that enforces it and the Owner's editor all
//! ask the same functions.
//!
//! ## The shape
//!
//! A role's grants are three resource families plus three content levels:
//!
//! - **tools, settings** — each a [`Rules`] (modules were folded into tools
//!   on 2026-11-21): one default plus any
//!   per-key exceptions, where a key is a module slug (`seo`), a tool key
//!   (`redirects`) or a settings group slug (`security`). The value is an
//!   [`Access`]: `None`, `View` or `Edit`.
//! - **posts, pages** — a [`Content`] level: `None`, `OwnDrafts`, `Own` or
//!   `All`.
//! - **tags** — a [`Tags`] level: `None`, `View` or `Manage`.
//!
//! ## Why a default plus exceptions, and not a list of allowed keys
//!
//! A stored list of allowed keys answers the question for the modules that
//! existed on the day it was saved. Every module added afterwards is then
//! invisible to everybody — including Administrators — until somebody
//! remembers to edit this screen, and the failure looks like the new module
//! being broken rather than like a permission. A default covers what does not
//! exist yet: `Administrator` defaults to `Edit`, so a new module behaves on
//! the day it ships exactly as it would have before this module existed.
//!
//! ## The Owner
//!
//! The Owner is not in the table and cannot be. Every check short-circuits to
//! `Edit` for them, because the only person who can repair a mistake on this
//! screen must not be able to lock themselves out of it.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The settings key the whole model is stored under.
pub const SETTING_KEY: &str = "permissions";

/// The key a [`Rules`] default is stored under. `*` rather than `default`
/// because a module could plausibly be called "default"; no module slug, tool
/// key or settings slug can contain an asterisk.
pub const DEFAULT_KEY: &str = "*";

/// What a role may do with one module, tool or settings group.
///
/// The three levels map onto HTTP in the middleware that enforces them: a
/// `GET` needs `View`, anything that changes state needs `Edit`. That is the
/// whole of "can see it" versus "can change it", and it does not need every
/// handler to be edited to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Not offered in the rail, not on the hub, and refused if typed.
    None,
    /// May open it and read it. Writes are refused.
    View,
    /// May open it and change it.
    Edit,
}

impl Access {
    pub fn from_name(name: &str) -> Access {
        match name {
            "edit" => Access::Edit,
            "view" => Access::View,
            // Anything unrecognised is the least privilege, never the most:
            // a typo in a stored grant must not read as access.
            _ => Access::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Access::Edit => "edit",
            Access::View => "view",
            Access::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Access::Edit => "Can edit",
            Access::View => "Can view",
            Access::None => "No access",
        }
    }

    /// Whether this level covers a request that changes state.
    pub fn may_write(self) -> bool {
        self == Access::Edit
    }

    /// Whether this level covers reading the screen at all.
    pub fn may_read(self) -> bool {
        self >= Access::View
    }
}

/// How much of the posts (or pages) list a role reaches.
///
/// One ladder rather than three separate switches (view/edit, own/everyone's,
/// may publish), because those three are not independent in practice and a
/// screen with three controls per row per role is one nobody reads. Every
/// rung is a sentence somebody would actually say about a colleague, and the
/// four roles' historic behaviours are all on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Content {
    /// The list is not offered at all.
    None,
    /// May read their own, and change nothing.
    ViewOwn,
    /// May read everybody's, and change nothing.
    ViewAll,
    /// Write and edit their own, but never publish — Ghost's Contributor.
    DraftOwn,
    /// Write, edit and publish their own — Ghost's Author.
    EditOwn,
    /// Edit, publish and delete anybody's — Ghost's Editor.
    EditAll,
}

impl Content {
    pub fn from_name(name: &str) -> Content {
        match name {
            "edit_all" => Content::EditAll,
            "edit_own" => Content::EditOwn,
            "draft_own" => Content::DraftOwn,
            "view_all" => Content::ViewAll,
            "view_own" => Content::ViewOwn,
            _ => Content::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Content::EditAll => "edit_all",
            Content::EditOwn => "edit_own",
            Content::DraftOwn => "draft_own",
            Content::ViewAll => "view_all",
            Content::ViewOwn => "view_own",
            Content::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Content::EditAll => "Edit everyone's",
            Content::EditOwn => "Write & publish their own",
            Content::DraftOwn => "Draft their own (no publishing)",
            Content::ViewAll => "View everyone's",
            Content::ViewOwn => "View their own",
            Content::None => "No access",
        }
    }

    /// Every rung, in the order the editor offers them.
    pub const LADDER: &'static [Content] = &[
        Content::None,
        Content::ViewOwn,
        Content::ViewAll,
        Content::DraftOwn,
        Content::EditOwn,
        Content::EditAll,
    ];

    /// Whether the list is offered in the rail at all.
    pub fn may_read(self) -> bool {
        self != Content::None
    }

    /// Whether this role sees entries that are not theirs. Written as a match
    /// rather than an ordering comparison: the ladder is ordered for the
    /// editor's benefit, and leaning on `>=` here is how "view everyone's"
    /// would quietly start meaning "may draft".
    pub fn any_author(self) -> bool {
        matches!(self, Content::ViewAll | Content::EditAll)
    }

    /// Whether this role may change anything at all.
    pub fn may_write(self) -> bool {
        matches!(self, Content::DraftOwn | Content::EditOwn | Content::EditAll)
    }

    /// Whether this role may publish. `DraftOwn` is the Contributor line:
    /// they write, somebody else decides it goes out.
    pub fn may_publish(self) -> bool {
        matches!(self, Content::EditOwn | Content::EditAll)
    }
}

/// What a role may do with tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tags {
    None,
    View,
    Manage,
}

impl Tags {
    pub fn from_name(name: &str) -> Tags {
        match name {
            "manage" => Tags::Manage,
            "view" => Tags::View,
            _ => Tags::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tags::Manage => "manage",
            Tags::View => "view",
            Tags::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tags::Manage => "Create & edit",
            Tags::View => "View only",
            Tags::None => "No access",
        }
    }

    pub fn may_read(self) -> bool {
        self >= Tags::View
    }

    pub fn may_write(self) -> bool {
        self == Tags::Manage
    }
}

/// One resource family: a default, plus the keys that differ from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub default: Access,
    pub by_key: BTreeMap<String, Access>,
}

impl Rules {
    pub fn everything() -> Rules {
        Rules { default: Access::Edit, by_key: BTreeMap::new() }
    }

    pub fn nothing() -> Rules {
        Rules { default: Access::None, by_key: BTreeMap::new() }
    }

    /// What this role may do with one key.
    pub fn access(&self, key: &str) -> Access {
        self.by_key.get(key).copied().unwrap_or(self.default)
    }

    pub fn set(&mut self, key: &str, access: Access) {
        if access == self.default {
            // Storing an exception equal to the default is how a stored
            // ruleset slowly becomes a list of every key that exists, which
            // is the shape this model exists to avoid.
            self.by_key.remove(key);
        } else {
            self.by_key.insert(key.to_string(), access);
        }
    }

    fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert(DEFAULT_KEY.into(), Value::String(self.default.name().into()));
        for (key, access) in &self.by_key {
            map.insert(key.clone(), Value::String(access.name().into()));
        }
        Value::Object(map)
    }

    fn from_json(value: Option<&Value>, fallback: Access) -> Rules {
        let Some(Value::Object(map)) = value else {
            return Rules { default: fallback, by_key: BTreeMap::new() };
        };
        let default = map
            .get(DEFAULT_KEY)
            .and_then(Value::as_str)
            .map(Access::from_name)
            .unwrap_or(fallback);
        let by_key = map
            .iter()
            .filter(|(key, _)| key.as_str() != DEFAULT_KEY)
            .filter_map(|(key, value)| {
                value.as_str().map(|name| (key.clone(), Access::from_name(name)))
            })
            .filter(|(_, access)| *access != default)
            .collect();
        Rules { default, by_key }
    }
}

/// Everything one role is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grants {
    /// Every hub card. Modules and Tools were two families until 2026-11-21,
    /// when the Modules hub was folded into Tools; a stored `modules` object
    /// is read into this one (see [`FORMER_MODULES`]) and never written back.
    pub tools: Rules,
    pub settings: Rules,
    pub posts: Content,
    pub pages: Content,
    pub tags: Tags,
}

impl Grants {
    /// The Owner's grants, and the only ones that are not stored: everything,
    /// always.
    pub fn owner() -> Grants {
        Grants {
            tools: Rules::everything(),
            settings: Rules::everything(),
            posts: Content::EditAll,
            pages: Content::EditAll,
            tags: Tags::Manage,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "tools": self.tools.to_json(),
            "settings": self.settings.to_json(),
            "posts": self.posts.name(),
            "pages": self.pages.name(),
            "tags": self.tags.name(),
        })
    }

    fn from_json(value: Option<&Value>, defaults: &Grants) -> Grants {
        let Some(Value::Object(map)) = value else {
            return defaults.clone();
        };
        let level = |key: &str, fallback: Content| {
            map.get(key).and_then(Value::as_str).map(Content::from_name).unwrap_or(fallback)
        };
        let mut tools = Rules::from_json(map.get("tools"), defaults.tools.default);
        // A grant saved before the merge. Each former module keeps exactly
        // the access it had — its own exception, or the old modules default —
        // unless the tools rules already name it. Written as an exception
        // only where it differs from the tools default, so nothing that was
        // not granted becomes granted. The next save drops `modules` for good.
        if let Some(stored) = map.get("modules") {
            let modules = Rules::from_json(Some(stored), Access::None);
            for key in FORMER_MODULES {
                if !tools.by_key.contains_key(*key) {
                    tools.set(key, modules.access(key));
                }
            }
        }
        Grants {
            tools,
            settings: Rules::from_json(map.get("settings"), defaults.settings.default),
            posts: level("posts", defaults.posts),
            pages: level("pages", defaults.pages),
            tags: map
                .get("tags")
                .and_then(Value::as_str)
                .map(Tags::from_name)
                .unwrap_or(defaults.tags),
        }
    }
}

/// The keys that were Modules until they became Tools on 2026-11-21. Only
/// [`Grants::from_json`] reads it, to carry a pre-merge grant across.
pub const FORMER_MODULES: &[&str] = &[
    "analytics", "cookies", "forms", "languages", "media", "memberships", "newsletters", "popups",
    "security", "seo",
];

/// The roles this module can grant to. The Owner is deliberately absent —
/// see the module docs.
pub const EDITABLE_ROLES: &[&str] = &["Administrator", "Editor", "Author", "Contributor"];

/// What a role is allowed when nobody has said otherwise.
///
/// **These reproduce the admin's behaviour from before this module existed**,
/// which is the rule every module here follows: installing it changes nothing
/// until somebody moves something. Administrators reached every module, tool
/// and settings page and nobody below them reached any; Editors managed
/// everyone's posts, pages and tags; Authors their own; Contributors their own
/// drafts, without publishing.
pub fn defaults(role: &str) -> Grants {
    match role {
        "Owner" => Grants::owner(),
        "Administrator" => Grants::owner(),
        "Editor" => Grants {
            tools: Rules::nothing(),
            settings: Rules::nothing(),
            posts: Content::EditAll,
            pages: Content::EditAll,
            tags: Tags::Manage,
        },
        "Author" => Grants {
            tools: Rules::nothing(),
            settings: Rules::nothing(),
            posts: Content::EditOwn,
            pages: Content::EditOwn,
            tags: Tags::None,
        },
        // Anything unrecognised is treated as the least privileged role there
        // is, for the same reason `Role::from_name` does: an unknown role must
        // never imply trust.
        _ => Grants {
            tools: Rules::nothing(),
            settings: Rules::nothing(),
            posts: Content::DraftOwn,
            pages: Content::DraftOwn,
            tags: Tags::None,
        },
    }
}

/// Every role's grants, read from the stored setting with the defaults filled
/// in behind it.
///
/// A missing, malformed or partial value is not an error: each role falls back
/// to [`defaults`] independently, so one bad entry cannot take the others —
/// or the site — down with it.
pub fn read(stored: Option<&Value>) -> BTreeMap<String, Grants> {
    // The settings cache hands objects back as a `Value::Object`, but a row
    // written before the column was typed can still be a JSON *string*. Both
    // shapes are read here rather than at every call site.
    let parsed;
    let map = match stored {
        Some(Value::Object(map)) => Some(map),
        Some(Value::String(raw)) => {
            parsed = serde_json::from_str::<Value>(raw).ok();
            match parsed.as_ref() {
                Some(Value::Object(map)) => Some(map),
                _ => None,
            }
        }
        _ => None,
    };
    EDITABLE_ROLES
        .iter()
        .map(|role| {
            let fallback = defaults(role);
            let grants = match map {
                // Stored under the role's own name, lowercased, so the JSON
                // reads the way the screen does.
                Some(map) => Grants::from_json(map.get(&role.to_lowercase()), &fallback),
                None => fallback,
            };
            (role.to_string(), grants)
        })
        .collect()
}

/// Serialize every role's grants for storage.
pub fn write(all: &BTreeMap<String, Grants>) -> Value {
    let mut map = Map::new();
    for role in EDITABLE_ROLES {
        if let Some(grants) = all.get(*role) {
            map.insert(role.to_lowercase(), grants.to_json());
        }
    }
    Value::Object(map)
}

/// One role's grants, with the Owner short-circuited.
pub fn for_role(stored: Option<&Value>, role: &str) -> Grants {
    if role == "Owner" {
        return Grants::owner();
    }
    read(stored).remove(role).unwrap_or_else(|| defaults(role))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_reproduce_the_admin_as_it_was() {
        // Administrators reached everything; an Editor reached no module,
        // tool or settings page but managed all content and tags.
        let admin = defaults("Administrator");
        assert_eq!(admin.tools.access("anything-at-all"), Access::Edit);
        assert_eq!(admin.settings.access("security"), Access::Edit);

        let editor = defaults("Editor");
        assert_eq!(editor.tools.access("seo"), Access::None);
        assert_eq!(editor.tools.access("redirects"), Access::None);
        assert_eq!(editor.posts, Content::EditAll);
        assert_eq!(editor.tags, Tags::Manage);

        let author = defaults("Author");
        assert_eq!(author.posts, Content::EditOwn);
        assert!(author.posts.may_publish());
        assert_eq!(author.tags, Tags::None);

        let contributor = defaults("Contributor");
        assert_eq!(contributor.posts, Content::DraftOwn);
        assert!(!contributor.posts.may_publish());
        assert!(contributor.posts.may_read());
    }

    #[test]
    fn an_unknown_role_gets_the_least_privilege() {
        let unknown = defaults("Chief Vibes Officer");
        assert_eq!(unknown.tools.access("seo"), Access::None);
        assert_eq!(unknown.posts, Content::DraftOwn);
        assert_eq!(unknown.tags, Tags::None);
    }

    #[test]
    fn the_owner_is_never_restricted() {
        // Even a stored value that tries to take everything away.
        let hostile = serde_json::json!({
            "owner": {"tools": {"*": "none"}, "posts": "none", "tags": "none"}
        });
        let grants = for_role(Some(&hostile), "Owner");
        assert_eq!(grants.tools.access("seo"), Access::Edit);
        assert_eq!(grants.posts, Content::EditAll);
    }

    #[test]
    fn a_default_covers_keys_that_did_not_exist_when_it_was_saved() {
        let stored = serde_json::json!({
            "editor": {"tools": {"*": "view", "security": "none"}}
        });
        let editor = for_role(Some(&stored), "Editor");
        // Named exception.
        assert_eq!(editor.tools.access("security"), Access::None);
        // A module nobody has heard of yet still follows the default.
        assert_eq!(editor.tools.access("module-from-next-year"), Access::View);
    }

    #[test]
    fn unparsable_or_partial_values_fall_back_per_role() {
        // Garbage in one role does not disturb another.
        let stored = serde_json::json!({
            "editor": "not an object",
            "author": {"posts": "edit_all"}
        });
        let all = read(Some(&stored));
        assert_eq!(all["Editor"], defaults("Editor"));
        // The author's one stated field is honoured and the rest default.
        assert_eq!(all["Author"].posts, Content::EditAll);
        assert_eq!(all["Author"].tags, defaults("Author").tags);
        // A completely missing setting is every default.
        assert_eq!(read(None)["Editor"], defaults("Editor"));
    }

    #[test]
    fn an_unrecognised_level_reads_as_no_access() {
        let stored = serde_json::json!({
            "editor": {"tools": {"*": "administrator"}, "posts": "everything", "tags": "yes"}
        });
        let editor = for_role(Some(&stored), "Editor");
        assert_eq!(editor.tools.access("seo"), Access::None);
        assert_eq!(editor.posts, Content::None);
        assert_eq!(editor.tags, Tags::None);
    }

    #[test]
    fn a_round_trip_keeps_what_was_set() {
        let mut all = read(None);
        let editor = all.get_mut("Editor").unwrap();
        editor.tools.set("seo", Access::Edit);
        editor.tools.set("security", Access::View);
        editor.settings.set("general", Access::View);
        editor.posts = Content::EditOwn;
        editor.tags = Tags::View;

        let json = write(&all);
        let back = read(Some(&json));
        assert_eq!(back["Editor"].tools.access("seo"), Access::Edit);
        assert_eq!(back["Editor"].tools.access("security"), Access::View);
        assert_eq!(back["Editor"].tools.access("forms"), Access::None);
        assert_eq!(back["Editor"].settings.access("general"), Access::View);
        assert_eq!(back["Editor"].posts, Content::EditOwn);
        assert_eq!(back["Editor"].tags, Tags::View);
        assert_eq!(back["Administrator"], defaults("Administrator"));
    }

    #[test]
    fn resetting_one_role_leaves_the_others_alone() {
        // What the Reset button does: put this role back to `defaults` and
        // write the map again. The other roles' customisations have to
        // survive that, or one reset quietly undoes every other edit.
        let mut all = read(None);
        all.get_mut("Editor").unwrap().tools.set("seo", Access::Edit);
        all.get_mut("Author").unwrap().posts = Content::ViewAll;

        all.insert("Editor".into(), defaults("Editor"));
        let back = read(Some(&write(&all)));

        assert_eq!(back["Editor"], defaults("Editor"), "the reset role is back to default");
        assert_eq!(back["Editor"].tools.access("seo"), Access::None);
        assert_eq!(back["Author"].posts, Content::ViewAll, "another role's edit survived");
    }

    #[test]
    fn setting_a_key_back_to_the_default_drops_the_exception() {
        let mut rules = Rules::nothing();
        rules.set("seo", Access::Edit);
        assert_eq!(rules.by_key.len(), 1);
        rules.set("seo", Access::None);
        assert!(rules.by_key.is_empty(), "an exception equal to the default is not stored");
        assert_eq!(rules.access("seo"), Access::None);
    }

    #[test]
    fn levels_answer_read_and_write_consistently() {
        assert!(Access::Edit.may_write() && Access::Edit.may_read());
        assert!(!Access::View.may_write() && Access::View.may_read());
        assert!(!Access::None.may_read());
        assert!(Content::EditAll.any_author() && Content::ViewAll.any_author());
        assert!(!Content::EditOwn.any_author() && !Content::ViewOwn.any_author());
        assert!(Content::ViewAll.may_read() && !Content::ViewAll.may_write());
        assert!(Tags::Manage.may_write() && Tags::View.may_read() && !Tags::View.may_write());
    }

    #[test]
    fn a_grant_saved_before_the_merge_keeps_exactly_its_access() {
        // An Editor allowed into SEO and every module by view, Security
        // refused, and one tool of their own — written by the old editor.
        let stored = serde_json::json!({
            "editor": {
                "modules": {"*": "view", "security": "none", "seo": "edit"},
                "tools": {"*": "none", "redirects": "edit"}
            }
        });
        let editor = read(Some(&stored)).remove("Editor").unwrap();
        assert_eq!(editor.tools.access("seo"), Access::Edit);
        assert_eq!(editor.tools.access("analytics"), Access::View, "the old modules default");
        assert_eq!(editor.tools.access("security"), Access::None);
        assert_eq!(editor.tools.access("redirects"), Access::Edit);
        assert_eq!(editor.tools.access("staff"), Access::None, "tools that were never modules are untouched");
        // Saved again, `modules` is gone and nothing changed.
        let mut all = read(Some(&stored));
        let rewritten = write(&all);
        assert!(rewritten["editor"].get("modules").is_none());
        let again = read(Some(&rewritten)).remove("Editor").unwrap();
        assert_eq!(again, editor);
        // And a later change to a former module sticks, rather than the old
        // modules value coming back on the next read.
        all.get_mut("Editor").unwrap().tools.set("seo", Access::None);
        let back = read(Some(&write(&all))).remove("Editor").unwrap();
        assert_eq!(back.tools.access("seo"), Access::None);
    }
}
