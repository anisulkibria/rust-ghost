//! Locations, campaigns and period-over-period comparison for Analytics.
//!
//! Ghost's analytics screen is backed by Tinybird, which answers these as
//! pre-aggregated pipes. There is no Tinybird here: the same questions are
//! asked of the `page_views` table the front end writes, which already
//! carries the dimensions they need — `country`, the five `utm_*` tags, and
//! `audience`. What was missing was the reporting layer, which is what this
//! module is.
//!
//! The map itself is a client-side enhancement over a server-rendered table:
//! the table is the readable, WCAG-clean twin and is always present, so no
//! figure is ever reachable only by hovering a shape.

/// ISO 3166-1 alpha-2 to English name.
///
/// Taken from the same Natural Earth v5.1.2 release the map paths come from,
/// so a country's name here and its shape on the map can never disagree.
/// Natural Earth is public domain.
const COUNTRY_NAMES: &[(&str, &str)] = &[
    ("AD", "Andorra"),
    ("AE", "United Arab Emirates"),
    ("AF", "Afghanistan"),
    ("AG", "Antigua and Barbuda"),
    ("AI", "Anguilla"),
    ("AL", "Albania"),
    ("AM", "Armenia"),
    ("AO", "Angola"),
    ("AQ", "Antarctica"),
    ("AR", "Argentina"),
    ("AS", "American Samoa"),
    ("AT", "Austria"),
    ("AU", "Australia"),
    ("AW", "Aruba"),
    ("AX", "Åland"),
    ("AZ", "Azerbaijan"),
    ("BA", "Bosnia and Herzegovina"),
    ("BB", "Barbados"),
    ("BD", "Bangladesh"),
    ("BE", "Belgium"),
    ("BF", "Burkina Faso"),
    ("BG", "Bulgaria"),
    ("BH", "Bahrain"),
    ("BI", "Burundi"),
    ("BJ", "Benin"),
    ("BL", "Saint Barthélemy"),
    ("BM", "Bermuda"),
    ("BN", "Brunei"),
    ("BO", "Bolivia"),
    ("BQ", "Bonaire, Sint Eustatius and Saba"),
    ("BR", "Brazil"),
    ("BS", "The Bahamas"),
    ("BT", "Bhutan"),
    ("BV", "Bouvet Island"),
    ("BW", "Botswana"),
    ("BY", "Belarus"),
    ("BZ", "Belize"),
    ("CA", "Canada"),
    ("CC", "Cocos (Keeling) Islands"),
    ("CD", "Democratic Republic of the Congo"),
    ("CF", "Central African Republic"),
    ("CG", "Republic of the Congo"),
    ("CH", "Switzerland"),
    ("CI", "Ivory Coast"),
    ("CK", "Cook Islands"),
    ("CL", "Chile"),
    ("CM", "Cameroon"),
    ("CN", "People's Republic of China"),
    ("CO", "Colombia"),
    ("CR", "Costa Rica"),
    ("CU", "Cuba"),
    ("CV", "Cape Verde"),
    ("CW", "Curaçao"),
    ("CX", "Christmas Island"),
    ("CY", "Cyprus"),
    ("CZ", "Czech Republic"),
    ("DE", "Germany"),
    ("DJ", "Djibouti"),
    ("DK", "Denmark"),
    ("DM", "Dominica"),
    ("DO", "Dominican Republic"),
    ("DZ", "Algeria"),
    ("EC", "Ecuador"),
    ("EE", "Estonia"),
    ("EG", "Egypt"),
    ("EH", "Western Sahara"),
    ("ER", "Eritrea"),
    ("ES", "Spain"),
    ("ET", "Ethiopia"),
    ("FI", "Finland"),
    ("FJ", "Fiji"),
    ("FK", "Falkland Islands"),
    ("FM", "Federated States of Micronesia"),
    ("FO", "Faroe Islands"),
    ("FR", "France"),
    ("GA", "Gabon"),
    ("GB", "United Kingdom"),
    ("GD", "Grenada"),
    ("GE", "Georgia"),
    ("GF", "French Guiana"),
    ("GG", "Guernsey"),
    ("GH", "Ghana"),
    ("GI", "Gibraltar"),
    ("GL", "Greenland"),
    ("GM", "The Gambia"),
    ("GN", "Guinea"),
    ("GP", "Guadeloupe"),
    ("GQ", "Equatorial Guinea"),
    ("GR", "Greece"),
    ("GS", "South Georgia and the South Sandwich Islands"),
    ("GT", "Guatemala"),
    ("GU", "Guam"),
    ("GW", "Guinea-Bissau"),
    ("GY", "Guyana"),
    ("HK", "Hong Kong"),
    ("HM", "Heard Island and McDonald Islands"),
    ("HN", "Honduras"),
    ("HR", "Croatia"),
    ("HT", "Haiti"),
    ("HU", "Hungary"),
    ("ID", "Indonesia"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("IM", "Isle of Man"),
    ("IN", "India"),
    ("IO", "British Indian Ocean Territory"),
    ("IQ", "Iraq"),
    ("IR", "Iran"),
    ("IS", "Iceland"),
    ("IT", "Italy"),
    ("JE", "Jersey"),
    ("JM", "Jamaica"),
    ("JO", "Jordan"),
    ("JP", "Japan"),
    ("KE", "Kenya"),
    ("KG", "Kyrgyzstan"),
    ("KH", "Cambodia"),
    ("KI", "Kiribati"),
    ("KM", "Comoros"),
    ("KN", "Saint Kitts and Nevis"),
    ("KP", "North Korea"),
    ("KR", "South Korea"),
    ("KW", "Kuwait"),
    ("KY", "Cayman Islands"),
    ("KZ", "Kazakhstan"),
    ("LA", "Laos"),
    ("LB", "Lebanon"),
    ("LC", "Saint Lucia"),
    ("LI", "Liechtenstein"),
    ("LK", "Sri Lanka"),
    ("LR", "Liberia"),
    ("LS", "Lesotho"),
    ("LT", "Lithuania"),
    ("LU", "Luxembourg"),
    ("LV", "Latvia"),
    ("LY", "Libya"),
    ("MA", "Morocco"),
    ("MC", "Monaco"),
    ("MD", "Moldova"),
    ("ME", "Montenegro"),
    ("MF", "Saint Martin"),
    ("MG", "Madagascar"),
    ("MH", "Marshall Islands"),
    ("MK", "North Macedonia"),
    ("ML", "Mali"),
    ("MM", "Myanmar"),
    ("MN", "Mongolia"),
    ("MO", "Macau"),
    ("MP", "Northern Mariana Islands"),
    ("MQ", "Martinique"),
    ("MR", "Mauritania"),
    ("MS", "Montserrat"),
    ("MT", "Malta"),
    ("MU", "Mauritius"),
    ("MV", "Maldives"),
    ("MW", "Malawi"),
    ("MX", "Mexico"),
    ("MY", "Malaysia"),
    ("MZ", "Mozambique"),
    ("NA", "Namibia"),
    ("NC", "New Caledonia"),
    ("NE", "Niger"),
    ("NF", "Norfolk Island"),
    ("NG", "Nigeria"),
    ("NI", "Nicaragua"),
    ("NL", "Netherlands"),
    ("NO", "Norway"),
    ("NP", "Nepal"),
    ("NR", "Nauru"),
    ("NU", "Niue"),
    ("NZ", "New Zealand"),
    ("OM", "Oman"),
    ("PA", "Panama"),
    ("PE", "Peru"),
    ("PF", "French Polynesia"),
    ("PG", "Papua New Guinea"),
    ("PH", "Philippines"),
    ("PK", "Pakistan"),
    ("PL", "Poland"),
    ("PM", "Saint Pierre and Miquelon"),
    ("PN", "Pitcairn Islands"),
    ("PR", "Puerto Rico"),
    ("PS", "Palestine"),
    ("PT", "Portugal"),
    ("PW", "Palau"),
    ("PY", "Paraguay"),
    ("QA", "Qatar"),
    ("RE", "Reunion"),
    ("RO", "Romania"),
    ("RS", "Serbia"),
    ("RU", "Russia"),
    ("RW", "Rwanda"),
    ("SA", "Saudi Arabia"),
    ("SB", "Solomon Islands"),
    ("SC", "Seychelles"),
    ("SD", "Sudan"),
    ("SE", "Sweden"),
    ("SG", "Singapore"),
    ("SH", "Saint Helena"),
    ("SI", "Slovenia"),
    ("SJ", "Svalbard and Jan Mayen"),
    ("SK", "Slovakia"),
    ("SL", "Sierra Leone"),
    ("SM", "San Marino"),
    ("SN", "Senegal"),
    ("SO", "Somalia"),
    ("SR", "Suriname"),
    ("SS", "South Sudan"),
    ("ST", "São Tomé and Príncipe"),
    ("SV", "El Salvador"),
    ("SX", "Sint Maarten"),
    ("SY", "Syria"),
    ("SZ", "Eswatini"),
    ("TC", "Turks and Caicos Islands"),
    ("TD", "Chad"),
    ("TF", "French Southern and Antarctic Lands"),
    ("TG", "Togo"),
    ("TH", "Thailand"),
    ("TJ", "Tajikistan"),
    ("TK", "Tokelau"),
    ("TL", "East Timor"),
    ("TM", "Turkmenistan"),
    ("TN", "Tunisia"),
    ("TO", "Tonga"),
    ("TR", "Turkey"),
    ("TT", "Trinidad and Tobago"),
    ("TV", "Tuvalu"),
    ("TW", "Taiwan"),
    ("TZ", "Tanzania"),
    ("UA", "Ukraine"),
    ("UG", "Uganda"),
    ("UM", "United States Minor Outlying Islands"),
    ("US", "United States of America"),
    ("UY", "Uruguay"),
    ("UZ", "Uzbekistan"),
    ("VA", "Vatican City"),
    ("VC", "Saint Vincent and the Grenadines"),
    ("VE", "Venezuela"),
    ("VG", "British Virgin Islands"),
    ("VI", "United States Virgin Islands"),
    ("VN", "Vietnam"),
    ("VU", "Vanuatu"),
    ("WF", "Wallis and Futuna"),
    ("WS", "Samoa"),
    ("XK", "Kosovo"),
    ("YE", "Yemen"),
    ("YT", "Mayotte"),
    ("ZA", "South Africa"),
    ("ZM", "Zambia"),
    ("ZW", "Zimbabwe"),
];

/// The English name for a country code, or `None` for one we do not know.
///
/// The table is sorted, so this is a binary search rather than a walk — it is
/// called once per row of every locations table.
pub fn country_name(code: &str) -> Option<&'static str> {
    let upper = code.to_ascii_uppercase();
    COUNTRY_NAMES
        .binary_search_by(|(c, _)| (*c).cmp(upper.as_str()))
        .ok()
        .map(|i| COUNTRY_NAMES[i].1)
}

/// The flag emoji for a country code.
///
/// Computed rather than tabulated: a flag is just its two letters as regional
/// indicator symbols, so every present and future code works without a table
/// to maintain. Anything that is not two ASCII letters gets a globe instead,
/// because half a flag renders as stray letters.
pub fn country_flag(code: &str) -> String {
    let bytes = code.as_bytes();
    if bytes.len() != 2 || !bytes.iter().all(|b| b.is_ascii_alphabetic()) {
        return "\u{1f310}".to_string();
    }
    bytes
        .iter()
        .filter_map(|b| char::from_u32(0x1F1E6 + (b.to_ascii_uppercase() - b'A') as u32))
        .collect()
}

// ---------------------------------------------------------------------------
// reporting windows
// ---------------------------------------------------------------------------

/// A reporting window as the half-open range `[from, to)`, in the
/// `YYYY-MM-DD HH:MM:SS` UTC form the `created_at` columns hold.
///
/// Half-open on purpose: the current window's `from` is the previous window's
/// `to`, so a view sitting exactly on the boundary is counted once rather than
/// in both — which is how a comparison quietly gains a visitor.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub from: String,
    pub to: String,
}

impl Window {
    /// The window `days` back from `now`, and the window of equal length
    /// immediately before it.
    ///
    /// `days` of `None` is all time, which has nothing before it: comparing
    /// against an empty window would report every figure as an infinite rise.
    /// `Some(0)` is "today", whose comparison is yesterday — a whole day
    /// against a partial one would always read as a collapse, so today is
    /// measured midnight-to-midnight against the same slice of yesterday.
    pub fn pair(now: chrono::DateTime<chrono::Utc>, days: Option<i64>) -> (Window, Option<Window>) {
        let stamp = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%d %H:%M:%S").to_string();
        let end = now + chrono::Duration::seconds(1);
        match days {
            None => (
                Window { from: "0000-01-01 00:00:00".into(), to: stamp(end) },
                None,
            ),
            Some(0) => {
                let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
                // Yesterday is cut at the same clock time, so a comparison made
                // at 09:00 is nine hours against nine hours.
                let yesterday = midnight - chrono::Duration::days(1);
                (
                    Window { from: stamp(midnight), to: stamp(end) },
                    Some(Window {
                        from: stamp(yesterday),
                        to: stamp(end - chrono::Duration::days(1)),
                    }),
                )
            }
            Some(n) => {
                let from = now - chrono::Duration::days(n);
                (
                    Window { from: stamp(from), to: stamp(end) },
                    Some(Window {
                        from: stamp(now - chrono::Duration::days(n * 2)),
                        to: stamp(from),
                    }),
                )
            }
        }
    }
}

/// A figure and what it was over the window before, so it can be shown as a
/// change rather than a bare number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    pub now: i64,
    pub before: Option<i64>,
}

impl Change {
    /// Percentage change, rounded. `None` when there is nothing to compare
    /// against, or when the previous window was zero — "up from nothing" has
    /// no percentage, and reporting one (or 100%) would be a fiction.
    pub fn percent(&self) -> Option<i64> {
        match self.before {
            Some(before) if before > 0 => {
                Some((((self.now - before) as f64 / before as f64) * 100.0).round() as i64)
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// queries
// ---------------------------------------------------------------------------

/// `AND audience = '…'`, or nothing for all traffic.
///
/// Inlined rather than bound because the value is chosen from a fixed list in
/// the caller, never taken from the query string.
fn audience_clause(audience: &str) -> &'static str {
    match audience {
        "public" => " AND audience = 'public'",
        "free" => " AND audience = 'free'",
        "paid" => " AND audience = 'paid'",
        _ => "",
    }
}

/// Unique visitors and total views inside a window.
pub async fn traffic(db: &sqlx::SqlitePool, w: &Window, audience: &str) -> (i64, i64) {
    let sql = format!(
        "SELECT COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE created_at >= ? AND created_at < ?{}",
        audience_clause(audience)
    );
    sqlx::query_as(&sql).bind(&w.from).bind(&w.to).fetch_one(db).await.unwrap_or((0, 0))
}

/// Visitors and views per country, most-visited first.
///
/// Rows with no country are kept and reported as unknown rather than dropped:
/// on a site behind no geolocating proxy that is *every* row, and a locations
/// table that silently showed nothing would look broken rather than
/// unconfigured.
pub async fn locations(
    db: &sqlx::SqlitePool,
    w: &Window,
    audience: &str,
    limit: i64,
) -> Vec<(Option<String>, i64, i64)> {
    let sql = format!(
        "SELECT NULLIF(country, ''), COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE created_at >= ? AND created_at < ?{} \
         GROUP BY 1 ORDER BY 2 DESC, 3 DESC LIMIT ?",
        audience_clause(audience)
    );
    sqlx::query_as(&sql)
        .bind(&w.from)
        .bind(&w.to)
        .bind(limit)
        .fetch_all(db)
        .await
        .unwrap_or_default()
}

/// The campaign dimensions, as (query slug, column, label, what it answers).
///
/// Ghost reports all five UTM tags; the collector here already writes all
/// five, so all five are reported rather than the three that fit a smaller
/// screen.
pub const CAMPAIGN_DIMENSIONS: &[(&str, &str, &str, &str)] = &[
    ("source", "utm_source", "Source", "Where the link was placed"),
    ("medium", "utm_medium", "Medium", "What kind of placement it was"),
    ("campaign", "utm_campaign", "Campaign", "Which push it belonged to"),
    ("term", "utm_term", "Term", "The keyword an ad was bought on"),
    ("content", "utm_content", "Content", "Which variant of the link"),
];

/// Visitors and views for one campaign dimension.
///
/// `dimension` is a slug from `CAMPAIGN_DIMENSIONS`; the column name is looked
/// up from that table rather than taken from the caller, so the identifier
/// interpolated into the SQL can only ever be one of the five.
pub async fn campaigns(
    db: &sqlx::SqlitePool,
    w: &Window,
    dimension: &str,
    audience: &str,
    limit: i64,
) -> Vec<(String, i64, i64)> {
    let Some((_, column, _, _)) = CAMPAIGN_DIMENSIONS.iter().find(|(slug, ..)| *slug == dimension)
    else {
        return Vec::new();
    };
    let sql = format!(
        "SELECT {column}, COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE created_at >= ? AND created_at < ? AND {column} IS NOT NULL AND {column} <> ''{} \
         GROUP BY 1 ORDER BY 2 DESC, 3 DESC LIMIT ?",
        audience_clause(audience)
    );
    sqlx::query_as(&sql)
        .bind(&w.from)
        .bind(&w.to)
        .bind(limit)
        .fetch_all(db)
        .await
        .unwrap_or_default()
}

/// `path IN (?, ?, …)` for however many paths a post is reachable at.
///
/// A post can be served at more than one path — its own, and whatever a
/// `routes.yaml` collection prefixes it with — so every per-post figure has to
/// be asked across the set rather than a single string.
fn path_placeholders(paths: &[String]) -> String {
    std::iter::repeat_n("?", paths.len()).collect::<Vec<_>>().join(",")
}

/// Visitors and views per country, for one post's paths.
pub async fn locations_for_paths(
    db: &sqlx::SqlitePool,
    paths: &[String],
    limit: i64,
) -> Vec<(Option<String>, i64, i64)> {
    if paths.is_empty() {
        return Vec::new();
    }
    let sql = format!(
        "SELECT NULLIF(country, ''), COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE path IN ({}) GROUP BY 1 ORDER BY 2 DESC, 3 DESC LIMIT ?",
        path_placeholders(paths)
    );
    let mut query = sqlx::query_as(&sql);
    for path in paths {
        query = query.bind(path);
    }
    query.bind(limit).fetch_all(db).await.unwrap_or_default()
}

/// Campaign tags on the links that brought readers to one post.
pub async fn campaigns_for_paths(
    db: &sqlx::SqlitePool,
    paths: &[String],
    dimension: &str,
    limit: i64,
) -> Vec<(String, i64, i64)> {
    let Some((_, column, _, _)) = CAMPAIGN_DIMENSIONS.iter().find(|(slug, ..)| *slug == dimension)
    else {
        return Vec::new();
    };
    if paths.is_empty() {
        return Vec::new();
    }
    let sql = format!(
        "SELECT {column}, COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE path IN ({}) AND {column} IS NOT NULL AND {column} <> '' \
         GROUP BY 1 ORDER BY 2 DESC, 3 DESC LIMIT ?",
        path_placeholders(paths)
    );
    let mut query = sqlx::query_as(&sql);
    for path in paths {
        query = query.bind(path);
    }
    query.bind(limit).fetch_all(db).await.unwrap_or_default()
}

/// Unique visitors per day for one post's paths, as `(day, visitors)`.
///
/// Days with no visits are absent rather than zero; the caller fills the gaps,
/// because only it knows how wide the chart is meant to be.
pub async fn daily_for_paths(
    db: &sqlx::SqlitePool,
    paths: &[String],
    since: &str,
) -> Vec<(String, i64)> {
    if paths.is_empty() {
        return Vec::new();
    }
    let sql = format!(
        "SELECT substr(created_at, 1, 10) AS day, COUNT(DISTINCT viewer) FROM page_views \
         WHERE path IN ({}) AND created_at >= ? GROUP BY day ORDER BY day",
        path_placeholders(paths)
    );
    let mut query = sqlx::query_as(&sql);
    for path in paths {
        query = query.bind(path);
    }
    query.bind(since).fetch_all(db).await.unwrap_or_default()
}

/// How many views in the window carry any campaign tag at all.
///
/// The Campaigns section leads with this: without it, five empty tables look
/// like a broken report rather than a site nobody has tagged a link for.
pub async fn tagged_views(db: &sqlx::SqlitePool, w: &Window, audience: &str) -> i64 {
    let sql = format!(
        "SELECT COUNT(*) FROM page_views WHERE created_at >= ? AND created_at < ? \
         AND (utm_source IS NOT NULL AND utm_source <> '' \
           OR utm_medium IS NOT NULL AND utm_medium <> '' \
           OR utm_campaign IS NOT NULL AND utm_campaign <> '' \
           OR utm_term IS NOT NULL AND utm_term <> '' \
           OR utm_content IS NOT NULL AND utm_content <> ''){}",
        audience_clause(audience)
    );
    sqlx::query_scalar(&sql).bind(&w.from).bind(&w.to).fetch_one(db).await.unwrap_or(0)
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

use crate::{compact, esc};

/// A stat tile carrying its change against the previous window.
///
/// The direction is said three ways — an arrow glyph, a signed percentage and
/// the words naming the window — so the colour is never what carries it. That
/// matters for more than colour blindness: "12%" with no arrow is ambiguous,
/// and a green that means "up" on visitors would have to mean "good" on a
/// metric where up is bad.
pub fn tile(label: &str, value: &str, change: Change, previous_label: &str) -> String {
    let delta = match change.percent() {
        // Two different silences. All time has no window before it at all;
        // a window that was simply empty does have one, and saying "nothing
        // to compare" there would hide a rise from zero that is real news.
        None if change.before == Some(0) => format!(
            r#"<span class="delta-flat">None in the {previous_label}</span>"#,
            previous_label = esc(previous_label),
        ),
        None => format!(
            r#"<span class="delta-flat">No {previous_label} to compare</span>"#,
            previous_label = esc(previous_label),
        ),
        Some(0) => format!(
            r#"<span class="delta-flat">Level with {previous_label}</span>"#,
            previous_label = esc(previous_label),
        ),
        Some(pct) => format!(
            r#"<span class="delta-move {dir}"><span class="arrow" aria-hidden="true">{glyph}</span>{sign}{pct}%</span> <span class="delta-vs">vs {previous_label}</span>"#,
            dir = if pct > 0 { "up" } else { "down" },
            glyph = if pct > 0 { "\u{2191}" } else { "\u{2193}" },
            sign = if pct > 0 { "+" } else { "" },
            pct = pct,
            previous_label = esc(previous_label),
        ),
    };
    format!(
        r#"<div class="tile"><div class="label">{label}</div><div class="value">{value}</div><div class="delta">{delta}</div></div>"#,
        label = esc(label),
        value = esc(value),
        delta = delta,
    )
}

/// One row of a breakdown table: a name, a proportion bar, and its figures.
///
/// The bar is the magnitude channel and the numbers are the exact value, so
/// the row is readable at a glance and precise on inspection. `share` is the
/// row's value over the largest in the table, never over the total — a bar
/// that filled the row only when a country held 100% of traffic would be flat
/// on every real report.
fn breakdown_row(lead: &str, name: &str, detail: &str, share: f64, figures: &str) -> String {
    format!(
        r#"<tr><th scope="row"><span class="bd-lead" aria-hidden="true">{lead}</span><span class="bd-name"><span class="bd-title">{name}</span>{detail}</span><span class="bd-bar" style="--share:{share:.4}"></span></th>{figures}</tr>"#,
        lead = lead,
        name = esc(name),
        detail = if detail.is_empty() {
            String::new()
        } else {
            format!(r#"<span class="bd-detail">{}</span>"#, esc(detail))
        },
        share = share.clamp(0.0, 1.0),
        figures = figures,
    )
}

/// Two right-aligned figure cells.
fn figures(visitors: i64, views: i64) -> String {
    format!(
        r#"<td>{}</td><td>{}</td>"#,
        compact(visitors),
        compact(views)
    )
}

/// A breakdown table, or a line saying why it is empty.
fn breakdown_table(caption: &str, rows: String, empty: &str) -> String {
    if rows.is_empty() {
        return format!(r#"<p class="bd-empty">{}</p>"#, esc(empty));
    }
    format!(
        r#"<table class="breakdown"><caption class="sr-only">{caption}</caption>
<thead><tr><th scope="col">{caption}</th><th scope="col">Visitors</th><th scope="col">Views</th></tr></thead>
<tbody>{rows}</tbody></table>"#,
        caption = esc(caption),
        rows = rows,
    )
}

/// How many colour classes the choropleth uses.
///
/// Five, because past about seven the adjacent classes stop being tellable
/// apart and the map becomes decoration. The steps are one hue, light to dark,
/// mixed from the admin's own accent so the ramp follows the theme — and so
/// it inverts correctly in dark mode, where "near zero" recedes toward a dark
/// surface instead of a light one.
pub const MAP_BINS: usize = 5;

/// Where the colour classes are cut, as a share of the busiest country.
///
/// Cut on share rather than on equal counts because traffic is almost always
/// one dominant country and a long tail: equal-count bins would paint half the
/// world the darkest step. Shipped to the map script in a data attribute so
/// there is one definition rather than two that can drift.
pub const MAP_EDGES: [f64; MAP_BINS] = [0.08, 0.22, 0.42, 0.68, 1.0];

/// Which bin a value falls in, 0-based.
fn bin_of(value: i64, max: i64) -> usize {
    if max <= 0 || value <= 0 {
        return 0;
    }
    let share = value as f64 / max as f64;
    MAP_EDGES.iter().position(|edge| share <= *edge).unwrap_or(MAP_BINS - 1)
}

/// The Locations card: a choropleth and the table it is drawn from.
///
/// The table is server-rendered and always present. The map is painted over
/// it by `analytics-map.js` from a cached asset, so the page is complete and
/// readable before — or without — that script: nothing here is reachable only
/// by hovering a shape.
pub fn locations_card(
    rows: &[(Option<String>, i64, i64)],
    assets: &str,
    unknown_only: bool,
) -> String {
    // Two scales, deliberately. The proportion bar measures a row against the
    // biggest row in the table, "unknown" included, because that is the table
    // it sits in. The colour class measures a country against the busiest
    // *placeable* country, because that is the scale the map is drawn on —
    // and the map has no "unknown" shape to anchor to. Using the table's max
    // for both would paint every country a step too light the moment unknown
    // led the table, and the swatch beside a row would stop matching the
    // shape it names.
    let max = rows.iter().map(|(_, visitors, _)| *visitors).max().unwrap_or(0);
    let scale_max = rows
        .iter()
        .filter(|(code, _, _)| code.is_some())
        .map(|(_, visitors, _)| *visitors)
        .max()
        .unwrap_or(0);

    // Only countries we can place go to the map; "unknown" is a real row in
    // the table but has nowhere to sit on a map.
    let values: Vec<String> = rows
        .iter()
        .filter_map(|(code, visitors, _)| {
            let code = code.as_deref()?;
            Some(format!(r#""{}":{}"#, esc(&code.to_ascii_uppercase()), visitors))
        })
        .collect();

    let table_rows: String = rows
        .iter()
        .map(|(code, visitors, views)| {
            let (lead, name) = match code.as_deref() {
                Some(code) => (
                    country_flag(code),
                    country_name(code).unwrap_or(code).to_string(),
                ),
                None => ("\u{1f310}".to_string(), "Unknown".to_string()),
            };
            let detail = match code.as_deref() {
                Some(_) => String::new(),
                None => "No country reported for these visits".to_string(),
            };
            // The same colour class the country is painted on the map, so a
            // row and a shape can be matched up in either direction. An
            // unplaceable row gets the no-data swatch, which is what the map
            // would have shown it as.
            let swatch = match code.as_deref() {
                Some(_) => format!(r#"<span class="sw b{}"></span>"#, bin_of(*visitors, scale_max)),
                None => r#"<span class="sw none"></span>"#.to_string(),
            };
            breakdown_row(
                &format!("{swatch}{lead}"),
                &name,
                &detail,
                if max > 0 { *visitors as f64 / max as f64 } else { 0.0 },
                &figures(*visitors, *views),
            )
        })
        .collect();

    // The scale legend. A continuous colour encoding is unreadable without
    // one, and "no data" is a swatch of its own rather than a sixth step of
    // the ramp — a country nobody visited is a different statement from one
    // that is merely quiet.
    let legend: String = (0..MAP_BINS)
        .map(|i| format!(r#"<span class="sw b{i}"></span>"#))
        .collect();

    let map = if values.is_empty() {
        // Nothing placeable: the map would be an empty world, which says less
        // than a sentence does.
        format!(
            r#"<p class="bd-empty">No visits in this range carry a country. Localhost and private network visits cannot be located. New public visits use proxy country headers or a country database in <code>content/data/</code> (or <code>hostSettings.analytics.geoipDatabase</code>).{}</p>"#,
            if unknown_only { " Earlier visits without a country remain unknown." } else { "" },
        )
    } else {
        format!(
            r#"<figure class="worldmap" data-world-map data-src="{assets}/world-map.json" data-values='{{{values}}}' data-edges="{edges}">
<div class="map-frame"><p class="map-pending" role="status">Loading the map…</p></div>
<figcaption class="map-legend"><span class="lg-label">Fewer visitors</span>{legend}<span class="lg-label">More</span>
<span class="lg-gap"></span><span class="sw none"></span><span class="lg-label">No visitors</span></figcaption>
</figure>"#,
            assets = assets,
            values = values.join(","),
            // One definition of where the classes are cut, sent to the script
            // rather than repeated in it.
            edges = MAP_EDGES
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(","),
            legend = legend,
        )
    };

    format!(
        r#"<div class="card padded rise locations" style="--i:4">{map}
<div class="bd-scroll">{table}</div><p class="bd-hint"><a href="https://db-ip.com">DB-IP country database</a></p></div>"#,
        map = map,
        table = breakdown_table(
            "Country",
            table_rows,
            "No visits recorded in this range yet.",
        ),
    )
}

/// The Campaigns card: one table per UTM dimension, behind a segmented picker.
///
/// `rows` is what the chosen dimension returned; `tagged` is how many views in
/// the window carry any tag at all, which is what tells "nobody has tagged a
/// link" apart from "this particular tag is unused".
pub fn campaigns_card(
    dimension: &str,
    rows: &[(String, i64, i64)],
    tagged: i64,
    segs: String,
) -> String {
    let (_, _, label, hint) = CAMPAIGN_DIMENSIONS
        .iter()
        .find(|(slug, ..)| *slug == dimension)
        .copied()
        .unwrap_or(CAMPAIGN_DIMENSIONS[0]);
    let max = rows.iter().map(|(_, visitors, _)| *visitors).max().unwrap_or(0);
    let table_rows: String = rows
        .iter()
        .map(|(value, visitors, views)| {
            breakdown_row(
                "",
                value,
                "",
                if max > 0 { *visitors as f64 / max as f64 } else { 0.0 },
                &figures(*visitors, *views),
            )
        })
        .collect();

    let empty = if tagged == 0 {
        "No visit in this range arrived on a tagged link. Add ?utm_source=… to the links you share and they will be told apart here."
    } else {
        "No visit in this range carried this tag, though other campaign tags were used."
    };

    format!(
        r#"<div class="seg-bar">{segs}</div>
<div class="card padded rise" style="--i:6"><p class="bd-hint">{hint}</p>
<div class="bd-scroll">{table}</div></div>"#,
        segs = segs,
        hint = esc(hint),
        table = breakdown_table(label, table_rows, empty),
    )
}

/// A generic breakdown card, for sections that are only a table.
pub fn breakdown_card(caption: &str, rows: &[(String, String, i64, i64)], empty: &str, order: u8) -> String {
    let max = rows.iter().map(|(_, _, visitors, _)| *visitors).max().unwrap_or(0);
    let body: String = rows
        .iter()
        .map(|(lead, name, visitors, views)| {
            breakdown_row(
                lead,
                name,
                "",
                if max > 0 { *visitors as f64 / max as f64 } else { 0.0 },
                &figures(*visitors, *views),
            )
        })
        .collect();
    format!(
        r#"<div class="card padded rise" style="--i:{order}"><div class="bd-scroll">{}</div></div>"#,
        breakdown_table(caption, body, empty),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flag is computed from the code, so a code the name table has never
    /// heard of still gets a flag — and a code that is not a code does not get
    /// two stray letters.
    #[test]
    fn flags_are_derived_from_the_country_code() {
        assert_eq!(country_flag("GB"), "\u{1f1ec}\u{1f1e7}");
        assert_eq!(country_flag("us"), "\u{1f1fa}\u{1f1f8}");
        // Not a country code: a globe, never half a flag.
        assert_eq!(country_flag("XXX"), "\u{1f310}");
        assert_eq!(country_flag("1"), "\u{1f310}");
        assert_eq!(country_flag(""), "\u{1f310}");
    }

    /// The name table is binary-searched, which is only correct while it is
    /// sorted — a hand-edited insertion in the wrong place would silently
    /// start returning the wrong country.
    #[test]
    fn the_country_table_is_sorted_and_searchable() {
        let mut sorted = COUNTRY_NAMES.to_vec();
        sorted.sort_by_key(|(code, _)| *code);
        assert_eq!(sorted, COUNTRY_NAMES.to_vec(), "COUNTRY_NAMES must stay sorted");
        assert_eq!(country_name("GB"), Some("United Kingdom"));
        assert_eq!(country_name("gb"), Some("United Kingdom"));
        assert_eq!(country_name("ZW"), Some("Zimbabwe"));
        assert_eq!(country_name("AD"), Some("Andorra"));
        assert_eq!(country_name("ZZ"), None);
    }

    /// The previous window has to be the same length and must not overlap the
    /// current one, or the comparison counts a boundary visit twice.
    #[test]
    fn the_previous_window_abuts_the_current_one() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T09:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let (current, previous) = Window::pair(now, Some(30));
        let previous = previous.expect("30 days has a previous window");
        assert_eq!(current.from, "2026-09-30 09:00:00");
        assert_eq!(previous.to, current.from, "windows must abut, not overlap");
        assert_eq!(previous.from, "2026-08-31 09:00:00");

        // Today runs from midnight, and is compared against the same hours of
        // yesterday rather than against a whole day.
        let (today, yesterday) = Window::pair(now, Some(0));
        let yesterday = yesterday.expect("today has a previous window");
        assert_eq!(today.from, "2026-10-30 00:00:00");
        assert_eq!(yesterday.from, "2026-10-29 00:00:00");
        assert_eq!(yesterday.to, "2026-10-29 09:00:01");

        // All time has nothing before it.
        assert_eq!(Window::pair(now, None).1, None);
    }

    /// A percentage against nothing is not a percentage.
    #[test]
    fn a_change_from_zero_has_no_percentage() {
        assert_eq!(Change { now: 120, before: Some(100) }.percent(), Some(20));
        assert_eq!(Change { now: 80, before: Some(100) }.percent(), Some(-20));
        assert_eq!(Change { now: 100, before: Some(100) }.percent(), Some(0));
        // Growth from an empty window is not "infinite" or "100%".
        assert_eq!(Change { now: 50, before: Some(0) }.percent(), None);
        assert_eq!(Change { now: 50, before: None }.percent(), None);
    }

    /// Direction is stated as an arrow and words, never by colour alone.
    #[test]
    fn a_tile_names_its_direction_without_relying_on_colour() {
        let up = tile("Visitors", "1.2K", Change { now: 120, before: Some(100) }, "previous 30 days");
        assert!(up.contains("\u{2191}"), "{up}");
        assert!(up.contains("+20%"), "{up}");
        assert!(up.contains("vs previous 30 days"), "{up}");

        let down = tile("Visitors", "80", Change { now: 80, before: Some(100) }, "previous 30 days");
        assert!(down.contains("\u{2193}") && down.contains("-20%"), "{down}");

        // Nothing to compare against says so rather than showing a 0%.
        let flat = tile("Visitors", "50", Change { now: 50, before: None }, "previous 30 days");
        assert!(flat.contains("No previous 30 days to compare"), "{flat}");

        // A window that existed and was empty is a different statement from
        // no window at all, and reads differently.
        let from_zero = tile("Visitors", "50", Change { now: 50, before: Some(0) }, "previous 30 days");
        assert!(from_zero.contains("None in the previous 30 days"), "{from_zero}");
        assert!(!from_zero.contains("to compare"), "{from_zero}");
    }

    /// The busiest country anchors the ramp, and zero is never painted as the
    /// darkest step.
    #[test]
    fn map_bins_are_cut_on_the_share_of_the_busiest_country() {
        assert_eq!(bin_of(1000, 1000), MAP_BINS - 1);
        assert_eq!(bin_of(0, 1000), 0);
        assert_eq!(bin_of(10, 1000), 0, "1% of the leader is the lightest step");
        assert_eq!(bin_of(900, 1000), MAP_BINS - 1);
        // No traffic anywhere must not divide by zero.
        assert_eq!(bin_of(0, 0), 0);
        for value in [1, 50, 300, 700, 1000] {
            assert!(bin_of(value, 1000) < MAP_BINS);
        }
    }

    /// Only placeable countries reach the map; "unknown" stays a table row.
    #[test]
    fn the_map_is_given_countries_and_the_table_keeps_the_unknowns() {
        let rows = vec![
            (Some("US".to_string()), 90i64, 140i64),
            (None, 20, 30),
        ];
        let html = locations_card(&rows, "/ghost/assets/x", false);
        assert!(html.contains(r#"data-values='{"US":90}'"#), "{html}");
        // The unknown row is still readable in the table.
        assert!(html.contains("Unknown"), "{html}");
        assert!(html.contains("United States of America"), "{html}");

        // Nothing placeable at all: a sentence explaining why, not an empty world.
        let only_unknown = locations_card(&[(None, 5, 9)], "/ghost/assets/x", true);
        assert!(!only_unknown.contains("data-world-map"), "{only_unknown}");
        assert!(only_unknown.contains("geoipDatabase"), "{only_unknown}");
    }

    /// The swatch beside a row and the shape it names have to be the same
    /// step. They are drawn by different code — Rust here, the map script
    /// there — and the script scales against the countries it was handed, so
    /// the table has to scale against those too. Anchoring the swatches on
    /// the table's own biggest row instead silently paints every country a
    /// step too light whenever "unknown" leads the table, which on a site
    /// behind no geolocating proxy is always.
    #[test]
    fn the_colour_scale_ignores_rows_the_map_cannot_show() {
        // Unknown leads the table; the busiest placeable country is the US.
        let rows = vec![
            (None, 557i64, 1200i64),
            (Some("US".to_string()), 227, 260),
            (Some("GB".to_string()), 109, 120),
        ];
        let html = locations_card(&rows, "/ghost/assets/x", false);
        // The US is the top of the scale, so it wears the darkest step — the
        // same one the map paints it.
        let us_row = html
            .split("<tr>")
            .find(|row| row.contains("United States"))
            .expect("a US row");
        assert!(
            us_row.contains(&format!("sw b{}", MAP_BINS - 1)),
            "the busiest placeable country should wear the darkest step: {us_row}"
        );
        // Unknown is not on the scale at all.
        let unknown_row = html.split("<tr>").find(|row| row.contains("Unknown")).unwrap();
        assert!(unknown_row.contains("sw none"), "{unknown_row}");
        // …and the bar still measures against the whole table, so unknown's
        // is the full-width one.
        assert!(unknown_row.contains("--share:1.0000"), "{unknown_row}");
    }

    /// The column interpolated into the campaign SQL can only ever come from
    /// the fixed table, never from the caller's string.
    #[test]
    fn campaign_dimensions_are_a_closed_set() {
        for (slug, column, ..) in CAMPAIGN_DIMENSIONS {
            assert!(column.starts_with("utm_"), "{column}");
            assert!(slug.chars().all(|c| c.is_ascii_lowercase()), "{slug}");
        }
        assert_eq!(CAMPAIGN_DIMENSIONS.len(), 5);
    }
}
