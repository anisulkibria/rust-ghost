//! Discounts: codes a buyer types at the cart or checkout.
//!
//! Administrator only — a code is money. Whether a code applies is decided
//! by `vuuut_core::ecom::discount_applies` on every checkout; nothing here
//! is trusted on the storefront.

use super::*;

const DISCOUNTS: &str = "/ghost/ecom/discounts/";

fn status(d: &repo::Discount, now: &str) -> (&'static str, &'static str) {
    if d.active == 0 {
        ("draft", "Off")
    } else if d.ends_at.as_deref().is_some_and(|e| e <= now) {
        ("draft", "Expired")
    } else if d.starts_at.as_deref().is_some_and(|x| x > now) {
        ("scheduled", "Scheduled")
    } else if d.usage_limit > 0 && d.used_count >= d.usage_limit {
        ("draft", "Used up")
    } else {
        ("published", "Active")
    }
}

pub(crate) async fn discounts_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let now = vuuut_core::dates::now_db();
    let list = repo::discounts(&state.db).await;
    let editing = q.get("edit").filter(|i| valid_id(i)).and_then(|i| list.iter().find(|d| &d.id == i));
    let rows: String = list
        .iter()
        .map(|d| {
            let (chip, label) = status(d, &now);
            let kind = rules::DiscountKind::from_key(&d.kind);
            format!(
                r#"<tr><td><b class="ec-code">{code}</b></td><td>{what}</td><td><span class="chip {chip}">{label}</span></td><td>{used}</td><td>{when}</td>
<td class="ec-row-tools"><a class="icon-btn" href="?edit={id}" title="Edit">{edit}</a><form method="post" action="{DISCOUNTS}{id}/delete/" data-confirm="Delete code|{code} will stop working at once."><button class="icon-btn" type="submit" title="Delete">{del}</button></form></td></tr>"#,
                code = esc(&d.code),
                what = esc(&format!("{}{}", rules::describe_discount(kind, d.value, &s.currency), if d.min_subtotal_cents > 0 { format!(" · orders over {}", rules::format_money(d.min_subtotal_cents, &s.currency)) } else { String::new() })),
                used = if d.usage_limit > 0 { format!("{} / {}", d.used_count, d.usage_limit) } else { d.used_count.to_string() },
                when = esc(&match (&d.starts_at, &d.ends_at) {
                    (_, Some(e)) => format!("Until {}", e.get(..10).unwrap_or(e)),
                    (Some(st), None) => format!("From {}", st.get(..10).unwrap_or(st)),
                    _ => "No end date".into(),
                }),
                id = esc(&d.id),
                edit = svg("edit"),
                del = svg("delete"),
            )
        })
        .collect();
    let g = |f: fn(&repo::Discount) -> String| editing.map(f).unwrap_or_default();
    let kind = editing.map(|d| rules::DiscountKind::from_key(&d.kind)).unwrap_or(rules::DiscountKind::Percent);
    let kinds: String = rules::DiscountKind::ALL.iter().map(|k| format!(r#"<option value="{}"{}>{}</option>"#, k.key(), if *k == kind { " selected" } else { "" }, k.label())).collect();
    let value = editing.map(|d| if kind == rules::DiscountKind::Fixed { rules::money_input(d.value, &s.currency) } else { d.value.to_string() }).unwrap_or_default();
    let action = match editing {
        Some(d) => format!("{DISCOUNTS}{}/", esc(&d.id)),
        None => DISCOUNTS.to_string(),
    };
    let form = format!(
        r#"<form method="post" action="{action}" class="card padded ec-card">
<h2 class="ec-card-title"><span class="ic tint-pink">{pct}</span>{h}</h2>
<div class="ec-row"><label class="field" style="flex:1"><input name="code" value="{code}" placeholder=" " maxlength="40" required id="ec-code"><span>Code</span></label><button class="btn text small" type="button" id="ec-gen-code">Generate</button></div>
<div class="ec-grid2"><label class="field fixed"><select name="kind">{kinds}</select><span>Type</span>{SELECT_CARET}</label>
<label class="field"><input name="value" value="{value}" placeholder=" " inputmode="decimal"><span>Value (% or amount)</span></label></div>
<label class="field"><input name="min_subtotal" value="{min}" placeholder=" " inputmode="decimal"><span>Minimum order subtotal (optional)</span></label>
<div class="ec-grid2"><label class="field"><input name="usage_limit" value="{limit}" placeholder=" " type="number" min="0"><span>Total uses allowed (0 = unlimited)</span></label>
<label class="ec-check"><input type="checkbox" name="once" value="1"{once}> One use per customer</label></div>
<div class="ec-grid2"><label class="field fixed"><input type="datetime-local" name="starts_at" value="{starts}"><span>Starts (optional, UTC)</span></label>
<label class="field fixed"><input type="datetime-local" name="ends_at" value="{ends}"><span>Ends (optional, UTC)</span></label></div>
<label class="ec-check"><input type="checkbox" name="active" value="1"{active}> Active</label>
<div class="ec-actions">{cancel}<button class="btn" type="submit">{check} {save}</button></div></form>"#,
        pct = svg("percent"),
        h = if editing.is_some() { "Edit discount" } else { "Create discount code" },
        code = esc(&g(|d| d.code.clone())),
        min = esc(&editing.map(|d| rules::money_input(d.min_subtotal_cents, &s.currency)).unwrap_or_default()),
        limit = editing.map(|d| d.usage_limit).unwrap_or(0),
        once = if editing.is_some_and(|d| d.once_per_customer != 0) { " checked" } else { "" },
        starts = esc(&editing.and_then(|d| d.starts_at.clone()).map(|x| x.get(..16).unwrap_or(&x).replace(' ', "T")).unwrap_or_default()),
        ends = esc(&editing.and_then(|d| d.ends_at.clone()).map(|x| x.get(..16).unwrap_or(&x).replace(' ', "T")).unwrap_or_default()),
        active = if editing.map(|d| d.active != 0).unwrap_or(true) { " checked" } else { "" },
        cancel = if editing.is_some() { format!(r#"<a class="btn text" href="{DISCOUNTS}">Cancel</a>"#) } else { String::new() },
        check = svg("check"),
        save = if editing.is_some() { "Save" } else { "Create" },
    );
    let body = format!(
        r#"<div class="ec-split"><div><div class="card ec-table-card rise" style="--i:2">{table}</div></div><div class="rise" style="--i:3">{form}</div></div><script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        table = if list.is_empty() {
            format!(r#"<div class="ec-empty">{}<p><b>No discount codes yet</b></p><p class="muted">Create a code for a sale, a newsletter, or a customer who deserves one.</p></div>"#, svg("percent"))
        } else {
            format!(r#"<table class="ec-table ec-list"><thead><tr><th>Code</th><th>Discount</th><th>Status</th><th>Used</th><th>Dates</th><th></th></tr></thead><tbody>{rows}</tbody></table>"#)
        },
    );
    ecom_page(&actor, "Discounts", "discounts", &q, "", Some(BASE), &body)
}

fn read_discount(f: &HashMap<String, String>, s: &Settings) -> Result<repo::DiscountInput, String> {
    let get = |k: &str| f.get(k).cloned().unwrap_or_default();
    let code = rules::normalise_code(&get("code"));
    if code.len() < 3 {
        return Err("A code needs at least three letters or numbers.".into());
    }
    let kind = rules::DiscountKind::from_key(&get("kind"));
    let value = match kind {
        rules::DiscountKind::Percent => match get("value").trim().trim_end_matches('%').parse::<i64>() {
            Ok(v) if (1..=100).contains(&v) => v,
            _ => return Err("A percentage is a whole number from 1 to 100.".into()),
        },
        rules::DiscountKind::Fixed => match rules::parse_money(&get("value"), &s.currency) {
            Some(v) if v > 0 => v,
            _ => return Err("Enter the amount to take off.".into()),
        },
        rules::DiscountKind::FreeShipping => 0,
    };
    let when = |k: &str| {
        let v = get(k);
        let v = v.trim();
        (v.len() >= 16 && v.contains('T')).then(|| format!("{}:00", v[..16].replace('T', " ")))
    };
    let (starts_at, ends_at) = (when("starts_at"), when("ends_at"));
    if let (Some(a), Some(b)) = (&starts_at, &ends_at) {
        if b <= a {
            return Err("The end has to be after the start.".into());
        }
    }
    Ok(repo::DiscountInput {
        code,
        kind: kind.key().into(),
        value,
        min_subtotal_cents: rules::money_or_zero(&get("min_subtotal"), &s.currency),
        usage_limit: get("usage_limit").trim().parse::<i64>().unwrap_or(0).clamp(0, 1_000_000),
        once_per_customer: f.contains_key("once"),
        active: f.contains_key("active"),
        starts_at,
        ends_at,
    })
}

pub(crate) async fn discount_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let s = store_settings(&state);
    match read_discount(&f, &s) {
        Err(e) => flash_to(DISCOUNTS, "error", &e),
        Ok(d) => match repo::save_discount(&state.db, None, &d).await {
            Ok(_) => flash_to(DISCOUNTS, "ok", &format!("{} created.", d.code)),
            Err(e) => flash_to(DISCOUNTS, "error", &e.to_string()),
        },
    }
}

pub(crate) async fn discount_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !valid_id(&id) || repo::find_discount(&state.db, &id).await.is_none() {
        return flash_to(DISCOUNTS, "error", "That discount no longer exists.");
    }
    let s = store_settings(&state);
    match read_discount(&f, &s) {
        Err(e) => flash_to(&format!("{DISCOUNTS}?edit={id}"), "error", &e),
        Ok(d) => match repo::save_discount(&state.db, Some(&id), &d).await {
            Ok(_) => flash_to(DISCOUNTS, "ok", &format!("{} saved.", d.code)),
            Err(e) => flash_to(&format!("{DISCOUNTS}?edit={id}"), "error", &e.to_string()),
        },
    }
}

pub(crate) async fn discount_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if valid_id(&id) {
        let _ = repo::delete_discount(&state.db, &id).await;
    }
    flash_to(DISCOUNTS, "ok", "Discount deleted.")
}
