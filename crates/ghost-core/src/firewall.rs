//! The firewall's rules: what an address rule is, and who it lets through.
//!
//! Pure, with no database and no HTTP types, so the middleware that runs on
//! every request, the admin screen that edits the rules and the tests all
//! agree about what a rule means. This is a trust boundary and the module
//! that says so: everything here decides whether a request is answered at
//! all.
//!
//! ## What a rule is
//!
//! A [`Rule`] is an action (block or allow), a scope (the whole site, or the
//! admin only), and a [`Pattern`] — a single address or a CIDR range, v4 or
//! v6. Optionally an expiry, because most blocks should not be permanent: an
//! address that attacked the site in March is usually somebody else's by
//! June, and a blocklist nobody prunes is a blocklist that eventually refuses
//! a customer.
//!
//! ## How a request is judged
//!
//! [`decide`] is the whole of it, and the order matters:
//!
//! 1. **An `Allow` rule matching the caller wins outright.** An allow is an
//!    exception somebody wrote deliberately; a block is often a range. If
//!    block won, you could not punch a hole in your own range block.
//! 2. **Otherwise a `Block` rule matching the caller refuses.**
//! 3. **Otherwise, if the scope is `Admin` and any admin `Allow` rule exists
//!    at all, the caller is refused.** This is the lock-the-admin-to-my-office
//!    mode, and it is the one piece of behaviour here that is not "a rule
//!    matched": adding the first admin allow rule changes the default for
//!    every address that is not on it. The admin screen says so in those
//!    words before you can save one.
//! 4. Otherwise the request is allowed.
//!
//! Site-scoped rules apply to the admin as well. An address barred from the
//! site is barred from everything; the `Admin` scope exists to bar somebody
//! from `/ghost/` *without* barring them from reading the publication.
//!
//! ## Why this cannot lock you out by accident
//!
//! [`would_lock_out`] is what the admin screen calls before saving. A rule
//! that would refuse the address the author is currently connected from is
//! refused at save time, with the address named. It is not a complete
//! guarantee — somebody can still block the range their colleague is on — but
//! it stops the single mistake that cannot be undone from the admin, which is
//! locking yourself out of the screen you would fix it from.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// ---------------------------------------------------------------------------
// patterns
// ---------------------------------------------------------------------------

/// How many rules one site may have.
///
/// Every rule is compared against the caller on every request, so this is a
/// per-request cost, and the list is held in memory. Two thousand single
/// addresses is a long blocklist for a publication; somebody who needs more
/// than that needs a real firewall in front of the process, not this one.
pub const MAX_RULES: usize = 2000;

/// The longest a note may be. It is shown on the rules screen and nowhere
/// else.
pub const NOTE_MAX: usize = 200;

/// A single address or a CIDR range.
///
/// Stored normalized — `1.2.3.4` rather than `001.002.003.004`, and
/// `10.0.0.0/8` rather than `10.1.2.3/8` — so the same range cannot be
/// entered twice in two spellings, and so the screen shows back what it will
/// actually match rather than what was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern {
    base: IpAddr,
    /// Prefix length in bits. 32 for a bare v4 address, 128 for a bare v6.
    bits: u8,
}

impl Pattern {
    /// Parse `1.2.3.4`, `10.0.0.0/8`, `2001:db8::1` or `2001:db8::/32`.
    ///
    /// Returns `None` for anything else, including a prefix longer than the
    /// family allows and a range written with a negative or non-numeric
    /// prefix. Nothing here guesses: a rule that was not understood must not
    /// be stored as some other rule that was.
    pub fn parse(raw: &str) -> Option<Pattern> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (addr, bits) = match raw.split_once('/') {
            Some((addr, prefix)) => {
                let bits: u8 = prefix.trim().parse().ok()?;
                (addr.trim(), Some(bits))
            }
            None => (raw, None),
        };
        // A bracketed v6 literal is what a `Host` header carries; accept it
        // so a value copied out of a log works.
        let addr = addr.trim_start_matches('[').trim_end_matches(']');
        let base: IpAddr = addr.parse().ok()?;
        let max = if base.is_ipv4() { 32 } else { 128 };
        let bits = bits.unwrap_or(max);
        if bits > max {
            return None;
        }
        Some(Pattern { base, bits }.normalized())
    }

    /// Zero every bit below the prefix, so `10.1.2.3/8` and `10.0.0.0/8` are
    /// one pattern rather than two that behave identically.
    fn normalized(self) -> Pattern {
        let base = match self.base {
            IpAddr::V4(v4) => IpAddr::V4(Ipv4Addr::from(mask_u32(v4.into(), self.bits))),
            IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(mask_u128(v6.into(), self.bits))),
        };
        Pattern { base, bits: self.bits }
    }

    /// Whether `ip` falls inside this pattern.
    ///
    /// A v4-mapped v6 address (`::ffff:1.2.3.4`, which is how a dual-stack
    /// socket reports an IPv4 client) is compared as the v4 address it
    /// carries. Without that, a v4 block silently fails to match exactly the
    /// callers it was written for, on exactly the servers most likely to be
    /// running this.
    pub fn matches(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
            v4 => v4,
        };
        match (self.base, ip) {
            (IpAddr::V4(base), IpAddr::V4(ip)) => {
                mask_u32(ip.into(), self.bits) == mask_u32(base.into(), self.bits)
            }
            (IpAddr::V6(base), IpAddr::V6(ip)) => {
                mask_u128(ip.into(), self.bits) == mask_u128(base.into(), self.bits)
            }
            // Different families never match. A v4 rule does not bar a v6
            // caller, which is worth knowing: a site that blocks an address
            // may still hear from the same person over v6.
            _ => false,
        }
    }

    /// Whether this pattern is a single address rather than a range.
    pub fn is_single(&self) -> bool {
        self.bits == if self.base.is_ipv4() { 32 } else { 128 }
    }

    /// How many addresses this pattern covers, saturating — a /0 is "the
    /// whole internet" and the screen says so rather than printing a number.
    pub fn size_label(&self) -> String {
        if self.is_single() {
            return "1 address".into();
        }
        let host_bits = (if self.base.is_ipv4() { 32 } else { 128 }) - self.bits;
        if host_bits >= 64 {
            return "an enormous range".into();
        }
        let n = 1u128 << host_bits;
        if self.bits == 0 {
            return "every address".into();
        }
        format!("{n} addresses")
    }
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_single() {
            write!(f, "{}", self.base)
        } else {
            write!(f, "{}/{}", self.base, self.bits)
        }
    }
}

fn mask_u32(value: u32, bits: u8) -> u32 {
    if bits == 0 {
        0
    } else {
        value & (u32::MAX << (32 - bits.min(32)))
    }
}

fn mask_u128(value: u128, bits: u8) -> u128 {
    if bits == 0 {
        0
    } else {
        value & (u128::MAX << (128 - bits.min(128)))
    }
}

// ---------------------------------------------------------------------------
// rules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Block,
    Allow,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Block => "block",
            Action::Allow => "allow",
        }
    }

    /// Anything unrecognised is a **block**, not an allow.
    ///
    /// The safe reading of a row nobody can parse: a corrupted rule that
    /// refuses a request is visible and fixable, and one that admits a
    /// request is neither.
    pub fn parse(raw: &str) -> Action {
        match raw.trim() {
            "allow" => Action::Allow,
            _ => Action::Block,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The whole site, admin included.
    Site,
    /// `/ghost/` only. The publication stays readable.
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Site => "site",
            Scope::Admin => "admin",
        }
    }

    /// Anything unrecognised is **admin**, the narrower of the two.
    ///
    /// The mirror of [`Action::parse`]'s argument: for a scope the safe
    /// reading is the one that affects less, because a rule that was meant to
    /// bar somebody from the admin and instead bars them from the whole site
    /// takes the publication down for a reader who did nothing.
    pub fn parse(raw: &str) -> Scope {
        match raw.trim() {
            "site" => Scope::Site,
            _ => Scope::Admin,
        }
    }
}

/// One firewall rule, as the middleware sees it.
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub action: Action,
    pub scope: Scope,
    pub pattern: Pattern,
    pub note: String,
}

/// Which half of the site a request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Anything under `/ghost/`.
    Admin,
    /// The published site and its public APIs.
    Public,
}

/// What the firewall decided, and which rule decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Refused. Carries the id of the rule responsible, so the middleware can
    /// count the hit against it — `None` for the admin-allowlist default,
    /// which is refused by the *absence* of a rule rather than by one.
    Block { rule: Option<String> },
}

impl Decision {
    pub fn is_block(&self) -> bool {
        matches!(self, Decision::Block { .. })
    }
}

/// Whether `ip` may reach `surface`, given `rules`.
///
/// See the module doc for the order and why it is that way. `rules` is every
/// live rule — expired ones are dropped by the repository before they get
/// here, so that "is this rule still in force" is answered once per request
/// rather than once per rule.
pub fn decide(rules: &[Rule], ip: IpAddr, surface: Surface) -> Decision {
    let applies = |rule: &Rule| match (rule.scope, surface) {
        // A site rule covers the admin too: an address barred from the site
        // is barred from everything.
        (Scope::Site, _) => true,
        (Scope::Admin, Surface::Admin) => true,
        (Scope::Admin, Surface::Public) => false,
    };

    // 1. An explicit allow wins outright, so a hole can be punched in a range
    //    block.
    if rules.iter().any(|r| r.action == Action::Allow && applies(r) && r.pattern.matches(ip)) {
        return Decision::Allow;
    }

    // 2. Any block that matches.
    if let Some(rule) =
        rules.iter().find(|r| r.action == Action::Block && applies(r) && r.pattern.matches(ip))
    {
        return Decision::Block { rule: Some(rule.id.clone()) };
    }

    // 3. Allowlist mode: the first admin allow rule closes the admin to
    //    everything that is not on the list. Site-scoped allow rules do *not*
    //    do this — a public site that answered only a list of addresses is
    //    not a publication, and somebody reaching for "allow my office" on
    //    the public side has misunderstood rather than chosen.
    if surface == Surface::Admin
        && rules.iter().any(|r| r.action == Action::Allow && r.scope == Scope::Admin)
    {
        return Decision::Block { rule: None };
    }

    Decision::Allow
}

/// Whether adding `candidate` to `existing` would refuse `self_ip`.
///
/// The check the admin screen makes before saving. Returns the surface that
/// would be lost, so the refusal can say "this would lock you out of the
/// admin" rather than something vaguer.
///
/// Both surfaces are tested, not just the admin: barring yourself from the
/// public site is less catastrophic but still almost never what was meant,
/// and the person saving the rule is the one person guaranteed to be able to
/// tell us it was a mistake.
pub fn would_lock_out(existing: &[Rule], candidate: &Rule, self_ip: IpAddr) -> Option<Surface> {
    let mut rules = existing.to_vec();
    rules.push(candidate.clone());
    for surface in [Surface::Admin, Surface::Public] {
        if decide(&rules, self_ip, surface).is_block() {
            return Some(surface);
        }
    }
    None
}

/// Whether removing the rule with `id` would refuse `self_ip`.
///
/// Deleting an *allow* rule can lock you out just as thoroughly as adding a
/// block: take away the only admin allow rule covering your own address while
/// others remain, and allowlist mode refuses you on the next request.
pub fn removal_would_lock_out(existing: &[Rule], id: &str, self_ip: IpAddr) -> Option<Surface> {
    let rules: Vec<Rule> = existing.iter().filter(|r| r.id != id).cloned().collect();
    for surface in [Surface::Admin, Surface::Public] {
        if decide(&rules, self_ip, surface).is_block() {
            return Some(surface);
        }
    }
    None
}

/// Tidy a rule's note: no control characters, bounded, trimmed.
///
/// Rebuilt rather than stored as sent, which is rule 2 of the module pattern.
/// The note is shown on the rules screen and is escaped there like everything
/// else; what this stops is a newline or a NUL making one rule look like two
/// in the list.
pub fn clean_note(raw: &str) -> String {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    cleaned.trim().chars().take(NOTE_MAX).collect::<String>().trim().to_string()
}

// ---------------------------------------------------------------------------
// spam
// ---------------------------------------------------------------------------

/// The `spam_*` and firewall settings, with the defaults that reproduce the
/// behaviour the site had before this half of the module existed.
///
/// Unlike the `security_*` switches, **every one of these is off by default**.
/// The argument that justified shipping the headers on does not reach here:
/// each of these refuses somebody, and a site that starts silently turning
/// away signups or requests because a module was installed has had a decision
/// made for it that it cannot see being made.
pub const BOOL_SETTINGS: &[(&str, bool)] = &[
    // Enforce the rules in `firewall_rules` at all. Off until the first rule
    // is added, so an empty firewall costs nothing and cannot surprise
    // anybody; the admin screen turns it on when a rule is saved.
    ("firewall_enabled", false),
    // Refuse member signups from the built-in disposable-address list, on top
    // of whatever is in Settings → Membership → Spam filters.
    ("spam_block_disposable_email", false),
    // Refuse requests whose User-Agent matches `spam_blocked_user_agents`.
    ("spam_block_user_agents", false),
    // Require a member signup to carry the same origin, like the forms
    // endpoint already does.
    ("spam_signup_same_origin", false),
];

/// The default for one of [`BOOL_SETTINGS`], or `false` for a key that is not
/// one. Every one of them is `false`, but the function exists so that adding
/// a switch that is not cannot silently read as off.
pub fn spam_default(key: &str) -> bool {
    BOOL_SETTINGS.iter().find(|(k, _)| *k == key).map(|(_, d)| *d).unwrap_or(false)
}

/// Read one `spam_*` or `firewall_*` switch out of a JSON object.
///
/// The twin of [`crate::security::flag`], and separate from it because the
/// two tables of defaults are separate: there, a missing row means the
/// defence is *on*; here it means the filter is *off*. Sharing one function
/// would mean sharing one table, and a key looked up in the wrong one reads
/// as the wrong default — which for a filter that refuses people is the
/// failure that is hardest to notice.
pub fn flag_spam(container: &serde_json::Value, key: &str) -> bool {
    match container.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => match s.trim() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => spam_default(key),
        },
        Some(serde_json::Value::Number(n)) => {
            n.as_i64().map(|i| i != 0).unwrap_or_else(|| spam_default(key))
        }
        _ => spam_default(key),
    }
}

/// The most a user-agent blocklist may be, in characters. Each line is
/// compared against every request's `User-Agent`, so this is a per-request
/// cost and not a place to paste a threat feed.
pub const USER_AGENTS_MAX: usize = 4000;

/// The most lines a user-agent blocklist may have.
pub const USER_AGENTS_MAX_LINES: usize = 100;

/// Disposable and throwaway email domains, for `spam_block_disposable_email`.
///
/// Deliberately short and deliberately conservative. A long list scraped from
/// a threat feed goes stale, grows false positives, and eventually refuses a
/// real reader with no way for them to tell you — the failure mode of an
/// email blocklist is silent and lands on somebody who did nothing wrong. So
/// this is the handful of services whose entire purpose is a ten-minute
/// address, and a site that wants more adds them in Settings → Membership →
/// Spam filters, where it can see what it chose.
pub const DISPOSABLE_DOMAINS: &[&str] = &[
    "0-mail.com",
    "10minutemail.com",
    "20minutemail.com",
    "33mail.com",
    "anonbox.net",
    "burnermail.io",
    "dispostable.com",
    "fakeinbox.com",
    "getairmail.com",
    "getnada.com",
    "guerrillamail.com",
    "guerrillamail.net",
    "guerrillamail.org",
    "inboxbear.com",
    "mailcatch.com",
    "maildrop.cc",
    "mailinator.com",
    "mailnesia.com",
    "mintemail.com",
    "mohmal.com",
    "mytemp.email",
    "sharklasers.com",
    "spam4.me",
    "temp-mail.org",
    "tempmail.com",
    "tempmailo.com",
    "tempr.email",
    "throwawaymail.com",
    "trashmail.com",
    "yopmail.com",
    "yopmail.net",
];

/// Whether an address is at one of [`DISPOSABLE_DOMAINS`].
///
/// Matches the domain **and its subdomains**, because every one of these
/// services hands out `something.mailinator.com` as readily as the apex. The
/// comparison is on the part after the last `@`, lowercased; an address with
/// no `@` is not a domain we know and is left to the validator that does know
/// what an address is.
pub fn is_disposable_email(email: &str) -> bool {
    let Some((_, domain)) = email.trim().rsplit_once('@') else { return false };
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    DISPOSABLE_DOMAINS
        .iter()
        .any(|bad| domain == *bad || domain.ends_with(&format!(".{bad}")))
}

/// Parse a user-agent blocklist from the textarea's text.
///
/// One fragment per line, lowercased and trimmed, blanks and `#` comments
/// dropped, bounded by [`USER_AGENTS_MAX_LINES`]. Fragments rather than
/// patterns: a regular expression here would be a regular expression
/// evaluated against a header on every request, which is a denial of service
/// somebody can write into their own settings by accident.
pub fn parse_user_agents(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| line.trim().to_ascii_lowercase())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(USER_AGENTS_MAX_LINES)
        .collect()
}

/// Whether a `User-Agent` contains any blocked fragment.
///
/// Substring, case-insensitive. An **empty or absent** User-Agent is *not*
/// blocked: plenty of legitimate clients send none, and a rule that refuses
/// them refuses feed readers and monitoring rather than spammers.
pub fn user_agent_blocked(user_agent: &str, blocked: &[String]) -> bool {
    if blocked.is_empty() {
        return false;
    }
    let ua = user_agent.trim().to_ascii_lowercase();
    if ua.is_empty() {
        return false;
    }
    blocked.iter().any(|fragment| ua.contains(fragment.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn rule(id: &str, action: Action, scope: Scope, pattern: &str) -> Rule {
        Rule {
            id: id.into(),
            action,
            scope,
            pattern: Pattern::parse(pattern).unwrap(),
            note: String::new(),
        }
    }

    // --- patterns --------------------------------------------------------

    #[test]
    fn a_single_address_and_a_range_both_parse_and_print_back() {
        let one = Pattern::parse("203.0.113.4").unwrap();
        assert!(one.is_single());
        assert_eq!(one.to_string(), "203.0.113.4");

        let many = Pattern::parse("203.0.113.0/24").unwrap();
        assert!(!many.is_single());
        assert_eq!(many.to_string(), "203.0.113.0/24");
        assert_eq!(many.size_label(), "256 addresses");
    }

    #[test]
    fn a_range_is_normalized_so_one_range_is_one_rule() {
        // `10.1.2.3/8` and `10.0.0.0/8` match identically; stored differently
        // they would be two rules in the list that cannot both be right.
        assert_eq!(Pattern::parse("10.1.2.3/8").unwrap().to_string(), "10.0.0.0/8");
        assert_eq!(
            Pattern::parse("2001:db8:1234::1/32").unwrap().to_string(),
            "2001:db8::/32"
        );
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        for bad in [
            "",
            "   ",
            "not-an-address",
            "203.0.113.4/33",   // past the family's maximum
            "2001:db8::/129",
            "203.0.113.4/-1",
            "203.0.113.4/abc",
            "203.0.113.999",
            "203.0.113.4/",
            "example.com",      // a name is not an address
        ] {
            assert!(Pattern::parse(bad).is_none(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_bracketed_v6_literal_copied_out_of_a_log_works() {
        assert_eq!(Pattern::parse("[2001:db8::1]").unwrap().to_string(), "2001:db8::1");
    }

    #[test]
    fn a_range_matches_inside_and_not_outside() {
        let p = Pattern::parse("203.0.113.0/24").unwrap();
        assert!(p.matches(ip("203.0.113.0")));
        assert!(p.matches(ip("203.0.113.255")));
        assert!(!p.matches(ip("203.0.114.0")));
        assert!(!p.matches(ip("203.0.112.255")));
    }

    #[test]
    fn a_v4_rule_matches_a_dual_stack_servers_v4_mapped_client() {
        // `::ffff:203.0.113.4` is how a socket bound to v6 reports an IPv4
        // caller. Without the unmapping, a v4 block silently fails to match
        // exactly the callers it was written for, on exactly the servers most
        // likely to be running this.
        let p = Pattern::parse("203.0.113.4").unwrap();
        assert!(p.matches(ip("::ffff:203.0.113.4")));
        let range = Pattern::parse("203.0.113.0/24").unwrap();
        assert!(range.matches(ip("::ffff:203.0.113.9")));
    }

    #[test]
    fn a_v4_rule_does_not_match_a_real_v6_caller() {
        let p = Pattern::parse("0.0.0.0/0").unwrap();
        assert!(p.matches(ip("203.0.113.4")));
        assert!(!p.matches(ip("2001:db8::1")), "a v4 /0 must not bar the v6 internet");
    }

    #[test]
    fn a_zero_prefix_is_every_address_of_its_family() {
        let p = Pattern::parse("0.0.0.0/0").unwrap();
        assert_eq!(p.size_label(), "every address");
        for a in ["1.1.1.1", "203.0.113.4", "10.0.0.1"] {
            assert!(p.matches(ip(a)));
        }
    }

    // --- decisions -------------------------------------------------------

    #[test]
    fn an_empty_firewall_lets_everybody_through() {
        for surface in [Surface::Admin, Surface::Public] {
            assert_eq!(decide(&[], ip("203.0.113.4"), surface), Decision::Allow);
        }
    }

    #[test]
    fn a_block_refuses_and_names_the_rule_that_did_it() {
        let rules = vec![rule("r1", Action::Block, Scope::Site, "203.0.113.0/24")];
        assert_eq!(
            decide(&rules, ip("203.0.113.4"), Surface::Public),
            Decision::Block { rule: Some("r1".into()) },
        );
        assert_eq!(decide(&rules, ip("198.51.100.1"), Surface::Public), Decision::Allow);
    }

    #[test]
    fn an_allow_punches_a_hole_in_a_range_block() {
        // The reason allow is evaluated first. If block won, you could not
        // block a range and admit one address inside it — which is the most
        // common thing anybody actually wants.
        let rules = vec![
            rule("block", Action::Block, Scope::Site, "203.0.113.0/24"),
            rule("allow", Action::Allow, Scope::Site, "203.0.113.4"),
        ];
        assert_eq!(decide(&rules, ip("203.0.113.4"), Surface::Public), Decision::Allow);
        assert!(decide(&rules, ip("203.0.113.5"), Surface::Public).is_block());
    }

    #[test]
    fn a_site_rule_covers_the_admin_but_an_admin_rule_leaves_the_site_readable() {
        let site = vec![rule("r", Action::Block, Scope::Site, "203.0.113.4")];
        assert!(decide(&site, ip("203.0.113.4"), Surface::Public).is_block());
        assert!(decide(&site, ip("203.0.113.4"), Surface::Admin).is_block());

        let admin = vec![rule("r", Action::Block, Scope::Admin, "203.0.113.4")];
        assert_eq!(
            decide(&admin, ip("203.0.113.4"), Surface::Public),
            Decision::Allow,
            "an admin rule took the publication away from a reader",
        );
        assert!(decide(&admin, ip("203.0.113.4"), Surface::Admin).is_block());
    }

    #[test]
    fn the_first_admin_allow_rule_closes_the_admin_to_everybody_else() {
        // The one piece of behaviour that is not "a rule matched": adding
        // this rule changes the default for every address that is not on it.
        let rules = vec![rule("office", Action::Allow, Scope::Admin, "203.0.113.0/24")];
        assert_eq!(decide(&rules, ip("203.0.113.4"), Surface::Admin), Decision::Allow);
        assert_eq!(
            decide(&rules, ip("198.51.100.1"), Surface::Admin),
            Decision::Block { rule: None },
            "allowlist mode did not close the admin",
        );
        // And the publication is untouched.
        assert_eq!(decide(&rules, ip("198.51.100.1"), Surface::Public), Decision::Allow);
    }

    #[test]
    fn a_site_allow_rule_does_not_close_the_public_site() {
        // A publication that answered only a list of addresses is not a
        // publication. Somebody reaching for "allow my office" on the public
        // side has misunderstood rather than chosen.
        let rules = vec![rule("office", Action::Allow, Scope::Site, "203.0.113.0/24")];
        assert_eq!(decide(&rules, ip("198.51.100.1"), Surface::Public), Decision::Allow);
        assert_eq!(decide(&rules, ip("198.51.100.1"), Surface::Admin), Decision::Allow);
    }

    // --- the lockout guard -----------------------------------------------

    #[test]
    fn a_rule_that_would_lock_the_author_out_is_detected() {
        let me = ip("203.0.113.4");
        let candidate = rule("new", Action::Block, Scope::Admin, "203.0.113.0/24");
        assert_eq!(would_lock_out(&[], &candidate, me), Some(Surface::Admin));

        // Blocking somebody else is fine.
        let other = rule("new", Action::Block, Scope::Admin, "198.51.100.0/24");
        assert_eq!(would_lock_out(&[], &other, me), None);
    }

    #[test]
    fn the_first_admin_allow_rule_is_caught_when_it_does_not_cover_the_author() {
        // The nastiest version: the rule names somebody *else's* address, and
        // the lockout is caused by allowlist mode switching on rather than by
        // the rule matching anybody.
        let me = ip("203.0.113.4");
        let candidate = rule("office", Action::Allow, Scope::Admin, "198.51.100.0/24");
        assert_eq!(would_lock_out(&[], &candidate, me), Some(Surface::Admin));

        // Naming the author's own address is what makes it safe.
        let safe = rule("office", Action::Allow, Scope::Admin, "203.0.113.0/24");
        assert_eq!(would_lock_out(&[], &safe, me), None);
    }

    #[test]
    fn deleting_the_allow_rule_that_covers_you_is_caught_too() {
        // Deleting an allow can lock you out as thoroughly as adding a block:
        // remove the only admin allow covering your address while others
        // remain, and allowlist mode refuses you on the next request.
        let me = ip("203.0.113.4");
        let existing = vec![
            rule("mine", Action::Allow, Scope::Admin, "203.0.113.0/24"),
            rule("theirs", Action::Allow, Scope::Admin, "198.51.100.0/24"),
        ];
        assert_eq!(removal_would_lock_out(&existing, "mine", me), Some(Surface::Admin));
        assert_eq!(removal_would_lock_out(&existing, "theirs", me), None);
    }

    #[test]
    fn removing_the_last_allow_rule_is_safe_because_it_leaves_no_allowlist() {
        let me = ip("203.0.113.4");
        let existing = vec![rule("theirs", Action::Allow, Scope::Admin, "198.51.100.0/24")];
        // With it gone there is no allowlist at all, so everybody is let
        // through — including the author, who was locked out until now.
        assert_eq!(removal_would_lock_out(&existing, "theirs", me), None);
    }

    // --- parsing of the enum-ish strings ---------------------------------

    #[test]
    fn an_unreadable_action_is_a_block_and_an_unreadable_scope_is_admin_only() {
        // A corrupted rule that refuses a request is visible and fixable; one
        // that admits a request is neither. The scope goes the other way,
        // because a rule that was meant for the admin and instead takes the
        // publication down hurts a reader who did nothing.
        for junk in ["", "ALLOW", "permit", "nonsense", "block "] {
            assert_eq!(Action::parse(junk), Action::Block, "{junk:?}");
        }
        assert_eq!(Action::parse("allow"), Action::Allow);
        for junk in ["", "SITE", "everything", "nonsense"] {
            assert_eq!(Scope::parse(junk), Scope::Admin, "{junk:?}");
        }
        assert_eq!(Scope::parse("site"), Scope::Site);
    }

    #[test]
    fn a_note_is_rebuilt_rather_than_stored_as_sent() {
        assert_eq!(clean_note("  a reason  "), "a reason");
        assert_eq!(clean_note("two\nlines"), "twolines", "a newline would look like two rules");
        assert_eq!(clean_note("nul\0byte"), "nulbyte");
        assert_eq!(clean_note(&"x".repeat(500)).len(), NOTE_MAX);
    }

    // --- spam ------------------------------------------------------------

    #[test]
    fn a_disposable_address_is_caught_at_the_apex_and_at_a_subdomain() {
        assert!(is_disposable_email("someone@mailinator.com"));
        assert!(is_disposable_email("someone@sub.mailinator.com"));
        assert!(is_disposable_email("  SOMEONE@Mailinator.COM  "));
        // A trailing dot is a fully-qualified name, not a different domain.
        assert!(is_disposable_email("someone@mailinator.com."));
    }

    #[test]
    fn a_real_address_is_not_caught_by_a_domain_that_merely_ends_the_same_way() {
        // `notmailinator.com` ends with `mailinator.com` as a *string*. The
        // check is on label boundaries for exactly this reason.
        assert!(!is_disposable_email("someone@notmailinator.com"));
        assert!(!is_disposable_email("someone@example.com"));
        assert!(!is_disposable_email("someone@gmail.com"));
        assert!(!is_disposable_email("not-an-address"));
        assert!(!is_disposable_email("someone@"));
        assert!(!is_disposable_email(""));
    }

    #[test]
    fn a_user_agent_list_is_fragments_and_not_patterns() {
        let list = parse_user_agents("  SemrushBot \n\n# a comment\nahrefsbot\n");
        assert_eq!(list, vec!["semrushbot", "ahrefsbot"]);
        assert!(user_agent_blocked("Mozilla/5.0 (compatible; SemrushBot/7~bl)", &list));
        assert!(!user_agent_blocked("Mozilla/5.0 (Macintosh)", &list));
    }

    #[test]
    fn an_absent_user_agent_is_not_blocked() {
        // Plenty of legitimate clients send none — feed readers, monitoring,
        // curl in somebody's cron. A rule that refuses them refuses the wrong
        // people, and silently.
        let list = parse_user_agents("badbot");
        assert!(!user_agent_blocked("", &list));
        assert!(!user_agent_blocked("   ", &list));
        // And an empty list never blocks anything, whatever arrives.
        assert!(!user_agent_blocked("badbot", &[]));
    }

    #[test]
    fn the_user_agent_list_is_bounded() {
        let many = (0..500).map(|i| format!("bot{i}")).collect::<Vec<_>>().join("\n");
        assert_eq!(parse_user_agents(&many).len(), USER_AGENTS_MAX_LINES);
    }

    #[test]
    fn a_missing_spam_row_reads_as_off_and_a_missing_security_row_reads_as_on() {
        // The two tables of defaults point opposite ways, and this is why
        // they are two functions rather than one: `security_headers` absent
        // means the defence is on, `spam_block_user_agents` absent means the
        // filter is off. A key looked up in the wrong table reads as the
        // wrong default, and for a filter that refuses people that is the
        // failure hardest to notice.
        let empty = serde_json::json!({});
        assert!(!flag_spam(&empty, "spam_block_user_agents"));
        assert!(!flag_spam(&empty, "firewall_enabled"));
        assert!(crate::security::flag(&empty, "security_headers"));

        // And both accept the string shape older rows and form posts produce.
        let on = serde_json::json!({ "spam_block_user_agents": "true" });
        assert!(flag_spam(&on, "spam_block_user_agents"));
        let off = serde_json::json!({ "spam_block_user_agents": "false" });
        assert!(!flag_spam(&off, "spam_block_user_agents"));
    }

    #[test]
    fn every_spam_and_firewall_switch_is_off_by_default() {
        // Unlike the `security_*` switches: each of these refuses somebody,
        // and a site that starts turning away signups because a module was
        // installed has had a decision made for it that it cannot see.
        for (key, default) in BOOL_SETTINGS {
            assert!(!default, "{key} ships on");
        }
    }
}
