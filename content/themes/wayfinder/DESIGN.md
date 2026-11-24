# Wayfinder design system

Wayfinder is a city guide. Its job is to help someone decide where to go, so
the one bold element is the **sentence search** — the question a visitor
already has, set in large type, with the blanks as the controls. Everything
else is calm and signpost-plain.

## Principles

1. **Ask the visitor's question.** "Find [restaurants] in [Lisbon]" reads as a
   sentence and works as a form. One choice opens that tag; two open the All
   places page already filtered.
2. **Facts over flourish.** Address, hours, phone and price are what people
   come for. They get a panel of their own and a live open/closed status.
3. **Signage type.** Overpass, after US highway signage, set heavy and tight
   for headings, plain for reading. One family.
4. **Prose labels.** Cards say "Restaurants in New York", not
   `RESTAURANTS · NEW YORK`. Price shows the unused signs faint ($$$$).
5. **Colour means something.** Pin pink is for action (buttons, the search
   blanks, links); park green is "open"; brick red is "closed".

## Tokens

All in `:root` in `assets/css/screen.css`; dark values switch on
`[data-scheme="dark"]` or `auto` with a dark device.

### Colour

| Token | Light | Dark | Role |
|---|---|---|---|
| `--paper` | `#f2f4f0` | `#0f1b19` | Page (pale map paper) |
| `--surface` | `#ffffff` | `#152522` | Facts panel, chips, card badges |
| `--sunk` | `#e6ebe4` | `#1a2d29` | Invitation band, image placeholders |
| `--ink` | `#12302b` | `#e7efec` | Text (spruce) |
| `--ink-soft` / `--muted` | `#3d5650` / `#5f716c` | `#bfd0cb` / `#8fa5a0` | Taglines / meta |
| `--rule` / `--faint` | `#d5ddd6` / `#b9c6c0` | `#263b37` / `#3b524d` | Borders / unused price signs |
| `--park` (+`-tint`) | `#2e7656` | `#5cc08f` | Open now, fact icons, list markers |
| `--stop` (+`-tint`) | `#b23a3a` | `#ff8a80` | Closed now |
| `--pin` → `--accent` | `#e23a6c` | same | Actions; replaced by the site accent |
| `--accent-text` | accent darkened 18% | accent lightened | Accent used as text |
| `--band` | `#12302b` | `#1a2d29` | Signup band, code |

### Type

| Token | Size | Use |
|---|---|---|
| `--step--1` | 13px | Meta, badges, fact labels |
| `--step-0` | 15px | Interface |
| `--step-1` | 17px | Reading text (line-height 1.7) |
| `--step-2` | 20px | Card titles, taglines |
| `--step-3` | 26px | Section subheads, place names |
| `--step-4` | 30→40px | Section titles |
| `--step-5` | 38→68px | Page and listing titles |
| `--step-6` | 38→84px | The sentence search |

Headings 800–850 weight, letter-spacing −0.02 to −0.03em.

### Space, shape, motion

- 4px base: `--s-1` 4 … `--s-9` 104.
- Radius by size: `--radius-s` 8px (thumbnails), `--radius-m` 16px (cards,
  facts panel), `--radius-l` 24px (hero postcard, place tiles, bands); pills
  for buttons, chips, filters.
- Motion only answers the reader: image zoom on hover, the search blanks
  resizing to their choice. Reduced motion removes the zoom.

## Components

| Component | Where | Notes |
|---|---|---|
| Sentence search | `partials/finder.hbs` | Selects sized to their text (`field-sizing: content`, JS fallback), caret drawn in CSS, labels hidden but present. |
| Listing card | `partials/listing-card.hbs` | 4:3 photo, Featured badge, "Category in Place", title, tagline, price. Carries tag classes for filtering. |
| Article card | `partials/article-card.hbs` | For guides: 16:10 photo, topic, title, byline. |
| Category chip | `partials/kinds.hbs` | Round tag image or initial, name, count. |
| Place tile | `partials/places.hbs` | Photo tile; first is large; grid adapts to 1–4 places. |
| Facts panel | `partials/listing.hbs` + `wayfinder.js` | Built from the post's `Label: value` list: address + directions, phone, website, email, other facts, hours table with today in bold. |
| Status | `[data-status]` | "Open now, closes at 9:00 PM" / "Closed now, opens at 5:00 PM tomorrow" / "Open 24 hours", in the site's time zone. |
| Filters | `custom-all-places.hbs` | Category and place selects, price toggles, count, shareable address, empty state. |

## Do and don't

| Do | Don't |
|---|---|
| Write facts as `Label: value` bullets | Put hours in a paragraph; the panel can't read it |
| Keep green and red for open and closed | Use them for decoration |
| Let the photo carry the card | Add borders and shadows around listing cards |
| Name a place tag in the Places setting | Use a slug that is part of another |
