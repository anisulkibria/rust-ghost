//! What is inside each tool, so the Tools hub's search box can find a tool by
//! anything on its screens — "firewall" finds Security, "synonyms" finds
//! Search and Filtering, and the name of a popup somebody made last week
//! finds Popups.
//!
//! Two halves:
//!
//! * [`CONTENT`] — each tool's own section headings and field names, as they
//!   read on its screens. Static, because the screens are. Taken from the
//!   pages themselves on 2026-11-21; when a screen gains a section, add its
//!   heading here (a test checks every tool has an entry).
//! * [`things_inside`] — the names of what staff have made inside a tool:
//!   popups, forms, automations, redirects, firewall rules, staff, custom
//!   integrations, declared cookies. Read per request, a handful of small
//!   queries; each list is bounded.
//!
//! Every phrase is user-visible text or staff-made content, so the hub
//! escapes it into a `data-` attribute and the script places matches with
//! `textContent` — never markup.
//!
//! The same index feeds the rail's search palette (`crate::search`), so a
//! tool is found by the same words in both boxes.

use super::*;

/// Each tool's sections and fields, by tool key. Titles and descriptions are
/// searched as well, so they are not repeated here.
pub(crate) const CONTENT: &[(&str, &[&str])] = &[
    ("ads", &["Ads", "Advertisers", "Sponsors", "Banner", "Image banner", "Text ad", "Ad network code", "AdSense", "Placement", "Sticky bar", "Ad slot", "data-ad-slot", "Impressions", "Clicks", "CTR", "CPM", "CPC", "Revenue", "ads.txt", "No ads for paying members", "Advertisement label", "Sponsored", "Campaign", "Schedule", "Frequency cap"]),
    ("workflows", &["Editorial board", "Kanban", "Stages", "Ideas", "Writing", "In review", "Approved", "Assigned to", "Reviewer", "Due date", "Overdue", "Priority", "Calendar", "Editorial calendar", "Checklist", "Notes", "History", "Require approval to publish", "Approval stage", "Work in progress limit"]),
    ("builder", &["Page builder", "Visual editor", "Drag and drop", "Sections", "Columns", "Widgets", "Heading", "Button", "Icon box", "Gallery", "Testimonial", "Accordion", "FAQ", "Pricing table", "Counter", "Video", "Map", "Templates", "Global colours", "Content width", "Responsive", "Mobile preview", "Landing page", "Elementor", "Hero section"]),
    ("caching", &["Page cache", "Purge everything", "Cached pages", "Hit rate", "Response time", "Slowest pages", "Cache-Control", "max-age", "s-maxage", "ETag", "304 Not Modified", "gzip", "Compression", "Lazy-load images", "Speculation rules", "Prerender", "Preconnect", "Cloudflare", "CDN", "Purge cache", "Performance", "Speed", "TTFB"]),
    ("ecom", &["Store", "Shop", "Products", "Variants", "Options", "Inventory", "Stock", "SKU", "Collections", "Smart collections", "Orders", "Fulfil", "Refund", "Customers", "Discount codes", "Coupons", "Shipping rates", "Taxes", "VAT", "Checkout", "Cart", "Abandoned checkouts", "Payments", "Bank transfer", "Cash on delivery", "Stripe", "Packing slip", "Refund policy", "Digital downloads", "Downloadable files", "Download limit", "Revoke downloads"]),
    ("directories", &["Directory", "Classifieds", "Listings", "Business listings", "For sale", "Wanted", "Jobs", "Categories", "Waiting for review", "Approve", "Reject", "Featured", "Expiry", "Renew", "Submit a listing", "Visitor submissions", "Price", "Location", "Contact details"]),
    ("dummies", &["Dummy content", "Demo content", "Sample content", "Demo site", "Placeholder posts", "Lorem ipsum", "Generate posts", "Generate members", "Test data", "Remove dummy content"]),
    ("analytics", &["Total views", "Unique visitors", "Top content", "Top sources", "Locations", "Campaigns", "Audience", "Signup sources", "New members per day", "Recent members", "Newsletters sent", "Traffic", "Referrers", "Web analytics"]),
    ("announcement", &["Announcement bar", "Banner text", "Background colour", "Visibility", "Visitors", "Free members", "Paid members", "Dismiss"]),
    ("automations", &["New automation", "Trigger", "Sequence", "Entries", "Exit automation", "Welcome email", "Delay", "Workflow", "Turn on", "Turn off"]),
    ("backups", &["Backup now", "Automatic backups", "Restore", "Download backup", "Export content", "Schedule", "Keep backups"]),
    ("code-injection", &["Site header", "Site footer", "Head", "Foot", "Scripts", "Styles", "Tracking code", "Google Analytics snippet"]),
    ("comments", &["All comments", "Reported", "Reply", "Commenting", "Moderation", "Hide comment", "Spam"]),
    ("cookies", &["Consent banner", "Cookie banner", "GDPR", "CCPA", "Opt-in", "Opt-out", "Consent model", "Compliance check", "Trackers found", "Found on this site", "Cookie declaration", "Add a cookie", "Consent log", "Proof of consent", "Ask everyone again", "Global Privacy Control", "Hold back embedded content", "Privacy policy link", "Delete records after"]),
    ("forms", &["Fields", "Responses", "Answers", "Notifications", "Email me about new responses", "Spam protection", "Hidden decoy field", "Minimum fill time", "Rate limits", "Privacy & retention", "Keep responses for", "Export CSV", "Contact form"]),
    ("growth", &["Recommendations", "Signup forms", "Offers", "Ghost Explore", "Embed signup form", "Discount", "Trial"]),
    ("history", &["Activity", "Changes", "Who changed it", "Audit log", "Staff actions"]),
    ("integrations", &["Built-in apps", "Custom integrations", "API keys", "Content API", "Admin API", "API URL", "Webhooks", "Unsplash", "Zapier", "Slack"]),
    ("languages", &["Default publication language", "Languages", "Translations", "Left to translate", "Language home pages", "Navigation per language", "Search engines", "hreflang", "Language prefix"]),
    ("machine-payments", &["Pay per article", "AI agents", "Stablecoin", "Card payments", "Price", "Paywall", "HTTP 402"]),
    ("media", &["Library", "Images", "Videos", "Files", "Upload", "Used by", "Where it lives", "Alt text", "Unused media", "Optimise images"]),
    ("memberships", &["Members", "Tiers", "Offers", "Growth", "Latest arrivals", "Where they came from", "What is in the way", "Monthly revenue", "Stripe", "Signup portal", "Paid members", "Free members"]),
    ("migrations", &["Import", "Export", "WordPress", "Substack", "Medium", "Ghost import", "Members CSV", "Universal import"]),
    ("navigations", &["Primary navigation", "Secondary navigation", "Menu items", "Footer links", "Language switcher"]),
    ("newsletters", &["Latest editions", "Delivery & measurement", "Open tracking", "Click tracking", "Newsletter sending", "Sending allowance", "Mailgun", "Retry failed edition", "Open rate", "Subscribers"]),
    ("permissions", &["Roles", "Administrator", "Editor", "Author", "Contributor", "Access", "View", "Edit", "Posts", "Pages", "Tags"]),
    ("popups", &["New popup", "Modal", "Slide-in", "Bar", "Who sees it", "What makes it appear", "Exit intent", "Scroll", "Frequency", "Leads", "Leads captured", "Collect email addresses", "Hide popups on phones", "Space between popups", "Consent note", "Lead generation"]),
    ("redirects", &["Rules", "Test an address", "301 Permanent", "302 Temporary", "redirects.yaml", "redirects.json", "Upload", "Download", "Old address", "New address"]),
    ("routes", &["Collections", "Custom routes", "Taxonomies", "Tag archive", "Author archive", "Posts per page", "Template", "routes.yaml", "Permalink", "Test an address"]),
    ("search", &["Engine", "Vuuut search", "Sodo search", "What can be found", "Include pages", "Members-only posts", "Search the whole post", "Full text", "Leave out posts tagged", "Ranking", "Weights", "Synonyms", "Results per search", "Placeholder", "Keyboard shortcut", "Try it", "Insights", "Most searched", "Found nothing", "Filters"]),
    ("security", &["Findings", "The ways in", "Signed in now", "Sessions", "Sign-in", "Two-factor", "2FA", "Firewall", "Block an address", "IP", "Rate limits", "Response headers", "HSTS", "Spam", "Disposable addresses", "Lockouts", "Shortest staff password", "Sign staff out after", "Trust proxy"]),
    ("seo", &["Search appearance", "Crawling", "robots.txt", "Sitemap", "IndexNow", "Structured data", "Meta description", "Social image", "Canonical", "Referrer policy", "What to fix first", "Site setup", "Average score", "noindex"]),
    ("social", &["Social web", "Notes", "Feed", "Followers", "Following", "Fediverse", "ActivityPub", "Publish note", "Suggested for you", "Blocked",
        "Social media", "X", "Twitter", "Bluesky", "Mastodon", "Threads", "LinkedIn", "Facebook", "Telegram", "Discord",
        "Connect an account", "Auto-share", "Share a post", "Schedule a share", "Share history", "Share buttons", "Hashtags", "UTM", "Campaign tags"]),
    ("staff", &["Invite people", "Pending invitations", "Roles", "Active people", "Password", "Email 2FA codes", "Remove from the team", "Social links", "Owner"]),
];

/// The phrases for one tool.
pub(crate) fn content_of(key: &str) -> &'static [&'static str] {
    CONTENT.iter().find(|(k, _)| *k == key).map(|(_, words)| *words).unwrap_or(&[])
}

/// How many of each kind of thing are listed. A site with five hundred
/// redirects is found by searching Redirects, not by shipping all of them.
const PER_TOOL: usize = 60;

/// Names of what staff have made inside each tool, labelled so a match says
/// what it is: `Popup: Spring sale`.
pub(crate) async fn things_inside(state: &AppState) -> HashMap<&'static str, Vec<String>> {
    let mut out: HashMap<&'static str, Vec<String>> = HashMap::new();
    let mut put = |key: &'static str, label: &str, name: &str| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let list = out.entry(key).or_default();
        if list.len() < PER_TOOL {
            list.push(format!("{label}: {name}"));
        }
    };
    for p in vuuut_db::repos::popups::list(&state.db).await.unwrap_or_default() {
        put("popups", "Popup", &p.name);
    }
    for f in vuuut_db::repos::forms::list(&state.db).await.unwrap_or_default() {
        put("forms", "Form", &f.name);
    }
    for a in vuuut_db::repos::automations::browse(&state.db, &["active", "inactive", "draft"]).await.unwrap_or_default() {
        put("automations", "Automation", &a.name);
    }
    for i in vuuut_db::repos::integrations::list_custom(&state.db).await.unwrap_or_default() {
        put("integrations", "Integration", &i.name);
    }
    for u in vuuut_db::repos::users::list(&state.db, false).await.unwrap_or_default() {
        put("staff", "Staff", u.name.as_deref().unwrap_or(""));
    }
    for r in vuuut_db::repos::firewall::all(&state.db).await.unwrap_or_default() {
        let what = if r.note.trim().is_empty() { r.pattern.clone() } else { format!("{} ({})", r.pattern, r.note) };
        put("security", "Firewall rule", &what);
    }
    for a in vuuut_db::repos::social_media::accounts(&state.db).await.unwrap_or_default() {
        let network = vuuut_core::social_media::Network::from_key(&a.network).map(|n| n.label()).unwrap_or("Account");
        put("social", network, &a.name);
    }
    for a in vuuut_db::repos::ads::list(&state.db).await.unwrap_or_default() {
        put("ads", "Ad", &a.name);
    }
    for a in vuuut_db::repos::ads::advertisers(&state.db).await.unwrap_or_default() {
        put("ads", "Advertiser", &a.name);
    }
    for c in vuuut_db::repos::ecom::collections(&state.db).await {
        put("ecom", "Collection", &c.title);
    }
    for c in vuuut_db::repos::directory::categories(&state.db).await.unwrap_or_default() {
        put("directories", "Category", &c.name);
    }
    for d in vuuut_db::repos::cookies::declarations(&state.db).await.unwrap_or_default() {
        put("cookies", "Cookie", &d.name);
    }
    {
        let redirects = state.redirects.read().await.clone();
        for r in redirects.rules() {
            put("redirects", "Redirect", &format!("{} → {}", r.from_raw, r.to));
        }
    }
    for key in ["navigation", "secondary_navigation"] {
        if let Some(serde_json::Value::Array(items)) = state.settings.get(key) {
            for item in items {
                if let Some(label) = item.get("label").and_then(|l| l.as_str()) {
                    put("navigations", "Menu item", label);
                }
            }
        }
    }
    out
}

/// The status line under each card that used to be a module, in the shape
/// `tool_states` gives the others: whether it is doing something, and one
/// short sentence.
pub(crate) async fn former_module_states(state: &AppState) -> Vec<(&'static str, (bool, String))> {
    let count = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql).fetch_one(&state.db).await.unwrap_or(0)
    };
    let plural = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut out = Vec::new();

    let es = vuuut_api::ecom::settings(state);
    let active = count("SELECT COUNT(*) FROM ecom_products WHERE status = 'active'").await;
    let to_ship = count("SELECT COUNT(*) FROM ecom_orders WHERE status = 'open' AND fulfillment_status = 'unfulfilled'").await;
    out.push(("ecom", (es.enabled, match (es.enabled, to_ship) {
        (_, n) if n > 0 => format!("{} to fulfil · {active} products", plural(n, "order", "orders")),
        (true, _) => format!("Open · {}", plural(active, "product", "products")),
        (false, _) => format!("Closed · {}", plural(active, "product", "products")),
    })));

    let ds = crate::directories::dir_settings(state);
    let live = count("SELECT COUNT(*) FROM directory_listings WHERE status = 'published'").await;
    let pending = count("SELECT COUNT(*) FROM directory_listings WHERE status = 'pending'").await;
    out.push(("directories", (ds.enabled, match (ds.enabled, pending) {
        (_, p) if p > 0 => format!("{p} to review · {live} live"),
        (true, _) => format!("Public · {live} live"),
        (false, _) => format!("Not public · {live} ready"),
    })));

    let bs = crate::builder::builder_settings(state);
    let built = count("SELECT COUNT(*) FROM builder_layouts WHERE active = 1").await;
    out.push(("builder", (bs.enabled && built > 0, match (bs.enabled, built) {
        (false, _) => "Off".to_string(),
        (true, 0) => "On · no layouts yet".to_string(),
        (true, n) => format!("{} live", plural(n, "page", "pages")),
    })));

    let cs = vuuut_api::caching::settings(state);
    let pages = vuuut_api::caching::snapshot(state).pages.len() as i64;
    out.push(("caching", (cs.page_cache, if cs.page_cache { format!("On · {}", plural(pages, "page cached", "pages cached")) } else { "Page cache off".to_string() })));

    let drafts = count("SELECT COUNT(*) FROM posts WHERE status = 'draft'").await;
    let gate = crate::workflows::settings_of(state).require_approval;
    out.push(("workflows", (drafts > 0, format!("{}{}", plural(drafts, "draft", "drafts"), if gate { " · approval on" } else { "" }))));

    let ads_on = vuuut_api::ads::settings(state).enabled;
    let live_ads = count("SELECT COUNT(*) FROM ads WHERE status = 'live'").await;
    out.push(("ads", (ads_on && live_ads > 0, match (ads_on, live_ads) {
        (false, _) => "Off".to_string(),
        (true, 0) => "On · nothing live".to_string(),
        (true, n) => format!("{} live", n),
    })));

    let analytics = state.settings.get_bool("web_analytics");
    out.push(("analytics", (analytics, if analytics { "Collecting".to_string() } else { "Off".to_string() })));

    let cookies = vuuut_api::cookies::settings(state);
    out.push(("cookies", (cookies.enabled, if cookies.enabled { format!("Banner on · {}", cookies.mode.short()) } else { "Banner off".into() })));

    let forms = count("SELECT COUNT(*) FROM forms").await;
    out.push(("forms", (forms > 0, if forms == 0 { "No forms yet".into() } else { plural(forms, "form", "forms") })));

    let langs = crate::site_locales(state).enabled().len() as i64;
    out.push(("languages", (langs > 1, plural(langs, "language", "languages"))));

    let members = count("SELECT COUNT(*) FROM members").await;
    out.push(("memberships", (members > 0, plural(members, "member", "members"))));

    let newsletters = count("SELECT COUNT(*) FROM newsletters WHERE status = 'active'").await;
    out.push(("newsletters", (newsletters > 0, plural(newsletters, "active newsletter", "active newsletters"))));

    let live = count("SELECT COUNT(*) FROM popups WHERE status = 'live'").await;
    out.push(("popups", (live > 0, if live == 0 { "None live".into() } else { format!("{live} live") })));

    let rules = count("SELECT COUNT(*) FROM firewall_rules").await;
    out.push(("security", (true, if rules == 0 { "Defaults on".into() } else { format!("Firewall · {}", plural(rules, "rule", "rules")) })));

    let media = count("SELECT COUNT(*) FROM media_meta").await;
    out.push(("media", (media > 0, if media == 0 { "Library".into() } else { plural(media, "described file", "described files") })));

    let indexed = !state.settings.get_bool("is_private");
    out.push(("seo", (indexed, if indexed { "Open to search engines".into() } else { "Private site".into() })));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_something_inside_it_to_find() {
        for t in crate::tools::TOOLS {
            assert!(!content_of(t.key).is_empty(), "{} has no entry in tool_finder::CONTENT", t.key);
        }
        for (key, _) in CONTENT {
            assert!(crate::tools::TOOLS.iter().any(|t| t.key == *key), "CONTENT names {key}, which is not a tool");
        }
    }
}
