# Margin

A literary blog theme for Vuuut. A cover story leads into the newest writing, with dates in the left margin and a flexible sidebar.

## What it includes

- Dated entries in the margin. On phones the date folds onto one line above the title.
- In **Excerpt** mode, the newest post leads the first homepage. Margin uses the post's feature image or its bundled reading-room photograph. Turn off **Show cover story** to use the dated feed alone, or turn off **Show feature images in list** for a text-only cover.
- A sidebar on the right, the left, or turned off. It holds: about, email signup, recent posts, topics with post counts, a blogroll (from Recommendations), and RSS.
- An option to show full posts on the homepage instead of excerpts.
- Two page templates: **Archive**, which lists the 100 newest posts by date, and **Full width**, which has no sidebar.
- Members: sign in, subscribe, account link, and a paywall for paid, members-only, and tier-only posts.
- Native comments, matched to the color scheme.
- Previous and next post links, related posts by primary tag, and an author bio.
- Light, dark, or follow the reader's device. Serif or sans for headings and body. Honors the site's brand fonts and accent color.
- Koenig cards: wide cards reach into the date margin, and full-width cards go edge to edge when there is no sidebar.

## Sample content

Import [margin-demo.json](demo/margin-demo.json) on a test publication to
preview nine essays, field notes, and reading entries. In Vuuut, use
**Settings → Advanced → Import content**. The CLI import can also load the
sample menus when run with `--with-settings`.

## Fonts

Literata and Instrument Sans are self-hosted in `assets/fonts`. Both are under the SIL Open Font License.

## Checking it

```
cargo run --release --example gscan-report -p vuuut-render -- content/themes/margin
cargo run --release --example compile-theme -p vuuut-render -- content/themes/margin
```
