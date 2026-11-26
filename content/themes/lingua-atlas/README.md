# Lingua Atlas

Lingua Atlas is a responsive Vuuut LMS theme for Arabic, English and French. It includes a language-learning homepage, the searchable catalogue, course details and syllabi, the lesson player, and the My learning dashboard.

Select **Lingua Atlas** under **Themes** in Vuuut admin. From the Vuuut workspace, run `./run.sh seed-languages` with a build that includes the command, then restart Vuuut. This adds six free beginner courses, three paid intermediate courses, and 21 editable lessons, including one public preview per course. The command enables the LMS and skips courses with an existing slug. The paid samples have linked store products. The store must be enabled to show the purchase button.

The homepage reads the LMS categories with slugs `arabic`, `english`, and `french`. Mark a free course as featured to show it in the free course grid. The homepage also lists paid language courses. The practice section lists non-featured free language courses with a recorded duration of 45 minutes or less. Individual course covers replace the built-in language artwork.

Use the theme settings to customize the homepage copy, choose a color scheme, hide the language section, or show the journal. If the LMS uses a custom route, enter it in **Catalog address**, including the trailing slash.

Import [lingua-atlas-demo.json](demo/lingua-atlas-demo.json) on a test publication to add six journal articles and an About page. Use **Settings → Advanced → Import content**, or run `vuuut import-content content/themes/lingua-atlas/demo/lingua-atlas-demo.json`. Add `--with-settings` if you want the sample menus as well. Enable **Show journal** and set **Journal tag** to `language-journal` to showcase the sample articles on the homepage.

The **Hero image** setting replaces the bundled phrasebook photograph. A portrait or square image works well. Instrument Serif and Plus Jakarta Sans are self-hosted under the SIL Open Font License 1.1.
