//! The CDN-hosted frontend apps `ghost_head` injects — Portal (members
//! signup/signin/account) and Sodo Search (the search overlay).
//!
//! Ghost keeps these in `core/shared/config/defaults.json` under `portal`,
//! `sodoSearch` and friends, as a URL template plus a pinned minor version
//! that `{version}` is substituted into. The jsDelivr `@~1.9` range then
//! floats patch releases. Mirrored here with the versions Ghost 6.73 pins, so
//! themes get byte-identical script tags to a real Ghost install.
//!
//! Keeping these current is not cosmetic: Portal reads the Content API and
//! `/members/api/*` shapes this port serves, and a pin several minors behind
//! means the site loads an app written against a contract the server has
//! already moved past.
//!
//! Both apps talk back to this server: Portal to `/members/api/*`, Sodo
//! Search to `/ghost/api/content/search-index/*`.

/// `portal.min.js` — renders every `data-portal` / `#/portal/*` link.
pub const PORTAL_VERSION: &str = "2.72";
/// `sodo-search.min.js` — renders every `data-ghost-search` / `#/search` link.
pub const SODO_SEARCH_VERSION: &str = "1.9";
/// `comments-ui.min.js` — the comment thread, form and moderation controls
/// that `{{comments}}` mounts.
pub const COMMENTS_VERSION: &str = "1.6";
/// `signup-form.min.js` — the embeddable signup form, for pasting onto a site
/// that is not this one. Unlike the three above it is never injected by
/// `ghost_head`: it only ever appears in the snippet Settings → Growth hands
/// out, which is why it is loaded from the CDN by whoever pastes it.
pub const SIGNUP_FORM_VERSION: &str = "0.4";
/// `announcement-bar.min.js` — the bar Settings → Site → Announcement bar
/// fills in. It has to be client-side rather than server-rendered for two
/// reasons: the audience is per-member, so the markup differs by visitor and
/// could not be cached; and a dismissal has to be remembered, which the
/// script does in `localStorage` keyed on the announcement's own content.
pub const ANNOUNCEMENT_BAR_VERSION: &str = "1.1";

pub fn portal_script_url() -> String {
    format!("https://cdn.jsdelivr.net/ghost/portal@~{PORTAL_VERSION}/umd/portal.min.js")
}

pub fn sodo_search_script_url() -> String {
    format!("https://cdn.jsdelivr.net/ghost/sodo-search@~{SODO_SEARCH_VERSION}/umd/sodo-search.min.js")
}

pub fn sodo_search_styles_url() -> String {
    format!("https://cdn.jsdelivr.net/ghost/sodo-search@~{SODO_SEARCH_VERSION}/umd/main.css")
}

pub fn comments_script_url() -> String {
    format!("https://cdn.jsdelivr.net/ghost/comments-ui@~{COMMENTS_VERSION}/umd/comments-ui.min.js")
}

pub fn signup_form_script_url() -> String {
    format!("https://cdn.jsdelivr.net/ghost/signup-form@~{SIGNUP_FORM_VERSION}/umd/signup-form.min.js")
}

pub fn announcement_bar_script_url() -> String {
    format!(
        "https://cdn.jsdelivr.net/ghost/announcement-bar@~{ANNOUNCEMENT_BAR_VERSION}/umd/announcement-bar.min.js"
    )
}

/// Ghost ships these in `core/frontend/helpers/tpl/styles.js` and injects them
/// as `<style id="gh-members-styles">`, because the `{{#unless}}`-gated upgrade
/// CTA that `post_content` emits for gated posts is engine-rendered markup, not
/// theme markup — so no theme stylesheet covers it.
pub const MEMBERS_CTA_STYLES: &str = r#".gh-post-upgrade-cta-content,
.gh-post-upgrade-cta {
    display: flex;
    flex-direction: column;
    align-items: center;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
    text-align: center;
    width: 100%;
    color: #ffffff;
    font-size: 16px;
}

.gh-post-upgrade-cta-content {
    border-radius: 8px;
    padding: 40px 4vw;
}

.gh-post-upgrade-cta h2 {
    color: #ffffff;
    font-size: 28px;
    letter-spacing: -0.2px;
    margin: 0;
    padding: 0;
}

.gh-post-upgrade-cta p {
    margin: 20px 0 0;
    padding: 0;
}

.gh-post-upgrade-cta small {
    font-size: 16px;
    letter-spacing: -0.2px;
}

.gh-post-upgrade-cta a {
    color: #ffffff;
    cursor: pointer;
    font-weight: 500;
    box-shadow: none;
    text-decoration: underline;
}

.gh-post-upgrade-cta a:hover {
    color: #ffffff;
    opacity: 0.8;
    box-shadow: none;
    text-decoration: underline;
}

.gh-post-upgrade-cta a.gh-btn {
    display: block;
    background: #ffffff;
    text-decoration: none;
    margin: 28px 0 0;
    padding: 8px 18px;
    border-radius: 4px;
    font-size: 16px;
    font-weight: 600;
}

.gh-post-upgrade-cta a.gh-btn:hover {
    opacity: 0.92;
}
"#;
