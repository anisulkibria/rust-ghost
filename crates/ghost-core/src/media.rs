//! The Media module's rules: where a stored file may live, what it may be
//! called, and what a browser is allowed to upload.
//!
//! Pure — no filesystem, no database, no HTTP types. The admin's media
//! manager, the upload endpoint in `ghost-api` and the tests all reach the
//! same answers through this module, and the answers can be checked directly.
//!
//! This is the trust boundary for two separate kinds of hostile input, and it
//! is worth being explicit about both:
//!
//! * A **content type** offered by an upload, checked against a fixed
//!   allowlist per family ([`Kind::extension`]). The extension written to disk
//!   is the one *this* table maps the type to, never the one the filename
//!   carried — so a video card cannot be handed a `.exe`, and an image cannot
//!   arrive named `.php`. The `File` family is deliberately open, the way
//!   Ghost's file card is, and keeps the browser's extension after reducing it
//!   to lowercase alphanumerics. Which of the three a *library* upload is
//!   checked against is itself chosen by the declared type, so read
//!   [`kind_for_upload`]'s own note on what that does and does not promise
//!   before relying on the allowlist there.
//! * A **path** arriving in a query string or a form, naming a file the server
//!   is about to read, rewrite or delete ([`parse_location`]). Nothing here
//!   resolves or pops a `..`: a path naming a parent, starting at the root, or
//!   not beginning with one of the three family directories is refused
//!   outright. Callers get back a [`Location`] they rebuild every path from,
//!   so the string that reaches the filesystem is one this module composed and
//!   not one a request supplied.

use crate::slug;

/// Longest title put beside a file. Matches the `VARCHAR(191)` the other
/// name-shaped columns in this schema use.
pub const MAX_TITLE: usize = 191;
/// Longest alt text. Long enough for a real description, short enough that it
/// cannot be used to park a document in the database.
pub const MAX_ALT: usize = 300;
/// Longest caption.
pub const MAX_CAPTION: usize = 500;
/// Longest stored path, family directory included.
///
/// Comfortably over `images/YYYY/MM/<60-character stem>-abcd.jpeg`, and short
/// enough that the path can be the `VARCHAR(400)` primary key of the metadata
/// table rather than a hash of itself.
pub const MAX_PATH: usize = 400;
/// Longest file name a rename may produce, extension included.
pub const MAX_NAME: usize = 120;
/// Longest extension accepted from a browser, for the open `File` family.
const MAX_EXTENSION: usize = 8;

/// Content types the image family accepts, and the extension each is stored
/// under. The extension comes from here, not from the uploaded filename.
pub const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/jpeg", "jpg"),
    ("image/jpg", "jpg"),
    ("image/png", "png"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("image/svg+xml", "svg"),
    ("image/avif", "avif"),
];

/// Content types the video/audio family accepts. Several spellings of the same
/// format appear because browsers disagree about which one to send.
pub const MEDIA_TYPES: &[(&str, &str)] = &[
    ("video/mp4", "mp4"),
    ("video/webm", "webm"),
    ("video/ogg", "ogv"),
    ("audio/mpeg", "mp3"),
    ("audio/mp3", "mp3"),
    ("audio/ogg", "ogg"),
    ("audio/wav", "wav"),
    ("audio/vnd.wav", "wav"),
    ("audio/wave", "wav"),
    ("audio/x-wav", "wav"),
    ("audio/mp4", "m4a"),
    ("audio/x-m4a", "m4a"),
];

/// Extensions a browser will draw as a picture. Used to decide whether a grid
/// tile shows a thumbnail or a glyph — never to decide whether an upload is
/// allowed, which is [`Kind::extension`]'s job.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "avif", "svg"];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "ogv", "mov", "m4v"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "ogg", "wav", "m4a", "aac", "flac"];

/// The three families a stored file can belong to.
///
/// They differ in where they land, what they accept and how large they may be.
/// The variant name is also the directory under `content/` and the resource
/// name the upload endpoint answers with, which is why [`Kind::as_str`] is the
/// one spelling used for all three.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Image,
    Media,
    File,
}

impl Kind {
    /// Every family, in the order the admin offers them.
    pub const ALL: &'static [Kind] = &[Kind::Image, Kind::Media, Kind::File];

    /// The directory under `content/`, and the family's key in a URL or form.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "images",
            Kind::Media => "media",
            Kind::File => "files",
        }
    }

    /// What a filter tab or a group heading calls it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Image => "Images",
            Kind::Media => "Video & audio",
            Kind::File => "Files",
        }
    }

    /// What one of them is called in a sentence.
    pub fn singular(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Media => "media file",
            Kind::File => "file",
        }
    }

    /// The glyph the admin's `icon()` draws for this family.
    pub fn glyph(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Media => "play",
            Kind::File => "file",
        }
    }

    /// Where files in this family are reachable on the public site.
    pub fn url_prefix(self) -> &'static str {
        match self {
            Kind::Image => "/content/images",
            Kind::Media => "/content/media",
            Kind::File => "/content/files",
        }
    }

    /// The upload ceiling. Bounds memory as much as disk: a multipart body is
    /// buffered whole before it is written out.
    pub fn max_bytes(self) -> usize {
        match self {
            Kind::Image => 25 * 1024 * 1024,
            Kind::Media => 200 * 1024 * 1024,
            Kind::File => 100 * 1024 * 1024,
        }
    }

    /// The family named by a directory or form value, if it names one.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// The extension an accepted upload is stored under, or why it was refused.
    ///
    /// For images and video/audio the answer comes from this module's
    /// allowlist, so what lands on disk is decided here and not by the
    /// uploader. The file family takes anything — that is what a file card is
    /// for — and keeps the browser's extension, reduced to at most
    /// [`MAX_EXTENSION`] lowercase alphanumerics so it cannot smuggle a path,
    /// a second extension or a control character.
    pub fn extension(self, content_type: &str, filename: Option<&str>) -> Result<String, String> {
        let allowed = match self {
            Kind::Image => IMAGE_TYPES,
            Kind::Media => MEDIA_TYPES,
            Kind::File => {
                return Ok(filename
                    .map(extension_of)
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| "bin".into()))
            }
        };
        let content_type = content_type.trim().to_ascii_lowercase();
        allowed
            .iter()
            .find(|(mime, _)| *mime == content_type)
            .map(|(_, ext)| (*ext).to_string())
            .ok_or_else(|| format!("unsupported type {content_type}"))
    }

    /// Which family a file already on disk belongs in, judged by its
    /// extension. Only consulted when something has been put in a directory
    /// by hand; an upload's family is decided before it is written.
    pub fn of_extension(ext: &str) -> Self {
        let ext = ext.to_ascii_lowercase();
        if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            Kind::Image
        } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) || AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            Kind::Media
        } else {
            Kind::File
        }
    }
}

/// Which family an upload belongs in, judged by the type it declares.
///
/// The media manager asks this instead of making someone choose: a picture
/// goes to `images`, a video or a sound to `media`, and everything else to
/// `files`.
///
/// Only the declared type routes an upload; the filename deliberately does
/// not. Guessing from the name would send a `.mov` to the media family, whose
/// allowlist has no entry for QuickTime, and it would be **refused** — where
/// routing it to the open `files` family accepts it, which is both what the
/// upload endpoint did before this function existed and the better answer. A
/// file whose type nothing recognises belongs with the other files, not in a
/// family that will turn it away.
///
/// ## What this does and does not promise
///
/// Be precise about it, because the shape is easy to misread: because the type
/// is what *chooses* the family, the family's own [`Kind::extension`] check can
/// never contradict it. An upload routed here is always accepted somewhere, and
/// a lie about the type changes only which directory it lands in and what
/// extension it is stored under — never whether it is stored.
///
/// So this function is not a filter, and the allowlists are not doing their
/// work here. They do it in the two places where the family is fixed by
/// something other than the declared type:
///
/// * the per-family upload endpoints (`/ghost/api/admin/images/upload/` and
///   friends), where the editor's card names the family in the URL, so a video
///   card genuinely cannot be handed an executable;
/// * replacing a file, where the file already on disk fixes the family and the
///   format has to match ([`same_format`]).
///
/// What protects the unified upload is a different property: the stored name is
/// composed by the server from a slugged stem, an extension this module chose,
/// and a random tail, so nothing an uploader sends decides where bytes land.
pub fn kind_for_upload(content_type: &str) -> Kind {
    let content_type = content_type.trim().to_ascii_lowercase();
    if IMAGE_TYPES.iter().any(|(mime, _)| *mime == content_type) {
        Kind::Image
    } else if MEDIA_TYPES.iter().any(|(mime, _)| *mime == content_type) {
        Kind::Media
    } else {
        Kind::File
    }
}

/// A stored file's place: which family, and where under that family's
/// directory.
///
/// Only [`parse_location`] builds one from untrusted text. Everything that
/// needs a path, a URL or a name asks this struct for it, so those strings are
/// composed from parts that have already been checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub kind: Kind,
    /// The path under the family's directory, e.g. `2026/11/cover-ab12.jpg`.
    /// Never empty, never absolute, and holds no `.` or `..` segment.
    pub rest: String,
}

impl Location {
    /// The canonical stored form: `images/2026/11/cover-ab12.jpg`.
    pub fn path(&self) -> String {
        format!("{}/{}", self.kind.as_str(), self.rest)
    }

    /// Where the file is reachable on the public site, without the install's
    /// base path — callers add that with `crate::url::base` at the point of
    /// output, as every other stored URL in this project does.
    pub fn url(&self) -> String {
        format!("{}/{}", self.kind.url_prefix(), self.rest)
    }

    /// The file's own name, extension included.
    pub fn name(&self) -> &str {
        self.rest.rsplit('/').next().unwrap_or(&self.rest)
    }

    /// The directories between the family and the file (`2026/11`), or `""`
    /// for a file sitting directly in the family's directory.
    pub fn folder(&self) -> &str {
        match self.rest.rfind('/') {
            Some(at) => &self.rest[..at],
            None => "",
        }
    }

    /// This file's extension, lowercased and without the dot.
    pub fn extension(&self) -> String {
        extension_of(self.name())
    }

    /// Whether a browser will draw this as a picture, so a grid tile can show
    /// the file itself rather than a glyph.
    pub fn is_viewable_image(&self) -> bool {
        IMAGE_EXTENSIONS.contains(&self.extension().as_str())
    }

    pub fn is_video(&self) -> bool {
        VIDEO_EXTENSIONS.contains(&self.extension().as_str())
    }

    pub fn is_audio(&self) -> bool {
        AUDIO_EXTENSIONS.contains(&self.extension().as_str())
    }

    /// The same file renamed, with the name already checked by
    /// [`rename_target`]. Keeps the folder it is in.
    pub fn with_name(&self, name: &str) -> Location {
        let folder = self.folder();
        Location {
            kind: self.kind,
            rest: if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") },
        }
    }
}

/// Read a stored path that arrived from a request.
///
/// Accepts exactly the shape this server writes: a family directory, then one
/// or more plain segments. Everything else is `None` — an absolute path, a
/// Windows-style path, a `.` or `..` segment, an empty segment, a segment of
/// only dots, a path over [`MAX_PATH`], or one whose first segment is not a
/// family. A `..` is **refused, not popped**: nothing this server stores has a
/// legitimate reason to name a parent, so treating it as a typo to be fixed
/// would only widen what the parser accepts.
///
/// Percent-encoding is not decoded here. By the time a path reaches this
/// function it has been through the query-string or form decoder, so `%2e%2e`
/// has already become `..` and is refused above; a literal `%` left in a
/// segment is just a character in a file name.
pub fn parse_location(path: &str) -> Option<Location> {
    let path = path.trim();
    if path.is_empty() || path.len() > MAX_PATH {
        return None;
    }
    // A backslash is a separator on one of the platforms this may run on and
    // a legal filename character on the other, which is reason enough never
    // to accept one.
    if path.contains('\\') || path.contains('\0') {
        return None;
    }
    let mut segments = path.split('/');
    let kind = Kind::parse(segments.next()?)?;
    let rest: Vec<&str> = segments.collect();
    if rest.is_empty() {
        return None;
    }
    for segment in &rest {
        if segment.is_empty() || segment.chars().all(|c| c == '.') {
            return None;
        }
        if segment.chars().any(|c| c.is_control()) {
            return None;
        }
    }
    Some(Location { kind, rest: rest.join("/") })
}

/// The lowercased extension of a file name, without the dot.
///
/// `""` when there is no extension, when it is longer than
/// [`MAX_EXTENSION`], or when it holds anything but ASCII alphanumerics — so
/// the result is always safe to append to a name this module composed.
pub fn extension_of(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let Some((stem, ext)) = name.rsplit_once('.') else { return String::new() };
    if stem.is_empty() {
        // A dotfile (`.gitkeep`) is all stem and no extension.
        return String::new();
    }
    let ext = ext.to_ascii_lowercase();
    if ext.is_empty() || ext.len() > MAX_EXTENSION || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return String::new();
    }
    ext
}

/// The name a rename should produce, or why it cannot.
///
/// The extension is never taken from what was typed: it is carried over from
/// the current name, so renaming cannot turn a `.png` into a `.html` and get
/// it served as a document from this site's own origin. The stem goes through
/// [`slug::safe_string`], which is also what an upload's stem goes through, so
/// a renamed file is named the way a freshly uploaded one would be.
pub fn rename_target(current_name: &str, requested: &str) -> Result<String, String> {
    let extension = extension_of(current_name);
    // Only the last component is considered: a rename names a file, never a
    // new place to put it.
    let requested = requested.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    if requested.is_empty() {
        return Err("Give the file a name.".into());
    }
    // A typed name usually carries the extension back; drop it so it is not
    // slugged into the stem as "-jpg".
    let stem = match requested.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.eq_ignore_ascii_case(&extension) => stem,
        _ => requested,
    };
    let room = MAX_NAME.saturating_sub(if extension.is_empty() { 0 } else { extension.len() + 1 });
    let stem: String = slug::safe_string(stem).chars().take(room).collect();
    let stem = stem.trim_matches('-').to_string();
    if stem.is_empty() {
        return Err("That name has no letters or numbers in it.".into());
    }
    Ok(if extension.is_empty() { stem } else { format!("{stem}.{extension}") })
}

/// Whether two extensions name the same stored format, so a file with one may
/// replace a file with the other.
///
/// Exists for a single pair. An upload of `image/jpeg` is always stored as
/// `.jpg`, so without this a file that reached the content directory as
/// `.jpeg` — copied in by hand, or restored from a Ghost export — could never
/// be replaced: the check would report that a `.jpg` cannot replace a `.jpeg`,
/// which is true of the spelling and false of the format.
pub fn same_format(a: &str, b: &str) -> bool {
    // Lowercased *before* the comparison, not after: `JPEG` has to reach the
    // `jpeg` arm, and folding the case afterwards would leave it in the
    // fall-through and report that a JPEG is not a JPG.
    fn canonical(extension: &str) -> String {
        let lower = extension.to_ascii_lowercase();
        if lower == "jpeg" {
            "jpg".to_string()
        } else {
            lower
        }
    }
    canonical(a) == canonical(b)
}

/// `248 KB`, `1.4 MB` — a size, in the units a person would say it in.
///
/// Binary multiples, matching what a file manager reports, with one decimal
/// place from a megabyte up so a 1.4 MB file does not read as the same size
/// as a 1.9 MB one.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{} KB", (bytes + KB / 2) / KB)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

/// Bound and de-control a line of text put beside a file.
///
/// Thin wrapper over [`crate::forms::clean_bounded`], named here so the media
/// module's call sites read in media terms and so this is the one place to
/// change if the two ever need to differ.
pub fn clean_text(raw: &str, multiline: bool, max: usize) -> String {
    crate::forms::clean_bounded(raw, multiline, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_naming_a_parent_or_a_root_is_refused_rather_than_repaired() {
        for hostile in [
            "images/../../etc/passwd",
            "images/..",
            "../images/x.jpg",
            "/images/x.jpg",
            "images//x.jpg",
            "images/./x.jpg",
            "images\\..\\x.jpg",
            "images/x\0.jpg",
            // not a family directory
            "themes/casper/package.json",
            "data/ghost-dev.db",
            "x.jpg",
            // a family with nothing under it names a directory, not a file
            "images",
            "images/",
            "",
            "   ",
        ] {
            assert_eq!(parse_location(hostile), None, "accepted {hostile:?}");
        }
        // and a path longer than the column that stores it
        let long = format!("images/{}.jpg", "a".repeat(MAX_PATH));
        assert_eq!(parse_location(&long), None);
    }

    #[test]
    fn a_stored_path_round_trips_through_its_parts() {
        let l = parse_location("images/2026/11/cover-ab12.jpg").expect("a real path");
        assert_eq!(l.kind, Kind::Image);
        assert_eq!(l.rest, "2026/11/cover-ab12.jpg");
        assert_eq!(l.path(), "images/2026/11/cover-ab12.jpg");
        assert_eq!(l.url(), "/content/images/2026/11/cover-ab12.jpg");
        assert_eq!(l.name(), "cover-ab12.jpg");
        assert_eq!(l.folder(), "2026/11");
        assert_eq!(l.extension(), "jpg");
        assert!(l.is_viewable_image() && !l.is_video() && !l.is_audio());
        assert_eq!(l.with_name("other.jpg").path(), "images/2026/11/other.jpg");

        let flat = parse_location("files/report.pdf").expect("a file at the top");
        assert_eq!(flat.folder(), "");
        assert_eq!(flat.with_name("new.pdf").path(), "files/new.pdf");
        assert!(!flat.is_viewable_image());

        let song = parse_location("media/2026/01/theme.mp3").expect("audio");
        assert_eq!(song.kind, Kind::Media);
        assert!(song.is_audio() && !song.is_video());
    }

    #[test]
    fn an_upload_is_stored_under_the_extension_the_allowlist_chose() {
        // the type decides, not the name the browser sent
        assert_eq!(Kind::Image.extension("image/png", Some("evil.php")).unwrap(), "png");
        assert_eq!(Kind::Image.extension("IMAGE/JPEG", Some("photo.jpeg")).unwrap(), "jpg");
        assert_eq!(Kind::Media.extension("audio/x-m4a", None).unwrap(), "m4a");
        // and an unlisted type is refused outright
        assert!(Kind::Image.extension("text/html", Some("x.png")).is_err());
        assert!(Kind::Media.extension("application/x-msdownload", Some("x.mp4")).is_err());
        // the open family keeps the browser's extension, reduced to a shape
        // that cannot carry a path or a second extension
        assert_eq!(Kind::File.extension("application/pdf", Some("report.pdf")).unwrap(), "pdf");
        assert_eq!(Kind::File.extension("application/octet-stream", Some("archive.TAR")).unwrap(), "tar");
        assert_eq!(Kind::File.extension("application/octet-stream", Some("x.ph p")).unwrap(), "bin");
        assert_eq!(Kind::File.extension("application/octet-stream", Some("../../x.sh")).unwrap(), "sh");
        assert_eq!(Kind::File.extension("application/octet-stream", Some("noextension")).unwrap(), "bin");
        assert_eq!(Kind::File.extension("application/octet-stream", None).unwrap(), "bin");
    }

    #[test]
    fn a_rename_cannot_change_the_extension_or_name_another_directory() {
        assert_eq!(rename_target("cover-ab12.jpg", "Autumn Cover").unwrap(), "autumn-cover.jpg");
        // the extension typed back is not slugged into the stem
        assert_eq!(rename_target("cover.jpg", "autumn.jpg").unwrap(), "autumn.jpg");
        // …and a different one is ignored, not honoured
        assert_eq!(rename_target("cover.png", "evil.html").unwrap(), "evil-html.png");
        assert_eq!(rename_target("cover.png", "../../evil").unwrap(), "evil.png");
        assert_eq!(rename_target("cover.png", "a/b/deep").unwrap(), "deep.png");
        assert_eq!(rename_target("report", "Quarter One").unwrap(), "quarter-one");
        // nothing usable in it
        assert!(rename_target("cover.png", "   ").is_err());
        assert!(rename_target("cover.png", "///").is_err());
        assert!(rename_target("cover.png", "!!!").is_err());
        // and the result is bounded, extension included
        let long = rename_target("cover.jpeg", &"nombre ".repeat(60)).unwrap();
        assert!(long.len() <= MAX_NAME, "{} chars", long.len());
        assert!(long.ends_with(".jpeg"));
    }

    #[test]
    fn jpg_and_jpeg_are_the_same_format_and_nothing_else_is() {
        assert!(same_format("jpg", "jpeg") && same_format("JPEG", "jpg"));
        assert!(same_format("png", "png"));
        assert!(!same_format("png", "jpg"));
        // genuinely different containers, however alike they look
        assert!(!same_format("ogg", "ogv"));
        assert!(!same_format("mp4", "m4a"));
        assert!(!same_format("html", "htm"), "those are served the same but are not this module's business");
    }

    #[test]
    fn sizes_read_the_way_a_file_manager_reports_them() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(254_000), "248 KB");
        assert_eq!(format_bytes(1_500_000), "1.4 MB");
        assert_eq!(format_bytes(3_221_225_472), "3.0 GB");
    }

    #[test]
    fn a_file_found_on_disk_is_classified_by_its_extension() {
        assert_eq!(Kind::of_extension("JPG"), Kind::Image);
        assert_eq!(Kind::of_extension("mov"), Kind::Media);
        assert_eq!(Kind::of_extension("flac"), Kind::Media);
        assert_eq!(Kind::of_extension("pdf"), Kind::File);
        assert_eq!(Kind::of_extension(""), Kind::File);
        assert_eq!(extension_of(".gitkeep"), "");
        assert_eq!(extension_of("no-dot"), "");
        assert_eq!(extension_of("x.verylongextension"), "");
        assert_eq!(extension_of("x.JPG"), "jpg");
    }

    #[test]
    fn an_upload_is_filed_by_the_type_it_declares_so_nobody_has_to_choose() {
        assert_eq!(kind_for_upload("image/png"), Kind::Image);
        assert_eq!(kind_for_upload("IMAGE/JPEG "), Kind::Image);
        assert_eq!(kind_for_upload("video/mp4"), Kind::Media);
        assert_eq!(kind_for_upload("audio/mpeg"), Kind::Media);
        assert_eq!(kind_for_upload("application/pdf"), Kind::File);
        // A type no family claims goes to the open one, which accepts it —
        // never to a family that would refuse it. QuickTime is the case that
        // matters: Safari sends `video/quicktime`, which is in no allowlist.
        assert_eq!(kind_for_upload("video/quicktime"), Kind::File);
        assert_eq!(kind_for_upload("application/octet-stream"), Kind::File);
        assert_eq!(kind_for_upload(""), Kind::File);
        assert!(Kind::File.extension("video/quicktime", Some("holiday.mov")).is_ok());
        // A lie about the type only picks the destination; that family's own
        // allowlist is what then refuses it.
        assert_eq!(kind_for_upload("image/png"), Kind::Image);
        assert!(Kind::Image.extension("application/x-sh", Some("payload.png")).is_err());
    }

    #[test]
    fn every_family_has_a_distinct_directory_url_and_ceiling() {
        let dirs: Vec<&str> = Kind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(dirs, ["images", "media", "files"]);
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(*k));
            assert_eq!(k.url_prefix(), format!("/content/{}", k.as_str()));
            assert!(k.max_bytes() > 0);
        }
        assert_eq!(Kind::parse("themes"), None);
        assert_eq!(Kind::parse("../images"), None);
    }
}
