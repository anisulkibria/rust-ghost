# Wire

A free news theme for Vuuut, organised by time. The newest story leads the homepage, and the
ones after it run down a timeline beside it: "The day so far". Each stop
shows how long ago the story went up ("12 min ago", "Yesterday, 9:40 PM"),
and stories under three hours old are marked in red, the newest with a
pulse. The same timeline sits beside every article as "Latest".

## Templates

| Template | Used for |
|---|---|
| `index.hbs` | Homepage: dateline, lead story with the timeline, Editor's choice, topic desks, more stories |
| `post.hbs` | Posts: compressed headline, standfirst, byline with time, feature image, "Latest" timeline beside the text |
| `page.hbs` | Pages, honouring the "show title and feature image" toggle |
| `tag.hbs` | Topic pages: lead story and timeline for that topic, then a grid |
| `author.hbs` | Writer pages with portrait, bio and links |
| `error-404.hbs`, `error.hbs` | Not found (with the latest stories) and server errors |

## How the homepage fills

- **Lead**: the newest post.
- **The day so far**: the next five posts.
- **Editor's choice**: a large story and three beside it, all from your
  **featured** posts (never the lead). If fewer than four are featured, the
  latest unfeatured stories fill the three rows.
- **Desks**: your four biggest public tags, each with its newest story and
  three more headlines.
- **More stories**: the rest of the page, 16 posts in all, then pagination.

The dateline under the masthead shows today's date in the reader's own time
zone, followed by your eight biggest tags.

## Theme settings

- **Color scheme**: light, dark, or follow the reader's device. Readers can
  switch with the button in the header either way.
- **Timeline title**: "The day so far" by default.
- **Signup heading / text**: "The morning briefing" and your site description
  by default.
- **Homepage**: Editor's choice, desks.
- **Post**: the "Latest" timeline beside posts, author cards, related stories.

The site accent colour (**Settings → Design → Brand**) replaces the default
red for buttons, topic labels and links. The "new" signal on the timeline
stays red whatever the accent, because it means something. Ghost's heading
and body font settings replace Archivo.

## Times

Times are written by the server in the site's time zone, so they read
correctly without JavaScript. `assets/js/wire.js` then rewrites them as
relative times in the reader's language (`Intl.RelativeTimeFormat`) and
refreshes them every minute.

## Fonts

Archivo, SIL Open Font License, bundled as a variable latin subset with its
width axis in `assets/fonts`, so the theme makes no third-party requests.

## Design system

Tokens, type scale and component rules are in [DESIGN.md](DESIGN.md).
