# Quire

A free theme for publications with many writers and many topics. A quire is
a gathering of pages bound together; this theme is for sites where many
people write about many things.

The homepage opens with your topics, biggest first, set as one sentence:
*Stories on Design⁴, Engineering⁴, Culture³ and News¹.* Pointing at a topic,
or tabbing to it, sweeps a highlighter across it and shows its newest story
beside the sentence.

## Templates

| Template | Used for |
|---|---|
| `index.hbs` | Homepage: topic sentence, lead story with three more, recent stories, editors' picks, writers |
| `post.hbs` | Posts: topic, title, standfirst, byline with every author, wide feature image |
| `page.hbs` | Pages, honouring the "show title and feature image" toggle |
| `custom-all-topics.hbs` | Page template **All topics**: every topic with its newest three stories |
| `custom-all-writers.hbs` | Page template **All writers**: everyone who has published |
| `tag.hbs` | Topic archive, ending with the other topics as a sentence |
| `author.hbs` | Writer archive with portrait, bio and links |
| `error-404.hbs`, `error.hbs` | Not found (offers the topic sentence) and server errors |

## Setting up

1. Give every post a **primary tag**. The tag is the topic shown on cards and
   counted in the homepage sentence. Tags with no posts are left out.
2. Mark a few posts **Featured** to fill the editors' picks.
3. Optional: create a page called *Topics* with the template **All topics**,
   and one called *Writers* with **All writers**, then add both to the
   navigation.
4. Writers' portraits come from **Staff → (person) → Profile picture**.
   Anyone without one gets their initial on a navy disc.

## Theme settings

- **Color scheme**: light, dark, or follow the reader's device. Readers can
  switch with the button in the header either way.
- **Topics intro**: the words before the list, "Stories on" by default.
- **Signup heading / text**: copy for the signup band above the footer.
- **Homepage**: topic sentence, editors' picks, writers.
- **Post**: author cards, three related stories from the same topic.

The site accent colour (**Settings → Design → Brand**) replaces the default
ink blue for buttons, topic labels and counts. Ghost's heading and body font
settings replace Bodoni Moda and Hanken Grotesk.

## Fonts

Bodoni Moda and Hanken Grotesk, both SIL Open Font License, bundled as
variable latin subsets in `assets/fonts`, so the theme makes no third-party
requests.

## Design system

Tokens, type scale and component rules are in [DESIGN.md](DESIGN.md).
