# migrations

Two directories, one per database engine. Which one runs is decided at startup
by `ghost_core::Config::db_target` and applied by `ghost_db::pool::migrate`.

    sqlite/   the source of truth — edit these
    mysql/    generated — do not edit by hand

## Why two

The engines differ in ways that are not cosmetic. MySQL cannot index a `TEXT`
primary key without a prefix length, has no `CREATE INDEX IF NOT EXISTS`, has
no partial indexes, silently *ignores* a column-level `REFERENCES` clause, and
needs `MEDIUMTEXT` where SQLite's unbounded `TEXT` suffices. Writing one
dialect for both would mean a schema that is worse on both.

## The files

| File | Keep? | Why |
|---|---|---|
| `translate.py` | **required** | Generates `mysql/` from `sqlite/`. The only thing keeping the two in step. |
| `column_kinds.py` | **required** | Records which columns hold a date and which hold a boolean. |

`column_kinds.py` is not a convenience — it is the only remaining record of
that information. Ghost's original SQLite schema declared those columns
`DATETIME` and `BOOLEAN`, and code could read the type back with
`PRAGMA table_info`. Those declarations had to change, because sqlx's `Any`
driver accepts only Null/Integer/Float/Blob/Text from SQLite and rejects
`DATETIME` and `BOOLEAN` outright. The schema now says `TEXT` and `INTEGER`,
so the meaning lives in this file. Deleting it loses it.

## The files in each directory

Eleven, grouped by subject. They were consolidated before the first release
from 42 migrations that had been added one at a time, so no database built from
the older numbering is supported.

    0001_init         0005_members    0009_apps
    0002_analytics    0006_payments   0010_click
    0003_publishing   0007_social     0011_forms
    0004_email        0008_automations

These eleven are applied history, like any migration: **do not edit them or add
to them.** A schema change is a new file, `0012_…` onward.

## Changing the schema

1. Add your migration to `sqlite/`, in Ghost's own dialect.
   > **Not at present.** The generator stops at a statement in
   > `0004_email.sql` it cannot translate (it has since `0036`, before the
   > consolidation), and by then has rewritten the files before it. Until it is
   > fixed, write the MySQL twin of a new migration by hand and skip step 2.

2. Run the generator:

       cd migrations && python3 -I translate.py

   It rewrites every file in `mysql/` and regenerates
   `../crates/ghost-db/src/schema_kinds.rs`.
3. Check it: `cargo test -p ghost-db --test schema_parity`, which fails if the
   two schemas have drifted or if a column was added in a type MySQL cannot
   return — see `ghost_db::types` for what those are.

A new column holding long text must be added to `LONG` or `MEDIUM` in
`translate.py`; anything in `LONG` is `MEDIUMTEXT` on MySQL and has to be read
through `ghost_db::types::long`, because MySQL sends it as a blob.
