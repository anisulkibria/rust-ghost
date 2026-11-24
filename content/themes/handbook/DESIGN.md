# Handbook design system

Handbook is for documentation people use while they are in the middle of
something. Its one distinctive element is **manual numbering**: a fixed
address for every section and article, shown the same way everywhere.
Everything else is quiet, dense and predictable.

## Principles

1. **Every page has an address.** Section 2, article 2.3. The numbers come
   from one source (the contents tree, ordered by the Sections setting) so
   they never disagree.
2. **You are here.** The current article is marked in the tree, its section
   stays open, the breadcrumb carries the numbers, and "On this page" tracks
   the heading in view.
3. **Read, then act.** A 46rem column at 17px/1.72 for reading; steps as
   numbered lists; code in Plex Mono on a dark panel in both schemes.
4. **One accent.** Indigo (or the site accent) marks numbers, links and the
   current place. Nothing decorative uses it.
5. **Plain labels.** Sentence case, prose meta ("Updated 5 September 2026,
   2 min read"), no capitals or dot-joined strings.

## Tokens

In `:root` in `assets/css/screen.css`; dark values switch on
`[data-scheme="dark"]` or `auto` with a dark device (the default).

### Colour

| Token | Light | Dark | Role |
|---|---|---|---|
| `--paper` | `#ffffff` | `#111318` | Page |
| `--panel` | `#f6f7f9` | `#181b22` | Hero, search field, feedback, hover rows, table heads |
| `--ink` | `#16181d` | `#e8eaef` | Text |
| `--ink-soft` / `--muted` | `#3f4450` / `#6b7080` | `#c2c6d0` / `#8a90a0` | Secondary text / meta and numbers |
| `--rule` | `#e5e7eb` | `#262a33` | Borders, tree line, table cells |
| `--accent` | `#4f46e5` | same | Buttons, current markers; replaced by the site accent |
| `--accent-text` | accent darkened 12% | accent lightened | Accent as text (numbers, links) |
| `--accent-tint` | accent 9% | accent 18% | Current article background in the tree |
| `--code-bg` / `--code-ink` | `#16181d` / `#e8eaef` | `#0b0d11` / same | Code blocks |

### Type

IBM Plex Sans for everything, IBM Plex Mono for code and keys only.

| Token | Size | Use |
|---|---|---|
| `--step--1` | 13px | Meta, breadcrumbs, "On this page", tree numbers |
| `--step-0` | 15px | Interface, tree, cards |
| `--step-1` | 17px | Reading |
| `--step-2` | 20px | Standfirst, card names, h3 |
| `--step-3` | 24px | Section titles on the homepage |
| `--step-4` | 28→36px | h2 |
| `--step-5` | 34→52px | Page and article titles |
| `.doc-num` | 40→56px, weight 200 | The number above the title |

### Space, shape, layout

- 4px base: `--s-1` 4 … `--s-9` 96.
- Radius: `--radius-s` 6px (buttons, inputs, tree rows), `--radius-m` 12px
  (cards, callouts, code, feedback).
- Article layout: tree 17rem, article up to 46rem, "On this page" 13rem.
  Below 1240px the right column goes; below 900px the tree folds behind a
  Contents button.
- Header 4rem, sticky, translucent.

## Components

| Component | Where | Notes |
|---|---|---|
| Contents tree | `partials/tree.hbs`, `tree-section.hbs` | `<details>` per section; numbers written by `handbook.js`; current article gets `aria-current="page"`, a tint and a 2px accent bar on the tree line. |
| Number | `.num`, `[data-num-for="url"]` | Any element with `data-num-for` receives that page's number. Empty numbers take no space. |
| Section card | `partials/section-card.hbs` | Light numeral, name, summary, first five articles, "All N articles". Two or four cards sit in pairs. |
| Article row | `partials/article-row.hbs` | Number, title, excerpt, reading time or "updated" date. |
| Search field | header and hero | A button styled as a field that opens Ghost search; `/` opens it too. |
| On this page | `post.hbs` + JS | h2/h3 of the article; current heading marked by IntersectionObserver. |
| Feedback | `post.hbs` + JS | Yes/No, remembered in the browser, "No" links to support. |
| Previous/next | `{{prev_post in="primary_tag"}}` | Within the section, with numbers. |

## Do and don't

| Do | Don't |
|---|---|
| Order sections in the Sections setting | Rename tags to force an order ("1. Getting started") |
| Reorder articles by publish date | Put numbers in titles; the theme adds them |
| Use h2/h3 for structure | Skip from h2 to h4; "On this page" reads h2 and h3 |
| Keep accent for numbers and the current place | Colour headings or backgrounds with it |
