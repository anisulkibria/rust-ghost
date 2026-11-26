//! "Load a sample show": *Open Workshop*, a conversation show about people
//! who make things by hand — two hosts, four guests, two seasons with a
//! trailer, a members-only bonus and one episode scheduled for next week,
//! chapters and a transcript, two sponsors, reviews, two inbox messages and
//! six weeks of downloads across apps and countries, so every screen and
//! the public pages can be seen working at once.
//!
//! The audio is SoundHelix's royalty-free example music, linked where it is
//! hosted. Inbox senders are `@example.com` and are never emailed.

use super::*;

const IMG: &str = "https://images.unsplash.com/photo-";
const AUDIO: &str = "https://www.soundhelix.com/examples/mp3/SoundHelix-Song-";

struct Person {
    name: &'static str,
    title: &'static str,
    photo: &'static str,
    bio: &'static str,
}

const PEOPLE: &[Person] = &[
    Person { name: "Ana Ferreira", title: "Furniture maker and host", photo: "1494790108377-be9c29b29330",
        bio: "Ana builds chairs in a converted dairy outside Porto. She started Open Workshop to ask other makers the questions she wishes someone had answered for her." },
    Person { name: "Ben Okafor", title: "Industrial designer and co-host", photo: "1500648767791-00dcc994a43e",
        bio: "Ben designs kitchen tools by day and restores old bicycles by night. He keeps the conversation honest about money, time and failure." },
    Person { name: "Mei Lin Zhou", title: "Ceramicist", photo: "1438761681033-6461ffad8d80",
        bio: "Mei Lin throws tableware for restaurants and teaches wheel classes in Melbourne." },
    Person { name: "Tomás García", title: "Letterpress printer", photo: "1507003211169-0a1dd7228f2d",
        bio: "Tomás runs a two-press print shop in Valencia and prints everything from wedding invitations to poetry broadsides." },
    Person { name: "Priya Raman", title: "Weaver and textile artist", photo: "1573496359142-b8d87734a5a2",
        bio: "Priya weaves on a 1950s floor loom and dyes her own yarn with plants from her garden." },
    Person { name: "Jonas Weber", title: "Knife maker", photo: "1506794778202-cad84cf45f1d",
        bio: "Jonas forges kitchen knives in the Black Forest, one blade at a time." },
];

struct Ep {
    title: &'static str,
    season: i64,
    number: i64,
    kind: &'static str,
    access: &'static str,
    days_ago: i64,
    song: u8,
    secs: i64,
    guest: Option<usize>,
    summary: &'static str,
    notes: &'static [&'static str],
    chapters: &'static [(i64, &'static str)],
    featured: bool,
}

const EPISODES: &[Ep] = &[
    Ep { title: "Welcome to Open Workshop", season: 1, number: 0, kind: "trailer", access: "public", days_ago: 200, song: 1, secs: 140, guest: None, featured: false,
        summary: "Two makers, one microphone, and a long list of questions about doing things by hand.",
        notes: &["Ana and Ben introduce the show: who they are, why they started it, and what to expect every other Tuesday."], chapters: &[] },
    Ep { title: "Clay remembers everything", season: 1, number: 1, kind: "full", access: "public", days_ago: 186, song: 2, secs: 2710, guest: Some(2), featured: false,
        summary: "Mei Lin Zhou on throwing for restaurants, pricing by the plate, and why she keeps her failures on a shelf.",
        notes: &["Mei Lin has made over forty thousand plates for restaurants across Australia. We talk about what chefs actually want, how she prices a commission, and the shelf of cracked pots she refuses to throw away.", "Mentioned: the studio's open wheel nights, and the glaze recipe book she's been writing for six years."],
        chapters: &[(0, "Welcome"), (95, "Meet Mei Lin"), (610, "Throwing for chefs"), (1420, "Pricing by the plate"), (2210, "The shelf of failures"), (2580, "Where to find her")] },
    Ep { title: "The slowest press in Valencia", season: 1, number: 2, kind: "full", access: "public", days_ago: 172, song: 3, secs: 3120, guest: Some(3), featured: false,
        summary: "Tomás García on letterpress, patience and saying no to rush jobs.",
        notes: &["Tomás prints on two presses that are older than his parents. He explains why he will never take a job due tomorrow, and how a printer's apprenticeship works in 2026."],
        chapters: &[(0, "Welcome"), (140, "Two old presses"), (1300, "Saying no"), (2400, "Apprentices")] },
    Ep { title: "Dyeing with the garden", season: 1, number: 3, kind: "full", access: "public", days_ago: 158, song: 4, secs: 2890, guest: Some(4), featured: true,
        summary: "Priya Raman grows her colours: madder, weld, indigo and a lot of patience.",
        notes: &["Priya walks us through a year of plant dyes, from planting madder roots that take three years to mature to the indigo vat that lives in her garage.", "Her loom is from 1953; we hear it at about the forty-minute mark."],
        chapters: &[(0, "Welcome"), (180, "A garden of colour"), (1200, "Three years for madder"), (2400, "The 1953 loom")] },
    Ep { title: "Bonus: Questions from the workbench", season: 1, number: 0, kind: "bonus", access: "members", days_ago: 150, song: 5, secs: 1840, guest: None, featured: false,
        summary: "For members: Ana and Ben answer your questions about tools, pricing and burnout.",
        notes: &["Thank you for supporting the show. This month's listener questions: which first tools are worth buying, how to raise prices without losing customers, and what to do when making stops being fun."], chapters: &[] },
    Ep { title: "One blade at a time", season: 2, number: 4, kind: "full", access: "public", days_ago: 60, song: 6, secs: 3305, guest: Some(5), featured: true,
        summary: "Jonas Weber forges kitchen knives in the Black Forest — and has a two-year waiting list.",
        notes: &["Season two opens at the anvil. Jonas talks about steel, heat treatment, and what it means to have a two-year waiting list you never wanted.", "We also hear about the knife he refuses to sell."],
        chapters: &[(0, "Season two"), (120, "The forge"), (900, "Steel and heat"), (2100, "A two-year waiting list"), (3000, "The knife he won't sell")] },
    Ep { title: "How we price our work", season: 2, number: 5, kind: "full", access: "public", days_ago: 46, song: 7, secs: 2450, guest: None, featured: false,
        summary: "Just the two of us, with spreadsheets: what our hours are really worth.",
        notes: &["The episode listeners asked for most. Ana and Ben open their books — materials, hours, overheads — and argue about whether to charge by the piece or by the day."],
        chapters: &[(0, "Hello"), (300, "Ana's chair"), (1100, "Ben's whisk"), (1800, "By the piece or by the day?")] },
    Ep { title: "Repair is a design brief", season: 2, number: 6, kind: "full", access: "public", days_ago: 32, song: 8, secs: 2780, guest: Some(2), featured: false,
        summary: "Mei Lin returns to talk about kintsugi, repair cafés and designing things to be fixed.",
        notes: &["Mei Lin is back, this time with a box of broken plates. We talk about visible mending, the repair café she runs on Saturdays, and why designing for repair changes everything upstream."], chapters: &[] },
    Ep { title: "Teaching what you know", season: 2, number: 7, kind: "full", access: "public", days_ago: 18, song: 9, secs: 3010, guest: Some(3), featured: false,
        summary: "Tomás on running workshops without losing the shop.",
        notes: &["Workshops pay Tomás's rent in winter. He shares how he plans a class, what he charges, and the mistakes he made in the first year."], chapters: &[] },
    Ep { title: "Studio tours: four small workshops", season: 2, number: 8, kind: "full", access: "public", days_ago: 4, song: 10, secs: 2620, guest: None, featured: true,
        summary: "We visit four tiny studios and ask how they make the space work.",
        notes: &["A garage, a shipping container, a spare bedroom and a corner of a bakery. Four makers show us how they fit a working studio into very little room."],
        chapters: &[(0, "Intro"), (240, "The garage"), (900, "The container"), (1500, "The spare room"), (2100, "The bakery corner")] },
    Ep { title: "The business of commissions", season: 2, number: 9, kind: "full", access: "public", days_ago: -7, song: 11, secs: 2900, guest: Some(4), featured: false,
        summary: "Priya on contracts, deposits and the commission that nearly broke her.",
        notes: &["Coming next week: Priya returns to talk about the business side of commissioned work."], chapters: &[] },
];

const TRANSCRIPT: &str = "WEBVTT\n\n00:00:00.000 --> 00:00:06.000\n<v Ana Ferreira>Welcome back to Open Workshop. Season two starts today at a forge in the Black Forest.\n\n00:00:06.000 --> 00:00:12.000\n<v Ben Okafor>It's loud, it's hot, and Jonas has promised not to set anything on fire.\n\n00:02:00.000 --> 00:02:09.000\n<v Jonas Weber>I started with a broken lawnmower blade and a barbecue. That's how most of us start, honestly.\n\n00:02:09.000 --> 00:02:15.000\n<v Ana Ferreira>And now there's a two-year waiting list.\n\n00:15:00.000 --> 00:15:12.000\n<v Jonas Weber>The steel matters less than people think. The heat treatment is everything. You can ruin the best steel in the world in ten seconds.\n\n00:35:00.000 --> 00:35:10.000\n<v Jonas Weber>I never wanted a waiting list. A waiting list means people are waiting, and waiting is not a nice thing to ask of someone.\n\n00:50:00.000 --> 00:50:08.000\n<v Ben Okafor>So tell us about the knife you won't sell.";

const REVIEWS: &[(&str, i64, &str, &str, &str)] = &[
    ("Hannah K.", 5, "My favourite thing on a Tuesday", "Every episode makes me want to go and make something. The pricing episode alone paid for itself ten times over.", "apple"),
    ("woodshop_dave", 5, "Honest and warm", "Finally a show about craft that talks about money and burnout as well as the beautiful bits.", "spotify"),
    ("Lucía", 5, "", "Mei Lin's episode on repair changed how I think about my own work. Thank you both.", "email"),
];

pub(crate) async fn sample_load(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    match seed_sample_podcast(&state.db, &state.settings).await {
        Ok(0) => flash_to(BASE, "error", "There is already a show here, so nothing was added."),
        Ok(n) => crate::tools::tool_saved(&state, BASE, &format!("Sample show loaded with {n} episodes, ready to edit. The podcast pages are switched on.")),
        Err(e) => {
            tracing::error!("podcast: sample failed: {}", e.message());
            flash_to(BASE, "error", "The sample show could not be loaded.")
        }
    }
}

/// Also the `vuuut seed-podcast` command. Does nothing when a show exists.
pub async fn seed_sample_podcast(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache) -> vuuut_core::Result<usize> {
    if !repo::shows(db, false).await.is_empty() {
        return Ok(0);
    }
    let mut people = Vec::new();
    for p in PEOPLE {
        let id = repo::save_person(db, None, &rules::PersonInput {
            name: p.name.into(), title: p.title.into(), bio: p.bio.into(), photo: format!("{IMG}{}?w=400&h=400&fit=crop", p.photo), ..Default::default()
        }).await?;
        people.push(id);
    }
    let show_id = repo::save_show(db, None, &rules::ShowInput {
        title: "Open Workshop".into(),
        subtitle: "Conversations with people who make things by hand".into(),
        description: "Every other Tuesday, furniture maker Ana Ferreira and designer Ben Okafor sit down with a potter, a printer, a weaver or a smith to talk about the work — the craft, the money, the mistakes and the joy of making things by hand.\n\nNew episodes every other Tuesday. Members get bonus episodes and an ad-free feed.".into(),
        artwork: format!("{IMG}1478737270239-2f02b77fc618?w=1400&h=1400&fit=crop"),
        author: "Ana Ferreira & Ben Okafor".into(),
        owner_name: "Ana Ferreira".into(),
        owner_email: "hello@example.com".into(),
        category: "Arts > Design".into(),
        category2: "Leisure > Crafts".into(),
        language: "en".into(),
        show_type: "episodic".into(),
        copyright: "© 2026 Open Workshop".into(),
        links: vec![
            ("apple".into(), "https://podcasts.apple.com/".into()),
            ("spotify".into(), "https://open.spotify.com/".into()),
            ("youtube".into(), "https://www.youtube.com/".into()),
            ("overcast".into(), "https://overcast.fm/".into()),
            ("pocketcasts".into(), "https://pocketcasts.com/".into()),
        ],
        funding_url: "https://example.com/support".into(),
        funding_label: "Become a member".into(),
        tint: "amber".into(),
        status: "active".into(),
        hosts: vec![(people[0].clone(), "host".into()), (people[1].clone(), "cohost".into())],
        ..Default::default()
    }).await?;
    let sponsor_a = repo::save_sponsor(db, None, &rules::SponsorInput { name: "Grain & Gauge Tools".into(), tagline: "Hand tools made to be sharpened, not replaced".into(), url: "https://example.com/grain".into(), promo_code: "WORKSHOP15".into(), active: true, ..Default::default() }).await?;
    let sponsor_b = repo::save_sponsor(db, None, &rules::SponsorInput { name: "Kiln House Supply".into(), tagline: "Clay, glaze and kiln parts, shipped fast".into(), url: "https://example.com/kiln".into(), promo_code: "OPEN10".into(), active: true, ..Default::default() }).await?;
    let now = Utc::now();
    let mut made = Vec::new();
    for (i, ep) in EPISODES.iter().enumerate() {
        let mut credits = vec![(people[0].clone(), "host".to_string()), (people[1].clone(), "cohost".to_string())];
        if let Some(g) = ep.guest {
            credits.push((people[g].clone(), "guest".into()));
        }
        let notes_html: String = ep.notes.iter().map(|p| format!("<p>{}</p>", esc(p))).collect();
        let input = rules::EpisodeInput {
            show_id: show_id.clone(),
            title: ep.title.into(),
            summary: ep.summary.into(),
            notes: ep.notes.join("\n\n"),
            notes_html,
            audio_url: format!("{AUDIO}{}.mp3", ep.song),
            audio_bytes: ep.secs * 16_000,
            duration: ep.secs,
            season: ep.season,
            number: ep.number,
            episode_type: ep.kind.into(),
            access: ep.access.into(),
            status: "published".into(),
            published_at: vuuut_core::dates::to_db_string(&(now - Duration::days(ep.days_ago) - Duration::hours(3))),
            transcript: if ep.number == 4 { TRANSCRIPT.into() } else { String::new() },
            chapters: ep.chapters.iter().map(|(s, t)| rules::Chapter { start: *s, title: t.to_string(), url: String::new() }).collect(),
            featured: ep.featured,
            people: credits,
            sponsors: if i % 3 == 1 { vec![sponsor_a.clone()] } else if i % 3 == 2 { vec![sponsor_b.clone()] } else { Vec::new() },
            ..Default::default()
        };
        let id = repo::save_episode(db, None, &input).await?;
        made.push((id, ep));
    }
    for (author, rating, title, body, source) in REVIEWS {
        repo::save_review(db, None, &show_id, &rules::ReviewInput { author: author.to_string(), rating: *rating, title: title.to_string(), body: body.to_string(), source: source.to_string(), featured: true }).await?;
    }
    for (kind, name, topic, msg) in [
        ("guest", "Sam Rivera", "Glassblowing in a shared studio", "I run a shared glass studio in Leeds with eleven members. Happy to talk about co-operative studios, splitting the cost of a furnace, and teaching beginners."),
        ("question", "Ruth Adeyemi", "Insurance for a home studio?", "Love the show. How do you both handle insurance for a workshop at home? Nobody seems to talk about it."),
    ] {
        let input = rules::RequestInput { kind: kind.into(), name: name.into(), email: format!("{}@example.com", vuuut_core::directory::slugify(name).replace('-', ".")), topic: topic.into(), message: msg.into(), link: String::new() };
        repo::insert_request(db, Some(&show_id), &input).await?;
    }
    // Six weeks of downloads: each episode's first days are its biggest.
    const APPS: &[(&str, u32)] = &[("Apple Podcasts", 38), ("Spotify", 30), ("Overcast", 8), ("Pocket Casts", 7), ("Web player", 6), ("YouTube", 5), ("Castro", 3), ("Other", 3)];
    const COUNTRIES: &[(&str, u32)] = &[("US", 34), ("GB", 18), ("DE", 9), ("AU", 9), ("PT", 7), ("ES", 6), ("CA", 6), ("NL", 4), ("BD", 3), ("IN", 4)];
    let pick = |table: &[(&'static str, u32)], n: u64| -> &'static str {
        let total: u32 = table.iter().map(|(_, w)| w).sum();
        let mut x = (n % total as u64) as u32;
        for (v, w) in table {
            if x < *w {
                return v;
            }
            x -= w;
        }
        table[0].0
    };
    let mut seq: u64 = 7;
    for (id, ep) in &made {
        if ep.days_ago < 0 {
            continue;
        }
        let span = ep.days_ago.min(45);
        for d in 0..=span {
            let age = ep.days_ago - (span - d);
            let base: i64 = if ep.access != "public" { 3 } else { 4 };
            let n = (base + (40 / (age + 1).max(1)).min(40) + (seq % 5) as i64) as u64;
            let day = (now - Duration::days(span - d)).format("%Y-%m-%d").to_string();
            for k in 0..n {
                seq = seq.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let app = pick(APPS, seq >> 33);
                let country = pick(COUNTRIES, (seq >> 17) ^ k);
                repo::record_download(db, &repo::Download {
                    episode_id: id, show_id: &show_id, day: &day, listener: &format!("sample{seq:x}"), app, country,
                    source: if app == "Web player" { "web" } else if ep.access != "public" { "private" } else { "feed" },
                }).await;
            }
        }
    }
    settings.set_many(db, &[
        (rules::SETTING_ENABLED, serde_json::json!(true)),
        (rules::SETTING_NOTIFY_EMAIL, serde_json::json!("")),
    ]).await?;
    Ok(made.len())
}
