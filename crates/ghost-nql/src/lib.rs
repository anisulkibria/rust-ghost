//! Ghost NQL (Ninja Query Language) — Mongo-like filter strings.
//!
//! Grammar (subset mirroring @tryghost/nql):
//! ```text
//! expr    := or_expr
//! or_expr := and_expr ( "," and_expr )*
//! and_expr:= unit ( "+" unit )*
//! unit    := "(" expr ")" | statement
//! statement := key ":" value | key ":" comparator value
//! value   := word | 'str' | "str" | number | true | false | null | [v,v]
//! ```
//! A leading `-` on a value negates equality (`tag:-news`).
//!
//! The parser produces an [`Expr`]; callers either compile it to
//! parameterized SQL ([`compile`]) or evaluate it in memory against a
//! serialized resource ([`evaluate`]) for routing ownership checks.
use ghost_core::{GhostError, Result};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum NqlValue {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
    List(Vec<NqlValue>),
}

impl NqlValue {
    fn to_bind(&self) -> Value {
        match self {
            NqlValue::Str(s) => Value::String(s.clone()),
            NqlValue::Num(n) => serde_json::json!(n),
            NqlValue::Bool(b) => Value::Bool(*b),
            NqlValue::Null => Value::Null,
            NqlValue::List(items) => items
                .first()
                .map(|v| v.to_bind())
                .unwrap_or(Value::Null),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
}

#[derive(Debug, Clone)]
pub struct Cond {
    pub key: String,
    pub cmp: Cmp,
    pub value: NqlValue,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Cond(Cond),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Number(f64),
    Bool(bool),
    Null,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Colon,
    Comparator(String),
    Plus,
    Comma,
    Minus,
}

fn lex(input: &str) -> Result<Vec<Tok>> {
    let mut toks = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '[' => { toks.push(Tok::LBracket); i += 1; }
            ']' => { toks.push(Tok::RBracket); i += 1; }
            '(' => { toks.push(Tok::LParen); i += 1; }
            ')' => { toks.push(Tok::RParen); i += 1; }
            '+' => { toks.push(Tok::Plus); i += 1; }
            ',' => { toks.push(Tok::Comma); i += 1; }
            ':' => { toks.push(Tok::Colon); i += 1; }
            '~' => { toks.push(Tok::Comparator("~".into())); i += 1; }
            '>' | '<' => {
                let mut op = c.to_string();
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    op.push('=');
                    i += 1;
                }
                toks.push(Tok::Comparator(op));
                i += 1;
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        s.push(match chars[i] {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        s.push(chars[i]);
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(GhostError::BadRequest("unterminated quote in filter".into()));
                }
                i += 1; // closing quote
                toks.push(Tok::Quoted(s));
            }
            _ => {
                let mut s = String::new();
                while i < chars.len() {
                    let ch = chars[i];
                    if " \t[],()+,:~<>'\"".contains(ch) {
                        break;
                    }
                    s.push(ch);
                    i += 1;
                }
                match s.as_str() {
                    "true" => toks.push(Tok::Bool(true)),
                    "false" => toks.push(Tok::Bool(false)),
                    "null" => toks.push(Tok::Null),
                    _ => {
                        if let Ok(n) = s.parse::<f64>() {
                            toks.push(Tok::Number(n));
                        } else {
                            toks.push(Tok::Word(s));
                        }
                    }
                }
            }
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Parser (recursive descent)
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut branches = vec![self.parse_and()?];
        while self.peek() == Some(&Tok::Comma) {
            self.next();
            branches.push(self.parse_and()?);
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { Expr::Or(branches) })
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut parts = vec![self.parse_unit()?];
        while self.peek() == Some(&Tok::Plus) {
            self.next();
            parts.push(self.parse_unit()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Expr::And(parts) })
    }

    fn parse_unit(&mut self) -> Result<Expr> {
        match self.peek() {
            Some(Tok::LParen) => {
                self.next();
                let e = self.parse_or()?;
                if self.next() != Some(Tok::RParen) {
                    return Err(GhostError::BadRequest("expected ) in filter".into()));
                }
                Ok(e)
            }
            Some(Tok::Word(_)) => self.parse_statement(),
            _ => Err(GhostError::BadRequest("unexpected token in filter".into())),
        }
    }

    fn parse_statement(&mut self) -> Result<Expr> {
        let key = match self.next() {
            Some(Tok::Word(w)) => w,
            _ => return Err(GhostError::BadRequest("expected key in filter".into())),
        };
        if self.next() != Some(Tok::Colon) {
            return Err(GhostError::BadRequest(format!("expected ':' after key {key}")));
        }
        // optional comparator directly after the colon: key:>value
        let cmp = match self.peek() {
            Some(Tok::Comparator(op)) => {
                let op = op.clone();
                self.next();
                match op.as_str() {
                    ">" => Cmp::Gt,
                    ">=" => Cmp::Gte,
                    "<" => Cmp::Lt,
                    "<=" => Cmp::Lte,
                    "~" => Cmp::Like,
                    _ => Cmp::Eq,
                }
            }
            _ => Cmp::Eq,
        };

        let negate = if self.peek() == Some(&Tok::Minus) {
            self.next();
            true
        } else {
            false
        };

        let mut value = self.parse_value()?;
        // `tag:-news` lexes '-' inside the word; strip a leading dash too.
        let negate = negate
            || match &mut value {
                NqlValue::Str(s) if s.starts_with('-') && s.len() > 1 => {
                    *s = s[1..].to_string();
                    true
                }
                _ => false,
            };
        let cmp = if negate {
            match cmp {
                Cmp::Eq => Cmp::Ne,
                other => other,
            }
        } else {
            cmp
        };
        Ok(Expr::Cond(Cond { key, cmp, value }))
    }

    fn parse_value(&mut self) -> Result<NqlValue> {
        match self.next() {
            Some(Tok::Quoted(s)) => Ok(NqlValue::Str(s)),
            Some(Tok::Word(w)) => Ok(NqlValue::Str(w)),
            Some(Tok::Number(n)) => Ok(NqlValue::Num(n)),
            Some(Tok::Bool(b)) => Ok(NqlValue::Bool(b)),
            Some(Tok::Null) => Ok(NqlValue::Null),
            Some(Tok::LBracket) => {
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        Some(Tok::RBracket) => {
                            self.next();
                            break;
                        }
                        None => return Err(GhostError::BadRequest("unterminated [ in filter".into())),
                        _ => {
                            items.push(self.parse_value()?);
                            if self.peek() == Some(&Tok::Comma) {
                                self.next();
                            }
                        }
                    }
                }
                Ok(NqlValue::List(items))
            }
            other => Err(GhostError::BadRequest(format!("unexpected filter value: {other:?}"))),
        }
    }
}

/// Parse a filter string into an expression tree.
pub fn parse(input: &str) -> Result<Expr> {
    if input.trim().is_empty() {
        return Err(GhostError::BadRequest("empty filter".into()));
    }
    let toks = lex(input)?;
    if toks.is_empty() {
        return Err(GhostError::BadRequest("empty filter".into()));
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.parse_or()?;
    if p.pos != p.toks.len() {
        return Err(GhostError::BadRequest("trailing tokens in filter".into()));
    }
    Ok(e)
}

/// AND-merge two optional expressions (enforced + user filters).
pub fn and(a: Option<Expr>, b: Option<Expr>) -> Option<Expr> {
    match (a, b) {
        (Some(x), Some(y)) => Some(Expr::And(vec![x, y])),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------
// SQL compilation
// ---------------------------------------------------------------------------

/// A compiled WHERE fragment with binds.
#[derive(Debug, Default, Clone)]
pub struct Sql {
    pub clause: String,
    pub binds: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColKind {
    Text,
    Numeric,
    Bool,
}

/// How a filter key maps onto SQL.
#[derive(Debug, Clone)]
pub enum Col {
    /// Direct column on the base table (e.g. `posts.slug`).
    Simple { expr: String, kind: ColKind },
    /// EXISTS subquery spec; `{op}`/`{v}` are placeholders.
    Exists { sql: String },
}

fn simple(table: &str, col: &str, kind: ColKind) -> Result<Col> {
    Ok(Col::Simple { expr: format!("{table}.{col}"), kind })
}

fn exists(sql: &str) -> Result<Col> {
    Ok(Col::Exists { sql: sql.to_string() })
}

fn unknown(key: &str) -> GhostError {
    GhostError::ValidationError {
        message: format!("filter key not recognised: {key}"),
        context: None,
    }
}

/// Known keys per resource (virtual keys expanded). Unknown keys → 422.
pub fn column_for(resource: &str, key: &str) -> Result<Col> {
    // Virtual key expansion (Ghost filterExpansions)
    let expanded: &str = match key {
        "primary_tag" | "tag" | "tags" => "tags.slug",
        "primary_author" | "author" | "authors" => "authors.slug",
        other => other,
    };

    match resource {
        "posts" | "pages" => {
            if let Some(c) = expanded.strip_prefix("posts_meta.") {
                let mcol = match c {
                    "meta_title" => "pm.meta_title",
                    "meta_description" => "pm.meta_description",
                    "og_image" => "pm.og_image",
                    "og_title" => "pm.og_title",
                    "og_description" => "pm.og_description",
                    "twitter_image" => "pm.twitter_image",
                    "twitter_title" => "pm.twitter_title",
                    "twitter_description" => "pm.twitter_description",
                    "email_subject" => "pm.email_subject",
                    "feature_image_alt" => "pm.feature_image_alt",
                    "feature_image_caption" => "pm.feature_image_caption",
                    _ => return Err(unknown(key)),
                };
                return exists(&format!(
                    "EXISTS (SELECT 1 FROM posts_meta pm WHERE pm.post_id = posts.id AND {mcol} {{op}} {{v}})"
                ));
            }
            match expanded {
                "id" | "uuid" | "slug" | "title" | "status" | "type" | "visibility" | "locale"
                | "comment_id" | "published_at" | "created_at" | "updated_at" | "published_by"
                | "custom_excerpt" | "html" | "plaintext" | "feature_image" | "canonical_url"
                | "custom_template" | "codeinjection_head" | "codeinjection_foot"
                | "newsletter_id" | "email_recipient_filter" => simple("posts", expanded, ColKind::Text),
                "featured" => simple("posts", "featured", ColKind::Bool),
                "reading_time" => simple("posts", "reading_time", ColKind::Numeric),
                "tags.slug" => exists("EXISTS (SELECT 1 FROM posts_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.post_id = posts.id AND t.slug {op} {v})"),
                "tags.name" => exists("EXISTS (SELECT 1 FROM posts_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.post_id = posts.id AND t.name {op} {v})"),
                "tags.visibility" => exists("EXISTS (SELECT 1 FROM posts_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.post_id = posts.id AND t.visibility {op} {v})"),
                "authors.slug" => exists("EXISTS (SELECT 1 FROM posts_authors pa JOIN users u ON u.id = pa.author_id WHERE pa.post_id = posts.id AND u.slug {op} {v})"),
                "authors.name" => exists("EXISTS (SELECT 1 FROM posts_authors pa JOIN users u ON u.id = pa.author_id WHERE pa.post_id = posts.id AND u.name {op} {v})"),
                "primary_tag.slug" => exists("EXISTS (SELECT 1 FROM posts_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.post_id = posts.id AND pt.sort_order = 0 AND t.slug {op} {v})"),
                "primary_author.slug" => exists("EXISTS (SELECT 1 FROM posts_authors pa JOIN users u ON u.id = pa.author_id WHERE pa.post_id = posts.id AND pa.sort_order = 0 AND u.slug {op} {v})"),
                _ => Err(unknown(key)),
            }
        }
        "tags" => match expanded {
            "id" | "slug" | "name" | "description" | "feature_image" | "visibility" | "parent_id"
            | "meta_title" | "meta_description" | "accent_color" | "created_at" | "updated_at" => {
                simple("tags", expanded, ColKind::Text)
            }
            "posts" => exists("EXISTS (SELECT 1 FROM posts_tags pt JOIN posts p ON p.id = pt.post_id WHERE pt.tag_id = tags.id AND p.status {op} {v})"),
            "count.posts" => Ok(Col::Simple {
                expr: "(SELECT COUNT(*) FROM posts_tags pt JOIN posts p ON p.id = pt.post_id WHERE pt.tag_id = tags.id AND p.status = 'published')".into(),
                kind: ColKind::Numeric,
            }),
            _ => Err(unknown(key)),
        },
        "users" | "authors" => match expanded {
            "id" | "slug" | "name" | "email" | "bio" | "website" | "location" | "facebook"
            | "twitter" | "status" | "visibility" | "locale" | "meta_title" | "meta_description"
            | "created_at" | "updated_at" | "last_seen" | "profile_image" => {
                simple("users", expanded, ColKind::Text)
            }
            "posts" => exists("EXISTS (SELECT 1 FROM posts_authors pa JOIN posts p ON p.id = pa.post_id WHERE pa.author_id = users.id AND p.status {op} {v})"),
            _ => Err(unknown(key)),
        },
        _ => Err(GhostError::BadRequest(format!("unsupported resource {resource}"))),
    }
}

fn sql_op(cmp: Cmp) -> &'static str {
    match cmp {
        Cmp::Eq => "=",
        Cmp::Ne => "<>",
        Cmp::Gt => ">",
        Cmp::Gte => ">=",
        Cmp::Lt => "<",
        Cmp::Lte => "<=",
        Cmp::Like => "LIKE",
    }
}

/// Compile an expression to a SQL WHERE fragment (without the `WHERE`
/// keyword). All values become positional `?` binds — nothing is
/// ever interpolated into the clause text.
pub fn compile(expr: &Expr, resource: &str) -> Result<Sql> {
    let mut sql = Sql::default();
    sql.clause = compile_expr(expr, resource, &mut sql.binds)?;
    Ok(sql)
}

fn compile_expr(expr: &Expr, resource: &str, binds: &mut Vec<Value>) -> Result<String> {
    match expr {
        Expr::And(parts) => {
            let mut frags = Vec::new();
            for p in parts {
                frags.push(format!("({})", compile_expr(p, resource, binds)?));
            }
            Ok(frags.join(" AND "))
        }
        Expr::Or(parts) => {
            let mut frags = Vec::new();
            for p in parts {
                frags.push(format!("({})", compile_expr(p, resource, binds)?));
            }
            Ok(frags.join(" OR "))
        }
        Expr::Cond(c) => compile_cond(c, resource, binds),
    }
}

fn compile_cond(c: &Cond, resource: &str, binds: &mut Vec<Value>) -> Result<String> {
    // key:null = "no value / not related"
    if c.value == NqlValue::Null {
        return compile_null(c, resource);
    }

    let col = column_for(resource, &c.key)?;
    let op = sql_op(c.cmp);

    let like_wrap = |v: &NqlValue| -> NqlValue {
        if c.cmp == Cmp::Like {
            match v {
                NqlValue::Str(s) => NqlValue::Str(format!("%{s}%")),
                other => other.clone(),
            }
        } else {
            v.clone()
        }
    };

    match col {
        Col::Simple { expr, kind } => match &c.value {
            NqlValue::List(items) => {
                if items.is_empty() {
                    return Ok("0 = 1".into());
                }
                let placeholders: Vec<&str> = items
                    .iter()
                    .map(|v| {
                        binds.push(like_wrap(v).to_bind());
                        "?"
                    })
                    .collect();
                Ok(format!("{expr} IN ({})", placeholders.join(", ")))
            }
            single => {
                if kind == ColKind::Bool {
                    let b = matches!(single, NqlValue::Bool(true))
                        || matches!(&single, NqlValue::Str(s) if s == "true" || s == "1");
                    binds.push(Value::Bool(b));
                } else {
                    binds.push(like_wrap(single).to_bind());
                }
                Ok(format!("{expr} {op} ?"))
            }
        },
        Col::Exists { sql: spec } => {
            let values: Vec<NqlValue> = match like_wrap(&c.value) {
                NqlValue::List(items) => items,
                single => vec![single],
            };
            let mut frags = Vec::new();
            for v in values {
                binds.push(v.to_bind());
                frags.push(spec.replace("{op}", op).replace("{v}", "?"));
            }
            if frags.len() == 1 {
                Ok(frags.pop().unwrap())
            } else {
                Ok(format!("({})", frags.join(" OR ")))
            }
        }
    }
}

/// Null comparisons: plain columns → `IS [NOT] NULL`; relation keys →
/// `[NOT] EXISTS (... join ...)` where the match column is dropped.
fn compile_null(c: &Cond, resource: &str) -> Result<String> {
    let not = c.cmp == Cmp::Eq; // key:null matches "absent"; Ne inverts
    let negate = |frag: &str| if not { format!("NOT ({frag})") } else { frag.to_string() };

    match (resource, c.key.as_str()) {
        (_, k) if matches!(k, "primary_tag" | "tag" | "tags" | "tags.slug" | "primary_tag.slug") => Ok(negate(
            "EXISTS (SELECT 1 FROM posts_tags pt WHERE pt.post_id = posts.id)",
        )),
        (_, k) if matches!(k, "primary_author" | "author" | "authors" | "authors.slug" | "primary_author.slug") => Ok(negate(
            "EXISTS (SELECT 1 FROM posts_authors pa WHERE pa.post_id = posts.id)",
        )),
        ("tags", "posts") => Ok(negate(
            "EXISTS (SELECT 1 FROM posts_tags pt JOIN posts p ON p.id = pt.post_id WHERE pt.tag_id = tags.id AND p.status = 'published')",
        )),
        _ => {
            let col = column_for(resource, &c.key)?;
            match col {
                Col::Simple { expr, .. } => {
                    if c.cmp == Cmp::Ne {
                        Ok(format!("{expr} IS NOT NULL"))
                    } else {
                        Ok(format!("{expr} IS NULL"))
                    }
                }
                Col::Exists { .. } => Ok(negate("1 = 1")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory evaluation (routing ownership checks)
// ---------------------------------------------------------------------------

/// Evaluate an expression against a serialized resource (post JSON).
pub fn evaluate(expr: &Expr, resource: &Value) -> bool {
    match expr {
        Expr::And(parts) => parts.iter().all(|p| evaluate(p, resource)),
        Expr::Or(parts) => parts.iter().any(|p| evaluate(p, resource)),
        Expr::Cond(c) => eval_cond(c, resource),
    }
}

fn json_str_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items
            .iter()
            .filter_map(|i| {
                match i {
                    Value::Object(o) => o
                        .get("slug")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                        .or_else(|| o.get("name").and_then(|s| s.as_str()).map(str::to_string)),
                    other => other.as_str().map(str::to_string),
                }
            })
            .collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn eval_cond(c: &Cond, resource: &Value) -> bool {
    let key = match c.key.as_str() {
        "primary_tag" | "tag" | "tags" => "tags",
        "primary_author" | "author" | "authors" => "authors",
        other => other,
    };

    if key == "tags" || key == "authors" {
        let list = json_str_list(resource.get(key).unwrap_or(&Value::Null));
        let target = match &c.value {
            NqlValue::Str(s) => s.clone(),
            _ => return false,
        };
        let matched = list.iter().any(|s| s.eq_ignore_ascii_case(&target));
        return match c.cmp {
            Cmp::Ne => !matched,
            _ => matched,
        };
    }

    let val = resource.get(key).unwrap_or(&Value::Null);
    let matched = match (&c.value, val) {
        (NqlValue::Str(s), Value::String(v)) => v.eq_ignore_ascii_case(s),
        (NqlValue::Num(n), Value::Number(v)) => v.as_f64() == Some(*n),
        (NqlValue::Bool(b), Value::Bool(v)) => b == v,
        (NqlValue::Null, Value::Null) => true,
        (NqlValue::Bool(b), Value::Null) => !*b, // absent treated as false
        (NqlValue::Str(s), Value::Bool(v)) => (s.eq_ignore_ascii_case("true") || s == "1") == *v,
        (NqlValue::Num(n), Value::Null) => *n == 0.0,
        _ => false,
    };
    match c.cmp {
        Cmp::Eq => matched,
        Cmp::Ne => !matched,
        _ => matched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple() {
        let e = parse("status:published+type:post").unwrap();
        assert!(matches!(e, Expr::And(_)));
    }

    #[test]
    fn parses_or_and_lists() {
        let e = parse("tags:[news,video],featured:true").unwrap();
        assert!(matches!(e, Expr::Or(_)));
    }

    #[test]
    fn parses_quoted_and_comparator() {
        let e = parse("title:~'hello world'+published_at:>2024-01-01").unwrap();
        assert!(matches!(e, Expr::And(_)));
    }

    #[test]
    fn parses_parens() {
        let e = parse("(status:published,featured:true)+type:post").unwrap();
        assert!(matches!(e, Expr::And(_)));
    }

    #[test]
    fn parses_negation() {
        let e = parse("tag:-news").unwrap();
        match e {
            Expr::Cond(c) => assert_eq!(c.cmp, Cmp::Ne),
            _ => panic!("expected cond"),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(":::").is_err());
        assert!(parse("").is_err());
        assert!(parse("key:value'").is_err());
    }

    #[test]
    fn compiles_to_parameterized_sql() {
        let e = parse("featured:true+tags.slug:news").unwrap();
        let sql = compile(&e, "posts").unwrap();
        assert_eq!(
            sql.clause,
            "(posts.featured = ?) AND (EXISTS (SELECT 1 FROM posts_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.post_id = posts.id AND t.slug = ?))"
        );
        assert_eq!(sql.binds.len(), 2);
    }

    #[test]
    fn compile_binds_values_never_interpolates() {
        // even with quotes and SQL keywords inside a quoted value, the clause
        // keeps a single ? and the payload lands in a bind
        let e = parse("slug:'x; DROP TABLE posts;--'").unwrap();
        let sql = compile(&e, "posts").unwrap();
        assert!(sql.clause.contains("posts.slug = ?"));
        assert!(!sql.clause.contains("DROP"));
        assert_eq!(sql.binds[0], serde_json::json!("x; DROP TABLE posts;--"));
    }

    #[test]
    fn unknown_key_rejected() {
        let e = parse("hax:x").unwrap();
        assert!(compile(&e, "posts").is_err());
    }

    #[test]
    fn evaluates_in_memory() {
        let post = serde_json::json!({
            "status": "published", "type": "post", "featured": true,
            "tags": [{"slug": "news", "name": "News"}],
            "authors": [{"slug": "ghost"}],
        });
        assert!(evaluate(&parse("status:published+type:post").unwrap(), &post));
        assert!(evaluate(&parse("tags:news").unwrap(), &post));
        assert!(!evaluate(&parse("featured:false").unwrap(), &post));
        assert!(!evaluate(&parse("tags:missing").unwrap(), &post));
        assert!(evaluate(&parse("tag:-missing").unwrap(), &post));
    }
}
