# Paperback

A free magazine theme for Vuuut. Every story opens like a paperback cover: a
coloured band names the section, a tinted panel holds the title, a thinner band
closes it.

## Section colours

The colour of every cover, card and shelf comes from the post's **primary
tag**. Set it in **Tags → (tag) → Accent colour**. Tags without one use the
site accent colour from **Settings → Design → Brand**. Text on a band switches
between black and white to stay readable on whatever colour you pick.

## Templates

| Template | Used for |
|---|---|
| `index.hbs` | Homepage: cover story, latest posts, section shelves |
| `post.hbs` | Posts: banded cover, feature image below |
| `custom-image-cover.hbs` | Post template "Image cover": feature image inside the cover |
| `page.hbs` | Pages, honouring the "show title and feature image" toggle |
| `tag.hbs`, `author.hbs` | Archives with their own cover |
| `error-404.hbs`, `error.hbs` | Not found and server errors |

## Theme settings

- **Color scheme**: follow the reader's device, light or dark
- **Title font / Body font**: Jost (sans) or Newsreader (serif) for each;
  Ghost's own font settings in Brand override both
- **Signup heading / text**: copy for the signup band on every page
- **Homepage**: cover story, section shelves for the three biggest tags,
  images on cards
- **Post**: author card, three related posts from the same tag

## Fonts

Jost and Newsreader, both SIL Open Font License, bundled as latin subsets in
`assets/fonts`, so the theme makes no third-party requests.
