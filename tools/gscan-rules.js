// Emits the GS001/GS030/GS051/GS060/GS070/GS080 rule table for the Rust port,
// read straight out of the real gscan 6.6.1 merged v6 spec so the codes,
// levels, wording and regexes cannot drift from upstream by transcription.
const GSCAN = process.argv[2];
const spec = require(GSCAN + '/lib/specs').get(['v6']);

// gscan's `rule`/`details` are HTML, for its own report UI. The Rust report
// renders text, so tags come out and entities come back.
function text(html) {
  return String(html || '')
    .replace(/<br\s*\/?>/gi, ' ')
    .replace(/<[^>]+>/g, '')
    .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'").replace(/&amp;/g, '&')
    .replace(/\s+/g, ' ')
    .trim();
}

function rustStr(s) {
  return '"' + String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';
}

// JS regex source -> Rust `regex` crate source. Only the differences that
// actually occur in this rule set are handled; anything else is a hard error
// rather than a silently wrong rule.

// JavaScript lets a bare `{` stand for itself when it cannot start a
// quantifier, which is why gscan writes `{{helper}}` directly. Rust's `regex`
// is stricter: an unescaped brace that is not a `{n,m}` repetition is an
// error, and 93 of these 107 patterns are literal Handlebars braces. So every
// brace that is not part of a real quantifier gets escaped.
//
// Doing this by rewrite rather than by hand-editing the table is the point:
// the patterns stay byte-identical to upstream's apart from a mechanical,
// reviewable transformation.
function escapeBraces(source) {
  let out = '';
  let inClass = false;
  for (let i = 0; i < source.length; i++) {
    const c = source[i];
    if (c === '\\') {
      out += c + (source[i + 1] || '');
      i++;
      continue;
    }
    if (c === '[') { inClass = true; out += c; continue; }
    if (c === ']') { inClass = false; out += c; continue; }
    if (c === '{' && !inClass) {
      // A quantifier is `{n}`, `{n,}` or `{n,m}` and nothing else.
      const quantifier = /^\{\d+(,\d*)?\}/.exec(source.slice(i));
      if (quantifier) { out += quantifier[0]; i += quantifier[0].length - 1; continue; }
      out += '\\{';
      continue;
    }
    if (c === '}' && !inClass) { out += '\\}'; continue; }
    out += c;
  }
  return out;
}

function rustRegex(code, re) {
  let s = re.source;
  if (/\(\?<[=!]/.test(s)) throw new Error(code + ': lookbehind');
  if (/\(\?[=!]/.test(s)) throw new Error(code + ': lookahead');
  if (/\\\d/.test(s)) throw new Error(code + ': backreference');
  // `\/` is a JS literal-delimiter escape and means nothing in Rust.
  s = s.replace(/\\\//g, '/');
  s = escapeBraces(s);
  const flags = (re.ignoreCase ? 'i' : '') + (re.multiline ? 'm' : '') + (re.dotAll ? 's' : '');
  return flags ? '(?' + flags + ')' + s : s;
}

// Two rules' tests are not one regex, and are implemented against their code
// in `gscan.rs` instead: GS030-ASSET-REQ needs "an /assets/ url that is not
// absolute", and GS051-CUSTOM-FONTS needs "both custom-font properties are
// present somewhere in the file". Both are negative/conjunctive lookaheads,
// which Rust's `regex` deliberately does not support. Listed explicitly so a
// rule that grows a lookahead in a later gscan fails the build rather than
// being quietly dropped.
// GS080-NO-EMPTY-TRANSLATIONS joins them for a different reason: its regex
// uses a backreference, and the port already recognises an empty `{{t}}`
// through its own argument parsing, which also catches `{{t  }}` forms the
// regex misses.
const HAND_WRITTEN = new Set([
    'GS030-ASSET-REQ', 'GS051-CUSTOM-FONTS', 'GS080-NO-EMPTY-TRANSLATIONS',
]);

// Every rule in the spec, not a chosen subset: the code-driven checks look
// their own level and wording up here too, so a theme author reads upstream's
// words whichever check fired.
const PREFIXES = /^GS/;
const rows = [];
for (const [code, r] of Object.entries(spec.rules)) {
  if (!PREFIXES.test(code)) continue;
  rows.push({
    code,
    level: { error: 'Error', warning: 'Warning', recommendation: 'Recommendation', feature: 'Feature' }[r.level] || 'Warning',
    fatal: !!r.fatal,
    rule: text(r.rule),
    details: text(r.details),
    regex: (r.regex && !HAND_WRITTEN.has(code)) ? rustRegex(code, r.regex) : null,
    helper: r.helper || null,
    css: !!r.css,
    notValidIn: r.notValidIn || null,
    validInAPI: r.validInAPI || null,
    className: r.className || null,
    cardAsset: r.cardAsset || null,
  });
}
rows.sort((a, b) => a.code.localeCompare(b.code));

// The `class_name` a GS050 Koenig rule is about, and the `card_asset` tag that
// decides whether it applies to a theme using Ghost's own card stylesheets.
const out = [];
out.push('//! Generated from gscan 6.6.1\'s merged v6 spec — do not hand-edit.');
out.push('//!');
out.push('//! Produced by `tools/gscan-rules.js`, which reads the real tool\'s own');
out.push('//! `lib/specs/v6` (itself the union of v1 through v6) and emits this table.');
out.push('//! Transcribing 100-odd codes, levels, regexes and wordings by hand is how a');
out.push('//! port drifts from the tool it claims to match, so none of it is transcribed.');
out.push('//!');
out.push('//! The rule and detail texts are gscan\'s HTML flattened to plain text, since');
out.push('//! this port renders its report as text rather than into gscan\'s own UI.');
out.push('');
out.push('use crate::gscan::Level;');
out.push('');
out.push('/// One regex-driven rule: the checks that are a pattern plus a message.');
out.push('pub struct Rule {');
out.push('    pub code: &\'static str,');
out.push('    pub level: Level,');
out.push('    pub fatal: bool,');
out.push('    pub rule: &\'static str,');
out.push('    pub details: &\'static str,');
out.push('    /// `None` where the test is not a single pattern — either upstream');
out.push('    /// has no regex for it, or its regex needs a lookaround or');
out.push('    /// backreference Rust\'s `regex` does not do. Those are implemented in');
out.push('    /// `gscan.rs` against this same code, and still take their level and');
out.push('    /// wording from here.');
out.push('    pub regex: Option<&\'static str>,');
out.push('    /// What to name in the failure message, when upstream names something.');
out.push('    pub helper: Option<&\'static str>,');
out.push('    /// Searches stylesheets rather than templates.');
out.push('    pub css: bool,');
out.push('    /// A template this rule does not apply to — the one rule that is');
out.push('    /// legitimate in the very file it would otherwise flag.');
out.push('    pub not_valid_in: Option<&\'static str>,');
out.push('    /// Only meaningful for themes declaring one of these `ghost-api`');
out.push('    /// versions. A v5 theme is not warned about a v3 behaviour change.');
out.push('    pub valid_in_api: Option<&\'static [&\'static str]>,');
out.push('    /// The CSS class a GS050 Koenig rule is about.');
out.push('    pub class_name: Option<&\'static str>,');
out.push('    /// Which of Ghost\'s card stylesheets supplies this rule\'s CSS. A');
out.push('    /// theme that opts into card assets is not asked to style it itself.');
out.push('    pub card_asset: Option<&\'static str>,');
out.push('}');
out.push('');

for (const row of rows) {
  if (row.validInAPI) {
    out.push(`const ${row.code.replace(/[^A-Z0-9]/g, '_')}_API: &[&str] = &[${row.validInAPI.map(rustStr).join(', ')}];`);
  }
}
out.push('');
out.push(`/// Every rule gscan's v6 spec carries, ${rows.length} of them.`);
out.push('///');
out.push('/// Sorted by code, which is also how a report reads.');
out.push('pub const RULES: &[Rule] = &[');
for (const row of rows) {
  out.push('    Rule {');
  out.push(`        code: ${rustStr(row.code)},`);
  out.push(`        level: Level::${row.level},`);
  out.push(`        fatal: ${row.fatal},`);
  out.push(`        rule: ${rustStr(row.rule)},`);
  out.push(`        details: ${rustStr(row.details)},`);
  out.push(`        regex: ${row.regex === null ? 'None' : 'Some(' + rustStr(row.regex) + ')'},`);
  out.push(`        helper: ${row.helper === null ? 'None' : 'Some(' + rustStr(row.helper) + ')'},`);
  out.push(`        css: ${row.css},`);
  out.push(`        not_valid_in: ${row.notValidIn === null ? 'None' : 'Some(' + rustStr(row.notValidIn) + ')'},`);
  out.push(`        valid_in_api: ${row.validInAPI === null ? 'None' : 'Some(' + row.code.replace(/[^A-Z0-9]/g, '_') + '_API)'},`);
  out.push(`        class_name: ${row.className === null ? 'None' : 'Some(' + rustStr(row.className) + ')'},`);
  out.push(`        card_asset: ${row.cardAsset === null ? 'None' : 'Some(' + rustStr(row.cardAsset) + ')'},`);
  out.push('    },');
}
out.push('];');
out.push('');
out.push('/// Every helper gscan\'s v6 spec knows, for GS120-NO-UNKNOWN-GLOBALS.');
out.push(`pub const KNOWN_HELPERS: &[&str] = &[${spec.knownHelpers.slice().sort().map(rustStr).join(', ')}];`);
out.push('');
out.push('/// The `{{@page}}` properties a v6 theme may read, for GS110.');
out.push(`pub const PAGE_BUILDER_PROPERTIES: &[&str] = &[${spec.pageBuilderProperties.map(rustStr).join(', ')}];`);
out.push('');
console.log(out.join('\n'));
