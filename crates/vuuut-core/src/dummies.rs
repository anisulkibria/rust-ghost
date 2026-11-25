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
    ListingCategories,
    Listings,
    /// Online courses with lessons, quizzes, students and reviews. Their
    /// course categories, demo students and store products come with them.
    Courses,
    CourseCategories,
    /// Bookable services. Their categories, team and sample bookings come
    /// with them.
    Services,
    ServiceCategories,
    BookingStaff,
    Navigation,
}

/// How many kinds there are.
pub const KINDS: usize = 14;

/// How many sample courses and services the catalogues hold.
pub const SAMPLE_COURSES: u32 = 7;
pub const SAMPLE_SERVICES: u32 = 8;

impl Kind {
    pub const ALL: [Kind; KINDS] = [
        Kind::Tags,
        Kind::Users,
        Kind::Posts,
        Kind::Pages,
        Kind::Members,
        Kind::Products,
        Kind::ListingCategories,
        Kind::Listings,
        Kind::Courses,
        Kind::CourseCategories,
        Kind::Services,
        Kind::ServiceCategories,
        Kind::BookingStaff,
        Kind::Navigation,
    ];

    /// Kinds made only alongside another kind, never asked for on their own.
    pub fn is_companion(self) -> bool {
        matches!(self, Kind::CourseCategories | Kind::ServiceCategories | Kind::BookingStaff)
    }

    pub fn key(self) -> &'static str {
        match self {
            Kind::Tags => "tags",
            Kind::Users => "users",
            Kind::Posts => "posts",
            Kind::Pages => "pages",
            Kind::Members => "members",
            Kind::Products => "products",
            Kind::ListingCategories => "listing_categories",
            Kind::Listings => "listings",
            Kind::Courses => "courses",
            Kind::CourseCategories => "course_categories",
            Kind::Services => "services",
            Kind::ServiceCategories => "service_categories",
            Kind::BookingStaff => "booking_staff",
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
            Kind::ListingCategories => "Directory categories",
            Kind::Listings => "Directory listings",
            Kind::Courses => "Courses",
            Kind::CourseCategories => "Course categories",
            Kind::Services => "Booking services",
            Kind::ServiceCategories => "Service categories",
            Kind::BookingStaff => "Booking team",
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
            Kind::ListingCategories => ("directory category", "directory categories"),
            Kind::Listings => ("directory listing", "directory listings"),
            Kind::Courses => ("course", "courses"),
            Kind::CourseCategories => ("course category", "course categories"),
            Kind::Services => ("bookable service", "bookable services"),
            Kind::ServiceCategories => ("service category", "service categories"),
            Kind::BookingStaff => ("team member", "team members"),
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
            Kind::ListingCategories => "Restaurants, Cafés, Hotels and more, with pictures.",
            Kind::Listings => "Places in four cities with hours, maps, photos and contact details. Switches the public directory on.",
            Kind::Courses => "Complete courses with lessons, quizzes, assignments, students and reviews; paid ones get a store product. Switches the course catalogue on.",
            Kind::CourseCategories => "Made with courses.",
            Kind::Services => "A wellness studio: massage, facials, classes and consultations, with its team, weekly hours and two weeks of sample bookings. Switches booking on.",
            Kind::ServiceCategories => "Made with services.",
            Kind::BookingStaff => "Made with services.",
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
            Kind::ListingCategories => LISTING_CATEGORIES.len() as u32,
            Kind::Listings => LISTINGS.len() as u32,
            Kind::Courses => SAMPLE_COURSES,
            Kind::Services => SAMPLE_SERVICES,
            Kind::CourseCategories | Kind::ServiceCategories | Kind::BookingStaff => 0,
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
            Kind::ListingCategories => LISTING_CATEGORIES.len() as u32,
            Kind::Listings => LISTINGS.len() as u32,
            // Courses and a booking business change what the site is, so
            // they come from their theme demos (see [`Demo`]), not here.
            Kind::Courses | Kind::CourseCategories | Kind::Services | Kind::ServiceCategories | Kind::BookingStaff => 0,
            Kind::Navigation => 1,
        }
    }
}

/// What one request generates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    counts: [u32; KINDS],
    /// Feature images on posts, from Unsplash. Off for a site that must not
    /// load anything from another host.
    pub images: bool,
    /// Posts go up as published, or wait as drafts.
    pub publish: bool,
    /// Whether this is the one-click demo, for the record it leaves.
    pub full: bool,
    /// A theme demo: the plan also writes that theme's own journal, pages,
    /// menu and settings.
    pub demo: Option<Demo>,
}

/// A ready-made site for one of the bundled themes that needs a platform
/// tool's content to look like itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demo {
    /// Lantern: an online academy.
    Academy,
    /// Atrium: an appointment business.
    Studio,
}

impl Demo {
    pub const ALL: [Demo; 2] = [Demo::Academy, Demo::Studio];

    pub fn key(self) -> &'static str {
        match self {
            Demo::Academy => "academy",
            Demo::Studio => "studio",
        }
    }

    pub fn from_key(key: &str) -> Option<Demo> {
        Demo::ALL.into_iter().find(|d| d.key() == key)
    }

    /// The bundled theme it is made for.
    pub fn theme(self) -> &'static str {
        match self {
            Demo::Academy => "lantern",
            Demo::Studio => "atrium",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Demo::Academy => "Online academy",
            Demo::Studio => "Wellness studio",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Demo::Academy => "Seven complete courses with lessons, quizzes, students and reviews, paid courses in the store, plus a learning journal, pages and menu.",
            Demo::Studio => "A booking business: eight services, a team of four with weekly hours, two weeks of bookings, a wellness journal, About and Visit pages, menu and the theme's settings.",
        }
    }

    /// What the demo makes.
    pub fn plan(self, images: bool) -> Plan {
        let mut counts = [0; KINDS];
        let mut set = |kind: Kind, n: u32| counts[Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0)] = n;
        match self {
            Demo::Academy => set(Kind::Courses, SAMPLE_COURSES),
            Demo::Studio => set(Kind::Services, SAMPLE_SERVICES),
        }
        set(Kind::Navigation, 1);
        Plan { counts, images, publish: true, full: false, demo: Some(self) }
    }
}

impl Plan {
    pub fn full(images: bool) -> Plan {
        let mut counts = [0; KINDS];
        for (i, kind) in Kind::ALL.into_iter().enumerate() {
            counts[i] = kind.full();
        }
        Plan { counts, images, publish: true, full: true, demo: None }
    }

    /// A custom plan from the form's fields: `count_<kind>` for each kind,
    /// `images` and `status`. Every number is clamped to its kind's ceiling,
    /// and anything unreadable is zero — a forged field can only ask for less.
    pub fn custom(fields: &[(String, String)]) -> Result<Plan, String> {
        let field = |name: &str| fields.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.trim().to_string());
        let mut counts = [0; KINDS];
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
            demo: None,
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
    pub fn node(&self) -> Value {
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

// ---------------------------------------------------------------------------
// The directory catalogue
// ---------------------------------------------------------------------------
//
// A small city guide: six kinds of place in four cities, every place
// invented, every address plausible and every phone number a fictional one.
// Enough for a directory theme to draw every section it has.

pub struct ListingCategorySeed {
    pub name: &'static str,
    pub slug: &'static str,
    pub description: &'static str,
    pub tint: &'static str,
    pub image: &'static str,
    /// Stored opening hours, the form `vuuut_core::directory::Hours` reads.
    pub hours: &'static str,
    pub payments: &'static [&'static str],
}

fn photo(id: &str) -> String {
    format!("https://images.unsplash.com/photo-{id}?w=1600&q=80")
}

pub const LISTING_CATEGORIES: &[ListingCategorySeed] = &[
    ListingCategorySeed { name: "Restaurants", slug: "restaurants", description: "Where to sit down and eat well, from neighbourhood places to the ones worth booking weeks ahead.", tint: "pink", image: "1517248135467-4c7edcad34c4",
        hours: r#"{"mon":"closed","tue":"12:00-15:00,18:00-22:30","wed":"12:00-15:00,18:00-22:30","thu":"12:00-15:00,18:00-22:30","fri":"12:00-15:00,18:00-23:30","sat":"12:00-15:00,18:00-23:30","sun":"12:00-16:00"}"#,
        payments: &["cash", "credit", "debit", "contactless", "apple-pay"] },
    ListingCategorySeed { name: "Cafés", slug: "cafes", description: "Good coffee, a table by the window and somewhere to read for an hour.", tint: "amber", image: "1501339847302-ac426a4a7cbb",
        hours: r#"{"mon":"07:00-18:00","tue":"07:00-18:00","wed":"07:00-18:00","thu":"07:00-18:00","fri":"07:00-18:00","sat":"08:00-17:00","sun":"08:00-17:00"}"#,
        payments: &["cash", "credit", "contactless", "apple-pay", "google-pay"] },
    ListingCategorySeed { name: "Hotels", slug: "hotels", description: "Places to stay that make the city feel like yours for a few nights.", tint: "blue", image: "1566073771259-6a8506099945",
        hours: r#"{"mon":"24h","tue":"24h","wed":"24h","thu":"24h","fri":"24h","sat":"24h","sun":"24h"}"#,
        payments: &["credit", "debit", "bank"] },
    ListingCategorySeed { name: "Bars", slug: "bars", description: "Cocktail rooms, wine bars and the corner pub everyone ends up in.", tint: "purple", image: "1514933651103-005eec06c04b",
        hours: r#"{"mon":"17:00-01:00","tue":"17:00-01:00","wed":"17:00-01:00","thu":"17:00-01:00","fri":"17:00-02:00","sat":"17:00-02:00","sun":"16:00-00:00"}"#,
        payments: &["cash", "credit", "contactless"] },
    ListingCategorySeed { name: "Museums & galleries", slug: "museums-galleries", description: "Collections large and small, and the galleries showing what comes next.", tint: "teal", image: "1518998053901-5348d3961a04",
        hours: r#"{"mon":"closed","tue":"10:00-18:00","wed":"10:00-18:00","thu":"10:00-21:00","fri":"10:00-18:00","sat":"10:00-18:00","sun":"10:00-18:00"}"#,
        payments: &["credit", "debit", "contactless"] },
    ListingCategorySeed { name: "Shops", slug: "shops", description: "Independent shops: books, records, clothes and things made nearby.", tint: "green", image: "1441986300917-64674bd600d8",
        hours: r#"{"mon":"10:00-19:00","tue":"10:00-19:00","wed":"10:00-19:00","thu":"10:00-19:00","fri":"10:00-19:00","sat":"10:00-19:00","sun":"11:00-17:00"}"#,
        payments: &["cash", "credit", "debit", "contactless", "apple-pay", "google-pay"] },
];

pub struct CitySeed {
    pub name: &'static str,
    pub region: &'static str,
    pub country: &'static str,
    pub postal: &'static str,
    pub phone: &'static str,
    pub lat: f64,
    pub lng: f64,
}

pub const CITIES: &[CitySeed] = &[
    CitySeed { name: "New York", region: "NY", country: "United States", postal: "10012", phone: "+1 212 555 01", lat: 40.7233, lng: -73.9985 },
    CitySeed { name: "Lisbon", region: "Lisboa", country: "Portugal", postal: "1200-195", phone: "+351 21 555 01", lat: 38.7110, lng: -9.1440 },
    CitySeed { name: "Tokyo", region: "Tokyo", country: "Japan", postal: "150-0001", phone: "+81 3 5550 01", lat: 35.6640, lng: 139.6980 },
    CitySeed { name: "London", region: "Greater London", country: "United Kingdom", postal: "E2 7DJ", phone: "+44 20 7946 01", lat: 51.5246, lng: -0.0740 },
];

struct ListingSpec {
    category: usize,
    city: usize,
    title: &'static str,
    summary: &'static str,
    about: &'static str,
    detail: &'static str,
    highlights: &'static [&'static str],
    street: &'static str,
    price_level: i64,
    amenities: &'static [&'static str],
    photos: &'static [&'static str],
    established: &'static str,
}

const LISTINGS: &[ListingSpec] = &[
    // Restaurants
    ListingSpec { category: 0, city: 0, title: "Hearth & Vine", summary: "Wood-fired cooking and a wine list that rewards curiosity.", about: "Everything at Hearth & Vine passes through the open hearth at the back of the room: bread in the morning, whole fish at night, vegetables charred until they are sweet.", detail: "The room is loud in the best way. Book the counter if you want to watch the cooks work.", highlights: &["Wood-roasted half chicken", "Charred leeks with romesco", "Brown butter cake"], street: "475 Driggs Ave", price_level: 3, amenities: &["reservations", "bar", "vegetarian", "groups"], photos: &["1517248135467-4c7edcad34c4", "1414235077428-338989a2e8c0", "1555396273-367ea4eb4db5"], established: "2016" },
    ListingSpec { category: 0, city: 1, title: "Tasca do Largo", summary: "A family tasca where the grilled fish comes straight from the morning market.", about: "Three generations run this narrow dining room off a quiet square. The menu is written on a board and changes with whatever came in from the coast that morning.", detail: "Order the house white by the jug and leave room for the custard tart.", highlights: &["Grilled sardines (in summer)", "Bacalhau à Brás", "Pastel de nata"], street: "Largo do Carmo 14", price_level: 1, amenities: &["outdoor", "kids", "groups"], photos: &["1552566626-52f8b828add9", "1559339352-11d035aa65de"], established: "1962" },
    ListingSpec { category: 0, city: 2, title: "Sumi Counter", summary: "Eight seats around a charcoal grill, and a chef who talks you through each course.", about: "Sumi Counter serves one tasting menu a night built around binchotan charcoal: skewers, rice cooked in the embers, and a clear broth to finish.", detail: "Two sittings, at six and at half past eight. Tell them about allergies when you book.", highlights: &["Seasonal omakase", "Charcoal-grilled wagyu", "Hojicha ice cream"], street: "2-14-5 Shibuya", price_level: 4, amenities: &["reservations", "ac"], photos: &["1414235077428-338989a2e8c0", "1424847651672-bf20a4b0982b"], established: "2019" },
    ListingSpec { category: 0, city: 3, title: "The Long Table", summary: "Shared tables, seasonal British cooking and Sunday roasts worth queueing for.", about: "Everyone sits at long oak tables and the plates come to the middle. The kitchen buys whole animals from two farms in Kent and uses all of them.", detail: "Walk-ins welcome at lunch; dinner is booked out most weekends.", highlights: &["Sunday roast", "Potted shrimp on toast", "Sticky toffee pudding"], street: "28 Redchurch Street", price_level: 2, amenities: &["groups", "kids", "vegetarian", "wheelchair"], photos: &["1555396273-367ea4eb4db5", "1517248135467-4c7edcad34c4"], established: "2012" },
    // Cafés
    ListingSpec { category: 1, city: 0, title: "Morning Standard", summary: "Single-origin pour-overs and the best cardamom bun in SoHo.", about: "A tiny bar with a big roaster in the back. Beans change every fortnight and the baristas will happily tell you why.", detail: "Laptop-friendly before ten and after two; the middle of the day belongs to the queue.", highlights: &["Pour-over flight", "Cardamom bun", "Cold brew tonic"], street: "112 Prince Street", price_level: 1, amenities: &["wifi", "vegan", "takeout"], photos: &["1501339847302-ac426a4a7cbb", "1495474472287-4d71bcdd2085"], established: "2018" },
    ListingSpec { category: 1, city: 1, title: "Café Azulejo", summary: "Tiled walls, strong bica and a terrace looking down to the river.", about: "Café Azulejo has occupied the ground floor of the same tiled building since the 1930s. The coffee is strong and quick, the pastries are made upstairs.", detail: "Take the terrace table on the left for the view of the Tagus.", highlights: &["Bica and a pastel de nata", "Torrada with salted butter", "Galão"], street: "Rua da Misericórdia 8", price_level: 1, amenities: &["outdoor", "wifi", "breakfast"], photos: &["1554118811-1e0d58224f24", "1509042239860-f550ce710b93"], established: "1934" },
    ListingSpec { category: 1, city: 2, title: "Kissa Lumen", summary: "A hushed jazz kissaten with siphon coffee and thick toast.", about: "Records play on a 1970s system while coffee brews in glass siphons along the bar. Talking is allowed, but most people come to listen.", detail: "No photos of other guests, and the last order is at half past five.", highlights: &["Siphon blend", "Thick toast with red bean", "Coffee jelly"], street: "1-3-7 Jingumae", price_level: 2, amenities: &["ac", "breakfast"], photos: &["1495474472287-4d71bcdd2085", "1445116572660-236099ec97a0"], established: "1978" },
    ListingSpec { category: 1, city: 3, title: "Common Grounds", summary: "A bright corner café with long tables, good Wi-Fi and serious toast.", about: "Common Grounds is half café, half workspace: power at every seat, a quiet room upstairs, and sourdough from the bakery two doors down.", detail: "Dogs welcome; there is a water bowl by the door.", highlights: &["Flat white", "Sourdough with ricotta and honey", "Banana bread"], street: "9 Columbia Road", price_level: 1, amenities: &["wifi", "pets", "vegan", "wheelchair"], photos: &["1509042239860-f550ce710b93", "1521017432531-fbd92d768814"], established: "2015" },
    // Hotels
    ListingSpec { category: 2, city: 0, title: "The Mercer Loft Hotel", summary: "Forty-two rooms in a converted printing works, with a rooftop that faces the bridge.", about: "Big windows, high ceilings and beds made with linen from a mill in Maine. The rooftop bar opens to guests at five.", detail: "Ask for a room on the north side for the skyline at night.", highlights: &["Rooftop bar", "Bicycles to borrow", "Late check-out on Sundays"], street: "201 Mercer Street", price_level: 4, amenities: &["wifi", "gym", "bar", "ac", "wheelchair"], photos: &["1551882547-ff40c63fe5fa", "1611892440504-42a792e24d32", "1582719478250-c89cae4dc85b"], established: "2014" },
    ListingSpec { category: 2, city: 1, title: "Casa dos Telhados", summary: "A guesthouse of twelve rooms above the rooftops of Alfama.", about: "Every room has a balcony and every balcony has a view. Breakfast is served on the terrace from eight until eleven.", detail: "The streets around are steep and cobbled; staff will meet you at the tram stop with a trolley.", highlights: &["Terrace breakfast", "River-view rooms", "Port tasting on Fridays"], street: "Rua de São Miguel 31", price_level: 2, amenities: &["wifi", "breakfast", "ac"], photos: &["1542314831-068cd1dbfeeb", "1566073771259-6a8506099945"], established: "2011" },
    ListingSpec { category: 2, city: 2, title: "Hotel Kōen", summary: "A calm hotel beside the park, with a hot bath on every floor.", about: "Rooms are small, quiet and perfectly arranged. On the top floor a public bath looks over the trees of the park.", detail: "Two minutes from the station; the front desk keeps umbrellas for guests.", highlights: &["Rooftop bath", "Japanese breakfast", "Park-view rooms"], street: "5-1-2 Yoyogi", price_level: 3, amenities: &["wifi", "breakfast", "ac", "wheelchair"], photos: &["1566073771259-6a8506099945", "1520250497591-112f2f40a3f4"], established: "2017" },
    ListingSpec { category: 2, city: 3, title: "The Brickfield", summary: "A Victorian warehouse turned hotel, with a pool in the old boiler room.", about: "The Brickfield keeps the warehouse's iron columns and adds deep baths and a heated pool in what used to be the boiler room.", detail: "The bar downstairs is open to non-guests and fills up after six.", highlights: &["Boiler-room pool", "Cocktail bar", "Weekend brunch"], street: "45 Shoreditch High Street", price_level: 3, amenities: &["wifi", "pool", "gym", "bar", "pets"], photos: &["1520250497591-112f2f40a3f4", "1551882547-ff40c63fe5fa"], established: "2020" },
    // Bars
    ListingSpec { category: 3, city: 0, title: "Low Tide", summary: "A dim, narrow cocktail bar with a menu that changes with the season.", about: "Twelve stools, one bartender per six guests and a menu written each season around a single ingredient.", detail: "No reservations. Put your name down at the door and they will text you.", highlights: &["Seasonal martini", "Smoked old fashioned", "Zero-proof spritz"], street: "38 Orchard Street", price_level: 3, amenities: &["late", "bar"], photos: &["1514933651103-005eec06c04b", "1470337458703-46ad1756a187"], established: "2017" },
    ListingSpec { category: 3, city: 1, title: "Ginjinha Sem Rival", summary: "Cherry liqueur poured in a doorway, drunk on the street.", about: "A doorway bar no wider than a wardrobe, pouring ginjinha since before anyone can remember. Ask for it with the cherry or without.", detail: "Cash only and standing room only, which is to say the whole street.", highlights: &["Ginjinha in a chocolate cup", "Eduardino", "Vinho verde"], street: "Rua das Portas de Santo Antão 7", price_level: 1, amenities: &["late", "outdoor"], photos: &["1572116469696-31de0f17cc34", "1543007630-9710e4a00a20"], established: "1890" },
    ListingSpec { category: 3, city: 2, title: "Bar Hoshi", summary: "Whisky and highballs, eight floors up, with the city lit below.", about: "A quiet bar with more than four hundred Japanese whiskies, poured with care and served with the view through floor-to-ceiling glass.", detail: "Smart casual. A cover charge includes a small plate of snacks.", highlights: &["Whisky highball", "Yuzu sour", "Tasting flights"], street: "3-18-9 Shinjuku", price_level: 4, amenities: &["late", "bar", "ac"], photos: &["1470337458703-46ad1756a187", "1514933651103-005eec06c04b"], established: "2008" },
    ListingSpec { category: 3, city: 3, title: "The Crooked Anchor", summary: "A proper pub: real ale, a fire in winter and quiz night on Tuesdays.", about: "The Crooked Anchor has served the same street for two hundred years. Six cask ales, a wood fire and a landlord who remembers your order.", detail: "Quiz night every Tuesday at eight; teams of up to six.", highlights: &["Cask ales", "Scotch eggs", "Tuesday quiz"], street: "17 Arnold Circus", price_level: 1, amenities: &["pets", "groups", "outdoor", "live-music"], photos: &["1543007630-9710e4a00a20", "1572116469696-31de0f17cc34"], established: "1824" },
    // Museums & galleries
    ListingSpec { category: 4, city: 0, title: "Lantern Gallery", summary: "New photography and print in a former bank, free to visit.", about: "Lantern shows emerging photographers in the old banking hall, with a print room downstairs where most work is for sale.", detail: "Free entry. Late opening with talks on Thursdays.", highlights: &["Main hall exhibition", "Print room", "Thursday talks"], street: "88 Bowery", price_level: 1, amenities: &["wheelchair", "kids"], photos: &["1554907984-15263bfd63bd", "1566127444979-b3d2b654e3d7"], established: "2010" },
    ListingSpec { category: 4, city: 1, title: "Museu do Azulejo Novo", summary: "Five centuries of painted tiles in a restored convent.", about: "Room after room of tile panels, from Moorish patterns to modern commissions, in a convent whose chapel is a work of art on its own.", detail: "Allow two hours. The café in the cloister is worth the stop.", highlights: &["The great Lisbon panorama", "The gilded chapel", "Cloister café"], street: "Rua da Madre de Deus 4", price_level: 2, amenities: &["wheelchair", "kids", "groups"], photos: &["1518998053901-5348d3961a04", "1564399579883-451a5d44ec08"], established: "1965" },
    ListingSpec { category: 4, city: 2, title: "Ukiyo House", summary: "Woodblock prints, shown a few at a time in a quiet townhouse.", about: "A private collection of woodblock prints shown in rotation, twenty at a time, so each one can be looked at properly.", detail: "Shoes off at the door; slippers are provided.", highlights: &["Rotating print display", "Printmaking workshop", "Tea room"], street: "1-10-10 Jingumae", price_level: 2, amenities: &["ac", "groups"], photos: &["1518998053901-5348d3961a04", "1554907984-15263bfd63bd"], established: "1980" },
    ListingSpec { category: 4, city: 3, title: "The Foundry Museum", summary: "Industrial history and hands-on workshops in a working foundry.", about: "The Foundry tells the story of the East End's makers in the buildings they worked in, with casting demonstrations every afternoon.", detail: "Children under twelve go free; workshops need booking.", highlights: &["Casting demonstrations", "Makers' workshops", "Archive tours"], street: "3 Whitechapel Road", price_level: 1, amenities: &["wheelchair", "kids", "groups"], photos: &["1564399579883-451a5d44ec08", "1518998053901-5348d3961a04"], established: "1998" },
    // Shops
    ListingSpec { category: 5, city: 0, title: "Paper & Pulp Books", summary: "An independent bookshop with a strong poetry shelf and a reading room.", about: "Floor-to-ceiling shelves, staff picks with handwritten notes and a reading room at the back where nobody hurries you.", detail: "Readings most Wednesdays; the newsletter lists them.", highlights: &["Staff picks", "Poetry room", "Wednesday readings"], street: "61 Spring Street", price_level: 2, amenities: &["wheelchair", "kids", "wifi"], photos: &["1472851294608-062f824d29cc", "1567401893414-76b7b1e5a7a5"], established: "2003" },
    ListingSpec { category: 5, city: 1, title: "Loja das Conservas", summary: "Tinned fish from every cannery on the coast, wrapped like presents.", about: "The walls are lined with tins from more than a dozen Portuguese canneries, arranged by year and region, with staff who know each one.", detail: "They ship anywhere in Europe.", highlights: &["Sardines in olive oil", "Gift boxes", "Tasting counter"], street: "Rua do Arsenal 130", price_level: 2, amenities: &["delivery", "wheelchair"], photos: &["1488459716781-31db52582fe9", "1542838132-92c53300491e"], established: "2013" },
    ListingSpec { category: 5, city: 2, title: "Kōgei Supply", summary: "Everyday things made by hand: knives, ceramics, cloth and paper.", about: "Kōgei Supply works directly with about forty workshops across Japan and sells what they make for daily use, not for display.", detail: "Knife sharpening on Saturdays.", highlights: &["Kitchen knives", "Mino ceramics", "Washi notebooks"], street: "4-26-14 Jingumae", price_level: 3, amenities: &["ac", "delivery"], photos: &["1567401893414-76b7b1e5a7a5", "1555529669-e69e7aa0ba9a"], established: "2015" },
    ListingSpec { category: 5, city: 3, title: "Spinning Wax Records", summary: "New and second-hand vinyl, listening booths and a Saturday DJ.", about: "Racks of soul, jazz and electronic records, new and used, with three listening booths and a turntable repair counter.", detail: "Saturday afternoons a guest DJ plays from the shop window.", highlights: &["Second-hand soul", "Listening booths", "Saturday DJ"], street: "120 Brick Lane", price_level: 2, amenities: &["live-music", "pets"], photos: &["1461360228754-6e81c478b882", "1483412033650-1015ddeb83d1"], established: "1994" },
];

/// A generated listing.
pub struct DummyListing {
    pub category: usize,
    pub title: &'static str,
    pub summary: &'static str,
    pub lexical: String,
    pub street: &'static str,
    pub city: &'static CitySeed,
    pub latitude: String,
    pub longitude: String,
    pub phone: String,
    pub website: String,
    pub email: String,
    pub hours: &'static str,
    pub price_level: i64,
    pub amenities: Vec<String>,
    pub payments: Vec<String>,
    pub socials: Vec<(String, String)>,
    pub photos: Vec<String>,
    pub established: &'static str,
    pub keywords: String,
    pub featured: bool,
}

impl ListingCategorySeed {
    pub fn image_url(&self) -> String {
        photo(self.image)
    }
}

impl Generator {
    /// The first `n` listings, spread over every city before any repeats.
    pub fn listings(&mut self, n: u32) -> Vec<DummyListing> {
        // Round by round, one place per category, each round shifting the
        // cities, so a small request still covers every city and category.
        let mut spread: Vec<&ListingSpec> = Vec::new();
        for round in 0..CITIES.len() {
            for cat in 0..LISTING_CATEGORIES.len() {
                if let Some(l) = LISTINGS.iter().find(|l| l.category == cat && l.city == (round + cat) % CITIES.len()) {
                    spread.push(l);
                }
            }
        }
        spread
            .into_iter()
            .take(n as usize)
            .enumerate()
            .map(|(i, l)| {
                let city = &CITIES[l.city];
                let cat = &LISTING_CATEGORIES[l.category];
                let domain = format!("{}.example", crate::directory::slugify(l.title).replace('-', ""));
                let offset = |k: usize| ((k * 7919 + i * 104729) % 200) as f64 / 10_000.0 - 0.01;
                let lexical = document(vec![
                    paragraph(l.about),
                    paragraph(l.detail),
                    heading("What to try", "h3"),
                    list(l.highlights),
                ]);
                DummyListing {
                    category: l.category,
                    title: l.title,
                    summary: l.summary,
                    lexical,
                    street: l.street,
                    city,
                    latitude: format!("{:.5}", city.lat + offset(1)),
                    longitude: format!("{:.5}", city.lng + offset(2)),
                    phone: format!("{}{:02}", city.phone, 10 + i),
                    website: format!("https://{domain}/"),
                    email: format!("hello@{domain}"),
                    hours: cat.hours,
                    price_level: l.price_level,
                    amenities: l.amenities.iter().map(|s| s.to_string()).collect(),
                    payments: cat.payments.iter().map(|s| s.to_string()).collect(),
                    socials: vec![
                        ("instagram".into(), format!("https://instagram.com/{}", crate::directory::slugify(l.title).replace('-', ""))),
                        ("facebook".into(), format!("https://facebook.com/{}", crate::directory::slugify(l.title).replace('-', ""))),
                    ],
                    photos: l.photos.iter().map(|p| photo(p)).collect(),
                    established: l.established,
                    keywords: format!("{} {} {}", cat.name.to_lowercase(), city.name.to_lowercase(), l.highlights.join(" ").to_lowercase()),
                    featured: i % 4 == 0,
                }
            })
            .collect()
    }
}

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
            assert!(plan.count(kind) > 0 || kind.full() == 0, "{kind:?}");
        }
        assert_eq!(plan.summary(), "6 tags, 3 authors, 16 posts, 5 pages, 40 members, 6 products, 6 directory categories, 24 directory listings and navigation links");
    }

    #[test]
    fn the_directory_catalogue_covers_every_city_and_category_without_repeats() {
        let mut g = Generator::new(7);
        let six = g.listings(6);
        let cats: std::collections::HashSet<usize> = six.iter().map(|l| l.category).collect();
        assert_eq!(cats.len(), 6, "a small request still has one of each kind of place");
        let all = g.listings(Kind::Listings.max());
        assert_eq!(all.len(), 24);
        let titles: std::collections::HashSet<&str> = all.iter().map(|l| l.title).collect();
        assert_eq!(titles.len(), 24);
        for l in &all {
            assert!(l.website.ends_with(".example/") && l.email.ends_with(".example"), "{}", l.title);
            assert!(!crate::directory::Hours::from_json(l.hours).is_empty(), "{}", l.title);
            assert!(l.latitude.parse::<f64>().is_ok() && l.longitude.parse::<f64>().is_ok());
            assert!(!l.photos.is_empty() && (1..=4).contains(&l.price_level));
        }
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
