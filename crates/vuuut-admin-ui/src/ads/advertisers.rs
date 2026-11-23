//! Advertisers: who the ads are for, and the report each of them is owed.
//!
//! An advertiser is a name, an address and some notes. Their page carries
//! their ads, 30 days of figures across all of them, and a CSV — day by day,
//! ad by ad — that can be sent to them as it is.

use super::*;

const LIST: &str = "/ghost/ads/advertisers/";

pub(crate) async fn advertisers_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let advertisers = repo::advertisers(&state.db).await.unwrap_or_default();
    let ads = repo::list(&state.db).await.unwrap_or_default();
    let rows: String = advertisers
        .iter()
        .map(|adv| {
            let theirs: Vec<&repo::Ad> = ads.iter().filter(|a| a.advertiser_id.as_deref() == Some(adv.id.as_str())).collect();
            let live = theirs.iter().filter(|a| a.status == "live").count();
            let imp: i64 = theirs.iter().map(|a| a.impressions).sum();
            let earned: i64 = theirs.iter().map(|a| rules::revenue_cents(a.impressions, a.clicks, a.cpm_cents, a.cpc_cents)).sum();
            format!(
                r#"<a class="row" href="{LIST}{id}/"><span class="ic tint-purple">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{ads} · {live} live{email}</span></div>
<div class="trailing ad-nums"><span><b>{imp}</b> seen</span><span><b>{earned}</b> earned</span>{chev}</div></a>"#,
                id = esc(&adv.id),
                ic = svg("person"),
                name = esc(&adv.name),
                ads = countable(theirs.len() as i64, "ad"),
                email = if adv.email.is_empty() { String::new() } else { format!(" · {}", esc(&adv.email)) },
                earned = rules::money(earned),
                chev = svg("chevron"),
            )
        })
        .collect();
    let list = if advertisers.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No advertisers yet. Ads without one are house ads — the site promoting itself.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };
    page_for_role(
        actor.role,
        "Advertisers",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="ad-split ad-split-even">
<div><div class="section">Advertisers</div><div class="rise" style="--i:2">{list}</div></div>
<div><div class="section">Add an advertiser</div>
<form class="card padded rise" style="--i:3" method="post" action="{LIST}">{fields}
<div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{add} Add advertiser</button></div></form></div>
</div>"#,
            head = head(""),
            tabs = tabs("advertisers"),
            banner = flash_banner(&q),
            fields = advertiser_fields(None),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

fn advertiser_fields(a: Option<&repo::Advertiser>) -> String {
    let v = |f: fn(&repo::Advertiser) -> &str| a.map(f).unwrap_or("");
    format!(
        r#"{name}{email}{web}<label class="field"><textarea name="notes" rows="3" placeholder=" " maxlength="1000">{notes}</textarea><span>Notes — contract, contact, terms</span></label>"#,
        name = input("name", "Name", v(|a| &a.name), "text", r#" required maxlength="120""#),
        email = input("email", "Email for reports", v(|a| &a.email), "email", r#" maxlength="191""#),
        web = input("website", "Website", v(|a| &a.website), "text", r#" maxlength="500" inputmode="url""#),
        notes = esc(v(|a| &a.notes)),
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct AdvertiserForm {
    name: String,
    email: String,
    website: String,
    notes: String,
}

/// Bounded and checked: a website is a link on the admin screen, so it has
/// to be one that cannot run.
fn clean_advertiser(f: &AdvertiserForm) -> Result<(String, String, String, String), &'static str> {
    let name = rules::clean(&f.name, 120);
    if name.is_empty() {
        return Err("An advertiser needs a name.");
    }
    let email = rules::clean(&f.email, 191);
    if !email.is_empty() && !vuuut_core::forms::is_email(&email) {
        return Err("That email address does not look right.");
    }
    let website = rules::safe_url(&f.website).ok_or("The website must be an http(s) address.")?;
    Ok((name, email, website, rules::clean(&f.notes, 1000)))
}

pub(crate) async fn advertiser_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AdvertiserForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let (name, email, website, notes) = match clean_advertiser(&form) {
        Ok(v) => v,
        Err(m) => return flash_to(LIST, "error", m),
    };
    match repo::save_advertiser(&state.db, None, &name, &email, &website, &notes).await {
        Ok(id) => flash_to(&format!("{LIST}{id}/"), "ok", &format!("{name} added.")),
        Err(e) => {
            tracing::error!("ads: adding an advertiser failed: {}", e.message());
            flash_to(LIST, "error", "That could not be saved.")
        }
    }
}

pub(crate) async fn advertiser_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That advertiser could not be found.");
    }
    let Ok(Some(adv)) = repo::advertiser(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That advertiser could not be found.");
    };
    let ads = repo::of_advertiser(&state.db, &id).await.unwrap_or_default();
    let ids: Vec<String> = ads.iter().map(|a| a.id.clone()).collect();
    let daily = repo::daily(&state.db, &day_ago(29), Some(&ids)).await.unwrap_or_default();
    let (imp, clk): (i64, i64) = daily.iter().fold((0, 0), |(i, c), r| (i + r.1, c + r.2));
    let since = day_ago(29);
    let per_ad = repo::totals_since(&state.db, &since).await.unwrap_or_default();
    let earned: i64 = per_ad
        .iter()
        .filter_map(|(aid, i, c)| ads.iter().find(|a| &a.id == aid).map(|a| rules::revenue_cents(*i, *c, a.cpm_cents, a.cpc_cents)))
        .sum();
    let rows: String = ads
        .iter()
        .map(|a| {
            format!(
                r#"<a class="row" href="{PATH}{id}/"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{status} · {place}</span></div>
<div class="trailing ad-nums"><span><b>{imp}</b> seen</span><span><b>{ctr}</b> CTR</span>{chev}</div></a>"#,
                id = esc(&a.id),
                tint = kind_tint(&a.kind),
                ic = svg(kind_icon(&a.kind)),
                name = esc(&a.name),
                status = esc(Status::from_key(&a.status).label()),
                place = esc(&placement_line(a)),
                imp = a.impressions,
                ctr = pct(rules::ctr(a.impressions, a.clicks)),
                chev = svg("chevron"),
            )
        })
        .collect();
    let list = if ads.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No ads for this advertiser yet.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };
    page_for_role(
        actor.role,
        &adv.name,
        "tools",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="{LIST}" title="Back to advertisers">{back}</a>
<div><h1>{name}</h1><p class="sub">{sub}</p></div>
<div class="actions"><a class="btn text" href="{LIST}{id}/report.csv">{dl} Report CSV</a><a class="btn" href="{PATH}new/?advertiser={id}">{add} New ad</a></div></header>
{banner}
<div class="ad-tiles rise" style="--i:1">{a}{b}{c}{d}</div>
<div class="section">Last 30 days</div><div class="rise" style="--i:2">{chart}</div>
<div class="ad-split ad-split-even">
<div><div class="section">Their ads</div>{list}</div>
<div><div class="section">Details</div>
<form class="card padded" method="post" action="{LIST}{id}/">{fields}
<div style="display:flex;justify-content:space-between;gap:8px">
<button class="btn tonal small danger" type="submit" form="adv-delete">{del} Delete</button>
<button class="btn" type="submit">{check} Save</button></div></form>
<form method="post" action="{LIST}{id}/delete/" id="adv-delete" data-confirm="Delete {name}|Their ads are kept, as house ads. Their figures stay with the ads."></form>
</div></div>"#,
            back = svg("chevron"),
            name = esc(&adv.name),
            sub = esc(&if adv.website.is_empty() { "Advertiser".to_string() } else { adv.website.clone() }),
            id = esc(&adv.id),
            dl = svg("import"),
            add = svg("add"),
            banner = flash_banner(&q),
            a = tile("Ads", &ads.len().to_string(), &format!("{} live", ads.iter().filter(|a| a.status == "live").count()), "campaign", "pink"),
            b = tile("Impressions", &imp.to_string(), "Last 30 days", "eye", "blue"),
            c = tile("Clicks", &clk.to_string(), &format!("{} click-through", pct(rules::ctr(imp, clk))), "trending", "purple"),
            d = tile("Earned", &rules::money(earned), "Last 30 days, at their prices", "chart", "amber"),
            chart = chart(&daily, 30),
            fields = advertiser_fields(Some(&adv)),
            del = svg("delete"),
            check = svg("check"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn advertiser_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<AdvertiserForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LIST, "error", "That advertiser could not be found.");
    }
    let back = format!("{LIST}{id}/");
    let (name, email, website, notes) = match clean_advertiser(&form) {
        Ok(v) => v,
        Err(m) => return flash_to(&back, "error", m),
    };
    match repo::save_advertiser(&state.db, Some(&id), &name, &email, &website, &notes).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(e) => {
            tracing::error!("ads: saving advertiser {id} failed: {}", e.message());
            flash_to(&back, "error", "That could not be saved.")
        }
    }
}

pub(crate) async fn advertiser_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LIST, "error", "That advertiser could not be found.");
    }
    match repo::delete_advertiser(&state.db, &id).await {
        Ok(()) => flash_to(LIST, "ok", "Advertiser deleted. Their ads are now house ads."),
        Err(e) => {
            tracing::error!("ads: deleting advertiser {id} failed: {}", e.message());
            flash_to(LIST, "error", "That could not be deleted.")
        }
    }
}

/// Day by day, ad by ad, for the last 90 days — the report an advertiser is
/// sent.
pub(crate) async fn advertiser_csv(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let Ok(Some(adv)) = repo::advertiser(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let ads = repo::of_advertiser(&state.db, &id).await.unwrap_or_default();
    let mut out = String::from("day,ad,impressions,clicks,ctr_percent,earned\n");
    for a in &ads {
        let rows = repo::daily(&state.db, &day_ago(89), Some(std::slice::from_ref(&a.id))).await.unwrap_or_default();
        for (day, i, c) in rows {
            out.push_str(&[
                cell(&day),
                cell(&a.name),
                cell(&i.to_string()),
                cell(&c.to_string()),
                cell(&rules::ctr(i, c).map(|r| format!("{r:.2}")).unwrap_or_default()),
                cell(&rules::money(rules::revenue_cents(i, c, a.cpm_cents, a.cpc_cents))),
            ].join(","));
            out.push('\n');
        }
    }
    let slug: String = adv.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    csv_response(out, &format!("ads-{}-{}.csv", slug.trim_matches('-'), crate::export_date_stamp()))
}
