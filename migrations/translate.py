#!/usr/bin/env python3
"""Generate migrations/mysql/* from migrations/sqlite/*.

Run with `python3 -I translate.py` from the `migrations` directory. The output
is reviewed and committed; this script exists so that 34 files stay consistent
and so the rules below are written down once rather than applied by hand 34
times.

The rules, and why each is necessary:

* `TEXT(n)` -> `VARCHAR(n)`. SQLite ignores the length entirely; MySQL needs a
  bounded type for anything indexed or used as a key.
* bare `TEXT` -> `MEDIUMTEXT`, except where the column has a `DEFAULT` (MySQL
  forbids defaults on TEXT), is a primary key, or is indexed (MySQL cannot
  index TEXT without a prefix length). Those become `VARCHAR`.
* `DATETIME` -> `VARCHAR(32)`. Not optional: sqlx's `Any` driver has no
  timestamp type, so a native MySQL `DATETIME` would not decode. Every date
  here is already a `'YYYY-MM-DD HH:MM:SS'` string, which is what SQLite
  stored anyway -- its `DATETIME` is only `TEXT` affinity.
* `BOOLEAN` -> `SMALLINT`. `Any` maps only SMALLINT/INT/BIGINT from MySQL, and
  MySQL's `BOOLEAN` is an alias for `TINYINT`, which it refuses outright.
* `INTEGER` -> `BIGINT`, matching SQLite's 64-bit integers.
* `REAL` -> `DOUBLE`.
* **A referencing column is given its target's exact type.** MySQL requires
  both sides of a foreign key to be the same type, and the SQLite schema is
  inconsistent here because SQLite does not care -- `newsletter_id TEXT(24)`
  pointing at `newsletters.id TEXT`. Resolved by propagating the target's
  mapped type, transitively, rather than by guessing a width.
* Inline `REFERENCES` -> a table-level `CONSTRAINT ... FOREIGN KEY`. MySQL
  *parses and silently ignores* a column-level `REFERENCES`, so leaving them
  as they are would mean no foreign key is enforced at all on MySQL -- a
  silent loss of a constraint SQLite does enforce.
* `CREATE INDEX IF NOT EXISTS` -> `CREATE INDEX`; MySQL has no such clause.
  Safe because sqlx applies each migration exactly once.
* An index on a column too wide for MySQL's 3072-byte key limit gets a prefix
  length: `(path)` -> `(path(191))`.
* `lower(hex(randomblob(n)))` -> `LOWER(HEX(RANDOM_BYTES(n)))`.
* `a || b` -> `CONCAT(a, b)`.
* `datetime('now')` -> `UTC_TIMESTAMP()`.
* `INSERT OR IGNORE` -> `INSERT IGNORE`.
* `key` and `interval` are reserved words in MySQL and are real column names
  here, so they are backquoted. `group` already is in the source. Backticks
  are valid in SQLite too, so this costs nothing.
* Every table gets `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
  COLLATE=utf8mb4_unicode_ci`: utf8mb4 so a post can hold an emoji, and the
  case-insensitive collation is what replaces SQLite's `lower()` comparisons.
"""
import re
import pathlib
import collections
import sys

# Explicit, because `python3 -I` does not put the script's own directory on
# the import path.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from column_kinds import BOOL_COLUMNS, DATE_COLUMNS

DATE_SET = {tuple(c) for c in DATE_COLUMNS}
BOOL_SET = {tuple(c) for c in BOOL_COLUMNS}

SRC = pathlib.Path("sqlite")
DST = pathlib.Path("mysql")

# MySQL's key limit is 3072 bytes and utf8mb4 charges 4 per character, so a
# column wider than 768 characters needs a prefix length when indexed. 191 is
# the conventional Ghost choice and is ample for a slug or a path.
PREFIX = 191
MAX_INDEXABLE = 768

# A bare `TEXT` column becomes `MEDIUMTEXT` only when it genuinely holds long
# content, because a `MEDIUMTEXT` has to be read through `ghost_db::types::Text`
# -- MySQL sends it as a blob. Everything else gets a bounded `VARCHAR`, which
# arrives as text on both engines and needs nothing special at the call site.
#
# The list is explicit rather than inferred: these are the columns that hold a
# post body, a rendered email, a theme setting, a PEM, or a JSON document, and
# no heuristic over column names would identify them reliably. A column that
# is wrongly absent fails loudly on write under STRICT_ALL_TABLES rather than
# truncating, which is the failure direction to prefer.
LONG = {
    # Post and email bodies, and the editor formats they are built from. These
    # are the columns that genuinely run to hundreds of kilobytes.
    ("posts", "mobiledoc"), ("posts", "lexical"), ("posts", "html"),
    ("posts", "plaintext"), ("posts", "markdown"),
    ("posts", "codeinjection_head"), ("posts", "codeinjection_foot"),
    ("posts_meta", "frontmatter"),
    ("post_revisions", "lexical"), ("post_revisions", "snapshot"),
    ("mobiledoc_revisions", "mobiledoc"),
    ("snippets", "mobiledoc"), ("snippets", "lexical"),
    ("automation_action_revisions", "email_lexical"),
    ("emails", "html"), ("emails", "plaintext"),
    ("tags", "codeinjection_head"), ("tags", "codeinjection_foot"),
    ("comments", "html"),
    # A theme's navigation, its codeinjection and its custom settings all live
    # in these, and all three are unbounded in practice.
    ("settings", "value"),
    ("custom_theme_settings", "value"),
    # Not because these are huge, but because their tables are already close to
    # MySQL's 65,535-byte row limit and a `VARCHAR` counts against it while a
    # `MEDIUMTEXT` is stored off-row.
    ("emails", "error_data"), ("emails", "recipient_filter"), ("emails", "source"),
    ("mentions", "payload"),
    ("posts", "email_recipient_filter"), ("posts", "newsletter_audience_filter"),
    ("users", "accessibility"), ("users", "tour"),
}

# Bigger than the default but bounded, and — crucially — under the ~16,000
# characters beyond which MySQL reports a string as a blob. A `VARCHAR` of this
# size arrives as text on both engines, so these need no `types::Text` at the
# call site, which is why the list above is kept as short as it honestly can
# be: a JSON document, a PEM, a filter expression, a profile bio.
MEDIUM = {
    ("actions", "context"),
    ("app_installations", "manifest"),
    ("app_manifest_history", "manifest"), ("app_manifest_history", "source"),
    ("email_design_settings", "settings"),
    ("email_verifications", "value"),
    ("export_requests", "components"),
    ("federation_activities", "body"),
    ("federation_actors", "summary"),
    ("federation_interactions", "activity"),
    ("federation_keys", "private_pem"), ("federation_keys", "public_pem"),
    ("federation_objects", "content"),
    ("federation_received", "body"),
    ("machine_payment_replay_store", "value"),
    ("members", "commenting"),
    ("members_metafield_change_events", "metafields"),
    ("members_metafield_values", "value_text"),
    ("newsletters", "footer_content"),
    ("saved_views", "filter"),
    ("signing_keys", "private_pem"), ("signing_keys", "public_pem"),
}

MEDIUM_WIDTH = 4000

# A bare `TEXT` column whose name ends this way is a timestamp: this schema
# stores every date as a string, and several were declared `TEXT` rather than
# `DATETIME`. 32 characters holds an ISO-8601 instant with room to spare.
TIMESTAMP_SUFFIXES = ("_at",)

# Generous but bounded, for a bare `TEXT` that is neither. Long enough for a
# URL, a filter expression or an error message; short enough that a table full
# of them stays inside MySQL's 65,535-byte row limit.
DEFAULT_TEXT = 1000

TABLE_SUFFIX = " ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
RESERVED = ("key", "interval")

COL_RE = re.compile(r"^(\s*)`?(\w+)`?\s+([A-Za-z]+(?:\(\d+\))?)(?=[\s,)]|$)(.*)$")
NOT_A_COLUMN = ("PRIMARY", "UNIQUE", "FOREIGN", "CONSTRAINT", "CHECK", "KEY")
# `ON DELETE CASCADE`, `ON DELETE SET NULL`, `ON UPDATE CASCADE`, possibly both.
REF_RE = re.compile(
    r"REFERENCES\s+(\w+)\s*\(\s*`?(\w+)`?\s*\)"
    r"((?:\s+ON\s+(?:DELETE|UPDATE)\s+(?:CASCADE|RESTRICT|NO\s+ACTION|SET\s+NULL|SET\s+DEFAULT))*)",
    re.I)


def base_type(decl, has_default, bounded, table=None, col=None):
    """Translate one column type declaration, before FK propagation."""
    m = re.match(r"TEXT\((\d+)\)$", decl, re.I)
    if m:
        return f"VARCHAR({m.group(1)})"
    up = decl.upper()
    # A date or a boolean is identified by name, not by its declared type: the
    # SQLite schema has to say TEXT and INTEGER because `Any` rejects DATETIME
    # and BOOLEAN from SQLite. `column_kinds.py` preserves which is which.
    if (table, col) in DATE_SET:
        return "VARCHAR(32)"
    if (table, col) in BOOL_SET:
        return "SMALLINT"
    if up == "TEXT":
        if has_default or bounded:
            return f"VARCHAR({PREFIX})"
        if (table, col) in LONG:
            return "MEDIUMTEXT"
        if (table, col) in MEDIUM:
            return f"VARCHAR({MEDIUM_WIDTH})"
        if col and col.endswith(TIMESTAMP_SUFFIXES):
            return "VARCHAR(32)"
        return f"VARCHAR({DEFAULT_TEXT})"
    return {
        "DATETIME": "VARCHAR(32)",
        "BOOLEAN": "SMALLINT",
        "INTEGER": "BIGINT",
        "REAL": "DOUBLE",
    }.get(up, decl)


class Schema:
    """What the SQLite migrations declare, as a model."""

    def __init__(self, files):
        self.decl = {}          # (table, col) -> declared type
        self.default = set()    # (table, col) with a DEFAULT
        self.pk = set()         # (table, col) that is a PRIMARY KEY
        self.indexed = set()    # (table, col) named in any index
        self.fks = {}           # (table, col) -> (target_table, target_col)
        for p in files:
            self._scan(p)
        self.final = self._resolve()

    def _scan(self, path):
        table = None
        for raw in join_index_statements(path.read_text()).splitlines():
            bare = raw.strip()
            if not bare or bare.startswith("--"):
                continue

            m = re.match(r"CREATE TABLE(?: IF NOT EXISTS)? (\w+)", bare, re.I)
            if m:
                table = m.group(1)
                continue

            m = re.match(r"ALTER TABLE (\w+) ADD COLUMN `?(\w+)`?\s+"
                         r"([A-Za-z]+(?:\(\d+\))?)(?=[\s,;]|$)(.*)$", bare, re.I)
            if m:
                tbl, col, decl, tail = m.groups()
                self._column(tbl, col, decl, tail)
                continue

            m = re.match(r"CREATE (?:UNIQUE )?INDEX(?: IF NOT EXISTS)? \w+ ON (\w+) ?\((.*?)\)"
                         r"(?:\s+WHERE\s+.*)?\s*;", bare, re.I)
            if m:
                for col in m.group(2).split(","):
                    c = col.strip().strip("`")
                    if c:
                        self.indexed.add((m.group(1), c.split()[0].strip("`")))
                continue

            if table and re.match(r"\)\s*;?\s*$", bare):
                table = None
                continue

            if table:
                # A table-level `PRIMARY KEY (a, b)` or `UNIQUE (a, b)` keys
                # those columns just as an index would.
                m = re.match(r"(?:PRIMARY KEY|UNIQUE)\s*\((.*?)\)", bare, re.I)
                if m:
                    for col in m.group(1).split(","):
                        c = col.strip().strip("`")
                        if c:
                            self.indexed.add((table, c.split()[0].strip("`")))
                    continue
                m = COL_RE.match(raw)
                if m and m.group(3).upper() not in NOT_A_COLUMN:
                    self._column(table, m.group(2), m.group(3), m.group(4))

    def _column(self, table, col, decl, tail):
        self.decl[(table, col)] = decl
        up = tail.upper()
        if "DEFAULT" in up:
            self.default.add((table, col))
        if "PRIMARY KEY" in up:
            self.pk.add((table, col))
        if "UNIQUE" in up:
            self.indexed.add((table, col))
        ref = REF_RE.search(tail)
        if ref:
            self.fks[(table, col)] = (ref.group(1), ref.group(2))

    def _resolve(self):
        """Map every column, then make each FK match its target exactly."""
        final = {}
        for key, decl in self.decl.items():
            bounded = key in self.pk or key in self.indexed
            final[key] = base_type(decl, key in self.default, bounded, key[0], key[1])

        # A target may itself be a foreign key, so iterate to a fixed point.
        # Bounded by the longest reference chain, and `passes` guards against a
        # cycle in the schema rather than looping forever.
        for _ in range(10):
            changed = False
            for src, target in self.fks.items():
                want = final.get(target)
                if want is None:
                    raise SystemExit(f"{src} references unknown column {target}")
                # A MEDIUMTEXT key is not indexable, so a referenced column
                # must be bounded; widen the target instead of narrowing it.
                if want == "MEDIUMTEXT":
                    final[target] = want = f"VARCHAR({PREFIX})"
                    changed = True
                if final.get(src) != want:
                    final[src] = want
                    changed = True
            if not changed:
                break
        else:
            raise SystemExit("foreign key types did not converge")
        return final

    def type_of(self, table, col):
        return self.final.get((table, col), "MEDIUMTEXT")


def quote_reserved(sql):
    for word in RESERVED:
        sql = re.sub(rf"(?<![`\w.]){word}(?![`\w(])", f"`{word}`", sql, flags=re.I)
        sql = re.sub(rf"(?<=\.)({word})(?![`\w])", r"`\1`", sql, flags=re.I)
    return sql


def translate_expressions(sql):
    sql = re.sub(r"\brandomblob\((\d+)\)", r"RANDOM_BYTES(\1)", sql, flags=re.I)
    # SQLite's strftime with 'now'. MySQL spells minutes `%i` and seconds
    # `%s`, where SQLite uses `%M` and `%S` -- getting this wrong would store
    # a month where the minutes belong.
    sql = re.sub(r"STRFTIME\(\s*'%Y-%m-%d %H:%M:%S'\s*,\s*'now'\s*\)",
                 "DATE_FORMAT(UTC_TIMESTAMP(), '%Y-%m-%d %H:%i:%s')", sql, flags=re.I)
    # SQLite's case-insensitive collation. The MySQL tables are created with
    # utf8mb4_unicode_ci, which compares case-insensitively already, so the
    # clause is redundant rather than merely unsupported.
    sql = re.sub(r"\s+COLLATE NOCASE\b", "", sql, flags=re.I)
    sql = re.sub(r"datetime\('now'\)", "UTC_TIMESTAMP()", sql, flags=re.I)
    sql = re.sub(r"\bINSERT OR IGNORE INTO\b", "INSERT IGNORE INTO", sql, flags=re.I)
    return sql


def fix_concat(line):
    m = re.match(r"^(\s*SET \w+ = )(.+?) \|\| (.+?),\s*$", line)
    if m:
        return f"{m.group(1)}CONCAT({m.group(2)}, {m.group(3)}),"
    raise SystemExit(f"unhandled || in: {line!r}")


def index_parts(schema, table, cols):
    parts = []
    for col in cols.split(","):
        c = col.strip()
        if not c:
            continue
        name = c.strip("`").split()[0].strip("`")
        mapped = schema.type_of(table, name)
        w = re.match(r"VARCHAR\((\d+)\)", mapped)
        if mapped == "MEDIUMTEXT" or (w and int(w.group(1)) > MAX_INDEXABLE):
            parts.append(f"`{name}`({PREFIX})")
        else:
            parts.append(f"`{name}`")
    return ", ".join(parts)


def join_index_statements(text):
    """Put each CREATE INDEX on one line.

    Several are wrapped across two lines in the source, and the rewriting
    below needs to see a whole statement in order to strip MySQL's unsupported
    `IF NOT EXISTS` and to add a key prefix length.
    """
    lines = text.splitlines()
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        if re.match(r"\s*CREATE (?:UNIQUE )?INDEX", line, re.I) and ";" not in line:
            buf = [line.rstrip()]
            while i + 1 < len(lines) and ";" not in buf[-1]:
                i += 1
                buf.append(lines[i].strip())
            out.append(" ".join(buf))
        else:
            out.append(line)
        i += 1
    return "\n".join(out)


# SQLite's `AFTER INSERT ... BEGIN UPDATE <same table> ... END` is illegal in
# MySQL (error 1442: a trigger cannot update the table it is on). Assigning to
# NEW in a BEFORE INSERT trigger is the equivalent, and is the form MySQL
# expects. Written out rather than translated: there is one trigger in the
# whole schema, and a general translator for trigger bodies would be far more
# machinery than the single case justifies.
TRIGGERS = {
    "labels_slug_on_insert":
        "CREATE TRIGGER labels_slug_on_insert BEFORE INSERT ON labels FOR EACH ROW\n"
        "SET NEW.slug = IFNULL(NEW.slug, LOWER(REPLACE(NEW.name, ' ', '-')));",
}


def translate(path, schema):
    out = []
    table = None
    fks = []
    deferred = []
    seq = collections.Counter()

    skip_until_trigger_end = False
    for raw in join_index_statements(path.read_text()).splitlines():
        bare = raw.strip()

        if skip_until_trigger_end:
            if bare.upper().rstrip().endswith("END;"):
                skip_until_trigger_end = False
            continue

        m = re.match(r"CREATE TRIGGER (\w+)", bare, re.I)
        if m:
            name = m.group(1)
            if name not in TRIGGERS:
                raise SystemExit(f"no MySQL form written for trigger {name}")
            out.append(TRIGGERS[name])
            # The SQLite body may run to a later line.
            if not bare.upper().rstrip().endswith("END;"):
                skip_until_trigger_end = True
            continue

        if not bare or bare.startswith("--"):
            out.append(raw)
            continue

        m = re.match(r"CREATE TABLE(?: IF NOT EXISTS)? (\w+)", bare, re.I)
        if m:
            table = m.group(1)
            fks = []
            out.append(translate_expressions(quote_reserved(raw)))
            continue

        if table and re.match(r"\)\s*;?\s*$", bare):
            # Whatever ended up last must not have a trailing comma.
            for i in range(len(out) - 1, -1, -1):
                s = out[i].rstrip()
                if s and not s.lstrip().startswith("--"):
                    if s.endswith(","):
                        out[i] = s[:-1]
                    break
            out.append(")" + TABLE_SUFFIX + ";")
            deferred.extend(fks)
            fks = []
            table = None
            continue

        if table:
            m = COL_RE.match(raw)
            if m and m.group(3).upper() not in NOT_A_COLUMN:
                indent, name, _decl, tail = m.groups()
                ref = REF_RE.search(tail)
                if ref:
                    tail = tail[:ref.start()] + tail[ref.end():]
                    seq[table] += 1
                    n = seq[table]
                    cname = f"{table}_{name}_fk" + (f"{n}" if n > 1 else "")
                    actions = re.sub(r"\s+", " ", ref.group(3) or "").strip()
                    fks.append(f"ALTER TABLE {table} ADD CONSTRAINT {cname} "
                               f"FOREIGN KEY (`{name}`) "
                               f"REFERENCES {ref.group(1)}(`{ref.group(2)}`)"
                               f"{(' ' + actions) if actions else ''};")
                tail = re.sub(r"\s+", " ", tail).strip()
                line = f"{indent}`{name}` {schema.type_of(table, name)}"
                if tail:
                    line += " " + tail
                out.append(translate_expressions(line))
                continue
            # A table-level key clause is emitted as written: `quote_reserved`
            # would backquote the `KEY` in `PRIMARY KEY`.
            if re.match(r"(?:PRIMARY KEY|UNIQUE|FOREIGN KEY|CONSTRAINT|CHECK)\b", bare, re.I):
                out.append(translate_expressions(raw))
            else:
                out.append(translate_expressions(quote_reserved(raw)))
            continue

        # A partial index -- `... WHERE <cond>` -- which MySQL does not have.
        m = re.match(r"(\s*)CREATE (UNIQUE )?INDEX(?: IF NOT EXISTS)? (\w+) ON (\w+) ?\((.*?)\)"
                     r"\s+WHERE\s+(.*?)\s*;\s*$", bare, re.I)
        if m:
            _i, uniq, iname, tbl, cols, cond = m.groups()
            if not uniq:
                # A partial *non-unique* index is only an optimisation, so the
                # full index is an acceptable substitute.
                out.append(f"CREATE INDEX {iname} ON {tbl} ({index_parts(schema, tbl, cols)});")
                continue
            # A partial unique index is a constraint, and dropping the
            # condition would make it stricter -- here, an app uninstalled and
            # reinstalled would collide. MySQL's equivalent is a generated
            # column that is NULL when the condition is false, since NULLs do
            # not collide in a unique index.
            col = cols.strip().strip("`").split(",")[0].strip().strip("`")
            gen = f"{iname}_active"
            # Must be indexable whatever the source column is: a MEDIUMTEXT
            # generated column cannot carry a unique index.
            typ = schema.type_of(tbl, col)
            if typ == "MEDIUMTEXT":
                typ = f"VARCHAR({PREFIX})"
            out.append(f"-- MySQL has no partial index. This generated column is NULL")
            out.append(f"-- unless the condition holds, and NULLs do not collide in a")
            out.append(f"-- unique index -- which is exactly what `WHERE {cond}` means.")
            out.append(f"ALTER TABLE {tbl} ADD COLUMN `{gen}` {typ} "
                       f"GENERATED ALWAYS AS (IF({cond}, `{col}`, NULL)) VIRTUAL;")
            out.append(f"CREATE UNIQUE INDEX {iname} ON {tbl} (`{gen}`);")
            continue

        m = re.match(r"(\s*)CREATE (UNIQUE )?INDEX(?: IF NOT EXISTS)? (\w+) ON (\w+) ?\((.*)\)\s*;\s*$",
                     bare, re.I)
        if m:
            _i, uniq, iname, tbl, cols = m.groups()
            out.append(f"CREATE {uniq or ''}INDEX {iname} ON {tbl} ({index_parts(schema, tbl, cols)});")
            continue

        m = re.match(r"ALTER TABLE (\w+) ADD COLUMN `?(\w+)`?\s+"
                     r"([A-Za-z]+(?:\(\d+\))?)(?=[\s,;]|$)(.*)$", bare, re.I)
        if m:
            tbl, name, _decl, tail = m.groups()
            ref = REF_RE.search(tail)
            fk_stmt = None
            if ref:
                tail = tail[:ref.start()] + tail[ref.end():]
                actions = re.sub(r"\s+", " ", ref.group(3) or "").strip()
                fk_stmt = (f"ALTER TABLE {tbl} ADD CONSTRAINT {tbl}_{name}_fk "
                           f"FOREIGN KEY (`{name}`) REFERENCES {ref.group(1)}(`{ref.group(2)}`)"
                           f"{(' ' + actions) if actions else ''};")
            tail = re.sub(r"\s+", " ", tail).strip().rstrip(";").strip()
            line = f"ALTER TABLE {tbl} ADD COLUMN `{name}` {schema.type_of(tbl, name)}"
            if tail:
                line += " " + tail
            out.append(translate_expressions(line) + ";")
            if fk_stmt:
                out.append(fk_stmt)
            continue

        line = raw
        if "||" in line:
            line = fix_concat(line)
        out.append(translate_expressions(quote_reserved(line)))

    if deferred:
        out.append("")
        out.append("-- Foreign keys are added after the tables they point at.")
        out.append("-- SQLite resolves a reference lazily and accepts a forward one;")
        out.append("-- MySQL requires the referenced table to exist already.")
        out.extend(deferred)
    return "\n".join(out) + "\n"


def main():
    files = sorted(SRC.glob("*.sql"))
    schema = Schema(files)
    DST.mkdir(exist_ok=True)
    for p in files:
        (DST / p.name).write_text(translate(p, schema))
    mediumtext = sum(1 for v in schema.final.values() if v == "MEDIUMTEXT")
    print(f"translated {len(files)} files: {len(schema.final)} columns "
          f"({mediumtext} MEDIUMTEXT), {len(schema.fks)} foreign keys, "
          f"{len(schema.indexed)} indexed columns")


if __name__ == "__main__":
    main()
