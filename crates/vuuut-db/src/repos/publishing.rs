//! Copies and restorable snapshots keep content, metadata and assignments.
use super::{posts, dberr};
use vuuut_core::{models::Post, GhostError, Result};
use serde::{Serialize,Deserialize};
use crate::Db;

#[derive(Serialize,Deserialize)]
pub struct Snapshot {
    pub post: Post,
    pub author_ids: Vec<String>,
    pub tag_ids: Vec<String>,
}
impl Snapshot {
    pub fn input(&self) -> posts::NewPost {
        let p=&self.post; let m=p.meta.clone().unwrap_or_default();
        posts::NewPost {
            title:p.title.clone().unwrap_or_default(),slug:p.slug.clone(),lexical:p.lexical.clone(),markdown:p.markdown.clone(),
            html:p.html.clone(),plaintext:p.plaintext.clone(),custom_excerpt:p.custom_excerpt.clone(),feature_image:p.feature_image.clone(),
            featured:p.featured,post_type:p.post_type.clone(),status:p.status.clone(),visibility:p.visibility.clone(),published_at:p.published_at.clone(),
            codeinjection_head:p.codeinjection_head.clone(),codeinjection_foot:p.codeinjection_foot.clone(),custom_template:p.custom_template.clone(),
            canonical_url:p.canonical_url.clone(),show_title_and_feature_image:Some(p.show_title_and_feature_image),
            meta_title:m.meta_title,meta_description:m.meta_description,og_title:m.og_title,og_description:m.og_description,og_image:m.og_image,
            twitter_title:m.twitter_title,twitter_description:m.twitter_description,twitter_image:m.twitter_image,email_subject:m.email_subject,
            feature_image_alt:m.feature_image_alt,feature_image_caption:m.feature_image_caption,
            author_ids:self.author_ids.clone(),tag_ids:self.tag_ids.clone(),tier_ids:p.tier_ids.clone(),reading_time:p.reading_time,
        }
    }
}
/// One entry in a post's history, as the list shows it.
///
/// The snapshot itself is deliberately *not* here. A snapshot is the whole
/// post — body, meta, tags, authors — and a post with eighty revisions would
/// mean eighty of them loaded to draw a list of dates. The body is read
/// separately, for the one revision being looked at, by
/// [`revision_snapshot`].
#[derive(Debug, Clone)]
pub struct Revision {
    pub id: String,
    pub created_at: String,
    /// The title the post had *before* this edit — the snapshot is of the
    /// state being replaced, which is what makes it restorable.
    pub title: Option<String>,
    /// Who made the edit, or `None` for the scheduler and for revisions
    /// written before the author was recorded.
    pub author_name: Option<String>,
    pub feature_image: Option<String>,
}

/// A post's revisions, newest first.
///
/// `created_at_ts` (milliseconds) is the real order; the id only breaks a
/// same-millisecond tie, and does so stably on both engines. It stands in for
/// SQLite's `rowid`, which MySQL does not have and this table's random-text
/// primary key cannot substitute for.
///
/// Rows with no snapshot are skipped: they predate the column
/// (`0003_publishing.sql` added it) and cannot be restored, so listing them
/// would offer a button that could only fail.
pub async fn revisions(pool:&Db,post_id:&str,limit:i64)->Result<Vec<Revision>> {
    let rows=sqlx::query_as::<_,(String,String,Option<String>,Option<String>,crate::types::Text)>(
        "SELECT r.id, r.created_at, u.name, r.feature_image, r.snapshot \
         FROM post_revisions r LEFT JOIN users u ON u.id = r.author_id \
         WHERE r.post_id = ? AND r.snapshot IS NOT NULL \
         ORDER BY r.created_at_ts DESC, r.id DESC LIMIT ?")
        .bind(post_id).bind(limit.clamp(1,200)).fetch_all(pool).await.map_err(dberr)?;
    Ok(rows.into_iter().map(|(id,created_at,author_name,feature_image,snapshot)|{
        // Only the title is read out of the snapshot here, so a malformed one
        // costs a title rather than the whole list.
        let title=serde_json::from_str::<serde_json::Value>(&snapshot.0).ok()
            .and_then(|v|v["post"]["title"].as_str().map(str::to_string));
        Revision{id,created_at,title,author_name,feature_image}
    }).collect())
}

/// How many restorable revisions a post has, for the editor's panel.
pub async fn revision_count(pool:&Db,post_id:&str)->Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM post_revisions WHERE post_id = ? AND snapshot IS NOT NULL")
        .bind(post_id).fetch_one(pool).await.map_err(dberr)
}

/// The full snapshot behind one revision, for previewing it before restoring.
///
/// Scoped by `post_id` as well as by revision id: the revision id arrives from
/// a URL, and a revision of someone else's post must not be readable by
/// guessing one.
pub async fn revision_snapshot(pool:&Db,post_id:&str,revision_id:&str)->Result<Option<Snapshot>> {
    let json:Option<crate::types::Text>=sqlx::query_scalar(
        "SELECT snapshot FROM post_revisions WHERE id = ? AND post_id = ? AND snapshot IS NOT NULL")
        .bind(revision_id).bind(post_id).fetch_optional(pool).await.map_err(dberr)?;
    Ok(json.and_then(|json|serde_json::from_str(&json.0).ok()))
}

pub async fn snapshot(pool:&Db,post:Post)->Result<Snapshot> {
    let authors=sqlx::query_scalar("SELECT author_id FROM posts_authors WHERE post_id=? ORDER BY sort_order").bind(&post.id).fetch_all(pool).await.map_err(dberr)?;
    let tags=sqlx::query_scalar("SELECT tag_id FROM posts_tags WHERE post_id=? ORDER BY sort_order").bind(&post.id).fetch_all(pool).await.map_err(dberr)?;
    Ok(Snapshot{post,author_ids:authors,tag_ids:tags})
}
pub async fn duplicate(pool:&Db,id:&str,author_id:&str)->Result<Post> {
    let post=posts::find_by_id(pool,id).await?.ok_or(GhostError::NotFoundError{message:"Post not found".into(),context:None})?;
    let mut input=snapshot(pool,post).await?.input();
    input.title=format!("{} (Copy)",input.title);input.slug=None;input.status="draft".into();input.published_at=None;
    posts::insert(pool,&input,author_id).await
}
pub async fn restore(pool:&Db,post_id:&str,revision_id:&str)->Result<Post> {
    crate::content::touch();
    let json:Option<crate::types::Text>=sqlx::query_scalar("SELECT snapshot FROM post_revisions WHERE id=? AND post_id=? AND snapshot IS NOT NULL")
        .bind(revision_id).bind(post_id).fetch_optional(pool).await.map_err(dberr)?;
    let Some(json)=json else{return Err(GhostError::NotFoundError{message:"Revision not found".into(),context:None})};
    let saved:Snapshot=serde_json::from_str(&json).map_err(|e|GhostError::Internal(e.to_string()))?;
    let n=saved.input();
    // Restoring changes content, never publication state or URL.
    posts::update(pool,post_id,&posts::PostChanges {
        title:Some(n.title),lexical:n.lexical,markdown:n.markdown,html:n.html,plaintext:n.plaintext,custom_excerpt:n.custom_excerpt,
        feature_image:n.feature_image,featured:Some(n.featured),visibility:Some(n.visibility),
        codeinjection_head:n.codeinjection_head,codeinjection_foot:n.codeinjection_foot,custom_template:n.custom_template,canonical_url:n.canonical_url,
        show_title_and_feature_image:n.show_title_and_feature_image,meta_title:n.meta_title,meta_description:n.meta_description,
        og_title:n.og_title,og_description:n.og_description,og_image:n.og_image,twitter_title:n.twitter_title,twitter_description:n.twitter_description,
        twitter_image:n.twitter_image,email_subject:n.email_subject,feature_image_alt:n.feature_image_alt,feature_image_caption:n.feature_image_caption,
        reading_time:Some(n.reading_time),tag_ids:Some(n.tag_ids),author_ids:Some(n.author_ids),tier_ids:Some(n.tier_ids),
        // A restore is somebody asking for it, never an autosave. Without
        // this the version being replaced would be folded into the last one
        // whenever the two were less than ten minutes apart, and the restore
        // would not be undoable — which is exactly what the screen promises.
        explicit_save:true,
        clearing:vec!["lexical","markdown","html","plaintext","custom_excerpt","feature_image","codeinjection_head","codeinjection_foot","custom_template","canonical_url","meta_title","meta_description","og_title","og_description","og_image","twitter_title","twitter_description","twitter_image","email_subject","feature_image_alt","feature_image_caption"],
        ..Default::default()
    }).await
}
