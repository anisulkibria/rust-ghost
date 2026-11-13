//! The members list, one member's screen, imports, exports and bulk actions.

use super::*;

// ---------------------------------------------------------------------------
// members (Ghost's Members list: everyone subscribed to the site)
// ---------------------------------------------------------------------------

use ghost_core::ids::uuid_v4;

/// The members list's status tabs. Ghost offers exactly these buckets, and
/// so does the posts list's filter bar — same component, same behaviour.
pub(crate) const MEMBER_FILTERS: &[(&str, &str, Option<&str>)] = &[
    ("all", "All", None),
    ("free", "Free", Some("free")),
    ("paid", "Paid", Some("paid")),
    ("comped", "Comped", Some("comped")),
];

/// Build the NQL a members view compiles to: a status bucket and/or a search
/// term over name, email and note.
///
/// The search term is an NQL *value*, so every character the tokenizer would
/// read as syntax is stripped rather than escaped — a member called
/// `O'Brien (VIP)` must be findable by typing exactly that, without their
/// name breaking out of the quoted value.
pub(crate) fn members_nql(filter: &str, search: &str, nql: &str) -> Option<String> {
    let clean: String = search
        .chars()
        .filter(|c| !"[](),+~<>'\"\\".contains(*c))
        .take(60)
        .collect();
    let clean = clean.trim();
    let mut parts: Vec<String> = Vec::new();
    if matches!(filter, "free" | "paid" | "comped") {
        parts.push(format!("status:{filter}"));
    }
    // The toolbar's filter menu writes raw NQL. It is parenthesised before it
    // joins the tab and the search term, so an `or` inside it cannot swallow
    // them — `status:free+a,b` and `status:free+(a,b)` are different queries.
    let nql = nql.trim();
    if !nql.is_empty() {
        parts.push(format!("({nql})"));
    }
    if !clean.is_empty() {
        // `:~` is the comparator form — `key:~'value'` compiles to a
        // LIKE; the bare `~` the tokenizer also accepts still needs its
        // colon first, or the parse stops at the key.
        parts.push(format!("(name:~'{clean}',email:~'{clean}',note:~'{clean}')"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("+"))
    }
}

pub(crate) async fn members_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let mut banner = flash_banner(&q);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let status_rows: Vec<(String, i64)> = sqlx::query_as("SELECT status, COUNT(*) FROM members GROUP BY status")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let counts: HashMap<String, i64> = status_rows.into_iter().collect();

    let filter = q
        .get("filter")
        .map(|s| s.as_str())
        .filter(|s| MEMBER_FILTERS.iter().any(|(key, _, _)| key == s))
        .unwrap_or("all");
    let search = q.get("search").map(|s| s.trim()).unwrap_or("").to_string();
    // A filter that does not parse is the operator's typo, not a broken
    // page: keep the list and say so, rather than replacing the screen with
    // an error they cannot navigate out of.
    let nql_raw = q.get("nql").map(|s| s.trim()).unwrap_or("").to_string();
    let (nql, nql_error) = match ghost_nql::parse(&nql_raw) {
        _ if nql_raw.is_empty() => (String::new(), None),
        Ok(_) => (nql_raw.clone(), None),
        Err(e) => (String::new(), Some(e.message())),
    };
    if let Some(message) = nql_error {
        banner = format!(
            r#"<div class="banner error rise" style="--i:1">{warn}That filter did not parse, so it was ignored — {message}</div>"#,
            warn = svg("flag"),
            message = esc(&message),
        );
    }
    let page = q.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let (members, filtered_count) = match repos::members::browse(&state.db, members_nql(filter, &search, &nql).as_deref(), 100, page).await {
        Ok(result) => result,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.message()).into_response(),
    };

    // Labels: the page's rows need their own, the remove-label popover needs
    // every label there is. One query for both — a join table at site scale
    // is smaller than the member list it hangs off.
    let label_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT ml.member_id, l.name FROM members_labels ml JOIN labels l ON l.id = ml.label_id ORDER BY l.name",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut labels_by_member: HashMap<String, Vec<String>> = HashMap::new();
    for (member_id, name) in &label_rows {
        labels_by_member.entry(member_id.clone()).or_default().push(name.clone());
    }
    let all_labels: Vec<String> = {
        let mut names: Vec<String> = label_rows.into_iter().map(|(_, name)| name).collect();
        names.sort();
        names.dedup();
        names
    };

    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let per_page = 100;
    let total_pages = (filtered_count + per_page - 1) / per_page;

    let rows: String = members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let name = m.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&m.email);
            let initial = name.chars().next().unwrap_or('?').to_uppercase().to_string();
            let (chip, status_label) = match m.status.as_str() {
                "paid" => ("paid", "Paid"),
                "comped" => ("comped", "Comped"),
                _ => ("free", "Free"),
            };
            // Three pills fit the column; the rest fold into a "+N" that
            // carries the full set in its tooltip.
            let labels = labels_by_member.get(&m.id).cloned().unwrap_or_default();
            let pills: String = match labels.len() {
                0 => String::new(),
                n if n > 3 => format!(
                    r#"{shown}<span class="label-pill more" title="{all}">+{rest}</span>"#,
                    shown = labels[..3].iter().map(|l| pill(l)).collect::<String>(),
                    all = esc(&labels.join(", ")),
                    rest = n - 3,
                ),
                _ => labels.iter().map(|l| pill(l)).collect::<String>(),
            };
            format!(
                r#"<article class="member-row rise" style="--i:{delay}">
<label class="row-check"><input type="checkbox" form="batch-list" name="ids" value="{id}" aria-label="Select {name_attr}"><span class="box">{check}</span></label>
<a class="member-row-link" href="/ghost/members/{id}/">
<span class="member-face" aria-hidden="true">{initial}</span>
<span class="member-main"><span class="member-name"{note_attr}>{name}</span><span class="member-email">{email}</span></span>
<span class="member-labels">{pills}</span>
<span class="member-status"><span class="chip {chip}">{status_label}</span></span>
<span class="member-added" title="Joined {added_title}">{added}</span>
</a>
<div class="row-end">
<form class="post-row-action" method="post" action="/ghost/members/{id}/delete/" data-confirm="Delete member|&#8220;{name_attr}&#8221; will be permanently removed, along with their subscriptions and labels.">
<button class="icon-btn danger" title="Delete" aria-label="Delete {name_attr}">{del}</button></form>
</div>
</article>"#,
                delay = i.min(14),
                id = esc(&m.id),
                check = svg("check"),
                initial = esc(&initial),
                name = esc(name),
                name_attr = esc(name),
                note_attr = m
                    .note
                    .as_deref()
                    .filter(|n| !n.trim().is_empty())
                    .map(|n| format!(" title=\"{}\"", esc(n)))
                    .unwrap_or_default(),
                email = esc(&m.email),
                pills = pills,
                chip = chip,
                status_label = status_label,
                added = esc(&ghost_core::dates::short_date_in_zone(&m.created_at, &timezone)),
                added_title = esc(&ghost_core::dates::short_date_in_zone(&m.created_at, &timezone)),
                del = svg("delete"),
            )
        })
        .collect();

    // Query params that every link on this page must carry — the tabs, the
    // pager and the search form would each otherwise drop the others' state.
    let carry = |page_no: i64| -> String {
        let mut s = form_urlencoded::Serializer::new(String::new());
        if filter != "all" {
            s.append_pair("filter", filter);
        }
        if !search.is_empty() {
            s.append_pair("search", &search);
        }
        if page_no > 1 {
            s.append_pair("page", &page_no.to_string());
        }
        s.finish()
    };
    let seg_suffix = if search.is_empty() {
        String::new()
    } else {
        format!("&amp;search={}", esc(&search))
    };

    let pager = if total_pages > 1 || (filtered_count != count) {
        format!(
            r#"<div class="pager"><span>{matching}{page_of}</span>{prev}{next}</div>"#,
            matching = if filtered_count != count {
                format!("{} matching", filtered_count)
            } else {
                String::new()
            },
            page_of = if total_pages > 1 {
                format!(" · page {} of {}", page, total_pages.max(1))
            } else {
                String::new()
            },
            prev = if page > 1 {
                format!(r#"<a class="btn tonal small" href="?{}">Previous</a>"#, carry(page - 1))
            } else {
                String::new()
            },
            next = if page < total_pages {
                format!(r#"<a class="btn tonal small" href="?{}">Next</a>"#, carry(page + 1))
            } else {
                String::new()
            },
        )
    } else {
        String::new()
    };

    let list = if count == 0 {
        // Nothing exists yet: tabs, search and checkboxes would all be
        // furniture in an empty room. The create form is one click up in
        // the header.
        r#"<div class="card padded rise" style="--i:3;text-align:center;padding:48px 20px">
<p style="margin:0 0 4px;font-weight:600;font-size:17px">No members yet</p>
<p class="muted" style="margin:0 0 20px">Add the first one with <b>New member</b>, above.</p></div>"#
            .to_string()
    } else {
        let head = if members.is_empty() {
            // A view with nothing in it — say so, and offer the way back,
            // which the old page left as a bare empty card.
            r#"<div class="filter-empty on"><p>No members match this view.</p>
<a class="btn tonal small" href="/ghost/members/">Clear filter &amp; search</a></div>"#
                .to_string()
        } else {
            format!(
                r#"<div class="list-head">
<label class="row-check" title="Select all members on this page"><input type="checkbox" data-select-all form="batch-list" aria-label="Select all members on this page"><span class="box">{check}</span></label>
<span class="lh-spacer face" aria-hidden="true"></span>
<div class="lh-main"><span class="lh-label">Member</span></div>
<div class="lh-labels"><span class="lh-label">Labels</span></div>
<div class="lh-status"><span class="lh-label">Status</span></div>
<div class="lh-label" style="width:96px;text-align:right">Added</div>
</div>
{rows}"#,
                check = svg("check"),
                rows = rows,
            )
        };
        // The search is its own form; the filter menu holds another. Forms
        // cannot nest, so the toolbar is the row that holds them both rather
        // than being one of them.
        format!(
            r#"{seg}
<div class="list-tools rise" style="--i:0">
<form class="tool-form" method="get" action="/ghost/members/">
<input type="hidden" name="filter" value="{filter}">
<input type="hidden" name="nql" value="{nql_attr}">
<div class="search">{search_icon}<input name="search" type="search" value="{search_attr}" placeholder="Search by name, email or note" aria-label="Search members"></div>
</form>
{filter_menu}</div>
<div data-batch>
<div class="member-list rise">{head}</div>
{batch}
</div>
{pager}"#,
            seg = seg_filter_bar(MEMBER_FILTERS, "/ghost/members/", filter, &seg_suffix, &counts),
            filter = esc(filter),
            nql_attr = esc(&nql),
            search_icon = svg("search"),
            search_attr = esc(&search),
            filter_menu = filter_menu(
                "/ghost/members/",
                "members",
                &nql,
                &format!(
                    r#"<input type="hidden" name="filter" value="{filter}"><input type="hidden" name="search" value="{search_attr}">"#,
                    filter = esc(filter),
                    search_attr = esc(&search),
                ),
                "status:paid+label:vip · created_at:>'2026-01-01' · email:~'@acme.com'",
            ),
            head = head,
            batch = member_batch_bar(&all_labels),
            pager = pager,
        )
    };

    page_for_role(
        actor_role,
        "Members",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Members</h1><p class="sub">{count} {word}</p></div>
<div class="actions">
<details class="head-pop" data-menu>
<summary class="btn tonal">{upload} Import</summary>
<div class="pop">
<p style="margin:0 0 14px;font-weight:600">Import members</p>
<form method="post" action="/ghost/members/import/" enctype="multipart/form-data">
<input type="file" name="file" accept=".csv,text/csv" required class="filefield">
<button class="btn tonal" type="submit" style="margin-top:14px">Import CSV</button>
</form>
<p class="muted" style="margin:12px 0 0;font-size:12px">A CSV with at least an <code>email</code> column. Rows whose email already exists update that member instead of duplicating it.</p>
</div>
</details>
<a class="btn tonal" href="/ghost/members/export/{export_query}" title="{export_title}">{download} Export</a>
<a class="btn" href="/ghost/members/new/">{add} New member</a>
</div></header>
{banner}
{list}"#,
            upload = svg("upload"),
            download = svg("open"),
            add = svg("add"),
            count = count,
            word = if count == 1 { "member" } else { "members" },
            // Export follows the view on screen. The old page had a second
            // "Export filtered members" button bolted on beside this one,
            // which meant two buttons whose difference you had to know.
            export_query = match members_nql(filter, &search, &nql) {
                Some(view) => format!("?filter={}", url_value(&view)),
                None => String::new(),
            },
            export_title = if members_nql(filter, &search, &nql).is_some() {
                "Download the members matching the current view as CSV"
            } else {
                "Download every member as CSV"
            },
            banner = banner,
            list = list,
        ),
        true,
    )
    .into_response()
}

/// One label pill.
pub(crate) fn pill(name: &str) -> String {
    format!(r#"<span class="label-pill">{}</span>"#, esc(name))
}

/// The members batch bar: label on, label off, unsubscribe, delete. The
/// label pickers open out of the bar: "add" takes typed text (with the
/// site's labels as datalist suggestions, and unknown names are created),
/// "remove" offers only labels that exist, as chips.
///
/// Unsubscribe used to live in a script-built modal reached from a "Bulk
/// actions" button parked in the page header, beside a bare checkbox that
/// selected rows through a second, invisible selection. One bar, one
/// selection.
///
/// Every button except "Remove label" carries `formnovalidate`, for the same
/// reason the posts bar's do: the remove picker's `required` radios belong to
/// this one form, so with none picked the browser refuses every submit on the
/// bar — and refuses silently, the radio being inside a closed `<details>`
/// and therefore not focusable enough to complain about.
pub(crate) fn member_batch_bar(all_labels: &[String]) -> String {
    let options: String = all_labels
        .iter()
        .map(|l| format!("<option value=\"{}\">", esc(l)))
        .collect();
    let remove_chips: String = if all_labels.is_empty() {
        r#"<p class="muted" style="margin:4px 0 0;font-size:12.5px">No labels exist yet.</p>"#.to_string()
    } else {
        all_labels
            .iter()
            .map(|l| {
                format!(
                    r#"<label class="fchip"><input type="radio" name="label" value="{v}" required><span class="ck">{check}</span>{v}</label>"#,
                    v = esc(l),
                    check = svg("check"),
                )
            })
            .collect()
    };
    format!(
        r#"<form id="batch-list" class="batch" method="post" action="/ghost/members/bulk/" data-word="member">
<div class="batch-bar">
<span class="bb-count"><b data-count>0 members</b> selected</span>
<details class="batch-pop" data-menu>
<summary class="bb-btn">{tags} Add label</summary>
<div class="pop">
<label class="field"><input name="label" list="all-labels" placeholder=" "><span>Label</span></label>
<datalist id="all-labels">{options}</datalist>
<button class="btn" type="submit" formnovalidate name="action" value="add_label">{check} Add label</button>
<p>Existing labels are suggested; a new name is created.</p>
</div>
</details>
<details class="batch-pop" data-menu>
<summary class="bb-btn">{close} Remove label</summary>
<div class="pop">
<div class="chips" style="margin-bottom:12px">{remove_chips}</div>
<button class="btn tonal" type="submit" name="action" value="remove_label">{close} Remove label</button>
</div>
</details>
<button class="bb-btn" type="submit" formnovalidate name="action" value="unsubscribe"
 data-confirm="Unsubscribe selected members|They stay members and keep their access; they stop receiving every newsletter."
 data-confirm-one="Unsubscribe 1 member|They stay a member and keep their access; they stop receiving every newsletter."
 data-confirm-many="Unsubscribe {{n}} members|They stay members and keep their access; they stop receiving every newsletter.">{mail} Unsubscribe</button>
<button class="bb-btn danger" type="submit" formnovalidate name="action" value="delete"
 data-confirm="Delete selected members|The selected members will be permanently removed, along with their subscriptions and labels."
 data-confirm-one="Delete 1 member|This member will be permanently removed, along with their subscriptions and labels."
 data-confirm-many="Delete {{n}} members|The {{n}} selected members will be permanently removed, along with their subscriptions and labels.">{del} Delete</button>
</div>
</form>"#,
        tags = svg("tags"),
        check = svg("check"),
        close = svg("close"),
        mail = svg("mail"),
        del = svg("delete"),
        options = options,
        remove_chips = remove_chips,
    )
}

// --- members CSV: export and import, in Ghost's column order ---

/// Ghost's export columns, from `@tryghost/members-csv`. Keeping the order
/// and names identical means a file exported here imports into Ghost, and
/// vice versa.
pub(crate) const MEMBER_CSV_COLUMNS: &[&str] = &[
    "id", "email", "name", "note", "subscribed_to_emails", "complimentary_plan",
    "stripe_customer_id", "created_at", "deleted_at", "labels", "tiers",
];

/// Every member as CSV, or only those in `only` when the caller is exporting
/// a filtered view.
///
/// Separate from the download handler because the migration bundle puts the
/// same bytes inside a zip: one definition of what a member export *is*, so
/// the standalone file and the backup can never disagree.
pub(crate) async fn build_members_csv(state: &AppState, only: Option<&HashSet<String>>) -> Option<Vec<u8>> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String, Option<String>)>(
        "SELECT m.id, m.email, m.name, m.note, m.status, m.created_at, sc.customer_id \
         FROM members m \
         LEFT JOIN members_stripe_customers sc ON sc.member_id = m.id \
         ORDER BY m.created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(MEMBER_CSV_COLUMNS).ok()?;
    for (id, email, name, note, status, created_at, customer_id) in rows {
        if only.is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        let labels = member_labels(state, &id).await.join(",");
        let tiers = sqlx::query_scalar::<_, String>(
            "SELECT p.name FROM products p JOIN members_products mp ON mp.product_id = p.id WHERE mp.member_id = ?",
        )
        .bind(&id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
        .join(",");
        // Ghost's `subscribed_to_emails` is "is this member on any
        // newsletter", not a column of its own.
        let subscribed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members_newsletters WHERE member_id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);
        let record = [
            id,
            email,
            name.unwrap_or_default(),
            note.unwrap_or_default(),
            if subscribed > 0 { "true".into() } else { "false".into() },
            if status == "comped" { "true".into() } else { "false".into() },
            customer_id.unwrap_or_default(),
            created_at,
            String::new(), // deleted_at: rows are hard-deleted here
            labels,
            tiers,
        ];
        w.write_record(&record).ok()?;
    }
    w.into_inner().ok()
}

/// Today's date, for the dated filenames Ghost's exports carry.
pub(crate) fn export_date_stamp() -> String {
    ghost_core::dates::now_db().chars().take(10).collect()
}

pub(crate) fn csv_download(filename: &str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        body,
    )
        .into_response()
}

/// Every member as CSV, as a file download.
pub(crate) async fn members_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // `?filter=` is the view the list was showing when Export was clicked —
    // exporting the whole site from a filtered screen is the kind of surprise
    // you only notice after mailing the wrong people.
    let only = match q.get("filter").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => None,
        Some(view) => match repos::members::browse(&state.db, Some(view), -1, 1).await {
            Ok((rows, _)) => Some(rows.into_iter().map(|m| m.id).collect::<HashSet<String>>()),
            Err(_) => return members_flash("error", "That filter did not parse, so nothing was exported."),
        },
    };
    match build_members_csv(&state, only.as_ref()).await {
        Some(body) => csv_download(&format!("members.{}.csv", export_date_stamp()), body),
        None => members_flash("error", "Could not build the export."),
    }
}

/// Outcome of an import, reported back to the operator.
#[derive(Default)]
pub(crate) struct ImportTally {
    created: usize,
    updated: usize,
    skipped: usize,
}

/// Import members from a CSV.
///
/// Matching is by email, which is the member's identity: a row whose address
/// already exists updates that member rather than creating a duplicate.
/// Unknown columns are ignored, so a file exported from Ghost — which carries
/// more columns than this port uses — imports cleanly.
/// Where a stashed upload waits between the mapping screen and the import.
///
/// The browser cannot hold the file across two posts, and re-uploading it to
/// confirm a mapping would be worse, so it is parked on disk under a random
/// name and removed as soon as it has been read.
pub(crate) fn import_stash_path(state: &AppState, token: &str) -> Option<std::path::PathBuf> {
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let dir = state.config.content_path().join("data").join("imports");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(format!("{token}.csv")))
}

/// The member fields an uploaded column can be pointed at.
pub(crate) const IMPORT_FIELDS: &[(&str, &str)] = &[
    ("email", "Email"),
    ("name", "Name"),
    ("note", "Note"),
    ("subscribed_to_emails", "Subscribed to emails"),
    ("complimentary_plan", "Complimentary plan"),
    ("labels", "Labels"),
    ("stripe_customer_id", "Stripe customer ID"),
];

/// Step one: read the upload, park it, and ask which column is which.
///
/// Ghost guesses by name and lets you correct it, which is the whole point —
/// before this, a column called `Email Address` simply did not import.
pub(crate) async fn members_import(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let data = loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => return members_flash("error", "No CSV file was uploaded."),
            Err(e) => return members_flash("error", &upload_error_message(&e)),
        };
        if field.name() != Some("file") {
            continue;
        }
        match field.bytes().await {
            Ok(d) => break d.to_vec(),
            Err(e) => return members_flash("error", &upload_error_message(&e)),
        }
    };

    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(data.as_slice());
    let headers_row = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => return members_flash("error", &format!("That file is not valid CSV: {e}")),
    };
    let columns: Vec<String> = headers_row.iter().map(|h| h.trim().to_string()).collect();
    if columns.is_empty() {
        return members_flash("error", "That CSV has no header row.");
    }
    // A sample row makes the mapping checkable at a glance.
    let sample: Vec<String> = reader
        .records()
        .next()
        .and_then(|r| r.ok())
        .map(|r| r.iter().map(|v| v.trim().to_string()).collect())
        .unwrap_or_default();

    let token = ghost_core::ids::random_hex(16);
    let Some(path) = import_stash_path(&state, &token) else {
        return members_flash("error", "Could not stage the upload.");
    };
    if std::fs::write(&path, &data).is_err() {
        return members_flash("error", "Could not stage the upload.");
    }

    // Ghost's guess: an exact-ish name match, so a tidy CSV needs no edits.
    let guess = |field: &str| -> Option<usize> {
        let norm = |s: &str| s.trim().to_lowercase().replace([' ', '-'], "_");
        let target = norm(field);
        columns
            .iter()
            .position(|c| norm(c) == target)
            .or_else(|| match field {
                "email" => columns.iter().position(|c| norm(c).contains("email")),
                "subscribed_to_emails" => columns.iter().position(|c| norm(c) == "subscribed"),
                _ => None,
            })
    };

    let options = |selected: Option<usize>| -> String {
        let mut out = format!(
            r#"<option value="">{}</option>"#,
            if selected.is_none() { "— not imported —" } else { "— not imported —" }
        );
        for (i, c) in columns.iter().enumerate() {
            out.push_str(&format!(
                r#"<option value="{i}"{sel}>{name}</option>"#,
                i = i,
                sel = if selected == Some(i) { " selected" } else { "" },
                name = esc(c),
            ));
        }
        out
    };

    let field_rows: String = IMPORT_FIELDS
        .iter()
        .map(|(key, label)| {
            let picked = guess(key);
            format!(
                r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="map_{key}">{opts}</select><span>{label}{req}</span>{car}</label>
<p class="muted" style="margin:6px 0 0">{preview}</p></div></div>"#,
                ic = svg("person"),
                key = esc(key),
                opts = options(picked),
                label = esc(label),
                req = if *key == "email" { " · required" } else { "" },
                car = SELECT_CARET,
                preview = match picked.and_then(|i| sample.get(i)) {
                    Some(v) if !v.is_empty() => format!("First row: <code>{}</code>", esc(v)),
                    _ => String::new(),
                },
            )
        })
        .collect();

    // Custom fields are importable too, which is most of why mapping matters:
    // a column nobody anticipated can still land somewhere useful.
    let metafields = repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let metafield_rows: String = metafields
        .iter()
        .filter(|m| m.is_active() && m.parts().is_none())
        .map(|m| {
            let picked = columns
                .iter()
                .position(|c| c.trim().to_lowercase().replace(' ', "_") == m.key);
            format!(
                r#"<div class="row"><span class="ic tint-purple">{ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="mf_{key}">{opts}</select><span>{label}</span>{car}</label></div></div>"#,
                ic = svg("notes"),
                key = esc(&m.key),
                opts = options(picked),
                label = esc(&m.name),
                car = SELECT_CARET,
            )
        })
        .collect();

    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    page_for_role(actor_role,
        "Import members",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/members/" title="Back to members">{back}</a>
<div><h1>Import members</h1><p class="sub">{count} column{plural} found — check where each one goes</p></div>
<div class="actions"><button class="btn" type="submit" form="import-form">{check} Import</button></div></header>
<form method="post" action="/ghost/members/import/run/" id="import-form">
<input type="hidden" name="token" value="{token}">
<div class="section">Member fields</div>
<div class="card rows rise" style="--i:1">{field_rows}</div>
{metafield_section}
<p class="muted" style="margin:14px 0 0 16px">Rows without a valid email are skipped. An email that already exists is updated rather than duplicated.</p>
</form>"#,
            back = svg("chevron"),
            check = svg("check"),
            count = columns.len(),
            plural = if columns.len() == 1 { "" } else { "s" },
            token = esc(&token),
            field_rows = field_rows,
            metafield_section = if metafield_rows.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<div class="section">Custom fields</div><div class="card rows rise" style="--i:2">{metafield_rows}</div>"#
                )
            },
        ),
        true,
    )
    .into_response()
}

/// Step two: apply the mapping and import.
pub(crate) async fn members_import_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let token = form.get("token").cloned().unwrap_or_default();
    let Some(path) = import_stash_path(&state, &token) else {
        return members_flash("error", "That upload has expired — please upload the file again.");
    };
    let Ok(data) = std::fs::read(&path) else {
        return members_flash("error", "That upload has expired — please upload the file again.");
    };
    // Read once: the file is gone whether the import succeeds or not, so a
    // stash cannot be replayed or left lying around.
    let _ = std::fs::remove_file(&path);

    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(data.as_slice());
    if reader.headers().is_err() {
        return members_flash("error", "That file is not valid CSV.");
    }
    let mapped = |field: &str| -> Option<usize> {
        form.get(&format!("map_{field}")).and_then(|v| v.parse::<usize>().ok())
    };
    let (c_email, c_name, c_note, c_sub, c_comp, c_labels, c_stripe) = (
        mapped("email"),
        mapped("name"),
        mapped("note"),
        mapped("subscribed_to_emails"),
        mapped("complimentary_plan"),
        mapped("labels"),
        mapped("stripe_customer_id"),
    );
    let Some(c_email) = c_email else {
        return members_flash("error", "Choose which column holds the email address.");
    };

    // Custom-field columns, by key.
    let metafield_cols: Vec<(String, usize)> = form
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("mf_")
                .and_then(|key| v.parse::<usize>().ok().map(|i| (key.to_string(), i)))
        })
        .collect();

    // Default newsletters, for rows that ask to be subscribed.
    let default_newsletters: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM newsletters WHERE status = 'active' AND subscribe_on_signup = 1 ORDER BY sort_order",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let truthy = |v: &str| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes");
    let mut tally = ImportTally::default();
    let now = ghost_core::dates::now_db();

    for record in reader.records().flatten() {
        let get = |i: Option<usize>| i.and_then(|i| record.get(i)).unwrap_or("").trim().to_string();
        let email = get(Some(c_email)).to_lowercase();
        if email.is_empty() || !email.contains('@') {
            tally.skipped += 1;
            continue;
        }
        let name = get(c_name);
        let note = get(c_note);
        let comped = truthy(&get(c_comp));
        let subscribe = c_sub.map(|_| truthy(&get(c_sub))).unwrap_or(true);
        let status = if comped { "comped" } else { "free" };

        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM members WHERE lower(email) = ?")
            .bind(&email)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

        let member_id = match existing {
            Some(id) => {
                let res = sqlx::query(
                    "UPDATE members SET name = COALESCE(NULLIF(?, ''), name), \
                     note = COALESCE(NULLIF(?, ''), note), status = ?, updated_at = ? WHERE id = ?",
                )
                .bind(&name).bind(&note).bind(status).bind(&now).bind(&id)
                .execute(&state.db).await;
                if res.is_err() {
                    tally.skipped += 1;
                    continue;
                }
                tally.updated += 1;
                id
            }
            None => {
                let id = ghost_core::ids::new_id();
                let res = sqlx::query(
                    "INSERT INTO members (id, uuid, transient_id, email, name, note, status, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(ghost_core::ids::uuid_v4())
                .bind(ghost_core::ids::uuid_v4())
                .bind(&email)
                .bind(if name.is_empty() { None } else { Some(&name) })
                .bind(if note.is_empty() { None } else { Some(&note) })
                .bind(status)
                .bind(&now).bind(&now)
                .execute(&state.db).await;
                if res.is_err() {
                    tally.skipped += 1;
                    continue;
                }
                let _ = sqlx::query("INSERT INTO members_created_events (id, member_id, source, created_at) VALUES (?, ?, 'import', ?)")
                    .bind(ghost_core::ids::new_id()).bind(&id).bind(&now)
                    .execute(&state.db).await;
                tally.created += 1;
                id
            }
        };

        let labels = get(c_labels);
        if !labels.is_empty() {
            sync_member_labels(&state, &member_id, &labels).await;
        }

        // Custom fields, where a column was pointed at one. Written through
        // the same path the admin uses, so the same validation applies and a
        // bad cell is skipped rather than stored.
        if !metafield_cols.is_empty() {
            let mut writes: HashMap<String, HashMap<String, String>> = HashMap::new();
            for (key, idx) in &metafield_cols {
                let value = get(Some(*idx));
                if !value.is_empty() {
                    writes.entry(key.clone()).or_default().insert(String::new(), value);
                }
            }
            if !writes.is_empty() {
                let writer = repos::metafields::Writer {
                    writer_type: "import".into(),
                    writer_id: None,
                    source: "import".into(),
                };
                let _ = repos::metafields::write_values(&state.db, &member_id, &writes, &writer).await;
            }
        }
        if subscribe {
            for nid in &default_newsletters {
                let _ = sqlx::query(
                    "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) \
                     SELECT ?, ?, ?, 'confirmed' WHERE NOT EXISTS \
                     (SELECT 1 FROM members_newsletters WHERE member_id = ? AND newsletter_id = ?)",
                )
                .bind(ghost_core::ids::new_id()).bind(&member_id).bind(nid).bind(&member_id).bind(nid)
                .execute(&state.db).await;
            }
        } else {
            let _ = sqlx::query("DELETE FROM members_newsletters WHERE member_id = ?")
                .bind(&member_id)
                .execute(&state.db)
                .await;
        }
        let stripe = get(c_stripe);
        if !stripe.is_empty() {
            let _ = sqlx::query(
                "INSERT INTO members_stripe_customers (id, member_id, customer_id, created_at, updated_at) \
                 SELECT ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM members_stripe_customers WHERE customer_id = ?)",
            )
            .bind(ghost_core::ids::new_id()).bind(&member_id).bind(&stripe).bind(&now).bind(&now).bind(&stripe)
            .execute(&state.db).await;
        }
    }

    let skipped = if tally.skipped > 0 {
        format!(", {} skipped (missing or invalid email)", tally.skipped)
    } else {
        String::new()
    };
    members_flash(
        "ok",
        &format!("Imported {} new and updated {}{skipped}.", tally.created, tally.updated),
    )
}

// --- member editor (Ghost's Members → <member>) ---

/// The posted member editor. Newsletter checkboxes repeat under one name, so
/// this needs the same hand-rolled parsing the editor's tags use — `Form`
/// would keep only the last one.
#[derive(Debug, Default)]
pub(crate) struct MemberEditForm {
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) note: String,
    pub(crate) labels: String,
    pub(crate) status: String,
    pub(crate) newsletters: Vec<String>,
    /// Custom field values, as `key` → (`path` → value). The inputs are named
    /// `mf:<key>:<path>`, with an empty path for a scalar.
    pub(crate) metafields: HashMap<String, HashMap<String, String>>,
}

impl MemberEditForm {
    pub(crate) fn parse(body: &str) -> Self {
        let mut f = MemberEditForm::default();
        for (k, v) in form_urlencoded::parse(body.as_bytes()) {
            match k.as_ref() {
                "name" => f.name = v.into_owned(),
                "email" => f.email = v.into_owned(),
                "note" => f.note = v.into_owned(),
                "labels" => f.labels = v.into_owned(),
                "status" => f.status = v.into_owned(),
                "newsletters" => f.newsletters.push(v.into_owned()),
                other => {
                    // `mf:<key>:<path>` — split on the first two colons only,
                    // so a path is free to contain one.
                    if let Some(rest) = other.strip_prefix("mf:") {
                        if let Some((key, path)) = rest.split_once(':') {
                            f.metafields
                                .entry(key.to_string())
                                .or_default()
                                .insert(path.to_string(), v.into_owned());
                        }
                    }
                }
            }
        }
        f
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for MemberEditForm {
    type Rejection = Response;
    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(MemberEditForm::parse(&String::from_utf8_lossy(&bytes)))
    }
}

pub(crate) fn member_edit_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/members/{id}/?{encoded}")).into_response()
}

/// Labels attached to a member, as a comma-separated string for the form.
pub(crate) async fn member_labels(state: &AppState, member_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT l.name FROM labels l JOIN members_labels ml ON ml.label_id = l.id \
         WHERE ml.member_id = ? ORDER BY l.name",
    )
    .bind(member_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}


/// How one post did: read on the web, received in the inbox, and the members
/// it brought in.
///
/// Ghost puts this behind every post in the list, and it is the screen the
/// per-post figures exist for. The numbers come from the same function the
/// `/stats/posts/{id}/stats` endpoint serves, so the page and the API cannot
/// drift apart.
pub(crate) async fn post_analytics_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Author).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, &id).await else {
        return page_for_role(actor_role, "Not found", "posts", Some(&user), "<div class='banner error'>That post no longer exists.</div>", true).into_response();
    };
    let Some(stats) = ghost_api::stats::post_stats_value(&state, &id).await else {
        return page_for_role(actor_role, "Not found", "posts", Some(&user), "<div class='banner error'>That post no longer exists.</div>", true).into_response();
    };

    let figure = |n: i64| n.to_string();
    let tile = |label: &str, value: String, note: &str| {
        format!(
            r#"<div class="tile"><div class="label">{}</div><div class="value">{}</div><div class="delta">{}</div></div>"#,
            esc(label), esc(&value), esc(note)
        )
    };
    let views = stats["views"].as_i64().unwrap_or(0);
    let visitors = stats["visitors"].as_i64().unwrap_or(0);
    let free = stats["free_members"].as_i64().unwrap_or(0);
    let paid = stats["paid_members"].as_i64().unwrap_or(0);

    // The email card only appears for a post that was emailed; a web-only
    // post has no open rate, and a zero would read as a bad one.
    let email_card = match stats["email"].as_object() {
        None => String::new(),
        Some(email) => {
            let number = |key: &str| email.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0);
            let rate = |key: &str| match email.get(key).and_then(serde_json::Value::as_i64) {
                Some(value) => format!("{value}%"),
                // Null rather than zero: nothing delivered yet means the
                // rate is not known, which is not the same as none.
                None => "—".to_string(),
            };
            let status = email.get("status").and_then(serde_json::Value::as_str).unwrap_or("");
            let failed = number("failed");
            format!(
                r#"<div class="section">Email</div>
<div class="tiles rise" style="--i:3">
{sent}{delivered}{opened}{clicked}
</div>
<div class="card rows rise" style="--i:4">
<div class="row"><span class="ic tint-blue">{send_ic}</span>
<div class="body"><span class="title">Delivery</span><span class="detail">{status_note}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span></div></div>
{failures}
</div>"#,
                sent = tile("Sent", figure(number("sent")), "recipients"),
                delivered = tile("Delivered", figure(number("delivered")), "accepted by their provider"),
                opened = tile("Open rate", rate("open_rate"), &format!("{} opened", number("opened"))),
                clicked = tile("Click rate", rate("click_rate"), &format!("{} clicked a link", number("clicked"))),
                send_ic = svg("send"),
                status = esc(status),
                chip = if status == "submitted" { "published" } else if status == "failed" { "draft" } else { "scheduled" },
                status_note = if status == "submitting" {
                    "This send is still going out."
                } else if status == "failed" {
                    "This send did not finish. Retrying addresses only the recipients it missed."
                } else {
                    "This send finished."
                },
                failures = if failed == 0 {
                    String::new()
                } else {
                    format!(
                        r#"<div class="row"><span class="ic tint-red">{icon}</span>
<div class="body"><span class="title">{failed} could not be delivered</span><span class="detail">Bounced or refused by the recipient's provider.</span></div></div>"#,
                        icon = svg("delete"),
                    )
                },
            )
        }
    };

    // Every per-post figure is asked across the set of paths the post is
    // reachable at, read once here rather than per section.
    let paths = post_paths_for(&state, &post).await;
    {
        if let Ok(Some(_)) = ghost_api::click::connection(&state).await {
            return click::post_analytics(&state, actor_role, &user, &id, &post.title.clone().unwrap_or_else(|| "Untitled".into()), &paths, free, paid, &email_card).await;
        }
    }
    let click_post_report=if actor_role>=Role::Administrator {click::post_report_link(&state,&paths).await} else {String::new()};

    // The shape of its readership over its first 30 days, or the last 30 if
    // it is older — a post's traffic is a curve, and four totals do not show
    // whether it is still being read.
    let since: String = sqlx::query_scalar(ghost_db::dialect::sql("SELECT datetime('now', '-29 days')"))
        .fetch_one(&state.db)
        .await
        .unwrap_or_default();
    let daily = analytics::daily_for_paths(&state.db, &paths, &since).await;
    let seen: HashMap<String, i64> = daily.into_iter().collect();
    let mut post_days: Vec<DayViews> = Vec::new();
    for back in (0..30).rev() {
        // Worked out here rather than asked of the database: `date('now', ?)`
        // takes its offset as a bound parameter, which MySQL cannot express,
        // and the answer never depended on the server anyway.
        let day: String = (chrono::Utc::now() - chrono::Duration::days(back))
            .format("%Y-%m-%d")
            .to_string();
        post_days.push(DayViews { views: seen.get(&day).copied().unwrap_or(0), day });
    }

    let post_locations = analytics::locations_for_paths(&state.db, &paths, 20).await;
    let post_placed = post_locations.iter().any(|(code, _, _)| code.is_some());
    let post_campaigns = analytics::campaigns_for_paths(&state.db, &paths, "campaign", 10).await;
    let campaign_rows: Vec<(String, String, i64, i64)> = post_campaigns
        .into_iter()
        .map(|(name, visitors, views)| (String::new(), name, visitors, views))
        .collect();

    // Where the readers came from. Direct traffic is a real answer, so it is
    // named rather than left blank.
    let mut referrers: Vec<(String, i64, i64)> = Vec::new();
    for path in &paths {
        let rows = sqlx::query_as::<_, (Option<String>, i64, i64)>(
            "SELECT COALESCE(referrer_source, referrer), COUNT(*), COUNT(DISTINCT viewer) \
             FROM page_views WHERE path = ? GROUP BY 1 ORDER BY COUNT(*) DESC",
        )
        .bind(path)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        for (source, views, visitors) in rows {
            referrers.push((source.unwrap_or_else(|| "Direct".into()), views, visitors));
        }
    }
    referrers.sort_by_key(|(_, views, _)| -views);
    referrers.truncate(10);
    let referrer_rows: String = referrers
        .iter()
        .map(|(source, views, _)| {
            analytics_row(&svg("send"), "purple", source, &format!("{views} views"), &figure(*views), None)
        })
        .collect();

    let title = post.title.clone().unwrap_or_else(|| "Untitled".into());
    page_for_role(
        actor_role,
        "Post analytics",
        "posts",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/posts/" title="Back to posts">{back}</a>
<div><h1>{title}</h1><p class="sub">{status} · {published}</p></div>
<div class="actions"><a class="btn tonal" href="/ghost/posts/{id}/">Edit</a></div></header>
{click_post_report}
<div class="section">Web</div>
<div class="tiles rise" style="--i:1">
{views_tile}{visitors_tile}{free_tile}{paid_tile}
</div>
<div class="section">Unique visitors per day · last 30 days</div>
<div class="card padded rise" style="--i:2">{chart}</div>
<div class="section">Sources</div>
<div class="card rows rise" style="--i:3">{referrer_rows}</div>
<div class="section">Locations</div>
{locations}
<div class="section">Campaigns</div>
{campaigns}
{email_card}
<p class="muted" style="margin:18px 0 0">Members are counted where a signup or a subscription was attributed to this post, not where a member happened to be reading when they joined.</p>
<script src="{assets}/analytics-map.js" defer></script>"#,
            back = svg("chevron"),
            title = esc(&title),
            status = esc(&post.status),
            published = esc(post.published_at.as_deref().unwrap_or("not published").get(..10).unwrap_or("")),
            id = esc(&id),
            views_tile = tile("Views", figure(views), "page loads"),
            visitors_tile = tile("Visitors", figure(visitors), "unique, counted daily"),
            free_tile = tile("Free signups", figure(free), "attributed to this post"),
            paid_tile = tile("Paid conversions", figure(paid), "attributed to this post"),
            chart = views_chart(&post_days),
            referrer_rows = analytics_rows(referrer_rows, "No traffic recorded for this post yet."),
            locations = analytics::locations_card(&post_locations, ASSETS, !post_placed),
            campaigns = analytics::breakdown_card(
                "Campaign",
                &campaign_rows,
                "No reader arrived at this post on a link tagged with a campaign.",
                6,
            ),
            email_card = email_card,
            assets = ASSETS,
        ),
        true,
    )
    .into_response()
}

/// The site paths a post is reachable at, for matching page views.
pub(crate) async fn post_paths_for(state: &AppState, post: &ghost_core::models::Post) -> Vec<String> {
    let router = state.router.read().await.clone();
    let json = ghost_core::models::post_to_json(post, &[], &[], "");
    let path = match router.permalink_for(&json) {
        Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, &json),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let bare = path.trim_end_matches('/').to_string();
    if bare.is_empty() { vec!["/".into()] } else { vec![format!("{bare}/"), bare] }
}

/// A member's activity, as the rows of a card.
///
/// One line per event, newest first, in the member's own words rather than
/// the event table's: "Opened Issue 14" rather than `email_opened_event`.
pub(crate) async fn member_activity_card(state: &AppState, member_id: &str) -> String {
    let events = ghost_api::member_events::events_for_member(state, member_id, 30).await;
    if events.is_empty() {
        return String::new();
    }
    let rows: String = events
        .iter()
        .map(|event| {
            let kind = event["type"].as_str().unwrap_or("");
            let data = &event["data"];
            let extra = &data["extra"];
            let post_title = extra["post"]["title"].as_str().unwrap_or("");
            let email_title = extra["email"]["title"].as_str().unwrap_or("");
            let (icon, tint, title, detail) = match kind {
                "signup_event" => ("ghost", "blue", "Signed up".to_string(), String::new()),
                "login_event" => ("ghost", "blue", "Signed in".to_string(), String::new()),
                "subscription_event" => {
                    let kind = extra["type"].as_str().unwrap_or("started");
                    let delta = extra["mrr_delta"].as_i64().unwrap_or(0);
                    (
                        "tags",
                        "amber",
                        format!("Subscription {kind}"),
                        if delta == 0 { String::new() } else { format!("{} MRR", money(delta, extra["currency"].as_str())) },
                    )
                }
                "cancel_event" => ("tags", "red", "Cancelled their subscription".to_string(), String::new()),
                "payment_event" => (
                    "tags",
                    "green",
                    format!("Paid {}", money(extra["amount"].as_i64().unwrap_or(0), extra["currency"].as_str())),
                    String::new(),
                ),
                "newsletter_event" => ("mail", "green", "Changed their newsletter preferences".to_string(), String::new()),
                "email_change_event" => ("mail", "blue", "Changed their email address".to_string(), String::new()),
                "comment_event" => ("comment", "purple", "Commented".to_string(), post_title.to_string()),
                "click_event" => (
                    "send",
                    "purple",
                    "Clicked a link in an email".to_string(),
                    extra["link"]["to"].as_str().unwrap_or("").to_string(),
                ),
                "feedback_event" => (
                    "send",
                    "purple",
                    if extra["score"].as_i64() == Some(1) { "More like this".to_string() } else { "Less like this".to_string() },
                    post_title.to_string(),
                ),
                "email_sent_event" => ("send", "blue", "Was sent an email".to_string(), email_title.to_string()),
                "email_delivered_event" => ("send", "green", "Received an email".to_string(), email_title.to_string()),
                "email_opened_event" => ("send", "green", "Opened an email".to_string(), email_title.to_string()),
                "email_failed_event" => ("send", "red", "An email could not be delivered".to_string(), email_title.to_string()),
                other => ("ghost", "blue", other.replace('_', " "), String::new()),
            };
            let at = data["created_at"].as_str().unwrap_or("");
            analytics_row(
                &svg(icon),
                tint,
                &title,
                &if detail.is_empty() { esc(&at[..10.min(at.len())]) } else { format!("{} · {}", esc(&detail), esc(&at[..10.min(at.len())])) },
                "",
                None,
            )
        })
        .collect();
    format!(
        r#"<div class="section">Activity</div>
<div class="card rows rise" style="--i:6">{rows}</div>"#
    )
}

/// An amount in a currency's smallest unit, as money.
pub(crate) fn money(amount: i64, currency: Option<&str>) -> String {
    let code = currency.unwrap_or("usd").to_uppercase();
    let symbol = match code.as_str() {
        "USD" => "$",
        "EUR" => "\u{20ac}",
        "GBP" => "\u{a3}",
        _ => "",
    };
    let whole = amount / 100;
    let cents = (amount % 100).abs();
    if symbol.is_empty() {
        format!("{whole}.{cents:02} {code}")
    } else {
        format!("{symbol}{whole}.{cents:02}")
    }
}

/// Everything the member form holds: details, newsletters, custom fields and
/// access, as one `<form id="member-form">` posting to `action`.
///
/// Shared by the editor and the new-member page so creating a member offers
/// the same fields as editing one. "New member" used to be a popover in the
/// page header with three inputs — name, email, note — which meant a member's
/// labels, newsletters, custom fields and complimentary access could only be
/// set by saving them once and then opening them again.
///
/// `subscribed` is `None` for a member who does not exist yet: there is
/// nothing to read, so the newsletters that new signups get are pre-ticked,
/// which is what signing up would have done.
pub(crate) async fn member_form(
    state: &AppState,
    action: &str,
    name: &str,
    email: &str,
    note: &str,
    labels: &str,
    status: &str,
    subscribed: Option<&[String]>,
    held: &HashMap<String, HashMap<String, String>>,
) -> String {
    let newsletters = sqlx::query_as::<_, (String, String, Option<String>, i64)>(
        "SELECT id, name, description, subscribe_on_signup FROM newsletters \
         WHERE status = 'active' ORDER BY sort_order ASC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let newsletter_rows: String = if newsletters.is_empty() {
        r#"<p class="muted" style="margin:0">No active newsletters.</p>"#.into()
    } else {
        newsletters
            .iter()
            .map(|(nid, nname, ndesc, on_signup)| {
                let on = match subscribed {
                    Some(ids) => ids.iter().any(|s| s == nid),
                    None => *on_signup != 0,
                };
                format!(
                    r#"<label class="fchip" title="{desc}"><input type="checkbox" name="newsletters" value="{nid}" {checked}><span class="ck">{check}</span>{nname}</label>"#,
                    nid = esc(nid),
                    checked = if on { "checked" } else { "" },
                    nname = esc(nname),
                    check = svg("check"),
                    desc = esc(ndesc.as_deref().unwrap_or("")),
                )
            })
            .collect()
    };

    // --- the member's custom fields -----------------------------------------
    //
    // Staff see every field whatever its member access, which is what that
    // setting is for: `none` means the member cannot see it, not that nobody
    // can. Archived fields show only when this member still holds a value, so
    // a retired field's data stays visible without inviting new entries.
    let definitions = repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let mut metafield_rows = String::new();
    for def in &definitions {
        let values = held.get(&def.key);
        if !def.is_active() && values.map(|v| v.is_empty()).unwrap_or(true) {
            continue;
        }
        let at = |path: &str| {
            values.and_then(|v| v.get(path)).map(String::as_str).unwrap_or_default().to_string()
        };
        // Inputs are named `mf:<key>:<path>` so one flat form carries every
        // field and part without the names colliding with the member's own.
        let body = match def.parts() {
            Some(parts) => {
                let inputs: String = parts
                    .iter()
                    .map(|(path, label, part_type)| {
                        let max = match *part_type {
                            "postal_code" => ghost_core::metafields::MAX_POSTAL_CODE_CHARS,
                            "country_code" => 2,
                            _ => ghost_core::metafields::MAX_SHORT_TEXT_CHARS,
                        };
                        format!(
                            r#"<label class="field" style="margin:0"><input name="mf:{key}:{path}" value="{v}" placeholder=" " maxlength="{max}"><span>{label}</span></label>"#,
                            key = esc(&def.key),
                            path = esc(path),
                            v = esc(&at(path)),
                            max = max,
                            label = esc(label),
                        )
                    })
                    .collect();
                format!(r#"<div class="grid2" style="gap:12px">{inputs}</div>"#)
            }
            None if def.field_type == "long_text" => format!(
                r#"<label class="field" style="margin:0"><textarea name="mf:{key}:" rows="3" placeholder=" ">{v}</textarea><span>{name}</span></label>"#,
                key = esc(&def.key),
                v = esc(&at("")),
                name = esc(&def.name),
            ),
            None => format!(
                r#"<label class="field" style="margin:0"><input name="mf:{key}:" value="{v}" placeholder=" " maxlength="{max}"><span>{name}</span></label>"#,
                key = esc(&def.key),
                v = esc(&at("")),
                max = ghost_core::metafields::MAX_SHORT_TEXT_CHARS,
                name = esc(&def.name),
            ),
        };
        metafield_rows.push_str(&format!(
            r#"<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{ic}</span>
<div class="body" style="width:100%">{heading}{body}</div></div>"#,
            ic = svg("notes"),
            // The inputs below float their own label into their top border, so
            // a heading needs clearance or the two overlap.
            heading = if def.parts().is_some() {
                format!(
                    r#"<span class="title" style="display:block;margin:0 0 14px">{}{}</span>"#,
                    esc(&def.name),
                    if def.is_active() { "" } else { " · archived" }
                )
            } else if def.is_active() {
                String::new()
            } else {
                r#"<span class="detail" style="display:block;margin:0 0 14px">archived</span>"#
                    .to_string()
            },
            body = body,
        ));
    }
    let metafields_card = if metafield_rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="section">Custom fields</div>
<div class="card rows rise" style="--i:3">{metafield_rows}</div>"#
        )
    };

    format!(
        r#"<form method="post" action="{action}" id="member-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{ghost_ic}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" "><span>Name</span></label>
<label class="field" style="margin:0"><input name="email" type="email" value="{email_v}" placeholder=" " required><span>Email</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="note" rows="2" placeholder=" " maxlength="2000">{note}</textarea><span>Note · staff only, never shown to the member</span></label>
<label class="field" style="margin:12px 0 0"><input name="labels" value="{labels}" placeholder=" "><span>Labels · comma separated</span></label>
<p class="muted" style="margin:10px 0 0">Labels group members for filtering and segmentation. New names are created as you type them.</p>
</div></div>
</div>
<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{mail}</span>
<div class="body" style="width:100%"><div class="chips">{newsletter_rows}</div>
<p class="muted" style="margin:10px 0 0">Unticking every newsletter unsubscribes the member from email entirely.</p>
</div></div>
</div>
{metafields_card}
<div class="section">Access</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:center"><span class="ic tint-amber">{tags_ic}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="status">{status_opts}</select><span>Status</span>{car}</label>
<p class="muted" style="margin:6px 0 0">{status_note}</p></div></div>
</div>
</form>"#,
        action = esc(action),
        name_v = esc(name),
        email_v = esc(email),
        note = esc(note),
        labels = esc(labels),
        ghost_ic = svg("ghost"),
        mail = svg("mail"),
        tags_ic = svg("tags"),
        car = SELECT_CARET,
        newsletter_rows = newsletter_rows,
        metafields_card = metafields_card,
        status_opts = select_options(
            &[("free", "Free"), ("comped", "Complimentary — paid access, no charge")],
            status
        ),
        status_note = if status == "paid" {
            "This member has a paid subscription. Changing it here does not touch billing, so the list is limited to the states this port can honour."
        } else {
            "Complimentary grants paid-tier access without Stripe. Real paid subscriptions are not managed here."
        },
    )
}

/// Members → New member. The editor's own form on a blank member, rather than
/// the three-field popover this used to be.
///
/// Fields prefill from the query string, so the error round-trip a rejected
/// email takes (duplicate, malformed) comes back with the typing intact.
/// Newsletters and custom fields do not survive that trip — they are lists,
/// and a URL is the wrong place to rebuild one.
pub(crate) async fn member_new_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let was = |key: &str| q.get(key).map(String::as_str).unwrap_or("").to_string();
    let status = match was("status").as_str() {
        "comped" => "comped",
        _ => "free",
    };
    page_for_role(
        actor_role,
        "New member",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/members/" title="Back to members">{back}</a>
<div><h1>New member</h1><p class="sub">Added by hand, not by signup</p></div>
<div class="actions"><button class="btn" type="submit" form="member-form">{check} Add member</button></div></header>
{banner}
{form}"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            form = member_form(
                &state,
                "/ghost/members/new/",
                &was("name"),
                &was("email"),
                &was("note"),
                &was("labels"),
                status,
                None,
                &HashMap::new(),
            )
            .await,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn member_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let row = sqlx::query_as::<_, (String, Option<String>, String, String, Option<String>, String, Option<String>, Option<String>)>(
        "SELECT id, name, email, status, note, created_at, uuid, commenting FROM members WHERE id = ? LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((mid, name, email, status, note, created, _uuid, commenting)) = row else {
        return not_found_page(&user, "That member could not be found.");
    };

    // Commenting is per-member state a moderator acts on from here: a
    // suspension is aimed at a person, not at a comment, so it belongs on
    // their profile rather than on the moderation queue.
    let commenting = ghost_api::comments::Commenting::parse(commenting.as_deref());
    let comment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE member_id = ? AND status <> 'deleted'")
            .bind(&mid)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    let labels = member_labels(&state, &mid).await;
    let subscribed: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT newsletter_id FROM members_newsletters WHERE member_id = ?",
    )
    .bind(&mid)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let held = repos::metafields::values_for_member(&state.db, &mid).await.unwrap_or_default();

    let initial = name.as_deref().unwrap_or(&email).chars().next().unwrap_or('?').to_uppercase().to_string();
    page_for_role(actor_role,
        "Member",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/members/" title="Back to members">{back}</a>
<div><h1>{title}</h1><p class="sub">Member since {created}</p></div>
<div class="actions"><button class="btn" type="submit" form="member-form">{check} Save</button></div></header>
{banner}
<div class="card padded rise" style="--i:1;display:flex;align-items:center;gap:16px;flex-wrap:wrap">
<span class="avatar lg">{initial}</span>
<div style="min-width:0"><b style="font-size:17px;display:block">{title}</b><span class="muted">{email}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span></div>
</div>
{form}
{commenting_card}
{activity_card}
<div class="section">Danger zone</div>
<div class="card rows rise" style="--i:5">
<div class="row"><span class="ic tint-red">{del}</span>
<div class="body"><span class="title">Delete this member</span><span class="detail">Removes the account, its subscriptions and its labels. This cannot be undone.</span></div>
<div class="trailing">
<form method="post" action="/ghost/members/{id}/delete/" data-confirm="Delete member|&#8220;{title}&#8221; will be permanently removed.">
<button class="btn tonal small danger" type="submit">Delete</button></form></div></div>
</div>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&mid),
            initial = esc(&initial),
            title = esc(name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&email)),
            email = esc(&email),
            created = esc(&created[..10.min(created.len())]),
            chip = if status == "free" { "draft" } else { "published" },
            status = esc(&status),
            del = svg("delete"),
            commenting_card = comments::member_commenting_card(&mid, &commenting, comment_count),
            activity_card = member_activity_card(&state, &mid).await,
            form = member_form(
                &state,
                &format!("/ghost/members/{mid}/"),
                name.as_deref().unwrap_or(""),
                &email,
                note.as_deref().unwrap_or(""),
                &labels.join(", "),
                &status,
                Some(&subscribed),
                &held,
            )
            .await,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn member_edit_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    form: MemberEditForm,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let email = form.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return member_edit_flash(&id, "error", "A valid email is required.");
    }
    // Email identifies a member at signin, so a collision would let one
    // person's magic link land on another's account.
    let clash: Option<String> = sqlx::query_scalar("SELECT id FROM members WHERE lower(email) = ? AND id != ?")
        .bind(&email)
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if clash.is_some() {
        return member_edit_flash(&id, "error", "Another member already uses that email.");
    }

    let status = match form.status.as_str() {
        "comped" => "comped",
        "paid" => "paid",
        _ => "free",
    };
    let now = ghost_core::dates::now_db();
    let updated = sqlx::query(
        "UPDATE members SET name = ?, email = ?, note = ?, status = ?, updated_at = ? WHERE id = ?",
    )
    .bind(form.name.trim())
    .bind(&email)
    .bind(form.note.trim())
    .bind(status)
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await;
    if let Err(e) = updated {
        return member_edit_flash(&id, "error", &format!("Could not save: {e}"));
    }

    // Newsletters: the posted set replaces whatever was stored, so an
    // unticked box really does unsubscribe.
    let _ = sqlx::query("DELETE FROM members_newsletters WHERE member_id = ?")
        .bind(&id)
        .execute(&state.db)
        .await;
    for nid in &form.newsletters {
        let _ = sqlx::query(
            "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, 'confirmed')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(&id)
        .bind(nid)
        .execute(&state.db)
        .await;
    }

    sync_member_labels(&state, &id, &form.labels).await;

    // Custom fields. A rejected value is reported rather than swallowed: the
    // rest of the member has already saved, so the message says what did not.
    if !form.metafields.is_empty() {
        let actor = match require_session(&state, &headers).await {
            Ok(u) => u.id,
            Err(_) => String::new(),
        };
        let writer = repos::metafields::Writer::staff(&actor);
        if let Err(e) =
            repos::metafields::write_values(&state.db, &id, &form.metafields, &writer).await
        {
            return member_edit_flash(
                &id,
                "error",
                &format!("Member saved, but a custom field was not — {}", e.message()),
            );
        }
    }

    fire_member_webhook(&state, "member.edited", &id, &email, form.name.trim());
    member_edit_flash(&id, "ok", "Member saved.")
}

/// Find or create the label row for `name`, returning its id.
///
/// `labels.name` is unique, so an existing row is reused rather than
/// duplicated — the same rule the member editor's picker follows.
pub(crate) async fn ensure_label(state: &AppState, name: &str) -> Option<String> {
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM labels WHERE name = ?")
        .bind(name)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if let Some(lid) = existing {
        return Some(lid);
    }
    let lid = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
    sqlx::query("INSERT INTO labels (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
        .bind(&lid)
        .bind(name)
        .bind(&now)
        .bind(&now)
        .execute(&state.db)
        .await
        .ok()?;
    Some(lid)
}

/// Replace a member's labels with the comma-separated list, creating any
/// label that does not exist yet (Ghost does the same from its label picker).
pub(crate) async fn sync_member_labels(state: &AppState, member_id: &str, csv: &str) {
    let _ = sqlx::query("DELETE FROM members_labels WHERE member_id = ?")
        .bind(member_id)
        .execute(&state.db)
        .await;
    let mut seen: Vec<String> = Vec::new();
    for raw in csv.split(',') {
        let name = raw.trim();
        if name.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            continue;
        }
        seen.push(name.to_string());
        let Some(label_id) = ensure_label(state, name).await else {
            continue;
        };
        let _ = sqlx::query(
            "INSERT INTO members_labels (id, member_id, label_id, sort_order) VALUES (?, ?, ?, 0)",
        )
        .bind(ghost_core::ids::new_id())
        .bind(member_id)
        .bind(&label_id)
        .execute(&state.db)
        .await;
    }
}

/// Create a member from the new-member page's form — the editor's form, so
/// every field it offers has to be saved, not just the three the old popover
/// had. Labels, newsletters and custom fields go through the same helpers the
/// editor's save uses, so the two cannot diverge in what a field means.
///
/// A rejected email comes back to the form with the typing in the query
/// string rather than to an emptied page.
pub(crate) async fn member_create(State(state): State<AppState>, headers: HeaderMap, form: MemberEditForm) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Back to the form, carrying what was typed.
    let retry = |message: &str| -> Response {
        let mut qs = form_urlencoded::Serializer::new(String::new());
        qs.append_pair("error", message)
            .append_pair("name", form.name.trim())
            .append_pair("email", form.email.trim())
            .append_pair("note", form.note.trim())
            .append_pair("labels", form.labels.trim())
            .append_pair("status", &form.status);
        Redirect::to(&format!("/ghost/members/new/?{}", qs.finish())).into_response()
    };

    let email = form.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return retry("A valid email is required.");
    }
    let name = form.name.trim();
    let status = match form.status.as_str() {
        "comped" => "comped",
        _ => "free",
    };
    let now = ghost_core::dates::now_db();
    let id = ghost_core::ids::new_id();
    let res = sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, name, note, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(uuid_v4())
    .bind(ghost_core::ids::new_id())
    .bind(&email)
    .bind(Some(name).filter(|n| !n.is_empty()))
    .bind(Some(form.note.trim()).filter(|n| !n.is_empty()))
    .bind(status)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        return if e.to_string().contains("UNIQUE") {
            retry("A member with that email already exists.")
        } else {
            retry(&format!("Could not add member: {e}"))
        };
    }

    for nid in &form.newsletters {
        let _ = sqlx::query(
            "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, 'confirmed')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(&id)
        .bind(nid)
        .execute(&state.db)
        .await;
    }
    sync_member_labels(&state, &id, &form.labels).await;

    let _ = repos::actions::record(&state.db, "members", &id, &user.id, "added").await;
    fire_member_webhook(&state, "member.added", &id, &email, name);

    // The member exists either way, so a rejected custom field is reported on
    // their page rather than sending the whole form back.
    if !form.metafields.is_empty() {
        let writer = repos::metafields::Writer::staff(&user.id);
        if let Err(e) =
            repos::metafields::write_values(&state.db, &id, &form.metafields, &writer).await
        {
            return member_edit_flash(
                &id,
                "error",
                &format!("Member added, but a custom field was not — {}", e.message()),
            );
        }
    }
    member_edit_flash(&id, "ok", &format!("Member {email} added."))
}

/// Remove one member and everything pointing at them. Shared by the row's
/// own delete and the list's batch delete so the two can never diverge in
/// what a "deleted member" leaves behind.
pub(crate) async fn delete_member(state: &AppState, actor_id: &str, id: &str) -> bool {
    // Who they were, read before the row goes: a delete webhook carrying
    // only an id tells a CRM nothing it can act on.
    let gone: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT email, name FROM members WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

    // clear the join tables first — FKs point at members
    for table in ["members_products", "members_newsletters", "members_labels", "members_stripe_customers", "subscriptions"] {
        let _ = sqlx::query(&format!("DELETE FROM {table} WHERE member_id = ?"))
            .bind(id)
            .execute(&state.db)
            .await;
    }
    let _ = sqlx::query("DELETE FROM members WHERE id = ?").bind(id).execute(&state.db).await;
    let _ = repos::actions::record(&state.db, "members", id, actor_id, "deleted").await;
    let existed = gone.is_some();
    if let Some((email, name)) = gone {
        fire_member_webhook(state, "member.deleted", id, &email, name.as_deref().unwrap_or(""));
    }
    existed
}

pub(crate) async fn member_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    delete_member(&state, &user.id, &id).await;
    members_flash("ok", "Member deleted.")
}

/// What the members list's batch bar posted. `ids` repeats under one name,
/// and `label` arrives as a list because the add and remove popovers each
/// hold an input under that name — whichever was filled is the one that
/// means anything.
#[derive(Debug, Default)]
pub(crate) struct BulkMembersForm {
    action: String,
    ids: Vec<String>,
    label: Vec<String>,
}

impl BulkMembersForm {
    pub(crate) fn parse(body: &str) -> Self {
        let mut f = BulkMembersForm::default();
        for (k, v) in form_urlencoded::parse(body.as_bytes()) {
            match k.as_ref() {
                "action" => f.action = v.into_owned(),
                "ids" => f.ids.push(v.into_owned()),
                "label" => f.label.push(v.into_owned()),
                _ => {}
            }
        }
        f
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for BulkMembersForm {
    type Rejection = Response;
    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(BulkMembersForm::parse(&String::from_utf8_lossy(&bytes)))
    }
}

/// The members list's batch actions: add label, remove label, delete —
/// Ghost's own three. Labels are created by name when they do not exist,
/// exactly as the member editor's picker does.
pub(crate) async fn members_bulk(State(state): State<AppState>, headers: HeaderMap, form: BulkMembersForm) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    if form.ids.is_empty() {
        return members_flash("error", "Nothing was selected.");
    }
    let label = form
        .label
        .iter()
        .rev()
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty());

    let mut done = 0usize;
    match form.action.as_str() {
        "delete" => {
            for id in &form.ids {
                if delete_member(&state, &user.id, id).await {
                    done += 1;
                }
            }
            if done == 0 {
                return members_flash("error", "Those members no longer exist.");
            }
            members_flash("ok", &format!("Deleted {} {}.", done, if done == 1 { "member" } else { "members" }))
        }
        "add_label" | "remove_label" => {
            let Some(label) = label else {
                return members_flash("error", "Type or pick a label first.");
            };
            let adding = form.action == "add_label";
            let label_id = if adding {
                match ensure_label(&state, &label).await {
                    Some(lid) => lid,
                    None => return members_flash("error", "The label could not be saved."),
                }
            } else {
                // Removing never creates anything; an unknown name simply
                // matches nothing, which is what was asked for.
                match sqlx::query_scalar::<_, String>("SELECT id FROM labels WHERE name = ?")
                    .bind(&label)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten()
                {
                    Some(lid) => lid,
                    None => {
                        return members_flash("ok", &format!("No member has the label “{label}”."))
                    }
                }
            };
            for id in &form.ids {
                if adding {
                    let _ = sqlx::query(
                        ghost_db::dialect::sql("INSERT INTO members_labels (id, member_id, label_id, sort_order) VALUES (?, ?, ?, 0) \
                         ON CONFLICT DO NOTHING"),
                    )
                    .bind(ghost_core::ids::new_id())
                    .bind(id)
                    .bind(&label_id)
                    .execute(&state.db)
                    .await;
                } else {
                    let _ = sqlx::query("DELETE FROM members_labels WHERE member_id = ? AND label_id = ?")
                        .bind(id)
                        .bind(&label_id)
                        .execute(&state.db)
                        .await;
                }
                done += 1;
            }
            let verb = if adding { "added to" } else { "removed from" };
            members_flash(
                "ok",
                &format!("Label “{label}” {verb} {} {}.", done, if done == 1 { "member" } else { "members" }),
            )
        }
        "unsubscribe" => {
            // Off every newsletter, not deleted: the member keeps their
            // account, their tier and their access. Ghost's own bulk
            // unsubscribe does the same.
            for id in &form.ids {
                let _ = sqlx::query("DELETE FROM members_newsletters WHERE member_id = ?")
                    .bind(id)
                    .execute(&state.db)
                    .await;
                done += 1;
            }
            members_flash(
                "ok",
                &format!(
                    "Unsubscribed {} {} from every newsletter.",
                    done,
                    if done == 1 { "member" } else { "members" }
                ),
            )
        }
        _ => members_flash("error", "Unknown batch action."),
    }
}

/// Redirect back to the members page carrying a one-shot banner.
pub(crate) fn members_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/members/?{encoded}")).into_response()
}
