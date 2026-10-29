//! Publishing workflows exposed to both the HTML admin and integrations.
use crate::{permissions::{self,Role},AppState};
use axum::{extract::{State,Path,Query,OriginalUri},http::{StatusCode,HeaderMap},response::{Response,IntoResponse},routing::{get,post},Json,Router};
use ghost_core::{ids,dates,models::{Post,User}};
use ghost_db::repos;
use serde_json::{json,Value};
use std::collections::HashMap;
use sha2::{Digest,Sha256};

pub fn router()->Router<AppState>{
    let mut router=Router::new().route("/views/",get(views).post(save_view))
        .route("/views/{id}/",axum::routing::delete(delete_view))
        .route("/editor/links/",get(search_links))
        .route("/editor/context/",get(editor_context))
        .route("/editor/newsletters/",get(editor_newsletters));
    // Ghost's own spellings of the same operations, so its SDK, Zapier and
    // every integration written against the documented API reach them. The
    // port's own paths above stay: the admin's editor is written to them.
    for resource in ["posts","pages"] {
        for suffix in ["","/"] {
            router=router
                // `gift_links` is what Ghost calls an article gift link.
                .route(&format!("/{resource}/{{id}}/gift_links{suffix}"),get(gift).post(gift).put(gift).delete(revoke_gift))
                .route(&format!("/email_previews/{resource}/{{id}}{suffix}"),get(ghost_preview).post(test_email));
        }
        // Ghost bulk-edits with PUT rather than POSTing to `/bulk/`. The
        // slashed form already carries this port's own POST, so only the
        // method it is missing is added there.
        router=router
            .route(&format!("/{resource}/bulk"),axum::routing::put(bulk).post(bulk))
            .route(&format!("/{resource}/bulk/"),axum::routing::put(bulk));
    }
    for resource in ["posts","pages"] {
        router=router.route(&format!("/{resource}/{{id}}/copy/"),post(copy))
            .route(&format!("/{resource}/{{id}}/copy"),post(copy))
            .route(&format!("/{resource}/{{id}}/revisions/"),get(revisions))
            .route(&format!("/{resource}/{{id}}/revisions/{{revision}}/"),axum::routing::put(restore))
            .route(&format!("/{resource}/{{id}}/gift/"),post(gift).delete(revoke_gift))
            .route(&format!("/{resource}/bulk/"),post(bulk))
            .route(&format!("/{resource}/{{id}}/preview/"),get(preview))
            .route(&format!("/{resource}/{{id}}/test/"),post(test_email))
            .route(&format!("/{resource}/{{id}}/audience/"),get(audience).put(save_audience));
    }
    router
}
fn error(status:StatusCode,message:&str)->Response{(status,Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]}))).into_response()}
fn failed(error:ghost_core::GhostError)->Response{let status=match error{ghost_core::GhostError::NotFoundError{..}=>StatusCode::NOT_FOUND,ghost_core::GhostError::ValidationError{..}=>StatusCode::UNPROCESSABLE_ENTITY,_=>StatusCode::INTERNAL_SERVER_ERROR};self::error(status,&error.message())}
async fn authorize(state:&AppState,headers:&HeaderMap,id:&str)->std::result::Result<(User,Post),Response>{
    let user=permissions::require(state,headers,Role::Contributor).await?;
    let post=repos::posts::find_by_id(&state.db,id).await.map_err(failed)?.ok_or_else(||error(StatusCode::NOT_FOUND,"Post not found"))?;
    permissions::post_access(state,&user,&post).await?; Ok((user,post))
}
async fn serialized(state:&AppState,post:&Post)->Value {
    let relations=repos::posts::load_relations(&state.db,std::slice::from_ref(post)).await.unwrap_or_default();
    let mut value=ghost_core::models::post_to_json(post,&relations.authors_for(&post.id),&relations.tags_for(&post.id),"");
    let resolver=crate::content::resolve(state).await;value["url"]=json!(resolver(&value));value
}
async fn copy(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    let (user,_)=match authorize(&state,&headers,&id).await{Ok(v)=>v,Err(r)=>return r};
    match repos::publishing::duplicate(&state.db,&id,&user.id).await {
        Ok(post)=>{let _=repos::actions::record(&state.db,&post.post_type,&post.id,&user.id,"added").await;
            (StatusCode::CREATED,Json(json!({if post.post_type=="page"{"pages"}else{"posts"}:[serialized(&state,&post).await]}))).into_response()},
        Err(e)=>failed(e)
    }
}
async fn revisions(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=authorize(&state,&headers,&id).await{return r}
    let rows=sqlx::query_as::<_,(String,String,String)>("SELECT id,created_at,snapshot FROM post_revisions WHERE post_id=? AND snapshot IS NOT NULL ORDER BY created_at_ts DESC,rowid DESC LIMIT 100")
        .bind(id).fetch_all(&state.db).await;
    match rows{Ok(rows)=>Json(json!({"revisions":rows.into_iter().map(|(id,date,snapshot)|{
        let snapshot:Value=serde_json::from_str(&snapshot).unwrap_or(Value::Null);
        json!({"id":id,"created_at":date,"title":snapshot["post"]["title"],"post":snapshot["post"]})
    }).collect::<Vec<_>>()})).into_response(),Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not load revisions")}
}
async fn restore(State(state):State<AppState>,headers:HeaderMap,Path((id,revision)):Path<(String,String)>)->Response {
    let (user,post)=match authorize(&state,&headers,&id).await{Ok(v)=>v,Err(r)=>return r};
    if let Err(r)=permissions::post_input(&state,&user,&json!({}),Some(&post)).await{return r}
    // A prior snapshot may have a different author list or code injection.
    if permissions::role(&state,&user).await.unwrap_or(Role::Contributor)<Role::Editor {
        let snapshot=sqlx::query_scalar::<_,String>("SELECT snapshot FROM post_revisions WHERE id=? AND post_id=?").bind(&revision).bind(&id).fetch_optional(&state.db).await.ok().flatten();
        if let Some(snapshot)=snapshot.and_then(|s|serde_json::from_str::<repos::publishing::Snapshot>(&s).ok()) {
            let body=json!({"authors":snapshot.author_ids.iter().map(|id|json!({"id":id})).collect::<Vec<_>>(),"codeinjection_head":snapshot.post.codeinjection_head,"codeinjection_foot":snapshot.post.codeinjection_foot});
            if let Err(r)=permissions::post_input(&state,&user,&body,Some(&post)).await{return r}
        }
    }
    match repos::publishing::restore(&state.db,&id,&revision).await {
        Ok(post)=>{let _=repos::actions::record(&state.db,&post.post_type,&id,&user.id,"restored").await;
            Json(json!({if post.post_type=="page"{"pages"}else{"posts"}:[serialized(&state,&post).await]})).into_response()},Err(e)=>failed(e)
    }
}
async fn bulk(State(state):State<AppState>,headers:HeaderMap,OriginalUri(uri):OriginalUri,Query(q):Query<HashMap<String,String>>,Json(body):Json<Value>)->Response {
    let user=match permissions::require(&state,&headers,Role::Editor).await{Ok(u)=>u,Err(r)=>return r};
    // Two body shapes reach here: this port's own `{ids, action, …}`, which
    // the admin's post list sends, and Ghost's `{bulk:{action, meta:{…}}}`,
    // which its SDK sends to the same operation. Accepting both is the point
    // of carrying Ghost's path as well — a path that rejects every Ghost
    // client's payload would be no use to one.
    let body=match body.get("bulk") {
        Some(wrapped)=>{
            let meta=wrapped.get("meta").cloned().unwrap_or(Value::Null);
            let take=|key:&str|wrapped.get(key).or_else(||meta.get(key)).cloned().unwrap_or(Value::Null);
            json!({"ids":take("ids"),"action":take("action"),"visibility":take("visibility"),"tags":take("tags"),"tiers":take("tiers")})
        }
        None=>body,
    };
    let mut body=body;
    if body["ids"].as_array().is_none_or(|ids|ids.is_empty()) {
        let Some(filter)=q.get("filter").filter(|s|!s.is_empty())else{return error(StatusCode::UNPROCESSABLE_ENTITY,"Choose posts or provide a filter")};
        let expression=match ghost_nql::parse(filter){Ok(e)=>e,Err(_)=>return error(StatusCode::UNPROCESSABLE_ENTITY,"Invalid post filter")};
        let kind=if uri.path().contains("/pages/"){"page"}else{"post"};
        let query=repos::posts::BrowsePosts{filter:Some(expression),post_type:Some(kind.into()),public:false,limit:-1,..Default::default()};
        match repos::posts::browse(&state.db,&query).await{Ok(rows)=>body["ids"]=json!(rows.posts.iter().map(|p|p.id.clone()).collect::<Vec<_>>()),Err(e)=>return failed(e)}
    }
    let ids=body["ids"].as_array().cloned().unwrap_or_default();
    if ids.is_empty()||ids.len()>10000||ids.iter().any(|v|v.as_str().is_none()){return error(StatusCode::UNPROCESSABLE_ENTITY,"Choose 1–10000 posts")}
    let requested=body["action"].as_str().unwrap_or("");
    let action=match requested{"unpublish"|"unschedule"=>"draft","access"=>"visibility","addTag"=>"tags",other=>other};
    if !matches!(action,"publish"|"draft"|"feature"|"unfeature"|"visibility"|"tags"){return error(StatusCode::UNPROCESSABLE_ENTITY,"Unsupported bulk action")}
    // Validate the complete selection before changing any post.
    let mut posts=Vec::new();
    for id in &ids {
        let Some(post)=repos::posts::find_by_id(&state.db,id.as_str().unwrap()).await.ok().flatten()else{return error(StatusCode::NOT_FOUND,"Post not found")};
        if action=="publish"&&post.visibility=="tiers"&&post.tier_ids.is_empty(){return error(StatusCode::UNPROCESSABLE_ENTITY,"Choose paid tiers before publishing")}
        posts.push(post);
    }
    let visibility=body["visibility"].as_str().unwrap_or("");
    if action=="visibility" && !matches!(visibility,"public"|"members"|"paid"|"tiers"){return error(StatusCode::UNPROCESSABLE_ENTITY,"Invalid bulk visibility")}
    let tags:Option<Vec<String>>=body["tags"].as_array().map(|v|v.iter().filter_map(|id|id.as_str().or_else(||id["id"].as_str()).map(str::to_owned)).collect());
    if action=="tags" {
        let Some(tags)=tags.as_ref()else{return error(StatusCode::UNPROCESSABLE_ENTITY,"Tags must be an array")};
        for id in tags{if repos::tags::find_by_id(&state.db,id).await.ok().flatten().is_none(){return error(StatusCode::UNPROCESSABLE_ENTITY,"Tag not found")}}
    }
    let tier_ids:Vec<String>=body["tiers"].as_array().into_iter().flatten().filter_map(|t|t.as_str().or_else(||t["id"].as_str()).map(str::to_owned)).collect();
    if action=="visibility"&&visibility=="tiers" {
        if tier_ids.is_empty(){return error(StatusCode::UNPROCESSABLE_ENTITY,"Choose paid tiers")}
        for id in &tier_ids{let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE id=? AND type='paid')").bind(id).fetch_one(&state.db).await.unwrap_or(false);if !valid{return error(StatusCode::UNPROCESSABLE_ENTITY,"Unknown paid tier")}}
    }
    let mut done=Vec::new();
    for post in posts {
        let mut tags=tags.clone();
        if requested=="addTag" {
            let current:Vec<String>=sqlx::query_scalar("SELECT tag_id FROM posts_tags WHERE post_id=? ORDER BY sort_order").bind(&post.id).fetch_all(&state.db).await.unwrap_or_default();
            let extra=tags.get_or_insert_with(Vec::new);let mut all=current;for id in extra.iter(){if !all.contains(id){all.push(id.clone())}}tags=Some(all);
        }
        let before=serialized(&state,&post).await;
        let changes=repos::posts::PostChanges{status:match action{"publish"=>Some("published".into()),"draft"=>Some("draft".into()),_=>None},
            featured:match action{"feature"=>Some(true),"unfeature"=>Some(false),_=>None},visibility:(action=="visibility").then(||visibility.into()),
            tag_ids:if action=="tags"{tags}else{None},tier_ids:if action=="visibility"{Some(if visibility=="tiers"{tier_ids.clone()}else{Vec::new()})}else{None},..Default::default()};
        match repos::posts::update(&state.db,&post.id,&changes).await{Ok(p)=>{let _=repos::actions::record(&state.db,&post.post_type,&post.id,&user.id,"edited").await;crate::webhooks::fire_post_change(&state,&serialized(&state,&p).await,Some(&before),"edited");done.push(p.id)},Err(e)=>return failed(e)}
    }
    Json(json!({"bulk":{"action":action,"successful":done}})).into_response()
}
async fn views(State(state):State<AppState>,headers:HeaderMap,Query(q):Query<HashMap<String,String>>)->Response {
    let user=match permissions::require(&state,&headers,Role::Contributor).await{Ok(u)=>u,Err(r)=>return r};
    let rows=sqlx::query_as::<_,(String,String,String,String)>("SELECT id,resource,name,filter FROM saved_views WHERE user_id=? AND (?='' OR resource=?) ORDER BY name")
        .bind(user.id).bind(q.get("resource").map(String::as_str).unwrap_or("")).bind(q.get("resource").map(String::as_str).unwrap_or("")).fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"views":rows.into_iter().map(|(id,resource,name,filter)|json!({"id":id,"resource":resource,"name":name,"filter":filter})).collect::<Vec<_>>()})).into_response()
}
async fn save_view(State(state):State<AppState>,headers:HeaderMap,Json(body):Json<Value>)->Response {
    let user=match permissions::require(&state,&headers,Role::Contributor).await{Ok(u)=>u,Err(r)=>return r};
    let name=body["name"].as_str().unwrap_or("").trim();let resource=body["resource"].as_str().unwrap_or("");let filter=body["filter"].as_str().unwrap_or("");
    if name.is_empty()||name.len()>100||!matches!(resource,"posts"|"pages"|"members")||filter.len()>2000{return error(StatusCode::UNPROCESSABLE_ENTITY,"Invalid saved view")}
    let id=ids::new_id();
    match sqlx::query("INSERT INTO saved_views (id,user_id,resource,name,filter,created_at) VALUES (?,?,?,?,?,?) ON CONFLICT(user_id,resource,name) DO UPDATE SET filter=excluded.filter RETURNING id")
        .bind(&id).bind(user.id).bind(resource).bind(name).bind(filter).bind(dates::now_db()).fetch_one(&state.db).await {
        Ok(row)=>(StatusCode::CREATED,Json(json!({"views":[{"id":sqlx::Row::get::<String,_>(&row,"id"),"resource":resource,"name":name,"filter":filter}]}))).into_response(),Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not save view")
    }
}
async fn delete_view(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    let user=match permissions::require(&state,&headers,Role::Contributor).await{Ok(u)=>u,Err(r)=>return r};
    match sqlx::query("DELETE FROM saved_views WHERE id=? AND user_id=?").bind(id).bind(user.id).execute(&state.db).await{Ok(r)if r.rows_affected()>0=>StatusCode::NO_CONTENT.into_response(),_=>StatusCode::NOT_FOUND.into_response()}
}
async fn search_links(State(state):State<AppState>,headers:HeaderMap,Query(q):Query<HashMap<String,String>>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Contributor).await{return r}
    let search=q.get("q").map(String::as_str).unwrap_or("");
    let ids=sqlx::query_scalar::<_,String>("SELECT id FROM posts WHERE status='published' AND (title LIKE ? OR slug LIKE ?) ORDER BY published_at DESC LIMIT 20")
        .bind(format!("%{search}%")).bind(format!("%{search}%")).fetch_all(&state.db).await.unwrap_or_default();
    let mut links=Vec::new();
    for id in ids {if let Ok(Some(post))=repos::posts::find_by_id(&state.db,&id).await{let value=serialized(&state,&post).await;links.push(json!({"label":post.title,"value":value["url"],"title":post.title,"url":value["url"],"id":post.id}));}}
    Json(json!({"links":links})).into_response()
}
async fn gift(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let Some(post)=repos::posts::find_by_id(&state.db,&id).await.ok().flatten()else{return StatusCode::NOT_FOUND.into_response()};
    if post.status!="published"{return error(StatusCode::UNPROCESSABLE_ENTITY,"Publish the post before creating a gift link")}
    let token=ids::random_hex(32);let hash=hex::encode(Sha256::digest(token.as_bytes()));
    if sqlx::query("INSERT INTO article_gift_links (id,post_id,token_hash,created_at) VALUES (?,?,?,?) ON CONFLICT(post_id) DO UPDATE SET token_hash=excluded.token_hash, created_at=excluded.created_at")
        .bind(ids::new_id()).bind(id).bind(hash).bind(dates::now_db()).execute(&state.db).await.is_err(){return error(StatusCode::INTERNAL_SERVER_ERROR,"Could not create gift link")}
    let value=serialized(&state,&post).await;
    Json(json!({"gift_links":[{"url":format!("{}?gift={token}",value["url"].as_str().unwrap_or(""))}]})).into_response()
}
async fn revoke_gift(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    match sqlx::query("DELETE FROM article_gift_links WHERE post_id=?").bind(id).execute(&state.db).await{Ok(_)=>StatusCode::NO_CONTENT.into_response(),Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not revoke gift link")}
}
pub async fn valid_gift(state:&AppState,post_id:&str,token:&str)->bool {
    if token.len()!=64{return false}
    let hash=hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM article_gift_links WHERE post_id=? AND token_hash=?)").bind(post_id).bind(hash).fetch_one(&state.db).await.unwrap_or(false)
}

async fn email_preview(state:&AppState,post:&Post)->ghost_core::Result<String>{
    let newsletter=repos::newsletters::default_newsletter(&state.db).await?.ok_or_else(||ghost_core::GhostError::BadRequest("Create a newsletter first".into()))?;
    let value=serialized(state,post).await;let site_url=state.site_url();let meta=post.meta.clone().unwrap_or_default();
    let site=ghost_mail::newsletter::SiteContext{title:state.settings.get_str("title").unwrap_or_default(),url:site_url.clone(),icon_url:state.settings.get_str("icon"),locale:state.settings.get_str("locale").unwrap_or_else(||"en".into()),direction:"ltr".into(),accent_color:state.settings.get_str("accent_color").unwrap_or_default()};
    let content=ghost_mail::newsletter::PostContext{id:post.id.clone(),title:post.title.clone().unwrap_or_default(),url:value["url"].as_str().unwrap_or("").into(),
        html:ghost_core::url::absolutize_html(&site_url,post.html.as_deref().unwrap_or("")),plaintext:post.plaintext.clone().unwrap_or_default(),custom_excerpt:post.custom_excerpt.clone(),
        feature_image:post.feature_image.clone(),feature_image_alt:meta.feature_image_alt,feature_image_caption:meta.feature_image_caption,
        published_at:post.published_at.clone().unwrap_or_else(dates::now_db),authors:value["authors"].as_array().map(|authors|authors.iter().filter_map(|a|a["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),..Default::default()};
    let html=ghost_mail::newsletter::NewsletterEmail{site:&site,newsletter:&newsletter,post:&content,latest_posts:&[],comments_enabled:state.settings.get_str("comments_enabled").as_deref()!=Some("off")}.render()?;
    Ok(ghost_mail::newsletter::personalize(&html,&ghost_mail::newsletter::Recipient{uuid:"preview".into(),email:"preview@example.test".into(),name:Some("Preview reader".into()),status:"paid".into(),created_at:dates::now_db(),unsubscribe_url:format!("{site_url}/#/portal/account/newsletters"),manage_account_url:format!("{site_url}/#/portal/account"),key:String::new()}))
}
async fn preview(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response{
    let (_,post)=match authorize(&state,&headers,&id).await{Ok(v)=>v,Err(r)=>return r};
    let email=match email_preview(&state,&post).await{Ok(html)=>html,Err(e)=>return failed(e)};
    let value=serialized(&state,&post).await;
    let esc=|s:&str|s.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;");
    let title=esc(post.title.as_deref().unwrap_or("Untitled"));let excerpt=esc(post.custom_excerpt.as_deref().or(post.plaintext.as_deref()).unwrap_or(""));
    let image=post.meta.as_ref().and_then(|m|m.og_image.as_deref()).or(post.feature_image.as_deref()).unwrap_or("");
    let social=format!("<!doctype html><meta charset=utf-8><style>body{{font:16px system-ui;padding:24px}}article{{max-width:600px;border:1px solid #ddd;border-radius:18px;overflow:hidden}}img{{width:100%;max-height:320px;object-fit:cover}}section{{padding:24px}}p{{max-height:5em;overflow:hidden}}</style><article><img alt=\"\" src=\"{}\"><section><small>{}</small><h1>{title}</h1><p>{excerpt}</p></section></article>",esc(&ghost_core::url::absolute(&state.site_url(),image)),esc(&state.settings.site_name()));
    ([("cache-control","private, no-store")],Json(json!({"preview":{"web_url":format!("{}/p/{}/",state.site_url(),post.uuid.as_deref().unwrap_or("")),"url":value["url"],"email_html":email,"social_html":social}}))).into_response()
}
async fn test_email(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>,Json(body):Json<Value>)->Response{
    let (user,post)=match authorize(&state,&headers,&id).await{Ok(v)=>v,Err(r)=>return r};
    if permissions::role(&state,&user).await.unwrap_or(Role::Contributor)<Role::Author{return permissions::denied()}
    if body["emails"].as_array().is_some_and(|emails|emails.len()!=1){return error(StatusCode::UNPROCESSABLE_ENTITY,"Choose one test recipient")}
    let email=body["email"].as_str().or_else(||body.pointer("/emails/0").and_then(Value::as_str)).unwrap_or("");
    if !email.contains('@')||email.contains(['\r','\n'])||email.len()>191{return error(StatusCode::UNPROCESSABLE_ENTITY,"Enter an email address")}
    let html=match email_preview(&state,&post).await{Ok(html)=>html,Err(e)=>return failed(e)};
    let cfg=ghost_mail::MailConfig::from_settings(&state.settings,&state.settings.site_name(),&state.site_url());
    let message=ghost_mail::Message{to:email.into(),subject:format!("[Test] {}",post.meta.as_ref().and_then(|m|m.email_subject.as_deref()).or(post.title.as_deref()).unwrap_or("Newsletter")),text:ghost_mail::newsletter::plaintext(&html),html};
    match ghost_mail::send(&cfg,&message).await{Ok(())=>StatusCode::NO_CONTENT.into_response(),Err(e)=>failed(e)}
}

async fn editor_context(State(state):State<AppState>,headers:HeaderMap,Query(q):Query<HashMap<String,String>>)->Response{
    let user=match permissions::require(&state,&headers,Role::Contributor).await{Ok(u)=>u,Err(r)=>return r};
    let path=q.get("path").map(String::as_str).unwrap_or("/");let route=state.router.read().await.match_path(path);
    let slug=match route{Some(ghost_routing::tree::RouteMatch::CollectionEntry{slug,..}|ghost_routing::tree::RouteMatch::StaticPage{slug})=>Some(slug),_=>None};
    let mut context=json!({"name":state.settings.site_name(),"logo":state.settings.get_str("logo").or_else(||state.settings.get_str("icon")),"edit_url":null});
    if let Some(slug)=slug{
        let post=match repos::posts::find_by_slug(&state.db,&slug,"post",true).await{Ok(Some(p))=>Some(p),_=>repos::posts::find_by_slug(&state.db,&slug,"page",true).await.ok().flatten()};
        if let Some(post)=post{if permissions::post_access(&state,&user,&post).await.is_ok(){context["edit_url"]=json!(ghost_core::url::base(&format!("/ghost/{}/{}/",if post.post_type=="page"{"pages"}else{"posts"},post.id)));context["type"]=json!(post.post_type);}}
    }
    ([("cache-control","private, no-store")],Json(json!({"context":context}))).into_response()
}

async fn audience(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response{
    if let Err(r)=authorize(&state,&headers,&id).await{return r}
    match crate::newsletter_audience::load(&state,&id).await{Ok(audience)=>Json(json!({"audience":audience})).into_response(),Err(e)=>failed(e)}
}
async fn save_audience(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>,Json(body):Json<crate::newsletter_audience::Audience>)->Response{
    if let Err(r)=authorize(&state,&headers,&id).await{return r}
    if sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM emails WHERE post_id=?)").bind(&id).fetch_one(&state.db).await.unwrap_or(true){return error(StatusCode::CONFLICT,"This post has already been emailed")}
    match crate::newsletter_audience::save(&state,&id,&body).await{Ok(())=>Json(json!({"audience":body})).into_response(),Err(e)=>failed(e)}
}
async fn editor_newsletters(State(state):State<AppState>,headers:HeaderMap)->Response{
    if let Err(r)=permissions::require(&state,&headers,Role::Contributor).await{return r}
    match repos::newsletters::list(&state.db).await{Ok(rows)=>Json(json!({"newsletters":rows.into_iter().filter(|n|n.status=="active").map(|n|json!({"id":n.id,"name":n.name})).collect::<Vec<_>>()})).into_response(),Err(e)=>failed(e)}
}

/// Fill derived body representations for imports that carry only editor source.
/// This runs before the transactional database import and leaves supplied HTML intact.
pub fn prepare_content_import(data:&Value)->Value{
    let mut data=data.clone();
    if let Some(posts)=data.get_mut("posts").and_then(Value::as_array_mut){
        for post in posts{
            if post["html"].as_str().is_none_or(|html|html.is_empty()){
                let rendered=if let Some(source)=post["markdown"].as_str().filter(|s|!s.is_empty()){
                    Some(ghost_render::markdown::render_markdown(source))
                }else if let Some(source)=post["lexical"].as_str().filter(|s|!s.is_empty()){
                    let html=ghost_render::lexical::to_html(source);let plain=ghost_render::markdown::html_to_plaintext(&html);let time=ghost_render::markdown::reading_time(&plain);Some((html,plain,time))
                }else{None};
                if let Some((html,plain,time))=rendered{post["html"]=json!(html);post["plaintext"]=json!(plain);post["reading_time"]=json!(time);}
            }
        }
    }
    data
}

async fn ghost_preview(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response{
    let (_,post)=match authorize(&state,&headers,&id).await{Ok(v)=>v,Err(r)=>return r};
    match email_preview(&state,&post).await{
        Ok(html)=>{let subject=post.meta.as_ref().and_then(|m|m.email_subject.as_deref()).or(post.title.as_deref()).unwrap_or("");let plaintext=ghost_mail::newsletter::plaintext(&html);([( "cache-control","private,no-store")],Json(json!({"email_previews":[{"html":html,"plaintext":plaintext,"subject":subject}]}))).into_response()},
        Err(e)=>failed(e)
    }
}
