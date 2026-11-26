//! Sample portfolios, so the screens and the public pages can be seen
//! working before anything real is added:
//!
//! * **personal** — Lena Hart, a product designer in Lisbon: six projects
//!   (one password-protected), services, a résumé, skills, testimonials.
//! * **studio** — Harbour Lane, a brand and digital studio: six projects,
//!   services with starting prices, clients, a team of four, awards and
//!   press, testimonials.
//!
//! Pictures are Unsplash photos linked where they are hosted. The sample's
//! enquiries come from `@example.com` and are never emailed. The password
//! of the protected sample project is `preview`.

use super::*;

const IMG: &str = "https://images.unsplash.com/photo-";

fn img(id: &str) -> String {
    format!("{IMG}{id}?w=1600&q=80")
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct SampleForm {
    kind: String,
}

pub(crate) async fn sample_load(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<SampleForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let studio = f.kind == "studio";
    match seed_sample_portfolio(&state.db, &state.settings, studio).await {
        Ok(0) => flash_to(BASE, "error", "There are projects here already, so nothing was added."),
        Ok(n) => crate::tools::tool_saved(&state, BASE, &format!("Sample {} loaded: {n} projects, ready to edit. The portfolio is switched on.", if studio { "studio" } else { "portfolio" })),
        Err(e) => {
            tracing::error!("portfolio: sample failed: {}", e.message());
            flash_to(BASE, "error", "The sample could not be loaded.")
        }
    }
}

struct Proj {
    title: &'static str,
    subtitle: &'static str,
    summary: &'static str,
    client: usize,
    year: i64,
    role: &'static str,
    duration: &'static str,
    industry: &'static str,
    tools: &'static str,
    cats: &'static [usize],
    services: &'static [usize],
    images: &'static [&'static str],
    body: &'static [&'static str],
    metrics: &'static [(&'static str, &'static str)],
    status: &'static str,
    featured: bool,
    accent: &'static str,
    layout: &'static str,
}

/// Also `vuuut seed-portfolio [studio]`. Does nothing when projects exist.
pub async fn seed_sample_portfolio(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache, studio: bool) -> vuuut_core::Result<usize> {
    static SAMPLE_WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _write = SAMPLE_WRITE.lock().await;
    let (existing, _) = repo::projects(db, &repo::ProjectQuery { limit: 1, ..Default::default() }).await;
    if !existing.is_empty() {
        return Ok(0);
    }
    let cat_names: &[(&str, &str)] = if studio {
        &[("Branding", "amber"), ("Digital", "blue"), ("Packaging", "green"), ("Campaigns", "pink"), ("Spaces", "teal")]
    } else {
        &[("Product design", "blue"), ("Mobile", "purple"), ("Web", "teal"), ("Design systems", "amber"), ("Research", "pink")]
    };
    let mut cats = Vec::new();
    for (i, (n, t)) in cat_names.iter().enumerate() {
        cats.push(repo::save_category(db, None, n, &vuuut_core::directory::slugify(n), "", t, i as i64).await?);
    }
    let service_defs: &[(&str, &str, &str, i64, &str, &str)] = if studio {
        &[
            ("Brand identity", "layers", "Names, marks, type and colour — a system that holds up everywhere it goes.", 1_800_000, "per project", "Brand strategy workshop\nLogo and visual identity\nTypography and colour\nBrand guidelines\nLaunch assets"),
            ("Websites", "code", "Fast, accessible sites your team can update without us.", 2_500_000, "per project", "Content strategy\nDesign in Figma\nBuilt and launched\nTraining for your team"),
            ("Packaging", "cube", "Boxes, bottles and labels that earn their place on the shelf.", 1_200_000, "per range", "Structural design\nPrinter-ready artwork\nPress checks"),
            ("Campaigns", "megaphone", "Launches and seasonal campaigns across print, social and outdoor.", 900_000, "per campaign", "Creative concept\nCopywriting\nArt direction\nAsset production"),
            ("Spaces & signage", "home", "Wayfinding, signage and interiors that carry the brand into the room.", 1_500_000, "per space", "Wayfinding system\nSignage design\nFabrication oversight"),
        ]
    } else {
        &[
            ("Product design", "brush", "End-to-end design for web and mobile products, from first sketch to shipped.", 0, "", "Discovery and research\nFlows and wireframes\nHigh-fidelity UI\nPrototypes\nHand-off and QA"),
            ("Design systems", "layers", "Components, tokens and documentation your team will actually use.", 0, "", "Audit of what exists\nTokens and components\nFigma library\nDocumentation site"),
            ("UX audits", "chart", "A clear-eyed review of your product with a prioritised list of fixes.", 250_000, "per audit", "Heuristic review\nUsability sessions\nPrioritised report\nWalk-through call"),
        ]
    };
    let mut services = Vec::new();
    for (i, (n, icon, sum, price, note, del)) in service_defs.iter().enumerate() {
        services.push(repo::save_service(db, None, &rules::ServiceInput {
            name: n.to_string(), slug: String::new(), summary: sum.to_string(), description: format!("{sum}\n\nEvery engagement starts with a short call to understand what you need, then a written proposal with a fixed scope and price."),
            image: String::new(), icon: icon.to_string(), price_from: *price, price_note: note.to_string(), deliverables: del.lines().map(str::to_string).collect(),
            status: "active".into(), featured: i < 3, sort_order: i as i64,
        }).await?);
    }
    let client_names: &[(&str, &str)] = if studio {
        &[("Northfield Coffee", "Food & drink"), ("Pebble Bank", "Finance"), ("Lumen Health", "Healthcare"), ("Orchard Labs", "Technology"), ("Kestrel Air", "Travel"), ("Tidewater Hotels", "Hospitality"), ("Maple & Moss", "Retail"), ("Harbour Museum", "Culture")]
    } else {
        &[("Pebble Bank", "Finance"), ("Lumen Health", "Healthcare"), ("Orchard Labs", "Technology"), ("Kestrel Air", "Travel"), ("Tidewater Hotels", "Hospitality")]
    };
    let mut clients = Vec::new();
    for (i, (n, ind)) in client_names.iter().enumerate() {
        clients.push(repo::save_client(db, None, &rules::ClientInput { name: n.to_string(), industry: ind.to_string(), featured: true, sort_order: i as i64, ..Default::default() }).await?);
    }
    let projects: Vec<Proj> = if studio {
        vec![
            Proj { title: "Northfield Coffee", subtitle: "A roastery brand built around the morning ritual", summary: "Identity, packaging and a café interior for a specialty roaster opening its first shop.", client: 0, year: 2026, role: "Brand, packaging, interiors", duration: "5 months", industry: "Food & drink", tools: "Figma, Illustrator, Glyphs", cats: &[0, 2, 4], services: &[0, 2, 4], images: &["1523726491678-bf852e717f6a", "1497366216548-37526070297c", "1524758631624-e2822e304c36"], status: "published", featured: true, accent: "#7a4b2a", layout: "case_study",
                body: &["Northfield had roasted for restaurants for ten years before opening a café of its own. They asked us for a brand that felt as considered as the coffee — without the clichés of the category.", "We built the identity around the hour before work: warm paper, a custom serif drawn from old shipping labels, and a palette taken from the roast curve itself. The packaging uses one bag for every coffee, with the origin printed on a tear-off band.", "The café opened in March. Wholesale enquiries doubled in the first quarter."],
                metrics: &[("2×", "wholesale enquiries in three months"), ("38%", "of café customers buy beans to take home"), ("1", "bag design for every coffee")] },
            Proj { title: "Pebble Bank", subtitle: "A calmer way to save", summary: "Brand refresh and a new website for a savings app with 400,000 members.", client: 1, year: 2025, role: "Brand, website", duration: "4 months", industry: "Finance", tools: "Figma, Webflow, After Effects", cats: &[0, 1], services: &[0, 1], images: &["1559028012-481c04fa702d", "1551650975-87deedd944c3", "1512941937669-90a1b58e7e9c"], status: "published", featured: true, accent: "#2f6f62", layout: "case_study",
                body: &["Pebble's members loved the app but its brand looked like every other bank. We softened the system: rounded type, illustrations of small everyday wins, and motion that slows down rather than celebrates.", "The new website explains how saving works in four short scrolls and loads in under a second."],
                metrics: &[("+27%", "sign-ups from the website"), ("0.8s", "largest contentful paint")] },
            Proj { title: "Kestrel Air — Summer", subtitle: "A campaign for a regional airline's new routes", summary: "Outdoor, social and print for eight new summer routes.", client: 4, year: 2025, role: "Concept, art direction, copy", duration: "8 weeks", industry: "Travel", tools: "Photoshop, InDesign", cats: &[3], services: &[3], images: &["1506744038136-46273834b3fb", "1470071459604-3b5ec3a7fe05"], status: "published", featured: false, accent: "", layout: "gallery",
                body: &["“Closer than you think” paired each new destination with the travel time from home, set big over the view you'd arrive to."],
                metrics: &[("8", "routes launched"), ("91%", "average load factor in July")] },
            Proj { title: "Lumen Health clinics", subtitle: "Wayfinding for twelve family clinics", summary: "Signage and wayfinding that makes a nervous visit a little easier.", client: 2, year: 2024, role: "Wayfinding, signage", duration: "7 months", industry: "Healthcare", tools: "Illustrator, SketchUp", cats: &[4], services: &[4], images: &["1487958449943-2429e8be8625", "1503387762-592deb58ef4e"], status: "published", featured: false, accent: "#3a6ea5", layout: "case_study",
                body: &["We spent two weeks in waiting rooms before drawing anything. The system that came out of it uses colour for floors, plain words for rooms, and pictograms tested with children."] , metrics: &[] },
            Proj { title: "Maple & Moss eyewear", subtitle: "Cases and boxes for a small eyewear brand", summary: "A packaging range that ships flat and looks good on any shelf.", client: 6, year: 2024, role: "Packaging", duration: "10 weeks", industry: "Retail", tools: "Illustrator, ArtiosCAD", cats: &[2], services: &[2], images: &["1572635196237-14b3f281503f", "1542291026-7eec264c27ff"], status: "published", featured: false, accent: "", layout: "gallery",
                body: &["Every box is cut from one sheet with no glue, printed in two colours, and doubles as the case you keep your glasses in."], metrics: &[] },
            Proj { title: "Orchard Labs (in progress)", subtitle: "Design system for a developer platform", summary: "Under NDA until launch — ask us for the password.", client: 3, year: 2026, role: "Design system", duration: "Ongoing", industry: "Technology", tools: "Figma, Storybook", cats: &[1], services: &[1], images: &["1517694712202-14dd9538aa97", "1498050108023-c5249f4df085"], status: "protected", featured: false, accent: "#5b3cc4", layout: "case_study",
                body: &["A component library, design tokens and documentation for a developer platform launching later this year."], metrics: &[] },
        ]
    } else {
        vec![
            Proj { title: "Pebble — saving, simplified", subtitle: "Redesigning a savings app for people who hate budgeting", summary: "A ground-up redesign of the Pebble app: onboarding, goals and the weekly check-in.", client: 0, year: 2026, role: "Lead product designer", duration: "6 months", industry: "Finance", tools: "Figma, Maze, Principle", cats: &[0, 1], services: &[0], images: &["1551650975-87deedd944c3", "1512941937669-90a1b58e7e9c", "1559028012-481c04fa702d"], status: "published", featured: true, accent: "#2f6f62", layout: "case_study",
                body: &["Pebble's members kept their accounts but stopped opening the app. Interviews told us why: every screen asked them to do something.", "We rebuilt the app around one weekly check-in. Goals became small, visual and forgiving; the home screen shows progress instead of tasks. I led design across a team of three and ran fortnightly tests with members.", "Six months after launch, weekly active members are up by half and support tickets about 'where is my money' have nearly disappeared."],
                metrics: &[("+52%", "weekly active members"), ("−70%", "support tickets about balances"), ("4.8★", "App Store rating")] },
            Proj { title: "Lumen patient portal", subtitle: "Booking, results and messages for 300,000 patients", summary: "Making a hospital's patient portal usable for people who are unwell, tired or worried.", client: 1, year: 2025, role: "Product designer", duration: "9 months", industry: "Healthcare", tools: "Figma, Dovetail", cats: &[0, 2, 4], services: &[0, 2], images: &["1586717791821-3f44a563fa4c", "1545235617-9465d2a55698"], status: "published", featured: true, accent: "#3a6ea5", layout: "case_study",
                body: &["We designed for the worst moment: reading test results alone at night. Plain language, no red unless it matters, and a clear next step on every page."],
                metrics: &[("3×", "online bookings"), ("AA", "WCAG 2.2 throughout")] },
            Proj { title: "Orchard design system", subtitle: "One library for four product teams", summary: "Tokens, 60 components and a documentation site adopted by every team in a year.", client: 2, year: 2025, role: "Design systems lead", duration: "12 months", industry: "Technology", tools: "Figma, Storybook, Style Dictionary", cats: &[3], services: &[1], images: &["1561070791-2526d30994b5", "1558655146-d09347e92766"], status: "published", featured: false, accent: "#5b3cc4", layout: "case_study",
                body: &["Four teams, four button styles. We audited every screen, agreed tokens with engineering, and shipped components one team at a time."],
                metrics: &[("60", "components"), ("4/4", "teams adopted it")] },
            Proj { title: "Kestrel check-in kiosk", subtitle: "Faster airport check-in", summary: "Redesigning a self-service kiosk flow used 20,000 times a day.", client: 3, year: 2024, role: "UX designer", duration: "4 months", industry: "Travel", tools: "Figma, ProtoPie", cats: &[0, 4], services: &[0], images: &["1531403009284-440f080d1e12", "1497366216548-37526070297c"], status: "published", featured: false, accent: "", layout: "case_study",
                body: &["We timed two hundred check-ins with a stopwatch and cut the flow from nine screens to four."], metrics: &[("−41s", "average check-in time")] },
            Proj { title: "Tidewater booking", subtitle: "A hotel booking flow that converts", summary: "Under NDA — enter the password you were given.", client: 4, year: 2026, role: "Lead designer", duration: "3 months", industry: "Hospitality", tools: "Figma", cats: &[2], services: &[0, 2], images: &["1493809842364-78817add7ffb", "1555041469-a586c61ea9bc"], status: "protected", featured: false, accent: "#9a6b3f", layout: "case_study",
                body: &["A new booking flow for a group of coastal hotels: dates first, rooms with real photos, and the total price from the start."], metrics: &[("+19%", "direct bookings")] },
            Proj { title: "Side project: Field Notes app", subtitle: "A tiny journaling app for walks", summary: "A weekend project that became an app 8,000 people use.", client: usize::MAX, year: 2024, role: "Design and front-end", duration: "Weekends", industry: "Personal", tools: "SwiftUI, Figma", cats: &[1], services: &[], images: &["1470071459604-3b5ec3a7fe05", "1506744038136-46273834b3fb"], status: "published", featured: false, accent: "", layout: "minimal",
                body: &["One button to start a walk, one to add a note. Everything else is a map."], metrics: &[] },
        ]
    };
    let mut made = 0;
    let mut ids = Vec::new();
    for (i, p) in projects.iter().enumerate() {
        let images: Vec<String> = p.images.iter().map(|id| img(id)).collect();
        let input = rules::ProjectInput {
            title: p.title.into(),
            subtitle: p.subtitle.into(),
            summary: p.summary.into(),
            body: p.body.join("\n\n"),
            body_html: p.body.iter().map(|b| format!("<p>{}</p>", esc(b))).collect(),
            cover: images.first().cloned().unwrap_or_default(),
            gallery: images.iter().skip(1).cloned().collect(),
            client_id: clients.get(p.client).cloned(),
            client_name: if p.client == usize::MAX { String::new() } else { client_names.get(p.client).map(|c| c.0.to_string()).unwrap_or_default() },
            year: p.year,
            duration: p.duration.into(),
            role: p.role.into(),
            industry: p.industry.into(),
            tools: rules::list(p.tools, 30, 40),
            metrics: p.metrics.iter().map(|(v, l)| rules::Metric { value: v.to_string(), label: l.to_string() }).collect(),
            credits: if studio { vec![rules::Credit { name: "Iris Calder".into(), role: "Creative director".into(), url: String::new() }, rules::Credit { name: if i % 2 == 0 { "Theo Marsh" } else { "Noor Haddad" }.into(), role: "Designer".into(), url: String::new() }] } else { Vec::new() },
            category_ids: p.cats.iter().filter_map(|c| cats.get(*c).cloned()).collect(),
            service_ids: p.services.iter().filter_map(|c| services.get(*c).cloned()).collect(),
            accent: p.accent.into(),
            layout: p.layout.into(),
            featured: p.featured,
            status: p.status.into(),
            password: if p.status == "protected" { "preview".into() } else { String::new() },
            sort_order: i as i64,
            ..Default::default()
        };
        let id = repo::save_project(db, None, &input).await?;
        // Some views, newest projects most.
        let _ = sqlx::query("UPDATE portfolio_projects SET views = ? WHERE id = ?").bind(((projects.len() - i) * 137 + 40) as i64).bind(&id).execute(db).await;
        ids.push(id);
        made += 1;
    }
    let quotes: &[(&str, &str, &str, &str, i64, usize)] = if studio {
        &[
            ("They listened more than any agency we've worked with, and the brand they made still feels like us two years on.", "Hannah Price", "Founder", "Northfield Coffee", 5, 0),
            ("Calm, clear and on time. Our sign-ups went up the week the new site launched.", "Marcus Lee", "Head of Marketing", "Pebble Bank", 5, 1),
            ("Patients tell us they can find their way without asking. That's the whole brief, delivered.", "Dr. Amira Saleh", "Operations Director", "Lumen Health", 5, 3),
        ]
    } else {
        &[
            ("Lena turned a messy brief into the clearest product we've ever shipped. She makes the whole team better.", "Marcus Lee", "VP Product", "Pebble Bank", 5, 0),
            ("Rare mix of research rigour and lovely craft. Our patients noticed the difference immediately.", "Dr. Amira Saleh", "Digital Lead", "Lumen Health", 5, 1),
            ("The design system Lena led is the reason four teams now ship consistent UI without arguing.", "Sam Okoye", "Engineering Manager", "Orchard Labs", 5, 2),
        ]
    };
    for (i, (q, a, r, c, rating, proj)) in quotes.iter().enumerate() {
        let tid = repo::save_testimonial(db, None, &rules::TestimonialInput {
            quote: q.to_string(), author: a.to_string(), role: r.to_string(), company: c.to_string(), photo: format!("{IMG}{}?w=200&h=200&fit=crop", ["1438761681033-6461ffad8d80", "1507003211169-0a1dd7228f2d", "1544005313-94ddf0286df2"][i % 3]),
            rating: *rating, project_id: ids.get(*proj).cloned(), featured: true, sort_order: i as i64,
        }).await?;
        if let Some(pid) = ids.get(*proj) {
            let _ = sqlx::query("UPDATE portfolio_projects SET testimonial_id = ? WHERE id = ?").bind(&tid).bind(pid).execute(db).await;
        }
    }
    if studio {
        for (i, (n, role, photo, bio)) in [
            ("Iris Calder", "Founder & creative director", "1534528741775-53994a69daeb", "Iris started Harbour Lane in 2014 after a decade at larger agencies. She leads every project's first workshop."),
            ("Theo Marsh", "Design director", "1519085360753-af0119f7cbe7", "Theo looks after identity and packaging, and draws most of the studio's custom lettering."),
            ("Noor Haddad", "Digital lead", "1580489944761-15a19d654956", "Noor designs and builds the studio's websites, and makes sure they stay fast after launch."),
            ("Felix Grant", "Strategist", "1506794778202-cad84cf45f1d", "Felix runs research and writes the words — names, taglines and the occasional manifesto."),
        ].iter().enumerate() {
            repo::save_member(db, None, &rules::MemberInput { name: n.to_string(), role: role.to_string(), bio: bio.to_string(), photo: format!("{IMG}{photo}?w=600&h=750&fit=crop"), status: "active".into(), sort_order: i as i64, ..Default::default() }).await?;
        }
        for (kind, title, org, from) in [
            ("award", "Gold, Brand Identity", "Lisbon Design Awards", "2025"),
            ("award", "Shortlist, Packaging", "International Pentawards", "2024"),
            ("press", "“Ten studios to watch”", "Design Weekly", "2025"),
            ("talk", "Designing for the morning ritual", "Brand New Conference", "2026-04"),
        ] {
            repo::save_entry(db, None, &rules::EntryInput { kind: kind.into(), title: title.into(), organisation: org.into(), start: from.into(), ..Default::default() }).await?;
        }
    } else {
        for (kind, title, org, loc, from, to, cur, desc) in [
            ("work", "Lead Product Designer", "Pebble Bank", "Lisbon", "2024-02", "", true, "Leading design for the savings app across onboarding, goals and growth. Team of three designers."),
            ("work", "Senior Product Designer", "Lumen Health", "Remote", "2021-05", "2024-01", false, "Designed the patient portal and the clinicians' scheduling tools."),
            ("work", "Product Designer", "Orchard Labs", "Berlin", "2018-09", "2021-04", false, "Built and ran the design system used by four product teams."),
            ("education", "MA Interaction Design", "Royal College of Art", "London", "2016", "2018", false, ""),
            ("education", "BA Graphic Design", "University of Porto", "Porto", "2012", "2015", false, ""),
            ("award", "Best Fintech App", "UX Design Awards", "", "2025", "", false, "For the Pebble redesign."),
            ("talk", "Designing for the worst moment", "UX Lisbon", "Lisbon", "2025-05", "", false, ""),
            ("certification", "Accessibility (CPACC)", "IAAP", "", "2023", "", false, ""),
        ] {
            repo::save_entry(db, None, &rules::EntryInput { kind: kind.into(), title: title.into(), organisation: org.into(), location: loc.into(), start: from.into(), end: to.into(), current: cur, description: desc.into(), ..Default::default() }).await?;
        }
        for (i, (n, g, l)) in [("Product strategy", "Design", 90), ("Interaction design", "Design", 95), ("Prototyping", "Design", 90), ("User research", "Research", 80), ("Usability testing", "Research", 85), ("Figma", "Tools", 0), ("SwiftUI", "Tools", 0), ("HTML & CSS", "Tools", 0), ("Portuguese", "Languages", 0), ("English", "Languages", 0)].iter().enumerate() {
            repo::save_skill(db, None, &rules::SkillInput { name: n.to_string(), group: g.to_string(), level: *l, sort_order: i as i64 }).await?;
        }
    }
    for (n, e, co, budget, msg, status) in [
        ("Ellen Brooks", "ellen.brooks@example.com", "Brightwater Co-op", "$15k – $50k", "We're a food co-op opening a second shop and need a brand that grows with us. Can we talk next week?", "new"),
        ("Raj Patel", "raj.patel@example.com", "Fenwick Robotics", "$50k+", "Looking for help with a design system for our operator dashboard. Timeline: start in January.", "replied"),
    ] {
        let id = repo::insert_enquiry(db, &rules::EnquiryInput { name: n.into(), email: e.into(), company: co.into(), budget: budget.into(), message: msg.into(), services: vec![service_defs[0].0.to_string()], ..Default::default() }).await?;
        repo::update_enquiry(db, &id, status, "").await?;
    }
    let (name, headline, about, portrait, location, founded, email) = if studio {
        ("Harbour Lane", "An independent brand and digital studio", "Harbour Lane is a studio of eleven designers, writers and developers in Lisbon. We make brands, websites, packaging and spaces for companies that care how things feel.\n\nWe work with a small number of clients at a time, and we stay with most of them for years.", "1534528741775-53994a69daeb", "Lisbon, Portugal", "2014", "studio@example.com")
    } else {
        ("Lena Hart", "Product designer making calm, useful software", "I'm a product designer in Lisbon with ten years of experience across fintech, health and developer tools. I like hard problems with real people on the other end of them.\n\nRight now I lead design at Pebble. I take on one or two freelance projects a year — mostly design systems and UX audits.", "1544005313-94ddf0286df2", "Lisbon, Portugal", "2015", "lena@example.com")
    };
    settings.set_many(db, &[
        (rules::SETTING_ENABLED, serde_json::json!(true)),
        (rules::SETTING_MODE, serde_json::json!(if studio { "studio" } else { "personal" })),
        (rules::SETTING_NAME, serde_json::json!(name)),
        (rules::SETTING_HEADLINE, serde_json::json!(headline)),
        (rules::SETTING_INTRO, serde_json::json!(if studio { "We help ambitious companies look and feel like themselves — in print, on screen and in the room." } else { "Ten years designing products for finance, health and developer tools. Currently leading design at Pebble." })),
        (rules::SETTING_ABOUT, serde_json::json!(about)),
        (rules::SETTING_PORTRAIT, serde_json::json!(format!("{IMG}{portrait}?w=600&h=600&fit=crop"))),
        (rules::SETTING_LOCATION, serde_json::json!(location)),
        (rules::SETTING_FOUNDED, serde_json::json!(founded)),
        (rules::SETTING_EMAIL, serde_json::json!(email)),
        (rules::SETTING_AVAILABILITY, serde_json::json!(if studio { "available" } else { "limited" })),
        (rules::SETTING_AVAILABILITY_NOTE, serde_json::json!(if studio { "Booking projects from March" } else { "One freelance project from spring" })),
        (rules::SETTING_SOCIALS, serde_json::json!(rules::socials_json(&if studio {
            vec![("instagram".to_string(), "https://instagram.com/".to_string()), ("linkedin".to_string(), "https://linkedin.com/".to_string()), ("behance".to_string(), "https://behance.net/".to_string())]
        } else {
            vec![("linkedin".to_string(), "https://linkedin.com/".to_string()), ("dribbble".to_string(), "https://dribbble.com/".to_string()), ("github".to_string(), "https://github.com/".to_string())]
        }))),
        (rules::SETTING_ENQUIRIES, serde_json::json!(true)),
    ]).await?;
    Ok(made)
}
