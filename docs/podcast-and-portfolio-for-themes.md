# Podcast and Portfolio: what a theme gets

Tools → Podcast and Tools → Portfolio render their own pages inside any
theme's `page.hbs`, so every theme works with them unchanged. A theme built
for a podcast or a portfolio can take over any screen by shipping a template
of the same name, and can show the tools' content anywhere with `{{#get}}`.
This page lists both.

`podcast` and `portfolio` (below) are on **every** page while the tool is on,
not only the tool's own — so a theme's `home.hbs`, header and footer can
link to the show or the work, and load `{{podcast.script_url}}` for the
player. `{{#if podcast}}` is the way to ask whether the tool is on. The
bundled themes **Earshot** (podcast) and **Studiolo** (portfolio) are worked
examples of everything here.

Both tools are off until switched on in their Settings; while off, every
address below is the theme's 404 and every `{{#get}}` below returns nothing,
so the theme's `{{else}}` shows.

## Podcast

Public addresses (the base `podcast` can be changed in Settings):

| Address | Template | Data |
|---|---|---|
| `/podcast/` | `podcast/index.hbs` | `podcast`, `shows`, `episodes` (latest 8). With exactly one show this address renders that show instead (`podcast/show.hbs`). |
| `/podcast/{show}/` | `podcast/show.hbs` | `podcast`, `show`, `episodes` (this page), `latest`, `trailer`, `seasons`, `reviews`, `pagination`, `private_url`, `contact_url` |
| `/podcast/{show}/{episode}/` | `podcast/episode.hbs` | `podcast`, `show`, `episode` (with `transcript` cues when the visitor may listen), `older`, `newer`, `signin_url`, `upgrade_url` |
| `/podcast/people/{slug}/` | `podcast/person.hbs` | `podcast`, `person`, `shows`, `episodes` (each with `role`) |
| `/podcast/{show}/feed.xml` | — | The RSS feed (iTunes + Podcasting 2.0 tags) |
| `/podcast/{show}/contact/` | — | Guest pitches and listener questions |
| `/podcast/{show}/private/` | — | A signed-in member's private feed address |

`podcast` carries `title`, `intro`, `url`, `style_url`, `script_url`.

An **episode** (`vuuut_db::repos::podcast::episode_json`): `title`, `url`,
`summary`, `artwork`, `label` ("S2 · E14"), `season`, `number`, `trailer`,
`bonus`, `explicit`, `members_only`, `can_listen`, `audio_url` (only when the
visitor may listen — it goes through the download counter), `duration`,
`duration_label`, `clock`, `date_label`, `published_at`, `notes` (HTML),
`video_embed.url`, `chapters[]` (`start`, `time`, `title`, `url`),
`transcript_url`, `people[]`, `guests[]`, `sponsors[]` (`name`, `url`,
`logo`, `promo_code`), `show.title`, `show.url`, `downloads`.

A **show**: `title`, `subtitle`, `description`, `description_html`,
`artwork`, `banner`, `author`, `category_label`, `explicit`, `serial`, `url`,
`feed_url`, `links[]` (`label`, `url` — Apple Podcasts, Spotify, …),
`funding` (`label`, `url`), `hosts[]`, `episode_count`.

`{{#get}}` resources: `podcasts`, `episodes` (`order="oldest"|"popular"`,
filter e.g. `show.slug:my-show`, `featured:true`, `season:2`),
`podcast_people`.

### The player

Load `{{podcast.script_url}}` (and `{{podcast.style_url}}` for the default
look) and mark elements up; the script does the rest — one dock per page,
speed, ±15/30 s, resume where the listener stopped, chapter highlighting,
`#t=12:30` links, the lock screen's controls:

```hbs
<button data-vp-play data-id="{{episode.id}}" data-src="{{episode.audio_url}}?s=web"
  data-title="{{episode.title}}" data-show="{{show.title}}" data-art="{{episode.artwork}}"
  data-url="{{episode.url}}" data-dur="{{episode.duration}}">Play</button>

{{#foreach episode.chapters}}<button data-vp-seek="{{start}}">{{time}}</button> {{title}}{{/foreach}}
```

`?s=web` counts the listen as "This site's player" in Analytics.

## Portfolio

Public addresses (the base `portfolio` can be changed — `work` is common):

| Address | Template | Data |
|---|---|---|
| `/portfolio/` | `portfolio/index.hbs` | `portfolio`, `projects`, `featured`, `categories`, `services`, `clients`, `testimonials` |
| `/portfolio/category/{slug}/` | `portfolio/category.hbs`, else `index` | the same, plus `category` |
| `/portfolio/{slug}/` | `portfolio/project.hbs` | `portfolio`, `project`, `prev`, `next`, `related`, `forms.unlock` |
| `/portfolio/services/` | `portfolio/services.hbs` | `portfolio`, `services` |
| `/portfolio/services/{slug}/` | `portfolio/service.hbs` | `portfolio`, `service`, `projects` |
| `/portfolio/about/` | `portfolio/about.hbs` | `portfolio`, `team`, `entries`, `skills`, `clients`, `testimonials`, `stats` |
| `/portfolio/resume/` | `portfolio/resume.hbs` | `portfolio`, `entries`, `skills` |
| `/portfolio/team/{slug}/` | `portfolio/member.hbs` | `portfolio`, `member`, `projects` |
| `/portfolio/contact/` | `portfolio/contact.hbs` | `portfolio`, `forms.contact` (the form, ready-made), `forms.sent`, `forms.error` |

`portfolio` carries the profile: `name`, `headline`, `intro`, `about`,
`about_html`, `portrait`, `location`, `studio` / `personal`, `available`,
`availability_label`, `availability_note`, `email`, `phone`, `resume_url`,
`founded`, `socials[]` (`label`, `url`), and `url`, `about_url`,
`resume_page_url`, `services_url`, `contact_url`, `style_url`, `script_url`.

A **project** (`vuuut_db::repos::portfolio::project_json`): `title`,
`subtitle`, `summary`, `url`, `cover`, `images[]`, `gallery[]`, `client`,
`client_logo`, `year`, `duration`, `role`, `industry`, `location`, `tools[]`,
`categories[]`, `category_slugs` (space-separated, for filter attributes),
`primary_category`, `accent`, `layout`, `featured`, `protected`, `locked`,
`body` (HTML), `video_embed.url`, `live_url`, `repo_url`, `metrics[]`
(`value`, `label`), `credits[]` (`name`, `role`, `url`), `testimonial`,
`views`. A password-protected project the visitor hasn't unlocked comes with
`locked: true` and no body, gallery, video, links, results or credits at all —
render `{{{forms.unlock}}}` instead.

`{{#get}}` resources: `projects` (`order="newest"|"popular"|"title"|"year"`;
filter e.g. `featured:true`), `project_categories`, `portfolio_services`,
`clients`, `testimonials`, `team_members`, `resume` (filter `kind:work`,
`kind:education`, `kind:award`…), `skills`.

`portfolio.js` adds in-place filtering to `.vf-chip[data-filter]` links over
`.vf-card[data-cats]` inside `[data-vf-grid]`, a lightbox to
`[data-vf-gallery] a[data-vf-zoom]`, and `[data-vf-print]`.
