//! Load the same guides bundled with Handbook's manual import, with dummy batch tracking.

use std::collections::HashMap;

use super::{content, fill_theme_settings, repo, repos, Demo, Kind, Menu};
use serde::Deserialize;
use vuuut_core::{GhostError, Result};

const EXPORT: &str = include_str!("../../../../../content/themes/handbook/demo/handbook-demo.json");

#[derive(Deserialize)]
struct Export {
    db: [Database; 1],
}

#[derive(Deserialize)]
struct Database {
    data: Documentation,
}

#[derive(Deserialize)]
struct Documentation {
    tags: Vec<Section>,
    posts: Vec<Article>,
    posts_tags: Vec<ArticleSection>,
}

#[derive(Deserialize)]
struct Section {
    id: String,
    slug: String,
    name: String,
    description: String,
}

#[derive(Deserialize)]
struct Article {
    id: String,
    slug: String,
    title: String,
    #[serde(rename = "type")]
    kind: String,
    custom_excerpt: String,
    lexical: String,
    published_at: chrono::DateTime<chrono::Utc>,
    featured: u8,
}

#[derive(Deserialize)]
struct ArticleSection {
    post_id: String,
    tag_id: String,
    sort_order: i64,
}

pub(super) async fn generate(
    db: &vuuut_db::Db,
    settings: &vuuut_db::SettingsCache,
    batch: &str,
    user_id: &str,
    menu: &mut Vec<(Menu, String, String)>,
) -> Result<Vec<(Kind, i64)>> {
    let Export {
        db: [Database { data }],
    } = serde_json::from_str(EXPORT)
        .map_err(|err| GhostError::Internal(format!("Invalid Handbook sample: {err}")))?;
    let mut tag_ids = HashMap::new();
    let mut sections = Vec::new();
    let mut tags = 0;
    for section in &data.tags {
        let tag = match repos::tags::find_by_slug(db, &section.slug).await? {
            Some(tag) if tag.visibility == "public" => tag,
            Some(_) => continue,
            None => {
                let tag = repos::tags::insert(
                    db,
                    &repos::tags::NewTag {
                        slug: Some(section.slug.clone()),
                        name: section.name.clone(),
                        description: Some(section.description.clone()),
                        visibility: "public".into(),
                        ..Default::default()
                    },
                )
                .await?;
                repo::record(db, batch, Kind::Tags, &tag.id).await?;
                tags += 1;
                tag
            }
        };
        sections.push(section.slug.as_str());
        tag_ids.insert(section.id.as_str(), tag.id);
    }

    menu.push((Menu::Primary, "Documentation".into(), "/".into()));
    let mut articles = 0;
    let mut pages = 0;
    let mut links = data.posts_tags;
    links.sort_by_key(|link| link.sort_order);
    for article in &data.posts {
        let is_page = article.kind == "page";
        let kind = if is_page { Kind::Pages } else { Kind::Posts };
        let saved =
            match repos::posts::find_by_slug(db, &article.slug, &article.kind, false).await? {
                Some(post) => post,
                None => {
                    let tag_ids: Vec<String> = links
                        .iter()
                        .filter(|link| link.post_id == article.id)
                        .filter_map(|link| tag_ids.get(link.tag_id.as_str()).cloned())
                        .collect();
                    if !is_page && tag_ids.is_empty() {
                        continue;
                    }
                    let post = content(
                        &article.title,
                        Some(&article.slug),
                        &article.custom_excerpt,
                        &article.lexical,
                        &article.kind,
                        true,
                        Some(article.published_at),
                        article.featured != 0,
                        None,
                        user_id,
                        tag_ids,
                    );
                    let saved = repos::posts::insert(db, &post, user_id).await?;
                    repo::record(db, batch, kind, &saved.id).await?;
                    if is_page {
                        pages += 1;
                    } else {
                        articles += 1;
                    }
                    saved
                }
            };
        if is_page && saved.status == "published" && saved.visibility == "public" {
            menu.push((Menu::Primary, "About".into(), format!("/{}/", article.slug)));
        }
    }
    if sections.contains(&"support") {
        menu.push((
            Menu::Secondary,
            "Support guides".into(),
            "/tag/support/".into(),
        ));
    }
    fill_theme_settings(
        db,
        settings,
        Demo::Documentation.theme(),
        &[("sections", &sections.join(","))],
    )
    .await?;
    Ok(vec![
        (Kind::Tags, tags),
        (Kind::Posts, articles),
        (Kind::Pages, pages),
    ])
}
