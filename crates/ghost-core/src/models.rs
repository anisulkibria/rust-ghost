//! Domain models + the canonical Content-API JSON shape.
//!
//! These structs are what repos return; the `*_to_json` functions produce
//! the exact shape templates, the Content API and `{{#get}}` all consume.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::dates;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Post {
    pub id: String,
    pub uuid: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub mobiledoc: Option<String>,
    pub lexical: Option<String>,
    /// markdown source for posts authored in the ghost-rust editor
    pub markdown: Option<String>,
    pub html: Option<String>,
    pub plaintext: Option<String>,
    pub comment_id: Option<String>,
    pub feature_image: Option<String>,
    pub featured: bool,
    #[serde(rename = "type")]
    pub post_type: String, // "post" | "page"
    pub status: String,    // published | draft | scheduled | sent
    pub locale: Option<String>,
    pub visibility: String, // public | members | paid | tiers
    #[serde(default)]
    pub tier_ids: Vec<String>,
    pub email_recipient_filter: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub published_at: Option<String>,
    pub published_by: Option<String>,
    pub custom_excerpt: Option<String>,
    pub auto_excerpt: Option<String>,
    pub reading_time: i64,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub custom_template: Option<String>,
    pub canonical_url: Option<String>,
    pub newsletter_id: Option<String>,
    pub show_title_and_feature_image: bool,
    /// Flattened from posts_meta on read.
    pub meta: Option<PostMeta>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PostMeta {
    pub post_id: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub twitter_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub email_subject: Option<String>,
    pub email_preview: Option<String>,
    pub frontmatter: Option<String>,
    pub feature_image_alt: Option<String>,
    pub feature_image_caption: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub password: Option<String>,
    pub email: Option<String>,
    pub profile_image: Option<String>,
    pub cover_image: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub location: Option<String>,
    pub facebook: Option<String>,
    pub twitter: Option<String>,
    pub threads: Option<String>,
    pub bluesky: Option<String>,
    pub mastodon: Option<String>,
    pub tiktok: Option<String>,
    pub youtube: Option<String>,
    pub instagram: Option<String>,
    pub linkedin: Option<String>,
    pub accessibility: Option<String>,
    pub status: Option<String>,
    pub locale: Option<String>,
    pub visibility: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub tour: Option<String>,
    pub last_seen: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    // Staff email-notification opt-outs, the switches on the staff user's
    // "Email notifications" tab. Defaults mirror Ghost's users table.
    pub comment_notifications: bool,
    pub free_member_signup_notification: bool,
    pub paid_subscription_started_notification: bool,
    pub paid_subscription_canceled_notification: bool,
    pub mention_notifications: bool,
    pub recommendation_notifications: bool,
    pub milestone_notifications: bool,
    pub donation_notifications: bool,
    pub gift_subscription_notifications: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub feature_image: Option<String>,
    pub parent_id: Option<String>,
    pub visibility: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub twitter_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub codeinjection_head: Option<String>,
    pub codeinjection_foot: Option<String>,
    pub canonical_url: Option<String>,
    pub accent_color: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// from include=count
    pub count_posts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub id: String,
    #[serde(rename = "group")]
    pub group: String,
    pub key: String,
    pub value: String,
    #[serde(rename = "type")]
    pub value_type: String,
    pub flags: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl Setting {
    /// Resolve the stored string to its typed JSON value (Ghost semantics).
    pub fn typed_value(&self) -> Value {
        match self.value_type.as_str() {
            "boolean" => matches!(self.value.as_str(), "true" | "1").into(),
            "number" => self
                .value
                .parse::<i64>()
                .map(Value::from)
                .or_else(|_| self.value.parse::<f64>().map(Value::from))
                .unwrap_or(Value::Null),
            "array" | "object" => serde_json::from_str(&self.value).unwrap_or(Value::Null),
            _ => Value::String(self.value.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Integration {
    pub id: String,
    #[serde(rename = "type")]
    pub integration_type: String,
    pub name: String,
    pub slug: Option<String>,
    pub icon_image: Option<String>,
    pub description: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    #[serde(rename = "type")]
    pub key_type: String, // content | admin
    pub secret: String,
    pub role_id: Option<String>,
    pub integration_id: Option<String>,
    pub user_id: Option<String>,
    pub last_seen_at: Option<String>,
    pub last_seen_version: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Serialize a post the way Ghost's Content API / templates see it.
///
/// `url` must be precomputed by the caller (routing-owned permalink).
/// Relations are already resolved: `authors`, `tags` ordered with primary first.
/// Put a stored image field into browser-ready form.
///
/// Image fields are persisted without the install's base path, and every
/// serializer in this module is an output boundary — so this is where it goes
/// on. A theme reading a field raw (`{{post.feature_image}}`) then gets a
/// working URL, and because `url::base_asset` cannot double-apply, routing the
/// same value through `{{img_url}}` stays correct as well.
fn public_image(v: &Option<String>) -> Value {
    match v.as_deref() {
        Some(p) => json!(crate::url::base_asset(p)),
        None => Value::Null,
    }
}

/// The body safe for anonymous readers, including segment-gated cards.
pub fn public_html(post: &Post) -> String {
    let mut html = post.html.clone().unwrap_or_default();
    if post.visibility != "public" {
        html = html.split_once("<!--members-only-->").map(|(preview, _)| preview.to_owned()).unwrap_or_default();
    }
    segment_html(&html,None)
}

/// Resolve Koenig audience blocks before exporting or sending any body.
pub fn segment_html(html:&str,status:Option<&str>)->String {
    static BLOCKS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||
        regex::Regex::new(r"(?s)<!--kg-gated-block:begin ([^\n]+?)\s*-->(.*?)<!--kg-gated-block:end-->").unwrap());
    let paid=matches!(status,Some("paid"|"comped"|"gift"));
    BLOCKS.replace_all(html, |caps: &regex::Captures<'_>| {
        let mut show=caps[1].split_whitespace().any(|param|match param {
            "nonMember:true"=>status.is_none(),"member:true"=>status.is_some(),
            "freeMember:true"=>status==Some("free"),"paidMember:true"=>paid,_=>false
        });
        if let Some(status)=status {
            static SEGMENT:std::sync::LazyLock<regex::Regex>=std::sync::LazyLock::new(||regex::Regex::new(r#"memberSegment:"([^"]*)""#).unwrap());
            if let Some(segment)=SEGMENT.captures(&caps[1]){
                let status=if paid{"paid"}else{status};
                show|=segment[1].split(',').any(|group|!group.is_empty()&&group.split('+').all(|term|{
                    let Some(value)=term.trim().strip_prefix("status:")else{return false};
                    let value=value.trim_matches('\'');
                    if let Some(not)=value.strip_prefix('-'){status!=not}else{status==value}
                }));
            }
        }
        if show{caps[2].to_string()}else{String::new()}
    }).into_owned()
}

/// Apply membership access to every theme-visible body representation.
pub fn apply_post_access(value: &mut Value, member_status: Option<&str>, member_tiers: &[String]) {
    let Some(visibility) = value.get("visibility").and_then(Value::as_str) else { return; };
    let tiers: Vec<String> = value.get("tiers").and_then(Value::as_array).map(|tiers| tiers.iter().filter_map(|tier| tier.get("id").and_then(Value::as_str)).map(str::to_string).collect()).unwrap_or_default();
    let access = crate::comments::check_post_tier_access(visibility, member_status, &tiers, member_tiers);
    if !access {
        let html = public_html(&Post { visibility: visibility.into(), html: value.get("html").and_then(Value::as_str).map(str::to_string), ..Default::default() });
        let plain = crate::comments::to_plaintext(&html);
        value["excerpt"] = value.get("custom_excerpt").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| json!(s))
            .unwrap_or_else(|| json!(excerpt_from_plaintext(Some(&plain), 50).unwrap_or_default()));
        value["plaintext"] = json!(plain);
        value["html"] = json!(html);
        for key in ["lexical", "mobiledoc", "markdown", "auto_excerpt"] { value.as_object_mut().unwrap().remove(key); }
    }
    if access {
        if let Some(html)=value.get("html").and_then(Value::as_str) {
            let filtered=segment_html(html,member_status);
            if filtered!=html {
                let plain=crate::comments::to_plaintext(&filtered);
                value["html"]=json!(filtered);value["plaintext"]=json!(plain);
                if value.get("custom_excerpt").and_then(Value::as_str).is_none_or(|s|s.is_empty()) {value["excerpt"]=json!(excerpt_from_plaintext(Some(&plain),50).unwrap_or_default());}
                for key in ["lexical","mobiledoc","markdown","auto_excerpt"]{value.as_object_mut().unwrap().remove(key);}
            }
        }
    }
    value["access"] = json!(access);
}

pub fn post_to_json(p: &Post, authors: &[User], tags: &[Tag], url: &str) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(p.id));
    m.insert("uuid".into(), json!(p.uuid));
    m.insert("title".into(), json!(p.title));
    m.insert("slug".into(), json!(p.slug));
    // stored post HTML carries root-relative image sources and internal
    // links; both need the base path to resolve in a browser
    m.insert("html".into(), json!(p.html.as_deref().map(crate::url::rewrite_html)));
    m.insert("comment_id".into(), json!(p.comment_id.clone().or_else(|| p.uuid.clone())));
    m.insert("featured".into(), json!(p.featured));
    m.insert("status".into(), json!(p.status));
    m.insert("type".into(), json!(p.post_type));
    m.insert("visibility".into(), json!(p.visibility));
    m.insert("tiers".into(), json!(p.tier_ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()));
    m.insert("reading_time".into(), json!(p.reading_time));
    m.insert("access".into(), json!(p.visibility == "public"));
    m.insert("url".into(), json!(url));
    m.insert("feature_image".into(), public_image(&p.feature_image));
    m.insert(
        "excerpt".into(),
        json!(p
            .custom_excerpt
            .clone()
            .or_else(|| excerpt_from_plaintext(p.plaintext.as_deref(), 50))
            .unwrap_or_default()),
    );
    m.insert("custom_excerpt".into(), json!(p.custom_excerpt));
    m.insert("published_at".into(), json!(dates::db_to_api(p.published_at.as_deref())));
    m.insert("updated_at".into(), json!(dates::db_to_api(p.updated_at.as_deref())));
    m.insert("codeinjection_head".into(), json!(p.codeinjection_head));
    m.insert("codeinjection_foot".into(), json!(p.codeinjection_foot));
    m.insert("custom_template".into(), json!(p.custom_template));
    m.insert("canonical_url".into(), json!(p.canonical_url));
    m.insert(
        "published_by".into(),
        json!(authors.iter().find(|a| Some(a.id.clone()) == p.published_by).map(|a| a.id.clone())),
    );

    // posts_meta flattened
    if let Some(meta) = &p.meta {
        for (k, v) in [
            ("meta_title", &meta.meta_title),
            ("meta_description", &meta.meta_description),
            ("og_title", &meta.og_title),
            ("og_description", &meta.og_description),
            ("twitter_title", &meta.twitter_title),
            ("twitter_description", &meta.twitter_description),
            ("email_subject", &meta.email_subject),
            ("feature_image_alt", &meta.feature_image_alt),
            ("feature_image_caption", &meta.feature_image_caption),
        ] {
            m.insert(k.into(), json!(v));
        }
        m.insert("og_image".into(), public_image(&meta.og_image));
        m.insert("twitter_image".into(), public_image(&meta.twitter_image));
    }

    m.insert("authors".into(), Value::Array(authors.iter().map(|a| user_public_json(a, None)).collect()));
    m.insert(
        "primary_author".into(),
        authors
            .first()
            .map(|a| user_public_json(a, None))
            .unwrap_or(Value::Null),
    );
    m.insert("tags".into(), Value::Array(tags.iter().map(|t| tag_to_json(t, None)).collect()));
    m.insert(
        "primary_tag".into(),
        tags.iter()
            .find(|t| t.visibility == "public")
            .map(|t| tag_to_json(t, None))
            .unwrap_or(Value::Null),
    );
    Value::Object(m)
}

/// Ghost's excerpt fallback: truncate plaintext to N words.
pub fn excerpt_from_plaintext(plaintext: Option<&str>, words: usize) -> Option<String> {
    let pt = plaintext?;
    let out: Vec<&str> = pt.split_whitespace().take(words).collect();
    if out.is_empty() {
        None
    } else {
        Some(out.join(" "))
    }
}

/// Public author shape (also used for staff on admin endpoints).
pub fn user_public_json(u: &User, url: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(u.id));
    m.insert("name".into(), json!(u.name));
    m.insert("slug".into(), json!(u.slug));
    m.insert("url".into(), json!(url));
    m.insert("profile_image".into(), public_image(&u.profile_image));
    m.insert("cover_image".into(), public_image(&u.cover_image));
    m.insert("bio".into(), json!(u.bio));
    m.insert("website".into(), json!(u.website));
    m.insert("location".into(), json!(u.location));
    m.insert("facebook".into(), json!(u.facebook));
    m.insert("twitter".into(), json!(u.twitter));
    m.insert("threads".into(), json!(u.threads));
    m.insert("bluesky".into(), json!(u.bluesky));
    m.insert("mastodon".into(), json!(u.mastodon));
    m.insert("tiktok".into(), json!(u.tiktok));
    m.insert("youtube".into(), json!(u.youtube));
    m.insert("instagram".into(), json!(u.instagram));
    m.insert("linkedin".into(), json!(u.linkedin));
    m.insert("meta_title".into(), json!(u.meta_title));
    m.insert("meta_description".into(), json!(u.meta_description));
    Value::Object(m)
}

pub fn tag_to_json(t: &Tag, url: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(t.id));
    m.insert("name".into(), json!(t.name));
    m.insert("slug".into(), json!(t.slug));
    m.insert("description".into(), json!(t.description));
    m.insert("feature_image".into(), public_image(&t.feature_image));
    m.insert("url".into(), json!(url));
    m.insert("visibility".into(), json!(t.visibility));
    m.insert("meta_title".into(), json!(t.meta_title));
    m.insert("meta_description".into(), json!(t.meta_description));
    m.insert("og_image".into(), public_image(&t.og_image));
    m.insert("og_title".into(), json!(t.og_title));
    m.insert("og_description".into(), json!(t.og_description));
    m.insert("twitter_image".into(), public_image(&t.twitter_image));
    m.insert("twitter_title".into(), json!(t.twitter_title));
    m.insert("twitter_description".into(), json!(t.twitter_description));
    m.insert("codeinjection_head".into(), json!(t.codeinjection_head));
    m.insert("codeinjection_foot".into(), json!(t.codeinjection_foot));
    m.insert("canonical_url".into(), json!(t.canonical_url));
    m.insert("accent_color".into(), json!(t.accent_color));
    if let Some(count) = t.count_posts {
        m.insert("count".into(), json!({ "posts": count }));
    }
    Value::Object(m)
}

/// Ghost pagination meta.
pub fn pagination_json(page: i64, limit: i64, total: i64) -> Value {
    let pages = if limit > 0 { (total + limit - 1) / limit } else { 1 }.max(1);
    json!({
        "page": page,
        "limit": limit,
        "pages": pages,
        "total": total,
        "next": if page < pages { Some(page + 1) } else { None },
        "prev": if page > 1 { Some(page - 1) } else { None },
    })
}

#[cfg(test)]
mod segment_membership_tests {
    use super::*;
    #[test]
    fn current_ghost_member_segments_gate_podcast_content(){
        let all=r#"<!--kg-gated-block:begin nonMember:false memberSegment:"status:free,status:-free" --><p>Podcast</p><!--kg-gated-block:end-->"#;
        assert_eq!(segment_html(all,None),"");
        for status in ["free","paid","comped","gift"]{assert_eq!(segment_html(all,Some(status)),"<p>Podcast</p>");}
        let paid=r#"<!--kg-gated-block:begin nonMember:false memberSegment:"status:-free" --><p>Paid</p><!--kg-gated-block:end-->"#;
        assert_eq!(segment_html(paid,Some("free")),"");assert_eq!(segment_html(paid,Some("gift")),"<p>Paid</p>");
    }
}
