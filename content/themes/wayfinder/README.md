# Wayfinder

A free city-guide and directory theme for Vuuut. Readers search in one
sentence — *Find [restaurants] in [Lisbon]* — and every listing page shows the
address, phone, website and opening hours in a panel, with a live
"Open now, closes at 9:00 PM" status.

Ghost has no custom fields, so Wayfinder reads its directory from things you
can set in the normal editor.

## Before you add listings

Until at least one post is tagged `#listing`, Wayfinder behaves as a blog:
the homepage lists every post, twelve to a page, under the search, and
counts read "posts" rather than "places". As soon as listings appear, the
directory sections take over and other posts move to "Guides and tips".

## Setting up a listing

| What | How |
|---|---|
| It is a listing | Add the internal tag **`#listing`**. Posts without it are guides and appear under "Guides and tips". |
| Category | The **primary tag**: Restaurants, Hotels, Cafés… |
| Place | The **second tag**: New York, Lisbon… |
| Price level | An internal tag **`#price-1`** (inexpensive) to **`#price-4`** (very expensive). |
| Tagline | The post **excerpt**. |
| Details | A **bullet list** of `Label: value` lines anywhere in the post (the first one found). |

### The details list

```
• Address: 475 Driggs Ave, Brooklyn, NY
• Phone: +1 718 555 0142
• Website: hearthandvine.example
• Email: hello@hearthandvine.example
• Mon–Thu: 5:00 pm – 11:00 pm
• Fri–Sat: 5:00 pm – 1:00 am
• Sunday: Closed
• Check-in: From 3 pm
```

- **Address** gets a "Get directions" link, **Phone** a call link,
  **Website** and **Email** links.
- **Hours**: one line per day or range (`Monday`, `Mon–Fri`, `Weekdays`,
  `Weekends`, `Daily`), or a single `Hours: Mon–Fri 9–17; Sat 10–14` line.
  Times can be `9:00–17:00`, `9am–5pm` or `7:00 am - 9:15 pm`; split shifts
  with a comma (`12–3pm, 6–10pm`); `Closed`; `Open 24 hours`. Hours after
  midnight work (`5:00 pm – 1:00 am`).
- Any other label is shown as written.
- "Open now" is worked out in the **site's time zone** (Settings → General),
  since that is where the places are.
- Without JavaScript the list simply stays in the text.

## Theme settings

- **Places**: the slugs of your place tags, comma-separated, e.g.
  `new-york,lisbon,tokyo`. This is how the theme tells places from
  categories. Avoid a slug that is part of another (e.g. `york` and
  `new-york`).
- **Directory page**: address of the page using the **All places**
  template (default `/places/`). Create a page with that slug and template.
  (`/directory/` is taken by Vuuut's built-in Directory app.)
- **Add a place URL**: shows an "Add a place" button and invitation; point it
  at a form or contact page. Empty hides them.
- **Signup heading / text**, **Color scheme**, and homepage/post toggles.

## Templates

| Template | Used for |
|---|---|
| `index.hbs` | Sentence search, categories, featured listings, places, new listings, guides |
| `post.hbs` | Listings (`#listing`) get the listing layout with the details panel; other posts read as articles |
| `custom-all-places.hbs` | **All places**: every listing, filtered by category, place and price, with shareable addresses (`?what=&where=&price=`) |
| `tag.hbs` | Category or place, with "in Lisbon" chips that narrow a category to a place |
| `page.hbs`, `author.hbs`, `error-404.hbs`, `error.hbs` | As usual; the 404 offers the search |

Vuuut's store, built-in directory and page builder render inside pages at
full width, with their own type.

## Engine notes

This engine can't filter tags by a list of slugs (`slug:[a,b]`) or negate a
tag filter (`tag:-x`), so the theme fetches all public tags and sorts them
with the Places setting, and skips listings in loops. Both work as written
on Ghost too.

## Fonts

Overpass (SIL Open Font License), modelled on American highway signage,
bundled as a variable latin subset.

Design system: [DESIGN.md](DESIGN.md).
