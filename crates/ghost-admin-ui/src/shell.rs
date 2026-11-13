//! The page shell: the Material You chrome every admin screen is rendered into.

use super::*;

// ---------------------------------------------------------------------------
// layout — Material You (Android Settings / ChromeOS style) shell
// ---------------------------------------------------------------------------

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Inline 24dp Material Design icons (outlined style, strokeless fill paths).
pub(crate) fn icon(name: &str) -> &'static str {
    match name {
        "posts" => r#"<path d="M19 5v14H5V5h14m0-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-2.5 14H7v-2h9.5v2zm0-4H7v-2h9.5v2zm0-4H7V7h9.5v2z"/>"#,
        "pages" => r#"<path d="M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"/>"#,
        "tags" => r#"<path d="M21.41 11.58l-9-9C12.05 2.22 11.55 2 11 2H4c-1.1 0-2 .9-2 2v7c0 .55.22 1.05.59 1.42l9 9c.36.36.86.58 1.41.58s1.05-.22 1.41-.59l7-7c.37-.36.59-.86.59-1.41s-.23-1.06-.59-1.42zM5.5 7C4.67 7 4 6.33 4 5.5S4.67 4 5.5 4 7 4.67 7 5.5 6.33 7 5.5 7z"/>"#,
        "archive" => r#"<path d="M20.54 5.23l-1.39-1.68C18.88 3.21 18.47 3 18 3H6c-.47 0-.88.21-1.16.55L3.46 5.23C3.17 5.57 3 6.02 3 6.5V19c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V6.5c0-.48-.17-.93-.46-1.27zM12 17.5L6.5 12H10v-2h4v2h3.5L12 17.5zM5.12 5l.81-1h12l.94 1H5.12z"/>"#,
        "themes" => r#"<path d="M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 8 6.5 8 8 8.67 8 9.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z"/>"#,
        "tune" => r#"<path d="M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z"/>"#,
        "settings" => r#"<path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/>"#,
        "integrations" => r#"<path d="M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-1.99.9-1.99 2v3.8H3.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V19c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7 1.49 0 2.7 1.21 2.7 2.7V21H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z"/>"#,
        "add" => r#"<path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"/>"#,
        "chevron" => r#"<path d="M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z"/>"#,
        "logout" => r#"<path d="M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z"/>"#,
        "open" => r#"<path d="M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/>"#,
        "delete" => r#"<path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/>"#,
        "edit" => r#"<path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34c-.39-.39-1.02-.39-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/>"#,
        "search" => r#"<path d="M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/>"#,
        "code" => r#"<path d="M9.4 16.6L4.8 12l4.6-4.6L8 6l-6 6 6 6 1.4-1.4zm5.2 0l4.6-4.6-4.6-4.6L16 6l6 6-6 6-1.4-1.4z"/>"#,
        "mail" => r#"<path d="M20 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 4l-8 5-8-5V6l8 5 8-5v2z"/>"#,
        "send" => r#"<path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>"#,
        "globe" => r#"<path d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zm6.93 6h-2.95c-.32-1.25-.78-2.45-1.38-3.56 1.84.63 3.37 1.91 4.33 3.56zM12 4.04c.83 1.2 1.48 2.53 1.91 3.96h-3.82c.43-1.43 1.08-2.76 1.91-3.96zM4.26 14C4.1 13.36 4 12.69 4 12s.1-1.36.26-2h3.38c-.08.66-.14 1.32-.14 2 0 .68.06 1.34.14 2H4.26zm.82 2h2.95c.32 1.25.78 2.45 1.38 3.56-1.84-.63-3.37-1.9-4.33-3.56zm2.95-8H5.08c.96-1.66 2.49-2.93 4.33-3.56C8.81 5.55 8.35 6.75 8.03 8zM12 19.96c-.83-1.2-1.48-2.53-1.91-3.96h3.82c-.43 1.43-1.08 2.76-1.91 3.96zM14.34 14H9.66c-.09-.66-.16-1.32-.16-2 0-.68.07-1.35.16-2h4.68c.09.65.16 1.32.16 2 0 .68-.07 1.34-.16 2zm.25 5.56c.6-1.11 1.06-2.31 1.38-3.56h2.95c-.96 1.65-2.49 2.93-4.33 3.56zM16.36 14c.08-.66.14-1.32.14-2 0-.68-.06-1.34-.14-2h3.38c.16.64.26 1.31.26 2s-.1 1.36-.26 2h-3.38z"/>"#,
        "image" => r#"<path d="M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z"/>"#,
        "link" => r#"<path d="M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z"/>"#,
        "title" => r#"<path d="M5 4v3h5.5v12h3V7H19V4z"/>"#,
        "notes" => r#"<path d="M3 18h12v-2H3v2zM3 6v2h18V6H3zm0 7h18v-2H3v2z"/>"#,
        "excerpt" => r#"<path d="M4 9h16v2H4V9zm0 4h10v2H4v-2z"/>"#,
        "flag" => r#"<path d="M14.4 6L14 4H5v17h2v-7h5.6l.4 2h7V6z"/>"#,
        "star" => r#"<path d="M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"/>"#,
        "folder" => r#"<path d="M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/>"#,
        "folder-open" => r#"<path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z"/>"#,
        "file" => r#"<path d="M6 2c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6H6zm7 7V3.5L18.5 9H13z"/>"#,
        "copy" => r#"<path d="M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z"/>"#,
        "check" => r#"<path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/>"#,
        "ghost" => r#"<path d="M12 2a8 8 0 0 0-8 8v10.6c0 .7.85 1.05 1.35.55l1.9-1.9 2.2 2.2c.3.3.78.3 1.08 0L12 20l1.47 1.45c.3.3.78.3 1.08 0l2.2-2.2 1.9 1.9c.5.5 1.35.15 1.35-.55V10a8 8 0 0 0-8-8zm-3 8.5A1.5 1.5 0 1 1 10.5 9 1.5 1.5 0 0 1 9 10.5zm6 0A1.5 1.5 0 1 1 16.5 9 1.5 1.5 0 0 1 15 10.5z"/>"#,
        "menu" => r#"<path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z"/>"#,
        "comment" => r#"<path d="M20 2H4c-1.1 0-2 .9-2 2v18l4-4h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM6 9h12v2H6V9zm8 5H6v-2h8v2zm4-6H6V6h12v2z"/>"#,
        // The two fediverse verbs. A heart for a like, and the boost arrows
        // Mastodon made the universal sign for a repost.
        "heart" => r#"<path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/>"#,
        "repeat" => r#"<path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"/>"#,
        // a node wired to two others: the shape of an automation flow
        // an eye, for page views
        "eye" => r#"<path d="M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zm0 12.5a5 5 0 1 1 0-10 5 5 0 0 1 0 10zm0-8a3 3 0 1 0 0 6 3 3 0 0 0 0-6z"/>"#,
        // a sheet with a ticked box and two lines: a form to fill in
        "form" => r#"<path d="M19 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V5h14v14zM7 7h4v4H7V7zm1.3 1.3v1.4h1.4V8.3H8.3zM13 8h4v2h-4V8zm-6 5h10v2H7v-2zm0 3h7v1.5H7V16z"/>"#,
        "flow" => r#"<path d="M5 3a2.5 2.5 0 0 1 2.45 2H11a3 3 0 0 1 3 3v1.05A2.5 2.5 0 1 1 14 14v1a3 3 0 0 1-3 3H7.45a2.5 2.5 0 1 1 0-2H11a1 1 0 0 0 1-1v-1.05A2.5 2.5 0 0 1 12 9.5V8a1 1 0 0 0-1-1H7.45A2.5 2.5 0 1 1 5 3zm0 2a.5.5 0 1 0 0 1 .5.5 0 0 0 0-1zm0 11a.5.5 0 1 0 0 1 .5.5 0 0 0 0-1zm11.5-5a.5.5 0 1 0 0 1 .5.5 0 0 0 0-1z"/>"#,
        "hide" => r#"<path d="M12 7c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92c1.51-1.26 2.7-2.89 3.43-4.75-1.73-4.39-6-7.5-11-7.5-1.4 0-2.74.25-3.98.7l2.16 2.16C10.74 7.13 11.35 7 12 7zM2 4.27l2.28 2.28.46.46C3.08 8.3 1.78 10.02 1 12c1.73 4.39 6 7.5 11 7.5 1.55 0 3.03-.3 4.38-.84l.42.42L19.73 22 21 20.73 3.27 3 2 4.27zM7.53 9.8l1.55 1.55c-.05.21-.08.43-.08.65 0 1.66 1.34 3 3 3 .22 0 .44-.03.65-.08l1.55 1.55c-.67.33-1.41.53-2.2.53-2.76 0-5-2.24-5-5 0-.79.2-1.53.53-2.2z"/>"#,
        "close" => r#"<path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/>"#,
        "up" => r#"<path d="M7.41 15.41L12 10.83l4.59 4.58L18 14l-6-6-6 6z"/>"#,
        "down" => r#"<path d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6z"/>"#,
        "chart" => r#"<path d="M5 9.2h3V19H5V9.2zm5.6-4.2h2.8v14h-2.8V5zm5.6 8H19v6h-2.8v-6z"/>"#,
        "history" => r#"<path d="M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L8.99 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.954 8.954 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"/>"#,
        "upload" => r#"<path d="M9 16h6v-6h4l-7-7-7 7h4v6zm-4 2h14v2H5v-2z"/>"#,
        // arrows both ways: content coming in (import) and going out (export)
        "import" => r#"<path d="M9 3L5 6.99h3V14h2V6.99h3L9 3zm7 14.01V10h-2v7.01h-3L15 21l4-3.99h-3z"/>"#,
        "calendar" => r#"<path d="M19 3h-1V1h-2v2H8V1H6v2H5c-1.11 0-1.99.9-1.99 2L3 19a2 2 0 0 0 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V9h14v10zm-9-6H7v-2h3v2zm4 0h-3v-2h3v2zm4 0h-3v-2h3v2z"/>"#,
        "person" => r#"<path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"/>"#,
        "lock" => r#"<path d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1s3.1 1.39 3.1 3.1v2z"/>"#,
        "share" => r#"<path d="M18 16.08c-.76 0-1.44.3-1.96.77L8.91 12.7c.05-.23.09-.46.09-.7s-.04-.47-.09-.7l7.05-4.11c.54.5 1.25.81 2.04.81 1.66 0 3-1.34 3-3s-1.34-3-3-3-3 1.34-3 3c0 .24.04.47.09.7L8.04 9.81C7.5 9.31 6.79 9 6 9c-1.66 0-3 1.34-3 3s1.34 3 3 3c.79 0 1.5-.31 2.04-.81l7.12 4.16c-.05.21-.08.43-.08.65 0 1.61 1.31 2.92 2.92 2.92s2.92-1.31 2.92-2.92-1.31-2.92-2.92-2.92z"/>"#,
        "layout" => r#"<path d="M3 5v14h18V5H3zm16 2v3H5V7h14zM5 12h5v5H5v-5zm7 5v-5h7v5h-7z"/>"#,
        // a panel open on the right, and the same panel closed
        "panel-hide" => r#"<path d="M3 3h18c1.1 0 2 .9 2 2v14c0 1.1-.9 2-2 2H3c-1.1 0-2-.9-2-2V5c0-1.1.9-2 2-2zm11 2v14h7V5h-7z"/>"#,
        "panel-show" => r#"<path d="M3 3h18c1.1 0 2 .9 2 2v14c0 1.1-.9 2-2 2H3c-1.1 0-2-.9-2-2V5c0-1.1.9-2 2-2zm0 2v14h18V5H3zm11 1.5 1.4-1.4L20.9 12l-5.5 5.5-1.4-1.4 4.1-4.1-4.1-4.5z"/>"#,
        _ => "",
    }
}

pub(crate) fn svg(name: &str) -> String {
    format!(r#"<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">{}</svg>"#, icon(name))
}

/// The one place the admin borrows another set: Ghost's appearance menu uses
/// lucide's stroked sun, moon and monitor, and no filled Material glyph reads
/// as cleanly for it. Same `class="i"` contract as [`svg`] so sizing rules
/// apply, but drawn with strokes like lucide does.
pub(crate) fn svg_stroke(name: &str) -> String {
    let body = match name {
        "sun" => r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>"#,
        "moon" => r#"<path d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401"/>"#,
        "monitor" => r#"<rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/>"#,
        _ => "",
    };
    format!(
        r#"<svg class="i t-{name}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{}</svg>"#,
        body,
        name = name
    )
}

/// The admin's stylesheet: Material You — a tonal palette generated from
/// Ghost's brand pink (#FF1A75), light + dark schemes, and M3 component
/// styles.
///
/// It lives in `assets/admin.css` rather than in this file. At 130 KB it was
/// the largest thing in every admin response and was sent again with each
/// page; it is now linked, served by [`admin_asset`] from the binary, and
/// cached behind [`admin_css_hash`].
pub(crate) const CSS: &str = include_str!("../assets/admin.css");

/// A short content hash, for an asset's cache-busting query string.
///
/// The admin's assets are served `immutable` behind a version in the path,
/// which is only as good as someone remembering to bump it: a file changed
/// without a bump is served from the browser's cache for a year, and the page
/// quietly uses the old one. Hashing the contents into the query string makes
/// a changed file a new URL by construction, so this is what every admin
/// asset should be requested with (see `docs/module-pattern.md`).
pub(crate) fn asset_hash(contents: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(contents.as_bytes()))[..12].to_string()
}

/// [`CSS`]'s hash, computed once.
pub(crate) fn admin_css_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| asset_hash(CSS));
    &HASH
}

/// Tiny progressive-enhancement script: confirm dialogs, list filtering,
/// copy-to-clipboard with a snackbar. Everything works without it too.
/// Runs in <head>, before the stylesheet: the whole of dark mode keys off a
/// `dark` class on <html>, and this settles whose choice that is — the stored
/// preference verbatim, or the OS when the choice is `system` — before
/// anything is painted. Left callable on `window` so the Appearance menu can
/// re-resolve after a pick without duplicating the rule.
pub(crate) const THEME_JS: &str = r#"
(function () {
  var root = document.documentElement;
  var media = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  window.ghostApplyTheme = function () {
    var choice = root.dataset.theme;
    root.classList.toggle('dark', choice === 'dark' || (choice === 'system' && !!media && media.matches));
  };
  window.ghostApplyTheme();
  if (media) {
    var onChange = function () { if (root.dataset.theme === 'system') window.ghostApplyTheme(); };
    if (media.addEventListener) media.addEventListener('change', onChange);
    else if (media.addListener) media.addListener(onChange);
  }
})();
"#;

pub(crate) const JS: &str = r#"
(function () {
  // On a phone the nav rail is a horizontal strip too narrow to hold every
  // section, and it opens scrolled to the start — so on /ghost/settings/ the
  // one item that says where you are is the one off the right-hand edge.
  // Bring it into view. Instant, not smooth: this runs at load, and a strip
  // that animates sideways on arrival reads as a glitch.
  // Same for the analytics range pills, where the chosen period is just as
  // likely to be the one past the edge.
  ['.rail .nav-scroll .nav-item.active', '.range-tabs a.active'].forEach(function (sel) {
    var active = document.querySelector(sel);
    if (!active) return;
    var strip = active.parentElement;
    if (strip.scrollWidth <= strip.clientWidth) return;
    // Centre it where there is room on both sides, so the items either side
    // stay visible and the strip reads as scrollable.
    strip.scrollLeft = Math.max(0, active.offsetLeft - (strip.clientWidth - active.offsetWidth) / 2);
  });

  // …then mark which edge of each strip still has items past it, so the CSS
  // above fades that side and only that side. Re-read on scroll and on
  // resize: a strip that fits at one width hides items at another.
  document.querySelectorAll('.rail .nav-scroll, .range-tabs').forEach(function (strip) {
    function edge() {
      var max = strip.scrollWidth - strip.clientWidth;
      if (max <= 1) { strip.removeAttribute('data-edge'); return; }
      var x = strip.scrollLeft;
      strip.dataset.edge = x <= 1 ? 'end' : (x >= max - 1 ? 'start' : 'both');
    }
    strip.addEventListener('scroll', edge, { passive: true });
    window.addEventListener('resize', edge);
    edge();
  });

  document.querySelectorAll('[data-brand-image]').forEach(function (row) {
    var url = row.querySelector('[data-image-url]');
    var file = row.querySelector('input[type="file"]');
    var preview = row.querySelector('img');
    var upload = row.querySelector('[data-image-upload]');
    var remove = row.querySelector('[data-image-remove]');
    var status = row.querySelector('[role="status"]');
    var form = url.form;
    var unsplash = row.querySelector('[data-image-unsplash]');
    if (unsplash) unsplash.addEventListener('click', function () {
      window.GhostUnsplash.open({
        headers: JSON.parse(form.dataset.unsplash || 'null'),
        onSelect: function (src) {
          url.value = src;
          refresh();
          status.textContent = 'Cover selected. Save to apply.';
        }
      });
    });
    function refresh() {
      var value = url.value.trim();
      preview.hidden = !value;
      if (value) preview.src = value;
      else preview.removeAttribute('src');
      remove.hidden = !value;
    }
    url.addEventListener('input', refresh);
    upload.addEventListener('click', function () { file.click(); });
    remove.addEventListener('click', function () {
      url.value = '';
      refresh();
      status.textContent = 'Image removed. Save to apply.';
    });
    file.addEventListener('change', async function () {
      var image = file.files[0];
      if (!image) return;
      var pending = Number(form.dataset.pendingUploads || 0) + 1;
      form.dataset.pendingUploads = String(pending);
      upload.disabled = remove.disabled = url.disabled = true;
      if (unsplash) unsplash.disabled = true;
      status.textContent = 'Uploading…';
      try {
        var data = new FormData();
        data.append('file', image);
        var response = await fetch(row.dataset.uploadUrl, {method: 'POST', credentials: 'same-origin', body: data});
        var result = await response.json().catch(function () { return {}; });
        if (!response.ok) throw new Error(result.errors?.[0]?.message || 'Upload failed. Please try again.');
        var uploadedUrl = result.images?.[0]?.url;
        if (!uploadedUrl) throw new Error('The upload did not return an image URL.');
        url.value = uploadedUrl;
        refresh();
        status.textContent = 'Image uploaded. Save to apply.';
      } catch (error) {
        status.textContent = error.message || 'Upload failed. Please try again.';
      } finally {
        upload.disabled = remove.disabled = url.disabled = false;
        if (unsplash) unsplash.disabled = false;
        file.value = '';
        form.dataset.pendingUploads = String(Number(form.dataset.pendingUploads) - 1);
      }
    });
    refresh();
  });
  var form = document.getElementById('design-form');
  if (form) form.addEventListener('submit', function (event) {
    if (Number(form.dataset.pendingUploads) > 0) {
      event.preventDefault();
      form.querySelectorAll('[data-brand-image] [role="status"]').forEach(function (status) {
        if (status.textContent === 'Uploading…') status.textContent = 'Please wait for the upload to finish before saving.';
      });
    }
  });
})();
(function () {
  // M3 confirm dialog for forms carrying data-confirm.
  //
  // `pending` has to live outside the handler: the dialog's close listener is
  // registered once, so if it closed over the form from the first submit it
  // would re-submit that same form every time afterwards — on a page with
  // several delete buttons, confirming the second one would act on the first.
  var dlg = null, pending = null, pendingBtn = null;
  document.addEventListener('submit', function (e) {
    var f = e.target;
    // A button may carry its own data-confirm, which wins over the form's —
    // one form can hold destructive and ordinary actions side by side (the
    // batch bar: delete asks, "add label" does not).
    var msg = (e.submitter && e.submitter.dataset.confirm) || f.dataset.confirm;
    if (!msg || f.dataset.confirmed) return;
    e.preventDefault();
    pending = f;
    pendingBtn = e.submitter || null;
    if (!dlg) {
      dlg = document.createElement('dialog');
      // The buttons must live in a <form method="dialog">. A bare button
      // inside a <dialog> does not close it and never sets returnValue, so
      // without this the Confirm button is inert and nothing is ever
      // submitted — which is what made every delete look broken.
      dlg.innerHTML = '<h3></h3><p></p><form method="dialog" class="d-actions">' +
        '<button class="btn text" value="cancel">Cancel</button>' +
        '<button class="btn tonal" value="ok">Confirm</button></form>';
      document.body.appendChild(dlg);
      dlg.addEventListener('close', function () {
        var form = pending, btn = pendingBtn;
        pending = null; pendingBtn = null;
        if (dlg.returnValue !== 'ok' || !form) return;
        form.dataset.confirmed = '1';
        // requestSubmit keeps the clicked button's name/value in the payload;
        // form.submit() would drop it and break action-carrying buttons
        if (form.requestSubmit) { form.requestSubmit(btn || undefined); }
        else { form.submit(); }
      });
    }
    var parts = msg.split('|');
    dlg.querySelector('h3').textContent = parts[0];
    dlg.querySelector('p').textContent = parts[1] || 'This cannot be undone.';
    dlg.showModal();
  });

  // Meta data previews follow the fields as they are typed into. Each
  // preview element is tagged data-prev="<field>", and empty fields fall back
  // the way the front end does: the specific tag, then the meta tag, then the
  // site's own title/description.
  var form = document.querySelector('[data-meta-previews]');
  if (form && document.querySelector('[data-prev]')) {
    var val = function (name) {
      var el = form.elements[name];
      return el ? el.value.trim() : '';
    };
    // The site title and description are edited on this same form now, so
    // they are read live and the stored values are only the starting point.
    var site = { title: form.dataset.siteTitle || '', description: form.dataset.siteDescription || '' };
    var put = function (key, text) {
      var el = document.querySelector('[data-prev="' + key + '"]');
      if (el) { el.textContent = text; }
    };
    var refresh = function () {
      var metaTitle = val('meta_title') || val('title') || site.title;
      var metaDesc = val('meta_description') || val('description') || site.description;
      put('search-title', metaTitle);
      put('search-description', metaDesc);
      put('twitter-title', val('twitter_title') || metaTitle);
      put('twitter-description', val('twitter_description') || metaDesc);
      put('og-title', val('og_title') || metaTitle);
      put('og-description', val('og_description') || metaDesc);
      [['twitter_image', 'twitter-shot'], ['og_image', 'og-shot']].forEach(function (pair) {
        var slot = document.querySelector('[data-prev="' + pair[1] + '"]');
        if (!slot) return;
        var url = val(pair[0]);
        if (url) {
          slot.className = 'shot';
          slot.innerHTML = '';
          var img = document.createElement('img');
          img.alt = '';
          img.src = url;
          slot.appendChild(img);
        } else {
          slot.className = 'shot empty';
          slot.textContent = 'No image set';
        }
      });
    };
    form.addEventListener('input', refresh);
  }

  // client-side list filtering. Rows hide as you type; the empty state says
  // what happened instead of leaving a blank card, and the batch select-all
  // resyncs — "all" after a filter means "all the rows still shown".
  var batchSyncs = [];
  var q = document.querySelector('[data-filter]');
  if (q) {
    var rows = Array.from(document.querySelectorAll('[data-filterable]'));
    var empty = document.querySelector('[data-filter-empty]');
    var emptyQuery = empty ? empty.querySelector('[data-query]') : null;
    q.addEventListener('input', function () {
      var v = q.value.toLowerCase();
      var shown = 0;
      rows.forEach(function (r) {
        var on = r.textContent.toLowerCase().includes(v);
        r.style.display = on ? '' : 'none';
        if (on) shown++;
      });
      if (empty) {
        empty.classList.toggle('on', shown === 0);
        if (emptyQuery) emptyQuery.textContent = q.value;
      }
      batchSyncs.forEach(function (sync) { sync(); });
    });
    var clear = document.querySelector('[data-filter-clear]');
    if (clear) clear.addEventListener('click', function () {
      q.value = '';
      q.dispatchEvent(new Event('input'));
      q.focus();
    });
  }

  // ---- batch selection ----
  // Whether the bar is on screen is CSS (`body:has(… :checked)`), so selection
  // needs no script at all. The script keeps the page honest: the count in
  // the bar, the select-all box's indeterminate state, the number inside the
  // delete confirm, and shift-click ranges across rows.
  document.querySelectorAll('[data-batch]').forEach(function (root) {
    var form = root.querySelector('form.batch');
    var boxes = Array.from(root.querySelectorAll('.row-check input[name="ids"]'));
    if (!form || !boxes.length) return;
    var heads = Array.from(root.querySelectorAll('[data-select-all]'));
    var count = form.querySelector('[data-count]');
    var del = form.querySelector('[data-confirm-many]');
    var word = form.getAttribute('data-word') || 'item';
    var last = null;

    function visible(b) {
      var row = b.closest('[data-filterable]');
      return !row || row.style.display !== 'none';
    }
    function sync() {
      var n = boxes.filter(function (b) { return b.checked; }).length;
      // the bar already says "selected" beside this slot, so the slot is
      // only ever the number and its noun
      if (count) count.textContent = n + ' ' + (n === 1 ? word : word + 's');
      heads.forEach(function (h) {
        var vis = boxes.filter(visible);
        var on = vis.filter(function (b) { return b.checked; }).length;
        h.checked = vis.length > 0 && on === vis.length;
        h.indeterminate = on > 0 && on < vis.length;
      });
      if (del) {
        var many = del.getAttribute('data-confirm-many') || '';
        var one = del.getAttribute('data-confirm-one') || many;
        del.dataset.confirm = (n === 1 ? one : many).split('{n}').join(String(n));
      }
    }
    boxes.forEach(function (b) {
      b.addEventListener('click', function (e) {
        if (e.shiftKey && last) {
          e.preventDefault();
          var from = boxes.indexOf(last), to = boxes.indexOf(b);
          var on = !b.checked; // the state a plain click would land on
          boxes.slice(Math.min(from, to), Math.max(from, to) + 1)
            .forEach(function (r) { if (visible(r)) r.checked = on; });
        }
        last = b;
        sync();
      });
    });
    heads.forEach(function (h) {
      h.addEventListener('click', function () {
        var on = h.checked; // already flipped by the time a click fires
        boxes.forEach(function (b) { if (visible(b)) b.checked = on; });
        sync();
      });
    });
    batchSyncs.push(sync);
    sync();
  });

  // popovers in the batch bar, page head and sort menu close on outside
  // click and Esc, like menus rather than disclosures that stay open out of
  // spite — anything carrying data-menu
  document.addEventListener('click', function (e) {
    document.querySelectorAll('details[data-menu][open]').forEach(function (d) {
      if (!d.contains(e.target)) d.removeAttribute('open');
    });
  });
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    document.querySelectorAll('details[data-menu][open]').forEach(function (d) {
      d.removeAttribute('open');
    });
  });

  // settings hub search: typing swaps the page list for a flat index of
  // pages, sub-pages, headings and fields, so any setting is one query away
  // no matter which page buries it. The haystack rides in data-search
  // because a row's visible text says what a thing is, not everything it
  // should answer to.
  var hub = document.getElementById('settings-search');
  if (hub) {
    var pages = document.getElementById('settings-groups');
    var hitsCard = document.getElementById('settings-hits');
    var emptyRow = document.getElementById('settings-empty');
    var hits = Array.from(document.querySelectorAll('#settings-hits [data-search]'));
    hub.addEventListener('input', function () {
      var v = hub.value.trim().toLowerCase();
      var shown = 0;
      hits.forEach(function (h) {
        var on = !!v && h.dataset.search.toLowerCase().indexOf(v) !== -1;
        h.style.display = on ? '' : 'none';
        if (on) shown++;
      });
      // a field only shows while something is being asked; clearing the box
      // restores the pages, or every field in the index would appear
      pages.style.display = v ? 'none' : '';
      hitsCard.style.display = v ? '' : 'none';
      emptyRow.style.display = v && !shown ? '' : 'none';
    });
  }

  // colour settings: picking a custom colour selects the custom mode too, so
  // the swatch chip and the picker can never disagree about which is in force
  document.addEventListener('input', function (e) {
    if (e.target.type !== 'color') return;
    var chip = e.target.closest('.swatch.custom');
    if (!chip) return;
    var radio = chip.querySelector('input[type=radio]');
    if (radio) radio.checked = true;
  });

  // navigation editor: reorder/remove rows in place instead of round-tripping
  // (the action= buttons still work without JS — the server applies them)
  document.addEventListener('click', function (e) {
    var b = e.target.closest('[data-nav-move], [data-nav-remove]');
    if (!b) return;
    e.preventDefault();
    var row = b.closest('.nav-row');
    if (!row) return;
    if (b.hasAttribute('data-nav-remove')) { row.remove(); return; }
    if (b.dataset.navMove === 'up' && row.previousElementSibling) {
      row.parentElement.insertBefore(row, row.previousElementSibling);
    } else if (b.dataset.navMove === 'down' && row.nextElementSibling) {
      row.parentElement.insertBefore(row.nextElementSibling, row);
    }
  });

  // copy-to-clipboard + snackbar
  var toast = null, timer = null;
  function say(msg) {
    if (!toast) { toast = document.createElement('div'); toast.id = 'toast'; document.body.appendChild(toast); }
    toast.textContent = msg; toast.classList.add('show');
    clearTimeout(timer); timer = setTimeout(function () { toast.classList.remove('show'); }, 2200);
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest('[data-copy]');
    if (!b) return;
    navigator.clipboard.writeText(b.dataset.copy).then(function () { say('Copied to clipboard'); },
      function () { say('Copy failed'); });
  });

  // ---- Portal links reference ----
  // Every row carries both spellings of the same destination: the href a
  // theme puts in an <a>, and the data-portal attribute that turns any
  // element into the same trigger. The switch rewrites the shown value and
  // the copy button together, so what is copied is always what is read.
  // Tier rows hold a template with {tier} in it, filled from the picker.
  function paintPortalLinks() {
    var root = document.querySelector('[data-plinks]');
    if (!root) return;
    var mode = root.querySelector('input[name="portal_link_mode"]:checked');
    var attrs = mode && mode.value === 'attrs';
    var tierSel = root.querySelector('[data-plink-tier]');
    var tier = tierSel ? tierSel.value : '';
    root.querySelectorAll('[data-plink]').forEach(function (row) {
      var value = (attrs ? row.dataset.attr : row.dataset.link).split('{tier}').join(tier);
      var out = row.querySelector('code');
      var btn = row.querySelector('[data-copy]');
      if (out) out.textContent = value;
      if (btn) btn.dataset.copy = value;
    });
    var note = root.querySelector('[data-plink-note]');
    if (note) note.textContent = attrs ? 'data attributes' : 'links';
  }
  document.addEventListener('change', function (e) {
    if (e.target.name === 'portal_link_mode' || e.target.hasAttribute('data-plink-tier')) {
      paintPortalLinks();
    }
  });
  paintPortalLinks();

  // ---- code editor ----
  // Highlighting is a paint pass over the textarea's own value, so nothing
  // here can lose or reorder what you typed: if the tokenizer throws, the
  // worst case is uncoloured text over a working textarea.
  function esc(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
  // Walk `re` over `src`, handing each match to `paint`. Text between matches
  // goes to `rest`, which defaults to escaping it — passing a `rest` is how a
  // second pass gets layered over the gaps the first one left.
  function scan(src, re, paint, rest) {
    var out = '', last = 0, m;
    re.lastIndex = 0;
    while ((m = re.exec(src)) !== null) {
      // A zero-width match would spin forever; step past it.
      if (m.index === re.lastIndex) { re.lastIndex++; continue; }
      var gap = src.slice(last, m.index);
      out += rest ? rest(gap) : esc(gap);
      out += paint(m);
      last = m.index + m[0].length;
    }
    var tail = src.slice(last);
    return out + (rest ? rest(tail) : esc(tail));
  }
  function span(cls, text) { return '<span class="' + cls + '">' + esc(text) + '</span>'; }

  var MUSTACHE = /\{\{\{?[\s\S]*?\}?\}\}/g;
  // A quoted attribute value, where a mustache may carry quotes of its own:
  // `src="{{img_url x size="m"}}"`. Consuming a whole mustache as one
  // alternative is what stops the inner `"` from closing the value early —
  // pair them naively and the rest of the tag colours as a string.
  var ATTR_VALUE = '"(?:\\{\\{[\\s\\S]*?\\}\\}|[^"])*"|\'(?:\\{\\{[\\s\\S]*?\\}\\}|[^\'])*\'';

  // String-coloured, except for the mustaches inside it.
  function paintValue(v) {
    return '<span class="tok-str">' +
      scan(v, MUSTACHE, function (m) { return span('tok-mus', m[0]); }) +
      '</span>';
  }

  // Inside one `<tag ...>`: the name, its attribute names, and their values.
  function paintTag(tag) {
    return scan(
      tag,
      new RegExp('(' + ATTR_VALUE + ')|(\\{\\{\\{?[\\s\\S]*?\\}?\\}\\})|([\\w:@.-]+)(?==)|(<\\/?[A-Za-z][\\w:-]*|\\/?>)', 'g'),
      function (m) {
        if (m[1]) return paintValue(m[1]);
        if (m[2]) return span('tok-mus', m[2]);
        if (m[3]) return span('tok-atr', m[3]);
        return span('tok-tag', m[4]);
      }
    );
  }

  var JS_WORDS = /\b(const|let|var|function|return|if|else|for|while|of|in|new|class|extends|import|export|from|default|async|await|try|catch|finally|throw|typeof|instanceof|this|null|undefined|true|false|break|continue|switch|case|do|delete|void|yield|static)\b/;

  var LANGS = {
    // Handlebars is HTML plus mustaches, so one pass handles both. Comments
    // come first in the alternation: `{{!-- <div> --}}` is a comment, not a
    // tag, and the leftmost-longest rule only holds if it is offered first.
    hbs: function (src) {
      // A tag runs to the `>` that is not inside one of its attribute values,
      // so `<a title="a > b">` stays one tag; the alternation spells that out
      // rather than using `[^>]*`.
      var TAG = '<\\/?[A-Za-z][\\w:-]*(?:' + ATTR_VALUE + '|\\{\\{[\\s\\S]*?\\}\\}|[^>"\'])*\\/?>';
      return scan(
        src,
        new RegExp('(\\{\\{!--[\\s\\S]*?--\\}\\}|<!--[\\s\\S]*?-->)|(\\{\\{\\{?[\\s\\S]*?\\}?\\}\\})|(' + TAG + ')|(<![A-Za-z][^>]*>)', 'g'),
        function (m) {
          if (m[1]) return span('tok-com', m[1]);
          if (m[2]) return span('tok-mus', m[2]);
          if (m[3]) return paintTag(m[3]);
          return span('tok-key', m[4]);
        }
      );
    },
    css: function (src) {
      return scan(
        src,
        /(\/\*[\s\S]*?\*\/)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(@[\w-]+)|(#[0-9a-fA-F]{3,8}\b)|([{;]\s*)([\w-]+)(\s*:)|(-?\d*\.?\d+(?:px|em|rem|%|vh|vw|vmin|vmax|s|ms|deg|fr|ch|ex|pt)?\b)/g,
        function (m) {
          if (m[1]) return span('tok-com', m[1]);
          if (m[2]) return span('tok-str', m[2]);
          if (m[3]) return span('tok-key', m[3]);
          if (m[4]) return span('tok-num', m[4]);
          // A property is only a property after `{` or `;` — anchoring on
          // that is what keeps `a:hover` a selector rather than a declaration.
          if (m[6]) return esc(m[5]) + span('tok-prp', m[6]) + esc(m[7]);
          return span('tok-num', m[8]);
        }
      );
    },
    js: function (src) {
      return scan(
        src,
        /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|(`(?:[^`\\]|\\.)*`|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(\b(?:const|let|var|function|return|if|else|for|while|of|in|new|class|extends|import|export|from|default|async|await|try|catch|finally|throw|typeof|instanceof|this|null|undefined|true|false|break|continue|switch|case|do|delete|void|yield|static)\b)|(\b\d+\.?\d*\b)/g,
        function (m) {
          if (m[1]) return span('tok-com', m[1]);
          if (m[2]) return span('tok-str', m[2]);
          if (m[3]) return span('tok-key', m[3]);
          return span('tok-num', m[4]);
        }
      );
    },
    json: function (src) {
      return scan(
        src,
        /("(?:[^"\\]|\\.)*")(\s*:)?|(\btrue\b|\bfalse\b|\bnull\b)|(-?\d+\.?\d*(?:[eE][+-]?\d+)?)/g,
        function (m) {
          // A string followed by a colon is a key, and reads differently.
          if (m[1]) return span(m[2] ? 'tok-prp' : 'tok-str', m[1]) + (m[2] ? esc(m[2]) : '');
          if (m[3]) return span('tok-key', m[3]);
          return span('tok-num', m[4]);
        }
      );
    },
    md: function (src) {
      return scan(
        src,
        /(^```[\s\S]*?^```|^~~~[\s\S]*?^~~~)|(^#{1,6} .*$)|(^>.*$)|(`[^`\n]+`)|(\*\*[^*\n]+\*\*|__[^_\n]+__)|(\[[^\]\n]*\]\([^)\n]*\))/gm,
        function (m) {
          if (m[1]) return span('tok-str', m[1]);
          if (m[2]) return span('tok-hed', m[2]);
          if (m[3]) return span('tok-com', m[3]);
          if (m[4]) return span('tok-str', m[4]);
          if (m[5]) return span('tok-key', m[5]);
          return span('tok-mus', m[6]);
        }
      );
    },
    txt: function (src) { return esc(src); },
  };

  function paintEditor(ed) {
    var ta = ed.querySelector('.ced-in');
    var hl = ed.querySelector('.ced-hl');
    var gut = ed.querySelector('.ced-gut');
    if (!ta || !hl) return;
    var src = ta.value;
    var lang = LANGS[ed.dataset.lang] || LANGS.txt;
    var painted;
    try {
      painted = lang(src);
    } catch (err) {
      painted = esc(src);
    }
    // A value ending in a newline leaves the last (empty) line with nothing
    // to give the block height, so the highlight stops one row short of the
    // caret. The trailing newline keeps the two in step.
    hl.innerHTML = painted + '\n';
    var lines = src.split('\n').length;
    if (gut && gut.childElementCount !== lines) {
      var nums = '';
      for (var i = 1; i <= lines; i++) nums += '<div>' + i + '</div>';
      gut.innerHTML = nums;
    }
    // Size to the content, so a twelve-line partial is not framed by half a
    // screen of nothing and a long stylesheet still gets a usable window.
    // The line box is measured rather than assumed: it follows the font the
    // browser actually resolved, which is not always the one asked for.
    var lineH = (gut && gut.firstElementChild && gut.firstElementChild.offsetHeight) || 21;
    var min = 320;
    var max = Math.max(min, Math.round(window.innerHeight * 0.68));
    var want = lines * lineH + 24; // the layers' 12px of padding, top and bottom
    ed.style.setProperty('--ced-h', Math.min(Math.max(want, min), max) + 'px');
    hl.scrollTop = ta.scrollTop;
    hl.scrollLeft = ta.scrollLeft;
    if (gut) gut.scrollTop = ta.scrollTop;
  }

  // Indent or outdent the lines the selection touches; with no selection,
  // Tab is just two spaces. Without this, Tab leaves the editor entirely.
  function tabKey(ta, outdent) {
    var v = ta.value, s = ta.selectionStart, e = ta.selectionEnd;
    var lineStart = v.lastIndexOf('\n', s - 1) + 1;
    if (s === e && !outdent) {
      ta.value = v.slice(0, s) + '  ' + v.slice(e);
      ta.selectionStart = ta.selectionEnd = s + 2;
      return;
    }
    var lineEnd = v.indexOf('\n', e);
    if (lineEnd === -1) lineEnd = v.length;
    var block = v.slice(lineStart, lineEnd);
    var next = outdent
      ? block.replace(/^[ \t]{1,2}/gm, '')
      : block.replace(/^/gm, '  ');
    ta.value = v.slice(0, lineStart) + next + v.slice(lineEnd);
    ta.selectionStart = lineStart;
    ta.selectionEnd = lineStart + next.length;
  }

  // Enter keeps the current line's indentation, which is most of what an
  // editor's auto-indent is worth in a template.
  function enterKey(ta) {
    var v = ta.value, s = ta.selectionStart;
    var lineStart = v.lastIndexOf('\n', s - 1) + 1;
    var indent = (v.slice(lineStart, s).match(/^[ \t]*/) || [''])[0];
    if (!indent) return false;
    ta.value = v.slice(0, s) + '\n' + indent + v.slice(ta.selectionEnd);
    ta.selectionStart = ta.selectionEnd = s + 1 + indent.length;
    return true;
  }

  var editors = Array.from(document.querySelectorAll('.ced'));
  if (editors.length) {
    // The height cap is a share of the viewport, so it has to be recomputed
    // when the viewport changes.
    window.addEventListener('resize', function () {
      editors.forEach(paintEditor);
    });
  }
  editors.forEach(function (ed) {
    var ta = ed.querySelector('.ced-in');
    if (!ta) return;
    paintEditor(ed);
    ta.addEventListener('input', function () { paintEditor(ed); });
    ta.addEventListener('scroll', function () { paintEditor(ed); });
    ta.addEventListener('keydown', function (e) {
      if (e.key === 'Tab') {
        e.preventDefault();
        tabKey(ta, e.shiftKey);
        paintEditor(ed);
      } else if (e.key === 'Enter' && !e.shiftKey) {
        if (enterKey(ta)) { e.preventDefault(); paintEditor(ed); }
      } else if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        // Reflex from every other editor; without it the browser offers to
        // save the admin page as a file instead.
        var form = ta.form || document.getElementById(ta.getAttribute('form'));
        if (form) { e.preventDefault(); form.requestSubmit(); }
      }
    });
  });
})();

(function () {
  // The Appearance menu in the rail foot. A pick switches immediately and
  // saves in the background; a save that fails is rolled back, the way
  // Ghost's night shift does, rather than leaving the screen and the stored
  // preference disagreeing until the next page load.
  var menu = document.querySelector('.theme-menu');
  if (!menu || !window.ghostApplyTheme) return;
  var root = document.documentElement;
  var switching = null;
  function mark(choice) {
    menu.querySelectorAll('[data-choice]').forEach(function (row) {
      row.classList.toggle('on', row.dataset.choice === choice);
    });
  }
  function settle() {
    // Two frames, Ghost's own cadence: the first lands the new colours, the
    // second has the transitions under way, so lifting the class cannot cut
    // them short.
    if (switching) cancelAnimationFrame(switching);
    switching = requestAnimationFrame(function () {
      switching = requestAnimationFrame(function () {
        root.classList.remove('theme-switching');
        switching = null;
      });
    });
  }
  menu.addEventListener('click', function (e) {
    var row = e.target.closest('[data-choice]');
    if (!row) return;
    var choice = row.dataset.choice;
    var previous = root.dataset.theme;
    if (choice === previous) { menu.removeAttribute('open'); return; }
    root.classList.add('theme-switching');
    root.dataset.theme = choice;
    window.ghostApplyTheme();
    mark(choice);
    menu.removeAttribute('open');
    fetch(menu.dataset.save, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ theme: choice })
    }).then(function (response) {
      if (!response.ok) throw new Error('save failed');
      mark(choice);
    }).catch(function () {
      root.classList.add('theme-switching');
      root.dataset.theme = previous;
      window.ghostApplyTheme();
      mark(previous);
    }).then(settle, settle);
  });
})();
"#;

/// The dashboard appearance a user chose — Ghost's "night shift": `light`,
/// `dark` or `system`, kept in that user's `accessibility` JSON.
///
/// Ghost migrated this field from an on/off boolean (`true` was dark), so both
/// shapes are read; anything missing or unreadable is light, which is Ghost's
/// own default rather than the OS.
pub(crate) fn night_shift(user: Option<&ghost_core::models::User>) -> &'static str {
    let raw = user.and_then(|u| u.accessibility.as_deref());
    let Ok(value) = raw
        .map(serde_json::from_str::<serde_json::Value>)
        .unwrap_or_else(|| Ok(serde_json::Value::Null))
    else {
        return "light";
    };
    match value.get("nightShift") {
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "dark" => "dark",
            "system" => "system",
            _ => "light",
        },
        Some(serde_json::Value::Bool(true)) => "dark",
        _ => "light",
    }
}

/// Fold a night-shift choice into the accessibility JSON, keeping every other
/// `key` — the blob is Ghost's, not ours, and its other keys can grow meaning
/// upstream at any time.
pub(crate) fn merge_night_shift(existing: Option<&str>, theme: &str) -> String {
    let mut map = existing
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    map.insert("nightShift".into(), serde_json::json!(theme));
    serde_json::Value::Object(map).to_string()
}

/// Side navigation. `active` is the nav key of the current page ("" for none),
/// `user` supplies the account block at the bottom of the rail.
pub(crate) fn rail(active: &str, user: Option<&ghost_core::models::User>, role: Role) -> String {
    let active = match active {
        "analytics" | "comments" | "automations" | "forms" => "modules",
        other => other,
    };
    // Showing a link that only leads to "Not allowed" is worse than not
    // showing it, so the rail is filtered by the same capabilities the
    // handlers enforce.
    let items: &[(&str, &str, &str, &str, &str)] = &[
        ("posts", "/ghost/posts/", "Posts", "posts", "blue"),
        ("pages", "/ghost/pages/", "Pages", "pages", "green"),
        ("tags", "/ghost/tags/", "Tags", "tags", "amber"),
        ("themes", "/ghost/themes/", "Themes", "themes", "slate"),
        ("members", "/ghost/members/", "Members", "ghost", "blue"),
        ("modules", "/ghost/modules/", "Modules", "integrations", "purple"),
        // Integrations is not a rail item: like Ghost, it lives under
        // Settings → Integrations.
        ("settings", "/ghost/settings/", "Settings", "settings", "pink"),
    ];
    let mut links: String = items
        .iter()
        .filter(|(key, _, _, _, _)| match *key {
            // Ghost grants the `comment` permission set to Administrator and
            // above only, so moderation is not offered below that.
            "modules" | "themes" | "design" | "navigation" | "members" | "settings" => role.is_admin(),
            "tags" => role.can_manage_tags(),
            _ => true,
        })
        .map(|(key, href, label, ic, tint)| {
            format!(
                r#"<a class="nav-item{cls}" href="{href}"><span class="ic tint-{tint}">{svg}</span><span class="nav-label">{label}</span></a>"#,
                cls = if *key == active { " active" } else { "" },
                href = href,
                tint = tint,
                svg = svg(ic),
                label = esc(label),
            )
        })
        .collect();
    links.push_str(&format!(
        r#"<a class="nav-item" href="/" target="_blank" rel="noopener"><span class="ic tint-blue">{open}</span><span class="nav-label">View site</span></a>"#,
        open = svg("open"),
    ));
    let (initial, name, email) = match user {
        Some(u) => (
            u.name.as_deref().unwrap_or("?").chars().next().unwrap_or('?').to_uppercase().to_string(),
            esc(u.name.as_deref().unwrap_or("Owner")),
            esc(u.email.as_deref().unwrap_or("")),
        ),
        None => ("G".into(), "Owner".into(), String::new()),
    };
    // The avatar and name open the signed-in user's profile, the way Ghost's
    // own account block does. With no session there is no profile to open, so
    // it stays inert rather than linking somewhere that would bounce to
    // signin.
    let who_inner = format!(
        r#"<span class="avatar">{initial}</span><div><b>{name}</b><span>{email}</span></div>"#,
        initial = esc(&initial),
        name = name,
        email = email,
    );
    let who = match user.and_then(|u| u.slug.as_deref()) {
        // The rail is narrow enough that a full name or address is usually
        // ellipsed, so the tooltip carries them rather than restating what
        // the link obviously is.
        Some(slug) => format!(
            r#"<a class="who-link" href="/ghost/settings/staff/{slug}/" title="{name} · {email}">{who_inner}</a>"#,
            slug = esc(slug),
            name = name,
            email = email,
            who_inner = who_inner,
        ),
        None => who_inner,
    };
    // Ghost's appearance switcher offers Dark, Light and System from the
    // account menu and keeps the choice per user. Same trio, same per-user
    // home in the accessibility JSON — only the venue differs, a button
    // beside sign-out, because this rail has no account dropdown.
    let shift = night_shift(user);
    let options: String = [("dark", "Dark", "moon"), ("light", "Light", "sun"), ("system", "System", "monitor")]
        .iter()
        .map(|(value, label, glyph)| {
            format!(
                r#"<button type="button" class="menu-item{on}" data-choice="{value}">{icon} {label}<span class="ck">{check}</span></button>"#,
                on = if *value == shift { " on" } else { "" },
                icon = svg_stroke(glyph),
                label = label,
                check = svg("check"),
                value = value,
            )
        })
        .collect();
    let appearance = format!(
        r#"<details class="theme-menu" data-menu data-save="/ghost/appearance/">
<summary class="icon-btn" title="Appearance">{sun}{moon}</summary>
<div class="pop">
<p class="pop-title">Appearance</p>{options}
</div>
</details>"#,
        sun = svg_stroke("sun"),
        moon = svg_stroke("moon"),
        options = options,
    );
    format!(
        r#"<aside class="rail">
<a class="brand" href="/ghost/analytics/" title="{site_name}">{brand_mark}</a>
<div class="nav-scroll">{links}</div>
<div class="rail-foot">
<div class="who">{who}
{appearance}
<a class="icon-btn" href="/ghost/signout/" title="Sign out">{logout}</a></div>
</div>
</aside>"#,
        brand_mark = admin_brand_mark(),
        site_name = esc(&admin_site_name()),
        links = links,
        who = who,
        appearance = appearance,
        logout = svg("logout"),
    )
}

/// Full HTML document. `nav` pages get the side rail (with the signed-in
/// `user` in the account block); auth/setup pages render bare.
/// Default shell. The role defaults to the *least* privileged, so a page
/// that forgets to say who is looking shows a minimal rail rather than
/// advertising screens the viewer cannot open.
pub(crate) fn page_u(title: &str, active: &str, user: Option<&ghost_core::models::User>, body: &str, nav: bool) -> Html<String> {
    page_for(title, active, user, Role::Contributor, body, nav)
}

/// `page_u` for a handler that already proved the viewer holds `role`.
pub(crate) fn page_for_role(role: Role, title: &str, active: &str, user: Option<&ghost_core::models::User>, body: &str, nav: bool) -> Html<String> {
    page_for(title, active, user, role, body, nav)
}

/// `page_u` with an explicit role, so the rail can be filtered.
pub(crate) fn page_for(title: &str, active: &str, user: Option<&ghost_core::models::User>, role: Role, body: &str, nav: bool) -> Html<String> {
    let shell = if nav {
        format!(r#"<div class="shell">{rail}<main class="main"><div class="content">{body}</div></main></div>"#, rail = rail(active, user, role))
    } else {
        format!(r#"<div class="content" style="max-width:640px;padding-top:40px">{body}</div>"#)
    };
    // Dark renders with the class already on <html>, so a forced-dark user
    // never sees a light flash; `system` is the only choice the server cannot
    // resolve, and the head script settles it before the first paint.
    let theme = night_shift(user);
    let dark = if theme == "dark" { r#" class="dark""# } else { "" };
    Html(format!(
        r#"<!DOCTYPE html><html{dark} data-theme="{theme}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<script>{theme_js}</script>
<title>{title} — {site_name}</title>
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='%23984059'%3E%3Cpath d='M12 2a8 8 0 0 0-8 8v10.6c0 .7.85 1.05 1.35.55l1.9-1.9 2.2 2.2c.3.3.78.3 1.08 0L12 20l1.47 1.45c.3.3.78.3 1.08 0l2.2-2.2 1.9 1.9c.5.5 1.35.15 1.35-.55V10a8 8 0 0 0-8-8z'/%3E%3C/svg%3E">
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Outfit:wght@400;500;600;700&family=Fraunces:opsz,wght@9..144,500..700&family=Roboto+Mono:wght@400;500&display=swap">
<link rel="stylesheet" href="{assets}/admin.css?v={css_hash}"></head><body>{shell}<script>{js}</script><script src="{assets}/publishing-workflows.js" defer></script></body></html>"#,
        dark = dark,
        theme = theme,
        theme_js = THEME_JS,
        title = esc(title),
        site_name = esc(&admin_site_name()),
        css_hash = admin_css_hash(),
        shell = shell,
        js = JS,
        assets = ASSETS,
    ))
}

/// `3 visitors` / `1 visitor` — a count with its noun agreeing.
pub(crate) fn countable(n: i64, singular: &str) -> String {
    if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{} {singular}s", compact(n))
    }
}
