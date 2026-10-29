//! Behavioural tests for the Ghost Handlebars helpers.
//!
//! Each case is checked against what ghost/core/core/frontend/helpers/<name>.js
//! produces for the same input, so a regression here means a theme would
//! render differently from the real Ghost.

use handlebars::Handlebars;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use ghost_render::helpers::*;

/// A registry with the helpers that need no database or theme state.
fn registry() -> Handlebars<'static> {
    let mut hb = Handlebars::new();
    hb.set_prevent_indent(true);
    hb.register_helper("blockHelperMissing", Box::new(BlockHelperMissing));
    hb.register_helper("helperMissing", Box::new(HelperMissing));
    hb.register_helper("foreach", Box::new(Foreach));
    hb.register_helper("is", Box::new(Is));
    hb.register_helper("has", Box::new(Has));
    hb.register_helper("match", Box::new(Match));
    hb.register_helper("content", Box::new(Content));
    hb.register_helper("excerpt", Box::new(Excerpt));
    hb.register_helper("title", Box::new(Title));
    hb.register_helper("url", Box::new(UrlHelper));
    hb.register_helper("img_url", Box::new(ImgUrl));
    hb.register_helper("date", Box::new(DateHelper));
    hb.register_helper("tags", Box::new(TagsList { is_authors: false }));
    hb.register_helper("authors", Box::new(TagsList { is_authors: true }));
    hb.register_helper("navigation", Box::new(Navigation));
    hb.register_helper("pagination", Box::new(Pagination));
    hb.register_helper("social_accounts", Box::new(SocialAccounts));
    hb.register_helper("social_url", Box::new(SocialUrl { fixed_platform: None }));
    hb.register_helper("twitter_url", Box::new(SocialUrl { fixed_platform: Some("twitter") }));
    hb.register_helper("link", Box::new(LinkHelper));
    hb.register_helper("link_class", Box::new(LinkClass));
    hb.register_helper("readable_url", Box::new(ReadableUrl));
    hb.register_helper("body_class", Box::new(BodyClass));
    hb.register_helper("post_class", Box::new(PostClass));
    hb.register_helper("concat", Box::new(Concat));
    hb.register_helper("length", Box::new(Length));
    hb.register_helper("split", Box::new(SplitHelper));
    hb.register_helper("encode", Box::new(Encode));
    hb.register_helper("json", Box::new(JsonHelper));
    hb.register_helper("plural", Box::new(Plural));
    hb.register_helper("page_url", Box::new(PageUrl));
    hb.register_helper("reading_time", Box::new(ReadingTime));
    hb.register_helper("color_to_rgba", Box::new(ColorToRgba));
    hb.register_helper("contrast_text_color", Box::new(ContrastTextColor));
    hb.register_helper("raw", Box::new(RawHelper));
    hb.register_helper("log", Box::new(LogHelper));
    hb.register_helper("search", Box::new(SearchHelper));
    hb.register_helper("comment_count", Box::new(CommentCount));
    hb.register_helper("comments", Box::new(CommentsHelper));
    hb.register_helper("tiers", Box::new(TiersHelper));
    hb.register_helper("price", Box::new(PriceHelper));
    hb.register_helper("total_members", Box::new(TotalMembers { paid: false }));
    hb.register_helper("meta_title", Box::new(MetaTitle));
    hb.register_helper("meta_description", Box::new(MetaDescription));
    hb.register_helper("ghost_head", Box::new(GhostHead));
    hb.register_helper("ghost_foot", Box::new(GhostFoot));
    hb.register_helper("t", Box::new(Translate { translations: Arc::new(HashMap::new()) }));
    hb.register_helper(
        "asset",
        Box::new(AssetHelper {
            hashes: Arc::new(HashMap::from([("css/screen.css".to_string(), "abcdef1234567890".to_string())])),
            site_origin: "https://example.com".into(),
            global_hash: "globalhash".into(),
        }),
    );
    hb
}

/// Render `template` against `data`, applying the same preprocessing a theme
/// template goes through.
fn render(template: &str, data: &Value) -> String {
    let mut hb = registry();
    let prepared = ghost_render::preprocess::prepare(template);
    hb.register_template_string("t", &prepared).expect("compile");
    hb.render("t", data).unwrap_or_else(|e| panic!("render {template:?}: {e}"))
}

fn check(cases: &[(&str, &Value, &str)]) {
    for (template, data, want) in cases {
        assert_eq!(&render(template, data), want, "template: {template}");
    }
}

/// A post as the theme layer sees it, with the keys `checks.isPost` looks for.
fn post() -> Value {
    json!({
        "id": "p1",
        // `comment_id` is how Ghost's `{{comments}}` tells a post from a
        // page, so the fixture carries it like the real serializer does.
        "comment_id": "p1-uuid",
        "access": true,
        "title": "Hello & <World>",
        "slug": "hello",
        "html": "<p>One two three four five six.</p>",
        "excerpt": "One two three four five six.",
        "custom_excerpt": null,
        "feature_image": "/content/images/2024/01/pic.jpg",
        "featured": true,
        "published_at": "2024-03-05T09:30:00.000Z",
        "reading_time": 4,
        "visibility": "public",
        "url": "/hello/",
        "tags": [
            {"name": "News", "slug": "news", "url": "/tag/news/", "visibility": "public"},
            {"name": "Hidden", "slug": "hidden", "url": "/tag/hidden/", "visibility": "internal"}
        ],
        "primary_tag": {"name": "News", "slug": "news", "url": "/tag/news/"},
        "authors": [
            {"name": "Pat", "slug": "pat", "url": "/author/pat/", "visibility": "public"},
            {"name": "Sam", "slug": "sam", "url": "/author/sam/", "visibility": "public"}
        ]
    })
}

fn root_with_post() -> Value {
    json!({
        "post": post(),
        "context": ["post"],
        "relativeUrl": "/hello/",
        "site": {"title": "My Site", "url": "https://example.com", "timezone": "Etc/UTC"},
        "config": {"image_sizes": [
            {"name": "s", "width": 300},
            {"name": "m", "width": 720, "height": 400}
        ]}
    })
}

// ---------------------------------------------------------------------------
// foreach
// ---------------------------------------------------------------------------

#[test]
fn foreach_exposes_loop_variables() {
    let data = json!({"items": [{"n": "a"}, {"n": "b"}, {"n": "c"}]});
    check(&[(
        "{{#foreach items}}{{@index}}{{n}}{{#if @first}}F{{/if}}{{#if @last}}L{{/if}}|{{/foreach}}",
        &data,
        "0aF|1b|2cL|",
    )]);
}

#[test]
fn foreach_limit_from_and_to_slice_the_collection() {
    let data = json!({"items": [1, 2, 3, 4, 5]});
    check(&[
        ("{{#foreach items limit=2}}{{this}}{{/foreach}}", &data, "12"),
        ("{{#foreach items from=3}}{{this}}{{/foreach}}", &data, "345"),
        ("{{#foreach items to=2}}{{this}}{{/foreach}}", &data, "12"),
        ("{{#foreach items from=2 to=4}}{{this}}{{/foreach}}", &data, "234"),
        // a limit that narrows the collection derives `to` from `from`
        ("{{#foreach items from=2 limit=2}}{{this}}{{/foreach}}", &data, "23"),
        // string-valued options are what themes actually write
        (r#"{{#foreach items limit="2"}}{{this}}{{/foreach}}"#, &data, "12"),
    ]);
}

#[test]
fn foreach_index_is_absolute_and_first_follows_from() {
    let data = json!({"items": [1, 2, 3, 4, 5]});
    // @index keeps counting from the start of the collection, and @first
    // marks the first item actually rendered
    check(&[(
        "{{#foreach items from=3}}{{@index}}{{#if @first}}F{{/if}}|{{/foreach}}",
        &data,
        "2F|3|4|",
    )]);
}

#[test]
fn foreach_filters_by_visibility() {
    // non-posts default to public only
    let tags = json!({"items": [
        {"name": "a", "visibility": "public"},
        {"name": "b", "visibility": "internal"}
    ]});
    check(&[
        ("{{#foreach items}}{{name}}{{/foreach}}", &tags, "a"),
        (r#"{{#foreach items visibility="all"}}{{name}}{{/foreach}}"#, &tags, "ab"),
        (r#"{{#foreach items visibility="internal"}}{{name}}{{/foreach}}"#, &tags, "b"),
    ]);

    // a collection of posts defaults to "all"
    let posts = json!({"items": [
        {"html": "", "title": "a", "slug": "a", "visibility": "public"},
        {"html": "", "title": "b", "slug": "b", "visibility": "paid"}
    ]});
    check(&[("{{#foreach items}}{{title}}{{/foreach}}", &posts, "ab")]);
}

#[test]
fn foreach_columns_drive_row_markers() {
    let data = json!({"items": [1, 2, 3, 4]});
    check(&[
        (
            "{{#foreach items columns=2}}{{#if @rowStart}}[{{/if}}{{this}}{{#if @rowEnd}}]{{/if}}{{/foreach}}",
            &data,
            "[12][34]",
        ),
        // without columns= neither marker is ever set
        (
            "{{#foreach items}}{{#if @rowStart}}S{{/if}}{{#if @rowEnd}}E{{/if}}{{this}}{{/foreach}}",
            &data,
            "1234",
        ),
    ]);
}

#[test]
fn foreach_renders_else_when_empty() {
    check(&[
        ("{{#foreach items}}x{{else}}none{{/foreach}}", &json!({"items": []}), "none"),
        ("{{#foreach items}}x{{else}}none{{/foreach}}", &json!({}), "none"),
    ]);
}

#[test]
fn foreach_iterates_object_values_with_keys() {
    let data = json!({"items": {"a": {"n": 1}, "b": {"n": 2}}});
    check(&[("{{#foreach items}}{{@key}}={{n}} {{/foreach}}", &data, "a=1 b=2 ")]);
}

#[test]
fn foreach_block_params_bind_value_then_key() {
    let data = json!({"items": [{"n": "a"}, {"n": "b"}]});
    check(&[(
        "{{#foreach items as |item idx|}}{{idx}}:{{item.n}} {{/foreach}}",
        &data,
        "0:a 1:b ",
    )]);
}

// ---------------------------------------------------------------------------
// is / has / match
// ---------------------------------------------------------------------------

#[test]
fn is_accepts_a_comma_separated_context_list() {
    let data = json!({"context": ["index", "paged"]});
    check(&[
        (r#"{{#is "index"}}y{{else}}n{{/is}}"#, &data, "y"),
        (r#"{{#is "post, paged"}}y{{else}}n{{/is}}"#, &data, "y"),
        (r#"{{#is "post"}}y{{else}}n{{/is}}"#, &data, "n"),
        (r#"{{#is "post,page"}}y{{else}}n{{/is}}"#, &data, "n"),
    ]);
}

#[test]
fn has_matches_tags_authors_and_counts() {
    let data = post();
    check(&[
        (r#"{{#has tag="News"}}y{{else}}n{{/has}}"#, &data, "y"),
        // tag matching is case-insensitive and comma-separated
        (r#"{{#has tag="news, other"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has tag="other"}}y{{else}}n{{/has}}"#, &data, "n"),
        (r#"{{#has tag="count:2"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has tag="count:>1"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has tag="count:>5"}}y{{else}}n{{/has}}"#, &data, "n"),
        (r#"{{#has tag="count:<5"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has author="Pat"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has author="count:2"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has slug="hello"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has visibility="public"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has visibility="paid"}}y{{else}}n{{/has}}"#, &data, "n"),
        // an unknown attribute never matches
        (r#"{{#has bogus="x"}}y{{else}}n{{/has}}"#, &data, ""),
    ]);
}

#[test]
fn has_number_and_index_use_the_loop_position() {
    let data = json!({"items": [1, 2, 3, 4, 5, 6]});
    // nth:3 means "every third", not "the third"
    check(&[
        (
            r#"{{#foreach items}}{{#has number="nth:3"}}[{{this}}]{{/has}}{{/foreach}}"#,
            &data,
            "[3][6]",
        ),
        (
            r#"{{#foreach items}}{{#has index="0,2"}}[{{this}}]{{/has}}{{/foreach}}"#,
            &data,
            "[1][3]",
        ),
    ]);
}

#[test]
fn has_any_and_all_test_property_presence() {
    let data = json!({
        "title": "set", "excerpt": "", "tags": [{"slug": "x"}],
        "site": {"logo": "/logo.png"}
    });
    check(&[
        (r#"{{#has any="title, excerpt"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has all="title, excerpt"}}y{{else}}n{{/has}}"#, &data, "n"),
        (r#"{{#has all="title, tags"}}y{{else}}n{{/has}}"#, &data, "y"),
        (r#"{{#has any="missing"}}y{{else}}n{{/has}}"#, &data, "n"),
        // @-prefixed names read the render root
        (r#"{{#has any="@site.logo"}}y{{else}}n{{/has}}"#, &data, "y"),
    ]);
}

#[test]
fn match_supports_one_two_and_three_arguments() {
    let data = json!({"style": "Landing", "count": 4, "flag": true, "zero": 0, "empty": ""});
    check(&[
        // one argument: truthiness, like {{#if}}
        (r#"{{#match flag}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match empty}}y{{else}}n{{/match}}"#, &data, "n"),
        (r#"{{#match zero}}y{{else}}n{{/match}}"#, &data, "n"),
        (r#"{{#match zero includeZero=true}}y{{else}}n{{/match}}"#, &data, "y"),
        // two arguments: equality
        (r#"{{#match style "Landing"}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match style "Other"}}y{{else}}n{{/match}}"#, &data, "n"),
        // equality is case sensitive, as === is in Ghost
        (r#"{{#match style "landing"}}y{{else}}n{{/match}}"#, &data, "n"),
        // three arguments: explicit operator
        (r#"{{#match style "!=" "Off"}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match count ">" 2}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match count ">=" 4}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match count "<" 2}}y{{else}}n{{/match}}"#, &data, "n"),
        (r#"{{#match style "~" "and"}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match style "~^" "Land"}}y{{else}}n{{/match}}"#, &data, "y"),
        (r#"{{#match style "~$" "ing"}}y{{else}}n{{/match}}"#, &data, "y"),
        // inline use renders the outcome
        (r#"{{match style "Landing"}}"#, &data, "true"),
    ]);
}

#[test]
fn match_chains_through_else() {
    let data = json!({"style": "Magazine"});
    check(&[(
        r#"{{#match style "Highlight"}}H{{else match style "Magazine"}}M{{else}}other{{/match}}"#,
        &data,
        "M",
    )]);
}

// ---------------------------------------------------------------------------
// content / excerpt
// ---------------------------------------------------------------------------

#[test]
fn content_truncates_without_breaking_markup() {
    let data = json!({"html": "<p>One two three four five</p>"});
    check(&[
        ("{{content}}", &data, "<p>One two three four five</p>"),
        // the cut closes the tags it left open, and appends nothing
        ("{{content words=2}}", &data, "<p>One two</p>"),
        ("{{content words=0}}", &data, ""),
        ("{{content characters=3}}", &data, "<p>One</p>"),
    ]);
}

#[test]
fn content_counts_only_text_towards_the_budget() {
    let data = json!({"html": "<div><p><em>One</em> two</p><p>three</p></div>"});
    check(&[("{{content words=2}}", &data, "<div><p><em>One</em> two</p></div>")]);
}

#[test]
fn excerpt_escapes_and_defaults_to_fifty_words() {
    let long: String = (1..=60).map(|i| format!("w{i} ")).collect();
    let data = json!({"excerpt": long.trim()});
    let rendered = render("{{excerpt}}", &data);
    assert_eq!(rendered.split_whitespace().count(), 50);
    assert!(rendered.starts_with("w1 w2"));
    assert!(rendered.ends_with("w50"));

    check(&[
        ("{{excerpt words=2}}", &json!({"excerpt": "one two three"}), "one two"),
        ("{{excerpt characters=3}}", &json!({"excerpt": "abcdef"}), "abc"),
        // HTML in the excerpt is escaped, never rendered
        ("{{excerpt}}", &json!({"excerpt": "a <b> & c"}), "a &lt;b&gt; &amp; c"),
    ]);
}

#[test]
fn excerpt_prefers_a_custom_excerpt_whole() {
    // a custom excerpt is shown in full, ignoring words=
    let data = json!({"custom_excerpt": "one two three four", "excerpt": "ignored"});
    check(&[
        ("{{excerpt}}", &data, "one two three four"),
        ("{{excerpt words=2}}", &data, "one two three four"),
    ]);
}

// ---------------------------------------------------------------------------
// title / url / img_url
// ---------------------------------------------------------------------------

#[test]
fn title_is_escaped() {
    check(&[("{{title}}", &post(), "Hello &amp; &lt;World&gt;")]);
}

#[test]
fn url_renders_relative_and_absolute_forms() {
    let data = json!({"url": "/hello/", "site": {"url": "https://example.com"}});
    check(&[
        ("{{url}}", &data, "/hello/"),
        (r#"{{url absolute="true"}}"#, &data, "https://example.com/hello/"),
        // absolute="false" is off, not merely present
        (r#"{{url absolute="false"}}"#, &data, "/hello/"),
    ]);
}

#[test]
fn img_url_builds_ghosts_size_paths() {
    let data = root_with_post();
    check(&[
        ("{{img_url post.feature_image}}", &data, "/content/images/2024/01/pic.jpg"),
        // width-only and width+height sizes
        (
            r#"{{img_url post.feature_image size="s"}}"#,
            &data,
            "/content/images/size/w300/2024/01/pic.jpg",
        ),
        (
            r#"{{img_url post.feature_image size="m"}}"#,
            &data,
            "/content/images/size/w720h400/2024/01/pic.jpg",
        ),
        (
            r#"{{img_url post.feature_image size="m" format="webp"}}"#,
            &data,
            "/content/images/size/w720h400/format/webp/2024/01/pic.jpg",
        ),
        // an unknown size is left alone
        (r#"{{img_url post.feature_image size="xl"}}"#, &data, "/content/images/2024/01/pic.jpg"),
        (
            r#"{{img_url post.feature_image size="s" absolute="true"}}"#,
            &data,
            "https://example.com/content/images/size/w300/2024/01/pic.jpg",
        ),
        // an unset image renders nothing rather than "null"
        ("{{img_url post.missing_image}}", &data, ""),
    ]);
}

#[test]
fn img_url_passes_external_images_through() {
    let data = json!({"site": {"url": "https://example.com"}, "config": {"image_sizes": [{"name": "s", "width": 300}]}});
    check(&[
        (
            r#"{{img_url "https://cdn.other.com/a.png" size="s"}}"#,
            &data,
            "https://cdn.other.com/a.png",
        ),
        // Unsplash can resize, so the size becomes a query parameter
        (
            r#"{{img_url "https://images.unsplash.com/photo-1?ixlib=rb-1.2.1" size="s"}}"#,
            &data,
            "https://images.unsplash.com/photo-1?ixlib=rb-1.2.1&w=300",
        ),
    ]);
}

// ---------------------------------------------------------------------------
// date
// ---------------------------------------------------------------------------

#[test]
fn date_formats_with_moment_tokens() {
    let data = json!({
        "published_at": "2024-03-05T09:30:00.000Z",
        "site": {"timezone": "Etc/UTC"}
    });
    check(&[
        // no argument uses published_at from the current scope
        ("{{date}}", &data, "Mar 5, 2024"),
        (r#"{{date format="YYYY-MM-DD"}}"#, &data, "2024-03-05"),
        (r#"{{date format="DD MMM YYYY"}}"#, &data, "05 Mar 2024"),
        (r#"{{date format="D MMMM YYYY"}}"#, &data, "5 March 2024"),
        (r#"{{date published_at format="dddd"}}"#, &data, "Tuesday"),
        (r#"{{date published_at format="ddd"}}"#, &data, "Tue"),
        (r#"{{date published_at format="Do"}}"#, &data, "5th"),
        (r#"{{date published_at format="h:mm A"}}"#, &data, "9:30 AM"),
        (r#"{{date published_at format="HH:mm"}}"#, &data, "09:30"),
        // localized formats
        (r#"{{date published_at format="LL"}}"#, &data, "March 5, 2024"),
        (r#"{{date published_at format="ll"}}"#, &data, "Mar 5, 2024"),
        (r#"{{date published_at format="L"}}"#, &data, "03/05/2024"),
        // bracketed text is literal
        (r#"{{date published_at format="[on] D MMM"}}"#, &data, "on 5 Mar"),
    ]);
}

#[test]
fn date_honours_the_timezone() {
    let data = json!({"published_at": "2024-03-05T23:30:00.000Z"});
    check(&[
        (
            r#"{{date published_at format="YYYY-MM-DD HH:mm" timezone="Asia/Tokyo"}}"#,
            &data,
            "2024-03-06 08:30",
        ),
        (
            r#"{{date published_at format="YYYY-MM-DD HH:mm" timezone="Etc/UTC"}}"#,
            &data,
            "2024-03-05 23:30",
        ),
    ]);
}

// ---------------------------------------------------------------------------
// tags / authors
// ---------------------------------------------------------------------------

#[test]
fn tags_renders_links_and_respects_visibility() {
    let data = post();
    check(&[
        // internal tags are hidden by default
        ("{{tags}}", &data, r#"<a href="/tag/news/">News</a>"#),
        (r#"{{tags autolink="false"}}"#, &data, "News"),
        (
            r#"{{tags visibility="all" autolink="false" separator=" - "}}"#,
            &data,
            "News - Hidden",
        ),
        (
            r#"{{tags autolink="false" prefix="on " suffix="!"}}"#,
            &data,
            "on News!",
        ),
        (r#"{{tags visibility="all" autolink="false" limit=1}}"#, &data, "News"),
        (r#"{{tags visibility="all" autolink="false" from=2}}"#, &data, "Hidden"),
    ]);
}

#[test]
fn authors_joins_names() {
    let data = post();
    check(&[
        (r#"{{authors autolink="false"}}"#, &data, "Pat, Sam"),
        (r#"{{authors autolink="false" separator=" and "}}"#, &data, "Pat and Sam"),
        (r#"{{authors autolink="false" limit=1}}"#, &data, "Pat"),
        ("{{authors}}", &data, r#"<a href="/author/pat/">Pat</a>, <a href="/author/sam/">Sam</a>"#),
    ]);
}

// ---------------------------------------------------------------------------
// classes
// ---------------------------------------------------------------------------

#[test]
fn body_class_follows_the_context() {
    check(&[
        (
            "{{body_class}}",
            &json!({"context": ["home", "index"], "post": null}),
            "home-template",
        ),
        ("{{body_class}}", &json!({"context": ["post"], "post": post()}), "post-template tag-news tag-hidden"),
        (
            "{{body_class}}",
            &json!({"context": ["page"], "page": {"slug": "about", "tags": []}}),
            "page-template page-about",
        ),
        (
            "{{body_class}}",
            &json!({"context": ["tag"], "tag": {"slug": "news"}}),
            "tag-template tag-news",
        ),
        (
            "{{body_class}}",
            &json!({"context": ["author"], "author": {"slug": "pat"}}),
            "author-template author-pat",
        ),
        (
            "{{body_class}}",
            &json!({"context": ["index", "paged"]}),
            "paged",
        ),
    ]);
}

#[test]
fn post_class_describes_the_post() {
    check(&[
        ("{{post_class}}", &post(), "post tag-news tag-hidden featured"),
        (
            "{{post_class}}",
            &json!({"tags": [], "featured": false, "feature_image": null}),
            "post no-image",
        ),
        // also works where the post sits under `post`
        ("{{post_class}}", &json!({"post": post()}), "post tag-news tag-hidden featured"),
    ]);
}

// ---------------------------------------------------------------------------
// utility helpers
// ---------------------------------------------------------------------------

#[test]
fn plural_substitutes_the_count() {
    let tpl = r#"{{plural n empty="No posts" singular="% post" plural="% posts"}}"#;
    check(&[
        (tpl, &json!({"n": 0}), "No posts"),
        (tpl, &json!({"n": 1}), "1 post"),
        (tpl, &json!({"n": 7}), "7 posts"),
    ]);
    // all three strings are required
    check(&[(r#"{{plural n singular="x"}}"#, &json!({"n": 1}), "")]);
}

#[test]
fn encode_uses_uri_component_rules() {
    check(&[
        (r#"{{encode "a b&c"}}"#, &json!({}), "a%20b%26c"),
        (r#"{{encode "already-safe_1.0"}}"#, &json!({}), "already-safe_1.0"),
        (r#"{{encode "héllo"}}"#, &json!({}), "h%C3%A9llo"),
    ]);
}

#[test]
fn json_is_safe_to_embed_in_a_script_tag() {
    check(&[
        // the characters that could close a <script> early are unicode-escaped
        (
            "{{json data}}",
            &json!({"data": {"a": "<b>&</b>"}}),
            r#"{"a":"\u003Cb\u003E\u0026\u003C/b\u003E"}"#,
        ),
        (r#"{{json a=1 b="x"}}"#, &json!({}), r#"{"a":1,"b":"x"}"#),
    ]);
}

#[test]
fn concat_joins_and_flattens() {
    check(&[
        (r#"{{concat "icons/" type}}"#, &json!({"type": "rss"}), "icons/rss"),
        (r#"{{concat a b separator="-"}}"#, &json!({"a": "x", "b": "y"}), "x-y"),
        (r#"{{concat list separator=", "}}"#, &json!({"list": ["a", "b"]}), "a, b"),
    ]);
}

#[test]
fn split_yields_an_iterable() {
    let data = json!({"s": "a,b,c"});
    check(&[
        ("{{#split s}}[{{this}}]{{/split}}", &data, "[a][b][c]"),
        (r#"{{#split s separator=","}}[{{this}}]{{/split}}"#, &data, "[a][b][c]"),
        (r#"{{#split "a|b" separator="|"}}[{{this}}]{{/split}}"#, &json!({}), "[a][b]"),
        // empty segments are dropped
        (r#"{{#split "a,,b"}}[{{this}}]{{/split}}"#, &json!({}), "[a][b]"),
        // composes into foreach as a subexpression
        ("{{#foreach (split s)}}[{{this}}]{{/foreach}}", &data, "[a][b][c]"),
    ]);
}

#[test]
fn reading_time_labels_are_overridable() {
    let data = post();
    check(&[
        ("{{reading_time}}", &data, "4 min read"),
        (r#"{{reading_time minutes="% min"}}"#, &data, "4 min"),
    ]);
    let one = json!({"html": "", "title": "t", "slug": "s", "reading_time": 1});
    check(&[
        ("{{reading_time}}", &one, "1 min read"),
        (r#"{{reading_time minute="één minuut"}}"#, &one, "één minuut"),
    ]);
    // nothing at all outside a post
    check(&[("{{reading_time}}", &json!({"reading_time": 3}), "")]);
}

#[test]
fn page_url_walks_the_current_channel() {
    let index = json!({"relativeUrl": "/", "pagination": {"page": 2, "pages": 5, "next": 3, "prev": 1}});
    check(&[
        (r#"{{page_url "next"}}"#, &index, "/page/3/"),
        (r#"{{page_url "prev"}}"#, &index, "/"),
        ("{{page_url 4}}", &index, "/page/4/"),
        ("{{page_url 1}}", &index, "/"),
    ]);
    // a tag archive keeps its base path
    let tag = json!({"relativeUrl": "/tag/news/page/2/", "pagination": {"page": 2, "pages": 5, "next": 3, "prev": 1}});
    check(&[
        (r#"{{page_url "next"}}"#, &tag, "/tag/news/page/3/"),
        (r#"{{page_url "prev"}}"#, &tag, "/tag/news/"),
    ]);
}

#[test]
fn readable_url_strips_scheme_www_and_query() {
    check(&[
        (r#"{{readable_url "https://www.google.com/x/?q=1#f"}}"#, &json!({}), "google.com/x"),
        (r#"{{readable_url "https://ghost.org/"}}"#, &json!({}), "ghost.org"),
        // a non-URL comes back unchanged
        (r#"{{readable_url "not a url"}}"#, &json!({}), "not a url"),
    ]);
}

#[test]
fn colour_helpers_match_ghost() {
    check(&[
        (r##"{{color_to_rgba "#15171A" 0.25}}"##, &json!({}), "rgba(21, 23, 26, 0.25)"),
        (r##"{{color_to_rgba "#15171A" 1}}"##, &json!({}), "rgb(21, 23, 26)"),
        (r##"{{contrast_text_color "#ffffff"}}"##, &json!({}), "#000000"),
        (r##"{{contrast_text_color "#15171A"}}"##, &json!({}), "#FFFFFF"),
    ]);
}

#[test]
fn raw_emits_its_body() {
    check(&[("{{#raw}}<p>kept</p>{{/raw}}", &json!({}), "<p>kept</p>")]);
}

#[test]
fn log_writes_nothing_to_the_page() {
    check(&[("a{{log title}}b", &post(), "ab")]);
}

// ---------------------------------------------------------------------------
// social
// ---------------------------------------------------------------------------

#[test]
fn social_url_builds_profile_links() {
    let data = json!({"site": {"twitter": "@ghost", "facebook": "ghostorg", "linkedin": "ghost"}});
    check(&[
        (r#"{{social_url type="twitter"}}"#, &data, "https://x.com/ghost"),
        (r#"{{social_url type="facebook"}}"#, &data, "https://www.facebook.com/ghostorg"),
        (r#"{{social_url type="linkedin"}}"#, &data, "https://www.linkedin.com/in/ghost"),
        // an unconfigured platform renders nothing
        (r#"{{social_url type="tiktok"}}"#, &data, ""),
        // no type at all is a no-op, not an error
        ("{{social_url}}", &data, ""),
        // the deprecated alias still works
        ("{{twitter_url}}", &data, "https://x.com/ghost"),
    ]);
}

#[test]
fn social_url_prefers_the_current_scope() {
    // an author's own handle wins over the site's
    let data = json!({
        "twitter": "author_handle",
        "site": {"twitter": "site_handle"}
    });
    check(&[(r#"{{social_url type="twitter"}}"#, &data, "https://x.com/author_handle")]);
}

#[test]
fn social_accounts_iterates_in_canonical_order() {
    let data = json!({"site": {
        "twitter": "@ghost", "facebook": "ghostorg", "instagram": "ghost", "youtube": "@ghostcms"
    }});
    check(&[(
        "{{#social_accounts @site}}{{type}}={{href}} {{/social_accounts}}",
        &data,
        "x=https://x.com/ghost facebook=https://www.facebook.com/ghostorg \
         youtube=https://www.youtube.com/@ghostcms instagram=https://www.instagram.com/ghost ",
    )]);
    // the else branch covers a site with nothing configured
    check(&[(
        "{{#social_accounts @site}}x{{else}}none{{/social_accounts}}",
        &json!({"site": {"title": "t"}}),
        "none",
    )]);
}

// ---------------------------------------------------------------------------
// link / link_class / navigation / pagination
// ---------------------------------------------------------------------------

#[test]
fn link_class_marks_the_current_page() {
    let data = json!({"relativeUrl": "/about/", "site": {"url": "https://example.com"}});
    check(&[
        (r#"{{link_class for="/about/"}}"#, &data, "nav-current"),
        (r#"{{link_class for="/other/"}}"#, &data, ""),
        (r#"{{link_class for="/about/" class="nav-item"}}"#, &data, "nav-item nav-current"),
        (
            r#"{{link_class for="/about/" activeClass="active"}}"#,
            &data,
            "active",
        ),
        // an absolute href is compared after the site URL is stripped
        (r#"{{link_class for="https://example.com/about/"}}"#, &data, "nav-current"),
    ]);
}

#[test]
fn link_builds_an_anchor() {
    let data = json!({"relativeUrl": "/about/", "site": {"url": "https://example.com"}});
    check(&[
        (
            r#"{{#link href="/about/"}}About{{/link}}"#,
            &data,
            r#"<a class="nav-current" href="/about/">About</a>"#,
        ),
        (
            r#"{{#link href="/other/" title="Go"}}Other{{/link}}"#,
            &data,
            r#"<a href="/other/" title="Go">Other</a>"#,
        ),
    ]);
}

#[test]
fn navigation_marks_the_current_item() {
    let mut hb = registry();
    hb.register_partial(
        "navigation",
        r#"{{#foreach navigation}}{{slug}}:{{#if current}}1{{else}}0{{/if}} {{/foreach}}"#,
    )
    .unwrap();
    hb.register_template_string("t", "{{navigation}}").unwrap();
    let data = json!({
        "relativeUrl": "/about/",
        "site": {"navigation": [
            {"label": "Home", "url": "/"},
            {"label": "About Us", "url": "/about/"}
        ]}
    });
    assert_eq!(hb.render("t", &data).unwrap(), "home:0 about-us:1 ");
}

#[test]
fn navigation_type_secondary_reads_the_other_menu() {
    let mut hb = registry();
    hb.register_partial("navigation", "{{#foreach navigation}}{{label}} {{/foreach}}").unwrap();
    hb.register_template_string("t", r#"{{navigation type="secondary"}}"#).unwrap();
    let data = json!({
        "relativeUrl": "/",
        "site": {
            "navigation": [{"label": "Primary", "url": "/"}],
            "secondary_navigation": [{"label": "Secondary", "url": "/s/"}]
        }
    });
    assert_eq!(hb.render("t", &data).unwrap(), "Secondary ");
}

#[test]
fn pagination_exposes_its_fields_at_the_top_level() {
    let mut hb = registry();
    hb.register_partial("pagination", "{{page}}/{{pages}} next:{{next}} prev:{{prev}}").unwrap();
    hb.register_template_string("t", "{{pagination}}").unwrap();
    let data = json!({
        "relativeUrl": "/page/2/",
        "pagination": {"page": 2, "pages": 5, "total": 42, "limit": 10, "next": 3, "prev": 1}
    });
    assert_eq!(hb.render("t", &data).unwrap(), "2/5 next:3 prev:1");
}

// ---------------------------------------------------------------------------
// members-era helpers
// ---------------------------------------------------------------------------

#[test]
fn tiers_lists_names_with_a_final_separator() {
    check(&[
        ("{{tiers}}", &json!({"tiers": [{"name": "Gold"}]}), "Gold tier"),
        (
            "{{tiers}}",
            &json!({"tiers": [{"name": "Gold"}, {"name": "Silver"}]}),
            "Gold and Silver tiers",
        ),
        (
            r#"{{tiers separator=", " lastSeparator=" or " suffix=""}}"#,
            &json!({"tiers": [{"name": "A"}, {"name": "B"}, {"name": "C"}]}),
            "A, B or C",
        ),
        ("{{tiers}}", &json!({}), ""),
    ]);
}

#[test]
fn total_members_is_rounded() {
    check(&[
        ("{{total_members}}", &json!({"site": {"total_members": 42}}), "42"),
        ("{{total_members}}", &json!({"site": {"total_members": 1250}}), "1.2k"),
        ("{{total_members}}", &json!({"site": {"total_members": 24500}}), "24k"),
    ]);
}

#[test]
fn search_and_comment_count_emit_their_mount_points() {
    assert!(render("{{search}}", &json!({})).contains("data-ghost-search"));
    let cc = render("{{comment_count}}", &post());
    assert!(cc.contains(r#"data-ghost-comment-count="p1""#), "{cc}");
    assert!(cc.contains(r#"data-ghost-comment-count-singular="comment""#), "{cc}");
}

/// The root a post page renders with comments available.
fn commentable() -> Value {
    json!({
        "post": post(),
        "site": {
            "url": "https://site.test",
            "title": "My Site",
            "locale": "en",
            "accent_color": "#FF1A75",
            "comments_enabled": "all",
        },
        "frontendKey": "abc123",
    })
}

/// `{{comments}}` emits the script tag the CDN widget needs, with the data
/// attributes it reads. Checked against Ghost 6.73's `comments` helper.
#[test]
fn comments_emits_the_widget_script_tag() {
    let out = render("{{comments}}", &commentable());
    // Built from the pinned version rather than spelling it out, so tracking
    // a new Ghost release is a one-line change in `apps.rs` instead of a
    // failing test that says nothing about the thing being tested.
    assert!(
        out.contains(&format!(
            r#"<script defer src="https://cdn.jsdelivr.net/ghost/comments-ui@~{}/umd/comments-ui.min.js""#,
            ghost_render::apps::COMMENTS_VERSION
        )),
        "{out}"
    );
    assert!(out.contains(r#"crossorigin="anonymous""#), "{out}");
    for attr in [
        r#"data-locale="en""#,
        r#"data-ghost-comments="https://site.test/""#,
        r#"data-api="https://site.test/ghost/api/content/""#,
        r#"data-admin="https://site.test/ghost/""#,
        r#"data-key="abc123""#,
        // the literal string "null" — the widget tests for it to pick its
        // own default heading
        r#"data-title="null""#,
        r#"data-count="true""#,
        r#"data-post-id="p1""#,
        r#"data-color-scheme="auto""#,
        r#"data-avatar-saturation="60""#,
        r##"data-accent-color="#FF1A75""##,
        r#"data-comments-enabled="all""#,
        r#"data-publication="My Site""#,
    ] {
        assert!(out.contains(attr), "missing {attr} in {out}");
    }
}

#[test]
fn comments_honours_its_options() {
    let root = commentable();
    let out = render(r#"{{comments mode="dark" saturation=40 count=false title="Discussion"}}"#, &root);
    assert!(out.contains(r#"data-color-scheme="dark""#), "{out}");
    assert!(out.contains(r#"data-avatar-saturation="40""#), "{out}");
    assert!(out.contains(r#"data-count="false""#), "{out}");
    assert!(out.contains(r#"data-title="Discussion""#), "{out}");
    // an unrecognised mode falls back to auto rather than passing through
    assert!(render(r#"{{comments mode="sepia"}}"#, &root).contains(r#"data-color-scheme="auto""#));
    // and so does a saturation that is not a number
    assert!(render(r#"{{comments saturation="x"}}"#, &root).contains(r#"data-avatar-saturation="60""#));
}

#[test]
fn comments_stays_silent_when_it_should() {
    let blank = |root: &Value| assert_eq!(render("{{comments}}", root), "", "{root}");

    // comments switched off site-wide
    let mut off = commentable();
    off["site"]["comments_enabled"] = json!("off");
    blank(&off);

    // a page, which has no comment_id
    let mut page = commentable();
    page["post"]["comment_id"] = Value::Null;
    blank(&page);

    // a post the visitor cannot read
    let mut gated = commentable();
    gated["post"]["access"] = json!(false);
    blank(&gated);

    // no Content API key, so every request the widget made would 401
    let mut keyless = commentable();
    keyless["frontendKey"] = json!("");
    blank(&keyless);
}

/// The publication title and accent colour reach an attribute value, so they
/// are escaped — Ghost interpolates them raw.
#[test]
fn comments_escapes_its_attribute_values() {
    let mut root = commentable();
    root["site"]["title"] = json!(r#"Pat" onload="x"#);
    let out = render("{{comments}}", &root);
    assert!(!out.contains(r#"onload="x""#), "{out}");
    assert!(out.contains("data-publication=\"Pat&quot; onload=&quot;x\""), "{out}");
}

/// The counts script goes into `ghost_head` whenever comments are on, since
/// an index page is where `{{comment_count}}` placeholders live.
#[test]
fn ghost_head_links_the_comment_counts_script() {
    let out = render("{{ghost_head}}", &commentable());
    assert!(out.contains(r#"src="/public/comment-counts.min.js""#), "{out}");
    assert!(
        out.contains(r#"data-ghost-comments-counts-api="https://site.test/members/api/comments/counts/""#),
        "{out}"
    );

    let mut off = commentable();
    off["site"]["comments_enabled"] = json!("off");
    assert!(!render("{{ghost_head}}", &off).contains("comment-counts.min.js"));
}

/// The announcement bar goes out as Ghost's widget, not as markup, because
/// the audience is per-member and a dismissal has to outlive the page.
#[test]
fn ghost_head_links_the_announcement_bar_widget() {
    let mut data = commentable();
    data["site"]["announcement_content"] = json!("<p>We moved</p>");
    data["site"]["announcement_visibility"] = json!(["visitors", "paid_members"]);

    let out = render("{{ghost_head}}", &data);
    assert!(
        out.contains(&format!(
            r#"<script defer src="https://cdn.jsdelivr.net/ghost/announcement-bar@~{}/umd/announcement-bar.min.js""#,
            ghost_render::apps::ANNOUNCEMENT_BAR_VERSION
        )),
        "{out}"
    );
    assert!(out.contains(r#"data-announcement-bar="https://site.test""#), "{out}");
    assert!(
        out.contains(r#"data-api-url="https://site.test/members/api/announcement/""#),
        "{out}"
    );
    // The content itself is never in the page — that is the point, since the
    // page is cacheable and the answer is not.
    assert!(!out.contains("We moved"), "{out}");

    // Nothing to announce, or nobody to announce it to, means no tag at all.
    let mut nobody = data.clone();
    nobody["site"]["announcement_visibility"] = json!([]);
    assert!(!render("{{ghost_head}}", &nobody).contains("announcement-bar"), "visibility []");

    let mut empty = data.clone();
    empty["site"]["announcement_content"] = json!("");
    assert!(!render("{{ghost_head}}", &empty).contains("announcement-bar"), "empty content");

    // And it is no longer in the foot, where this port used to render it.
    assert!(!render("{{ghost_foot}}", &data).contains("announcement"), "foot");
}

/// Without this link nothing can send us a webmention: a sender discovers the
/// endpoint from the page it is linking to.
#[test]
fn ghost_head_advertises_the_webmention_endpoint() {
    let out = render("{{ghost_head}}", &root_with_post());
    assert!(
        out.contains(r#"<link href="https://example.com/webmentions/receive/" rel="webmention">"#),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// meta / head
// ---------------------------------------------------------------------------

#[test]
fn meta_title_and_description_follow_the_context() {
    let home = json!({
        "context": ["home"],
        "site": {"title": "My Site", "description": "A blog"}
    });
    check(&[
        ("{{meta_title}}", &home, "My Site - A blog"),
        ("{{meta_description}}", &home, "A blog"),
    ]);

    let post_ctx = json!({
        "context": ["post"],
        "title": "A Post",
        "excerpt": "Some words",
        "site": {"title": "My Site"}
    });
    check(&[
        ("{{meta_title}}", &post_ctx, "A Post - My Site"),
        ("{{meta_description}}", &post_ctx, "Some words"),
    ]);
}

#[test]
fn ghost_head_emits_the_essential_tags() {
    let head = render("{{ghost_head}}", &root_with_post());
    for expected in [
        "<title>",
        r#"<link rel="canonical""#,
        r#"<meta name="generator""#,
        r#"<meta property="og:title""#,
        r#"<meta name="twitter:card""#,
        r#"type="application/rss+xml""#,
    ] {
        assert!(head.contains(expected), "ghost_head missing {expected}:\n{head}");
    }
}

#[test]
fn ghost_foot_emits_code_injection() {
    check(&[(
        "{{ghost_foot}}",
        &json!({"site": {"codeinjection_foot": "<script>a()</script>"}}),
        "<script>a()</script>",
    )]);
}

// ---------------------------------------------------------------------------
// handlebars.js semantics handlebars-rust does not provide
// ---------------------------------------------------------------------------

#[test]
fn bare_block_on_a_value_uses_it_as_context() {
    check(&[
        (
            "{{#post}}{{title}}{{/post}}",
            &json!({"post": {"title": "Hi"}}),
            "Hi",
        ),
        ("{{#post}}x{{else}}none{{/post}}", &json!({"post": null}), "none"),
    ]);
}

#[test]
fn block_and_content_for_pass_content_up() {
    let mut hb = registry();
    let blocks: BlockStore = Arc::new(std::sync::Mutex::new(HashMap::new()));
    hb.register_helper("contentFor", Box::new(ContentFor { blocks: blocks.clone() }));
    hb.register_helper("block", Box::new(BlockHelper { blocks }));
    hb.register_template_string(
        "t",
        r#"{{#contentFor "scripts"}}<script>a</script>{{/contentFor}}[{{block "scripts"}}]"#,
    )
    .unwrap();
    assert_eq!(hb.render("t", &json!({})).unwrap(), "[<script>a</script>]");
}

#[test]
fn asset_tolerates_a_leading_slash_the_way_ghost_does() {
    // Themes write it both ways. Ghost strips the slash; without that,
    // `/assets//built/main.min.js` 404s and the theme's script never runs.
    check(&[
        (r#"{{asset "built/main.min.js"}}"#, &json!({}), "/assets/built/main.min.js?v=globalhash"),
        (r#"{{asset "/built/main.min.js"}}"#, &json!({}), "/assets/built/main.min.js?v=globalhash"),
        // a path with a file of its own is busted with that file's hash
        (r#"{{asset "css/screen.css"}}"#, &json!({}), "/assets/css/screen.css?v=abcdef1234"),
        (r#"{{asset "/css/screen.css"}}"#, &json!({}), "/assets/css/screen.css?v=abcdef1234"),
        // Ghost-owned paths stay at the site root rather than under /assets/
        (r#"{{asset "public/cards.min.css"}}"#, &json!({}), "/public/cards.min.css?v=globalhash"),
        (r#"{{asset "/public/cards.min.css"}}"#, &json!({}), "/public/cards.min.css?v=globalhash"),
    ]);
}

#[test]
fn ghost_head_links_the_koenig_card_assets() {
    // The editor can put a gallery or a toggle in any post, so the card
    // styles ship with the platform rather than with the theme.
    let data = json!({"config": {"card_assets": true, "card_assets_version": "abc123"}, "site": {}});
    let head = render("{{ghost_head}}", &data);
    assert!(head.contains(r#"<link rel="stylesheet" type="text/css" href="/public/cards.min.css?v=abc123">"#), "{head}");
    assert!(head.contains(r#"<script defer src="/public/cards.min.js?v=abc123"></script>"#), "{head}");
}

#[test]
fn a_theme_can_turn_the_card_assets_off() {
    let data = json!({"config": {"card_assets": false}, "site": {}});
    let head = render("{{ghost_head}}", &data);
    assert!(!head.contains("cards.min"), "{head}");
}

#[test]
fn card_assets_default_to_on_when_the_theme_says_nothing() {
    let head = render("{{ghost_head}}", &json!({"config": {}, "site": {}}));
    assert!(head.contains("cards.min.css"), "{head}");
}

/// Themes guard the section with `{{#if comments}}` — Source does exactly
/// that — and handlebars.js answers it by *calling* the helper, since a
/// registered helper wins an ambiguous reference. handlebars-rust only looks
/// up the context, so the renderer seeds a root-level `comments` flag.
///
/// This pins both halves of that arrangement: the flag makes the guard pass,
/// and the helper still wins in helper position so `{{comments}}` renders
/// the script tag rather than printing the flag.
#[test]
fn the_comments_flag_answers_the_themes_guard() {
    let mut root = commentable();
    root["comments"] = json!(true);
    assert_eq!(render("{{#if comments}}YES{{else}}NO{{/if}}", &root), "YES");
    assert!(render("{{comments}}", &root).contains("comments-ui.min.js"));

    root["comments"] = json!(false);
    assert_eq!(render("{{#if comments}}YES{{else}}NO{{/if}}", &root), "NO");
}

// ---------------------------------------------------------------------------
// handlebars.js semantics a theme from outside the official set relies on
// ---------------------------------------------------------------------------

/// A block opened on a list iterates it, the way handlebars.js routes a block
/// opened on an array through `each`. Rendering it once with the array as the
/// context instead would silently produce one garbled item.
///
/// The name has to be one no helper claims — a registered helper wins an
/// ambiguous reference, which is why `{{#tags}}` reaches the tags helper
/// rather than this path.
#[test]
fn a_block_opened_on_an_array_iterates_it() {
    let data = json!({"items": [{"name": "Design"}, {"name": "Tutorials"}]});
    assert_eq!(render("{{#items}}[{{name}}]{{/items}}", &data), "[Design][Tutorials]");
    assert_eq!(render("{{#items}}{{@index}}{{/items}}", &data), "01");
    // an empty list is falsy and takes the inverse
    assert_eq!(render("{{#items}}x{{else}}none{{/items}}", &json!({"items": []})), "none");
}

/// A block opened on an object enters it; one opened on a missing path takes
/// the inverse.
#[test]
fn a_block_opened_on_a_value_enters_or_inverts() {
    let data = json!({"post": {"title": "T"}});
    assert_eq!(render("{{#post}}{{title}}{{/post}}", &data), "T");
    assert_eq!(render("{{#missing}}x{{else}}none{{/missing}}", &data), "none");
}

/// `{{#../primary_tag}}` — Lyra and Fizzy reach past a loop like this.
/// handlebars-rust will not parse a path as a block name, so the preprocessor
/// turns it into an explicit `blockHelperMissing` call.
#[test]
fn a_block_can_be_opened_on_a_parent_scope_path() {
    let data = json!({"primary_tag": {"name": "Design"}, "posts": [{"title": "A"}]});
    assert_eq!(
        render("{{#foreach posts}}{{#../primary_tag}}{{name}}{{/../primary_tag}}{{/foreach}}", &data),
        "Design"
    );
}

/// `{{^helper}}…{{else}}…{{/helper}}` inverts the test and nothing else, so
/// the branches have to swap when it is rewritten to a `#` block. Journal and
/// Wave both write this shape in their layout.
#[test]
fn an_inverted_block_with_an_else_picks_the_right_branch() {
    let paged = json!({"context": ["index", "paged"]});
    let not_paged = json!({"context": ["index"]});
    assert_eq!(render(r#"{{^is "paged"}}A{{else}}B{{/is}}"#, &paged), "B");
    assert_eq!(render(r#"{{^is "paged"}}A{{else}}B{{/is}}"#, &not_paged), "A");
    // and without an else the body is still the negative branch
    assert_eq!(render(r#"{{^is "paged"}}A{{/is}}"#, &not_paged), "A");
    assert_eq!(render(r#"{{^is "paged"}}A{{/is}}"#, &paged), "");
}

/// `posts.length` is a JavaScript array member that JSON has no equivalent
/// for; Headline and Taste open their home page with one.
#[test]
fn length_reads_as_a_count() {
    let data = json!({"posts": [1, 2, 3], "context": ["index"]});
    assert_eq!(render("{{posts.length}}", &data), "3");
    assert_eq!(render(r#"{{#match posts.length ">" 1}}many{{/match}}"#, &data), "many");
    assert_eq!(render(r#"{{#match posts.length ">" 5}}many{{/match}}"#, &data), "");
    // a collection that is not there counts as empty rather than failing
    assert_eq!(render("{{missing.length}}", &data), "0");
}

/// `{{meta_title}}` is called from `default.hbs`, where `this` is the layout's
/// root rather than the post — it has to resolve the routed resource the same
/// way `{{ghost_head}}` does, or every post page gets the bare site name.
#[test]
fn meta_title_resolves_the_routed_post_from_the_layout() {
    let data = json!({
        "site": {"title": "Site"},
        "context": ["post"],
        "post": {"id": "1", "title": "A Post"},
    });
    assert_eq!(render("{{meta_title}}", &data), "A Post - Site");

    // the page suffix only applies once past page one
    let mut paged = data.clone();
    paged["context"] = json!(["index", "paged"]);
    paged["pagination"] = json!({"page": 2});
    assert_eq!(render(r#"{{meta_title page=" (Page %)"}}"#, &paged), "A Post - Site (Page 2)");
    assert_eq!(render(r#"{{meta_title page=" (Page %)"}}"#, &data), "A Post - Site");
}

/// `{{price}}` goes through `Intl.NumberFormat` in Ghost, which groups
/// thousands — a yearly plan printed without grouping reads as `$1200`.
/// Ported from core/frontend/helpers/price.js.
#[test]
fn price_formats_the_way_intl_does() {
    let data = json!({});
    // minor units, and a whole amount drops its decimals
    assert_eq!(render(r#"{{price 2100 currency="USD"}}"#, &data), "$21");
    assert_eq!(render(r#"{{price 2150 currency="USD"}}"#, &data), "$21.50");
    assert_eq!(render(r#"{{price 120000 currency="USD"}}"#, &data), "$1,200");
    assert_eq!(render(r#"{{price 123456789 currency="EUR"}}"#, &data), "€1,234,567.89");
    // numberFormat="long" keeps the decimals on a whole amount
    assert_eq!(render(r#"{{price 2100 currency="USD" numberFormat="long"}}"#, &data), "$21.00");
    // currencyFormat swaps the symbol for the code or the name
    assert_eq!(render(r#"{{price 2100 currency="USD" currencyFormat="code"}}"#, &data), "USD 21");
    assert_eq!(render(r#"{{price 2100 currency="USD" currencyFormat="name"}}"#, &data), "US dollars 21");
    // no amount at all is how a theme asks for just the unit
    assert_eq!(render(r#"{{price currency="USD"}}"#, &data), "$");
    // a tier carries its own amount and currency
    let tier = json!({"tier": {"amount": 500, "currency": "GBP"}});
    assert_eq!(render("{{price tier}}", &tier), "£5");
}

#[test]
fn feature_image_caption_renders_html_in_theme_contexts() {
    let data = json!({
        "feature_image_caption": "Photo by <a href=\"https://unsplash.com/@author\">Author &amp; Co</a>",
        "title": "<b>Title</b>",
        "post": {"feature_image_caption": "<em>Credit</em>"},
        "posts": [{"feature_image_caption": "<strong>Nested credit</strong>"}]
    });
    assert_eq!(
        render("{{#if feature_image_caption}}<figcaption>{{feature_image_caption}}</figcaption>{{/if}}", &data),
        "<figcaption>Photo by <a href=\"https://unsplash.com/@author\">Author &amp; Co</a></figcaption>"
    );
    assert_eq!(render("{{post.feature_image_caption}}", &data), "<em>Credit</em>");
    assert_eq!(render("{{#foreach posts}}{{feature_image_caption}} / {{../feature_image_caption}}{{/foreach}}", &data),
        "<strong>Nested credit</strong> / Photo by <a href=\"https://unsplash.com/@author\">Author &amp; Co</a>");
    assert_eq!(render("{{{feature_image_caption}}}", &data), data["feature_image_caption"].as_str().unwrap());
    assert_eq!(render("{{title}}", &data), "&lt;b&gt;Title&lt;/b&gt;");
    assert_eq!(render("{{! feature_image_caption}}", &data), "");
}

#[test]
fn remote_storage_images_use_local_originals_for_responsive_variants(){
    let mut data=root_with_post();
    data["post"]["feature_image"]=json!("https://assets.example.org/images/2026/10/remote.avif");
    data["config"]["storage_image_prefix"]=json!("https://assets.example.org/images/");
    check(&[
        ("{{img_url post.feature_image}}",&data,"https://assets.example.org/images/2026/10/remote.avif"),
        (r#"{{img_url post.feature_image size="s" format="webp" absolute="true"}}"#,&data,"https://example.com/content/images/size/w300/format/webp/2026/10/remote.avif"),
    ]);
}

#[test]
fn transistor_content_uses_member_and_publication_identity_at_render_time(){
    let mut data=root_with_post();
    let lexical=r#"{"root":{"children":[{"type":"transistor","version":1}]}}"#;
    data["html"]=json!(ghost_render::lexical::to_html(lexical));
    data["member"]=json!({"uuid":"member-123"});data["site"]["site_uuid"]=json!("publication-456");
    let html=registry().render_template("{{content}}",&data).unwrap();
    assert!(html.contains("/ghost/embed/member-123?ctx=publication%2D456"),"{html}");
    assert!(!html.contains("%7Buuid%7D"));
}
