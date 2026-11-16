//! Memberships → Offers: every discount, and whether anybody took it.
//!
//! Offers were — and still are — edited under Settings → Growth, beside
//! recommendations and signup forms. That is a reasonable place to *build*
//! one and a poor place to judge one, because the question about an offer is
//! never "what are its settings"; it is "did it work", and the answer is a
//! redemption count that lives on the members table.
//!
//! So this screen reads and links. The editor is `settings::offer_edit_page`,
//! the status switch is `settings::offer_set_status`, and the one form here
//! posts to `settings::offer_new` — the handler that already refuses to
//! create an offer when there is no paid tier to discount.

use super::*;

pub async fn offers_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let stats = match repo::offer_stats(&state.db).await {
        Ok(s) => s,
        Err(e) => return server_error(&e),
    };

    let live = stats.iter().filter(|o| o.is_active()).count();
    let taken: i64 = stats.iter().map(|o| o.redemptions).sum();
    let verdict = if stats.is_empty() {
        "No offers. An offer is a discount on a paid tier with a link of its own, which is what \
         turns \"maybe later\" into a decision today."
            .to_string()
    } else {
        format!(
            "{} running out of {}. {} arrived through one.",
            countable(live as i64, "offer"),
            countable(stats.len() as i64, "offer"),
            countable(taken, "member"),
        )
    };

    let body = if stats.is_empty() {
        r#"<div class="card padded rise" style="--i:2;text-align:center;padding:48px 20px">
<p style="margin:0 0 4px;font-weight:600;font-size:17px">No offers yet</p>
<p class="muted" style="margin:0">Add the first one with <b>New offer</b>, above. An offer discounts a paid tier, so there has to be one.</p></div>"#
            .to_string()
    } else {
        format!(
            r#"<div class="card rows rise" style="--i:2">{rows}</div>"#,
            rows = stats.iter().map(offer_row).collect::<String>(),
        )
    };

    page_for_role(
        actor.role,
        "Offers",
        "memberships",
        Some(&actor.user),
        &format!(
            r#"{head}
{banner}
<div class="section">Offers</div>
<p class="muted" style="margin:0 0 14px 16px">{verdict}</p>
{body}"#,
            head = head("offers"),
            banner = flash_banner(&q),
            verdict = esc(&verdict),
            body = body,
        ),
        true,
    )
    .into_response()
}

/// What an offer takes off, in words.
///
/// Both halves are optional in the schema, which is the case worth handling
/// rather than unwrapping: an offer mid-edit can have a type and no amount,
/// and "20% off" with a missing 20 would read as "% off".
pub(crate) fn discount_label(o: &repo::OfferStat) -> String {
    let currency = o.currency.as_deref();
    match (o.discount_type.as_deref(), o.discount_amount) {
        (Some("percent"), Some(n)) => format!("{n}% off"),
        (Some("fixed"), Some(n)) => format!("{} off", money(n, currency)),
        (Some("trial"), Some(n)) => countable(n, "day").to_string() + " free",
        _ => "No discount set".into(),
    }
}

/// How long the discount lasts, in words.
fn duration_label(o: &repo::OfferStat) -> &'static str {
    let per = match o.interval.as_deref() {
        Some("year") => "year",
        _ => "month",
    };
    match (o.duration.as_deref(), per) {
        (Some("forever"), "year") => "every year",
        (Some("forever"), _) => "every month",
        (Some("repeating"), _) => "for the first few months",
        (Some("trial"), _) => "as a trial",
        (_, "year") => "on the first year",
        _ => "on the first month",
    }
}

/// One offer, as a row.
fn offer_row(o: &repo::OfferStat) -> String {
    let detail = format!(
        "{discount} {duration} · {tier} · code {code}",
        discount = discount_label(o),
        duration = duration_label(o),
        tier = o.tier_name.as_deref().unwrap_or("tier removed"),
        code = o.code,
    );
    let inner = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip {chip}">{state_label}</span>
<span class="chip tonal" style="margin-left:8px">{taken}</span></div>"#,
        tint = if o.is_active() { "green" } else { "slate" },
        icon = svg("flag"),
        name = esc(&o.name),
        // The code and the tier name are both free text somebody typed, so
        // the whole detail is escaped rather than its pieces trusted.
        detail = esc(&detail),
        chip = if o.is_active() { "published" } else { "tonal" },
        state_label = if o.is_active() { "Running" } else { "Off" },
        taken = esc(&countable(o.redemptions, "redemption")),
    );
    if ghost_core::ids::is_objectid(&o.id) {
        format!(
            r#"<a class="row" href="/ghost/settings/growth/offers/{id}/">{inner}</a>"#,
            id = esc(&o.id),
        )
    } else {
        format!(r#"<div class="row">{inner}</div>"#)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(active: i64) -> repo::OfferStat {
        repo::OfferStat {
            id: "0123456789abcdef01234567".into(),
            name: "Spring sale".into(),
            code: "spring".into(),
            active,
            discount_type: Some("percent".into()),
            discount_amount: Some(20),
            duration: Some("once".into()),
            interval: Some("month".into()),
            currency: Some("usd".into()),
            tier_name: Some("Supporter".into()),
            redemptions: 4,
        }
    }

    #[test]
    fn a_percentage_and_a_fixed_discount_both_read_as_money_off() {
        let mut o = offer(1);
        assert_eq!(discount_label(&o), "20% off");
        o.discount_type = Some("fixed".into());
        o.discount_amount = Some(250);
        assert_eq!(discount_label(&o), "$2.50 off");
    }

    #[test]
    fn a_half_made_offer_says_so_rather_than_rendering_a_gap() {
        let mut o = offer(1);
        o.discount_amount = None;
        assert_eq!(discount_label(&o), "No discount set");
        o.discount_type = None;
        o.discount_amount = Some(20);
        assert_eq!(discount_label(&o), "No discount set");
    }

    #[test]
    fn the_duration_follows_the_interval_so_a_yearly_offer_does_not_say_month() {
        let mut o = offer(1);
        assert_eq!(duration_label(&o), "on the first month");
        o.interval = Some("year".into());
        assert_eq!(duration_label(&o), "on the first year");
        o.duration = Some("forever".into());
        assert_eq!(duration_label(&o), "every year");
        o.interval = Some("month".into());
        assert_eq!(duration_label(&o), "every month");
    }

    #[test]
    fn a_running_offer_and_a_switched_off_one_are_told_apart() {
        assert!(offer_row(&offer(1)).contains(">Running<"));
        assert!(offer_row(&offer(0)).contains(">Off<"));
    }

    #[test]
    fn an_offer_whose_tier_has_gone_says_so_instead_of_rendering_nothing() {
        let mut o = offer(1);
        o.tier_name = None;
        assert!(offer_row(&o).contains("tier removed"));
    }

    #[test]
    fn the_code_and_the_name_are_escaped_and_a_bad_id_is_not_linked() {
        let mut o = offer(1);
        o.name = r#"<script>alert(1)</script>"#.into();
        o.code = r#"a"onmouseover="x"#.into();
        o.id = "nope".into();
        let row = offer_row(&o);
        assert!(!row.contains("<script>"));
        assert!(!row.contains(r#"onmouseover=""#));
        assert!(!row.contains("href="), "an unchecked id reached a link: {row}");
    }

    #[test]
    fn the_redemption_count_is_shown_even_when_it_is_nought() {
        // "0 redemptions" is the answer somebody opened this screen for.
        let mut o = offer(1);
        o.redemptions = 0;
        assert!(offer_row(&o).contains("0 redemptions"));
    }
}
