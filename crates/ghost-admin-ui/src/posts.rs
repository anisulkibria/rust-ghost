//! The posts and pages list, as a grid of cards.
//!
//! Ghost's own list is a dense table: title, a column of author avatars,
//! tags, status, and — if you have analytics — opens and clicks. It is
//! efficient and almost unreadable at a glance. A post is a *thing a reader
//! sees*, so this shows it the way a reader would: the feature image, the
//! headline, the byline. The operational numbers sit underneath it rather
//! than replacing it.
//!
//! What each card answers, in the order a publisher asks:
//!
//! 1. **What is this?** — thumbnail, title, excerpt.
//! 2. **Whose is it, and when?** — byline and date.
//! 3. **Did it go anywhere?** — newsletter it was sent on, how many members
//!    received it, how many opened it, how many people read it on the site.
//!
//! Posts with no feature image get a generated two-tone gradient keyed to
//! their id rather than an empty grey rectangle. It is stable per post, so
//! an image-less post still has a face you recognise in the grid — which is
//! the whole reason to show thumbnails at all.
//!
//! All of the page's numbers are gathered in four queries regardless of how
//! many posts are listed; a card does not fetch its own anything.

use ghost_api::AppState;
use ghost_core::models::Post;
use ghost_db::repos;
use std::collections::HashMap;

use crate::{esc, svg, truncate_chars};

/// Everything one card renders from, resolved up front.
pub struct Card {
    pub post: Post,
    /// The site timezone, for rendering the date the way the rest of the
    /// admin does — a post published at 23:30 in New York is already
    /// tomorrow in UTC, and the list should not say so.
    pub timezone: String,
    /// Display names of the post's authors, in order.
    pub authors: Vec<String>,
    /// Newsletter this went out on, when it was emailed.
    pub newsletter: Option<String>,
    /// Members the email was sent to.
    pub sent: i64,
    /// How many of them opened it.
    pub opened: i64,
    /// Page views on the site.
    pub views: i64,
}

/// The status filters along the top. `None` is "everything".
pub const FILTERS: &[(&str, &str, Option<&str>)] = &[
    ("all", "All", None),
    ("published", "Published", Some("published")),
    ("draft", "Drafts", Some("draft")),
    ("scheduled", "Scheduled", Some("scheduled")),
    ("sent", "Sent", Some("sent")),
];

// ---------------------------------------------------------------------------
// gathering
// ---------------------------------------------------------------------------

/// Resolve every card's data for a page of posts.
///
/// Four queries total: authors, email stats, view counts, and the posts
/// themselves (already fetched by the caller). Per-card lookups would be
/// 4×N on a list that routinely shows a hundred posts.
pub async fn load_cards(state: &AppState, posts: Vec<Post>) -> Vec<Card> {
    if posts.is_empty() {
        return Vec::new();
    }
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let relations = repos::posts::load_relations(&state.db, &posts).await.unwrap_or_default();

    // Email stats, keyed by post. `emails.post_id` is unique, so one row per
    // post at most.
    let email_rows: Vec<(String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT e.post_id, e.email_count, e.opened_count, n.name \
         FROM emails e LEFT JOIN newsletters n ON n.id = e.newsletter_id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let emails: HashMap<String, (i64, i64, Option<String>)> = email_rows
        .into_iter()
        .map(|(post_id, sent, opened, newsletter)| (post_id, (sent, opened, newsletter)))
        .collect();

    // Views, counted the same way the analytics screen counts them: a view
    // belongs to a post when the path is its `/slug/`, or something beneath
    // it (pagination, anchors).
    let view_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT p.id, COUNT(*) FROM page_views v \
         JOIN posts p ON v.path = '/' || p.slug || '/' OR v.path LIKE '/' || p.slug || '/%' \
         GROUP BY p.id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let views: HashMap<String, i64> = view_rows.into_iter().collect();

    posts
        .into_iter()
        .map(|post| {
            let (sent, opened, newsletter) =
                emails.get(&post.id).cloned().unwrap_or((0, 0, None));
            Card {
                timezone: timezone.clone(),
                authors: relations
                    .authors_for(&post.id)
                    .iter()
                    .filter_map(|u| u.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .collect(),
                newsletter,
                sent,
                opened,
                views: views.get(&post.id).copied().unwrap_or(0),
                post,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

/// The whole list. `tags` are the site's tags, for the batch bar's tag
/// picker — passed in rather than fetched here so the list renders from one
/// round of queries.
pub fn grid(cards: &[Card], base: &str, tags: &[(String, String)]) -> String {
    // The reach and open-rate columns are only worth their width if anything
    // in this list was actually emailed. On a site that has never sent a
    // newsletter they would be two columns of nothing on every row.
    let emailed = cards.iter().any(|c| c.sent > 0);
    let plural = if base.contains("/pages/") { "pages" } else { "posts" };
    let items: String = cards
        .iter()
        .enumerate()
        .map(|(i, card)| row(card, base, i, emailed))
        .collect();
    format!(
        r#"<div class="post-list">
{head}
{items}
<div class="filter-empty" data-filter-empty><p>No {plural} match &#8220;<b data-query></b>&#8221;.</p>
<button class="btn tonal small" type="button" data-filter-clear>Clear search</button></div>
</div>
{bar}"#,
        head = head_row(base, plural, emailed),
        items = items,
        plural = plural,
        bar = batch_form(base, plural, tags),
    )
}

/// The strip above the rows: a select-all checkbox and the column names.
///
/// The rows are not a `<table>`, so the strip re-creates their column
/// geometry with the same fixed widths — its labels have to sit over the
/// columns they name, or they are decoration. Select-all here (rather than
/// in a toolbar) is the M3 data-table pattern: it sits in the column it
/// acts on, and its indeterminate state says "some of this page".
fn head_row(_base: &str, plural: &str, emailed: bool) -> String {
    // The email columns only exist when something was sent; the header
    // mirrors the rows or it would label empty space.
    // Each label sits in a cell the width of the stat under it, right-aligned
    // like the number — a bare span is as wide as its own text, which left
    // "Views" floating 30px off the column it names.
    let email_heads = if emailed {
        r#"<div class="lh-stat"><span class="lh-label">Sent</span></div><div class="lh-stat"><span class="lh-label">Opened</span></div>"#.to_string()
    } else {
        String::new()
    };
    format!(
        r#"<div class="list-head">
<label class="row-check" title="Select all {plural}"><input type="checkbox" data-select-all form="batch-list" aria-label="Select all {plural}"><span class="box">{check}</span></label>
<span class="lh-spacer" aria-hidden="true"></span>
<div class="lh-main"><span class="lh-label">Title</span></div>
<div class="lh-status"><span class="lh-label">Status</span></div>
<div class="lh-stats"><div class="lh-stat"><span class="lh-label">Views</span></div>{email_heads}</div>
</div>"#,
        plural = plural,
        check = svg("check"),
        email_heads = email_heads,
    )
}

/// The batch form: everything the selection can do, floating.
///
/// It is a sibling of the rows, not their parent, so the per-row delete
/// forms stay valid HTML — the checkboxes join this form by `form=`
/// association instead. Visibility is CSS (`:has` a ticked box), so no
/// script is needed to bring it up; the script only maintains the count.
///
/// Every bulk action the list offers lives here. Publish, unpublish, access
/// and tags used to be a script-built `<select>` and an "Apply to selected"
/// button sitting permanently above the list, next to a second column of raw
/// checkboxes; they are plain submit buttons on this bar now, so there is one
/// selection, one place its actions live, and nothing on screen until a row
/// is actually ticked.
fn batch_form(base: &str, plural: &str, tags: &[(String, String)]) -> String {
    let word = if plural == "pages" { "page" } else { "post" };
    // The two pickers follow the members bar's: chips in a popover that opens
    // out of the bar, each submitting the same form it hangs inside.
    let access_chips: String = [
        ("public", "Public", "Anyone can read it"),
        ("members", "Members only", "Signed-in members"),
        ("paid", "Paid members only", "Members on a paid plan"),
    ]
    .iter()
    .map(|(value, label, hint)| {
        format!(
            r#"<label class="fchip" title="{hint}"><input type="radio" name="visibility" value="{value}" required><span class="ck">{check}</span>{label}</label>"#,
            check = svg("check"),
        )
    })
    .collect();
    let tag_chips: String = if tags.is_empty() {
        r#"<p class="muted" style="margin:4px 0 0;font-size:12.5px">No tags exist yet.</p>"#.to_string()
    } else {
        tags.iter()
            .map(|(id, name)| {
                format!(
                    r#"<label class="fchip"><input type="checkbox" name="tags" value="{id}"><span class="ck">{check}</span>{name}</label>"#,
                    id = esc(id),
                    name = esc(name),
                    check = svg("check"),
                )
            })
            .collect()
    };
    format!(
        r#"<form id="batch-list" class="batch" method="post" action="{base}bulk/" data-word="{word}">
<div class="batch-bar">
<span class="bb-count"><b data-count>0 {word}s</b> selected</span>
<button class="bb-btn" type="submit" name="action" value="publish">{globe} Publish</button>
<button class="bb-btn" type="submit" name="action" value="draft">{hide} Unpublish</button>
<button class="bb-btn" type="submit" name="action" value="feature">{star} Feature</button>
<button class="bb-btn" type="submit" name="action" value="unfeature">{star_off} Unfeature</button>
<details class="batch-pop" data-menu>
<summary class="bb-btn">{lock} Access</summary>
<div class="pop">
<p class="pop-title">Who can read the selected {plural}</p>
<div class="chips" style="margin-bottom:12px">{access_chips}</div>
<button class="btn" type="submit" name="action" value="visibility">{check} Set access</button>
</div>
</details>
<details class="batch-pop" data-menu>
<summary class="bb-btn">{tags_icon} Tags</summary>
<div class="pop">
<p class="pop-title">Tags for the selected {plural}</p>
<div class="chips" style="margin-bottom:12px">{tag_chips}</div>
<button class="btn tonal" type="submit" name="action" value="tags">{check} Replace tags</button>
<p>Whatever is ticked becomes the complete set of tags; ticking none clears them.</p>
</div>
</details>
<button class="bb-btn danger" type="submit" name="action" value="delete"
 data-confirm="Delete selected {plural}|The selected {plural} will be permanently deleted."
 data-confirm-one="Delete 1 {word}|This {word} will be permanently deleted."
 data-confirm-many="Delete {{n}} {plural}|The {{n}} selected {plural} will be permanently deleted.">{del} Delete</button>
</div>
</form>"#,
        base = base,
        plural = plural,
        word = word,
        star = svg("star"),
        star_off = svg("star"),
        globe = svg("globe"),
        hide = svg("hide"),
        lock = svg("lock"),
        tags_icon = svg("tags"),
        check = svg("check"),
        del = svg("delete"),
        access_chips = access_chips,
        tag_chips = tag_chips,
    )
}

/// One post as a row.
///
/// The column order follows Ghost's: thumbnail, title and byline, what
/// happened to it, then the numbers. Keeping the columns a fixed width means
/// they line up down the page — which is the entire reason to use rows
/// rather than cards.
fn row(c: &Card, base: &str, index: usize, show_email: bool) -> String {
    let p = &c.post;
    let title = p.title.as_deref().unwrap_or("Untitled");
    let is_page = base.contains("/pages/");

    // Stagger the reveal, but cap it: on a hundred-post list the last row
    // would otherwise arrive seven seconds after the first.
    let delay = index.min(14);

    format!(
        r#"<article class="post-row rise" style="--i:{delay}" data-filterable>
<label class="row-check"><input type="checkbox" form="batch-list" name="ids" value="{id}" aria-label="Select {title_attr}"><span class="box">{check}</span></label>
<a class="post-row-link" href="{base}{id}/">
{thumb}
<div class="post-row-main">
<h2 class="post-row-title">{title}</h2>
<p class="post-row-meta">{meta}</p>
</div>
<div class="post-row-status">{status}</div>
<div class="post-row-stats">{stats}</div>
</a>
<div class="row-end">{analytics}
<form class="post-row-action" method="post" action="{base}{id}/delete/" data-confirm="Delete {what}|&#8220;{title_attr}&#8221; will be permanently deleted.">
<button class="icon-btn danger" title="Delete" aria-label="Delete {title_attr}">{del}</button></form>
</div>
</article>"#,
        delay = delay,
        base = base,
        id = esc(&p.id),
        check = svg("check"),
        // Only a published post has anything to report on, and only a post —
        // a page is not emailed and is not attributed to. The slot is held
        // open either way, so the delete button lands in the same place on
        // every row rather than sliding left on rows with nothing to analyse.
        analytics = if is_page || !matches!(p.status.as_str(), "published" | "sent") {
            r#"<span class="post-row-action" aria-hidden="true"></span>"#.to_string()
        } else {
            format!(
                r#"<a class="post-row-action icon-btn" href="{base}{id}/analytics/" title="Analytics" aria-label="Analytics for {title_attr}">{chart}</a>"#,
                base = base,
                id = esc(&p.id),
                title_attr = esc(title),
                chart = svg("chart"),
            )
        },
        thumb = thumbnail(p),
        title = esc(title),
        title_attr = esc(title),
        meta = meta_line(c),
        status = status_cell(c, is_page),
        stats = stats(c, is_page, show_email),
        what = if is_page { "page" } else { "post" },
        del = svg("delete"),
    )
}

/// The feature image at row scale, or a generated stand-in.
///
/// Ghost uses an 80–100px 16:10 tile here; this matches it. A post with no
/// image gets a gradient keyed to its id rather than a grey rectangle, so
/// the column still carries some recognition value when scanning.
fn thumbnail(p: &Post) -> String {
    let inner = match p.feature_image.as_deref().filter(|i| !i.trim().is_empty()) {
        Some(src) => format!(
            r#"<img class="post-row-img" src="{src}" alt="" loading="lazy" decoding="async">"#,
            src = esc(&ghost_core::url::base_asset(src)),
        ),
        None => {
            let initial = p
                .title
                .as_deref()
                .and_then(|t| t.trim().chars().next())
                .unwrap_or('·')
                .to_uppercase()
                .to_string();
            let (from, to) = gradient_for(&p.id);
            format!(
                r#"<span class="post-row-blank" style="--from:{from};--to:{to}" aria-hidden="true"><span>{initial}</span></span>"#,
                initial = esc(&initial),
            )
        }
    };
    let featured = if p.featured {
        format!(r#"<span class="post-row-star" title="Featured">{}</span>"#, svg("star"))
    } else {
        String::new()
    };
    format!(r#"<span class="post-row-thumb">{inner}{featured}</span>"#)
}

/// Two hues from the post's id, 140° apart on the colour wheel.
///
/// Deterministic so a post keeps the same face between page loads, and
/// spaced apart so the gradient reads as a gradient rather than a smudge.
/// Saturation and lightness are fixed low enough that the initial sits on it
/// comfortably in either colour scheme.
fn gradient_for(id: &str) -> (String, String) {
    let hash = id.bytes().fold(2166136261u32, |acc, b| {
        (acc ^ b as u32).wrapping_mul(16777619)
    });
    let hue = hash % 360;
    // Lightness is held low enough that the white initial reads on every
    // hue — at 58% a pale pink or yellow swallowed it.
    (
        format!("hsl({hue} 48% 47%)"),
        format!("hsl({} 52% 34%)", (hue + 140) % 360),
    )
}

/// Ghost's secondary line: `By {authors} in {tag} · {date}`.
fn meta_line(c: &Card) -> String {
    let mut parts: Vec<String> = Vec::new();

    if !c.authors.is_empty() {
        let who = match c.authors.len() {
            1 => esc(&c.authors[0]),
            n => format!("{} +{}", esc(&c.authors[0]), n - 1),
        };
        parts.push(format!("By {who}"));
    }
    // Drafts and scheduled posts are dated by when they were last touched;
    // everything else by when it went out. Ghost makes the same split.
    let raw = match c.post.status.as_str() {
        "draft" | "scheduled" => c.post.updated_at.as_deref(),
        _ => c.post.published_at.as_deref(),
    };
    if let Some(date) = raw.map(|d| ghost_core::dates::short_date_in_zone(d, &c.timezone)) {
        if !date.is_empty() {
            parts.push(esc(&date));
        }
    }
    if c.post.reading_time > 0 {
        parts.push(format!("{} min read", c.post.reading_time));
    }
    parts.join(" <span class=\"post-row-dot\">·</span> ")
}

/// What happened to this post, and the one detail worth knowing about it.
///
/// Mirrors Ghost's status copy: a post that was both published and emailed
/// says so, and says how many members it reached.
fn status_cell(c: &Card, is_page: bool) -> String {
    let emailed = !is_page && c.sent > 0;
    let (class, label) = match c.post.status.as_str() {
        "scheduled" => ("scheduled", "Scheduled"),
        "sent" => ("sent", "Sent"),
        "published" if emailed => ("published", "Published and sent"),
        "published" => ("published", "Published"),
        _ => ("draft", "Draft"),
    };

    let detail = if c.post.status == "scheduled" {
        c.post
            .published_at
            .as_deref()
            .map(|d| format!("for {}", ghost_core::dates::short_date_in_zone(d, &c.timezone)))
            .unwrap_or_default()
    } else if emailed {
        match c.newsletter.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(n) => format!(
                "to {} members on {}",
                compact(c.sent),
                esc(&truncate_chars(&short_newsletter(n), 20))
            ),
            None => format!("to {} members", compact(c.sent)),
        }
    } else {
        String::new()
    };

    format!(
        r#"<span class="post-row-chip {class}">{label}</span>{detail}"#,
        detail = if detail.is_empty() {
            String::new()
        } else {
            format!(r#"<span class="post-row-detail">{detail}</span>"#)
        },
    )
}

/// The numbers, as fixed-width cells so they line up down the page.
///
/// Only facts that exist get a value; the cell itself is always rendered so
/// a row with no views does not shunt the next row's numbers sideways.
fn stats(c: &Card, is_page: bool, show_email: bool) -> String {
    let views = if c.views > 0 {
        stat_cell(&compact(c.views), "views", &format!("{} page views", c.views))
    } else {
        empty_cell()
    };

    // A page is never emailed, and neither is anything on a site that has
    // not sent a newsletter — drop the columns rather than render them
    // permanently blank.
    if is_page || !show_email {
        return views;
    }

    let sent = if c.sent > 0 {
        stat_cell(&compact(c.sent), "sent", &format!("Emailed to {} members", c.sent))
    } else {
        empty_cell()
    };
    let opened = if c.sent > 0 && c.opened > 0 {
        let rate = (c.opened as f64 / c.sent as f64 * 100.0).round() as i64;
        stat_cell(
            &format!("{rate}%"),
            "opened",
            &format!("{} of {} opened", c.opened, c.sent),
        )
    } else {
        empty_cell()
    };
    format!("{views}{sent}{opened}")
}

fn stat_cell(value: &str, label: &str, title: &str) -> String {
    format!(
        r#"<span class="post-stat" title="{title}"><b>{value}</b><span>{label}</span></span>"#,
        title = esc(title),
        value = esc(value),
        label = esc(label),
    )
}

/// A cell that holds its column width without drawing anything.
///
/// The alignment is what the fixed width is for; a dash in every blank cell
/// just adds ten marks to the page that mean "no".
fn empty_cell() -> String {
    r#"<span class="post-stat" aria-hidden="true"></span>"#.to_string()
}

/// Drop a trailing "Newsletter" from the name for display.
///
/// Most sites name theirs "<Something> Newsletter", and in a pill that
/// already carries a send icon the word is the least informative part of a
/// string with no room to spare. The full name stays in the tooltip.
fn short_newsletter(name: &str) -> String {
    let trimmed = name.trim();
    match trimmed.strip_suffix(" Newsletter").or_else(|| trimmed.strip_suffix(" newsletter")) {
        // Only when something recognisable is left — a newsletter actually
        // called "Newsletter" keeps its name.
        Some(rest) if rest.trim().len() >= 3 => rest.trim().to_string(),
        _ => trimmed.to_string(),
    }
}

/// `1.2k` rather than `1203`: the cards are scanned, not audited.
fn compact(n: i64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1_000.0),
        n => n.to_string(),
    }
}

/// How the grid can be ordered.
///
/// "Most viewed" is the one Ghost's own list cannot do without opening
/// analytics, and it is the question a publisher asks most often after
/// "what did I write last".
pub const SORTS: &[(&str, &str)] = &[
    ("newest", "Newest first"),
    ("oldest", "Oldest first"),
    ("views", "Most viewed"),
    ("title", "Title A–Z"),
];

/// Order the cards in place.
///
/// Dates are ISO strings, so a lexical comparison is a chronological one;
/// a post with no date sorts to the bottom of a newest-first list rather
/// than to the top, which is where `None` would otherwise land it.
pub fn sort_cards(cards: &mut [Card], sort: &str) {
    match sort {
        "oldest" => cards.sort_by(|a, b| date_of(a).cmp(date_of(b))),
        "views" => cards.sort_by(|a, b| b.views.cmp(&a.views).then_with(|| date_of(b).cmp(date_of(a)))),
        "title" => cards.sort_by_key(|c| title_of(c).to_lowercase()),
        _ => cards.sort_by(|a, b| date_of(b).cmp(date_of(a))),
    }
}

/// The date a card is ordered by: when it went out, or failing that when it
/// was last touched — a draft has never been published but still has a place
/// in a chronological list.
fn date_of(c: &Card) -> &str {
    c.post
        .published_at
        .as_deref()
        .or(c.post.updated_at.as_deref())
        .or(c.post.created_at.as_deref())
        .unwrap_or("")
}

fn title_of(c: &Card) -> &str {
    c.post.title.as_deref().unwrap_or("Untitled")
}

/// The sort picker, as a menu of links.
///
/// A `<select>` was the one native control left on this toolbar, and it
/// looked it. Each order is a plain link (carrying the current filter, as
/// the select's handler tried to), so it navigates without script; the
/// open/close is the same `<details>` menu the rest of the admin uses.
pub fn sort_picker(base: &str, filter: &str, active: &str, nql: &str) -> String {
    let active_label = SORTS
        .iter()
        .find(|(key, _)| *key == active)
        .map(|(_, label)| *label)
        .unwrap_or("Newest first");
    // An active NQL filter has to survive a change of order, the same way the
    // tab does — reordering a filtered list silently dropping the filter is
    // how a control loses its user's trust.
    let carry = if nql.trim().is_empty() {
        String::new()
    } else {
        format!("&amp;nql={}", crate::url_value(nql))
    };
    let items: String = SORTS
        .iter()
        .map(|(key, label)| {
            format!(
                r#"<a class="menu-item{on}" href="{base}?filter={filter}&amp;sort={key}{carry}"><span class="ck">{check}</span>{label}</a>"#,
                on = if *key == active { " on" } else { "" },
                base = base,
                filter = esc(filter),
                key = key,
                carry = carry,
                check = if *key == active { svg("check") } else { String::new() },
                label = esc(label),
            )
        })
        .collect();
    format!(
        r#"<details class="sort-menu" data-menu>
<summary><span class="sr-only">Sort {plural}: </span>{active_label}{caret}</summary>
<div class="pop">{items}</div>
</details>"#,
        plural = if base.contains("/pages/") { "pages" } else { "posts" },
        active_label = esc(active_label),
        caret = crate::SELECT_CARET,
        items = items,
    )
}

/// The status filter bar. The sort and any NQL filter ride along, so moving
/// between tabs narrows the current view rather than replacing it.
pub fn filter_bar(base: &str, active: &str, sort: &str, nql: &str, counts: &HashMap<String, i64>) -> String {
    let mut suffix = format!("&amp;sort={}", esc(sort));
    if !nql.trim().is_empty() {
        suffix.push_str(&format!("&amp;nql={}", crate::url_value(nql)));
    }
    crate::seg_filter_bar(FILTERS, base, active, &suffix, counts)
}

/// The NQL filter menu for this list, with the tab and sort carried as
/// hidden inputs so applying a filter keeps the rest of the view.
pub fn filter_menu(base: &str, filter: &str, sort: &str, nql: &str) -> String {
    let carry = format!(
        r#"<input type="hidden" name="filter" value="{filter}"><input type="hidden" name="sort" value="{sort}">"#,
        filter = esc(filter),
        sort = esc(sort),
    );
    let resource = if base.contains("/pages/") { "pages" } else { "posts" };
    crate::filter_menu(
        base,
        resource,
        nql,
        &carry,
        "status:draft+featured:true · tag:news · authors:jane",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients_are_stable_and_two_toned() {
        let a = gradient_for("6ad3d5e8968d4e6930000028");
        // the same post always gets the same face
        assert_eq!(a, gradient_for("6ad3d5e8968d4e6930000028"));
        // and the two stops differ, so it reads as a gradient
        assert_ne!(a.0, a.1);
        assert!(a.0.starts_with("hsl("));
    }

    #[test]
    fn different_posts_usually_get_different_hues() {
        let ids = ["aaa", "bbb", "ccc", "ddd", "eee", "fff", "ggg", "hhh"];
        let hues: std::collections::HashSet<String> =
            ids.iter().map(|id| gradient_for(id).0).collect();
        // FNV over short ids spreads well; anything less than most-distinct
        // would make the grid look repetitive
        assert!(hues.len() >= ids.len() - 1, "only {} distinct hues", hues.len());
    }

    #[test]
    fn newsletter_names_lose_a_redundant_suffix() {
        assert_eq!(short_newsletter("Rust Ghost Newsletter"), "Rust Ghost");
        assert_eq!(short_newsletter("Weekly newsletter"), "Weekly");
        // nothing recognisable would be left, so the name stands
        assert_eq!(short_newsletter("Newsletter"), "Newsletter");
        assert_eq!(short_newsletter("The Dispatch"), "The Dispatch");
    }

    #[test]
    fn counts_are_abbreviated_for_scanning() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1.0k");
        assert_eq!(compact(1_234), "1.2k");
        assert_eq!(compact(1_500_000), "1.5M");
    }

    #[test]
    fn the_filter_bar_hides_empty_buckets_but_keeps_the_active_one() {
        let counts: HashMap<String, i64> =
            [("published".to_string(), 3i64)].into_iter().collect();
        let bar = filter_bar("/ghost/posts/", "all", "newest", "", &counts);
        assert!(bar.contains(">Published<"), "{bar}");
        // nothing is scheduled, so that tab is not offered
        assert!(!bar.contains(">Scheduled<"), "{bar}");

        // unless it is the one in force, which must stay clickable
        let bar = filter_bar("/ghost/posts/", "scheduled", "newest", "", &counts);
        assert!(bar.contains(">Scheduled<"), "{bar}");
        assert!(bar.contains(r#"class="seg on""#), "{bar}");
    }

    fn card_of(title: &str, published: Option<&str>, views: i64) -> Card {
        Card {
            post: Post {
                id: title.into(),
                title: Some(title.into()),
                published_at: published.map(str::to_string),
                status: "published".into(),
                ..Default::default()
            },
            timezone: "Etc/UTC".into(),
            authors: vec![],
            newsletter: None,
            sent: 0,
            opened: 0,
            views,
        }
    }

    fn order(cards: &[Card]) -> Vec<&str> {
        cards.iter().map(|c| c.post.title.as_deref().unwrap_or("")).collect()
    }

    #[test]
    fn sorting_orders_by_date_views_and_title() {
        let base = || {
            vec![
                card_of("Beta", Some("2026-02-01 00:00:00"), 5),
                card_of("Alpha", Some("2026-03-01 00:00:00"), 50),
                card_of("Gamma", Some("2026-01-01 00:00:00"), 20),
            ]
        };

        let mut c = base();
        sort_cards(&mut c, "newest");
        assert_eq!(order(&c), ["Alpha", "Beta", "Gamma"]);

        let mut c = base();
        sort_cards(&mut c, "oldest");
        assert_eq!(order(&c), ["Gamma", "Beta", "Alpha"]);

        let mut c = base();
        sort_cards(&mut c, "views");
        assert_eq!(order(&c), ["Alpha", "Gamma", "Beta"]);

        let mut c = base();
        sort_cards(&mut c, "title");
        assert_eq!(order(&c), ["Alpha", "Beta", "Gamma"]);

        // anything unrecognised falls back to newest rather than leaving the
        // grid in whatever order the query happened to return
        let mut c = base();
        sort_cards(&mut c, "sideways");
        assert_eq!(order(&c), ["Alpha", "Beta", "Gamma"]);
    }

    /// A draft has never been published, but it still belongs somewhere in a
    /// chronological list — at the time it was last worked on.
    #[test]
    fn an_undated_post_falls_back_to_when_it_was_touched() {
        let mut drafted = card_of("Draft", None, 0);
        drafted.post.updated_at = Some("2026-05-01 00:00:00".into());
        let published = card_of("Published", Some("2026-04-01 00:00:00"), 0);

        let mut cards = vec![published, drafted];
        sort_cards(&mut cards, "newest");
        assert_eq!(order(&cards), ["Draft", "Published"]);
    }

    #[test]
    fn the_sort_picker_keeps_the_current_filter() {
        let picker = sort_picker("/ghost/posts/", "draft", "views", "");
        assert!(picker.contains("filter=draft"), "{picker}");
        // the active order is both what the closed menu shows and the one
        // item carrying the check
        assert!(picker.contains("Most viewed"), "{picker}");
        assert_eq!(picker.matches(svg("check").as_str()).count(), 1, "{picker}");
        assert!(picker.contains(r#"href="/ghost/posts/?filter=draft&amp;sort=oldest""#), "{picker}");
    }

    /// Changing the order must not silently move you back to "All", and
    /// changing the filter must not reset the order.
    #[test]
    fn filter_and_sort_survive_each_other() {
        let counts: HashMap<String, i64> = [("draft".to_string(), 2i64)].into_iter().collect();
        let bar = filter_bar("/ghost/posts/", "draft", "views", "", &counts);
        assert!(bar.contains("sort=views"), "{bar}");
    }

    #[test]
    fn every_filter_key_is_a_real_status_or_the_catch_all() {
        for (key, label, status) in FILTERS {
            assert!(!label.is_empty(), "{key} has no label");
            if let Some(s) = status {
                assert!(
                    matches!(*s, "published" | "draft" | "scheduled" | "sent"),
                    "{key} filters on an unknown status {s}"
                );
            }
        }
    }

    fn one_card(id: &str) -> Card {
        Card {
            post: Post { id: id.into(), title: Some("A title".into()), status: "draft".into(), ..Default::default() },
            timezone: "Etc/UTC".into(),
            authors: vec![],
            newsletter: None,
            sent: 0,
            opened: 0,
            views: 0,
        }
    }

    /// The rows and the floating bar must join the same form — the whole
    /// batch selection hangs off that one `form=` association, so a drift
    /// here would tick boxes that submit nothing.
    #[test]
    fn rows_and_bar_share_the_batch_form() {
        let html = grid(&[one_card("p1"), one_card("p2")], "/ghost/posts/", &[]);
        assert_eq!(html.matches(r#"form="batch-list""#).count(), 3, "{html}");
        assert!(html.contains(r#"<form id="batch-list" class="batch" method="post" action="/ghost/posts/bulk/""#), "{html}");
        assert!(html.contains(r#"action="/ghost/pages/bulk/""#) == false);
        // every row can be named for screen readers
        assert!(html.matches(r#"aria-label="Select A title""#).count() == 2, "{html}");
    }

    #[test]
    fn the_pages_list_points_its_own_bulk_endpoint() {
        let html = grid(&[one_card("pg1")], "/ghost/pages/", &[]);
        assert!(html.contains(r#"action="/ghost/pages/bulk/""#), "{html}");
        assert!(html.contains("data-word=\"page\""), "{html}");
    }

    /// The header strip only names the email columns when the rows carry
    /// them, or it would label empty space.
    #[test]
    fn the_header_mirrors_the_email_columns() {
        let quiet = grid(&[one_card("p1")], "/ghost/posts/", &[]);
        assert!(!quiet.contains(">Sent<"), "{quiet}");
        let mut emailed = one_card("p2");
        emailed.sent = 12;
        let loud = grid(&[emailed], "/ghost/posts/", &[]);
        assert!(loud.contains(">Sent<") && loud.contains(">Opened<"), "{loud}");
    }

    /// One checkbox per row, and one select-all. A second selection UI used
    /// to be injected over this one by a script, which is what a duplicate
    /// column of raw checkboxes on every row looked like from the outside.
    #[test]
    fn a_row_offers_exactly_one_checkbox() {
        let html = grid(&[one_card("p1"), one_card("p2")], "/ghost/posts/", &[]);
        assert_eq!(html.matches(r#"type="checkbox""#).count(), 3, "{html}");
        assert_eq!(html.matches("data-select-all").count(), 1, "{html}");
        // and every one of them is the drawn control, never a bare input
        assert_eq!(html.matches(r#"class="row-check""#).count(), 3, "{html}");
    }

    /// Every bulk action the list offers is a button on the one bar, posting
    /// to the one endpoint — not a `<select>` sitting above the list.
    #[test]
    fn the_batch_bar_carries_every_bulk_action() {
        let tags = [("t1".to_string(), "News".to_string())];
        let html = grid(&[one_card("p1")], "/ghost/posts/", &tags);
        for action in ["publish", "draft", "feature", "unfeature", "visibility", "tags", "delete"] {
            assert!(html.contains(&format!(r#"name="action" value="{action}""#)), "no {action}: {html}");
        }
        // the pickers' own inputs post alongside the action
        assert!(html.contains(r#"name="visibility" value="paid""#), "{html}");
        assert!(html.contains(r#"name="tags" value="t1""#), "{html}");
        assert!(html.contains(">News<"), "{html}");
    }

    /// A list with no tags still offers the picker, saying so, rather than
    /// an empty popover that looks broken.
    #[test]
    fn the_tag_picker_says_when_there_is_nothing_to_pick() {
        let html = grid(&[one_card("p1")], "/ghost/posts/", &[]);
        assert!(html.contains("No tags exist yet."), "{html}");
    }

    /// A filter has to survive every other control on the toolbar, or
    /// changing the sort silently widens the list back out.
    #[test]
    fn the_toolbar_carries_an_active_filter_between_its_controls() {
        let nql = "status:draft+featured:true";
        let picker = sort_picker("/ghost/posts/", "all", "newest", nql);
        assert!(picker.contains("nql=status%3Adraft%2Bfeatured%3Atrue"), "{picker}");

        let counts: HashMap<String, i64> = [("draft".to_string(), 3i64)].into_iter().collect();
        let bar = filter_bar("/ghost/posts/", "all", "newest", nql, &counts);
        assert!(bar.contains("nql=status%3Adraft%2Bfeatured%3Atrue"), "{bar}");

        // and with no filter nothing extra is appended
        assert!(!sort_picker("/ghost/posts/", "all", "newest", "").contains("nql="));
    }

    /// The filter menu reflects its own state: the field is filled, the pill
    /// is marked active and a way out is offered. An applied filter that
    /// looks identical to no filter is how people lose rows.
    #[test]
    fn the_filter_menu_shows_whether_a_filter_is_on() {
        let off = filter_menu("/ghost/posts/", "all", "newest", "");
        assert!(!off.contains("fm-dot"), "{off}");
        assert!(!off.contains(">Clear<"), "{off}");

        let on = filter_menu("/ghost/posts/", "all", "newest", "tag:news");
        assert!(on.contains(r#"class="sort-menu filter-menu on""#), "{on}");
        assert!(on.contains("fm-dot") && on.contains(">Clear<"), "{on}");
        assert!(on.contains(r#"value="tag:news""#), "{on}");
        // the tab and sort ride along, so filtering does not reset the view
        assert!(on.contains(r#"name="filter" value="all""#), "{on}");
        assert!(on.contains(r#"name="sort" value="newest""#), "{on}");
        // and the saved-views slot names what the script should fetch
        assert!(on.contains(r#"data-views="posts""#), "{on}");
        assert!(filter_menu("/ghost/pages/", "all", "newest", "").contains(r#"data-views="pages""#));
    }
}
