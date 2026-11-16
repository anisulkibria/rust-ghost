//! Memberships → Tiers: the inventory, not the shop window.
//!
//! Every tier the site has, including the ones that are switched off and the
//! ones a visitor is never shown — because a tier that is off has to be
//! visible here or there is no way to turn it back on, and because "there are
//! three paid tiers and none of them is for sale" is the single most
//! expensive thing this admin used to be unable to tell anybody.
//!
//! The editor is `settings::tier_edit_page`, where it has always been. This
//! screen reads and links; the one form on it posts to
//! `settings::tier_create`, which is the handler that already created tiers
//! from Settings → Membership.

use super::*;

pub async fn tiers_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let stats = match repo::tier_stats(&state.db).await {
        Ok(s) => s,
        Err(e) => return server_error(&e),
    };

    let paid: Vec<_> = stats.iter().filter(|t| t.is_paid()).collect();
    let for_sale = paid.iter().filter(|t| t.is_active() && t.is_visible()).count();
    let held: i64 = stats.iter().map(|t| t.members).sum();

    // The sentence the screen exists to say. Three cases, because the
    // difference between them is what somebody needs to act on: nothing to
    // sell, something to sell that nobody can see, something to sell.
    let verdict = if paid.is_empty() {
        "There is no paid tier, so nothing on this site can be bought.".to_string()
    } else if for_sale == 0 {
        format!(
            "{} and none of them is both active and visible, so a visitor is never shown a way to pay.",
            countable(paid.len() as i64, "paid tier"),
        )
    } else {
        format!(
            "{} on sale out of {}. {} held across every tier.",
            countable(for_sale as i64, "paid tier"),
            countable(stats.len() as i64, "tier"),
            countable(held, "membership"),
        )
    };

    let body = if stats.is_empty() {
        r#"<div class="card padded rise" style="--i:2;text-align:center;padding:48px 20px">
<p style="margin:0 0 4px;font-weight:600;font-size:17px">No tiers yet</p>
<p class="muted" style="margin:0">A tier is what a member gets, and what they pay for it. Add the first one with <b>New tier</b>, above.</p></div>"#
            .to_string()
    } else {
        format!(
            r#"<div class="card rows rise" style="--i:2">{rows}</div>"#,
            rows = stats.iter().map(tier_row).collect::<String>(),
        )
    };

    page_for_role(
        actor.role,
        "Tiers",
        "memberships",
        Some(&actor.user),
        &format!(
            r#"{head}
{banner}
<div class="section">Tiers</div>
<p class="muted" style="margin:0 0 14px 16px">{verdict}</p>
{body}"#,
            head = head("tiers"),
            banner = flash_banner(&q),
            verdict = esc(&verdict),
            body = body,
        ),
        true,
    )
    .into_response()
}

/// One tier, as a row: what it is, what it costs, who has it.
fn tier_row(t: &repo::TierStat) -> String {
    let currency = t.currency.as_deref();
    let price = match (t.is_paid(), t.monthly_price, t.yearly_price) {
        (false, _, _) => "Free".to_string(),
        (true, None, None) => "No price set".to_string(),
        (true, m, y) => {
            let mut parts = Vec::new();
            if let Some(m) = m.filter(|m| *m > 0) {
                parts.push(format!("{} a month", money(m, currency)));
            }
            if let Some(y) = y.filter(|y| *y > 0) {
                parts.push(format!("{} a year", money(y, currency)));
            }
            if parts.is_empty() {
                "No price set".to_string()
            } else {
                parts.join(" · ")
            }
        }
    };
    let trial = match t.trial_days {
        0 => String::new(),
        n => format!(" · {} free trial", countable(n, "day")),
    };
    let detail = format!(
        "{price}{trial} · {visibility}{inactive}",
        price = price,
        trial = trial,
        visibility = rules::visibility_label(&t.visibility),
        inactive = if t.is_active() { "" } else { " · switched off" },
    );

    // The chip reports what a visitor actually experiences, which is the
    // conjunction of three columns and the thing nobody checks: a tier is on
    // sale only if it is paid, active *and* visible.
    let (chip, state_label) = if !t.is_active() {
        ("tonal", "Off")
    } else if !t.is_visible() {
        ("draft", "Hidden")
    } else if t.is_paid() {
        ("published", "On sale")
    } else {
        ("published", "Offered")
    };

    let inner = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{detail}</span></div>
<div class="trailing"><span class="chip {chip}">{state_label}</span>
<span class="chip tonal" style="margin-left:8px">{members}</span></div>"#,
        tint = if t.is_paid() { "amber" } else { "blue" },
        icon = svg(if t.is_paid() { "star" } else { "ghost" }),
        name = esc(&t.name),
        detail = esc(&detail),
        chip = chip,
        state_label = state_label,
        members = esc(&countable(t.members, "member")),
    );

    // Rule 8: an id is checked before it is built into a link. A row with an
    // id of an unexpected shape is shown without one rather than dropped —
    // hiding it would make a broken row invisible instead of inert.
    if ghost_core::ids::is_objectid(&t.id) {
        format!(
            r#"<a class="row" href="/ghost/settings/membership/tiers/{id}/">{inner}</a>"#,
            id = esc(&t.id),
        )
    } else {
        format!(r#"<div class="row">{inner}</div>"#)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(name: &str, kind: &str, active: i64, visibility: &str) -> repo::TierStat {
        repo::TierStat {
            id: "0123456789abcdef01234567".into(),
            name: name.into(),
            slug: "slug".into(),
            kind: kind.into(),
            visibility: visibility.into(),
            active,
            currency: Some("usd".into()),
            monthly_price: Some(500),
            yearly_price: Some(5_000),
            trial_days: 0,
            members: 3,
        }
    }

    #[test]
    fn a_tier_is_on_sale_only_when_it_is_paid_active_and_visible() {
        // The conjunction is the whole value of this screen. Each of the
        // three columns on its own looks fine on the tier editor.
        assert!(tier_row(&tier("Patron", "paid", 1, "public")).contains(">On sale<"));
        assert!(tier_row(&tier("Patron", "paid", 1, "none")).contains(">Hidden<"));
        assert!(tier_row(&tier("Patron", "paid", 0, "public")).contains(">Off<"));
        assert!(tier_row(&tier("Free", "free", 1, "public")).contains(">Offered<"));
    }

    #[test]
    fn a_price_is_shown_in_the_tiers_own_currency() {
        let mut t = tier("Patron", "paid", 1, "public");
        t.currency = Some("gbp".into());
        let row = tier_row(&t);
        assert!(row.contains("\u{a3}5.00 a month"), "{row}");
        assert!(row.contains("\u{a3}50.00 a year"), "{row}");
    }

    #[test]
    fn a_paid_tier_with_no_price_says_so_rather_than_showing_nothing() {
        let mut t = tier("Patron", "paid", 1, "public");
        t.monthly_price = None;
        t.yearly_price = None;
        assert!(tier_row(&t).contains("No price set"));
        // Nor does a zero count as a price: a tier at $0.00 is a half-made
        // one, and reading it as free would hide that.
        t.monthly_price = Some(0);
        t.yearly_price = Some(0);
        assert!(tier_row(&t).contains("No price set"));
    }

    #[test]
    fn a_free_tier_is_never_described_as_having_a_price() {
        let row = tier_row(&tier("Free", "free", 1, "public"));
        assert!(row.contains(">Free "), "{row}");
        assert!(!row.contains("a month"), "{row}");
    }

    #[test]
    fn a_trial_is_named_in_days() {
        let mut t = tier("Patron", "paid", 1, "public");
        t.trial_days = 1;
        assert!(tier_row(&t).contains("1 day free trial"));
        t.trial_days = 14;
        assert!(tier_row(&t).contains("14 days free trial"));
    }

    #[test]
    fn a_tier_name_is_escaped_and_a_bad_id_is_not_linked() {
        let mut t = tier(r#"<script>alert(1)</script>"#, "paid", 1, "public");
        t.id = "not-an-id".into();
        let row = tier_row(&t);
        assert!(!row.contains("<script>"));
        assert!(!row.contains("href="), "an unchecked id reached a link: {row}");
    }
}
