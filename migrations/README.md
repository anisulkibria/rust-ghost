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

## Changing the schema

1. Add your migration to `sqlite/`, in Ghost's own dialect.
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

## A database created before the port

Rewriting `DATETIME` to `TEXT` and `BOOLEAN` to `INTEGER` edited migrations
that were already applied, so an older SQLite database fails to start twice
over: sqlx sees the changed checksums and refuses —

    migration failed: migration 1 was previously applied but has been modified

— and underneath that, its tables still declare the two types `Any` rejects on
read. Convert it once:

    python3 -I tools/convert-sqlite-db.py content/data/ghost-dev.db

That rewrites the declared types in place and re-checksums `_sqlx_migrations`.
No stored value changes, because none was ever wrong: a date was already held
as `'YYYY-MM-DD HH:mm:ss'` text and a boolean as integer 0/1 — only the
declarations disagreed. The script builds a reference schema from `sqlite/`
first and aborts unless the target matches it once declared types are
normalised away, so it cannot paper over a database that is genuinely behind.
Pass `--dry-run` to see what it would touch. A new database needs none of this.
