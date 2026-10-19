//! CSS colour parsing for `{{color_to_rgba}}` and `{{contrast_text_color}}`.
//!
//! Ghost leans on the `color` npm package via `@tryghost/color-utils`; this
//! covers the notations a theme setting or accent colour can actually hold:
//! hex (3/4/6/8 digits), `rgb()`/`rgba()`, `hsl()`/`hsla()` and CSS named
//! colours. Anything unparseable falls back the way Ghost's helpers do.

/// Ghost's fallback background when a helper is handed a non-colour.
pub const DEFAULT_BACKGROUND: &str = "#15171A";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f64,
}

/// Parse a CSS colour string. Returns `None` when the notation is not one we
/// understand — callers map that onto their own documented fallback.
pub fn parse(input: &str) -> Option<Rgba> {
    let s = input.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(rest) = s.strip_prefix("rgba").or_else(|| s.strip_prefix("rgb")) {
        return parse_rgb_fn(rest);
    }
    if let Some(rest) = s.strip_prefix("hsla").or_else(|| s.strip_prefix("hsl")) {
        return parse_hsl_fn(rest);
    }
    // bare hex without '#' is what Ghost stores for accent_color in some
    // older installs
    if s.len() == 3 || s.len() == 6 {
        if let Some(c) = parse_hex(s) {
            return Some(c);
        }
    }
    named(&s.to_ascii_lowercase())
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    let h = hex.trim();
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let dup = |c: u8| -> u8 {
        let v = (c as char).to_digit(16).unwrap_or(0) as u8;
        v * 17
    };
    let b = h.as_bytes();
    let pair = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(match b.len() {
        3 => Rgba { r: dup(b[0]), g: dup(b[1]), b: dup(b[2]), a: 1.0 },
        4 => Rgba { r: dup(b[0]), g: dup(b[1]), b: dup(b[2]), a: dup(b[3]) as f64 / 255.0 },
        6 => Rgba { r: pair(0)?, g: pair(2)?, b: pair(4)?, a: 1.0 },
        8 => Rgba { r: pair(0)?, g: pair(2)?, b: pair(4)?, a: pair(6)? as f64 / 255.0 },
        _ => return None,
    })
}

/// Split the `(…)` argument list of a colour function on commas or spaces
/// (CSS Color 4 allows both).
fn fn_args(rest: &str) -> Option<Vec<String>> {
    let inner = rest.trim().strip_prefix('(')?.strip_suffix(')')?;
    Some(
        inner
            .replace('/', " ")
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// A channel given either as 0-255 or as a percentage.
fn channel(s: &str) -> Option<u8> {
    let v = if let Some(p) = s.strip_suffix('%') {
        p.trim().parse::<f64>().ok()? * 255.0 / 100.0
    } else {
        s.parse::<f64>().ok()?
    };
    Some(v.round().clamp(0.0, 255.0) as u8)
}

fn alpha(s: &str) -> f64 {
    let v = if let Some(p) = s.strip_suffix('%') {
        p.trim().parse::<f64>().unwrap_or(100.0) / 100.0
    } else {
        s.parse::<f64>().unwrap_or(1.0)
    };
    v.clamp(0.0, 1.0)
}

fn parse_rgb_fn(rest: &str) -> Option<Rgba> {
    let args = fn_args(rest)?;
    if args.len() < 3 {
        return None;
    }
    Some(Rgba {
        r: channel(&args[0])?,
        g: channel(&args[1])?,
        b: channel(&args[2])?,
        a: args.get(3).map(|a| alpha(a)).unwrap_or(1.0),
    })
}

fn parse_hsl_fn(rest: &str) -> Option<Rgba> {
    let args = fn_args(rest)?;
    if args.len() < 3 {
        return None;
    }
    let h = args[0].trim_end_matches("deg").parse::<f64>().ok()?.rem_euclid(360.0);
    let pct = |s: &str| s.trim_end_matches('%').parse::<f64>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0));
    let (s, l) = (pct(&args[1])?, pct(&args[2])?);

    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Some(Rgba {
        r: to8(r1),
        g: to8(g1),
        b: to8(b1),
        a: args.get(3).map(|a| alpha(a)).unwrap_or(1.0),
    })
}

/// `{{color_to_rgba color alpha}}` — the `color` package's `.rgb().string()`
/// output: `rgb(r, g, b)` at full opacity, `rgba(r, g, b, a)` otherwise.
pub fn to_rgba_string(color: &str, alpha_arg: Option<f64>) -> String {
    let base = if color.trim().is_empty() { DEFAULT_BACKGROUND } else { color.trim() };
    // a non-finite / absent alpha falls back to Ghost's 0.25
    let a = alpha_arg.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).unwrap_or(0.25);
    let Some(c) = parse(base) else {
        return "rgba(21, 23, 26, 0.25)".to_string();
    };
    if (a - 1.0).abs() < f64::EPSILON {
        format!("rgb({}, {}, {})", c.r, c.g, c.b)
    } else {
        format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, trim_float(a))
    }
}

/// `{{contrast_text_color background}}` — the YIQ split Ghost and Portal share.
pub fn contrast_text_color(color: &str) -> String {
    let base = if color.trim().is_empty() { DEFAULT_BACKGROUND } else { color.trim() };
    let Some(c) = parse(base) else {
        return "#FFFFFF".to_string();
    };
    let yiq = c.r as f64 * 0.299 + c.g as f64 * 0.587 + c.b as f64 * 0.114;
    if yiq >= 186.0 { "#000000".into() } else { "#FFFFFF".into() }
}

/// Render a float the way JSON.stringify would: no trailing zeros.
fn trim_float(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// The CSS named colours a theme setting realistically carries.
fn named(name: &str) -> Option<Rgba> {
    let (r, g, b) = match name {
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "purple" => (128, 0, 128),
        "fuchsia" | "magenta" => (255, 0, 255),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "olive" => (128, 128, 0),
        "yellow" => (255, 255, 0),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "orange" => (255, 165, 0),
        "pink" => (255, 192, 203),
        "transparent" => return Some(Rgba { r: 0, g: 0, b: 0, a: 0.0 }),
        _ => return None,
    };
    Some(Rgba { r, g, b, a: 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(parse("#fff"), Some(Rgba { r: 255, g: 255, b: 255, a: 1.0 }));
        assert_eq!(parse("#15171A"), Some(Rgba { r: 21, g: 23, b: 26, a: 1.0 }));
        assert_eq!(parse("  #15171a  "), Some(Rgba { r: 21, g: 23, b: 26, a: 1.0 }));
        assert_eq!(parse("#15171Aff").map(|c| c.a), Some(1.0));
    }

    #[test]
    fn parses_functional_forms() {
        assert_eq!(parse("rgb(21, 23, 26)"), Some(Rgba { r: 21, g: 23, b: 26, a: 1.0 }));
        assert_eq!(parse("rgba(21,23,26,0.5)").map(|c| c.a), Some(0.5));
        assert_eq!(parse("hsl(0, 100%, 50%)"), Some(Rgba { r: 255, g: 0, b: 0, a: 1.0 }));
        assert_eq!(parse("red"), Some(Rgba { r: 255, g: 0, b: 0, a: 1.0 }));
    }

    #[test]
    fn rgba_string_matches_ghost() {
        assert_eq!(to_rgba_string("#15171A", Some(0.25)), "rgba(21, 23, 26, 0.25)");
        assert_eq!(to_rgba_string("#15171A", Some(1.0)), "rgb(21, 23, 26)");
        // absent / unparseable alpha uses Ghost's 0.25 default
        assert_eq!(to_rgba_string("#ffffff", None), "rgba(255, 255, 255, 0.25)");
        // out of range alpha is clamped, not rejected
        assert_eq!(to_rgba_string("#ffffff", Some(5.0)), "rgb(255, 255, 255)");
        // an unparseable colour yields Ghost's literal fallback
        assert_eq!(to_rgba_string("not-a-color", Some(0.5)), "rgba(21, 23, 26, 0.25)");
        // an empty colour falls back to the default background first
        assert_eq!(to_rgba_string("  ", Some(0.5)), "rgba(21, 23, 26, 0.5)");
    }

    #[test]
    fn contrast_follows_yiq_threshold() {
        assert_eq!(contrast_text_color("#ffffff"), "#000000");
        assert_eq!(contrast_text_color("#15171A"), "#FFFFFF");
        assert_eq!(contrast_text_color(""), "#FFFFFF");
        assert_eq!(contrast_text_color("nonsense"), "#FFFFFF");
    }
}
