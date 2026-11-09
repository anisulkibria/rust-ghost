//! Native Click reports and a private standalone connector form.
use super::*;
use ghost_api::click::{self as connector, Connection};
use serde_json::Value;

pub async fn page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let days = q
        .get("days")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(30)
        .clamp(1, 90);
    let view = q.get("view").map(String::as_str).unwrap_or("traffic");
    let mut body=r#"<header class="page-head"><div><h1>Vuuut Click</h1><p class="sub">Traffic, member activity and revenue</p></div><a class="btn tonal" href="/ghost/analytics/">Site analytics</a></header>"#.to_string();
    body.push_str("<nav class=\"range-tabs\" aria-label=\"Report period\">");
    for range in [7, 30, 90] {
        let mut url = reqwest::Url::parse("http://localhost/").unwrap();
        url.query_pairs_mut()
            .append_pair("days", &range.to_string())
            .append_pair("view", view);
        if let Some(path) = q.get("path") {
            url.query_pairs_mut().append_pair("path", path);
        }
        body.push_str(&format!(
            "<a href=\"?{}\">{range} days</a>",
            esc(url.query().unwrap_or_default())
        ));
    }
    body.push_str("</nav>");
    body.push_str("<nav aria-label=\"Click reports\" style=\"display:flex;flex-wrap:wrap;gap:8px;margin:14px 0\">");
    for (key, label) in [
        ("traffic", "Traffic"),
        ("content", "Content & engagement"),
        ("retention", "Returning visitors"),
        ("newsletters", "Newsletter editions"),
        ("campaigns", "Campaign returns"),
        ("performance", "Performance"),
    ] {
        body.push_str(&format!(
            "<a class=\"btn tonal\" href=\"?view={key}&amp;days={days}\">{label}</a>"
        ));
    }
    body.push_str("</nav>");
    if let Some(message) = q.get("message") {
        body.push_str(&format!("<p role=\"status\">{}</p>", esc(message)));
    }
    let c = connector::connection(&state).await;
    match c {
        Ok(Some(ref c)) => {
            body.push_str(&format!(
                "<p>Connected to {} · Website {}</p>",
                esc(&c.base_url),
                esc(&c.website_id)
            ));
            let role = if actor.role == Role::Owner {
                "Owner"
            } else {
                "Administrator"
            };
            match connector::report_view(
                &state,
                &actor.user.id,
                role,
                days,
                q.get("path").map(String::as_str),
                view,
            )
            .await
            {
                Ok(report) => {
                    if view == "traffic" {
                        body.push_str(&render_report(&report, days));
                        body.push_str(&cms_snapshot(&state).await);
                        body.push_str(&newsletter_editions(&state, c, &report).await);
                    } else if view == "newsletters" {
                        body.push_str(&newsletter_editions(&state, c, &report).await);
                    } else {
                        body.push_str(&render_extended(&report, view));
                    }
                    if view == "campaigns" && actor.role == Role::Owner {
                        body.push_str(&cost_form());
                    }
                }
                Err(ref e) => body.push_str(&format!(
                    "<p class=\"banner\" role=\"alert\">{}</p>",
                    esc(&e.message())
                )),
            }
        }
        Ok(None) => body
            .push_str("<p>Connect a verified website in Vuuut Click to see its reports here.</p>"),
        Err(ref e) => body.push_str(&format!("<p role=\"alert\">{}</p>", esc(&e.message()))),
    }
    let queue: Vec<(String, i64)> =
        sqlx::query_as("SELECT state,COUNT(*) FROM click_outbox GROUP BY state")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
    for (state, count) in queue {
        body.push_str(&format!("<p>{} event deliveries: {count}</p>", esc(&state)));
    }
    if actor.role == Role::Owner {
        let managed = !state.config.host_settings["click"].is_null();
        if managed {
            body.push_str(
                "<p>Your hosting platform manages this connection, its credentials and plan.</p>",
            );
        } else {
            body.push_str(r#"<section class="card padded"><h2>Connection</h2><p>In Click, create and verify this website, then create an integration credential in Workspace &amp; settings. Its origin must match this site's public URL.</p><form method="post" action="/ghost/analytics/click/"><input type="hidden" name="action" value="connect"><label class="field"><input type="url" name="base_url" placeholder="https://click.example.com" required><span>Click server</span></label><label class="field"><input name="website_id" placeholder=" " required><span>Website ID</span></label><label class="field"><input type="password" name="integration_key" placeholder=" " autocomplete="new-password" required><span>Integration credential</span></label><label style="display:flex;align-items:center;gap:10px;margin:18px 0"><input type="checkbox" name="consent_required" value="1" checked> Require visitor consent before tracking</label><label style="display:flex;align-items:center;gap:10px;margin:18px 0"><input type="checkbox" name="engagement" value="1"> Measure visible time and scroll depth (additional events)</label><label style="display:flex;align-items:center;gap:10px;margin:18px 0"><input type="checkbox" name="persistent_identity" value="1"> Use persistent visitor IDs after consent</label><button class="btn" type="submit">Connect and test</button></form></section>"#);
            if let Ok(Some(ref current)) = c {
                body.push_str(&format!("<section class=\"card padded\"><h2>Measurement preferences</h2><form method=\"post\" action=\"/ghost/analytics/click/\"><input type=\"hidden\" name=\"action\" value=\"measurement\"><label style=\"display:flex;gap:10px;margin:12px 0\"><input type=\"checkbox\" name=\"consent_required\" {}> Require visitor consent</label><label style=\"display:flex;gap:10px;margin:12px 0\"><input type=\"checkbox\" name=\"engagement\" {}> Measure visible time and scroll depth</label><label style=\"display:flex;gap:10px;margin:12px 0\"><input type=\"checkbox\" name=\"persistent_identity\" {}> Use persistent visitor IDs after consent</label><p class=\"muted\">Engagement is optional and adds events to your allowance. DNT and GPC remain respected.</p><button class=\"btn tonal\">Save measurement preferences</button></form></section>",if current.consent_required{"checked"}else{""},if current.engagement{"checked"}else{""},if current.persistent_identity{"checked"}else{""}));
            }
            body.push_str(r#"<form class="inline-form" style="display:flex;flex-wrap:wrap;gap:8px;margin:12px 0" method="post" action="/ghost/analytics/click/"><button class="btn tonal" name="action" value="rotate">Rotate credential</button> <button class="btn tonal" name="action" value="disconnect">Disconnect</button></form>"#);
        }
        body.push_str(r#"<form class="inline-form" style="display:flex;flex-wrap:wrap;gap:8px;margin:12px 0" method="post" action="/ghost/analytics/click/"><button class="btn tonal" name="action" value="test">Test connection</button> <button class="btn tonal" name="action" value="retry">Retry event deliveries</button></form>"#);
    }
    if actor.role == Role::Owner
        && ghost_stripe::StripeConfig::from_settings(&state.settings).is_some()
    {
        body.push_str(r#"<section class="card padded"><h2>Payment events</h2><p>Renewal and refund reporting requires invoice.payment_succeeded and charge.refunded in the CMS Stripe webhook. Update an existing endpoint after upgrading this CMS.</p><form class="inline-form" style="display:flex;flex-wrap:wrap;gap:8px;margin:12px 0" method="post" action="/ghost/analytics/click/"><button class="btn tonal" name="action" value="webhooks">Update Stripe webhook events</button></form></section>"#);
    }
    let mut response = page_for_role(
        actor.role,
        "Vuuut Click",
        "analytics",
        Some(&actor.user),
        &body,
        true,
    )
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    response
}
fn render_report(r: &Value, days: u32) -> String {
    let mut html=String::from("<p class=\"muted\">Reports cover the whole website, except the selected post section.</p><section class=\"card padded\"><h2>Overview</h2><table class=\"breakdown\"><thead><tr><th scope=\"col\">Metric</th><th scope=\"col\">Value</th></tr></thead><tbody>");
    for (key, label) in [
        ("pageviews", "Pageviews"),
        ("visitors", "Visitors"),
        ("sessions", "Sessions"),
        ("realtime", "Active now"),
        ("bounce_rate", "Bounce rate (%)"),
    ] {
        html.push_str(&format!(
            "<tr><th scope=\"row\">{label}</th><td>{}</td></tr>",
            esc(&r["overview"][key].to_string())
        ));
    }
    html.push_str("</tbody></table></section>");
    if let Some(page) = r["native"]["page"].as_object() {
        let path = page.get("path").and_then(Value::as_str).unwrap_or("/");
        html.push_str(&format!("<section class=\"card padded\"><h2>Selected post traffic</h2><p>{}</p><table class=\"breakdown\"><thead><tr><th scope=\"col\">Metric</th><th scope=\"col\">Total</th></tr></thead><tbody>",esc(path)));
        for (key, label) in [
            ("pageviews", "Pageviews"),
            ("visitors", "Visitors"),
            ("sessions", "Sessions"),
        ] {
            html.push_str(&format!(
                "<tr><th scope=\"row\">{label}</th><td>{}</td></tr>",
                esc(&page.get(key).map(Value::to_string).unwrap_or_default())
            ));
        }
        html.push_str("</tbody></table><p class=\"muted\">Slash and non-slash forms of this CMS post share the report.</p></section>");
    }
    html.push_str(&activity_report(r));

    for (key, label) in [
        ("timeseries", "Daily traffic"),
        ("pages", "Pages"),
        ("sources", "Sources"),
        ("countries", "Countries"),
        ("devices", "Devices"),
        ("events", "Events"),
        ("revenue", "Net revenue by source"),
    ] {
        let data = if key == "timeseries" {
            &r["overview"][key]
        } else {
            &r[key]
        };
        html.push_str(&format!("<section class=\"card padded\"><h2>{label}</h2>"));
        if let Some(rows) = data.as_array().filter(|a| !a.is_empty()) {
            html.push_str("<div style=\"overflow-x:auto\"><table class=\"breakdown\"><thead><tr><th scope=\"col\">Source or value</th><th scope=\"col\">Total</th></tr></thead><tbody>");
            for row in rows {
                let value = if key == "revenue" {
                    let currency = row["currency"].as_str().unwrap_or("");
                    let minor = row["total_minor"]
                        .as_str()
                        .and_then(|v| v.parse::<i128>().ok())
                        .unwrap_or(0);
                    if currency == "JPY" {
                        format!("JPY {minor}")
                    } else {
                        let abs = minor.unsigned_abs();
                        format!(
                            "{currency} {}{}.{:02}",
                            if minor < 0 { "-" } else { "" },
                            abs / 100,
                            abs % 100
                        )
                    }
                } else {
                    row["count"].to_string()
                };
                let label = match row["label"].as_str().unwrap_or("") {
                    "" if matches!(key, "sources" | "revenue") => "Direct",
                    "" | "ZZ" => "Unknown",
                    label => label,
                };
                let label = if key == "pages" {
                    let mut url =
                        reqwest::Url::parse("http://localhost/ghost/analytics/click/").unwrap();
                    url.query_pairs_mut()
                        .append_pair("days", &days.to_string())
                        .append_pair("path", label);
                    format!(
                        "<a href=\"?{}\">{}</a>",
                        esc(url.query().unwrap_or_default()),
                        esc(label)
                    )
                } else {
                    esc(label)
                };
                html.push_str(&format!(
                    "<tr><th scope=\"row\">{label}</th><td>{}</td></tr>",
                    esc(&value)
                ));
            }
            html.push_str("</tbody></table></div>");
        } else {
            html.push_str("<p>No events in this period.</p>");
        }
        html.push_str("</section>");
    }
    html
}
pub async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Owner).await {
        return r;
    }
    let origin = reqwest::Url::parse(&state.site_url())
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(origin.as_str()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let action = f.get("action").map(String::as_str).unwrap_or("");
    if !state.config.host_settings["click"].is_null()
        && matches!(action, "connect" | "disconnect" | "rotate" | "measurement")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let result = match action {
        "connect" => {
            connector::connect(
                &state,
                Connection {
                    base_url: f.get("base_url").cloned().unwrap_or_default(),
                    website_id: f.get("website_id").cloned().unwrap_or_default(),
                    integration_key: f.get("integration_key").cloned().unwrap_or_default(),
                    collection_id: String::new(),
                    consent_required: f.contains_key("consent_required"),
                    engagement: f.contains_key("engagement"),
                    persistent_identity: f.contains_key("persistent_identity"),
                    pending_key: None,
                },
            )
            .await
        }
        "cost"=>async {
            let amount=f.get("amount_minor").and_then(|s|s.parse::<i64>().ok()).ok_or_else(||ghost_core::GhostError::BadRequest("Use integer currency minor units".into()))?;
            let revision=f.get("revision").and_then(|s|s.parse::<i64>().ok()).ok_or_else(||ghost_core::GhostError::BadRequest("Use a positive record version".into()))?;
            connector::record_cost(&state,&serde_json::json!({"reference":f.get("reference"),"revision":revision,"day":f.get("day"),"campaign":f.get("campaign"),"currency":f.get("currency"),"amount_minor":amount,"source":f.get("source")})).await
        }.await,
        "measurement" => {
            async {
                let mut c = connector::connection(&state).await?.ok_or_else(|| {
                    ghost_core::GhostError::BadRequest("Connect a website first".into())
                })?;
                c.consent_required = f.contains_key("consent_required");
                c.engagement = f.contains_key("engagement");
                c.persistent_identity = f.contains_key("persistent_identity");
                connector::connect(&state, c).await
            }
            .await
        }
        "disconnect" => connector::disconnect(&state).await,
        "rotate" => connector::rotate(&state).await,
        "retry" => connector::retry(&state).await,
        "webhooks" => connector::sync_stripe_webhooks(&state).await,
        "test" => connector::report(&state, "connector-test", "Owner", 1)
            .await
            .map(|_| ()),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let message = match result {
        Ok(()) => match action {
            "cost" => "Cost record saved",
            "measurement" => "Measurement preferences saved",
            _ => "Connection updated",
        }
        .into(),
        Err(e) => e.message(),
    };
    let mut url = reqwest::Url::parse("http://localhost/ghost/analytics/click/").unwrap();
    url.query_pairs_mut().append_pair("message", &message);
    if action == "cost" {
        url.query_pairs_mut().append_pair("view", "campaigns");
    }
    Redirect::to(&format!(
        "{}?{}",
        url.path(),
        url.query().unwrap_or_default()
    ))
    .into_response()
}

fn activity_report(r: &Value) -> String {
    let count = |kind: &str| {
        r["native"]["activity"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["label"] == format!("server.{kind}"))
            .and_then(|row| row["count"].as_u64())
            .unwrap_or(0)
    };
    let mut html = String::new();
    for (title, rows) in [
        (
            "Member growth and subscriptions",
            vec![
                ("New member signups", "member_signup"),
                ("Subscription activations", "subscription_started"),
                ("Subscription cancellations", "subscription_cancelled"),
            ],
        ),
        (
            "Newsletter performance",
            vec![
                ("Sent deliveries", "newsletter_sent"),
                ("First recipient opens", "newsletter_open"),
                ("Unique link / recipient clicks", "newsletter_click"),
            ],
        ),
    ] {
        html.push_str(&format!("<section class=\"card padded\"><h2>{title}</h2><p class=\"muted\">Recorded events in this period.</p><table class=\"breakdown\"><thead><tr><th scope=\"col\">Activity</th><th scope=\"col\">Total</th></tr></thead><tbody>"));
        for (label, kind) in rows {
            html.push_str(&format!(
                "<tr><th scope=\"row\">{label}</th><td>{}</td></tr>",
                count(kind)
            ));
        }
        html.push_str("</tbody></table></section>");
    }
    html
}
async fn cms_snapshot(state: &AppState) -> String {
    let data=sqlx::query_as::<_,(i64,i64,i64)>("SELECT COUNT(*),(SELECT COUNT(*) FROM members WHERE status='paid'),(SELECT COUNT(*) FROM subscriptions WHERE status IN ('active','trialing')) FROM members").fetch_one(&state.db).await;
    match data {
        Ok((members,paid,subscriptions))=>format!("<section class=\"card padded\"><h2>Current CMS membership</h2><p class=\"muted\">Current source counts, independent of the selected analytics period.</p><table class=\"breakdown\"><tbody><tr><th scope=\"row\">All members</th><td>{members}</td></tr><tr><th scope=\"row\">Paid members</th><td>{paid}</td></tr><tr><th scope=\"row\">Active or trial subscriptions</th><td>{subscriptions}</td></tr></tbody></table></section>"),
        Err(_)=>"<p role=\"alert\">Current CMS membership counts are unavailable.</p>".into(),
    }
}
pub async fn post_report_link(state: &AppState, paths: &[String]) -> String {
    if !matches!(connector::connection(state).await, Ok(Some(_))) {
        return String::new();
    }
    let Some(path) = paths.first() else {
        return String::new();
    };
    let mut url = reqwest::Url::parse("http://localhost/ghost/analytics/click/").unwrap();
    url.query_pairs_mut()
        .append_pair("days", "30")
        .append_pair("path", path);
    format!("<p><a class=\"btn tonal\" href=\"/ghost/analytics/click/?{}\">Vuuut Click post report</a></p>",esc(url.query().unwrap_or_default()))
}

async fn newsletter_editions(state: &AppState, c: &Connection, r: &Value) -> String {
    let rows:Vec<(String,String)>=sqlx::query_as("SELECT e.id,coalesce(p.title,'Untitled edition') FROM emails e LEFT JOIN posts p ON p.id=e.post_id ORDER BY e.created_at DESC LIMIT 200").fetch_all(&state.db).await.unwrap_or_default();
    let titles: HashMap<String, String> = rows
        .into_iter()
        .map(|(id, title)| (connector::edition_hash(&c.website_id, &id), title))
        .collect();
    let mut html=String::from("<section class=\"card padded\"><h2>Newsletter editions</h2><div style=\"overflow-x:auto\"><table class=\"breakdown\"><thead><tr><th scope=\"col\">Edition</th><th scope=\"col\">Sent recipients</th><th scope=\"col\">Opened</th><th scope=\"col\">Clicked</th><th scope=\"col\">Open rate</th><th scope=\"col\">Click rate</th></tr></thead><tbody>");
    let editions = r["newsletters"]["editions"].as_array();
    if let Some(editions) = editions.filter(|e| !e.is_empty()) {
        for row in editions {
            let id = row["newsletter_id"].as_str().unwrap_or("");
            let title = titles
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("Edition {}", id.chars().take(12).collect::<String>()));
            let sent = row["sent_recipients"].as_u64().unwrap_or(0);
            let opened = row["opened_recipients"].as_u64().unwrap_or(0);
            let clicked = row["clicked_recipients"].as_u64().unwrap_or(0);
            let rate = |n: u64| {
                if sent == 0 {
                    "Unavailable".to_string()
                } else {
                    format!("{:.1}%", n as f64 / sent as f64 * 100.0)
                }
            };
            html.push_str(&format!("<tr><th scope=\"row\">{}</th><td>{sent}</td><td>{opened}</td><td>{clicked}</td><td>{}</td><td>{}</td></tr>",esc(&title),rate(opened),rate(clicked)));
        }
    } else {
        html.push_str(
            "<tr><td colspan=\"6\">No captured newsletter send cohorts in this period.</td></tr>",
        );
    }
    html.push_str(&format!("</tbody></table></div><p class=\"muted\">Recipients sent in this period, with matched activity through its end. {} unassigned facts; {} open/click facts outside the captured send cohort. Titles stay in this CMS.</p></section>",r["newsletters"]["unassigned_events"].as_u64().unwrap_or(0),r["newsletters"]["activity_outside_send_cohort"].as_u64().unwrap_or(0)));
    html
}

fn extra_table(title: &str, headings: &[&str], rows: &str, note: &str) -> String {
    let head = headings
        .iter()
        .map(|h| format!("<th scope=\"col\">{}</th>", esc(h)))
        .collect::<String>();
    let empty = format!(
        "<tr><td colspan=\"{}\">No recorded measurements in this period.</td></tr>",
        headings.len()
    );
    format!("<section class=\"card padded\"><h2>{}</h2><div style=\"overflow-x:auto\"><table class=\"breakdown\"><thead><tr>{head}</tr></thead><tbody>{}</tbody></table></div><p class=\"muted\">{}</p></section>",esc(title),if rows.is_empty(){&empty}else{rows},esc(note))
}
fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
fn td(value: impl std::fmt::Display) -> String {
    format!("<td>{}</td>", esc(&value.to_string()))
}
fn measured_seconds(v: &Value) -> String {
    let n = count(v, "measured_sessions");
    if n == 0 {
        "Unavailable".into()
    } else {
        format!("{:.1} s", count(v, "visible_ms") as f64 / 1000.0 / n as f64)
    }
}
fn render_extended(r: &Value, view: &str) -> String {
    let mut rows = String::new();
    match view {
        "content" => {
            if let Some(data) = r["content"].as_array() {
                for v in data {
                    rows.push_str(&format!(
                        "<tr><th scope=\"row\">{}</th>{}{}{}{}{}{}{}</tr>",
                        esc(v["path"].as_str().unwrap_or("")),
                        td(count(v, "pageviews")),
                        td(count(v, "sessions")),
                        td(count(v, "entries")),
                        td(count(v, "exits")),
                        td(count(v, "converting_sessions")),
                        td(measured_seconds(v)),
                        td(v["mean_scroll_percent"]
                            .as_f64()
                            .map(|n| format!("{n:.1}%"))
                            .unwrap_or_else(|| "Unavailable".into()))
                    ));
                }
            }
            extra_table("Website content and engagement",&["Page","Views","Sessions","Entries","Exits","Goal sessions","Visible seconds / measured session","Mean maximum scroll"],&rows,"Top 100 paths. Entry and exit are within this selected period. Goal sessions also viewed this path; this is association. Engagement is observed visible time, with unavailable values when optional measurement has no coverage.")
        }
        "retention" => {
            let identity = &r["retention"]["identity"];
            for (key, label) in [
                ("new_visitors", "New provided visitor IDs"),
                ("returning_visitors", "Returning provided visitor IDs"),
                ("session_only_sessions", "Session-only sessions"),
                ("unclassified_sessions", "Unclassified sessions"),
            ] {
                rows.push_str(&format!(
                    "<tr><th scope=\"row\">{label}</th>{}</tr>",
                    td(count(identity, key))
                ));
            }
            let mut html=extra_table("Visitor identity coverage",&["Recorded identity","Count"],&rows,"Returning browsers require explicit persistent IDs after consent. Session-only and legacy records cannot identify returning browsers. Identifiers do not identify people.");
            rows.clear();
            if let Some(cohorts) = r["retention"]["cohorts"].as_array() {
                for c in cohorts {
                    let size = count(c, "visitors");
                    rows.push_str(&format!(
                        "<tr><th scope=\"row\">{}</th>{}",
                        esc(c["week"].as_str().unwrap_or("")),
                        td(size)
                    ));
                    if let Some(cells) = c["retained"].as_array() {
                        for value in cells {
                            rows.push_str(&td(value
                                .as_u64()
                                .filter(|_| size > 0)
                                .map(|n| {
                                    format!("{:.1}% ({n}/{size})", n as f64 / size as f64 * 100.0)
                                })
                                .unwrap_or_else(|| "Not complete".into())));
                        }
                    }
                    rows.push_str("</tr>");
                }
            }
            html.push_str(&extra_table("Weekly visitor retention",&["First observed UTC week","Visitor IDs","Week 0","Week 1","Week 2","Week 3","Week 4"],&rows,"First observed means within retained history, not a lifetime first visit. New IDs in this selected period form cohorts. Incomplete follow-up weeks remain unavailable."));
            html
        }
        "campaigns" => {
            if let Some(data) = r["campaigns"].as_array() {
                for v in data {
                    let currency = v["currency"].as_str().unwrap_or("");
                    let revenue = if v["revenue_recorded"] == true {
                        minor_money(v["net_revenue_minor"].as_str().unwrap_or("0"), currency)
                    } else {
                        "No revenue recorded".into()
                    };
                    let cost = v["cost_minor"]
                        .as_str()
                        .map(|n| minor_money(n, currency))
                        .unwrap_or_else(|| "No cost recorded".into());
                    rows.push_str(&format!(
                        "<tr><th scope=\"row\">{}</th>{}{}{}{}{}{}</tr>",
                        esc(v["campaign"].as_str().unwrap_or("")),
                        td(currency),
                        td(revenue),
                        td(cost),
                        td(format!(
                            "{} / {} days",
                            count(v, "covered_days"),
                            count(v, "expected_days")
                        )),
                        td(v["roi_percent"]
                            .as_f64()
                            .map(|n| format!("{n:.1}%"))
                            .unwrap_or_else(|| "Unavailable".into())),
                        td(v["roas"]
                            .as_f64()
                            .map(|n| format!("{n:.2}×"))
                            .unwrap_or_else(|| "Unavailable".into()))
                    ));
                }
            }
            extra_table("Campaign costs and returns",&["Campaign","Currency","Net revenue","Recorded cost","Cost coverage","ROI","ROAS"],&rows,"Returns require complete campaign attribution, observed same-currency revenue, positive recorded cost and explicit daily coverage. Missing, incomplete and zero cost remain unavailable. Supplied costs are not independently provider-verified. No currency conversion is inferred.")
        }
        "performance" => {
            let current = r["performance"]["current"].as_array();
            let previous = r["performance"]["previous"].as_array();
            for metric in ["LCP", "INP", "CLS", "FCP", "TTFB"] {
                let c = current.and_then(|a| a.iter().find(|v| v["metric"] == metric));
                let p = previous.and_then(|a| a.iter().find(|v| v["metric"] == metric));
                if c.is_none() && p.is_none() {
                    continue;
                }
                let value = |v: Option<&Value>| {
                    v.and_then(|v| v["p75"].as_f64())
                        .map(|n| format!("{n:.2}{}", if metric == "CLS" { "" } else { " ms" }))
                        .unwrap_or_else(|| "Unavailable".into())
                };
                rows.push_str(&format!(
                    "<tr><th scope=\"row\">{metric}</th>{}{}{}{}</tr>",
                    td(value(c)),
                    td(c.map(|v| count(v, "samples")).unwrap_or(0)),
                    td(value(p)),
                    td(p.map(|v| count(v, "samples")).unwrap_or(0))
                ));
            }
            extra_table("Web Vitals period comparison",&["Metric","Current p75","Current samples","Previous p75","Previous samples"],&rows,"Previous values use the preceding equal-duration window. Lower values indicate faster or more stable experiences. Missing measurements remain unavailable; sample sizes may differ.")
        }
        _ => String::new(),
    }
}
fn minor_money(raw: &str, currency: &str) -> String {
    match raw.parse::<i128>() {
        Ok(n) if currency == "JPY" => format!("JPY {n}"),
        Ok(n) => format!(
            "{currency} {}{}.{:02}",
            if n < 0 { "-" } else { "" },
            n.unsigned_abs() / 100,
            n.unsigned_abs() % 100
        ),
        Err(_) => "Unavailable".into(),
    }
}
fn cost_form() -> String {
    format!(
        r#"<section class="card padded"><h2>Record an advertising cost</h2><form method="post" action="/ghost/analytics/click/"><input type="hidden" name="action" value="cost"><label class="field"><input name="campaign" maxlength="120" required placeholder=" "><span>Campaign label</span></label><label class="field"><input name="day" type="date" value="{}" required><span>UTC cost date</span></label><label class="field"><input name="currency" maxlength="3" value="USD" required><span>Currency code</span></label><label class="field"><input name="amount_minor" type="number" min="0" step="1" required placeholder=" "><span>Amount in minor currency units</span></label><p>USD 5.25 is 525 minor units; JPY 500 is 500.</p><label class="field"><input name="source" maxlength="32" pattern="[a-z0-9_-]+" required placeholder=" "><span>Cost source</span></label><label class="field"><input name="reference" maxlength="200" required placeholder=" "><span>Source record reference</span></label><label class="field"><input name="revision" type="number" min="1" value="1" required><span>Record version</span></label><p class="muted">Enter amounts from your advertising ledger. Corrections keep the same source/reference and increase the version. Explicit zero-cost days establish coverage. Do not enter personal data or credentials.</p><button class="btn" type="submit">Save cost record</button></form></section>"#,
        chrono::Utc::now().date_naive()
    )
}

/// Click is the traffic source for the normal CMS Analytics destination.
pub async fn analytics_home(
    state: &AppState,
    headers: &HeaderMap,
    q: &HashMap<String, String>,
) -> Response {
    let actor = match require_role(state, headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let days = q
        .get("days")
        .or_else(|| q.get("range"))
        .and_then(|s| s.trim_end_matches('d').parse::<u32>().ok())
        .unwrap_or(30)
        .clamp(1, 90);
    let days = if matches!(days, 1 | 7 | 30 | 90) {
        days
    } else {
        30
    };
    let tab = q.get("tab").map(String::as_str).unwrap_or("overview");
    let role = if actor.role == Role::Owner {
        "Owner"
    } else {
        "Administrator"
    };
    let mut body = format!(
        r#"<header class="page-head"><div><h1>Analytics</h1><p class="sub">Last {days} days · Vuuut Click</p></div><a class="btn tonal" href="/ghost/analytics/click/">Connection &amp; advanced reports</a></header><nav class="range-tabs" aria-label="Report period">"#
    );
    for n in [7, 30, 90] {
        body.push_str(&format!(
            "<a href=\"?tab={}&amp;days={n}\">{n} days</a>",
            esc(tab)
        ));
    }
    body.push_str("</nav><nav class=\"seg-bar\" aria-label=\"Analytics reports\">");
    let tabs = [
        ("overview", "Overview"),
        ("web", "Web traffic"),
        ("growth", "Growth"),
        ("newsletters", "Newsletters"),
        ("content", "Content & engagement"),
        ("retention", "Returning visitors"),
        ("campaigns", "Campaign returns"),
        ("performance", "Performance"),
    ];
    for (slug, label) in &tabs {
        body.push_str(&format!(
            "<a class=\"seg{}\" href=\"?tab={slug}&amp;days={days}\">{label}</a>",
            if *slug == tab { " on" } else { "" }
        ));
    }
    body.push_str("</nav>");
    if !state.settings.get_bool("web_analytics") {
        body.push_str("<p role=\"status\">Web analytics is switched off. Existing reports remain available. <a href=\"/ghost/settings/analytics/\">Manage analytics</a></p>");
    }
    if tab == "growth" {
        let range =
            AnalyticsRange::from_query(&HashMap::from([("range".into(), days.to_string())]));
        body.push_str(&analytics_growth_tab(state, &range).await);
    } else {
        let view = match tab {
            "newsletters" | "content" | "retention" | "campaigns" | "performance" => tab,
            _ => "traffic",
        };
        match connector::report_view(state,&actor.user.id,role,days,None,view).await {
            Ok(report) => {
                if view == "traffic" {
                    body.push_str(&traffic_summary(&report));
                    body.push_str(&render_report(&report,days));
                    if tab == "overview" { body.push_str(&cms_snapshot(state).await); }
                } else if view != "newsletters" {
                    body.push_str(&render_extended(&report,view));
                    if view=="campaigns" && actor.role==Role::Owner {body.push_str(&cost_form());}
                } else if let Ok(Some(c)) = connector::connection(state).await {
                    body.push_str(&newsletter_editions(state,&c,&report).await);
                    let range = AnalyticsRange::from_query(&HashMap::from([("range".into(),days.to_string())]));
                    body.push_str(&analytics_newsletters_tab(state,&range).await);
                }
            },
            Err(e) => body.push_str(&format!("<p class=\"banner\" role=\"alert\">{}</p><p><a href=\"/ghost/analytics/click/\">Check connection</a> or refresh to retry.</p>",esc(&e.message()))),
        }
    }
    private_page(
        page_for_role(
            actor.role,
            "Analytics",
            "analytics",
            Some(&actor.user),
            &body,
            true,
        )
        .into_response(),
    )
}
fn private_page(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    response
}
fn traffic_summary(report: &Value) -> String {
    let mut html = String::from("<div class=\"tiles\">");
    for (key, label) in [
        ("visitors", "Visitors"),
        ("pageviews", "Views"),
        ("sessions", "Sessions"),
    ] {
        let current = report["comparison"]["current"][key].as_u64().unwrap_or(0);
        let previous = report["comparison"]["previous"][key].as_u64();
        html.push_str(&analytics::tile(
            label,
            &current.to_string(),
            analytics::Change {
                now: current as i64,
                before: previous.map(|n| n as i64),
            },
            "previous period",
        ));
    }
    html.push_str(&format!("<div class=\"tile\"><div class=\"label\">Active now</div><div class=\"value\">{}</div></div></div>",esc(&report["overview"]["realtime"].to_string())));
    let daily: Vec<DayViews> = report["overview"]["timeseries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(DayViews {
                day: r["label"].as_str()?.into(),
                views: r["count"].as_i64()?,
            })
        })
        .collect();
    html.push_str(&format!(
        "<div class=\"section\">Pageviews per day</div><div class=\"card padded\">{}</div>",
        views_chart(&daily)
    ));
    html
}
#[allow(clippy::too_many_arguments)]
pub async fn post_analytics(
    state: &AppState,
    role: Role,
    user: &ghost_core::models::User,
    id: &str,
    title: &str,
    paths: &[String],
    free: i64,
    paid: i64,
    email: &str,
) -> Response {
    let staff_role = if role == Role::Owner {
        "Owner"
    } else {
        "Administrator"
    };
    let mut body=format!("<header class=\"page-head\"><div><h1>{}</h1><p class=\"sub\">Post analytics · last 30 days</p></div><a class=\"btn tonal\" href=\"/ghost/posts/{}/\">Edit post</a></header>",esc(title),esc(id));
    if role < Role::Administrator {
        body.push_str(&format!("<p>Traffic reports require Administrator or Owner access.</p><section class=\"card padded\"><h2>Attributed growth · all time</h2><p>Free signups: {free} · Paid conversions: {paid}</p></section>{email}"));
        return private_page(
            page_for_role(role, "Post analytics", "posts", Some(user), &body, true).into_response(),
        );
    }
    let mut traffic_ready = false;
    match connector::report_view(
        state,
        &user.id,
        staff_role,
        30,
        paths.first().map(String::as_str),
        "post",
    )
    .await
    {
        Ok(r) => {
            traffic_ready = true;
            body.push_str("<section class=\"card padded\"><h2>Web traffic</h2><table class=\"breakdown\"><thead><tr><th>Metric</th><th>Total</th></tr></thead><tbody>");
            for (key, label) in [
                ("pageviews", "Views"),
                ("visitors", "Visitors"),
                ("sessions", "Sessions"),
            ] {
                body.push_str(&format!(
                    "<tr><th>{label}</th><td>{}</td></tr>",
                    esc(&r["native"]["page"][key].to_string())
                ));
            }
            body.push_str("</tbody></table><p class=\"muted\">Traffic comes from Vuuut Click. Slash and non-slash URLs share one report.</p></section>");
        }
        Err(e) => body.push_str(&format!("<p role=\"alert\">{}</p>", esc(&e.message()))),
    }
    let today = chrono::Utc::now().date_naive();
    let from = (today - chrono::Duration::days(29)).to_string();
    let to = today.to_string();
    if traffic_ready {
        for (metric, title) in [
            ("daily", "Pageviews per day"),
            ("sources", "Sources"),
            ("countries", "Locations"),
            ("campaigns", "Campaigns"),
        ] {
            match connector::cms_stats(
                state,
                &user.id,
                staff_role,
                metric,
                &from,
                &to,
                paths.first().map(String::as_str),
            )
            .await
            {
                Ok(report) => {
                    body.push_str(&format!("<section class=\"card padded\"><h2>{title}</h2>"));
                    let rows = report["stats"].as_array().cloned().unwrap_or_default();
                    if metric == "daily" {
                        let by_day: HashMap<&str, i64> = rows
                            .iter()
                            .filter_map(|r| Some((r["label"].as_str()?, r["views"].as_i64()?)))
                            .collect();
                        let daily = (0..30)
                            .rev()
                            .map(|n| {
                                let day = (today - chrono::Duration::days(n)).to_string();
                                DayViews {
                                    views: by_day.get(day.as_str()).copied().unwrap_or(0),
                                    day,
                                }
                            })
                            .collect::<Vec<_>>();
                        body.push_str(&views_chart(&daily));
                    } else if rows.is_empty() {
                        body.push_str("<p>No traffic in this period.</p>");
                    } else {
                        body.push_str("<div style=\"overflow-x:auto\"><table class=\"breakdown\"><thead><tr><th>Source or value</th><th>Views</th><th>Visitors</th></tr></thead><tbody>");
                        for row in rows {
                            let label = row["label"].as_str().unwrap_or("");
                            let label = if label.is_empty() {
                                "Direct / untagged"
                            } else if metric == "countries" {
                                analytics::country_name(label).unwrap_or("Unknown")
                            } else {
                                label
                            };
                            body.push_str(&format!(
                                "<tr><th>{}</th><td>{}</td><td>{}</td></tr>",
                                esc(label),
                                esc(&row["views"].to_string()),
                                esc(&row["visitors"].to_string())
                            ));
                        }
                        body.push_str("</tbody></table></div>");
                    }
                    body.push_str("</section>");
                }
                Err(e) => {
                    body.push_str(&format!(
                        "<p role=\"alert\">{title}: {}</p>",
                        esc(&e.message())
                    ));
                    break;
                }
            }
        }
    }
    body.push_str(&format!("<section class=\"card padded\"><h2>Attributed growth · all time</h2><p>Free signups: {free} · Paid conversions: {paid}</p></section>{email}"));
    private_page(
        page_for_role(role, "Post analytics", "posts", Some(user), &body, true).into_response(),
    )
}
