//! Original sample courses for the Lingua Atlas theme.
use super::{repo, rules, sample::html_of};
use vuuut_core::directory::slugify;
use vuuut_core::dummies::Block::{H, P};
use vuuut_db::repos::ecom;

struct Language {
    name: &'static str,
    slug: &'static str,
    tint: &'static str,
    description: &'static str,
    courses: [DemoCourse; 3],
}

struct DemoCourse {
    title: &'static str,
    subtitle: &'static str,
    outcome: &'static str,
    price_cents: i64,
    lessons: &'static [(&'static str, &'static str, &'static str)],
}

const LANGUAGES: &[Language] = &[
    Language {
        name: "Arabic",
        slug: "arabic",
        tint: "teal",
        description: "Read Arabic script and practice everyday Modern Standard Arabic.",
        courses: [
            DemoCourse {
                title: "Arabic Script from Scratch",
                subtitle: "Start reading from right to left and get to know your first Arabic letters.",
                outcome: "Recognize ب, ت, and ث and find the beginning of an Arabic word.",
                price_cents: 0,
                lessons: &[
                    ("Start at the right", "Arabic script is read from right to left. Many letters change shape as they connect to their neighbors.", "Look at باب (bāb, door). Start with the letter on the right and trace the word toward the left."),
                    ("Three letters, one shape", "ب (bāʾ), ت (tāʾ), and ث (thāʾ) share a basic shape. The dots tell you which letter you are reading.", "Draw ب with one dot below, ت with two above, and ث with three above. Cover the names and identify each letter."),
                ],
            },
            DemoCourse {
                title: "Your First Arabic Conversation",
                subtitle: "Say hello, introduce yourself and thank someone in Modern Standard Arabic.",
                outcome: "Use a greeting, a name introduction, and a polite thank you.",
                price_cents: 0,
                lessons: &[
                    ("Say hello and thank you", "مَرْحَبًا (marḥaban) means hello. شُكْرًا (shukran) means thank you.", "Read each expression aloud. Imagine greeting someone and then thanking them for their help."),
                    ("Introduce yourself", "اسْمِي (ismī) means my name is. Put your name after it, as in اسْمِي لَيْلَى (ismī Laylā).", "Start with مَرْحَبًا and then say اسْمِي with your own name."),
                ],
            },
            DemoCourse {
                title: "Read Arabic in Context",
                subtitle: "Move from single words to short Modern Standard Arabic sentences with guided reading practice.",
                outcome: "Identify a sentence topic, use a verb pattern, and work out the main idea of a short paragraph.",
                price_cents: 3900,
                lessons: &[
                    ("Find the topic", "In البَيْتُ كَبِيرٌ (the house is big), البَيْتُ names the topic and كَبِيرٌ describes it.", "Circle the topic, then make another sentence with البَيْتُ and صَغِيرٌ (small)."),
                    ("Notice the verb", "In يَكْتُبُ عَلِيٌّ رِسَالَةً (Ali writes a letter), يَكْتُبُ tells you the action, عَلِيٌّ names the writer, and رِسَالَةً tells you what he writes.", "Underline the action and say the sentence in English. Then try changing the writer’s name."),
                    ("Read for the main idea", "عِنْدِي كِتَابٌ. أَقْرَأُ الكِتَابَ فِي البَيْتِ. means I have a book. I read the book at home.", "Read the two sentences once without the translation. Find the repeated word and explain what connects the sentences."),
                ],
            },
        ],
    },
    Language {
        name: "English",
        slug: "english",
        tint: "blue",
        description: "Build confidence with everyday conversation and practical vocabulary.",
        courses: [
            DemoCourse {
                title: "Everyday English Conversations",
                subtitle: "Introduce yourself, ask simple questions and keep a conversation going.",
                outcome: "Start a conversation and ask an open question.",
                price_cents: 0,
                lessons: &[
                    ("Make an introduction", "Try: Hi, I’m Sam. Nice to meet you. You can name the place you are from with I’m from …", "Write a two-sentence introduction, then say it aloud without reading it."),
                    ("Keep the conversation going", "A question beginning with what or where invites a longer answer. Try asking: What do you like to do in your free time?", "Answer the question in one sentence. Follow up by asking the same question back."),
                ],
            },
            DemoCourse {
                title: "English for Getting Around",
                subtitle: "Ask for directions and use the words you’ll see at a bus or train station.",
                outcome: "Ask where something is and follow basic directions.",
                price_cents: 0,
                lessons: &[
                    ("Ask for directions", "Use Excuse me, where is the station? as a polite way to start. Replace the station with the place you need.", "Practice asking for a library, a bus stop, and a museum."),
                    ("Follow a simple route", "Go straight means keep going in the same direction. Turn left and turn right tell you where to change direction.", "Draw a simple map and write three instructions that take someone from a bus stop to a café."),
                ],
            },
            DemoCourse {
                title: "Speak English with Confidence",
                subtitle: "Practice telling a short story, asking for clarification, and responding naturally.",
                outcome: "Share a two-minute story and clarify a point with a follow-up question.",
                price_cents: 4900,
                lessons: &[
                    ("Put a story in order", "Use first, then, and finally to make the order of events clear. Try telling a short story about getting somewhere late.", "Write three events and connect them using the three signposts. Tell the story again without looking at your notes."),
                    ("Ask for clarification", "Could you say that again? asks for repetition. Do you mean …? checks your understanding of a specific point.", "Imagine a friend gave you confusing directions. Write one question using each pattern."),
                    ("Record a two-minute story", "Introduce where you were, explain what happened, and end with what you learned or noticed.", "Record yourself telling the story. Listen once and choose one sentence to make shorter or clearer."),
                ],
            },
        ],
    },
    Language {
        name: "French",
        slug: "french",
        tint: "pink",
        description: "Learn useful French phrases for introductions, travel, and everyday life.",
        courses: [
            DemoCourse {
                title: "French for Your First Conversation",
                subtitle: "Greet someone, share your name and ask how they are.",
                outcome: "Greet someone politely and share a short introduction.",
                price_cents: 0,
                lessons: &[
                    ("Bonjour or salut?", "Bonjour is a polite daytime greeting. Salut is informal and fits a conversation with a friend.", "Choose a greeting for a shopkeeper and another for a friend. Say both aloud."),
                    ("Introduce yourself in French", "Je m’appelle … means my name is … . Use Comment vous appelez-vous ? to ask someone’s name politely.", "Say Bonjour, je m’appelle … with your own name, then practice the question and answer together."),
                ],
            },
            DemoCourse {
                title: "French at the Café",
                subtitle: "Read a simple menu, order something and ask for the bill.",
                outcome: "Order a drink politely and ask for the bill.",
                price_cents: 0,
                lessons: &[
                    ("Order a drink", "Je voudrais un café, s’il vous plaît means I would like a coffee, please. You can also order un thé (a tea).", "Say your order twice. Ask for a coffee first and a tea the second time."),
                    ("Ask for the bill", "L’addition, s’il vous plaît means the bill, please. Merci means thank you.", "Practice a short café dialogue from Bonjour to Merci, including your order and a request for the bill."),
                ],
            },
            DemoCourse {
                title: "French for Real Conversations",
                subtitle: "Make plans, offer alternatives, and respond politely with guided French dialogues.",
                outcome: "Invite someone out, suggest a different time, and confirm the plan in French.",
                price_cents: 3900,
                lessons: &[
                    ("Make a plan", "On se retrouve à quelle heure ? means What time shall we meet? Try answering À trois heures (at three).", "Write an invitation to meet at a café. Ask what time works, then answer with a time."),
                    ("Offer an alternative", "Et si on se retrouvait demain ? means How about meeting tomorrow? Use this pattern to suggest a different plan.", "Suggest a different place or day. Read the dialogue aloud, switching roles after each line."),
                    ("Confirm the details", "D’accord, à demain ! means All right, see you tomorrow! Add the time or place if the plan needs clarification.", "Write a six-line dialogue from invitation to confirmation. Include a time, a place, and a polite alternative."),
                ],
            },
        ],
    },
];

/// Add editable demo content, skipping any course with an existing slug.
pub async fn seed_language_academy(
    db: &vuuut_db::Db,
    settings: &vuuut_db::SettingsCache,
) -> vuuut_core::Result<usize> {
    let store_currency = vuuut_core::ecom::Settings::read(|key| settings.get_str(key)).currency;
    let mut made = 0;
    for (position, language) in LANGUAGES.iter().enumerate() {
        let category_id = match repo::category_by_slug(db, language.slug).await {
            Some(category) => category.id,
            None => {
                repo::save_category(
                    db,
                    None,
                    language.name,
                    language.slug,
                    language.description,
                    language.tint,
                    "",
                    position as i64,
                )
                .await?
            }
        };

        for (index, course) in language.courses.iter().enumerate() {
            let slug = slugify(course.title);
            if repo::course_by_slug(db, &slug).await.is_some() {
                continue;
            }
            seed_course(
                db,
                &category_id,
                language.name,
                course,
                slug,
                index == 0,
                &store_currency,
            )
            .await?;
            made += 1;
        }
    }
    settings
        .set_many(db, &[(rules::SETTING_ENABLED, serde_json::json!(true))])
        .await?;
    Ok(made)
}

async fn seed_course(
    db: &vuuut_db::Db,
    category_id: &str,
    language: &str,
    course: &DemoCourse,
    slug: String,
    featured: bool,
    store_currency: &str,
) -> vuuut_core::Result<()> {
    let (lexical, html) = html_of(&[P(course.subtitle)]);
    let paid = course.price_cents > 0;
    let mut input = rules::CourseInput {
        title: course.title.into(),
        slug,
        subtitle: course.subtitle.into(),
        description: course.subtitle.into(),
        description_lexical: lexical,
        description_html: html,
        category_id: Some(category_id.into()),
        language: "English".into(),
        keywords: language.into(),
        level: if paid { "intermediate" } else { "beginner" }.into(),
        status: "draft".into(),
        access: if paid { "paid" } else { "free" }.into(),
        price_cents: course.price_cents,
        featured,
        outcomes: vec![course.outcome.into()],
        seo_description: course.subtitle.into(),
        ..Default::default()
    };
    let id = repo::insert_course(db, &input).await?;
    let result = async {
        populate_course(db, &id, course).await?;
        if paid {
            let product = ecom::ProductInput {
                handle: format!("{}-course", input.slug),
                title: input.title.clone(),
                status: "active".into(),
                description_lexical: input.description_lexical.clone(),
                description_html: input.description_html.clone(),
                excerpt: input.subtitle.clone(),
                product_type: "Online course".into(),
                tags: vec!["course".into(), language.into()],
                ..Default::default()
            };
            let variant = ecom::VariantInput {
                price_cents: vuuut_core::currency::fixed_price_to_minor(
                    course.price_cents,
                    store_currency,
                )
                .expect("whole sample price"),
                ..Default::default()
            };
            input.product_id = Some(ecom::save_product(db, None, &product, &[variant], &[]).await?);
        }
        input.status = "published".into();
        repo::update_course(db, &id, &input).await
    }
    .await;

    if let Err(error) = result {
        if let Some(product_id) = input.product_id {
            ecom::delete_product(db, &product_id).await?;
        }
        repo::delete_course(db, &id).await?;
        return Err(error);
    }
    Ok(())
}

async fn populate_course(
    db: &vuuut_db::Db,
    id: &str,
    course: &DemoCourse,
) -> vuuut_core::Result<()> {
    let durations = if course.price_cents > 0 {
        &[12, 25, 28][..]
    } else {
        &[8, 12][..]
    };
    let section = repo::add_section(db, id, "Learn and practice").await?;
    for (index, (&(title, example, exercise), &minutes)) in
        course.lessons.iter().zip(durations).enumerate()
    {
        let (lexical, html) = html_of(&[P(example), H("Try it"), P(exercise)]);
        repo::insert_lesson(
            db,
            id,
            &rules::LessonInput {
                title: title.into(),
                slug: slugify(title),
                kind: "lesson".into(),
                status: "published".into(),
                section_id: Some(section.clone()),
                summary: if index == 0 {
                    "Read the examples and try a short exercise."
                } else {
                    "Put the new phrases and patterns into practice."
                }
                .into(),
                content_lexical: lexical,
                content_html: html,
                duration_minutes: minutes,
                preview: index == 0,
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}
