# Handbook

A free documentation and knowledge-base theme for Vuuut. Sections and
articles are numbered like a manual — section 2, article 2.3 — and the
number follows the article everywhere: the contents tree, breadcrumbs, the
large numeral above the title, section pages and previous/next links. Readers
can tell each other "see 2.3".

## How it is organised

| What | How |
|---|---|
| Article | A post. |
| Section | The article's **primary tag**. The tag description becomes the section's summary. |
| Article order | **Oldest first** within a section. To move an article, change its publish date. |
| Section order | The **Sections** setting: tag slugs, in order, comma-separated, e.g. `getting-started,publishing,account`. Tags not listed are left out. Empty shows every tag, alphabetically. |

## Pages

| Template | Used for |
|---|---|
| `index.hbs` | Heading and search, a card per section (number, summary, first five articles), recently updated, support |
| `post.hbs` | Contents tree (left), article, "On this page" (right), "Was this page helpful?", previous/next within the section |
| `tag.hbs` | A section: its number, summary and every article in reading order |
| `page.hbs`, `author.hbs`, `error-404.hbs`, `error.hbs` | As usual; the 404 offers search and the contents |

Vuuut's store, built-in directory and page builder render inside pages at
full width with their own type.

## Reading aids

- Press **/** anywhere to search.
- **On this page** lists the article's h2 and h3 and marks the one in view.
- Hovering a heading shows a **#** link to that heading.
- The current article is marked in the tree and its section stays open.
- **Was this page helpful?** remembers the answer in the reader's browser.
  "No" offers the support link. Answers are not collected anywhere.

## Theme settings

- **Sections**: which tags are sections, and their order.
- **Hero heading**: defaults to "How can we help?".
- **Support URL**: a contact page or `mailto:` link. Adds "Contact support"
  to the header, the homepage and the feedback answer. Empty hides them.
- **Color scheme**: follows the reader's device by default.
- **Show recent**, **Show signup** (product updates, off by default),
  **Show feedback**.

## Writing tips

- Use h2 for steps or topics and h3 inside them; they build "On this page".
- Numbered lists show their numbers in the accent colour: use them for steps.
- Ghost's callout cards make good notes and warnings.

## Fonts

IBM Plex Sans and IBM Plex Mono (SIL Open Font License), bundled as latin
subsets. Mono is used for code only.

Design system: [DESIGN.md](DESIGN.md).
