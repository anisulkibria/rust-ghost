//! Reading a column whose type differs between engines.
//!
//! This module exists because of one sharp edge in sqlx's `Any` driver, and
//! because of a schema decision worth keeping.
//!
//! Ghost's schema uses `TEXT` for everything long: a post's `mobiledoc`, its
//! `lexical`, its rendered `html`, a theme's source, an email's body. That is
//! upstream Ghost's own choice — `server/data/schema/schema.js` — and this
//! project mirrors it deliberately, so that a real Ghost database can be
//! imported and exported without translation.
//!
//! The edge: **MySQL sends a `TEXT` column as `MYSQL_TYPE_BLOB`.** Only the
//! column's charset distinguishes text from binary, and the `Any` layer does
//! not look at the charset. So `Any` hands back `AnyValueKind::Blob`, and
//! `String` decodes only from `AnyValueKind::Text`. A `TEXT` column therefore
//! migrates, inserts, and then fails the first time anything reads it.
//!
//! The obvious fix — make them all `VARCHAR` — does not work here. MySQL
//! reports any string wider than roughly 16,000 characters as a blob anyway,
//! including the output of `CAST(col AS CHAR(n))`, and a post's HTML is
//! routinely longer than that. There is no column type and no cast that lets
//! a long value arrive as text on MySQL.
//!
//! So instead of changing the schema to suit the driver, [`Text`] accepts
//! *either* representation. It decodes from text on SQLite and from a blob on
//! MySQL, and callers see a `String` on both.
use sqlx::any::AnyTypeInfo;
use sqlx::decode::Decode;
use sqlx::encode::{Encode, IsNull};
use sqlx::error::BoxDynError;
use sqlx::{Any, Type};

/// A string column that may arrive as text or as bytes.
///
/// Use for any column declared `TEXT` (or `MEDIUMTEXT`/`LONGTEXT`) in the
/// MySQL schema. A short, indexed column is a `VARCHAR` on MySQL and arrives
/// as text on both engines, so it needs nothing special — plain `String` is
/// correct and clearer there.
///
/// Works in a `#[derive(sqlx::FromRow)]` struct as well as in
/// `row.get::<Text, _>("html")`, which is what keeps the call sites honest:
/// the type in the struct says which columns are long.
/// `serde(transparent)` so it serialises as a plain string. Without it a
/// column read through this type would appear in an API response as a wrapper
/// object, which would be a visible change in the JSON Ghost's clients expect.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Text(pub String);

impl Text {
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<Text> for String {
    fn from(t: Text) -> String {
        t.0
    }
}

impl From<String> for Text {
    fn from(s: String) -> Text {
        Text(s)
    }
}

impl std::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl Type<Any> for Text {
    fn type_info() -> AnyTypeInfo {
        <String as Type<Any>>::type_info()
    }

    /// Accepts both representations.
    ///
    /// `try_get` checks this before decoding, so without it a blob column
    /// would be refused before [`Decode`] ever ran — which is the whole
    /// problem this type solves.
    fn compatible(ty: &AnyTypeInfo) -> bool {
        <String as Type<Any>>::compatible(ty) || <Vec<u8> as Type<Any>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Any> for Text {
    fn decode(value: <Any as sqlx::Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        // Text first, because that is the SQLite path and the common one.
        // `AnyValueRef` is `Clone`, which is what makes trying twice possible
        // — the alternative would be inspecting `AnyValueKind`, and that
        // field is private to sqlx.
        if let Ok(s) = <String as Decode<Any>>::decode(value.clone()) {
            return Ok(Text(s));
        }
        let bytes = <Vec<u8> as Decode<Any>>::decode(value)?;
        // Lossy rather than strict: a single bad byte in one post's body must
        // not fail the query that renders a page. Ghost's own columns are
        // utf8mb4 and this should never trigger, but a database imported from
        // elsewhere is not something this project controls.
        Ok(Text(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

impl<'q> Encode<'q, Any> for Text {
    fn encode_by_ref(
        &self,
        buf: &mut <Any as sqlx::Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, BoxDynError> {
        // Always written as text. Both engines accept a text parameter for a
        // `TEXT` column; it is only the *read* direction that diverges.
        <String as Encode<'q, Any>>::encode_by_ref(&self.0, buf)
    }
}

/// Read a long column — one held as `MEDIUMTEXT` on MySQL — as a `String`.
///
/// MySQL sends `MEDIUMTEXT` as a blob, so a plain `String` will not decode it;
/// [`Text`] accepts either representation. This wrapper exists so that a
/// hand-written [`sqlx::FromRow`] can keep its struct field as `String` and
/// convert here, which means nothing above the row mapping has to know which
/// columns are long.
///
/// Use it for the columns listed as `MEDIUMTEXT` in
/// `migrations/mysql/0001_init.sql` — post and email bodies, editor formats,
/// code injection, theme settings. A short `VARCHAR` arrives as text on both
/// engines and needs nothing.
pub fn long(row: &sqlx::any::AnyRow, column: &str) -> sqlx::Result<String> {
    use sqlx::Row;
    row.try_get::<Text, _>(column).map(|t| t.0)
}

/// [`long`] for a nullable column, which most of them are.
pub fn long_opt(row: &sqlx::any::AnyRow, column: &str) -> sqlx::Result<Option<String>> {
    use sqlx::Row;
    row.try_get::<Option<Text>, _>(column).map(|t| t.map(|t| t.0))
}

/// Read a boolean column.
///
/// Booleans are stored as 0/1 integers — `INTEGER` on SQLite, `SMALLINT` on
/// MySQL — and sqlx's `Any` driver cannot produce a Rust `bool` from either.
/// `bool` decodes only from `AnyValueKind::Bool`, and neither driver ever
/// yields that: SQLite maps its integers to `BigInt` and MySQL maps
/// `SMALLINT` to `SmallInt`.
///
/// Note that the schema *cannot* be changed to fix this. Declaring a SQLite
/// column `BOOLEAN` makes its driver report `DataType::Bool`, which `Any`
/// rejects outright rather than mapping — so a boolean column has to be an
/// integer, and reading it has to go through here.
///
/// Anything non-zero is true, matching SQLite's own truthiness and what every
/// writer in this project binds.
pub fn flag(row: &sqlx::any::AnyRow, column: &str) -> bool {
    use sqlx::Row;
    row.try_get::<i64, _>(column).unwrap_or(0) != 0
}

/// [`flag`] for a nullable boolean column, where absent is not false.
pub fn flag_opt(row: &sqlx::any::AnyRow, column: &str) -> Option<bool> {
    use sqlx::Row;
    row.try_get::<Option<i64>, _>(column).ok().flatten().map(|v| v != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_derefs_to_its_string() {
        let t = Text("hello".into());
        assert_eq!(&*t, "hello");
        assert!(t.starts_with("hel"));
    }

    #[test]
    fn it_converts_both_ways() {
        let t: Text = String::from("x").into();
        let s: String = t.clone().into();
        assert_eq!(s, "x");
        assert_eq!(t.into_string(), "x");
    }

    /// The point of the type: it must accept the blob representation, or
    /// `try_get` refuses a MySQL `TEXT` column before decoding begins.
    #[test]
    fn it_accepts_text_and_blob_columns() {
        use sqlx::any::AnyTypeInfoKind;
        let text = AnyTypeInfo { kind: AnyTypeInfoKind::Text };
        let blob = AnyTypeInfo { kind: AnyTypeInfoKind::Blob };
        assert!(<Text as Type<Any>>::compatible(&text), "SQLite's representation");
        assert!(<Text as Type<Any>>::compatible(&blob), "MySQL's representation");
        // And a plain String must still refuse the blob — otherwise this type
        // would be unnecessary and the module docs would be wrong.
        assert!(!<String as Type<Any>>::compatible(&blob));
    }
}
