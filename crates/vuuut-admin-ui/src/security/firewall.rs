//! The Firewall screen: the rules, and the guards that stop you locking
//! yourself out with one.
//!
//! The rules themselves are [`vuuut_core::firewall`], the storage is
//! [`vuuut_db::repos::firewall`], and the enforcement is
//! [`vuuut_api::firewall`]. What is here is the screen and the four actions
//! behind it — add, remove, remove everything, and the master switch.
//!
//! ## The lockout guard is the point of this file
//!
//! Three of those four actions can lock an administrator out of the screen
//! they would undo it from, and each checks before it writes:
//!
//! * **Adding a rule** simulates it. Not a special-case "is this my address"
//!   test — the real [`vuuut_core::firewall::decide`] over the real current
//!   rules plus the candidate. Anything less misses the case that actually
//!   catches people out, which is the first admin *allow* rule naming
//!   somebody else's address and switching allowlist mode on for everybody.
//! * **Removing a rule** simulates the removal, because taking away the only
//!   allow rule covering your own address is as effective a lockout as any
//!   block.
//! * **Turning the firewall on** runs the same simulation, because the rules
//!   may have been written while it was off.
//!
//! Turning it **off** is never checked. It is the undo, and an undo that can
//! refuse is not one.
//!
//! The guard protects the person saving and nobody else — a colleague on
//! another network can still be barred by a careless range — and the screen
//! says so in those words rather than implying a safety it does not have.

use super::*;

// ---------------------------------------------------------------------------
// Firewall
// ---------------------------------------------------------------------------

const FIREWALL_PATH: &str = "/ghost/security/firewall/";

/// How long a temporary rule lasts. Offered values only; a rule with an
/// expiry nobody chose is a rule that lifts at a time nobody expects.
const DURATIONS: &[(&str, &str)] = &[
    ("0", "Until I remove it"),
    ("1", "1 hour"),
    ("24", "24 hours"),
    ("168", "7 days"),
    ("720", "30 days"),
];

/// The caller's own address, as the firewall will see it on their next
/// request.
///
/// Read through exactly the same function the middleware uses, so the
/// lockout guard cannot be checking a different address from the one that
/// will be matched. If these two ever disagreed, the guard would cheerfully
/// approve the rule that locks you out.
fn my_ip(state: &AppState, headers: &HeaderMap, peer: Option<std::net::IpAddr>) -> Option<std::net::IpAddr> {
    vuuut_api::brute::client_ip_for(state, headers, peer).parse().ok()
}

pub async fn firewall_page(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let stored = match fw_repo::all(&state.db).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };
    let enabled = state.settings.get_bool("firewall_enabled");
    let now = vuuut_core::dates::now_db();
    let mine = my_ip(&state, &headers, peer);

    // Allowlist mode is the one piece of behaviour that is not "a rule
    // matched", so it is stated on the page rather than left to be inferred
    // from the list.
    let allowlisting = stored
        .iter()
        .filter(|r| !r.is_expired(&now) && r.is_valid())
        .any(|r| r.action == "allow" && r.scope == "admin");

    let rows: String = stored
        .iter()
        .map(|r| {
            let expired = r.is_expired(&now);
            let broken = !r.is_valid();
            let allow = r.action == "allow";
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title"><code{strike}>{pattern}</code>{chips}</span>
<span class="detail">{what} · {size} · added {added}{note}{hits}{expiry}</span></div>
<div class="trailing"><form method="post" action="{path}delete/" style="margin:0">
<input type="hidden" name="id" value="{id}">
<button class="btn text{danger}" type="submit">Remove</button></form></div></div>"#,
                tint = if broken || expired { "slate" } else if allow { "green" } else { "red" },
                icon = svg(if allow { "check" } else { "lock" }),
                // Struck through when the pattern will not parse: the rule is
                // in the table and is not in force, and that belongs on the
                // page rather than in a log.
                strike = if broken { r#" style="text-decoration:line-through""# } else { "" },
                pattern = esc(&r.pattern),
                chips = format!(
                    "{a}{s}{e}{b}",
                    a = if allow {
                        r#" <span class="chip tonal">allow</span>"#
                    } else {
                        r#" <span class="chip error">block</span>"#
                    },
                    s = if r.scope == "site" {
                        r#" <span class="chip draft">whole site</span>"#
                    } else {
                        r#" <span class="chip tonal">admin only</span>"#
                    },
                    e = if expired { r#" <span class="chip draft">expired</span>"# } else { "" },
                    b = if broken { r#" <span class="chip error">unreadable</span>"# } else { "" },
                ),
                what = if allow { "Always let through" } else { "Refuse" },
                size = esc(
                    &vuuut_core::firewall::Pattern::parse(&r.pattern)
                        .map(|p| p.size_label())
                        .unwrap_or_else(|| "cannot be read".into())
                ),
                added = esc(&when(&r.created_at, &timezone)),
                // A note is somebody's own words about a person. Escaped like
                // everything else.
                note = if r.note.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", esc(&r.note))
                },
                hits = if r.hits > 0 {
                    format!(
                        " · {} {}",
                        crate::countable(r.hits, "hit"),
                        r.last_hit_at
                            .as_deref()
                            .map(|at| format!("(last {})", when(at, &timezone)))
                            .unwrap_or_default(),
                    )
                } else {
                    String::new()
                },
                expiry = r
                    .expires_at
                    .as_deref()
                    .filter(|at| !at.is_empty())
                    .map(|at| format!(" · {} {}", if expired { "expired" } else { "expires" }, when(at, &timezone)))
                    .unwrap_or_default(),
                path = FIREWALL_PATH,
                id = esc(&r.id),
                danger = if allow { " danger" } else { "" },
            )
        })
        .collect();

    page_for_role(
        actor.role,
        "Security",
        "security",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}

<div class="card padded rise" style="--i:2">
<div class="rows"><div class="row" style="padding-left:0;padding-right:0">
<span class="ic tint-{state_tint}">{shield}</span>
<div class="body"><span class="title">The firewall is {state_word}</span>
<span class="detail">{state_detail}</span></div>
<div class="trailing"><form method="post" action="{path}toggle/" style="margin:0">
<input type="hidden" name="enabled" value="{next}">
<button class="btn tonal{toggle_danger}" type="submit">{toggle_label}</button></form></div>
</div></div>
<p class="muted" style="margin:10px 0 0">Your address right now is <code>{mine}</code>. A rule that would refuse it is refused when you save it — but that only protects <em>you</em>, not a colleague on another network.</p>
{allowlist_note}
</div>

<div class="section">Add a rule</div>
<div class="card padded rise" style="--i:3">
<form method="post" action="{path}add/">
<div style="display:flex;gap:10px;flex-wrap:wrap;align-items:flex-start">
<label class="field" style="flex:2;min-width:190px;margin:0"><input name="pattern" placeholder=" " required maxlength="60" spellcheck="false" autocomplete="off"><span>Address or range</span></label>
<div class="selbox" style="flex:1 1 140px;min-width:140px"><select name="action" aria-label="Action">{actions}</select>{car}</div>
<div class="selbox" style="flex:1 1 170px;min-width:170px"><select name="scope" aria-label="Applies to">{scopes}</select>{car}</div>
<div class="selbox" style="flex:1 1 190px;min-width:190px"><select name="hours" aria-label="Lasts for">{durations}</select>{car}</div>
</div>
<label class="field" style="margin:10px 0 8px"><input name="note" placeholder=" " maxlength="{note_max}"><span>Why (optional)</span></label>
<button class="btn" type="submit">{plus} Add rule</button>
<p class="muted" style="margin:10px 0 0">A single address (<code>203.0.113.4</code>, <code>2001:db8::1</code>) or a CIDR range (<code>203.0.113.0/24</code>, <code>2001:db8::/32</code>). An <b>allow</b> beats a <b>block</b>, so you can bar a range and admit one address inside it.</p>
</form>
</div>

<div class="section">Rules{count}</div>
{list}
<p class="muted" style="margin:10px 0 0">Rules are held in memory and reloaded when you save one here. A rule written straight into the database takes effect at the next restart.{clear}</p>"#,
            head = head("firewall"),
            banner = flash_banner(&q),
            path = FIREWALL_PATH,
            shield = svg(if enabled { "lock" } else { "hide" }),
            state_tint = if enabled { "green" } else { "slate" },
            state_word = if enabled { "on" } else { "off" },
            state_detail = if enabled {
                "Every request is checked against the rules below before anything else happens."
            } else {
                "The rules below are stored but not enforced. Adding a rule turns the firewall on."
            },
            next = if enabled { "0" } else { "1" },
            toggle_label = if enabled { "Turn off" } else { "Turn on" },
            toggle_danger = if enabled { " danger" } else { "" },
            mine = esc(&mine.map(|ip| ip.to_string()).unwrap_or_else(|| "not known".into())),
            allowlist_note = if allowlisting {
                format!(
                    r#"<p class="banner error" role="status" style="margin:10px 0 0">{warn}<span class="msg"><b>Allowlist mode is on.</b> Because at least one <b>allow</b> rule applies to the admin, <b>every address not covered by one is refused</b> from {ghost}. Removing the last admin allow rule opens it again.</span></p>"#,
                    warn = svg("warning"),
                    ghost = "<code>/ghost/</code>",
                )
            } else {
                String::new()
            },
            actions = select_options(&[("block", "Block"), ("allow", "Always allow")], "block"),
            scopes = select_options(
                &[("admin", "The admin only"), ("site", "The whole site")],
                "admin",
            ),
            durations = select_options(DURATIONS, "0"),
            car = SELECT_CARET,
            note_max = vuuut_core::firewall::NOTE_MAX,
            plus = svg("add"),
            count = if stored.is_empty() {
                String::new()
            } else {
                format!(" · {}", crate::countable(stored.len() as i64, "rule"))
            },
            list = if stored.is_empty() {
                r#"<div class="card padded rise" style="--i:4"><p class="muted" style="margin:0">No rules. The firewall lets everybody through.</p></div>"#.to_string()
            } else {
                format!(r#"<div class="card rows rise" style="--i:4">{rows}</div>"#)
            },
            clear = if stored.len() > 1 {
                format!(
                    r#" <form method="post" action="{path}clear/" style="display:inline"><button class="btn text danger" type="submit">Remove every rule</button></form>"#,
                    path = FIREWALL_PATH,
                )
            } else {
                String::new()
            },
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct RuleForm {
    pattern: String,
    action: String,
    scope: String,
    hours: String,
    note: String,
}

/// Add a rule, refusing one that would lock the author out.
///
/// The order here is the argument: parse, then *simulate*, then store. The
/// simulation runs the real [`vuuut_core::firewall::decide`] over the real
/// current rules plus the candidate, against the author's own address read
/// through the same function the middleware will use. Anything less — a
/// special-case "is this my IP" check, say — would miss the case that
/// actually catches people out, which is the first admin allow rule naming
/// somebody *else's* address and switching allowlist mode on.
pub(crate) async fn firewall_add(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<RuleForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };

    let Some(pattern) = vuuut_core::firewall::Pattern::parse(&form.pattern) else {
        return flash_to(
            FIREWALL_PATH,
            "error",
            "That is not an address or a range. Try 203.0.113.4 or 203.0.113.0/24.",
        );
    };
    let action = vuuut_core::firewall::Action::parse(&form.action);
    let scope = vuuut_core::firewall::Scope::parse(&form.scope);

    // Only a duration we offered. An expiry nobody chose is a rule that lifts
    // at a time nobody expects.
    let hours = DURATIONS
        .iter()
        .find(|(value, _)| *value == form.hours.trim())
        .and_then(|(value, _)| value.parse::<i64>().ok())
        .unwrap_or(0);
    // Computed here, not in SQL: `datetime('now', ?)` with a bound modifier
    // does not work on MySQL through the Any driver.
    let expires_at = (hours > 0).then(|| {
        vuuut_core::dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::hours(hours)))
    });

    let existing = match fw_repo::all(&state.db).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };
    if existing.len() >= vuuut_core::firewall::MAX_RULES {
        return flash_to(
            FIREWALL_PATH,
            "error",
            "This site already has as many firewall rules as it can hold. Remove some first.",
        );
    }
    let live: Vec<vuuut_core::firewall::Rule> =
        existing.iter().filter_map(|r| r.as_rule()).collect();

    let candidate = vuuut_core::firewall::Rule {
        id: "candidate".into(),
        action,
        scope,
        pattern,
        note: String::new(),
    };
    if let Some(mine) = my_ip(&state, &headers, peer) {
        if let Some(lost) = vuuut_core::firewall::would_lock_out(&live, &candidate, mine) {
            return flash_to(
                FIREWALL_PATH,
                "error",
                &format!(
                    "That rule would lock you out of {}. Your address is {mine}. Nothing was saved.",
                    match lost {
                        vuuut_core::firewall::Surface::Admin => "the admin",
                        vuuut_core::firewall::Surface::Public => "this site",
                    },
                ),
            );
        }
    }

    let new = fw_repo::NewRule {
        action,
        scope,
        pattern,
        note: &vuuut_core::firewall::clean_note(&form.note),
        created_by: Some(&actor.user.id),
        expires_at,
    };
    if let Err(e) = fw_repo::upsert(&state.db, &new).await {
        tracing::error!("firewall: saving a rule failed: {}", e.message());
        return flash_to(FIREWALL_PATH, "error", "The rule could not be saved.");
    }
    state.firewall.reload(&state.db).await;

    // Adding the first rule turns the firewall on, so the switch is never the
    // thing somebody forgot — a stored rule that is quietly not enforced is
    // worse than no rule, because the screen says it is there.
    let mut message = format!("Rule added for {pattern}.");
    if !state.settings.get_bool("firewall_enabled") {
        if state.settings.set(&state.db, "firewall_enabled", &serde_json::Value::Bool(true)).await.is_ok() {
            message.push_str(" The firewall is now on.");
        }
    }
    let _ = repos::actions::record(&state.db, "setting", "firewall", &actor.user.id, "edited").await;
    flash_to(FIREWALL_PATH, "ok", &message)
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct RuleIdForm {
    id: String,
}

/// Remove one rule, refusing a removal that would lock the author out.
///
/// Deleting an *allow* can lock you out as thoroughly as adding a block: take
/// away the only admin allow rule covering your own address while others
/// remain, and allowlist mode refuses you on the next request.
pub(crate) async fn firewall_delete(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<RuleIdForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let id = form.id.trim();
    if !vuuut_core::ids::is_objectid(id) {
        return flash_to(FIREWALL_PATH, "error", "That is not a rule.");
    }
    let existing = match fw_repo::all(&state.db).await {
        Ok(r) => r,
        Err(e) => return server_error(&e),
    };
    let live: Vec<vuuut_core::firewall::Rule> =
        existing.iter().filter_map(|r| r.as_rule()).collect();
    if let Some(mine) = my_ip(&state, &headers, peer) {
        if vuuut_core::firewall::removal_would_lock_out(&live, id, mine).is_some() {
            return flash_to(
                FIREWALL_PATH,
                "error",
                "Removing that rule would lock you out of the admin — it is the allow rule that \
                 covers your own address. Remove the other allow rules first.",
            );
        }
    }
    match fw_repo::delete(&state.db, id).await {
        Ok(true) => {
            state.firewall.reload(&state.db).await;
            let _ = repos::actions::record(&state.db, "setting", "firewall", &actor.user.id, "edited").await;
            flash_to(FIREWALL_PATH, "ok", "Rule removed.")
        }
        Ok(false) => flash_to(FIREWALL_PATH, "ok", "That rule had already gone."),
        Err(e) => {
            tracing::error!("firewall: removing a rule failed: {}", e.message());
            flash_to(FIREWALL_PATH, "error", "The rule could not be removed.")
        }
    }
}

/// Remove every rule. Always safe — an empty firewall lets everybody through,
/// so this is the way out of a firewall somebody has tangled and it needs no
/// lockout check.
pub(crate) async fn firewall_clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match fw_repo::delete_all(&state.db).await {
        Ok(n) => {
            state.firewall.reload(&state.db).await;
            let _ = repos::actions::record(&state.db, "setting", "firewall", &actor.user.id, "edited").await;
            flash_to(
                FIREWALL_PATH,
                "ok",
                &format!("{} removed. The firewall lets everybody through.", crate::countable(n as i64, "rule")),
            )
        }
        Err(e) => {
            tracing::error!("firewall: clearing the rules failed: {}", e.message());
            flash_to(FIREWALL_PATH, "error", "The rules could not be removed.")
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ToggleForm {
    enabled: String,
}

/// Turn the whole firewall on or off.
///
/// Turning it **on** runs the same lockout simulation an added rule does: the
/// rules may have been written while it was off, and switching it on is
/// exactly as capable of locking somebody out as adding the last of them.
/// Turning it off is always allowed and never checked — it is the undo.
pub(crate) async fn firewall_toggle(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<ToggleForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let on = form.enabled.trim() == "1";

    if on {
        let existing = match fw_repo::all(&state.db).await {
            Ok(r) => r,
            Err(e) => return server_error(&e),
        };
        let live: Vec<vuuut_core::firewall::Rule> =
            existing.iter().filter_map(|r| r.as_rule()).collect();
        if let Some(mine) = my_ip(&state, &headers, peer) {
            for surface in
                [vuuut_core::firewall::Surface::Admin, vuuut_core::firewall::Surface::Public]
            {
                if vuuut_core::firewall::decide(&live, mine, surface).is_block() {
                    return flash_to(
                        FIREWALL_PATH,
                        "error",
                        &format!(
                            "Turning the firewall on would lock you out — the rules already stored \
                             refuse your address, {mine}. Remove or change them first."
                        ),
                    );
                }
            }
        }
    }

    match state.settings.set(&state.db, "firewall_enabled", &serde_json::Value::Bool(on)).await {
        Ok(()) => {
            let _ = repos::actions::record(&state.db, "setting", "firewall", &actor.user.id, "edited").await;
            flash_to(
                FIREWALL_PATH,
                "ok",
                if on { "The firewall is on." } else { "The firewall is off. No rule is enforced." },
            )
        }
        Err(e) => {
            tracing::error!("firewall: toggling failed: {}", e.message());
            flash_to(FIREWALL_PATH, "error", "That could not be changed.")
        }
    }
}
