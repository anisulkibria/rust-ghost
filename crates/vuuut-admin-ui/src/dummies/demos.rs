//! Theme demos: what a bundled theme built for a platform tool needs beyond
//! the tool's own content to look finished — a journal written in the
//! business's voice, the pages its menu points at, and the theme's settings.
//!
//! * [`Demo::Studio`] dresses Atrium as a wellness studio in Bath.
//! * [`Demo::Academy`] dresses Lantern as an online academy.
//! * [`Demo::Documentation`] gives Handbook an ordered set of guides.
//!
//! Everything made here is recorded in the same batch as the courses or
//! services, so the demo is removed as one. Theme settings are only filled
//! where empty and are not recorded: they are harmless, and the owner may
//! have kept them.

use super::*;
use vuuut_core::dummies::{document, Block, Block::*, Demo};

mod handbook;

const IMG: &str = "https://images.unsplash.com/photo-";

struct Post {
    title: &'static str,
    excerpt: &'static str,
    image: &'static str,
    /// The topical tag, besides the journal tag every post carries.
    topic: usize,
    days_ago: i64,
    featured: bool,
    body: &'static [Block],
}

struct Page {
    title: &'static str,
    slug: &'static str,
    excerpt: &'static str,
    menu: Menu,
    body: &'static [Block],
}

struct Set {
    /// Name, description, accent. The first is the journal itself.
    tags: &'static [(&'static str, &'static str, &'static str)],
    posts: &'static [Post],
    pages: &'static [Page],
    /// `(theme, key, value)`, filled only where empty.
    theme_settings: &'static [(&'static str, &'static str)],
}

/// Make the demo's content, pages and settings, record them in `batch`, and
/// add their menu links to `menu`. Returns what was made, by kind.
pub(super) async fn extras(
    db: &vuuut_db::Db,
    settings: &vuuut_db::SettingsCache,
    batch: &str,
    user_id: &str,
    demo: Demo,
    images: bool,
    menu: &mut Vec<(Menu, String, String)>,
) -> vuuut_core::Result<Vec<(Kind, i64)>> {
    let s = match demo {
        Demo::Studio => &STUDIO,
        Demo::Academy => &ACADEMY,
        Demo::Documentation => return handbook::generate(db, settings, batch, user_id, menu).await,
    };
    let mut made = Vec::new();

    // Tags: an existing public tag is used, not duplicated.
    let mut tag_ids: Vec<Option<String>> = Vec::new();
    let mut n = 0;
    for (name, description, accent) in s.tags {
        let slug = vuuut_core::slug::safe_string(name);
        if let Some(existing) = repos::tags::find_by_slug(db, &slug).await? {
            tag_ids.push((existing.visibility == "public").then_some(existing.id));
            continue;
        }
        let new = repos::tags::NewTag {
            name: name.to_string(),
            slug: Some(slug),
            description: Some(description.to_string()),
            visibility: "public".into(),
            accent_color: Some(accent.to_string()),
            ..Default::default()
        };
        let created = repos::tags::insert(db, &new).await?;
        repo::record(db, batch, Kind::Tags, &created.id).await?;
        tag_ids.push(Some(created.id));
        n += 1;
    }
    made.push((Kind::Tags, n));
    if let Some((journal, _, _)) = s.tags.first() {
        menu.push((Menu::Primary, journal.to_string(), format!("/tag/{}/", vuuut_core::slug::safe_string(journal))));
    }

    // The journal. A post whose address is taken is skipped.
    let mut n = 0;
    for post in s.posts {
        let slug = vuuut_core::slug::safe_string(post.title);
        if repos::posts::find_by_slug(db, &slug, "post", false).await?.is_some() {
            continue;
        }
        let tags: Vec<String> = [Some(0), Some(post.topic)].into_iter().flatten().filter_map(|i| tag_ids.get(i).cloned().flatten()).collect::<Vec<_>>();
        let mut tags_unique = Vec::new();
        for t in tags {
            if !tags_unique.contains(&t) {
                tags_unique.push(t);
            }
        }
        let lexical = document(post.body.iter().map(Block::node).collect());
        let published = chrono::Utc::now() - chrono::Duration::days(post.days_ago) - chrono::Duration::hours(3);
        let image = images.then(|| format!("{IMG}{}?w=2000&q=80", post.image));
        let new = content(post.title, Some(&slug), post.excerpt, &lexical, "post", true, Some(published), post.featured, image.as_deref(), user_id, tags_unique);
        let created = repos::posts::insert(db, &new, user_id).await?;
        repo::record(db, batch, Kind::Posts, &created.id).await?;
        n += 1;
    }
    made.push((Kind::Posts, n));

    // Pages: one that already exists keeps its content and is linked.
    let mut n = 0;
    for page in s.pages {
        let saved = match repos::posts::find_by_slug(db, page.slug, "page", false).await? {
            Some(existing) => existing,
            None => {
                let lexical = document(page.body.iter().map(Block::node).collect());
                let new = content(page.title, Some(page.slug), page.excerpt, &lexical, "page", true, None, false, None, user_id, Vec::new());
                let created = repos::posts::insert(db, &new, user_id).await?;
                repo::record(db, batch, Kind::Pages, &created.id).await?;
                n += 1;
                created
            }
        };
        if saved.status == "published" && saved.visibility == "public" {
            menu.push((page.menu, page.title.to_string(), format!("/{}/", page.slug)));
        }
    }
    made.push((Kind::Pages, n));

    fill_theme_settings(db, settings, demo.theme(), s.theme_settings).await?;
    Ok(made)
}

async fn fill_theme_settings(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache, theme: &str, fields: &[(&str, &str)]) -> vuuut_core::Result<()> {
    let mut all = match settings.get("custom_theme_settings") {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let mut mine = all.get(theme).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    let mut changed = false;
    for (key, value) in fields {
        if mine.get(*key).is_none_or(|v| v.is_null() || v.as_str() == Some("")) {
            mine.insert(key.to_string(), serde_json::json!(value));
            changed = true;
        }
    }
    if changed {
        all.insert(theme.to_string(), serde_json::Value::Object(mine));
        settings.set(db, "custom_theme_settings", &serde_json::Value::Object(all)).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Atrium: a wellness studio
// ---------------------------------------------------------------------------

static STUDIO: Set = Set {
    tags: &[
        ("Journal", "Notes from the studio: rest, skin, movement and the small habits that hold them together.", "#2f5d50"),
        ("Rest", "Massage, sleep and the art of switching off.", "#3f7a68"),
        ("Skin", "Gentle, honest skin care from our treatment rooms.", "#c4785a"),
        ("Movement", "Pilates, barre and moving well every day.", "#d39b4a"),
        ("Nourish", "Simple, lasting habits from our nutritionist.", "#7a6aa8"),
    ],
    posts: &[
        Post {
            title: "Why a massage works better when you book it before you need it",
            excerpt: "Tension builds quietly for weeks. A regular treatment catches it early — here is how often our therapists suggest coming in, and why.",
            image: "1540555700478-4be289fbecef",
            topic: 1,
            days_ago: 2,
            featured: true,
            body: &[
                P("Most people book a massage on the day their shoulders finally give up. It helps, of course — but by then the muscles have been guarding for weeks, and the first session is spent simply persuading them to let go."),
                P("Booked a little ahead, the same hour goes much further. Your therapist can work on what is actually there rather than undoing a month of desk posture in sixty minutes."),
                H("How often is often enough?"),
                L(&[
                    "Desk-bound and generally well: every four to six weeks keeps things moving.",
                    "Training for an event: every two to three weeks through the heavy block, then after race day.",
                    "Recovering from a busy season: two sessions close together, then settle into a rhythm.",
                ]),
                H("Make the most of the hour"),
                P("Arrive ten minutes early, drink water afterwards, and tell your therapist what you did last week as well as where it hurts. The more we know, the better we can help."),
            ],
        },
        Post {
            title: "A five-minute evening routine for calmer skin",
            excerpt: "Sofia's pared-back routine for skin that has had a long day: three steps, no gadgets, and one habit most of us skip.",
            image: "1515377905703-c4788e51af15",
            topic: 2,
            days_ago: 6,
            featured: false,
            body: &[
                P("Skin does most of its repair overnight, which makes the last five minutes before bed the most useful of the day. You do not need a twelve-step routine — you need a gentle, consistent one."),
                H("The routine"),
                L(&[
                    "Cleanse twice: an oil or balm to lift sunscreen and make-up, then a soft cream cleanser.",
                    "Treat once: one active, not three. Alternate nights if you use retinol and acids.",
                    "Seal it in: a simple moisturiser while skin is still slightly damp.",
                ]),
                P("The habit most of us skip? Washing the pillowcase. Change it twice a week and you will see the difference sooner than with any serum."),
            ],
        },
        Post {
            title: "Pilates for people who sit all day",
            excerpt: "Three mat exercises Daniel teaches in every beginners' class — and why they matter most if you work at a desk.",
            image: "1518611012118-696072aa579a",
            topic: 3,
            days_ago: 11,
            featured: true,
            body: &[
                P("Eight hours in a chair teaches the body some unhelpful lessons: hips that forget how to extend, a back that rounds forward, breathing that stays high in the chest. Pilates is a patient way to unlearn them."),
                H("Three to start with"),
                L(&[
                    "Bridge: lifts the hips and wakes up the glutes that sitting switches off.",
                    "Swimming: a gentle back extension that counters the desk slump.",
                    "Dead bug: deep core control without straining the neck.",
                ]),
                P("Ten minutes a day, three times a week, is enough to feel a difference within a month. Our small-group classes cap at ten so there is always time for a correction."),
            ],
        },
        Post {
            title: "Eating for steady energy, not perfect energy",
            excerpt: "Ella on the afternoon slump, why breakfast matters less than lunch, and the plate she suggests to almost everyone.",
            image: "1490645935967-10de6ba17061",
            topic: 4,
            days_ago: 17,
            featured: false,
            body: &[
                P("When clients ask for more energy, they usually mean fewer crashes. The goal is not a perfect diet — it is a steadier afternoon."),
                H("The everyday plate"),
                L(&[
                    "Half vegetables or salad, for volume and fibre.",
                    "A palm of protein, which keeps you full for longer.",
                    "A fist of slow carbohydrate — grains, potatoes, beans.",
                    "A thumb of fat, for flavour and to make it satisfying.",
                ]),
                P("Build lunch that way for a fortnight and notice four o'clock. If it still drags, book a consultation and we will look at sleep, timing and hydration together."),
            ],
        },
        Post {
            title: "What to expect at your first visit",
            excerpt: "From booking to the moment you leave: everything that happens at your first appointment, so you can simply arrive.",
            image: "1544161515-4ab6ce6db874",
            topic: 0,
            days_ago: 24,
            featured: false,
            body: &[
                P("A first appointment anywhere comes with small unknowns. Here is ours, step by step, so the only thing left to do is relax."),
                H("Before you arrive"),
                L(&[
                    "You will get a confirmation email with a link to reschedule or cancel — no phone calls needed.",
                    "A reminder arrives the day before.",
                    "If anything changes with your health, reply and let us know.",
                ]),
                H("On the day"),
                P("Come ten minutes early. We will offer tea, run through a short consultation and show you to your room. Afterwards there is no rush to leave — the lounge is yours for as long as you like."),
            ],
        },
        Post {
            title: "The case for a proper day off",
            excerpt: "Rest is a skill, and like any skill it improves with practice. A few ideas for a day that actually restores you.",
            image: "1499209974431-9dddcece7f88",
            topic: 1,
            days_ago: 33,
            featured: false,
            body: &[
                P("A day off spent catching up on errands is not a day off. Real rest has a different shape: unhurried mornings, time outdoors, something for the body and nothing on the calendar after six."),
                L(&[
                    "Leave the phone in another room for the first hour.",
                    "Walk somewhere green, without headphones.",
                    "Book something that is purely for you — a treatment, a class, a long lunch.",
                    "Go to bed early, without guilt.",
                ]),
                P("Try it once a month. You may find the other days get easier too."),
            ],
        },
    ],
    pages: &[
        Page {
            title: "About",
            slug: "about",
            excerpt: "A small studio in Bath for massage, skin care, movement and nutrition.",
            menu: Menu::Primary,
            body: &[
                P("We opened the studio in 2019 with a simple idea: care should fit around your life, not the other way round. Four practitioners, three treatment rooms and one light-filled studio for small classes."),
                H("What we believe"),
                L(&[
                    "Unhurried appointments, with time built in between every one.",
                    "Honest advice — including when you don't need to book again yet.",
                    "Clean, gentle products from small makers.",
                ]),
                P("Book online any time; you'll have a confirmation in seconds and can move or cancel it from the email."),
            ],
        },
        Page {
            title: "Visit",
            slug: "visit",
            excerpt: "How to find us, when we are open, and what to bring.",
            menu: Menu::Primary,
            body: &[
                P("We are at 14 Willow Lane, a five-minute walk from Bath Spa station. Step-free access is through the garden gate on the left."),
                H("Opening hours"),
                L(&["Monday to Friday, 9am to 7pm", "Saturday, 9am to 2pm", "Sunday, closed"]),
                H("Good to know"),
                L(&[
                    "Please arrive ten minutes before your appointment.",
                    "Changes and cancellations are free up to 24 hours before.",
                    "There is paid parking on Southgate Street.",
                ]),
            ],
        },
        Page {
            title: "Cancellation policy",
            slug: "cancellation-policy",
            excerpt: "Moving or cancelling an appointment.",
            menu: Menu::Secondary,
            body: &[
                P("You can move or cancel an appointment from the link in your confirmation email, free of charge, up to 24 hours before it starts."),
                P("Within 24 hours we may charge half the price of the treatment, because we are rarely able to fill the time. A missed appointment is charged in full. If something unexpected happens, call us — we are always reasonable."),
            ],
        },
    ],
    theme_settings: &[
        ("visit_address", "14 Willow Lane, Bath BA1 2QT"),
        ("visit_phone", "+44 117 496 0123"),
        ("visit_hours", "Mon–Fri 9am–7pm · Sat 9am–2pm"),
    ],
};

// ---------------------------------------------------------------------------
// Lantern: an online academy
// ---------------------------------------------------------------------------

static ACADEMY: Set = Set {
    tags: &[
        ("Learning", "Notes on studying well, from the people who teach here.", "#c9822b"),
        ("Study skills", "Techniques that make learning stick.", "#3e6fb0"),
        ("Careers", "Turning new skills into new work.", "#4b8a6b"),
    ],
    posts: &[
        Post {
            title: "How to actually finish an online course",
            excerpt: "Most online courses are never finished. Four small habits that change that — none of which need more willpower.",
            image: "1434030216411-0b793f4b4173",
            topic: 1,
            days_ago: 3,
            featured: true,
            body: &[
                P("The courses people finish are rarely the easiest ones. They are the ones that became part of the week."),
                L(&[
                    "Pick a fixed slot — the same two evenings — and protect it like a meeting.",
                    "Stop mid-lesson, so the next session starts with momentum.",
                    "Do the quizzes even when they are optional: recall is how memory forms.",
                    "Tell someone what you are learning. Explaining is the best revision.",
                ]),
            ],
        },
        Post {
            title: "Spaced repetition, explained in five minutes",
            excerpt: "Why reviewing a little, at growing intervals, beats a long cram — and how our courses build it in for you.",
            image: "1456513080510-7bf3a84b82f8",
            topic: 1,
            days_ago: 9,
            featured: false,
            body: &[
                P("We forget most of what we learn within days, unless we meet it again just as it begins to fade. Each review then lasts longer than the one before."),
                H("Using it in practice"),
                L(&["Review a lesson the next day.", "Then after three days.", "Then after a week, and a month."]),
                P("Drip-scheduled courses on this site release lessons on a similar rhythm, so the spacing happens without you planning it."),
            ],
        },
        Post {
            title: "Building a portfolio while you learn",
            excerpt: "Every assignment can become a portfolio piece. How past students turned coursework into interviews.",
            image: "1522202176988-66273c2fd55f",
            topic: 2,
            days_ago: 16,
            featured: false,
            body: &[
                P("Employers want to see what you can do, not just what you have completed. The good news: you are already making the evidence."),
                L(&[
                    "Keep every assignment, and write three sentences on the decisions behind it.",
                    "Pick your best three and polish them after the course ends.",
                    "Share your certificate link alongside the work, not instead of it.",
                ]),
            ],
        },
    ],
    pages: &[Page {
        title: "About",
        slug: "about",
        excerpt: "Practical courses, taught by people who do the work.",
        menu: Menu::Primary,
        body: &[
            P("We make practical, self-paced courses taught by people who do the work every day. Every course has short lessons, quizzes that help it stick and a certificate when you finish."),
            H("How it works"),
            L(&["Start with a free preview lesson.", "Learn at your own pace, on any device.", "Earn a certificate you can share."]),
        ],
    }],
    theme_settings: &[],
};
