//! Tools → Dummy content: the rules.
//!
//! Dummy content is demonstration material an administrator generates so a
//! new site (or a theme being tried) has something real-shaped to show: the
//! way a theme store's demo site looks full. It comes in two forms:
//!
//! * **The full demo** — one click, a finished-looking site: tags, authors,
//!   posts, pages, members, store products and both menus ([`Plan::full`]).
//! * **Custom** — any of those kinds, in the numbers asked for
//!   ([`Plan::custom`]).
//!
//! Everything generated is recorded (see `vuuut_db::repos::dummies`) so it can
//! be removed again; removal deletes only what is still exactly as generated.
//!
//! This module is pure. It decides what may be asked for (kinds, ceilings,
//! options), and it writes the content itself — titles, bodies as Lexical,
//! people, products — from a fixed catalogue and a seeded generator, so the
//! same seed always produces the same site. Nothing here touches the
//! database, and nothing generated ever carries a reachable email address:
//! every person is `@example.com`, a domain reserved for exactly this.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};

/// The kinds of content the tool can generate, in the order they are created.
/// Order matters: posts need the tags and authors made before them, and the
/// menus need the pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Tags,
    Users,
    Posts,
    Pages,
    Members,
    Products,
    Navigation,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Tags, Kind::Users, Kind::Posts, Kind::Pages, Kind::Members, Kind::Products, Kind::Navigation];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Tags => "tags",
            Kind::Users => "users",
            Kind::Posts => "posts",
            Kind::Pages => "pages",
            Kind::Members => "members",
            Kind::Products => "products",
            Kind::Navigation => "navigation",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Tags => "Tags",
            Kind::Users => "Authors",
            Kind::Posts => "Posts",
            Kind::Pages => "Pages",
            Kind::Members => "Members",
            Kind::Products => "Store products",
            Kind::Navigation => "Navigation menus",
        }
    }

    /// One item and several, for "3 posts" / "1 post".
    pub fn noun(self) -> (&'static str, &'static str) {
        match self {
            Kind::Tags => ("tag", "tags"),
            Kind::Users => ("author", "authors"),
            Kind::Posts => ("post", "posts"),
            Kind::Pages => ("page", "pages"),
            Kind::Members => ("member", "members"),
            Kind::Products => ("product", "products"),
            Kind::Navigation => ("menu link", "menu links"),
        }
    }

    /// What one of this kind is, for the custom form.
    pub fn hint(self) -> &'static str {
        match self {
            Kind::Tags => "Topics with descriptions and colours.",
            Kind::Users => "Staff authors with bios. They cannot sign in.",
            Kind::Posts => "Articles with headings, lists and quotes, spread over recent weeks.",
            Kind::Pages => "About, Contact, Privacy policy and similar.",
            Kind::Members => "Free members at @example.com addresses. They are never emailed.",
            Kind::Products => "Store products with prices, physical and digital.",
            Kind::Navigation => "Adds links to published pages or public tags.",
        }
    }

    /// The most one request may make. The catalogue behind each kind is
    /// finite, and a demo never needs more; the ceiling is also what keeps a
    /// stray number in the form from filling the database.
    pub fn max(self) -> u32 {
        match self {
            Kind::Tags => TAGS.len() as u32,
            Kind::Users => PEOPLE.len() as u32,
            Kind::Posts => 48,
            Kind::Pages => PAGES.len() as u32,
            Kind::Members => 200,
            Kind::Products => PRODUCTS.len() as u32,
            Kind::Navigation => 1,
        }
    }

    /// What the one-click full demo makes of each kind.
    pub fn full(self) -> u32 {
        match self {
            Kind::Tags => 6,
            Kind::Users => 3,
            Kind::Posts => 16,
            Kind::Pages => 5,
            Kind::Members => 40,
            Kind::Products => 6,
            Kind::Navigation => 1,
        }
    }
}

/// What one request generates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    counts: [u32; 7],
    /// Feature images on posts, from Unsplash. Off for a site that must not
    /// load anything from another host.
    pub images: bool,
    /// Posts go up as published, or wait as drafts.
    pub publish: bool,
    /// Whether this is the one-click demo, for the record it leaves.
    pub full: bool,
}

impl Plan {
    pub fn full(images: bool) -> Plan {
        let mut counts = [0; 7];
        for (i, kind) in Kind::ALL.into_iter().enumerate() {
            counts[i] = kind.full();
        }
        Plan { counts, images, publish: true, full: true }
    }

    /// A custom plan from the form's fields: `count_<kind>` for each kind,
    /// `images` and `status`. Every number is clamped to its kind's ceiling,
    /// and anything unreadable is zero — a forged field can only ask for less.
    pub fn custom(fields: &[(String, String)]) -> Result<Plan, String> {
        let field = |name: &str| fields.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.trim().to_string());
        let mut counts = [0; 7];
        for (i, kind) in Kind::ALL.into_iter().enumerate() {
            let raw = field(&format!("count_{}", kind.key())).unwrap_or_default();
            let n = if kind == Kind::Navigation {
                u32::from(matches!(raw.as_str(), "1" | "on" | "true"))
            } else {
                raw.parse::<u32>().unwrap_or(0)
            };
            counts[i] = n.min(kind.max());
        }
        let plan = Plan {
            counts,
            images: matches!(field("images").as_deref(), Some("on" | "1" | "true")),
            publish: field("status").as_deref() != Some("draft"),
            full: false,
        };
        if plan.is_empty() {
            return Err("Choose at least one kind of content and how many to make.".into());
        }
        Ok(plan)
    }

    pub fn count(&self, kind: Kind) -> u32 {
        self.counts[Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0)]
    }

    pub fn is_empty(&self) -> bool {
        self.counts.iter().all(|n| *n == 0)
    }

    /// "6 tags, 3 authors and 16 posts", for the record and the confirmation.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = Kind::ALL
            .into_iter()
            .filter(|k| self.count(*k) > 0)
            .map(|k| match k {
                Kind::Navigation => "navigation links".to_string(),
                _ => counted(self.count(k) as i64, k),
            })
            .collect();
        join_and(&parts)
    }
}

/// "1 post" / "3 posts".
pub fn counted(n: i64, kind: Kind) -> String {
    let (one, many) = kind.noun();
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn join_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// ---------------------------------------------------------------------------
// What gets made
// ---------------------------------------------------------------------------

pub struct DummyTag {
    pub name: &'static str,
    pub description: &'static str,
    pub accent: &'static str,
}

pub struct DummyPerson {
    pub name: &'static str,
    pub email: String,
    pub bio: &'static str,
    pub location: &'static str,
    /// The staff role. Authors and Contributors only: a generated account
    /// must never hold power over the site, even one nobody can sign in to.
    pub role: &'static str,
}

pub struct DummyPost {
    pub title: String,
    pub excerpt: &'static str,
    pub lexical: String,
    pub featured: bool,
    pub image: Option<&'static str>,
    /// How long ago it was published, so a list has a believable spread.
    pub days_ago: i64,
}

pub struct DummyPage {
    pub title: &'static str,
    pub slug: &'static str,
    pub excerpt: &'static str,
    pub lexical: String,
    /// Which menu the page belongs in.
    pub menu: Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Primary,
    Secondary,
}

pub struct DummyMember {
    pub name: String,
    pub email: String,
    pub days_ago: i64,
}

pub struct DummyProduct {
    pub title: &'static str,
    pub excerpt: &'static str,
    pub lexical: String,
    pub price_cents: i64,
    pub compare_at_cents: i64,
    pub product_type: &'static str,
    pub tags: Vec<String>,
    pub digital: bool,
    pub featured: bool,
}

/// The generator for one request. Seeded, so a seed and a plan always make
/// the same content; the admin seeds it from the batch id.
pub struct Generator {
    rng: StdRng,
    /// Short, per-batch, so addresses never collide with an earlier batch's.
    tag: String,
}

impl Generator {
    pub fn new(seed: u64) -> Generator {
        Generator { rng: StdRng::seed_from_u64(seed), tag: format!("{:04x}", seed & 0xffff) }
    }

    /// The first `n` tags of the catalogue, shuffled so two custom batches
    /// of three tags do not always pick the same three.
    pub fn tags(&mut self, n: u32) -> Vec<&'static DummyTag> {
        let mut all: Vec<&DummyTag> = TAGS.iter().collect();
        all.shuffle(&mut self.rng);
        all.truncate(n as usize);
        all
    }

    pub fn people(&mut self, n: u32) -> Vec<DummyPerson> {
        let mut all: Vec<&Person> = PEOPLE.iter().collect();
        all.shuffle(&mut self.rng);
        all.into_iter()
            .take(n as usize)
            .enumerate()
            .map(|(i, p)| DummyPerson {
                name: p.name,
                email: format!("{}.{}@example.com", ascii_handle(p.name), self.tag),
                bio: p.bio,
                location: p.location,
                role: if i % 3 == 2 { "Contributor" } else { "Author" },
            })
            .collect()
    }

    pub fn posts(&mut self, n: u32) -> Vec<DummyPost> {
        let mut order: Vec<usize> = (0..ARTICLES.len()).collect();
        order.shuffle(&mut self.rng);
        let mut days = 1i64;
        (0..n as usize)
            .map(|i| {
                let article = &ARTICLES[order[i % ARTICLES.len()]];
                // Past the catalogue, a second round reads as a follow-up.
                let title = if i < ARTICLES.len() { article.title.to_string() } else { format!("{}, revisited", article.title) };
                let post = DummyPost {
                    title,
                    excerpt: article.excerpt,
                    lexical: self.article_body(article),
                    featured: i % 7 == 1,
                    image: Some(IMAGES[self.rng.random_range(0..IMAGES.len())]),
                    days_ago: days,
                };
                days += self.rng.random_range(2..6);
                post
            })
            .collect()
    }

    fn article_body(&mut self, article: &Article) -> String {
        let mut blocks = vec![paragraph(article.lede)];
        let mut paras: Vec<&str> = PARAGRAPHS.to_vec();
        paras.shuffle(&mut self.rng);
        let mut heads: Vec<&str> = HEADINGS.to_vec();
        heads.shuffle(&mut self.rng);
        blocks.push(paragraph(paras[0]));
        blocks.push(heading(heads[0], "h2"));
        blocks.push(paragraph(paras[1]));
        let mut items: Vec<&str> = LIST_ITEMS.to_vec();
        items.shuffle(&mut self.rng);
        blocks.push(list(&items[..3]));
        blocks.push(paragraph(paras[2]));
        blocks.push(quote(QUOTES[self.rng.random_range(0..QUOTES.len())]));
        blocks.push(heading(heads[1], "h2"));
        blocks.push(paragraph(paras[3]));
        blocks.push(paragraph(paras[4]));
        document(blocks)
    }

    pub fn pages(&mut self, n: u32) -> Vec<DummyPage> {
        PAGES.iter()
            .take(n as usize)
            .map(|p| DummyPage {
                title: p.title,
                slug: p.slug,
                excerpt: p.excerpt,
                lexical: document(p.body.iter().map(|b| b.node()).collect()),
                menu: p.menu,
            })
            .collect()
    }

    pub fn members(&mut self, n: u32) -> Vec<DummyMember> {
        let mut names: Vec<(usize, usize)> = (0..FIRST_NAMES.len()).flat_map(|f| (0..LAST_NAMES.len()).map(move |l| (f, l))).collect();
        names.shuffle(&mut self.rng);
        names
            .into_iter()
            .take(n as usize)
            .map(|(f, l)| {
                let name = format!("{} {}", FIRST_NAMES[f], LAST_NAMES[l]);
                DummyMember {
                    email: format!("{}.{}@example.com", ascii_handle(&name), self.tag),
                    name,
                    // Signups bunch towards the present, the way a growing
                    // list does, which gives the member charts a shape.
                    days_ago: (self.rng.random::<f64>().powi(2) * 180.0) as i64,
                }
            })
            .collect()
    }

    pub fn products(&mut self, n: u32) -> Vec<DummyProduct> {
        PRODUCTS.iter()
            .take(n as usize)
            .enumerate()
            .map(|(i, p)| DummyProduct {
                title: p.title,
                excerpt: p.excerpt,
                lexical: document(vec![paragraph(p.excerpt), paragraph(p.detail)]),
                price_cents: p.price_cents,
                compare_at_cents: if i % 4 == 1 { p.price_cents + p.price_cents / 4 } else { 0 },
                product_type: p.product_type,
                tags: p.tags.iter().map(|t| t.to_string()).collect(),
                digital: p.digital,
                featured: i < 2,
            })
            .collect()
    }

    /// Up to two indices from `0..len`, for a post's tags.
    pub fn pick_some(&mut self, len: usize) -> Vec<usize> {
        if len == 0 {
            return Vec::new();
        }
        let mut picks = vec![self.rng.random_range(0..len)];
        if len > 1 && self.rng.random_bool(0.5) {
            let second = self.rng.random_range(0..len);
            if second != picks[0] {
                picks.push(second);
            }
        }
        picks
    }

    pub fn pick(&mut self, len: usize) -> usize {
        if len == 0 { 0 } else { self.rng.random_range(0..len) }
    }
}

/// "Maya Okafor" → "maya.okafor", ASCII only, for an address.
fn ascii_handle(name: &str) -> String {
    name.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

// ---------------------------------------------------------------------------
// Lexical
// ---------------------------------------------------------------------------
//
// Generated bodies are Koenig's own document format rather than HTML, so a
// dummy post opens in the editor as ordinary paragraphs and headings that can
// be changed one by one — not as a single HTML card.

fn text(t: &str) -> Value {
    json!({"detail": 0, "format": 0, "mode": "normal", "style": "", "text": t, "type": "extended-text", "version": 1})
}

fn block(kind: &str, children: Vec<Value>) -> Value {
    json!({"children": children, "direction": "ltr", "format": "", "indent": 0, "type": kind, "version": 1})
}

pub fn paragraph(t: &str) -> Value {
    block("paragraph", vec![text(t)])
}

pub fn heading(t: &str, tag: &str) -> Value {
    let mut node = block("extended-heading", vec![text(t)]);
    node["tag"] = json!(tag);
    node
}

pub fn quote(t: &str) -> Value {
    block("extended-quote", vec![text(t)])
}

pub fn list(items: &[&str]) -> Value {
    let children = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mut li = block("listitem", vec![text(item)]);
            li["value"] = json!(i + 1);
            li
        })
        .collect();
    let mut node = block("list", children);
    node["listType"] = json!("bullet");
    node["start"] = json!(1);
    node["tag"] = json!("ul");
    node
}

pub fn document(children: Vec<Value>) -> String {
    json!({"root": {"children": children, "direction": "ltr", "format": "", "indent": 0, "type": "root", "version": 1}}).to_string()
}

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------
//
// One imaginary publication about work, design and the tools people make
// things with. It reads as a real site in any theme, and it matches the
// photographs below, which are of desks, screens and teams.

/// Unsplash photographs, hotlinked the way Unsplash asks. The same set the
/// `seed-demo` command uses.
pub const IMAGES: &[&str] = &[
    "https://images.unsplash.com/photo-1461749280684-dccba630e2f6?w=2000&q=80",
    "https://images.unsplash.com/photo-1555066931-4365d14bab8c?w=2000&q=80",
    "https://images.unsplash.com/photo-1531482615713-2afd69097998?w=2000&q=80",
    "https://images.unsplash.com/photo-1522071820081-009f0129c71c?w=2000&q=80",
    "https://images.unsplash.com/photo-1517180102446-f3ece451e9d8?w=2000&q=80",
    "https://images.unsplash.com/photo-1499750310107-5fef28a66643?w=2000&q=80",
    "https://images.unsplash.com/photo-1504384308090-c894fdcc538d?w=2000&q=80",
    "https://images.unsplash.com/photo-1507003211169-0a1dd7228f2d?w=2000&q=80",
    "https://images.unsplash.com/photo-1486312338219-ce68d2c6f44d?w=2000&q=80",
    "https://images.unsplash.com/photo-1484417894907-623942c8ee29?w=2000&q=80",
];

pub const TAGS: &[DummyTag] = &[
    DummyTag { name: "Design", description: "Interfaces, type and the craft of making things feel considered.", accent: "#9c2f5a" },
    DummyTag { name: "Engineering", description: "How things are built, and what breaks along the way.", accent: "#2f5aa8" },
    DummyTag { name: "Product", description: "Deciding what to make, and what to leave out.", accent: "#2f7a4b" },
    DummyTag { name: "Culture", description: "How small teams work, day to day.", accent: "#b5591e" },
    DummyTag { name: "Guides", description: "Step-by-step pieces you can follow from start to finish.", accent: "#5d3fa3" },
    DummyTag { name: "News", description: "Releases, changes and the occasional milestone.", accent: "#1f6f7a" },
    DummyTag { name: "Interviews", description: "Long conversations with people who make things.", accent: "#7a4a1f" },
    DummyTag { name: "Writing", description: "On drafting, editing and saying things plainly.", accent: "#4a5a6a" },
    DummyTag { name: "Tools", description: "The software and hardware on our desks, reviewed honestly.", accent: "#3f6f2f" },
    DummyTag { name: "Remote work", description: "Working well across time zones and kitchen tables.", accent: "#8a3f3f" },
    DummyTag { name: "Data", description: "Measuring what matters, and reading the numbers carefully.", accent: "#2f4f7a" },
    DummyTag { name: "Opinion", description: "Arguments we are willing to have.", accent: "#6a2f6a" },
];

struct Person {
    name: &'static str,
    bio: &'static str,
    location: &'static str,
}

const PEOPLE: &[Person] = &[
    Person { name: "Maya Okafor", bio: "Maya edits the design desk and has opinions about kerning she will share if asked.", location: "Lagos" },
    Person { name: "Daniel Reyes", bio: "Daniel writes about engineering teams and the systems they inherit.", location: "Mexico City" },
    Person { name: "Hannah Lindqvist", bio: "Hannah covers product strategy and spent ten years shipping software before writing about it.", location: "Stockholm" },
    Person { name: "Arjun Mehta", bio: "Arjun writes the guides, tests every step twice, and answers reader email himself.", location: "Pune" },
    Person { name: "Claire Dubois", bio: "Claire interviews makers and keeps a list of every good question she has ever heard.", location: "Lyon" },
    Person { name: "Tom Whitaker", bio: "Tom reviews tools and has a drawer full of keyboards to prove it.", location: "Leeds" },
    Person { name: "Sofia Marino", bio: "Sofia writes about data, and about when not to trust it.", location: "Turin" },
    Person { name: "Kenji Watanabe", bio: "Kenji covers remote teams from wherever his laptop is that month.", location: "Osaka" },
];

struct Article {
    title: &'static str,
    excerpt: &'static str,
    lede: &'static str,
}

const ARTICLES: &[Article] = &[
    Article { title: "The quiet power of boring technology", excerpt: "Why the tools nobody talks about are the ones still running in ten years.", lede: "Every team has a shelf of exciting tools it tried once. The ones that stayed are rarely the ones anybody wrote a launch post about." },
    Article { title: "Designing a type system that gets out of the way", excerpt: "Five sizes, two weights and a rule for everything else.", lede: "A type scale is a promise: every heading on every page will be one of these sizes, and nothing else. Keeping it is harder than making it." },
    Article { title: "What we learned shipping to ten thousand users", excerpt: "The first thousand teach you about your product. The next nine teach you about your support inbox.", lede: "Growth changes what is hard. The bugs stop being in the code and start being in the assumptions the code was written on." },
    Article { title: "A practical guide to colour contrast", excerpt: "How to check it, how to fix it, and why your brand colour is probably fine.", lede: "Contrast is the one accessibility rule that can be measured with a number, which makes it the easiest to get right and the most embarrassing to get wrong." },
    Article { title: "How we run design reviews across four time zones", excerpt: "Written first, spoken second, and never at midnight for anyone.", lede: "A design review used to mean a room and a projector. Ours now means a document, a deadline and a short call for whatever the document could not settle." },
    Article { title: "Writing documentation people actually read", excerpt: "Start with the task, not the feature. End when the task is done.", lede: "Nobody opens documentation to admire it. They open it halfway through something, stuck, and they want to be unstuck in one paragraph." },
    Article { title: "Measuring what matters: a short guide to performance budgets", excerpt: "Pick three numbers, write them down, and make the build fail when they are broken.", lede: "A performance budget is a limit agreed before anybody is attached to the thing that would break it. That timing is the whole trick." },
    Article { title: "Our roadmap for the year ahead", excerpt: "What we are building next, what we have stopped building, and why.", lede: "A roadmap is mostly a list of things we decided not to do. Here is ours, including the parts that were difficult to cut." },
    Article { title: "The case for fewer meetings", excerpt: "We cancelled every recurring meeting for a month. Most of them did not come back.", lede: "The experiment was simple: every recurring meeting was deleted, and any that people missed could be booked again. Three weeks later, four had returned." },
    Article { title: "In conversation with a bookbinder", excerpt: "On patience, paper grain and doing one thing for thirty years.", lede: "Her workshop is two rooms above a bakery, and every surface in it is covered in paper that is either drying or waiting to be folded." },
    Article { title: "Small teams, sharp tools", excerpt: "Why a team of five should not run the software of a team of five hundred.", lede: "Enterprise tools are built for coordination problems small teams do not have yet, and they bring those problems with them." },
    Article { title: "How to choose a typeface for a long read", excerpt: "Test it at the size people will read it, on the screens they own.", lede: "A typeface chosen from a specimen page is chosen for the wrong job. Set three paragraphs at the real size and read them on a phone before deciding." },
    Article { title: "Notes from a year of remote work", excerpt: "The kitchen table, the bad microphone and the habits that finally stuck.", lede: "The first month felt like a holiday and the third felt like a trap. By the twelfth, a few small habits had made it simply work." },
    Article { title: "When to trust the numbers", excerpt: "A dashboard is an argument. Read it like one.", lede: "Every chart was made by someone who chose what to count. Knowing what they left out is most of reading it well." },
    Article { title: "A beginner's guide to version control", excerpt: "Commits, branches and the three commands you will use every day.", lede: "Version control is a time machine with a very literal operator. Learn to ask it precise questions and it will save every project you start." },
    Article { title: "The tools on our desks this year", excerpt: "What we kept, what we returned, and the one cable everyone borrows.", lede: "We asked the whole team to list the tools they could not work without. The overlap was smaller than anyone expected." },
    Article { title: "Editing is mostly deleting", excerpt: "The second draft is the first draft with the throat-clearing removed.", lede: "Most first drafts begin three paragraphs before the point. Find the sentence where the piece actually starts, and cut everything above it." },
    Article { title: "Why we rewrote our onboarding", excerpt: "New users were finishing the tour and still not knowing what to do first.", lede: "The old onboarding explained every button. The new one asks one question and shows the single screen that answers it." },
    Article { title: "Interview: the engineer who keeps the lights on", excerpt: "Twenty years of on-call shifts, and what she would tell her younger self.", lede: "She has been paged at weddings, on aeroplanes and once in the middle of giving a talk about not being paged." },
    Article { title: "Accessibility is not a feature", excerpt: "It is a quality of every feature, or of none of them.", lede: "Treating accessibility as a ticket guarantees it will be the ticket that slips. Treating it as part of done means it never has to." },
    Article { title: "Announcing our new reader app", excerpt: "Read offline, pick up where you left off, and never see an ad.", lede: "After a year of testing with a small group of readers, the app is ready for everyone, on every phone we could find." },
    Article { title: "How we price our products", excerpt: "Three tiers, no surprises, and a free plan that is actually useful.", lede: "Pricing pages are where honesty is cheapest and most valuable. Ours fits on one screen and has no asterisks." },
    Article { title: "The slow craft of a good error message", excerpt: "Say what happened, say how to fix it, and do not apologise.", lede: "An error message is written for someone at their least patient. Every word in it has to earn its place." },
    Article { title: "Opinion: the open web is worth defending", excerpt: "A link should be a promise, and a page should still be there next year.", lede: "Platforms come and go and take their archives with them. A site you own, at an address you control, is still the most durable thing on the internet." },
];

const PARAGRAPHS: &[&str] = &[
    "The obvious answer was the wrong one, and it took us longer than it should have to notice. What looked like a problem with the tool turned out to be a problem with how we were asking it to work.",
    "We started small: one team, one project, and a promise to write down every decision and the reason behind it. Six months later, that document is the most useful thing we own.",
    "None of this is new. People have been solving versions of this problem for as long as there have been teams, and most of the good answers are older than the software we use to apply them.",
    "The hardest part was not the change itself but agreeing on what done would look like. Once that was written down, the work went quickly.",
    "It is tempting to measure everything. We learned to measure three things well and to look at them every week, which turned out to be worth more than a dashboard nobody opened.",
    "Feedback arrived faster than we expected, and most of it was kind. The rest was useful, which is better.",
    "There is a difference between a decision that is reversible and one that is not, and the mistake is to treat them with the same caution. We now move quickly on the first kind and slowly on the second.",
    "What surprised us most was how much of the work was communication. The code took a week; explaining why it changed took a month.",
    "A good default saves more time than a good feature. Most people never change a setting, so the setting they never change had better be right.",
    "We tried three approaches before settling on this one. The first two were not wrong, exactly, but they solved problems we did not have yet.",
    "The simplest version shipped first, and it is still the version most people use. The clever additions we made later are used by a few, and that is fine.",
    "Looking back, the turning point was a single conversation in which someone asked what we would do if we had to start again tomorrow. The answer was smaller than anything on the roadmap.",
    "Reading the old notes is humbling. Half of what we worried about never happened, and the thing that did go wrong was not on the list.",
    "The lesson is not that the old way was bad. It served us well for years. The lesson is to notice when the reasons for a choice have quietly changed.",
];

const HEADINGS: &[&str] = &[
    "Where we started",
    "What changed",
    "The part that was hard",
    "What we would do again",
    "A few numbers",
    "What we got wrong",
    "How it works now",
    "Questions we still have",
    "The short version",
    "What comes next",
];

const LIST_ITEMS: &[&str] = &[
    "Write the decision down before acting on it.",
    "Choose the reversible option when you are unsure.",
    "Ask the person closest to the problem first.",
    "Make the default the thing most people want.",
    "Measure a few things well instead of everything badly.",
    "Delete what nobody has used in six months.",
    "Explain the why, not just the what.",
    "Test with real content, never with lorem ipsum.",
    "Keep the first version small enough to finish.",
    "Review the work, not the person.",
];

const QUOTES: &[&str] = &[
    "The best tool is the one the whole team already knows how to use.",
    "Simple is not the same as easy. Simple is what is left after the hard work.",
    "We did not need more process. We needed fewer surprises.",
    "Every rule we kept was one somebody could explain in a sentence.",
    "Good work is mostly the work nobody sees.",
];

/// One block of a page body.
pub enum Block {
    P(&'static str),
    H(&'static str),
    L(&'static [&'static str]),
}

impl Block {
    fn node(&self) -> Value {
        match self {
            Block::P(t) => paragraph(t),
            Block::H(t) => heading(t, "h2"),
            Block::L(items) => list(items),
        }
    }
}

pub struct PageSpec {
    pub title: &'static str,
    pub slug: &'static str,
    pub excerpt: &'static str,
    pub body: &'static [Block],
    pub menu: Menu,
}

/// In the order a menu shows them; a custom request for two pages makes the
/// first two.
pub const PAGES: &[PageSpec] = &[
    PageSpec {
        title: "About", slug: "about", menu: Menu::Primary,
        excerpt: "Who we are, what we write about, and why.",
        body: &[
            Block::P("We are a small independent publication about work, design and the tools people make things with. We publish a few long pieces a month and no filler."),
            Block::H("What we cover"),
            Block::L(&["How teams make decisions and ship work", "Design, type and the craft of interfaces", "Honest reviews of the tools on our desks", "Long interviews with people who make things"]),
            Block::H("How we are funded"),
            Block::P("By our readers. Members get every piece by email, and paid members keep the whole thing independent. We do not run ads we would not click on."),
        ],
    },
    PageSpec {
        title: "Contact", slug: "contact", menu: Menu::Primary,
        excerpt: "How to reach us, and how quickly we reply.",
        body: &[
            Block::P("The quickest way to reach us is by email at hello@example.com. We read everything and reply to most messages within two working days."),
            Block::H("Pitches"),
            Block::P("We commission a handful of outside pieces each season. Send a paragraph about the idea and a link to something you have written."),
            Block::H("Corrections"),
            Block::P("If we got something wrong, tell us and we will fix it, and say so at the end of the piece."),
        ],
    },
    PageSpec {
        title: "Privacy policy", slug: "privacy", menu: Menu::Secondary,
        excerpt: "What we collect, why we collect it, and how to have it deleted.",
        body: &[
            Block::P("We collect as little as we can. When you sign up we keep your email address and name so we can send you what you asked for, and nothing else."),
            Block::H("What we store"),
            Block::L(&["Your email address and name, if you give them", "Which emails you have opened, so we can stop sending what nobody reads", "Your payment details, held by our payment provider rather than by us"]),
            Block::H("Your choices"),
            Block::P("You can unsubscribe from any email with one click, and ask us to delete your account at any time by writing to privacy@example.com."),
        ],
    },
    PageSpec {
        title: "Terms of service", slug: "terms", menu: Menu::Secondary,
        excerpt: "The rules for using this site and its memberships.",
        body: &[
            Block::P("By using this site you agree to these terms. They are short on purpose."),
            Block::H("Memberships"),
            Block::P("Paid memberships renew until you cancel. You can cancel at any time from your account page, and you keep access until the end of the period you paid for."),
            Block::H("Our content"),
            Block::P("You are welcome to quote and link to our writing. Please ask before republishing a whole piece."),
        ],
    },
    PageSpec {
        title: "Frequently asked questions", slug: "faq", menu: Menu::Primary,
        excerpt: "Quick answers to the questions readers ask most.",
        body: &[
            Block::H("How often do you publish?"),
            Block::P("Two or three long pieces a month, and a short note most weeks."),
            Block::H("Is there a free plan?"),
            Block::P("Yes. Free members get most of what we publish by email. Paid members also get the archive and the occasional extra."),
            Block::H("Can I change my email address?"),
            Block::P("Yes, from your account page. Sign in, open your account, and edit the address."),
        ],
    },
    PageSpec {
        title: "Work with us", slug: "work-with-us", menu: Menu::Secondary,
        excerpt: "Open roles, and how we hire.",
        body: &[
            Block::P("We are a small team that works remotely across several time zones. We hire rarely and carefully."),
            Block::H("How we hire"),
            Block::L(&["A short written application, no cover letter needed", "One paid trial project, a few days long", "A conversation with the people you would work with"]),
            Block::P("There are no open roles right now. Write to jobs@example.com and we will keep your note on file."),
        ],
    },
    PageSpec {
        title: "Advertise", slug: "advertise", menu: Menu::Secondary,
        excerpt: "Sponsorships that respect our readers.",
        body: &[
            Block::P("We accept a small number of sponsors each season, one per email, clearly labelled and never disguised as writing."),
            Block::H("What we look for"),
            Block::P("Products our readers would choose for themselves. If we would not use it, we will not run it."),
        ],
    },
    PageSpec {
        title: "Newsletter", slug: "newsletter", menu: Menu::Primary,
        excerpt: "Every new piece, in your inbox, and nothing else.",
        body: &[
            Block::P("Sign up and we will send each new piece by email on the day it is published. No round-ups of other people's links, no promotions."),
            Block::P("You can unsubscribe from any email with one click."),
        ],
    },
];

const FIRST_NAMES: &[&str] = &[
    "Amara", "Ben", "Chloe", "Diego", "Elena", "Farah", "Gabriel", "Hana", "Isaac", "Julia",
    "Kofi", "Lena", "Marco", "Nadia", "Oliver", "Priya", "Rosa", "Samuel", "Tara", "Victor",
];

const LAST_NAMES: &[&str] = &[
    "Adams", "Bianchi", "Chen", "Dlamini", "Evans", "Fischer", "Garcia", "Haddad", "Ivanova", "Jensen",
    "Kowalski", "Larsen", "Moreau", "Nakamura", "Osei", "Patel", "Quinn", "Rossi", "Silva", "Tanaka",
];

struct ProductSpec {
    title: &'static str,
    excerpt: &'static str,
    detail: &'static str,
    price_cents: i64,
    product_type: &'static str,
    tags: &'static [&'static str],
    digital: bool,
}

const PRODUCTS: &[ProductSpec] = &[
    ProductSpec { title: "Field notebook, set of three", excerpt: "Dot-grid pocket notebooks with a stitched spine and a cover that softens with use.", detail: "Forty-eight pages each, 90 gsm paper that takes a fountain pen without bleeding through.", price_cents: 1800, product_type: "Stationery", tags: &["paper", "bestseller"], digital: false },
    ProductSpec { title: "The long-read typography guide", excerpt: "A practical PDF guide to setting type for reading on screens.", detail: "Sixty pages with worked examples, a printable checklist and sample stylesheets.", price_cents: 2400, product_type: "Guide", tags: &["design", "pdf"], digital: true },
    ProductSpec { title: "Desk print: the grid", excerpt: "A risograph print of our modular grid, signed and numbered.", detail: "A3, printed in two colours on recycled paper. Shipped flat in a rigid sleeve.", price_cents: 3500, product_type: "Print", tags: &["art", "limited"], digital: false },
    ProductSpec { title: "Interface icon set", excerpt: "Four hundred outline icons, drawn on one grid, in SVG and PNG.", detail: "Includes a Figma library and a licence for unlimited commercial projects.", price_cents: 4900, product_type: "Digital asset", tags: &["design", "icons"], digital: true },
    ProductSpec { title: "Canvas tote bag", excerpt: "A heavy canvas tote with the masthead printed on one side.", detail: "Holds a laptop, two notebooks and lunch. Machine washable.", price_cents: 2200, product_type: "Merchandise", tags: &["merch"], digital: false },
    ProductSpec { title: "Writing workshop recording", excerpt: "A three-hour workshop on editing your own drafts, recorded live.", detail: "Includes the exercises, the slides and a transcript of the questions.", price_cents: 3900, product_type: "Course", tags: &["writing", "video"], digital: true },
    ProductSpec { title: "Enamel mug", excerpt: "A camp mug for the desk, the garden and the long meeting.", detail: "Holds 350 ml. Dishwasher safe; microwave very much not.", price_cents: 1600, product_type: "Merchandise", tags: &["merch", "bestseller"], digital: false },
    ProductSpec { title: "Remote team handbook template", excerpt: "The handbook we use ourselves, as an editable template.", detail: "Covers working hours, decisions, meetings and how to disagree well.", price_cents: 1900, product_type: "Template", tags: &["remote", "docs"], digital: true },
    ProductSpec { title: "Wall planner", excerpt: "A year on one sheet, for planning in pencil rather than in software.", detail: "A2, printed on uncoated paper that takes pencil and eraser well.", price_cents: 2800, product_type: "Stationery", tags: &["paper"], digital: false },
    ProductSpec { title: "Annual print edition", excerpt: "The year's best pieces, edited and printed as a paperback.", detail: "Two hundred and forty pages, sewn binding, ships worldwide.", price_cents: 3200, product_type: "Book", tags: &["book", "limited"], digital: false },
    ProductSpec { title: "Presentation slide kit", excerpt: "Clean slide layouts for talks, built on our type system.", detail: "Keynote, PowerPoint and Google Slides versions, with forty layouts.", price_cents: 2900, product_type: "Template", tags: &["design", "slides"], digital: true },
    ProductSpec { title: "Gift membership, one year", excerpt: "A year of paid membership for someone else, delivered by email.", detail: "Choose the date it arrives. The recipient can start reading straight away.", price_cents: 6000, product_type: "Gift", tags: &["gift"], digital: true },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn custom_clamps_every_count_to_its_ceiling() {
        let plan = Plan::custom(&fields(&[("count_posts", "100000"), ("count_tags", "999"), ("count_members", "5")])).unwrap();
        assert_eq!(plan.count(Kind::Posts), Kind::Posts.max());
        assert_eq!(plan.count(Kind::Tags), TAGS.len() as u32);
        assert_eq!(plan.count(Kind::Members), 5);
    }

    #[test]
    fn unreadable_or_negative_counts_are_zero() {
        let plan = Plan::custom(&fields(&[("count_posts", "-3"), ("count_pages", "lots"), ("count_tags", "2")])).unwrap();
        assert_eq!(plan.count(Kind::Posts), 0);
        assert_eq!(plan.count(Kind::Pages), 0);
        assert_eq!(plan.count(Kind::Tags), 2);
    }

    #[test]
    fn an_empty_request_is_refused() {
        assert!(Plan::custom(&fields(&[("count_posts", "0"), ("images", "on")])).is_err());
        assert!(Plan::custom(&[]).is_err());
    }

    #[test]
    fn navigation_is_a_switch_not_a_number() {
        let plan = Plan::custom(&fields(&[("count_navigation", "on")])).unwrap();
        assert_eq!(plan.count(Kind::Navigation), 1);
        let plan = Plan::custom(&fields(&[("count_navigation", "7"), ("count_tags", "1")])).unwrap();
        assert_eq!(plan.count(Kind::Navigation), 0);
    }

    #[test]
    fn status_defaults_to_published_and_images_to_off_in_custom() {
        let plan = Plan::custom(&fields(&[("count_posts", "2")])).unwrap();
        assert!(plan.publish);
        assert!(!plan.images);
        let plan = Plan::custom(&fields(&[("count_posts", "2"), ("status", "draft"), ("images", "on")])).unwrap();
        assert!(!plan.publish);
        assert!(plan.images);
    }

    #[test]
    fn the_full_demo_fits_inside_every_ceiling() {
        let plan = Plan::full(true);
        for kind in Kind::ALL {
            assert!(plan.count(kind) <= kind.max(), "{kind:?}");
            assert!(plan.count(kind) > 0, "{kind:?}");
        }
        assert_eq!(plan.summary(), "6 tags, 3 authors, 16 posts, 5 pages, 40 members, 6 products and navigation links");
    }

    #[test]
    fn every_generated_address_is_at_the_reserved_domain() {
        let mut g = Generator::new(42);
        let people = g.people(Kind::Users.max());
        let members = g.members(Kind::Members.max());
        for email in people.iter().map(|p| &p.email).chain(members.iter().map(|m| &m.email)) {
            assert!(email.ends_with("@example.com"), "{email}");
            assert!(email.is_ascii(), "{email}");
        }
    }

    #[test]
    fn generated_staff_are_never_powerful() {
        let mut g = Generator::new(7);
        for p in g.people(Kind::Users.max()) {
            assert!(matches!(p.role, "Author" | "Contributor"), "{}", p.role);
        }
    }

    #[test]
    fn member_addresses_are_unique_within_a_batch() {
        let mut g = Generator::new(1);
        let members = g.members(Kind::Members.max());
        let mut emails: Vec<&String> = members.iter().map(|m| &m.email).collect();
        emails.sort();
        emails.dedup();
        assert_eq!(emails.len(), members.len());
    }

    #[test]
    fn the_same_seed_makes_the_same_site() {
        let a: Vec<String> = Generator::new(99).posts(10).into_iter().map(|p| p.title).collect();
        let b: Vec<String> = Generator::new(99).posts(10).into_iter().map(|p| p.title).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn post_titles_stay_unique_past_the_catalogue() {
        let posts = Generator::new(3).posts(Kind::Posts.max());
        let mut titles: Vec<&String> = posts.iter().map(|p| &p.title).collect();
        titles.sort();
        titles.dedup();
        assert_eq!(titles.len(), posts.len());
    }

    #[test]
    fn bodies_are_lexical_documents() {
        let post = Generator::new(5).posts(1).remove(0);
        let doc: Value = serde_json::from_str(&post.lexical).unwrap();
        let children = doc["root"]["children"].as_array().unwrap();
        assert!(children.iter().any(|c| c["type"] == "extended-heading"));
        assert!(children.iter().any(|c| c["type"] == "list"));
        for page in Generator::new(5).pages(Kind::Pages.max()) {
            let doc: Value = serde_json::from_str(&page.lexical).unwrap();
            assert!(!doc["root"]["children"].as_array().unwrap().is_empty(), "{}", page.title);
        }
    }

    #[test]
    fn kinds_round_trip_through_their_keys() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_key(kind.key()), Some(kind));
        }
        assert_eq!(Kind::from_key("settings"), None);
    }
}
