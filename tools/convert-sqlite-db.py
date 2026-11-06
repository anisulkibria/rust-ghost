#!/usr/bin/env python3
"""Convert a pre-MySQL-port SQLite database to the current schema.

The MySQL port rewrote the SQLite migrations in place: `DATETIME` became
`TEXT` and `BOOLEAN` became `INTEGER`, because sqlx's `Any` driver accepts
only Null/Integer/Float/Blob/Text from SQLite and rejects the other two
outright. A database created before that change is left in two ways
inconsistent with the migrations now compiled into the binary:

  1. its tables still *declare* `DATETIME`/`BOOLEAN`, which `Any` rejects on
     read; and
  2. `_sqlx_migrations` holds the checksums of the old migration files, so
     sqlx refuses to start at all -- "migration 1 was previously applied but
     has been modified".

This script fixes both, in place, without touching a single stored value.
Nothing needs converting: a date was already stored as `'YYYY-MM-DD
HH:mm:ss'` text (`DATETIME` carries NUMERIC affinity, and that string is not
convertible to a number, so SQLite stored it as text), and a boolean was
already stored as integer 0/1. Only the declared types were ever wrong.

It is deliberately narrow. It rewrites `sqlite_master` directly, which is
only safe because the edit is provably type-tokens-only, so it proves that
first: it builds a reference database from `migrations/sqlite/*.sql`, and
refuses to touch anything unless the target's schema is identical to the
reference once declared types are normalised away. Any other difference --
a missing table, an extra index, a changed constraint -- means the database
is not merely an old-dialect copy of this schema, and a blind schema
rewrite would corrupt it.

Usage:

    python3 -I tools/convert-sqlite-db.py content/data/ghost-dev.db
    python3 -I tools/convert-sqlite-db.py --dry-run <db>   # report only
    python3 -I tools/convert-sqlite-db.py --no-backup <db>

Safe to re-run: on an already-converted database it reports no changes.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import sqlite3
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIGRATIONS = REPO / "migrations" / "sqlite"

# The only declared-type rewrites the port made. Applied to both sides before
# comparing, so a difference that survives is a real structural difference.
TYPE_ALIASES = (("DATETIME", "TEXT"), ("BOOLEAN", "INTEGER"))


def normalise(sql: str) -> str:
    """Strip away the differences this script is allowed to introduce.

    That is: the two declared-type renames, and the decorative length on a
    `TEXT(n)` column -- SQLite ignores it entirely (it only matters to the
    generated MySQL schema, where it becomes a `VARCHAR` width), so a change
    to it cannot affect stored data.
    """
    for old, new in TYPE_ALIASES:
        sql = re.sub(rf"(?<![A-Za-z0-9_]){old}(?![A-Za-z0-9_])", new, sql)
    sql = re.sub(r"TEXT\(\d+\)", "TEXT", sql)
    return re.sub(r"\s+", " ", sql).strip()


def migration_files() -> list[tuple[int, str, bytes]]:
    """Every migration, as sqlx's `migrate!` macro sees it."""
    out = []
    for path in sorted(MIGRATIONS.glob("*.sql")):
        version, _, rest = path.stem.partition("_")
        out.append((int(version), rest.replace("_", " "), path.read_bytes()))
    if not out:
        sys.exit(f"no migrations found in {MIGRATIONS}")
    return out


def reference_schema(tmp: Path) -> dict[tuple[str, str], str]:
    """The schema the current migrations produce, built from scratch."""
    db = tmp / "reference.db"
    con = sqlite3.connect(db)
    try:
        for version, _, sql in migration_files():
            try:
                con.executescript(sql.decode())
            except sqlite3.Error as e:
                sys.exit(f"migration {version} does not apply to an empty database: {e}")
        con.commit()
        return schema_of(con)
    finally:
        con.close()


def schema_of(con: sqlite3.Connection) -> dict[tuple[str, str], str]:
    rows = con.execute(
        "SELECT type, name, sql FROM sqlite_master "
        "WHERE name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'"
    )
    return {(t, n): (s or "") for t, n, s in rows}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("database", type=Path)
    ap.add_argument("--dry-run", action="store_true", help="report what would change, change nothing")
    ap.add_argument("--no-backup", action="store_true", help="do not write a .bak copy first")
    args = ap.parse_args()

    db: Path = args.database
    if not db.is_file():
        sys.exit(f"{db}: no such database")

    with tempfile.TemporaryDirectory() as td:
        reference = reference_schema(Path(td))

    con = sqlite3.connect(db)
    con.isolation_level = None  # explicit transactions; PRAGMAs must not be wrapped
    try:
        return convert(con, db, reference, args)
    finally:
        con.close()


def convert(con, db: Path, reference, args) -> int:
    current = schema_of(con)

    # --- prove the database is an old-dialect copy of exactly this schema ---

    missing = sorted(k for k in reference if k not in current)
    extra = sorted(k for k in current if k not in reference)
    if missing or extra:
        for t, n in missing:
            print(f"  missing {t} {n}", file=sys.stderr)
        for t, n in extra:
            print(f"  unexpected {t} {n}", file=sys.stderr)
        sys.exit(
            f"{db} is not at the current schema version, so this script will not touch it.\n"
            "It converts declared types only; it cannot add or drop objects. Let the\n"
            "normal migrations bring the database up to date first."
        )

    structural = [k for k in reference if normalise(reference[k]) != normalise(current[k])]
    if structural:
        for t, n in sorted(structural):
            print(f"  {t} {n} differs by more than its declared types", file=sys.stderr)
        sys.exit(
            f"{db} differs structurally from the current migrations, so a schema rewrite\n"
            "would corrupt it. Convert it by copying the data into a fresh database instead."
        )

    stale = sorted(k for k in reference if reference[k] != current[k])
    for t, n in stale:
        if t != "table":
            sys.exit(f"refusing to rewrite {t} {n}: only table definitions are convertible")

    # --- the migration log ---

    files = migration_files()
    applied = {v: (d, c) for v, d, c in con.execute(
        "SELECT version, description, checksum FROM _sqlx_migrations"
    )}
    if set(applied) != {v for v, _, _ in files}:
        sys.exit(
            f"{db} has applied migrations {sorted(applied)}, but the tree has "
            f"{sorted(v for v, _, _ in files)}.\nThis script only re-checksums a database "
            "that is already fully migrated."
        )

    rechecksum = []
    for version, description, sql in files:
        want = hashlib.sha384(sql).digest()
        have_description, have = applied[version]
        if bytes(have) != want:
            rechecksum.append((version, description, want))
        if have_description != description:
            print(
                f"  note: migration {version} is logged as {have_description!r}, "
                f"the file is named {description!r}",
                file=sys.stderr,
            )

    if not stale and not rechecksum:
        print(f"{db} is already at the current schema. Nothing to do.")
        return 0

    print(f"{db}")
    print(f"  {len(stale)} table definition(s) to convert to TEXT/INTEGER declared types")
    print(f"  {len(rechecksum)} migration checksum(s) to refresh")
    if args.dry_run:
        for _, name in stale:
            print(f"    table {name}")
        for version, description, _ in rechecksum:
            print(f"    migration {version} {description}")
        print("  --dry-run: nothing written")
        return 0

    if not args.no_backup:
        backup = db.with_suffix(db.suffix + ".bak")
        n = 1
        while backup.exists():
            backup = db.with_suffix(f"{db.suffix}.bak{n}")
            n += 1
        # sqlite3's own backup API, so a live WAL is captured consistently.
        dest = sqlite3.connect(backup)
        try:
            con.backup(dest)
        finally:
            dest.close()
        print(f"  backed up to {backup}")

    # --- rewrite ---

    (schema_version,) = con.execute("PRAGMA schema_version").fetchone()
    con.execute("PRAGMA writable_schema=ON")
    try:
        con.execute("BEGIN")
        for _, name in stale:
            con.execute(
                "UPDATE sqlite_master SET sql = ? WHERE type = 'table' AND name = ?",
                (reference[("table", name)], name),
            )
        for version, _, checksum in rechecksum:
            con.execute(
                "UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?",
                (checksum, version),
            )
        con.execute("COMMIT")
        # Tell every other connection the schema moved under it.
        con.execute(f"PRAGMA schema_version = {schema_version + 1}")
    except BaseException:
        con.execute("ROLLBACK")
        raise
    finally:
        con.execute("PRAGMA writable_schema=OFF")

    # --- verify, on a fresh connection that has reparsed the schema ---

    con.close()
    check = sqlite3.connect(db)
    try:
        (integrity,) = check.execute("PRAGMA integrity_check").fetchone()
        if integrity != "ok":
            sys.exit(f"integrity_check failed after conversion: {integrity}")
        violations = check.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            print(f"  warning: {len(violations)} pre-existing foreign key violation(s)", file=sys.stderr)

        after = schema_of(check)
        if after != reference:
            bad = sorted(k for k in reference if reference[k] != after.get(k))
            sys.exit(f"schema still differs after conversion: {bad}")

        for version, _, sql in migration_files():
            (stored,) = check.execute(
                "SELECT checksum FROM _sqlx_migrations WHERE version = ?", (version,)
            ).fetchone()
            if bytes(stored) != hashlib.sha384(sql).digest():
                sys.exit(f"checksum for migration {version} did not take")
    finally:
        check.close()

    print("  converted; schema matches the migrations and integrity_check is ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
