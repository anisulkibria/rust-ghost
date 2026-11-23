//! Member metafields: the custom fields a publisher defines on their members.
//!
//! The Rust side of Ghost's `@tryghost/metafield-types` (Ghost 6.72). That
//! package is the single source of truth for what a field type is and what a
//! valid value looks like, so Admin and core cannot disagree; this module is
//! that catalog, kept to the same vocabulary.
//!
//! A field has a *type*. Most types are a single value. An `address` is a
//! *record*: named parts, each a value in its own right. Storage keeps one
//! string per leaf, so a scalar occupies one row under the empty path and a
//! record occupies one row per part the member filled in.

/// The field types a publisher can choose. Ghost's `FIELD_TYPE_IDS`.
pub const FIELD_TYPE_IDS: &[&str] = &["short_text", "long_text", "address"];

/// What a part of a record type holds. Ghost's `PART_TYPE_IDS`.
pub const PART_TYPE_IDS: &[&str] = &["short_text", "postal_code", "country_code"];

/// Ghost's `MEMBER_ACCESS_LEVELS`, ordered: `write` implies `read`.
pub const MEMBER_ACCESS_LEVELS: &[&str] = &["none", "read", "write"];

/// Where a change was made, as a member's activity feed names it.
pub const CHANGE_SOURCES: &[&str] = &["admin", "admin_api", "portal", "import", "checkout"];

/// The namespace a site's own fields live in. A constant because the table
/// holds this namespace and nothing else, so the query layer supplies the name
/// the rows implicitly carry.
pub const CUSTOM_NAMESPACE: &str = "custom";

/// `namespace.key`, or `namespace.key.part` for one part of a record.
pub const SEPARATOR: char = '.';

/// Ghost's `MAX_LONG_TEXT_BYTES`.
pub const MAX_LONG_TEXT_BYTES: usize = 65535;
/// The bound on `short_text`, and on a record part of that type.
pub const MAX_SHORT_TEXT_CHARS: usize = 255;
/// Bounded well under a street address: no country's postal code is long.
pub const MAX_POSTAL_CODE_CHARS: usize = 32;

/// An address, modelled on Stripe's Address object, in the order it is shown.
/// Ghost's `FIELD_PARTS.address`.
///
/// Who the parcel is addressed to is deliberately absent: a parcel needs a
/// name as well as an address, but that is a fact about posting parcels rather
/// than about the type, and the name is often not the account name.
pub const ADDRESS_PARTS: &[(&str, &str, &str)] = &[
    ("line1", "Address line 1", "short_text"),
    ("line2", "Address line 2", "short_text"),
    ("city", "City", "short_text"),
    ("state", "State / Province", "short_text"),
    ("postal_code", "Postal code", "postal_code"),
    ("country", "Country", "country_code"),
];

/// The parts a type is made of, or `None` for a type whose value is a single
/// thing. Ghost's `subFieldsOf`.
pub fn parts_of(field_type: &str) -> Option<&'static [(&'static str, &'static str, &'static str)]> {
    match field_type {
        "address" => Some(ADDRESS_PARTS),
        _ => None,
    }
}

/// Whether this build recognises the type. A definition written by a newer
/// Ghost reads as a type with no parts rather than as an error.
pub fn is_known_type(field_type: &str) -> bool {
    FIELD_TYPE_IDS.contains(&field_type)
}

pub fn is_member_access(level: &str) -> bool {
    MEMBER_ACCESS_LEVELS.contains(&level)
}

/// Whether a member may see a field at this level. Ghost's `readableLevels`
/// for the `members` audience; staff read everything.
pub fn member_can_read(level: &str) -> bool {
    matches!(level, "read" | "write")
}

/// Whether a member may set it themselves. Ghost's `writableLevels`.
pub fn member_can_write(level: &str) -> bool {
    level == "write"
}

/// A field's key: lower-case letters, digits and underscores. Ghost's
/// `IDENTITY_SEGMENT`.
///
/// Dots are excluded because an identity is split on them by position —
/// `custom.shipping_address.country` only parses if a key can never contain
/// one.
pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Mint a key from a field's display name, the way Admin offers one.
///
/// Anything that is not a key character becomes an underscore, runs collapse,
/// and the edges are trimmed — so "Shipping address!" gives `shipping_address`.
/// An empty result is the caller's to reject: a name of only punctuation
/// cannot name a field.
pub fn key_from_name(name: &str) -> String {
    let mut out = String::new();
    let mut pending_underscore = false;
    for c in name.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_underscore && !out.is_empty() {
                out.push('_');
            }
            pending_underscore = false;
            out.push(c);
        } else {
            pending_underscore = true;
        }
    }
    out
}

/// `custom.company`, or `custom.shipping_address.country`.
pub fn format_identity(namespace: &str, key: &str, part: Option<&str>) -> String {
    match part {
        Some(p) => format!("{namespace}{SEPARATOR}{key}{SEPARATOR}{p}"),
        None => format!("{namespace}{SEPARATOR}{key}"),
    }
}

/// What is wrong with a value, in the sentence Ghost shows.
///
/// Each states what is expected rather than what went wrong, so one sentence
/// covers every way of breaking that rule.
pub fn validate_part(part_type: &str, value: &str) -> Result<String, &'static str> {
    // A write naming a part with an empty string is clearing it, which every
    // part allows regardless of its own rule — emptying is a statement about
    // the write, not about the part.
    if value.is_empty() {
        return Ok(String::new());
    }
    match part_type {
        "short_text" => {
            if value.chars().count() > MAX_SHORT_TEXT_CHARS {
                Err("Use 255 characters or fewer.")
            } else {
                Ok(value.to_string())
            }
        }
        "postal_code" => {
            if value.chars().count() > MAX_POSTAL_CODE_CHARS {
                Err("Use 32 characters or fewer.")
            } else {
                Ok(value.to_string())
            }
        }
        "country_code" => {
            // Two ASCII letters, checked on the way in rather than by length
            // on the way out: upper-casing does not preserve length (`ß`
            // becomes `SS`). Case is normalised so `gb` and `GB` are not two
            // values for one place, which a filter for either would half-miss.
            if value.len() == 2 && value.chars().all(|c| c.is_ascii_alphabetic()) {
                Ok(value.to_ascii_uppercase())
            } else {
                Err("Enter a 2-letter country code, like US.")
            }
        }
        "long_text" => {
            if value.len() > MAX_LONG_TEXT_BYTES {
                Err("Use 65535 bytes or fewer.")
            } else {
                Ok(value.to_string())
            }
        }
        // A type this build has never heard of is bounded by the widest rule
        // rather than refused: a definition written by a newer Ghost must not
        // make a member's existing value unsaveable.
        _ => {
            if value.len() > MAX_LONG_TEXT_BYTES {
                Err("Use 65535 bytes or fewer.")
            } else {
                Ok(value.to_string())
            }
        }
    }
}

/// Validate a scalar field's own value. A record's parts go through
/// [`validate_part`] individually.
pub fn validate_scalar(field_type: &str, value: &str) -> Result<String, &'static str> {
    validate_part(field_type, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_has_ghosts_six_parts_in_order() {
        let parts: Vec<&str> = parts_of("address").unwrap().iter().map(|(k, _, _)| *k).collect();
        assert_eq!(parts, ["line1", "line2", "city", "state", "postal_code", "country"]);
        // the scalar types are not records
        assert!(parts_of("short_text").is_none());
        assert!(parts_of("long_text").is_none());
    }

    #[test]
    fn keys_are_minted_the_way_admin_offers_them() {
        assert_eq!(key_from_name("Shipping address!"), "shipping_address");
        assert_eq!(key_from_name("  Company  Name "), "company_name");
        assert_eq!(key_from_name("T-Shirt Size"), "t_shirt_size");
        assert_eq!(key_from_name("VAT number 2"), "vat_number_2");
        // no leading or trailing underscore, and runs do not double up
        assert_eq!(key_from_name("--odd---name--"), "odd_name");
        // a name of only punctuation yields nothing for the caller to reject
        assert_eq!(key_from_name("!!!"), "");
    }

    #[test]
    fn a_key_may_not_contain_a_dot() {
        // identities split on dots by position, so a dotted key would break
        // `custom.key.part` parsing
        assert!(!is_valid_key("ship.ping"));
        assert!(!is_valid_key("Shipping"));
        assert!(!is_valid_key(""));
        assert!(is_valid_key("shipping_address"));
        assert!(is_valid_key("vat2"));
    }

    #[test]
    fn identities_read_as_ghost_writes_them() {
        assert_eq!(format_identity(CUSTOM_NAMESPACE, "company", None), "custom.company");
        assert_eq!(
            format_identity(CUSTOM_NAMESPACE, "shipping_address", Some("country")),
            "custom.shipping_address.country"
        );
    }

    #[test]
    fn every_part_can_be_cleared_whatever_its_rule() {
        // a format rule would otherwise leave its part the only one that could
        // be set but never removed
        assert_eq!(validate_part("country_code", "").unwrap(), "");
        assert_eq!(validate_part("postal_code", "").unwrap(), "");
    }

    #[test]
    fn country_codes_are_normalised_not_listed() {
        assert_eq!(validate_part("country_code", "gb").unwrap(), "GB");
        assert_eq!(validate_part("country_code", "Us").unwrap(), "US");
        assert!(validate_part("country_code", "GBR").is_err());
        assert!(validate_part("country_code", "1A").is_err());
    }

    #[test]
    fn text_bounds_match_ghosts() {
        assert!(validate_part("short_text", &"a".repeat(255)).is_ok());
        assert!(validate_part("short_text", &"a".repeat(256)).is_err());
        assert!(validate_part("postal_code", &"a".repeat(32)).is_ok());
        assert!(validate_part("postal_code", &"a".repeat(33)).is_err());
        assert!(validate_scalar("long_text", &"a".repeat(MAX_LONG_TEXT_BYTES)).is_ok());
        assert!(validate_scalar("long_text", &"a".repeat(MAX_LONG_TEXT_BYTES + 1)).is_err());
    }

    /// `short_text` is bounded in characters, not bytes, so a multi-byte
    /// string is not refused for being long in storage.
    #[test]
    fn short_text_counts_characters() {
        assert!(validate_part("short_text", &"é".repeat(255)).is_ok());
    }

    #[test]
    fn member_access_is_ordered() {
        assert!(!member_can_read("none"));
        assert!(member_can_read("read"));
        // write implies read
        assert!(member_can_read("write"));
        assert!(!member_can_write("read"));
        assert!(member_can_write("write"));
    }
}
