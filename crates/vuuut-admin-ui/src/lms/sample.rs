//! "Load a sample academy": seven real courses, with sections, written
//! lessons, a video, quizzes, assignments, categories, a few demonstration
//! students (`@example.com`, never emailed) with progress, and reviews — so
//! a new site, and a theme, can be seen working before anyone writes a
//! course. Everything is ordinary content afterwards: edit or delete it.
//!
//! Lesson bodies are built as Lexical (paragraphs, headings, lists) through
//! `vuuut_core::dummies::Block`, so they open in the editor as normal text.

use super::*;
use vuuut_core::dummies::{document, Block, Block::*};

const IMG: &str = "https://images.unsplash.com/photo-";
const VIDEO: &str = "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4";

struct Cat {
    name: &'static str,
    slug: &'static str,
    tint: &'static str,
    image: &'static str,
    description: &'static str,
}

const CATS: &[Cat] = &[
    Cat { name: "Development", slug: "development", tint: "blue", image: "1542831371-29b0f74f9713", description: "Build for the web and work with data, one solid step at a time." },
    Cat { name: "Design", slug: "design", tint: "pink", image: "1561070791-2526d30994b5", description: "Type, colour and layout — the craft behind interfaces people love." },
    Cat { name: "Business", slug: "business", tint: "amber", image: "1454165804606-c3d57bc86b40", description: "Start, run and grow something of your own." },
    Cat { name: "Photography", slug: "photography", tint: "teal", image: "1452587925148-ce544e77e70d", description: "See light, frame stories and take control of your camera." },
    Cat { name: "Writing", slug: "writing", tint: "purple", image: "1471107340929-a87cd0f5b5f3", description: "Say what you mean, clearly, and enjoy doing it." },
    Cat { name: "Wellbeing", slug: "wellbeing", tint: "green", image: "1544367567-0f2fcb009e0b", description: "Small daily practices for a calmer, stronger you." },
];

/// `(prompt, kind, options as "text" with a leading `*` for right answers, accepted answer, explanation)`.
type Q = (&'static str, &'static str, &'static [&'static str], &'static str, &'static str);

enum Item {
    Lesson { title: &'static str, minutes: i64, preview: bool, video: bool, body: &'static [Block] },
    Quiz { title: &'static str, minutes: i64, intro: &'static [Block], questions: &'static [Q] },
    Task { title: &'static str, minutes: i64, body: &'static [Block] },
}

struct Course {
    title: &'static str,
    subtitle: &'static str,
    cat: &'static str,
    image: &'static str,
    level: &'static str,
    access: &'static str,
    price: i64,
    compare: i64,
    featured: bool,
    drip: bool,
    about: &'static [Block],
    outcomes: &'static str,
    requirements: &'static str,
    audience: &'static str,
    sections: &'static [(&'static str, &'static [Item])],
}

const COURSES: &[Course] = &[
    Course {
        title: "Web Development Foundations",
        subtitle: "HTML, CSS and your first lines of JavaScript — build and publish a real page in a weekend.",
        cat: "development", image: "1517694712202-14dd9538aa97", level: "beginner", access: "free", price: 0, compare: 0, featured: true, drip: false,
        about: &[
            P("The web is the largest publishing platform ever built, and its three languages are small enough to learn in a weekend. This course takes you from an empty folder to a finished, responsive page you can put online and be proud of."),
            P("Every lesson is short and practical. You will write code from the first hour, check your understanding with quick quizzes, and finish by building a profile card that you hand in for feedback."),
            H("How the course works"),
            L(&["Watch or read a lesson, then try the exercise in your own editor.", "Take the quiz at the end of each section — you can retry as often as you like.", "Hand in the final project and get written feedback from your instructor."]),
        ],
        outcomes: "Explain how browsers, servers and URLs work together\nWrite clean, semantic HTML for any page\nStyle pages with CSS, including Flexbox and Grid layouts\nMake a layout that works on phones and desktops\nAdd simple interactivity with JavaScript\nPublish a page on the open web",
        requirements: "A computer with a modern browser\nA free code editor such as VS Code\nNo previous coding experience",
        audience: "Complete beginners who want to understand how websites are made\nDesigners who want to build what they draw\nAnyone starting a career in tech",
        sections: &[
            ("Getting started", &[
                Item::Lesson { title: "Welcome — what you will build", minutes: 4, preview: true, video: true, body: &[
                    P("Welcome! Over the next few hours you will build a personal profile page from scratch: a photo, a short introduction, a list of skills and links to your work. It will look good on any screen."),
                    P("Each section ends with a short quiz. They are not tests to pass or fail — they are a quick way to check what has stuck before you move on."),
                    H("Before you start"),
                    L(&["Create a folder called my-site somewhere easy to find.", "Install a code editor — VS Code is free and works everywhere.", "Keep this course open in one window and your editor in another."]),
                ] },
                Item::Lesson { title: "How the web works", minutes: 9, preview: true, video: false, body: &[
                    P("When you type an address into your browser, three things happen. The browser asks the domain name system where the site lives, it sends a request to that server, and the server replies with files — usually HTML, CSS, JavaScript and images."),
                    P("HTML describes what is on the page: headings, paragraphs, images, links. CSS describes how it looks: colours, fonts, spacing, layout. JavaScript describes how it behaves when someone clicks, types or scrolls."),
                    H("The one idea to keep"),
                    P("Content, presentation and behaviour are kept apart. That separation is why the same article can look completely different on a phone, in a reader app, or read aloud by a screen reader."),
                ] },
                Item::Lesson { title: "Setting up your editor", minutes: 7, preview: false, video: false, body: &[
                    P("Open your my-site folder in your editor and create a file called index.html. Type an exclamation mark and press Tab — most editors expand it into a complete starter page."),
                    L(&["Install the Live Server extension so the page reloads as you save.", "Turn on Format on Save so your code stays tidy.", "Pick a font size that is comfortable — you will be reading a lot of code."]),
                    P("Open the page in your browser. It is empty, but it is yours, and it is a real web page."),
                ] },
            ]),
            ("HTML: structure and meaning", &[
                Item::Lesson { title: "Elements, tags and attributes", minutes: 12, preview: false, video: false, body: &[
                    P("An HTML element is made of an opening tag, some content and a closing tag. A paragraph is written <p>Hello</p>. Some elements, like images, have no content and no closing tag."),
                    P("Attributes add information to an element. A link needs an href to say where it goes; an image needs a src for the file and an alt that describes it for people who cannot see it."),
                    H("Try it"),
                    L(&["Add an h1 with your name.", "Add a paragraph that introduces you in two sentences.", "Add a link to a site you like, opening in the same tab."]),
                ] },
                Item::Lesson { title: "Semantic HTML", minutes: 10, preview: false, video: false, body: &[
                    P("Semantic elements say what a part of the page is, not just how it looks. header, nav, main, article, section and footer give your page a shape that browsers, search engines and assistive technology understand."),
                    P("A good rule: if you are reaching for a div, ask whether a more meaningful element exists. Usually it does."),
                    L(&["Wrap your introduction in a header.", "Put your skills list inside a section with its own h2.", "Add a footer with a copyright line."]),
                ] },
                Item::Quiz { title: "Quiz: HTML basics", minutes: 5, intro: &[P("Five quick questions on what you have just learned. You need 60% to pass, and you can try again as many times as you like.")], questions: &[
                    ("Which element is the most important heading on a page?", "single", &["<header>", "*<h1>", "<head>", "<title>"], "", "There should usually be one h1 per page; header is a landmark, head holds metadata."),
                    ("Which attribute describes an image for people who cannot see it?", "single", &["title", "src", "*alt", "href"], "", "Screen readers announce the alt text in place of the image."),
                    ("Which of these are semantic landmark elements?", "multiple", &["*<nav>", "<div>", "*<main>", "*<footer>", "<span>"], "", "div and span carry no meaning; nav, main and footer do."),
                    ("HTML describes how a page looks.", "truefalse", &["False"], "", "HTML describes structure and meaning; CSS describes presentation."),
                    ("What attribute tells a link where to go?", "text", &[], "href", "The href attribute holds the destination address."),
                ] },
            ]),
            ("CSS: style and layout", &[
                Item::Lesson { title: "Selectors and the cascade", minutes: 14, preview: false, video: false, body: &[
                    P("CSS rules pair a selector with declarations: h1 { color: navy; } turns every h1 navy. Class selectors (.card) let you style exactly the elements you choose."),
                    P("When two rules disagree, the cascade decides: more specific selectors win, and later rules beat earlier ones of the same specificity. Keep selectors simple and you will rarely have to think about it."),
                    L(&["Link a styles.css file from your page's head.", "Set a comfortable font and line height on the body.", "Give your header a background colour and some padding."]),
                ] },
                Item::Lesson { title: "Layout with Flexbox and Grid", minutes: 18, preview: false, video: true, body: &[
                    P("Flexbox lays items out in one direction — a row of buttons, a navigation bar, a card with an image beside text. Grid lays them out in two — a gallery, a page with a sidebar, a dashboard."),
                    H("A responsive grid in one line"),
                    P("grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)) gives you as many columns as fit, each at least 220 pixels wide. No media queries needed."),
                    L(&["Use Flexbox to put your photo beside your introduction.", "Use Grid to lay out your skills as cards.", "Resize the window and watch both adapt."]),
                ] },
                Item::Task { title: "Project: your profile card", minutes: 60, body: &[
                    P("Bring everything together. Build a profile card page with a photo, your name, a short bio, a grid of at least four skills and links to two places you can be found online."),
                    H("What we look for"),
                    L(&["Semantic HTML with a sensible heading order.", "A layout that works from 360px wide upwards.", "Readable type and enough contrast.", "Clean, consistently indented code."]),
                    P("Publish it (GitHub Pages and Netlify are free) and hand in the link, with a sentence about the part you are proudest of."),
                ] },
            ]),
            ("Next steps", &[
                Item::Lesson { title: "A first taste of JavaScript", minutes: 15, preview: false, video: false, body: &[
                    P("JavaScript makes pages respond. Add a button to your profile that switches between a light and a dark theme: listen for a click, then toggle a class on the body."),
                    P("You do not need a framework to do useful things. The browser gives you everything to read the page, change it and react to people using it."),
                ] },
                Item::Quiz { title: "Final quiz", minutes: 8, intro: &[P("Ten minutes, all topics. Pass with 70% to finish the course and receive your certificate.")], questions: &[
                    ("Which language controls layout and colour?", "single", &["HTML", "*CSS", "JavaScript", "HTTP"], "", ""),
                    ("Which CSS layout system works in two dimensions?", "single", &["Flexbox", "*Grid", "Float", "Inline-block"], "", "Grid handles rows and columns at once."),
                    ("A later CSS rule beats an earlier one of the same specificity.", "truefalse", &["True"], "", ""),
                    ("Which are good reasons to write semantic HTML?", "multiple", &["*Accessibility", "*Search engines understand the page", "It makes pages load slower", "*The code is easier to read"], "", ""),
                    ("What does the browser ask first to find a site: the DNS or the server?", "text", &[], "dns|the dns|domain name system", "The domain name system turns the name into an address."),
                ] },
            ]),
        ],
    },
    Course {
        title: "Python for Data Analysis",
        subtitle: "Clean, explore and chart real data with Python and pandas — no statistics degree required.",
        cat: "development", image: "1551288049-bebda4e38f71", level: "intermediate", access: "paid", price: 7900, compare: 12900, featured: true, drip: false,
        about: &[
            P("Spreadsheets are wonderful until they are not. When the data gets big, messy or repetitive, a few lines of Python do in seconds what takes an afternoon by hand — and they do it the same way every time."),
            P("You will work through a real dataset from first look to finished charts, learning the handful of pandas tools that cover most everyday analysis."),
        ],
        outcomes: "Load CSV and Excel files into pandas\nClean messy columns, dates and missing values\nGroup, filter and summarise data confidently\nMake clear charts that answer a question\nShare a reproducible analysis notebook",
        requirements: "Basic Python (variables, lists, loops)\nPython 3 and Jupyter installed, or a free Google Colab account",
        audience: "Analysts who live in spreadsheets\nDevelopers who want to work with data\nResearchers and students",
        sections: &[
            ("Setting up", &[
                Item::Lesson { title: "Why pandas?", minutes: 6, preview: true, video: false, body: &[
                    P("pandas gives Python a table — the DataFrame — with rows, named columns and hundreds of operations that work on whole columns at once. If you have used a spreadsheet, you already understand the idea."),
                    L(&["Install with pip install pandas matplotlib.", "Or open a new notebook in Google Colab, where both are ready."]),
                ] },
                Item::Lesson { title: "Your first DataFrame", minutes: 14, preview: false, video: false, body: &[
                    P("Read a CSV with pd.read_csv('sales.csv'). Look before you leap: df.head() shows the first rows, df.info() the columns and their types, df.describe() a quick summary of the numbers."),
                    P("Most surprises in analysis come from data you did not look at. Make these three calls a habit."),
                ] },
            ]),
            ("Cleaning and shaping", &[
                Item::Lesson { title: "Missing values and types", minutes: 16, preview: false, video: false, body: &[
                    P("Real data has holes. df.isna().sum() counts them per column. Decide deliberately: drop the rows, fill with a sensible value, or keep them and note it."),
                    L(&["Convert dates with pd.to_datetime.", "Strip currency symbols before converting prices to numbers.", "Use categories for columns with few distinct values."]),
                ] },
                Item::Lesson { title: "Group, filter, summarise", minutes: 20, preview: false, video: false, body: &[
                    P("df[df.region == 'North'] filters rows. df.groupby('region')['revenue'].sum() answers 'how much did each region make?' in one line. Add .sort_values() and you have a ranking."),
                ] },
                Item::Quiz { title: "Quiz: pandas essentials", minutes: 6, intro: &[P("Check the core ideas before charting.")], questions: &[
                    ("Which call shows column names and types?", "single", &["df.head()", "*df.info()", "df.plot()", "df.sum()"], "", ""),
                    ("Which method counts missing values per column when followed by .sum()?", "text", &[], "isna|df.isna()|isnull", ""),
                    ("groupby is used to split data into groups and summarise each.", "truefalse", &["True"], "", ""),
                ] },
            ]),
            ("Charts and the final project", &[
                Item::Lesson { title: "Charts that answer questions", minutes: 18, preview: false, video: false, body: &[
                    P("A chart should answer one question. Bars compare categories, lines show change over time, scatter plots show relationships. Title the chart with the answer, not the variable names."),
                ] },
                Item::Task { title: "Project: analyse a dataset", minutes: 90, body: &[
                    P("Pick a public dataset that interests you. Ask three questions of it, answer each with a table or a chart, and write a paragraph of conclusions. Hand in a link to your notebook."),
                ] },
            ]),
        ],
    },
    Course {
        title: "UI Design Essentials",
        subtitle: "Typography, colour, spacing and layout — the fundamentals behind every beautiful interface.",
        cat: "design", image: "1558655146-9f40138edfeb", level: "beginner", access: "paid", price: 5900, compare: 0, featured: true, drip: false,
        about: &[
            P("Good interfaces are not decorated; they are arranged. This course teaches the small set of principles — hierarchy, rhythm, contrast and restraint — that make designs feel calm, clear and trustworthy."),
            P("You will redesign a cluttered settings screen step by step, and finish with a portfolio piece."),
        ],
        outcomes: "Choose and pair typefaces with confidence\nBuild a colour palette with accessible contrast\nUse a spacing scale for consistent rhythm\nCreate clear visual hierarchy\nCritique and improve existing screens",
        requirements: "Any design tool — Figma's free plan is perfect",
        audience: "Developers who want their work to look considered\nNew product designers\nFounders designing their first product",
        sections: &[
            ("Seeing like a designer", &[
                Item::Lesson { title: "Hierarchy in five seconds", minutes: 8, preview: true, video: true, body: &[
                    P("Look at any screen for five seconds and look away. What do you remember? That is its hierarchy. Size, weight, colour and position decide what is noticed first."),
                ] },
                Item::Lesson { title: "Typography that reads", minutes: 15, preview: false, video: false, body: &[
                    P("Body text between 16 and 18 pixels, lines of 60–75 characters and a line height around 1.5 make reading effortless. Use one family for most things and a second, at most, for contrast."),
                    L(&["Limit yourself to three or four sizes.", "Use weight before colour for emphasis.", "Tighten letter spacing on big headings, never on body text."]),
                ] },
            ]),
            ("Colour and space", &[
                Item::Lesson { title: "Building a palette", minutes: 14, preview: false, video: false, body: &[
                    P("Start with one brand colour, a range of neutrals and three status colours (success, warning, danger). Generate tints and shades for each so you never invent a colour on the spot."),
                    P("Check contrast: body text needs at least 4.5:1 against its background."),
                ] },
                Item::Lesson { title: "A spacing scale", minutes: 10, preview: false, video: false, body: &[
                    P("Pick a base of 4 or 8 pixels and only use multiples: 4, 8, 12, 16, 24, 32, 48, 64. Related things sit closer together than unrelated ones — that alone fixes most layouts."),
                ] },
                Item::Quiz { title: "Quiz: colour and type", minutes: 5, intro: &[P("Three questions to lock in the fundamentals.")], questions: &[
                    ("What is the minimum contrast ratio for body text (WCAG AA)?", "single", &["2:1", "3:1", "*4.5:1", "7:1"], "", ""),
                    ("Which make text easier to read?", "multiple", &["*Lines of 60–75 characters", "*Line height around 1.5", "All caps paragraphs", "Centred long paragraphs"], "", ""),
                    ("Related elements should sit closer together than unrelated ones.", "truefalse", &["True"], "", "This is the law of proximity."),
                ] },
            ]),
            ("Putting it together", &[
                Item::Task { title: "Project: redesign a settings screen", minutes: 120, body: &[
                    P("Take the cluttered settings screen from the downloads and redesign it using everything from the course. Hand in a link to your file and a before-and-after image, with a short note on three decisions you made."),
                ] },
            ]),
        ],
    },
    Course {
        title: "Photography: From Auto to Manual",
        subtitle: "Understand exposure, light and composition, and take the photos you actually see.",
        cat: "photography", image: "1502920917128-1aa500764cbd", level: "beginner", access: "members", price: 0, compare: 0, featured: false, drip: false,
        about: &[
            P("Automatic mode is clever, but it does not know what you are trying to say. Learn the three settings that control every exposure and you will never be at the mercy of the camera again."),
        ],
        outcomes: "Balance aperture, shutter speed and ISO\nRead and use natural light\nCompose with intention\nShoot confidently in manual mode",
        requirements: "Any camera with manual controls, or a phone with a pro mode",
        audience: "New camera owners\nPhone photographers ready for more control",
        sections: &[
            ("Exposure", &[
                Item::Lesson { title: "The exposure triangle", minutes: 12, preview: true, video: false, body: &[
                    P("Aperture controls how much is in focus, shutter speed controls motion, and ISO controls sensitivity — and noise. Change one and you must balance another."),
                ] },
                Item::Lesson { title: "Shooting in manual", minutes: 15, preview: false, video: false, body: &[
                    P("Set your aperture for the look you want, your shutter speed fast enough to freeze movement, then raise ISO only until the meter reads zero."),
                ] },
            ]),
            ("Light and composition", &[
                Item::Lesson { title: "Finding good light", minutes: 11, preview: false, video: false, body: &[
                    P("The hour after sunrise and before sunset gives soft, warm, directional light. On grey days, the whole sky is a giant softbox — perfect for portraits."),
                ] },
                Item::Lesson { title: "Composition beyond the rule of thirds", minutes: 13, preview: false, video: false, body: &[
                    P("Leading lines, framing, negative space and layers give a photograph depth and direction. Fill the frame, then step back and simplify."),
                ] },
                Item::Task { title: "Assignment: one subject, five ways", minutes: 45, body: &[
                    P("Photograph one ordinary object in five different lights or compositions. Hand in a link to the set and tell us which works best and why."),
                ] },
            ]),
        ],
    },
    Course {
        title: "The Writing Habit",
        subtitle: "Clear, confident prose in fifteen minutes a day — for emails, essays and everything between.",
        cat: "writing", image: "1488190211105-8b0e65b80b4e", level: "all", access: "free", price: 0, compare: 0, featured: false, drip: false,
        about: &[P("Clear writing is clear thinking made visible. This short course gives you a daily practice and a handful of editing habits that make anything you write shorter, warmer and easier to act on.")],
        outcomes: "Write a first draft without stalling\nCut a draft by a third without losing meaning\nWrite emails people answer\nKeep a sustainable daily practice",
        requirements: "A notebook or a blank document",
        audience: "Anyone who writes at work\nAspiring bloggers and essayists",
        sections: &[
            ("Drafting", &[
                Item::Lesson { title: "The fifteen-minute draft", minutes: 6, preview: true, video: false, body: &[
                    P("Set a timer for fifteen minutes and do not stop typing. No editing, no deleting. You are shovelling sand into the box; later you will build castles."),
                ] },
                Item::Lesson { title: "Know your one sentence", minutes: 8, preview: false, video: false, body: &[
                    P("Before you edit, write the single sentence your reader should remember. Everything that does not serve it is a candidate for cutting."),
                ] },
            ]),
            ("Editing", &[
                Item::Lesson { title: "Cut a third", minutes: 10, preview: false, video: false, body: &[
                    L(&["Delete throat-clearing openings.", "Swap long words for short ones.", "Turn passive sentences active.", "Read it aloud — your ear finds what your eye misses."]),
                ] },
                Item::Quiz { title: "Quick check", minutes: 3, intro: &[P("Two questions.")], questions: &[
                    ("When should you edit during a fifteen-minute draft?", "single", &["Every paragraph", "*Not at all", "Every sentence", "Only spelling"], "", ""),
                    ("Reading aloud helps you find awkward sentences.", "truefalse", &["True"], "", ""),
                ] },
            ]),
        ],
    },
    Course {
        title: "Starting a Small Business",
        subtitle: "From idea to first paying customers — validate, price, launch and keep the books straight.",
        cat: "business", image: "1507679799987-c73779587ccf", level: "beginner", access: "paid", price: 9900, compare: 14900, featured: false, drip: false,
        about: &[
            P("Most new businesses fail for predictable reasons: nobody needed the thing, it was priced wrong, or the money ran out. This course walks you through avoiding each, with templates you can use straight away."),
        ],
        outcomes: "Test demand before you build\nSet prices with confidence\nLaunch to your first ten customers\nTrack cash, costs and profit simply",
        requirements: "An idea, or a problem you care about",
        audience: "First-time founders\nFreelancers turning a skill into a business",
        sections: &[
            ("Validate", &[
                Item::Lesson { title: "Talk to ten people", minutes: 12, preview: true, video: false, body: &[
                    P("Before you build anything, find ten people with the problem and ask how they handle it today. Listen for money already being spent — that is your best signal."),
                ] },
                Item::Lesson { title: "Pricing from value", minutes: 16, preview: false, video: false, body: &[
                    P("Price from the value you create, not from your costs. If you save a business ten hours a month, charging for two of them is a bargain."),
                ] },
            ]),
            ("Launch and run", &[
                Item::Lesson { title: "Your first ten customers", minutes: 14, preview: false, video: false, body: &[
                    P("Your first customers come from people who already know you. Do things that do not scale: personal messages, hand-holding, quick fixes."),
                ] },
                Item::Lesson { title: "Simple bookkeeping", minutes: 12, preview: false, video: false, body: &[
                    P("Open a separate bank account, keep every receipt, and look at three numbers each month: revenue, costs and cash in the bank."),
                ] },
                Item::Task { title: "Assignment: your one-page plan", minutes: 60, body: &[
                    P("Fill in the one-page plan: customer, problem, offer, price, first ten customers, and the number that tells you it is working."),
                ] },
            ]),
        ],
    },
    Course {
        title: "Mindful Mornings",
        subtitle: "A seven-day yoga and meditation practice that fits before breakfast. One new session unlocks each day.",
        cat: "wellbeing", image: "1506126613408-eca07ce68773", level: "all", access: "free", price: 0, compare: 0, featured: false, drip: true,
        about: &[P("Seven short sessions, one a day, each building on the last. Ten minutes of movement and five of stillness — enough to change how the rest of the day feels.")],
        outcomes: "Build a daily ten-minute routine\nBreathe to calm your nervous system\nMove with more ease and less stiffness",
        requirements: "A mat or a soft floor\nComfortable clothes",
        audience: "Complete beginners\nBusy people who want a gentle routine",
        sections: &[
            ("The week", &[
                Item::Lesson { title: "Day 1 — Arriving", minutes: 12, preview: true, video: true, body: &[P("Sit comfortably, lengthen the spine and take ten slow breaths, counting each exhale. Then roll the shoulders and neck gently. That is the whole practice today.")] },
                Item::Lesson { title: "Day 2 — Breath", minutes: 12, preview: false, video: false, body: &[P("Breathe in for four, out for six. A longer exhale tells your body it is safe. Repeat for five minutes.")] },
                Item::Lesson { title: "Day 3 — Spine", minutes: 14, preview: false, video: false, body: &[P("Cat and cow, gentle twists and a forward fold wake up the spine after sleep.")] },
                Item::Lesson { title: "Day 4 — Balance", minutes: 14, preview: false, video: false, body: &[P("Tree pose and a slow warrior sequence build steadiness — in the body and the mind.")] },
                Item::Lesson { title: "Day 5 — Strength", minutes: 15, preview: false, video: false, body: &[P("A short plank, bridge and chair sequence. Move slowly; strength grows in the pauses.")] },
                Item::Lesson { title: "Day 6 — Stillness", minutes: 12, preview: false, video: false, body: &[P("Ten minutes of sitting meditation. When the mind wanders, notice it kindly and return to the breath.")] },
                Item::Lesson { title: "Day 7 — Your own practice", minutes: 15, preview: false, video: false, body: &[P("Put together your favourite parts of the week into a routine you can keep. Write it down and keep it by your mat.")] },
            ]),
        ],
    },
];

const STUDENTS: &[(&str, &str)] = &[
    ("Amara Okafor", "amara.okafor@example.com"),
    ("Lucas Bernard", "lucas.bernard@example.com"),
    ("Priya Raman", "priya.raman@example.com"),
    ("Tomás Ortega", "tomas.ortega@example.com"),
    ("Hannah Weiss", "hannah.weiss@example.com"),
    ("Kenji Watanabe", "kenji.watanabe@example.com"),
    ("Sofia Lindqvist", "sofia.lindqvist@example.com"),
    ("Daniel Mensah", "daniel.mensah@example.com"),
    ("Leila Haddad", "leila.haddad@example.com"),
    ("Ethan Clarke", "ethan.clarke@example.com"),
];

const REVIEWS: &[(i64, &str, &str)] = &[
    (5, "Exactly what I needed", "Clear, practical and never boring. I finally understand how the pieces fit together."),
    (5, "Brilliant teaching", "Short lessons, real exercises, and the feedback on my project was genuinely useful."),
    (4, "Great value", "Covers the essentials really well. I would have liked one more practice exercise in the middle section."),
    (5, "Confidence boost", "I went from copying snippets to understanding them. The quizzes are a nice touch."),
    (5, "Calm and clear", "The pace is perfect and the explanations are kind. Recommended to everyone in my team."),
    (4, "Very good", "Well structured and to the point. A few more examples would make it perfect."),
];

pub(super) fn html_of(blocks: &[Block]) -> (String, String) {
    let lexical = document(blocks.iter().map(|b| b.node()).collect());
    let html = vuuut_render::lexical::to_html(&lexical);
    (lexical, html)
}

pub(crate) async fn sample_load(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match seed_sample_academy(&state.db, &state.settings, Some(&actor.user.id)).await {
        Ok(n) => crate::tools::tool_saved(&state, BASE, &format!("Sample academy loaded: {n} courses, ready to edit. The catalogue is switched on.")),
        Err(e) => {
            tracing::error!("lms: loading the sample academy failed: {}", e.message());
            flash_to(BASE, "error", "The sample academy could not be loaded.")
        }
    }
}

/// Load the sample academy and switch the catalogue on. `instructor` is the
/// staff user shown on the courses; none picks the site's owner. Courses
/// whose address is already taken are skipped, so running it twice adds
/// nothing. Also the `vuuut seed-lms` command.
pub async fn seed_sample_academy(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache, instructor: Option<&str>) -> vuuut_core::Result<usize> {
    let owner: Option<String> = match instructor {
        Some(i) => Some(i.to_string()),
        None => sqlx::query_scalar("SELECT u.id FROM users u JOIN roles_users ru ON ru.user_id = u.id JOIN roles r ON r.id = ru.role_id WHERE r.name = 'Owner' LIMIT 1")
            .fetch_optional(db).await.ok().flatten(),
    };
    let currency = vuuut_core::ecom::Settings::read(|key| settings.get_str(key)).currency;
    let loaded = load(db, owner.as_deref().unwrap_or(""), &currency, usize::MAX).await?;
    settings.set_many(db, &[(rules::SETTING_ENABLED, serde_json::json!(true))]).await?;
    Ok(loaded.courses.len())
}

/// What one load created — only rows that did not exist before — so Tools →
/// Dummy content can record them for removal.
#[derive(Default)]
pub struct Loaded {
    pub categories: Vec<String>,
    pub members: Vec<String>,
    pub courses: Vec<String>,
    pub products: Vec<String>,
}

/// Up to `limit` sample courses, for Tools → Dummy content. Switches the
/// catalogue on when it made any.
pub async fn load_courses(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache, instructor: &str, limit: usize) -> vuuut_core::Result<Loaded> {
    let currency = vuuut_core::ecom::Settings::read(|key| settings.get_str(key)).currency;
    let loaded = load(db, instructor, &currency, limit).await?;
    if !loaded.courses.is_empty() {
        settings.set_many(db, &[(rules::SETTING_ENABLED, serde_json::json!(true))]).await?;
    }
    Ok(loaded)
}

async fn load(db: &vuuut_db::Db, instructor: &str, currency: &str, limit: usize) -> vuuut_core::Result<Loaded> {
    let mut out = Loaded::default();
    let mut cat_ids: HashMap<&str, String> = HashMap::new();
    for (i, c) in CATS.iter().enumerate() {
        let id = match repo::category_by_slug(db, c.slug).await {
            Some(existing) => existing.id,
            None => {
                let id = repo::save_category(db, None, c.name, c.slug, c.description, c.tint, &format!("{IMG}{}?w=1200&q=80", c.image), i as i64).await?;
                out.categories.push(id.clone());
                id
            }
        };
        cat_ids.insert(c.slug, id);
    }
    let now = chrono::Utc::now();
    let mut members: Vec<String> = Vec::new();
    for (i, (name, email)) in STUDENTS.iter().enumerate() {
        let id = match repo::member_by_email(db, email).await {
            Some(m) => Some(m.id),
            None => {
                let joined = vuuut_core::dates::to_db_string(&(now - chrono::Duration::days(40 - i as i64 * 3)));
                let id = vuuut_db::repos::dummies::insert_member(db, name, email, &joined).await?;
                out.members.extend(id.clone());
                id
            }
        };
        if let Some(id) = id {
            members.push(id);
        }
    }
    for (ci, c) in COURSES.iter().enumerate().take(limit) {
        let slug = vuuut_core::directory::slugify(c.title);
        if repo::course_by_slug(db, &slug).await.is_some() {
            continue;
        }
        let (lexical, html) = html_of(c.about);
        let input = rules::CourseInput {
            title: c.title.into(),
            slug: slug.clone(),
            subtitle: c.subtitle.into(),
            description: vuuut_render::markdown::html_to_plaintext(&html),
            description_lexical: lexical,
            description_html: html,
            image: format!("{IMG}{}?w=1600&q=80", c.image),
            category_id: cat_ids.get(c.cat).cloned(),
            instructor_id: Some(instructor.to_string()).filter(|i| !i.is_empty()),
            level: c.level.into(),
            language: "English".into(),
            status: "published".into(),
            access: c.access.into(),
            price_cents: c.price,
            compare_cents: c.compare,
            outcomes: rules::lines(c.outcomes),
            requirements: rules::lines(c.requirements),
            audience: rules::lines(c.audience),
            featured: c.featured,
            sequential: c.drip,
            certificate: true,
            seo_description: c.subtitle.into(),
            ..Default::default()
        };
        let mut input = input;
        // A paid course is sold through the store: make its product and link it.
        if c.access == "paid" {
            let (lexical, html) = html_of(c.about);
            let product = vuuut_db::repos::ecom::ProductInput {
                handle: vuuut_core::directory::slugify(&format!("{} course", c.title)),
                title: c.title.into(),
                status: "active".into(),
                description_lexical: lexical,
                description_html: html,
                excerpt: c.subtitle.into(),
                product_type: "Online course".into(),
                tags: vec!["course".into()],
                images: vec![format!("{IMG}{}?w=1600&q=80", c.image)],
                taxable: true,
                ..Default::default()
            };
            let minor = |price| vuuut_core::currency::fixed_price_to_minor(price, currency).expect("whole sample price");
            let variant = vuuut_db::repos::ecom::VariantInput { price_cents: minor(c.price), compare_at_cents: minor(c.compare), ..Default::default() };
            if let Ok(pid) = vuuut_db::repos::ecom::save_product(db, None, &product, &[variant], &[]).await {
                out.products.push(pid.clone());
                input.product_id = Some(pid);
            }
        }
        let course_id = repo::insert_course(db, &input).await?;
        let mut day = 0;
        for (title, items) in c.sections {
            let sid = repo::add_section(db, &course_id, title).await?;
            for item in *items {
                let (title, kind, minutes, preview, video, blocks, questions): (&str, &str, i64, bool, bool, &[Block], &[Q]) = match item {
                    Item::Lesson { title, minutes, preview, video, body } => (title, "lesson", *minutes, *preview, *video, body, &[]),
                    Item::Quiz { title, minutes, intro, questions } => (title, "quiz", *minutes, false, false, intro, questions),
                    Item::Task { title, minutes, body } => (title, "assignment", *minutes, false, false, body, &[]),
                };
                let (lexical, html) = html_of(blocks);
                let li = rules::LessonInput {
                    title: title.into(),
                    slug: vuuut_core::directory::slugify(title),
                    kind: kind.into(),
                    status: "published".into(),
                    section_id: Some(sid.clone()),
                    summary: vuuut_render::markdown::html_to_plaintext(&html).chars().take(140).collect::<String>().trim().to_string(),
                    content_lexical: lexical,
                    content_html: html,
                    video_url: if video { VIDEO.into() } else { String::new() },
                    duration_minutes: minutes,
                    preview,
                    drip_days: if c.drip { day } else { 0 },
                    pass_mark: if kind == "quiz" { 60 } else { 70 },
                    reveal: true,
                    points: 100,
                    ..Default::default()
                };
                day += 1;
                let lid = repo::insert_lesson(db, &course_id, &li).await?;
                if !questions.is_empty() {
                    let qs: Vec<rules::Question> = questions.iter().map(|(prompt, k, opts, answer, explanation)| rules::Question {
                        id: String::new(),
                        kind: (*k).into(),
                        prompt: (*prompt).into(),
                        options: if *k == "truefalse" {
                            let truth = opts.first().is_some_and(|o| *o == "True");
                            vec![rules::Choice { t: "True".into(), c: truth }, rules::Choice { t: "False".into(), c: !truth }]
                        } else {
                            opts.iter().map(|o| rules::Choice { t: o.trim_start_matches('*').into(), c: o.starts_with('*') }).collect()
                        },
                        answers: answer.split('|').filter(|a| !a.is_empty()).map(str::to_string).collect(),
                        explanation: (*explanation).into(),
                        points: 1,
                    }).collect();
                    repo::save_questions(db, &lid, &qs).await?;
                }
            }
        }
        // Students, progress and reviews, varied per course.
        let course = repo::find_course(db, &course_id).await.unwrap_or_default();
        let lesson_ids: Vec<String> = repo::lessons(db, &course_id).await.into_iter().map(|l| l.id).collect();
        let take = [9, 7, 6, 5, 8, 4, 6][ci % 7].min(members.len());
        for (k, m) in members.iter().enumerate().skip(ci % 3).take(take) {
            let mut e = repo::enroll(db, &course, m, if c.access == "paid" { "purchase" } else { "free" }, None).await?;
            // Spread the demonstration enrolments over the last four weeks.
            let when = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::hours(((k * 7 + ci * 11) % 28) as i64 * 24 + 3)));
            let _ = sqlx::query("UPDATE lms_enrollments SET enrolled_at = ?, created_at = ? WHERE id = ?").bind(&when).bind(&when).bind(&e.id).execute(db).await;
            e.enrolled_at = when;
            let done = match k % 4 { 0 => lesson_ids.len(), 1 => lesson_ids.len() / 2, 2 => 2.min(lesson_ids.len()), _ => 0 };
            for lid in lesson_ids.iter().take(done) {
                e = repo::complete_lesson(db, &e, lid, &lesson_ids, true).await?;
            }
            if k % 2 == 0 {
                let (rating, title, body) = REVIEWS[(k + ci) % REVIEWS.len()];
                repo::save_review(db, &course_id, m, rating, title, body, "published").await?;
            }
        }
        out.courses.push(course_id);
    }
    Ok(out)
}
