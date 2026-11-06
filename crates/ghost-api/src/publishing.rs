//! Publishing workflows exposed to both the HTML admin and integrations.
use crate::{permissions::{self,Role},AppState};
use axum::{extract::{State,Path,Query,OriginalUri},http::{StatusCode,HeaderMap},response::{Response,IntoResponse},routing::{get,post},Json,Router};
use ghost_core::{ids,dates,models::{Post,User}};
use ghost_db::repos;
use serde_json::{json,Value};
use std::collections::HashMap;
use base64::Engine;

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
                .route(&format!("/{resource}/{{id}}/gift_links{suffix}"),get(read_gift).post(gift).put(ensure_gift).delete(revoke_gift))
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
    // `created_at_ts` (milliseconds) is the real order; the id only breaks a
    // same-millisecond tie, and does so stably on both engines. It replaces
    // SQLite's `rowid`, which MySQL does not have and this table's
    // random-text primary key cannot stand in for.
    let rows=sqlx::query_as::<_,(String,String,ghost_db::types::Text)>("SELECT id,created_at,snapshot FROM post_revisions WHERE post_id=? AND snapshot IS NOT NULL ORDER BY created_at_ts DESC,id DESC LIMIT 100")
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
        for id in &tier_ids{let valid: bool = sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM products WHERE id=? AND type='paid')").bind(id).fetch_one(&state.db).await.unwrap_or(0) != 0;if !valid{return error(StatusCode::UNPROCESSABLE_ENTITY,"Unknown paid tier")}}
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
    // Upsert, then read the id back: `RETURNING` is SQLite-only. The select
    // keys on the same unique triple the upsert conflicts on, so it finds
    // either the row just inserted or the one just updated.
    let upserted = sqlx::query(ghost_db::dialect::sql("INSERT INTO saved_views (id,user_id,resource,name,filter,created_at) VALUES (?,?,?,?,?,?) ON CONFLICT(user_id,resource,name) DO UPDATE SET filter=excluded.filter"))
        .bind(&id).bind(&user.id).bind(resource).bind(name).bind(filter).bind(dates::now_db()).execute(&state.db).await;
    if upserted.is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not save view");
    }
    match sqlx::query_scalar::<_, String>(
        "SELECT id FROM saved_views WHERE user_id=? AND resource=? AND name=?",
    )
        .bind(&user.id).bind(resource).bind(name).fetch_one(&state.db).await {
        Ok(saved_id)=>(StatusCode::CREATED,Json(json!({"views":[{"id":saved_id,"resource":resource,"name":name,"filter":filter}]}))).into_response(),Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not save view")
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
    mint_gift(state,headers,id,false).await
}
async fn ensure_gift(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    mint_gift(state,headers,id,true).await
}
async fn gift_post(state:&AppState,headers:&HeaderMap,id:&str)->std::result::Result<Post,Response> {
    permissions::require(state,headers,Role::Editor).await?;
    repos::posts::find_by_id(&state.db,id).await.map_err(failed)?
        .ok_or_else(||error(StatusCode::NOT_FOUND,"Post not found"))
}
async fn gift_response(state:&AppState,post:&Post,link:Option<repos::gift_links::GiftLink>)->Response {
    let links=if let Some(link)=link {
        let value=serialized(state,post).await;
        let mut url=value["url"].as_str().unwrap_or("").to_owned();
        let separator=if url.contains('?') {'&'} else {'?'};
        url.push_str(&format!("{separator}gift={}",link.token));
        vec![json!({"token":link.token,"created_at":dates::db_to_api(Some(&link.created_at)),"url":url})]
    } else {Vec::new()};
    ([("cache-control","private, no-store")],Json(json!({"gift_links":links}))).into_response()
}
async fn read_gift(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    let post=match gift_post(&state,&headers,&id).await {Ok(post)=>post,Err(r)=>return r};
    match repos::gift_links::read(&state.db,&id).await {
        Ok(link)=>gift_response(&state,&post,link).await,Err(e)=>failed(e)
    }
}
async fn mint_gift(state:AppState,headers:HeaderMap,id:String,ensure:bool)->Response {
    let post=match gift_post(&state,&headers,&id).await {Ok(post)=>post,Err(r)=>return r};
    let token=base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8;24]>());
    match repos::gift_links::mint(&state.db,&id,&token,ensure).await {
        Ok((link,changed))=>{
            if changed {record_gift_action(&state,&headers,Some(&id),if ensure {"added"} else {"edited"}).await;}
            gift_response(&state,&post,Some(link)).await
        },Err(e)=>failed(e)
    }
}
async fn revoke_gift(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=gift_post(&state,&headers,&id).await{return r}
    match repos::gift_links::revoke(&state.db,Some(&id)).await{
        Ok(count)=>{
            if count>0 {record_gift_action(&state,&headers,Some(&id),"deleted").await;}
            StatusCode::NO_CONTENT.into_response()
        },Err(e)=>failed(e)
    }
}
pub(crate) async fn record_gift_action(state:&AppState,headers:&HeaderMap,subject:Option<&str>,event:&str) {
    let integration=if let Some(header)=headers.get("authorization").and_then(|h|h.to_str().ok()) {
        if let Some(token)=crate::apikey_auth::token_from_header(header) {
            crate::apikey_auth::verify(state,token,"admin").await.and_then(|key|key.integration_id)
        } else {None}
    } else {None};
    let actor=if let Some(id)=integration {Some((id,"integration"))}
        else {crate::admin::current_user(state,headers).await.map(|user|(user.id,"user"))};
    if let Some((id,kind))=actor {
        let context=if event=="edited" {json!({"action_name":"reset"})} else {json!({})};
        let _=sqlx::query("INSERT INTO actions(id,resource_id,resource_type,actor_id,actor_type,event,context,created_at) VALUES(?,?,'gift_link',?,?,?,?,?)")
            .bind(ids::new_id()).bind(subject).bind(id).bind(kind).bind(event).bind(context.to_string()).bind(dates::now_db()).execute(&state.db).await;
    }
}
pub async fn valid_gift(state:&AppState,post_id:&str,token:&str)->bool {
    repos::gift_links::valid(&state.db,post_id,token).await.unwrap_or(false)
}

async fn email_preview(state:&AppState,post:&Post)->ghost_core::Result<String>{
    let value=serialized(state,post).await;
    email_preview_with_value(state,post,&value).await
}
/// Render a newsletter from an in-memory editor snapshot without saving it.
pub async fn email_preview_with_value(state:&AppState,post:&Post,value:&Value)->ghost_core::Result<String>{
    let newsletter=if let Some(id)=post.newsletter_id.as_deref() {
        repos::newsletters::find_by_id(&state.db,id).await?
    } else {repos::newsletters::default_newsletter(&state.db).await?}
        .ok_or_else(||ghost_core::GhostError::BadRequest("Create or select a newsletter first".into()))?;
    let site_url=state.site_url();let meta=post.meta.clone().unwrap_or_default();
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
    let value=serialized(&state,&post).await;
    ([("cache-control","private, no-store")],Json(json!({"preview":preview_data(&state,&post,&value).await}))).into_response()
}

/// Web/email/social previews share the same transient post and relation values.
pub async fn preview_data(state:&AppState,post:&Post,value:&Value)->Value {
    let (email,email_error)=match email_preview_with_value(state,post,value).await {
        Ok(html)=>(html,None),Err(e)=>(String::new(),Some(e.message()))
    };
    let social=social_previews(state,post);
    json!({
        "web_url":format!("{}/p/{}/",state.site_url(),post.uuid.as_deref().unwrap_or("")),
        "url":value["url"],
        "email_html":email,
        "email_error":email_error,
        // Both spellings: `social_html` is the Facebook/Open Graph card, kept
        // under its original key, and `twitter_html` is the X card beside it.
        "social_html":social.0,
        "twitter_html":social.1,
        "newsletter_name":newsletter_name_for(state,post).await,
    })
}

/// Which newsletter the email preview was rendered as.
///
/// Shown next to the email tab because the rendered design, sender name and
/// footer all come from the newsletter — a publisher looking at an email
/// preview needs to know which one they are looking at, especially on a site
/// with several.
async fn newsletter_name_for(state:&AppState,post:&Post)->Value{
    let found=match post.newsletter_id.as_deref().filter(|id|!id.is_empty()) {
        Some(id)=>ghost_db::repos::newsletters::find_by_id(&state.db,id).await.ok().flatten(),
        None=>ghost_db::repos::newsletters::default_newsletter(&state.db).await.ok().flatten(),
    };
    found.map(|n|json!(n.name)).unwrap_or(Value::Null)
}

/// The two social cards Ghost previews: Facebook, then X.
///
/// Ghost shows these separately rather than as one generic card because they
/// are not the same preview. Each reads its **own** meta fields — `og_*` for
/// Facebook, `twitter_*` for X — falling back through the post's own title
/// and excerpt, so a publisher who set a different headline for one network
/// can see that it took. They also crop differently and truncate at
/// different lengths, which is the thing a preview is for: checking the
/// headline does not get cut in half.
fn social_previews(state:&AppState,post:&Post)->(String,String){
    let esc=|s:&str|s.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;");
    let meta=post.meta.as_ref();
    let domain=state.site_url().replace("https://","").replace("http://","").trim_end_matches('/').to_string();
    let absolute=|path:&str|ghost_core::url::absolute(&state.site_url(),path);

    let fallback_title=post.title.as_deref().unwrap_or("(Untitled)");
    let fallback_excerpt=post.custom_excerpt.as_deref()
        .or(post.plaintext.as_deref())
        .map(|text|text.chars().take(300).collect::<String>())
        .unwrap_or_default();
    let fallback_image=post.feature_image.as_deref().unwrap_or("");

    let card=|title:&str,description:&str,image:&str,network:&str|->String{
        let image_block=if image.is_empty() {
            // Ghost shows a neutral placeholder rather than a broken image, so
            // the absence of a social image is itself visible.
            r#"<div class="ph"></div>"#.to_string()
        } else {
            format!(r#"<img alt="" src="{}">"#,esc(&absolute(image)))
        };
        let frame=if network=="x" {"x"} else {"fb"};
        format!(
            concat!(
                "<!doctype html><meta charset=utf-8><style>",
                "body{{margin:0;padding:24px;background:#fff;",
                "font:15px -apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;",
                "-webkit-font-smoothing:antialiased}}",
                "h1{{font-size:13px;font-weight:600;letter-spacing:.02em;text-transform:uppercase;",
                "color:#626d79;margin:0 0 12px}}",
                "article{{max-width:540px}}",
                "img,.ph{{display:block;width:100%;aspect-ratio:1.91/1;object-fit:cover;background:#e4e6eb}}",
                ".ph{{background:repeating-linear-gradient(45deg,#e4e6eb,#e4e6eb 10px,#dadde1 10px,#dadde1 20px)}}",
                // Facebook: square corners, grey caption strip, domain above the headline.
                ".fb article{{border:1px solid #dadde1;border-radius:8px;overflow:hidden}}",
                ".fb section{{background:#f2f3f5;padding:10px 12px}}",
                ".fb small{{display:block;font-size:12px;color:#606770;text-transform:uppercase;",
                "letter-spacing:.02em;margin-bottom:3px}}",
                ".fb h2{{font-size:16px;font-weight:600;color:#1d2129;margin:0;line-height:20px;",
                "display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}",
                ".fb p{{font-size:14px;color:#606770;margin:3px 0 0;line-height:18px;",
                "display:-webkit-box;-webkit-line-clamp:1;-webkit-box-orient:vertical;overflow:hidden}}",
                // X: rounded card, headline inside the border, domain last.
                ".x article{{border:1px solid #e1e8ed;border-radius:16px;overflow:hidden}}",
                ".x section{{padding:12px}}",
                ".x h2{{font-size:15px;font-weight:700;color:#0f1419;margin:0;line-height:20px;",
                "display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}",
                ".x p{{font-size:15px;color:#536471;margin:2px 0 0;line-height:20px;",
                "display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}",
                ".x small{{display:block;font-size:15px;color:#536471;margin-top:2px}}",
                "@media (prefers-color-scheme:dark){{body{{background:#15171a}}}}",
                "</style><div class=\"{frame}\"><h1>{label}</h1><article>{image}",
                "<section>{caption_before}<h2>{title}</h2><p>{description}</p>{caption_after}</section>",
                "</article></div>",
            ),
            frame=frame,
            label=if network=="x" {"X (Twitter) card"} else {"Facebook card"},
            image=image_block,
            // Facebook puts the domain above the headline; X puts it below.
            caption_before=if network=="x" {String::new()} else {format!("<small>{}</small>",esc(&domain))},
            caption_after=if network=="x" {format!("<small>{}</small>",esc(&domain))} else {String::new()},
            title=esc(title),
            description=esc(description),
        )
    };

    let facebook=card(
        meta.and_then(|m|m.og_title.as_deref()).filter(|t|!t.is_empty()).unwrap_or(fallback_title),
        meta.and_then(|m|m.og_description.as_deref()).filter(|d|!d.is_empty()).unwrap_or(&fallback_excerpt),
        meta.and_then(|m|m.og_image.as_deref()).filter(|i|!i.is_empty()).unwrap_or(fallback_image),
        "facebook",
    );
    let x=card(
        meta.and_then(|m|m.twitter_title.as_deref()).filter(|t|!t.is_empty()).unwrap_or(fallback_title),
        meta.and_then(|m|m.twitter_description.as_deref()).filter(|d|!d.is_empty()).unwrap_or(&fallback_excerpt),
        meta.and_then(|m|m.twitter_image.as_deref()).filter(|i|!i.is_empty()).unwrap_or(fallback_image),
        "x",
    );
    (facebook,x)
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
    // `EXISTS` is 0/1; `Any` does not decode an integer as a `bool`. The
    // fallback stays "assume already emailed", which is the safe answer.
    if sqlx::query_scalar::<_,i64>("SELECT EXISTS(SELECT 1 FROM emails WHERE post_id=?)").bind(&id).fetch_one(&state.db).await.unwrap_or(1)!=0{return error(StatusCode::CONFLICT,"This post has already been emailed")}
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
            // An export from Ghost 4 or earlier carries its content as
            // Mobiledoc and nothing else. Converting it to Lexical here is
            // what makes the imported post renderable and editable; without
            // it the source arrives intact and unreadable.
            let has_lexical=post["lexical"].as_str().is_some_and(|l|!l.trim().is_empty());
            if !has_lexical{
                if let Some(mobiledoc)=post["mobiledoc"].as_str().filter(|m|crate::admin::mobiledoc_has_content(m)){
                    post["lexical"]=json!(ghost_render::mobiledoc::to_lexical(Some(mobiledoc)));
                }
            }
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

#[cfg(test)]
mod import_tests {
    use super::*;

    const LEGACY: &str = r#"{"version":"0.3.1","markups":[["strong"]],"atoms":[],"cards":[],
        "sections":[[1,"h2",[[0,[],0,"Old heading"]]],[1,"p",[[0,[0],1,"Bold words"]]]]}"#;

    #[test]
    fn a_mobiledoc_only_post_gains_lexical_and_html() {
        // A Ghost 4 export: the source is there, and before this nothing could
        // read it.
        let prepared = prepare_content_import(&json!({
            "posts": [{ "title": "Legacy", "mobiledoc": LEGACY }]
        }));
        let post = &prepared["posts"][0];
        let lexical: Value = serde_json::from_str(post["lexical"].as_str().unwrap()).unwrap();
        assert_eq!(lexical["root"]["children"][0]["type"], json!("heading"));
        let html = post["html"].as_str().unwrap();
        assert!(html.contains("Old heading"), "rendered html: {html}");
        assert!(html.contains("Bold words"), "rendered html: {html}");
        assert!(post["plaintext"].as_str().is_some_and(|p| p.contains("Old heading")));
        assert!(post["reading_time"].as_i64().is_some());
    }

    #[test]
    fn an_incoming_lexical_is_not_replaced_by_a_legacy_mobiledoc() {
        // Exports can carry both; the Lexical is the newer source of truth.
        let lexical = r#"{"root":{"children":[{"type":"paragraph","version":1,"children":[{"text":"current","type":"text","format":0,"version":1}]}],"type":"root","version":1,"direction":null,"format":"","indent":0}}"#;
        let prepared = prepare_content_import(&json!({
            "posts": [{ "title": "Both", "mobiledoc": LEGACY, "lexical": lexical }]
        }));
        assert_eq!(prepared["posts"][0]["lexical"].as_str().unwrap(), lexical);
        assert!(prepared["posts"][0]["html"].as_str().unwrap().contains("current"));
    }

    #[test]
    fn supplied_html_is_left_alone() {
        let prepared = prepare_content_import(&json!({
            "posts": [{ "title": "Rendered", "mobiledoc": LEGACY, "html": "<p>as exported</p>" }]
        }));
        let post = &prepared["posts"][0];
        assert_eq!(post["html"], json!("<p>as exported</p>"), "an export's own html wins");
        // the editable source is still migrated, so the post can be opened
        assert!(post["lexical"].as_str().is_some_and(|l| l.contains("heading")));
    }

    #[test]
    fn an_empty_mobiledoc_placeholder_is_not_treated_as_content() {
        for placeholder in ["{}", "", r#"{"version":"0.3.1","sections":[]}"#] {
            let prepared = prepare_content_import(&json!({
                "posts": [{ "title": "Empty", "mobiledoc": placeholder, "markdown": "# From markdown" }]
            }));
            let post = &prepared["posts"][0];
            assert!(post["lexical"].as_str().is_none(), "{placeholder:?} has no content to convert");
            assert!(
                post["html"].as_str().unwrap().contains("From markdown"),
                "{placeholder:?} must not shadow the markdown source"
            );
        }
    }

    #[test]
    fn a_post_with_no_content_at_all_is_passed_through() {
        let prepared = prepare_content_import(&json!({ "posts": [{ "title": "Bare" }] }));
        let post = &prepared["posts"][0];
        assert_eq!(post["title"], json!("Bare"));
        assert!(post["html"].as_str().is_none());
    }
}
