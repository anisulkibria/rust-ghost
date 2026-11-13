//! The Media module end to end: upload a file through the real router,
//! describe it, rename it, replace it, find out what uses it, and delete it.
//!
//! The checks that matter most here are the negative ones — who is refused,
//! and what happens to a path that tries to name something outside the content
//! directory. Those come first in the file for that reason.
//!
//! The library reads the filesystem, so each test gets a content directory of
//! its own under the system temp dir and puts real files in it.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";
const HOSTILE: &str = r#"<img src=x onerror=alert(1)>"><script>alert(2)</script>"#;

struct Fixture {
    _db: ghost_db::TestDb,
    state: ghost_api::AppState,
    owner: String,
    content: PathBuf,
}

impl Fixture {
    /// Put a file in the content directory, as an earlier upload or an SFTP
    /// copy would have left it. Returns its stored path.
    fn put(&self, rel: &str, bytes: &[u8]) -> String {
        let full = self.content.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
        rel.to_string()
    }

    fn exists(&self, rel: &str) -> bool {
        self.content.join(rel).is_file()
    }

    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.content.join(rel)).unwrap()
    }

    /// Every file under the content directory, as stored paths.
    fn all_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![self.content.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    out.push(
                        path.strip_prefix(&self.content).unwrap().to_string_lossy().replace('\\', "/"),
                    );
                }
            }
        }
        out.sort();
        out
    }

    async fn session(&self) -> String {
        let id = ghost_db::repos::sessions::create(&self.state.db, &self.owner).await.unwrap();
        format!("ghost-admin-api-session={id}")
    }
}

async fn fixture() -> Fixture {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(ghost_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Media test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let content = std::env::temp_dir().join(format!("ghost-media-test-{}", ghost_core::ids::new_id()));
    std::fs::create_dir_all(&content).unwrap();
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": content.to_string_lossy()}
    }))
    .unwrap();
    let routes = ghost_routing::routes_yaml::parse_routes_yaml(ghost_routing::routes_yaml::DEFAULT_ROUTES_YAML).unwrap();
    let router = Arc::new(ghost_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
    let deps = ghost_render::HelperDeps {
        pool: Arc::new(db.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: ghost_api::themes::url_resolver(router.clone()),
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: "test".into(),
        theme_dir: theme_dir.clone(),
        site_origin: ORIGIN.into(),
        content_api_key: String::new(),
    };
    let theme = ghost_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
    let state = ghost_api::AppState {
        db,
        config: Arc::new(config),
        settings,
        theme: Arc::new(tokio::sync::RwLock::new(Arc::new(theme))),
        router: Arc::new(tokio::sync::RwLock::new(router)),
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(ghost_routing::redirects::Redirects::default()))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(ghost_db::limits::Limits::default()),
    };
    Fixture { _db: test_db, state, owner: owner.id, content }
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

impl Reply {
    /// The message a redirect put in its query string, decoded.
    fn flash(&self) -> String {
        let Some(query) = self.location.split_once('?').map(|(_, q)| q) else { return String::new() };
        form_urlencoded::parse(query.as_bytes())
            .filter(|(key, _)| key == "ok" || key == "error")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn redirected_ok(&self) -> bool {
        self.location.contains("ok=")
    }
}

/// One request to the admin, as a browser on the admin's own pages sends it
/// unless `origin` says otherwise.
async fn send(
    state: &ghost_api::AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    body: Option<(&str, Vec<u8>)>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let payload = match body {
        Some((content_type, bytes)) => {
            request = request.header("content-type", content_type);
            Body::from(bytes)
        }
        None => Body::empty(),
    };
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(payload).unwrap()).await.unwrap();
    let status = response.status();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(state: &ghost_api::AppState, uri: &str, cookie: &str) -> Reply {
    send(state, "GET", uri, Some(cookie), None, None).await
}

async fn post(state: &ghost_api::AppState, uri: &str, cookie: &str, pairs: &[(&str, &str)]) -> Reply {
    let form = encode(pairs);
    send(
        state,
        "POST",
        uri,
        Some(cookie),
        Some(ORIGIN),
        Some(("application/x-www-form-urlencoded", form.into_bytes())),
    )
    .await
}

fn encode(pairs: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

/// A multipart body: text fields, then files as `(field, filename, type, bytes)`.
fn multipart(fields: &[(&str, &str)], files: &[(&str, &str, &str, &[u8])]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----ghostmediatestboundary";
    let mut body: Vec<u8> = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes(),
        );
    }
    for (name, filename, content_type, bytes) in files {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

async fn upload(
    state: &ghost_api::AppState,
    cookie: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> Reply {
    let (ct, body) = multipart(&[], &[("file", filename, content_type, bytes)]);
    send(state, "POST", "/media/upload/", Some(cookie), Some(ORIGIN), Some((&ct, body))).await
}

/// The `path` of the one file in the library, read off the grid.
fn only_path(body: &str) -> String {
    let marker = r#"name="paths" value=""#;
    let from = body.find(marker).expect("a tile in the grid") + marker.len();
    let len = body[from..].find('"').unwrap();
    body[from..from + len].to_string()
}

fn item_uri(path: &str) -> String {
    format!("/media/item/?{}", encode(&[("path", path)]))
}

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_library_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let f = fixture().await;
    f.put("images/2026/11/secret-ab12.jpg", b"a draft's cover image");

    let secret = "images/2026/11/secret-ab12.jpg";
    // Each request is well-formed for its route, so what refuses it is the
    // guard and not a rejected body — a 400 from a malformed multipart would
    // prove nothing about who is allowed in.
    let form = || {
        let pairs = encode(&[("path", secret), ("paths", secret), ("name", "x")]);
        Some(("application/x-www-form-urlencoded".to_string(), pairs.into_bytes()))
    };
    let files = || {
        let (ct, body) = multipart(&[("path", secret)], &[("file", "x.png", "image/png", b"bytes")]);
        Some((ct, body))
    };

    for (method, uri, body) in [
        ("GET", "/media/".to_string(), None),
        ("GET", format!("/media/item/?{}", encode(&[("path", secret)])), None),
        ("POST", "/media/delete/".to_string(), form()),
        ("POST", "/media/item/save/".to_string(), form()),
        ("POST", "/media/item/rename/".to_string(), form()),
        ("POST", "/media/item/delete/".to_string(), form()),
        ("POST", "/media/upload/".to_string(), files()),
        ("POST", "/media/item/replace/".to_string(), files()),
    ] {
        // no cookie at all, and a cookie that names no session
        for cookie in [None, Some("ghost-admin-api-session=not-a-session")] {
            let payload = body.as_ref().map(|(ct, bytes)| (&ct[..], bytes.clone()));
            let r = send(&f.state, method, &uri, cookie, Some(ORIGIN), payload).await;
            assert!(
                (r.status.is_redirection() && r.location.contains("signin")) || r.status == StatusCode::UNAUTHORIZED,
                "{method} {uri} answered {} to a stranger",
                r.status
            );
            assert!(!r.body.contains("secret-ab12"), "{uri} named a file to a stranger");
            assert!(!r.body.contains("ml-grid"), "{uri} showed the library to a stranger");
        }
    }
    // Nothing was uploaded by any of those posts either.
    assert_eq!(f.all_files(), vec![secret.to_string()]);
    // and nothing was touched by any of it
    assert!(f.exists("images/2026/11/secret-ab12.jpg"));
}

#[tokio::test]
async fn a_request_from_another_site_cannot_change_anything() {
    let f = fixture().await;
    let cookie = f.session().await;
    let path = f.put("images/2026/11/cover-ab12.jpg", b"the original bytes");

    for (uri, pairs) in [
        ("/media/item/save/", vec![("path", path.as_str()), ("title", "Stolen")]),
        ("/media/item/rename/", vec![("path", path.as_str()), ("name", "stolen")]),
        ("/media/item/delete/", vec![("path", path.as_str())]),
        ("/media/delete/", vec![("paths", path.as_str())]),
    ] {
        let form = encode(&pairs);
        let r = send(
            &f.state,
            "POST",
            uri,
            Some(&cookie),
            Some("https://evil.test"),
            Some(("application/x-www-form-urlencoded", form.into_bytes())),
        )
        .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{uri} accepted a cross-site post");
    }
    // the upload and replace routes too, which arrive as multipart
    for uri in ["/media/upload/", "/media/item/replace/"] {
        let (ct, body) = multipart(&[("path", &path)], &[("file", "x.png", "image/png", b"new bytes")]);
        let r = send(&f.state, "POST", uri, Some(&cookie), Some("https://evil.test"), Some((&ct, body))).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{uri} accepted a cross-site upload");
    }

    assert_eq!(f.read(&path), b"the original bytes");
    assert_eq!(f.all_files(), vec![path]);
}

#[tokio::test]
async fn a_path_that_names_anything_outside_the_library_is_refused() {
    let f = fixture().await;
    let cookie = f.session().await;
    // Two things a traversal would be reaching for: a theme's files, and the
    // database. Both are under the content directory and neither is media.
    std::fs::create_dir_all(f.content.join("themes/casper")).unwrap();
    std::fs::write(f.content.join("themes/casper/package.json"), b"{}").unwrap();
    std::fs::create_dir_all(f.content.join("data")).unwrap();
    std::fs::write(f.content.join("data/ghost.db"), b"the database").unwrap();
    let before = f.all_files();

    let hostile = [
        "images/../../themes/casper/package.json",
        "images/../data/ghost.db",
        "themes/casper/package.json",
        "data/ghost.db",
        "/etc/passwd",
        "images/..",
        "images/./../data/ghost.db",
        "images\\..\\data\\ghost.db",
        "",
        "images",
    ];

    for path in hostile {
        // Opening one is a plain not-found page, never a file's screen.
        let r = get(&f.state, &item_uri(path), &cookie).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "the item screen opened {path:?}");
        assert!(!r.body.contains("ml-preview"), "{path:?} rendered a preview");

        // Writing to one is refused and redirects to the library.
        for (uri, pairs) in [
            ("/media/item/save/", vec![("path", path), ("title", "x")]),
            ("/media/item/rename/", vec![("path", path), ("name", "x")]),
            ("/media/item/delete/", vec![("path", path)]),
            ("/media/delete/", vec![("paths", path)]),
        ] {
            let r = post(&f.state, uri, &cookie, &pairs).await;
            assert!(r.status.is_redirection(), "{uri} with {path:?} answered {}", r.status);
            assert!(r.location.contains("error="), "{uri} accepted {path:?}: {}", r.location);
        }

        let (ct, body) = multipart(&[("path", path)], &[("file", "x.png", "image/png", b"replacement")]);
        let r = send(&f.state, "POST", "/media/item/replace/", Some(&cookie), Some(ORIGIN), Some((&ct, body))).await;
        assert!(r.location.contains("error="), "replace accepted {path:?}: {}", r.location);
    }

    // Nothing was read, written, moved or unlinked.
    assert_eq!(f.all_files(), before);
    assert_eq!(f.read("data/ghost.db"), b"the database");
}

/// The library's own upload never refuses a file for its type — and this
/// states why that is the right answer rather than a gap.
///
/// Because the declared type is what *chooses* the family, that family's
/// allowlist can never disagree with it: a type the image or media family
/// would refuse was never routed there in the first place, it went to the open
/// `files` family. So the allowlist does its work in the two places where the
/// family is fixed by something other than the type —
/// `/ghost/api/admin/images/upload/`, where the editor's card names the family
/// in the URL, and Replace, where the file already on disk fixes it. Those are
/// covered by `a_replacement_has_to_be_the_same_kind_of_file` and by the unit
/// tests in `ghost_core::media`.
///
/// The property that actually protects this endpoint is a different one: the
/// stored *name* is composed by the server, so nothing an uploader sends can
/// decide where the bytes land. That is what the shape assertion below pins.
#[tokio::test]
async fn the_librarys_upload_files_an_unrecognised_type_away_rather_than_refusing_it() {
    let f = fixture().await;
    let cookie = f.session().await;

    // A shell script. No family claims `application/x-sh`, so it goes to the
    // open one — it is not, and could not be, filed as a picture.
    let r = upload(&f.state, &cookie, "photo.png", "application/x-sh", b"#!/bin/sh\nrm -rf /\n").await;
    assert!(r.redirected_ok(), "{}", r.flash());
    let stored = f.all_files();
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert!(stored[0].starts_with("files/"), "a script was filed as a picture: {}", stored[0]);

    // An empty form post says so rather than claiming success.
    let (ct, body) = multipart(&[], &[]);
    let r = send(&f.state, "POST", "/media/upload/", Some(&cookie), Some(ORIGIN), Some((&ct, body))).await;
    assert!(r.flash().contains("Choose a file"), "{}", r.flash());
    assert_eq!(f.all_files(), stored, "the empty post wrote something");
}

/// The `files` family takes anything, and this pins that down rather than
/// leaving it implicit.
///
/// It is the behaviour `/ghost/api/admin/files/upload/` has always had and the
/// behaviour Ghost's own file card has: a publication attaches spreadsheets,
/// archives and whatever else to its posts, and an allowlist of document types
/// would be a list nobody could finish. The protection is that only an
/// Administrator can upload at all, not a filter on what they may upload.
///
/// What *is* guaranteed, and what this test exists for, is that the server
/// composes the stored name — so an uploaded name cannot smuggle a path, a
/// second extension, or anything but a slug.
#[tokio::test]
async fn the_files_family_deliberately_takes_anything_under_a_name_this_server_composed() {
    let f = fixture().await;
    let cookie = f.session().await;

    let r = upload(&f.state, &cookie, "../../etc/pass wd.HTML", "text/html", b"<script>alert(1)</script>").await;
    assert!(r.redirected_ok(), "{}", r.flash());
    let stored = f.all_files();
    assert_eq!(stored.len(), 1, "{stored:?}");
    let path = &stored[0];

    // The shape is the guarantee: `files/YYYY/MM/<stem>-<8 hex>.<ext>`, and
    // exactly four segments. The separators in the uploaded name were slugged
    // *into* the stem rather than dropped — `../../etc/pass wd` becomes
    // `etc-pass-wd` — so there is no way for a name to add a directory, which
    // is what a traversal would need.
    let segments: Vec<&str> = path.split('/').collect();
    assert_eq!(segments.len(), 4, "the uploaded name changed the shape of the path: {path}");
    assert_eq!(segments[0], "files");
    assert!(segments[1].len() == 4 && segments[1].chars().all(char::is_numeric), "{path}");
    assert!(segments[2].len() == 2 && segments[2].chars().all(char::is_numeric), "{path}");
    let name = segments[3];
    // Slugged stem, lowercase extension, and a random tail so the next upload
    // of the same name does not overwrite this one.
    assert!(name.starts_with("etc-pass-wd-"), "the stem was not slugged: {name}");
    assert!(name.ends_with(".html"), "the extension was not lowercased: {name}");
    assert!(
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
        "the stored name holds something other than a slug: {name}"
    );

    // A type nothing recognises is accepted here too, rather than being sent
    // to a family that would turn it away — QuickTime is the real case.
    let r = upload(&f.state, &cookie, "holiday.mov", "video/quicktime", b"a home video").await;
    assert!(r.redirected_ok(), "a .mov was refused: {}", r.flash());
    assert!(f.all_files().iter().any(|p| p.starts_with("files/") && p.ends_with(".mov")), "{:?}", f.all_files());

    // …and the library still offers it a video player, which it decides from
    // the extension rather than from the directory it ended up in.
    let mov = f.all_files().into_iter().find(|p| p.ends_with(".mov")).unwrap();
    let item = get(&f.state, &item_uri(&mov), &cookie).await;
    assert!(item.body.contains("<video"), "a .mov in files/ got no player");
}

// ---------------------------------------------------------------------------
// the module working
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_file_is_uploaded_described_renamed_replaced_and_deleted() {
    let f = fixture().await;
    let cookie = f.session().await;

    // The module is reachable from the Modules hub.
    let modules = get(&f.state, "/modules/", &cookie).await;
    assert!(modules.body.contains(r#"href="/ghost/media/""#), "Media is not on the Modules page");

    // --- upload
    let r = upload(&f.state, &cookie, "Autumn Harbour.PNG", "image/png", b"the first version").await;
    assert!(r.redirected_ok(), "{}", r.location);
    assert!(r.flash().contains("Uploaded 1 file"), "{}", r.flash());

    let files = f.all_files();
    assert_eq!(files.len(), 1, "{files:?}");
    let path = files[0].clone();
    // Filed by what it is, named the way this server names things: the stem
    // slugged, the extension from the allowlist, a random tail.
    assert!(path.starts_with("images/"), "{path}");
    assert!(path.contains("autumn-harbour-"), "{path}");
    assert!(path.ends_with(".png"), "{path}");

    // --- it is in the library, and the library found it on disk
    let library = get(&f.state, "/media/", &cookie).await;
    assert!(library.body.contains(&format!(r#"name="paths" value="{path}""#)), "the upload is not in the grid");
    assert_eq!(only_path(&library.body), path);
    assert!(library.body.contains("1 file · 17 B on disk"), "the summary is wrong");

    // --- describe it
    let r = post(
        &f.state,
        "/media/item/save/",
        &cookie,
        &[("path", &path), ("title", "Autumn harbour"), ("alt", "Boats at dawn"), ("caption", "From the east pier")],
    )
    .await;
    assert!(r.redirected_ok(), "{}", r.location);
    let item = get(&f.state, &item_uri(&path), &cookie).await;
    for expected in ["Autumn harbour", "Boats at dawn", "From the east pier"] {
        assert!(item.body.contains(expected), "the item screen lost {expected:?}");
    }
    // The grid shows the title in place of the file name once there is one.
    let library = get(&f.state, "/media/", &cookie).await;
    assert!(library.body.contains("Autumn harbour"), "the title is not on the tile");

    // --- rename, metadata and all
    let r = post(&f.state, "/media/item/rename/", &cookie, &[("path", &path), ("name", "Harbour At Dawn")]).await;
    assert!(r.redirected_ok(), "{}", r.location);
    let renamed = f.all_files();
    assert_eq!(renamed.len(), 1, "{renamed:?}");
    let renamed = renamed[0].clone();
    assert!(renamed.ends_with("harbour-at-dawn.png"), "{renamed}");
    assert!(!f.exists(&path), "the old name is still there");
    let item = get(&f.state, &item_uri(&renamed), &cookie).await;
    assert!(item.body.contains("Boats at dawn"), "the description did not follow the rename");
    assert_eq!(f.read(&renamed), b"the first version");

    // --- replace the bytes, keeping the name
    let (ct, body) = multipart(
        &[("path", &renamed)],
        &[("file", "whatever-it-is-called.png", "image/png", b"the second version")],
    );
    let r = send(&f.state, "POST", "/media/item/replace/", Some(&cookie), Some(ORIGIN), Some((&ct, body))).await;
    assert!(r.redirected_ok(), "{}", r.location);
    assert_eq!(f.read(&renamed), b"the second version");
    assert_eq!(f.all_files(), vec![renamed.clone()], "replacing added a file instead of overwriting");

    // --- delete
    let r = post(&f.state, "/media/item/delete/", &cookie, &[("path", &renamed)]).await;
    assert!(r.redirected_ok(), "{}", r.location);
    assert!(f.all_files().is_empty(), "{:?} survived the delete", f.all_files());
    // …and what was written about it went with it
    let at = ghost_core::media::parse_location(&renamed).unwrap();
    assert_eq!(ghost_db::repos::media::find(&f.state.db, &at).await.unwrap(), None);
    // Deleting it again is answered, not crashed into.
    let again = post(&f.state, "/media/item/delete/", &cookie, &[("path", &renamed)]).await;
    assert!(again.location.contains("error="), "{}", again.location);
}

#[tokio::test]
async fn each_upload_is_filed_by_what_it_is_so_nobody_has_to_choose() {
    let f = fixture().await;
    let cookie = f.session().await;

    for (filename, content_type, expected) in [
        ("photo.jpg", "image/jpeg", "images/"),
        ("logo.svg", "image/svg+xml", "images/"),
        ("clip.mp4", "video/mp4", "media/"),
        ("theme.mp3", "audio/mpeg", "media/"),
        ("terms.pdf", "application/pdf", "files/"),
        // A type no family claims goes to the open one. Routing by the
        // filename instead would send this to the media family, which has no
        // entry for QuickTime and would refuse it outright.
        ("holiday.mov", "video/quicktime", "files/"),
        ("notes.txt", "application/octet-stream", "files/"),
    ] {
        let r = upload(&f.state, &cookie, filename, content_type, b"bytes").await;
        assert!(r.redirected_ok(), "{filename} was refused: {}", r.flash());
        assert!(
            f.all_files().iter().any(|p| p.starts_with(expected) && p.contains(filename.split('.').next().unwrap())),
            "{filename} did not land under {expected}: {:?}",
            f.all_files()
        );
    }

    // Several files in one post, which is what a drag-and-drop sends.
    let (ct, body) = multipart(
        &[],
        &[
            ("file", "one.png", "image/png", b"a"),
            ("file", "two.png", "image/png", b"b"),
            ("file", "three.png", "image/png", b"c"),
        ],
    );
    let r = send(&f.state, "POST", "/media/upload/", Some(&cookie), Some(ORIGIN), Some((&ct, body))).await;
    assert!(r.flash().contains("Uploaded 3 files"), "{}", r.flash());

    // And the library's tabs count each family.
    let library = get(&f.state, "/media/", &cookie).await;
    assert!(library.body.contains("Video &amp; audio"), "the media tab is missing");
    assert_eq!(library.body.matches(r#"name="paths" value=""#).count(), 10, "ten files should be on the grid");
    // 3 PNGs + a JPEG + an SVG under images, the mp4 and mp3 under media, and
    // the PDF, the .mov and the .txt under files.
    assert_eq!(get(&f.state, "/media/?kind=images", &cookie).await.body.matches(r#"name="paths" value=""#).count(), 5);
    assert_eq!(get(&f.state, "/media/?kind=media", &cookie).await.body.matches(r#"name="paths" value=""#).count(), 2);
    assert_eq!(get(&f.state, "/media/?kind=files", &cookie).await.body.matches(r#"name="paths" value=""#).count(), 3);
}

#[tokio::test]
async fn the_library_lists_what_is_already_on_disk_and_narrows_it_on_request() {
    let f = fixture().await;
    let cookie = f.session().await;
    // Files nothing uploaded through this admin — an SFTP copy, or a restored
    // backup. There is no row for any of them, and they are all in the
    // library anyway.
    f.put("images/2026/10/old-cover.jpg", b"october");
    f.put("images/2026/11/new-cover.jpg", b"november");
    f.put("media/2026/11/interview.mp3", b"an interview");
    f.put("files/2026/11/prospectus.pdf", b"a prospectus");
    // Bookkeeping the library should pass over rather than list.
    f.put("images/2026/11/.DS_Store", b"junk");
    f.put("images/.gitkeep", b"");

    let all = get(&f.state, "/media/", &cookie).await;
    assert_eq!(all.body.matches(r#"name="paths" value=""#).count(), 4, "the four real files should be listed");
    assert!(!all.body.contains("DS_Store") && !all.body.contains("gitkeep"), "dot-files were listed");
    assert!(all.body.contains("4 files"), "the summary should count four");

    // one family
    let images = get(&f.state, "/media/?kind=images", &cookie).await;
    assert_eq!(images.body.matches(r#"name="paths" value=""#).count(), 2);
    assert!(images.body.contains("old-cover.jpg") && images.body.contains("new-cover.jpg"));
    assert!(!images.body.contains("interview.mp3"));

    // one month
    let november = get(&f.state, "/media/?folder=2026%2F11", &cookie).await;
    assert_eq!(november.body.matches(r#"name="paths" value=""#).count(), 3);
    assert!(!november.body.contains("old-cover.jpg"));

    // a search, which looks at names
    let hunt = get(&f.state, "/media/?q=cover", &cookie).await;
    assert_eq!(hunt.body.matches(r#"name="paths" value=""#).count(), 2);

    // …and at the words written beside a file
    let at = ghost_core::media::parse_location("files/2026/11/prospectus.pdf").unwrap();
    ghost_db::repos::media::save(
        &f.state.db,
        &at,
        &ghost_db::repos::media::MetaInput { title: "", alt: "", caption: "Everything about the autumn term" },
    )
    .await
    .unwrap();
    let hunt = get(&f.state, "/media/?q=autumn+term", &cookie).await;
    assert_eq!(hunt.body.matches(r#"name="paths" value=""#).count(), 1);
    assert!(hunt.body.contains("prospectus.pdf"));

    // a search that matches nothing says so instead of showing an empty grid
    let nothing = get(&f.state, "/media/?q=nothinglikethis", &cookie).await;
    assert!(nothing.body.contains("Nothing matches that"), "no empty state");

    // an empty library says something different again
    let empty = fixture().await;
    let empty_cookie = empty.session().await;
    let r = get(&empty.state, "/media/", &empty_cookie).await;
    assert!(r.body.contains("Nothing uploaded yet"), "no first-run empty state");
    // …and still offers somewhere to drop a file
    assert!(r.body.contains("data-ml-drop"), "nowhere to drop a file");
}

#[tokio::test]
async fn a_rename_cannot_change_a_files_extension_or_overwrite_its_neighbour() {
    let f = fixture().await;
    let cookie = f.session().await;
    let path = f.put("images/2026/11/cover-ab12.png", b"a picture");
    let neighbour = f.put("images/2026/11/taken-cd34.png", b"someone else's picture");

    // The extension is carried over, so this cannot become a document served
    // from this site's own origin.
    let r = post(&f.state, "/media/item/rename/", &cookie, &[("path", &path), ("name", "payload.html")]).await;
    assert!(r.redirected_ok(), "{}", r.flash());
    assert!(f.exists("images/2026/11/payload-html.png"), "{:?}", f.all_files());
    assert!(!f.all_files().iter().any(|p| p.ends_with(".html")), "{:?}", f.all_files());

    // A rename cannot move a file to another directory or another family.
    let path = "images/2026/11/payload-html.png";
    let r = post(&f.state, "/media/item/rename/", &cookie, &[("path", path), ("name", "../../../escaped")]).await;
    assert!(r.redirected_ok(), "{}", r.flash());
    assert!(f.exists("images/2026/11/escaped.png"), "{:?}", f.all_files());

    // A name already taken is refused rather than overwriting what is there.
    let path = "images/2026/11/escaped.png";
    let r = post(&f.state, "/media/item/rename/", &cookie, &[("path", path), ("name", "taken-cd34")]).await;
    assert!(r.location.contains("error="), "an existing file was overwritten: {}", r.location);
    assert!(r.flash().contains("already a file called"), "{}", r.flash());
    assert_eq!(f.read(&neighbour), b"someone else's picture");

    // A name with nothing usable in it is refused with a reason.
    let r = post(&f.state, "/media/item/rename/", &cookie, &[("path", path), ("name", "!!!")]).await;
    assert!(r.flash().contains("no letters or numbers"), "{}", r.flash());
    assert!(f.exists(path), "the file went missing after a refused rename");
}

#[tokio::test]
async fn a_replacement_has_to_be_the_same_kind_of_file() {
    let f = fixture().await;
    let cookie = f.session().await;
    let path = f.put("images/2026/11/cover-ab12.png", b"the original");

    for (filename, content_type) in [("other.jpg", "image/jpeg"), ("page.html", "text/html"), ("clip.mp4", "video/mp4")] {
        let (ct, body) = multipart(&[("path", &path)], &[("file", filename, content_type, b"something else")]);
        let r = send(&f.state, "POST", "/media/item/replace/", Some(&cookie), Some(ORIGIN), Some((&ct, body))).await;
        assert!(r.location.contains("error="), "a {content_type} replaced a PNG: {}", r.location);
        assert_eq!(f.read(&path), b"the original", "{content_type} overwrote the file");
    }

    // Choosing nothing is answered rather than writing an empty file.
    let (ct, body) = multipart(&[("path", &path)], &[]);
    let r = send(&f.state, "POST", "/media/item/replace/", Some(&cookie), Some(ORIGIN), Some((&ct, body))).await;
    assert!(r.flash().contains("Choose the replacement"), "{}", r.flash());
    assert_eq!(f.read(&path), b"the original");
}

#[tokio::test]
async fn the_selection_deletes_every_file_it_names_and_only_those() {
    let f = fixture().await;
    let cookie = f.session().await;
    let one = f.put("images/2026/11/one.jpg", b"1");
    let two = f.put("images/2026/11/two.jpg", b"2");
    let kept = f.put("images/2026/11/kept.jpg", b"3");

    // A hostile entry in the middle of the list does not stop the others and
    // does not redirect them anywhere else.
    let form = encode(&[("paths", &one), ("paths", "images/../../data/ghost.db"), ("paths", &two)]);
    let r = send(
        &f.state,
        "POST",
        "/media/delete/",
        Some(&cookie),
        Some(ORIGIN),
        Some(("application/x-www-form-urlencoded", form.into_bytes())),
    )
    .await;
    assert!(r.redirected_ok(), "{}", r.location);
    assert!(r.flash().contains("2 files deleted"), "{}", r.flash());
    assert_eq!(f.all_files(), vec![kept]);

    // Selecting nothing says so.
    let r = post(&f.state, "/media/delete/", &cookie, &[]).await;
    assert!(r.flash().contains("Nothing was selected"), "{}", r.flash());
}

#[tokio::test]
async fn a_file_says_which_posts_use_it_before_it_is_deleted() {
    let f = fixture().await;
    let cookie = f.session().await;
    let path = f.put("images/2026/11/cover-ab12.jpg", b"a picture");
    sqlx::query(
        "INSERT INTO posts (id, title, slug, html, type, status, created_at, updated_at) \
         VALUES ('000000000000000000000001', 'The autumn issue', 'the-autumn-issue', ?, 'post', 'published', '2026-11-14 10:00:00', '2026-11-14 10:00:00')",
    )
    .bind(r#"<img src="/content/images/2026/11/cover-ab12.jpg">"#)
    .execute(&f.state.db)
    .await
    .unwrap();

    let item = get(&f.state, &item_uri(&path), &cookie).await;
    assert!(item.body.contains("The autumn issue"), "the post using the file is not named");
    assert!(item.body.contains("/ghost/posts/000000000000000000000001/"), "no link to the post");
    assert!(item.body.contains("1 post or page still points at this file"), "no warning: {}", item.body.len());

    // A file nothing uses says that instead, so the warning means something.
    let unused = f.put("images/2026/11/spare-cd34.jpg", b"unused");
    let item = get(&f.state, &item_uri(&unused), &cookie).await;
    assert!(item.body.contains("No post or page points at this file"), "the unused file still warned");
}

#[tokio::test]
async fn what_someone_typed_about_a_file_is_escaped_everywhere_it_is_shown() {
    let f = fixture().await;
    let cookie = f.session().await;
    let path = f.put("images/2026/11/cover-ab12.jpg", b"a picture");

    post(&f.state, "/media/item/save/", &cookie, &[("path", &path), ("title", HOSTILE), ("caption", HOSTILE)]).await;

    for uri in ["/media/", &item_uri(&path)] {
        let r = get(&f.state, uri, &cookie).await;
        assert!(!r.body.contains("<img src=x"), "{uri} rendered markup from a title");
        assert!(!r.body.contains("<script>alert(2)"), "{uri} rendered a script from a title");
        assert!(r.body.contains("&lt;img src=x"), "{uri} did not show the title at all");
    }

    // A search term comes back into the page and into every filter link.
    let r = get(&f.state, &format!("/media/?q={}", form_urlencoded::byte_serialize(HOSTILE.as_bytes()).collect::<String>()), &cookie).await;
    assert!(!r.body.contains("<img src=x"), "a search term was rendered as markup");
    assert!(!r.body.contains("<script>alert(2)"), "a search term was rendered as a script");
}

#[tokio::test]
async fn the_modules_hub_lays_its_cards_out_four_across() {
    let f = fixture().await;
    let cookie = f.session().await;
    let page = get(&f.state, "/modules/", &cookie).await;
    // The stylesheet is linked, not inlined, so the rule is checked where it
    // lives and the page is checked for carrying the grid it applies to.
    assert!(page.body.contains(r#"class="module-grid"#), "the hub is not a module grid");
    // Read from the file the crate serves rather than through a widened public
    // API — the stylesheet is `pub(crate)` and should stay that way.
    let css = include_str!("../assets/admin.css");
    assert!(
        css.contains(".module-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr))"),
        "the module grid is not four across"
    );
    // …and steps down rather than keeping four columns on a narrow window
    for narrower in ["repeat(3, minmax(0, 1fr))", "repeat(2, minmax(0, 1fr))"] {
        assert!(css.contains(narrower), "the grid never narrows to {narrower}");
    }
}
