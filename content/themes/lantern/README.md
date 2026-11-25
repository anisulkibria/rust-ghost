# Lantern

An academy theme for Vuuut's **LMS & Online Courses** tool.

- **Homepage** — a lit hero with course search, subjects, featured, popular and
  free courses, "how it works", the journal and a closing call to join.
- **Catalogue** (`lms/courses.hbs`) — search, level, price and sort filters,
  subject chips, a featured spotlight and course cards with ratings and prices.
- **Course page** (`lms/course.hbs`) — a night-blue hero, a sticky enrol card
  with the intro video, what you'll learn, an expandable syllabus, the
  instructor, rating breakdown and reviews.
- **Lesson player** (`lms/lesson.hbs`) — a focused two-column player with the
  course outline and progress ring, video, downloads, quizzes, assignments and
  "Complete & continue"; the outline becomes a drawer on phones.
- **My learning** (`lms/dashboard.hbs`) — progress, continue buttons and
  certificates.

Data comes from `{{#get "courses"}}` / `{{#get "course_categories"}}` and the
LMS templates' context (see `crates/vuuut-front/src/lms.rs`). Set the
catalogue's address under *Theme settings → Catalog address* if you moved it.

Light and dark schemes; the site accent colour overrides the lantern amber.

Fonts: Fraunces and Plus Jakarta Sans, both SIL Open Font License 1.1,
self-hosted in `assets/fonts/`.
