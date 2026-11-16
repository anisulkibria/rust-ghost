//! Newsletters → Editions: every post that was emailed, and what became of it.
//!
//! One row per `emails` record, newest first, with the counts the send and the
//! provider webhooks wrote: attempted, delivered, opened, failed. A failed
//! edition carries the provider's own error text and a **Send again** button.
//!
//! Before this screen the record of a send was readable in three places and
//! none of them was a list: the posts list showed a sent count per post, the
//! Analytics → Newsletters tab showed the last fifteen with their open rates,
//! and a failure was visible only as a count on the dashboard's queue card.
//! Retrying one meant a `PUT` to the Admin API by hand.
//!
//! ## The retry
//!
//! The one thing this module writes. It is deliberately narrow:
//!
//! * Only a **failed** edition offers the button —
//!   [`ghost_core::newsletters::is_retryable`]. Retrying a send that worked
//!   would be a way to mail everybody a second time; retrying a pending one
//!   would race the send loop still working on it.
//! * The work is [`crate::retry_email`], the same function the Admin API
//!   endpoint calls, which resolves the audience exactly as the first attempt
//!   did and then narrows it to the recipients whose row says `failed`. A
//!   retry cannot reach anybody twice.
//! * The handler re-reads the edition and re-checks its status before acting,
//!   rather than trusting that the page which rendered the button was looking
//!   at the same row. Between the render and the click the send loop may have
//!   finished it.

use super::*;

/// How many editions one page shows.
const PER_PAGE: i64 = 25;

/// Which page of the list to show.
///
/// Rule 8: a query parameter that comes back as a link is reduced to a known
/// value first. A page number is a number rather than a member of a list, so
/// "known" here means a positive integer inside the range the data actually
/// has — anything else, including a negative offset that would otherwise
/// reach the end of the table, reads as page 1.
fn page_of(q: &HashMap<String, String>, total: i64) -> i64 {
    let last = ((total.max(1) + PER_PAGE - 1) / PER_PAGE).max(1);
    q.get("page")
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|n| *n >= 1 && *n <= last)
        .unwrap_or(1)
}

pub async fn editions_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let total = repo::edition_count(&state.db).await;
    let page = page_of(&q, total);
    let editions = match repo::editions(&state.db, PER_PAGE, (page - 1) * PER_PAGE).await {
        Ok(e) => e,
        Err(e) => return server_error(&e),
    };

    let failed = editions.iter().filter(|e| rules::is_retryable(&e.status)).count();
    let verdict = if total == 0 {
        "No post has been emailed yet. Publishing one to a newsletter puts it here.".to_string()
    } else if failed > 0 {
        format!(
            "{} on this page failed. Sending again reaches only the subscribers the first \
             attempt could not, so nobody is mailed twice.",
            countable(failed as i64, "edition"),
        )
    } else {
        format!("{} emailed, newest first.", countable(total, "post"))
    };

    let body = if editions.is_empty() {
        r#"<div class="card padded rise" style="--i:2;text-align:center;padding:48px 20px">
<p style="margin:0 0 4px;font-weight:600;font-size:17px">Nothing emailed yet</p>
<p class="muted" style="margin:0">A post published to a newsletter appears here with what became of it.</p></div>"#
            .to_string()
    } else {
        format!(
            r#"<div class="card rows rise" style="--i:2">{rows}</div>{pager}"#,
            rows = editions.iter().map(|e| edition_row(e, true)).collect::<String>(),
            pager = pager(page, total),
        )
    };

    page_for_role(
        actor.role,
        "Editions",
        "newsletters",
        Some(&actor.user),
        &format!(
            r#"{head}
{banner}
<div class="section">Editions</div>
<p class="muted" style="margin:0 0 14px 16px">{verdict}</p>
{body}"#,
            head = head("editions"),
            banner = flash_banner(&q),
            verdict = esc(&verdict),
            body = body,
        ),
        true,
    )
    .into_response()
}

/// Previous/next, drawn only when there is more than one page.
fn pager(page: i64, total: i64) -> String {
    let last = ((total.max(1) + PER_PAGE - 1) / PER_PAGE).max(1);
    if last <= 1 {
        return String::new();
    }
    let link = |target: i64, label: &str| {
        if target < 1 || target > last {
            return String::new();
        }
        format!(
            r#"<a class="btn tonal small" href="{EDITIONS_PATH}?page={target}">{label}</a>"#,
            target = target,
            label = esc(label),
        )
    };
    format!(
        r#"<div class="nl-pager">{prev}<span class="muted">Page {page} of {last}</span>{next}</div>"#,
        prev = link(page - 1, "Previous"),
        page = page,
        last = last,
        next = link(page + 1, "Next"),
    )
}

/// One edition, as a row.
///
/// `actions` is false on the overview's compact card, where a Send again
/// button beside five rows would be the loudest thing on the page, and true
/// on this screen, which is where somebody has come to act.
pub(crate) fn edition_row(e: &repo::Edition, actions: bool) -> String {
    let (status_label, chip) = rules::edition_status(&e.status);
    let open_rate = rules::rate(e.opened, e.delivered);

    // The error is text a mail provider returned. It is the only genuinely
    // remote input on this screen, and it goes through `esc` like everything
    // else — but it is also truncated, because a provider that returns a
    // kilobyte of JSON should not own the page.
    let detail = match (rules::is_retryable(&e.status), e.error.as_deref()) {
        (true, Some(reason)) if !reason.trim().is_empty() => {
            let mut short: String = reason.trim().chars().take(160).collect();
            if reason.trim().chars().count() > 160 {
                short.push('…');
            }
            short
        }
        _ => format!(
            "{newsletter} · {attempted} attempted · {delivered} delivered{failed}{opened}",
            newsletter = e.newsletter_name.as_deref().unwrap_or("no newsletter recorded"),
            attempted = e.attempted,
            delivered = e.delivered,
            failed = match e.failed {
                0 => String::new(),
                n => format!(" · {n} failed"),
            },
            opened = match open_rate {
                Some(rate) => format!(" · {rate:.0}% opened"),
                None => String::new(),
            },
        ),
    };

    // Rule 8: an id is checked before it is built into a link or posted back.
    let valid_id = ghost_core::ids::is_objectid(&e.id);
    let retry = if actions && rules::is_retryable(&e.status) && valid_id {
        format!(
            r#"<form method="post" action="{EDITIONS_PATH}retry/" style="display:inline">
<input type="hidden" name="id" value="{id}">
<button class="btn tonal small" type="submit" title="Send again to the subscribers this edition could not reach">{repeat} Send again</button></form>"#,
            id = esc(&e.id),
            repeat = svg("repeat"),
        )
    } else {
        String::new()
    };

    let title = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip {chip}">{status_label}</span>
<span class="muted nl-when">{when}</span>{retry}</div>"#,
        tint = match e.status.as_str() {
            "failed" => "red",
            "pending" => "amber",
            _ => "purple",
        },
        icon = svg(match e.status.as_str() {
            "failed" => "warning",
            "pending" => "history",
            _ => "send",
        }),
        title = esc(e.title()),
        detail = esc(&detail),
        chip = chip,
        status_label = status_label,
        when = esc(&e.created_at.get(..10).unwrap_or(&e.created_at).to_string()),
        retry = retry,
    );

    // The row links to the post, which is where the edition's content and its
    // link clicks are. An edition whose post has been deleted is shown
    // without a link rather than hidden: the record of a send outlives the
    // post, and losing it from the screen would hide a failure.
    match (valid_id, ghost_core::ids::is_objectid(&e.post_id), e.post_title.is_some()) {
        (_, true, true) => format!(
            r#"<a class="row" href="/ghost/editor/post/{post}/">{title}</a>"#,
            post = esc(&e.post_id),
        ),
        _ => format!(r#"<div class="row">{title}</div>"#),
    }
}

/// The retry form.
///
/// `#[serde(default)]` and a plain `String`, so that a stranger posting an
/// empty body is refused by [`guard`] with the same `302 /ghost/signin/` as
/// every other route rather than by the extractor with a `422` — an axum
/// extractor runs *before* the handler, which would otherwise make "every
/// handler's first statement calls the guard" untrue. Be lenient in the
/// extractor and strict in the handler.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct RetryForm {
    id: String,
}

pub async fn edition_retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<RetryForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&form.id) {
        return flash_to(EDITIONS_PATH, "error", "That is not an edition.");
    }

    // Re-read rather than trusting the page that drew the button. Between the
    // render and the click the send loop may have finished this edition, and
    // retrying a succeeded send would mail everybody a second time.
    let edition = match repo::find_edition(&state.db, &form.id).await {
        Ok(Some(e)) => e,
        Ok(None) => return flash_to(EDITIONS_PATH, "error", "That edition no longer exists."),
        Err(e) => return server_error(&e),
    };
    if !rules::is_retryable(&edition.status) {
        let (label, _) = rules::edition_status(&edition.status);
        return flash_to(
            EDITIONS_PATH,
            "error",
            &format!(
                "“{}” is {}, not failed — there is nothing to send again.",
                edition.title(),
                label.to_lowercase(),
            ),
        );
    }

    match crate::retry_email(&state, &form.id).await {
        Ok(sent) => flash_to(
            EDITIONS_PATH,
            "ok",
            &format!(
                "“{}” sent again to {}.",
                edition.title(),
                countable(sent as i64, "subscriber"),
            ),
        ),
        // The message comes from the send path and names the real obstacle —
        // a missing transport, an exhausted allowance, nothing left to retry.
        // Passing it through beats replacing it with "it did not work".
        Err(message) => {
            tracing::error!("newsletters: retrying {} failed: {}", form.id, message);
            flash_to(EDITIONS_PATH, "error", &message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edition(status: &str) -> repo::Edition {
        repo::Edition {
            id: "0123456789abcdef01234567".into(),
            post_id: "0123456789abcdef01234568".into(),
            post_title: Some("The Friday letter".into()),
            post_slug: Some("the-friday-letter".into()),
            subject: Some("The Friday letter".into()),
            newsletter_name: Some("Weekly".into()),
            status: status.into(),
            error: None,
            created_at: "2026-11-14 09:00:00".into(),
            submitted_at: Some("2026-11-14 09:01:00".into()),
            attempted: 100,
            delivered: 98,
            opened: 40,
            failed: 2,
        }
    }

    #[test]
    fn only_a_failed_edition_offers_to_send_again() {
        assert!(edition_row(&edition("failed"), true).contains("Send again"));
        for status in ["submitted", "sent", "pending"] {
            assert!(
                !edition_row(&edition(status), true).contains("Send again"),
                "{status} was offered a retry"
            );
        }
    }

    #[test]
    fn the_compact_card_on_the_overview_offers_no_buttons() {
        // Five rows each with a Send again would be the loudest thing on a
        // page that is meant to be read.
        assert!(!edition_row(&edition("failed"), false).contains("Send again"));
        assert!(edition_row(&edition("failed"), false).contains(">Failed<"));
    }

    #[test]
    fn an_edition_with_an_unknown_id_is_never_posted_back() {
        let mut e = edition("failed");
        e.id = "not-an-id".into();
        let row = edition_row(&e, true);
        assert!(!row.contains("Send again"), "an unchecked id reached a form: {row}");
    }

    #[test]
    fn a_providers_error_is_escaped_and_truncated() {
        let mut e = edition("failed");
        e.error = Some(format!("<script>alert(1)</script> {}", "x".repeat(400)));
        let row = edition_row(&e, true);
        assert!(!row.contains("<script>"), "{row}");
        assert!(row.contains("&lt;script&gt;"));
        // 160 characters plus an ellipsis, not four hundred.
        assert!(row.contains('…'), "a long provider error was not truncated");
        assert!(!row.contains(&"x".repeat(200)), "the provider owned the page");
    }

    #[test]
    fn a_failed_edition_shows_why_and_a_successful_one_shows_its_numbers() {
        let mut e = edition("failed");
        e.error = Some("Mailgun rejected the domain".into());
        assert!(edition_row(&e, true).contains("Mailgun rejected the domain"));

        // A failure with no recorded reason falls back to the counts rather
        // than rendering an empty line.
        e.error = Some("   ".into());
        assert!(edition_row(&e, true).contains("100 attempted"));

        let ok = edition_row(&edition("submitted"), true);
        assert!(ok.contains("Weekly · 100 attempted · 98 delivered · 2 failed · 41% opened"), "{ok}");
    }

    #[test]
    fn an_edition_whose_post_is_gone_is_still_listed_without_a_link() {
        // The record of a send outlives the post. Hiding it would hide a
        // failure.
        let mut e = edition("failed");
        e.post_title = None;
        let row = edition_row(&e, true);
        assert!(!row.contains(r#"class="row" href=/ghost/editor"#));
        assert!(row.starts_with(r#"<div class="row">"#), "{row}");
        assert!(row.contains("Send again"), "a retry is still possible without the post");
    }

    #[test]
    fn a_title_is_escaped() {
        let mut e = edition("submitted");
        e.post_title = Some(r#"<img src=x onerror=alert(1)>"#.into());
        let row = edition_row(&e, true);
        assert!(!row.contains("<img src=x"));
        assert!(row.contains("&lt;img src=x"));
    }

    #[test]
    fn a_page_number_outside_the_data_reads_as_the_first_page() {
        let q = |raw: &str| {
            let mut map = HashMap::new();
            map.insert("page".to_string(), raw.to_string());
            map
        };
        // 60 editions is three pages of 25.
        assert_eq!(page_of(&q("2"), 60), 2);
        assert_eq!(page_of(&q("3"), 60), 3);
        for forged in ["4", "0", "-1", "99999", "two", "", "1e3"] {
            assert_eq!(page_of(&q(forged), 60), 1, "{forged} was believed");
        }
        assert_eq!(page_of(&HashMap::new(), 60), 1);
        // An empty table still has a first page.
        assert_eq!(page_of(&q("1"), 0), 1);
    }

    #[test]
    fn the_pager_appears_only_when_there_is_more_than_one_page() {
        assert_eq!(pager(1, 0), "");
        assert_eq!(pager(1, PER_PAGE), "");
        let two = pager(1, PER_PAGE + 1);
        assert!(two.contains("Page 1 of 2"));
        // No Previous on the first page, no Next on the last.
        assert!(!two.contains("Previous"));
        assert!(two.contains("Next"));
        let last = pager(2, PER_PAGE + 1);
        assert!(last.contains("Previous"));
        assert!(!last.contains("Next"));
    }
}
