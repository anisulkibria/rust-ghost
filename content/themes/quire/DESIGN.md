# Quire design system

Quire is for a publication with many writers and many topics. The navy **topic sentence** opens the front page. A photographic lead,
companion stories, and a reading list give the issue its structure.

## Principles

1. **Topics are the way in.** Readers arrive wanting a subject, not a date.
   The topic list is the hero, every card leads with its topic, and every
   topic page ends by offering the others.
2. **Bylines are first-class.** Many people write here, so faces and names
   appear on cards, in the byline, at the end of a post and in their own
   section.
3. **One mark, used sparingly.** The highlighter yellow means "this one":
   the active topic, a hovered title, or selected text.
4. **Didone for voice, grotesk for work.** Bodoni Moda speaks (titles,
   topics, section names); Hanken Grotesk does the reading and the
   interface.
5. **Labels read as prose.** Meta is written as a phrase ("22 Nov 2026,
   2 min read"). No all-caps eyebrows and no dot-joined strings.

## Tokens

All tokens are custom properties on `:root` in `assets/css/screen.css`.
Dark values switch on `[data-scheme="dark"]`, or `auto` with a dark device.

### Colour

| Token | Light | Dark | Role |
|---|---|---|---|
| `--paper` | `#f8f6ef` | `#10131d` | Page background |
| `--paper-sunk` | `#f0eee6` | `#1a1f2e` | Picks band, author cards, image placeholders |
| `--ink` | `#161b2e` | `#e7eaf3` | Text |
| `--ink-soft` | `#3a4157` | `#c4c9d8` | Excerpts, standfirsts |
| `--muted` | `#5f667d` | `#9aa1b7` | Meta, intro words, counts' labels |
| `--rule` | `#ddd9cd` | `#2b3144` | Dividers between items, never as decoration |
| `--blue` | `#2743c4` | `#8ea2ff` | Default accent (royal ink) |
| `--accent` | site accent, else `--blue` | same | Buttons |
| `--accent-text` | accent mixed with ink | accent mixed 52% with white | Accent used as text: topic labels, counts, links' underline |
| `--mark` | `#ffe14d` | `#e9cd3a` | Highlighter, see principle 3 |
| `--band` | `#1c2547` | `#1c2547` | Signup band, code blocks, initial discs |

### Type

| Token | Size | Use |
|---|---|---|
| `--step--1` | 14px | Meta, labels |
| `--step-0` | 16px | Interface |
| `--step-1` | 19px | Reading text (line-height 1.72) |
| `--step-2` | 22px | Card titles, standfirsts |
| `--step-3` | 28px | Side-card titles, picks, author names |
| `--step-4` | 36px | Section titles (italic), content h2 |
| `--step-5` | 36→52px | Wide card title, signup heading |
| `--step-6` | 40→76px | Post title, writer name |
| `--step-7` | 42→92px | Topic sentence |

Display sizes use Bodoni Moda's optical-size axis
(`font-optical-sizing: auto`), so the hairlines thicken at small sizes.

### Space, shape, motion

- Space runs on a 4px base: `--s-1` 4 through `--s-9` 112. Sections are
  separated by `--s-9` and items by `--s-6` to `--s-8`.
- Radius has three steps. `--radius-s` (3px) is for code, `--radius-m` (9px)
  for images, and `--radius-l` (16px) for cards. Pills are for
  buttons and tag chips only.
- Motion uses `--ease` (`cubic-bezier(.22,.8,.24,1)`). The only motion that
  isn't triggered by the reader is the first topic's mark sweeping in
  (`--mark-time`, 520ms). Everything else responds to the reader. Reduced
  motion turns it all off.

## Components

| Component | Partial | Notes |
|---|---|---|
| Topic sentence | `partials/topics.hbs` | Up to five public topics with posts, by size. Hover or focus updates the newest-story preview, which is a link on desktop and mobile. |
| Card | `partials/card.hbs` | `size="wide"` (lead), `size="side"` (compact row with square thumbnail), or default (grid). Topic, title, excerpt, faces, authors, date, reading time. |
| Pick | `partials/picks.hbs` | Featured posts as a two-column reading list with small square images. |
| Writer | `partials/writer.hbs` | Portrait or initial disc, name, story count when the engine supplies `count.posts`. |
| Byline | `post.hbs` | Overlapping faces (up to 3), linked names, date and reading time. |
| Author card | `partials/article-footer.hbs` | One per author, on `--paper-sunk`. |
| Signup | `partials/signup.hbs` | Navy band, white pill field, highlighter button. Shown to signed-out visitors when signups are open. |

## Do and don't

| Do | Don't |
|---|---|
| Use `--mark` to say "this one" | Use yellow for backgrounds, badges or decoration |
| Put the accent on topic labels and counts | Colour whole headlines |
| Use `--rule` for card edges and separators | Add heavy shadows under story cards |
| Write meta as a phrase | Join meta with `·` or put labels in capitals |
| Let titles balance (`text-wrap: balance`) | Truncate titles; truncate excerpts at 3 lines instead |
