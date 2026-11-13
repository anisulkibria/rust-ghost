//! The Modules hub: one entry point for the publication's administrative tools.

use super::*;

// ---------------------------------------------------------------------------
// Modules: a single entry point for the publication's administrative tools.
pub(crate) const MODULES: &[(&str, &str, &str, &str, &str)] = &[
    ("Analytics", "/ghost/analytics/", "chart", "green", "Track visitors, content performance, and member growth."),
    ("Comments", "/ghost/comments/", "comment", "purple", "Review discussions and manage comments across your posts."),
    ("Automations", "/ghost/automations/", "flow", "blue", "Manage automated workflows and their activity."),
    ("Forms", "/ghost/forms/", "form", "amber", "Build contact forms and read the responses visitors send."),
    ("Social web", "/ghost/social/", "share", "teal", "Read, publish notes and connect with the fediverse."),
];

pub(crate) async fn modules_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let cards: String = MODULES.iter().map(|(name, href, icon, tint, description)| {
        format!(
            r#"<a class="module-card" href="{href}">
<span class="ic tint-{tint}">{icon}</span>
<div><h2>{name}</h2><p>{description}</p></div>
<span class="module-open">Open {name} {arrow}</span>
</a>"#,
            href = href, tint = tint, icon = svg(icon), name = esc(name),
            description = esc(description), arrow = svg("chevron"),
        )
    }).collect();
    page_for_role(actor.role, "Modules", "modules", Some(&actor.user), &format!(
        r#"<header class="page-head rise"><div><h1>Modules</h1>
<p class="sub">Insights, conversations, and workflows for your publication.</p></div></header>
<div class="module-grid rise" style="--i:1">{cards}</div>"#,
    ), true).into_response()
}

// analytics (Ghost's Dashboard: page views recorded by ghost-front's
// web_analytics middleware, members, and content counts)
// ---------------------------------------------------------------------------

/// One day of the views chart.
pub(crate) struct DayViews {
    pub(crate) day: String,
    pub(crate) views: i64,
}

/// Compact number for tile values and ticks (1,284 → 1.3K).
pub(crate) fn compact(n: i64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Round up to a clean axis maximum from a ladder of chart-friendly tops
/// (1 → 4, 6 → 8, 70 → 80, 1_234 → 2_000) so the half/whole ticks stay even.
pub(crate) fn clean_max(max: i64) -> i64 {
    let mut candidates = vec![4, 8, 10];
    let mut base = 10;
    while base <= 1_000_000_000 {
        for m in [2, 4, 6, 8, 10] {
            candidates.push(m * base);
        }
        base *= 10;
    }
    candidates.into_iter().find(|c| *c >= max).unwrap_or(max.max(1))
}

/// SVG column chart of daily views — thin columns with 4px rounded caps
/// (square at the baseline), hairline grid at clean ticks, the max value
/// labeled, and a native tooltip on every day. Single series, so the card
/// title carries the legend and no legend box is drawn.
pub(crate) fn views_chart(days: &[DayViews]) -> String {
    if days.is_empty() {
        return r#"<p class="muted" style="margin:8px 0 0">No views recorded yet.</p>"#.into();
    }
    let (w, h, pad_l, pad_b, pad_t) = (728usize, 180usize, 34usize, 22usize, 18usize);
    let plot_w = w - pad_l - 8;
    let plot_h = h - pad_b - pad_t;
    let slot = plot_w as f64 / days.len() as f64;
    let bar_w = slot.min(24.0) * 0.75; // ≤24px thick, air in the band
    let axis = clean_max(days.iter().map(|d| d.views).max().unwrap_or(1));
    let y = |v: i64| pad_t as f64 + plot_h as f64 * (1.0 - v as f64 / axis as f64);
    let max_i = days
        .iter()
        .enumerate()
        .max_by_key(|(_, d)| d.views)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let mut grid = String::new();
    for frac in [0.0, 0.5, 1.0] {
        let value = (axis as f64 * frac).round() as i64;
        let gy = y(value);
        grid.push_str(&format!(
            r#"<line x1="{pad_l}" x2="{}" y1="{gy:.1}" y2="{gy:.1}" stroke="var(--outline-variant)" stroke-width="1"/>"#,
            w - 8,
            gy = gy,
        ));
        grid.push_str(&format!(
            r#"<text x="{}" y="{:.1}" text-anchor="end" dominant-baseline="middle">{}</text>"#,
            pad_l - 8,
            gy + 3.0,
            compact(value),
        ));
    }

    let mut bars = String::new();
    let mut labels = String::new();
    for (i, d) in days.iter().enumerate() {
        let cx = pad_l as f64 + slot * (i as f64 + 0.5);
        // x labels: first, last, and the midpoint; every day on a 7-day range
        let show_label = days.len() <= 7 || i == 0 || i == days.len() - 1 || i == days.len() / 2;
        if show_label {
            labels.push_str(&format!(
                r#"<text x="{cx:.0}" y="{}" text-anchor="middle">{}</text>"#,
                h - 6,
                esc(&d.day[5..]), // MM-DD
                cx = cx,
            ));
        }
        if d.views == 0 {
            continue;
        }
        let (x0, y_top) = (cx - bar_w / 2.0, y(d.views));
        let height = (pad_t + plot_h) as f64 - y_top;
        let r = 4.0_f64.min(height); // a cap never taller than the bar
        let (x1, cap_l, cap_r) = (x0 + bar_w, x0 + r, x0 + bar_w - r);
        let (yt, top, base) = (y_top + r, y_top, (pad_t + plot_h) as f64);
        let path = format!(
            "M{x0:.1},{base} L{x0:.1},{yt:.1} Q{x0:.1},{top:.1} {cap_l:.1},{top:.1} L{cap_r:.1},{top:.1} Q{x1:.1},{top:.1} {x1:.1},{yt:.1} L{x1:.1},{base} Z",
        );
        let value_label = if i == max_i {
            format!(
                r#"<text x="{cx:.0}" y="{:.1}" text-anchor="middle" font-weight="600">{}</text>"#,
                y_top - 6.0,
                compact(d.views),
            )
        } else {
            String::new()
        };
        bars.push_str(&format!(
            r#"<g><title>{}: {} views</title><path class="bar" d="{path}" fill="var(--primary)"/>{value_label}</g>"#,
            esc(&d.day),
            d.views,
            path = path,
            value_label = value_label,
        ));
    }

    format!(
        r#"<svg class="chart" viewBox="0 0 {w} {h}" role="img" aria-label="Views per day">{grid}{bars}{labels}</svg>"#,
        w = w,
        h = h,
        grid = grid,
        bars = bars,
        labels = labels,
    )
}
