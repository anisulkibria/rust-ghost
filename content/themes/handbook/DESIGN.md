# Handbook design system

Handbook takes its typography and spacing from print reference books. The
homepage opens with a photographic cover, then uses a grid of numbered
sections to lead readers to the relevant article. Post and section pages
carry those same numbers into the contents, breadcrumbs, and reading links.

## Typography and color

| Element | Treatment |
|---|---|
| Cover and page titles | Instrument Serif, with a variable size for different viewports |
| Reading and controls | IBM Plex Sans, 17px in prose and 15px in the interface |
| Section and article numbers, code, key hints | IBM Plex Mono with tabular numerals |
| Background and surfaces | Warm ivory in light mode and charcoal in dark mode |
| Section cards | Clay, sage, and lilac headers in reading order |
| Links and current positions | Terracotta or the publication's accent color |

The body and heading fonts honor the publication’s font overrides. The color
scheme starts from the theme setting and readers can switch it at any time.

## Layout

- The homepage pairs the heading and search with the cover artwork. The
  columns stack on phones. A custom Hero image can replace the bundled art.
- Section cards reuse the same title, summary, article links, and count at
  every breakpoint. The grid collapses from three columns to two and then one.
- Article pages use a sticky contents tree and outline alongside a reading
  column up to 44rem wide. At 1080px and below, the outline moves into a
  collapsible panel above the article. At 760px and below, the contents tree
  also collapses behind a button.
- The article outline uses the direct h2 and h3 headings. Heading links
  preserve existing anchors, and the outline marks the reader’s position.
- The article feedback and previous/next links follow the body. Store,
  directory, course, and page-builder embeds keep their own styles.

Place user-facing strings in the theme locale, keep numbering derived from
the contents tree, and check the light and dark schemes when changing color
tokens in `assets/css/screen.css`.
