# Wire design system

Wire is a news theme. News is about *when*, so time is the organising idea
and the **timeline** is the theme's one signature element. Everything else
is plain and dense, so a reader can scan it.

## Principles

1. **Time is the structure.** Every story carries its time. The timeline
   shows the order the news arrived in, beside the lead and beside every
   article.
2. **Red means new.** `--live` marks stories under three hours old and
   nothing else. It does not follow the site accent, so "new" reads the same
   on every Wire site.
3. **One family, two voices.** Archivo compressed (`font-stretch: 72%`,
   weight 800+) is the headline voice. Archivo at normal width is the
   reading voice. No second typeface.
4. **Dense, not cramped.** Headlines sit tight (line-height around 1). The
   reading column gets room (18px, line-height 1.72, 43rem measure).
5. **Labels read as prose.** Topic in the accent colour, then the time.
   No all-caps labels, no dot-joined meta, no invented numbers (follower
   counts, view counts).

## Tokens

All tokens are custom properties on `:root` in `assets/css/screen.css`.
Dark values switch on `[data-scheme="dark"]`, or `auto` with a dark device.

### Colour

| Token | Light | Dark | Role |
|---|---|---|---|
| `--paper` | `#fafafb` | `#12141a` | Page background |
| `--surface` | `#ffffff` | `#1a1d25` | Timeline panel |
| `--sunk` | `#eef0f3` | `#1d2029` | Editor's choice band, tag chips, image placeholders |
| `--ink` | `#15171c` | `#eceef2` | Text, desk rules |
| `--ink-soft` | `#3b3f4a` | `#c6cad3` | Standfirsts, excerpts |
| `--muted` | `#5c6270` | `#9096a3` | Times, bylines, timeline stops |
| `--rule` | `#dfe2e8` | `#2a2e38` | Dividers, the timeline line |
| `--red` | `#d4202c` | `#ff5a5f` | Default accent |
| `--accent` | site accent, else `--red` | same | Buttons, nav underline, pull-quote bar |
| `--accent-text` | `--accent` | accent mixed 62% with white | Accent used as text |
| `--live` | `#d4202c` | `#ff5a5f` | "New" on the timeline (fixed) |
| `--band` | `#15171c` | `#1d2029` | Signup band, code blocks |

### Type

| Token | Size | Use |
|---|---|---|
| `--step--1` | 13px | Times, kickers, timeline topics |
| `--step-0` | 15px | Interface |
| `--step-1` | 18px | Reading text |
| `--step-2` | 21px | Grid headlines, standfirsts |
| `--step-3` | 28px | Timeline heading, pull quotes |
| `--step-4` | 32→40px | Section and desk names, content h2 |
| `--step-5` | 40→72px | Lead, article and signup headlines |
| `--step-6` | 48→112px | Topic page title (at `font-stretch: 62%`) |

Widths: `--compressed` (72%) for headline voice, `--condensed` (86%) for
card and timeline titles, 100% for reading.

### Space, shape, motion

- Space runs on a 4px base: `--s-1` 4 through `--s-9` 104.
- Radius has three steps. `--radius-s` (6px) is for row thumbnails,
  `--radius-m` (10px) for grid images, and `--radius-l` (16px) for lead
  images and the timeline panel. Pills are for buttons, the signup field
  and tag chips.
- Motion: the newest fresh stop pulses (2.2s loop). That is the only motion
  that isn't triggered by the reader. Image zoom on hover is the only other
  effect. Reduced motion removes both.

## Components

| Component | Partial | Notes |
|---|---|---|
| Timeline | `partials/timeline-item.hbs` inside `ol.tl[data-timeline]` | Stops hang off a 2px line. `.is-fresh` (set by wire.js) turns the time and dot red; the first fresh stop pulses. |
| Time | `partials/when.hbs` | `<time datetime="…Z" data-when>`. The server writes "24 Nov, 12:06"; wire.js rewrites it as relative time. |
| Card | `partials/card.hbs` | `size="lead"` (16:9, compressed 72px headline, excerpt, faces), `"row"` (thumbnail beside text), `"line"` (headline only), default (grid). |
| Desk | `partials/desks.hbs` | Tag name over a 2px ink rule, "All N stories", one row and three lines. |
| Editor's choice | `partials/editors-choice.hbs` | Receives `lead=id` so it never repeats the lead. One large story, three rows; rows top up with unfeatured stories and CSS caps them at three. |
| Dateline | `partials/header.hbs` | Today's date in the reader's time zone, then the eight biggest tags in a horizontally scrolling row. |
| Signup | `partials/signup.hbs` | Ink band, white pill field, accent button. Shown to signed-out visitors when signups are open. |

## Do and don't

| Do | Don't |
|---|---|
| Show a time on every story | Hide the date to look "evergreen" |
| Keep `--live` for things under three hours old | Use red for decoration or emphasis |
| Use the compressed width for headlines only | Compress body text or labels |
| Separate list items with `--rule` | Box every story in its own card; the timeline is the only panel |
| Let the reader's locale format times | Hard-code "ago" strings or 12/24-hour formats |
