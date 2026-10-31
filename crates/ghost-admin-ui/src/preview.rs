//! Transient post/page previews: rendering never calls a persistence method.
use super::*;
use axum::{extract::OriginalUri, Json};
use ghost_core::{dates, models::{Post, PostMeta}};
use serde_json::{json, Value};

async fn snapshot(state: &AppState, headers: &HeaderMap, id: &str, resource: &str, form: &EditorForm) -> Result<(Post,Value),Response> {
    let actor=require_actor(state,headers).await?;
    let existing=if id=="new" {None} else {
        let post=repos::posts::find_by_id(&state.db,id).await
            .map_err(|e|editor_error_json(&e.message(),StatusCode::INTERNAL_SERVER_ERROR))?
            .filter(|p|p.post_type==resource)
            .ok_or_else(||editor_error_json("Post not found",StatusCode::NOT_FOUND))?;
        if !may_edit_post(state,&actor,id).await {return Err(ghost_api::permissions::denied())}
        Some(post)
    };
    let mut permission_input=json!({});
    if !form.authors.is_empty() {permission_input["authors"]=json!(form.authors.iter().map(|id|json!({"id":id})).collect::<Vec<_>>());}
    for (key,value) in [("codeinjection_head",&form.codeinjection_head),("codeinjection_foot",&form.codeinjection_foot)] {
        if let Some(value)=value {permission_input[key]=json!(value);}
    }
    ghost_api::permissions::post_input(state,&actor.user,&permission_input,existing.as_ref()).await?;
    let is_new=existing.is_none();
    let mut post=existing.unwrap_or_else(||Post {
        id:ghost_core::ids::new_id(),post_type:resource.into(),
        status:"draft".into(),visibility:"public".into(),created_at:Some(dates::now_db()),..Default::default()
    });
    post.title=Some(form.title.clone());
    post.slug=form.slug.clone().or(post.slug);
    if form.has_content() {
        let (html,plain,reading_time)=form.rendered();
        post.html=Some(html);post.plaintext=Some(plain);post.reading_time=reading_time;
        post.lexical=form.lexical.clone();post.markdown=form.markdown.clone();
    }
    post.custom_excerpt=form.custom_excerpt.clone();
    post.feature_image=form.feature_image.clone();
    post.featured=form.featured;
    post.show_title_and_feature_image=form.show_title_and_feature_image;
    post.visibility=form.visibility.clone().unwrap_or(post.visibility);
    post.published_at=form.published_at.clone().or(post.published_at);
    post.custom_template=form.custom_template.clone();post.canonical_url=form.canonical_url.clone();
    if actor.role.can_edit_any_post() {
        post.codeinjection_head=form.codeinjection_head.clone();post.codeinjection_foot=form.codeinjection_foot.clone();
    }
    if form.tiers_present {post.tier_ids=form.tiers.clone();}
    if form.audience_present {post.newsletter_id=form.audience.newsletter_id.clone();}
    post.meta=Some(PostMeta {
        post_id:post.id.clone(),meta_title:form.meta_title.clone(),meta_description:form.meta_description.clone(),
        og_title:form.og_title.clone(),og_description:form.og_description.clone(),og_image:form.og_image.clone(),
        twitter_title:form.twitter_title.clone(),twitter_description:form.twitter_description.clone(),twitter_image:form.twitter_image.clone(),
        email_subject:form.email_subject.clone(),feature_image_alt:form.feature_image_alt.clone(),feature_image_caption:form.feature_image_caption.clone(),
        ..post.meta.unwrap_or_default()
    });
    let author_ids=if !form.authors.is_empty() {form.authors.clone()}
        else if is_new {vec![actor.user.id.clone()]}
        else {sqlx::query_scalar("SELECT author_id FROM posts_authors WHERE post_id=? ORDER BY sort_order")
            .bind(id).fetch_all(&state.db).await.map_err(|e|editor_error_json(&e.to_string(),StatusCode::INTERNAL_SERVER_ERROR))?};
    let mut authors=Vec::new();
    for id in author_ids {
        if let Some(author)=repos::users::find_by_id(&state.db,&id).await.ok().flatten() {authors.push(author);}
        else {return Err(editor_error_json("Author not found",StatusCode::UNPROCESSABLE_ENTITY))}
    }
    let mut tags=Vec::new();
    for id in &form.tags {
        if let Some(tag)=repos::tags::find_by_id(&state.db,id).await.ok().flatten() {tags.push(tag);}
        else {return Err(editor_error_json("Tag not found",StatusCode::UNPROCESSABLE_ENTITY))}
    }
    let mut value=ghost_core::models::post_to_json(&post,&authors,&tags,"");
    let resolver=ghost_front::resolver_now(state).await;
    value["url"]=json!(resolver(&value));
    ghost_render::util::resolve_nested_urls(&mut value,&*resolver);
    Ok((post,value))
}

fn resource(uri: &OriginalUri) -> &'static str {
    if uri.0.path().contains("/pages/") {"page"} else {"post"}
}

pub(super) async fn render(State(state):State<AppState>,Path(id):Path<String>,uri:OriginalUri,headers:HeaderMap,form:EditorForm)->Response {
    let (post,value)=match snapshot(&state,&headers,&id,resource(&uri),&form).await {Ok(v)=>v,Err(r)=>return r};
    let mut data=ghost_api::publishing::preview_data(&state,&post,&value).await;
    // The rendered preview is the unsaved editor state; `/p/<uuid>/` serves
    // what is stored. Those are different things, so the link is only offered
    // for a post that exists — and the modal labels it as the saved version
    // rather than letting it read as a link to what is on screen.
    if id=="new" {
        data["web_url"]=Value::Null;
    }
    match ghost_front::preview_snapshot(&state,&post,value).await {
        Ok(mut html)=>{
            let base=format!("<base href=\"{}/\">",esc(state.site_url().trim_end_matches('/')));
            if let Some(at)=html.find("<head>") {html.insert_str(at+6,&base);} else {html.insert_str(0,&base);}
            data["web_html"]=json!(html);
            ([(header::CACHE_CONTROL,"private, no-store")],Json(json!({"preview":data}))).into_response()
        },Err(e)=>editor_error_json(&e.message(),StatusCode::INTERNAL_SERVER_ERROR)
    }
}

pub(super) async fn send_test(State(state):State<AppState>,Path(id):Path<String>,uri:OriginalUri,headers:HeaderMap,form:EditorForm)->Response {
    if let Err(r)=require_role(&state,&headers,Role::Author).await {return r}
    let email=form.preview_email.as_deref().unwrap_or("");
    if !email.contains('@') || email.contains(['\r','\n']) || email.len()>191 {
        return editor_error_json("Enter an email address",StatusCode::UNPROCESSABLE_ENTITY)
    }
    let (post,value)=match snapshot(&state,&headers,&id,resource(&uri),&form).await {Ok(v)=>v,Err(r)=>return r};
    let html=match ghost_api::publishing::email_preview_with_value(&state,&post,&value).await {
        Ok(html)=>html,Err(e)=>return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)
    };
    let config=ghost_mail::MailConfig::from_settings(&state.settings,&state.settings.site_name(),&state.site_url());
    let message=ghost_mail::Message {
        to:email.into(),subject:format!("[Test] {}",post.meta.as_ref().and_then(|m|m.email_subject.as_deref()).or(post.title.as_deref()).unwrap_or("Newsletter")),
        text:ghost_mail::newsletter::plaintext(&html),html
    };
    match ghost_mail::send(&config,&message).await {
        Ok(())=>StatusCode::NO_CONTENT.into_response(),Err(e)=>editor_error_json(&e.message(),StatusCode::INTERNAL_SERVER_ERROR)
    }
}
