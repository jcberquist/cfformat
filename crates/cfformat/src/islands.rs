//! Formatting of `<script>` / `<style>` islands: which option an island goes
//! to, the text it is handed, the per-run cache, and for an island holding
//! `#…#` and `##` the stand-ins it is handed in their place and the checks
//! that put them back ([`holes`]).
//!
//! The printer (`print/islands.rs`) decides *whether* an island is handed off
//! and splices the result; everything about the formatter lives here. An
//! `islands.*` key is `"oxc"`, formatted in process ([`Oxc`]), or `"off"`,
//! printed verbatim by the printer; there is nothing else.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cfparse::{Island, IslandSite, Lang};

use crate::options::{IslandConfigMode, IslandPreset, Options};
use crate::IndentStyle;

mod config;
pub(crate) mod holes;
mod oxc;

use holes::Site;

pub use config::IslandConfig;
pub use oxc::Oxc;

/// The stack of the thread [`Oxc`] formats on (and walks its output for
/// [`literal_lines`] on, for [`Islands`]) and [`literal_lines`] and
/// [`literal_texts`] walk on: 1 GB. The oxc parsers and formatters, and
/// the walks over their trees, recurse once per level of nesting with no
/// bound of their own, so their depth is bounded by the stack; this one is
/// 128 times the CLI's 8 MB worker, whatever thread the caller is on. It is
/// address space, not memory: a thread touches only as much of it as the
/// island is deep. The size is set by the deepest parse the limits let
/// through, an operator chain at [`SIZE_LIMIT`]: on Linux it needs over
/// 64 MB in a release build and over 128 MB in a debug build, and Windows
/// frames are larger still (256 MB overflowed there in debug).
pub const ISLAND_STACK: usize = 1 << 30;

/// The name of that thread. The CLI's panic hook is quiet for it, as for
/// its workers: a panic there is reported as the island's warning.
pub const ISLAND_THREAD: &str = "cfformat-island";

/// The deepest nesting [`Oxc`] formats: 500. For JavaScript it is the
/// depth of the parsed tree ([`tree_depth`]), every node counted — a
/// bracket, an unbraced `if` or loop body, a JSX element, an operator, a
/// call or a member alike; for CSS and JSON, the depth of their brackets
/// outside strings and comments ([`nesting_depth`]). The bracket depth is
/// checked first for all three, on the text: it is never below a
/// stylesheet's nesting, nor below that of a JSON text the JSON formatter
/// takes, so their parsers never see deeper brackets; the tree after the
/// parse, before the formatter. A
/// deeper island is refused with a warning (`nested N levels deep, over
/// the limit of 500`, `N` the count that tripped) and prints as written.
/// The deepest island in the test corpora nests 15 brackets; on the CLI's 8 MB
/// worker alone the first overflow of a bracket shape was at 577 (CSS
/// `@media{`), and the [`ISLAND_STACK`] thread has 32 times that room.
pub const NESTING_LIMIT: usize = 500;

/// The largest hand-off text ([`hand_off_text`]) [`Oxc`] formats, in
/// bytes: 256 KB. A larger island is refused with a warning and prints as
/// written. It bounds what runs before [`NESTING_LIMIT`] can count the
/// tree — the parse of a chain of operators, which nests without a bracket
/// — and the formatter's output, which for deep nesting grows faster than
/// the input. The largest island in the test corpora is 26 KB.
pub const SIZE_LIMIT: usize = 256 << 10;

/// The deepest nesting of `(`, `[` and `{` in `text`, an island of `lang`:
/// the running count of openers less closers, never below zero, at its
/// highest. An opener counts wherever it is, in a string, a comment or a
/// regular expression too; a closer counts only where the scan is sure it
/// is code. So for a well-formed text the count is the language's nesting
/// or above it, never below: a closer hidden in a literal cannot cancel a
/// real opener. The literal forms, by language:
///
/// - **CSS:** `"…"` and `'…'` strings (`\` escapes a byte; an unescaped
///   line feed ends one, where the tokenizer ends a bad string),
///   `/* … */` comments, a `\` escape anywhere (an escaped closer is part
///   of a name), and the unquoted body of a `url(` — under a vendor
///   prefix too, `url-prefix(` and `domain(`, and any function whose name
///   holds an escape, which may spell one — up to its `)`. Brackets are
///   matched by kind, as the parser matches them: a block or a function's
///   arguments run to their own closer, and a closer of another kind
///   inside them is a token (`foo(})` closes no rule), so only the
///   innermost open bracket's own closer closes it.
/// - **JavaScript, and JSON**, which oxc parses with its JavaScript
///   parser: `"…"` and `'…'` strings (an unescaped line break ends one),
///   `/* … */` and `// …` comments, `<!--` to the end of the line and a
///   line that starts with `-->`, template literals (the text up to each
///   `${` and the closing backtick; the code inside `${…}` up to its `}`),
///   and regular expressions, up to the next unescaped `/` outside a
///   `[…]` class or the end of the line. A `/` starts one where the
///   previous significant byte is none or one of
///   `( , = : [ ! & | ? { } ; + - * % < > ~ ^`, and divides after
///   anything else. That is a heuristic, and some texts it misreads —
///   `return /]/` and `typeof /]/` read as division, `a++ / b` as a
///   regular expression, JSX text as code — may count below the parser's
///   nesting. A script's parsed tree is bounded after the parse
///   ([`tree_depth`]); JSON with a `/` outside its strings and comments
///   does not format (the JSON formatter refuses a regular expression and
///   a division alike), so for a JSON text it takes the count is never
///   below its nesting.
/// - **Any other language** (never handed off): the JavaScript rules.
///
/// One pass over the bytes, deciding on ASCII bytes only (every delimiter
/// is one, and no byte of a multi-byte character is); nothing is
/// allocated but a stack: the open brackets of a stylesheet, or the
/// `${…}` a script's scan is inside.
pub fn nesting_depth(text: &str, lang: Lang) -> usize {
    scan(text.as_bytes(), lang, &mut |_, _| {})
}

/// The scan [`nesting_depth`] makes, which also hands `visit` each literal
/// region it reads — a string, a comment, a template literal's text, a
/// regular expression, an unquoted `url(…)` body — with its range, in
/// order (the interpolated islands of [`holes`] classify their `#…#` by
/// them). Everything outside the regions is code. Returns the depth.
fn scan(b: &[u8], lang: Lang, visit: &mut impl FnMut(Site, Range<usize>)) -> usize {
    match lang {
        Lang::Css => css_nesting_depth(b, visit),
        _ => js_nesting_depth(b, visit),
    }
}

/// The running count [`nesting_depth`] keeps.
#[derive(Default)]
struct Brackets {
    depth: usize,
    deepest: usize,
}

impl Brackets {
    /// An opener, wherever it is.
    fn open(&mut self) {
        self.depth += 1;
        self.deepest = self.deepest.max(self.depth);
    }

    /// A closer in code.
    fn close(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// A byte of literal text: an opener counts, a closer does not.
    fn literal(&mut self, c: u8) {
        if matches!(c, b'(' | b'[' | b'{') {
            self.open();
        }
    }
}

/// [`nesting_depth`] of a stylesheet. The brackets open in code are kept
/// by kind: a closer closes the innermost only when it is of its kind
/// ([`css_close`]), as oxc-css-parser reads a block or a function's
/// arguments to their own closer and keeps any other closer inside them
/// as a token (`parse_raw_function`). Before that, `.a{b:foo(});`×n
/// counted each `}` as the rule's closer, about 2 against n, and at the
/// size limit the page aborted the process.
fn css_nesting_depth(b: &[u8], visit: &mut impl FnMut(Site, Range<usize>)) -> usize {
    let mut n = Brackets::default();
    // The openers in code still open, the innermost last.
    let mut open = Vec::new();
    // Where the name that ends here started: a function's, when `(` follows.
    let mut name = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = block_comment(b, i + 2, &mut n);
            visit(Site::Comment, i..end);
            i = end;
            name = None;
            continue;
        }
        match c {
            b'"' | b'\'' => {
                let end = css_string(b, i + 1, c, &mut n);
                visit(Site::String(c), i..end);
                i = end;
                name = None;
            }
            b'\\' => {
                name.get_or_insert(i);
                i = css_escape(b, i + 1, &mut n);
            }
            b'(' => {
                n.open();
                open.push(c);
                let url = name.is_some_and(|start| url_function(&b[start..i]));
                name = None;
                i = if url {
                    // A quoted URL is read as code holding a string: the
                    // scan returns at once.
                    let end = css_url(b, i + 1, &mut n, &mut open);
                    if end > i + 1 {
                        visit(Site::Url, i + 1..end);
                    }
                    end
                } else {
                    i + 1
                };
            }
            b'[' | b'{' => {
                n.open();
                open.push(c);
                name = None;
                i += 1;
            }
            b')' | b']' | b'}' => {
                css_close(c, &mut n, &mut open);
                name = None;
                i += 1;
            }
            c if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_') || c >= 0x80 => {
                name.get_or_insert(i);
                i += 1;
            }
            _ => {
                name = None;
                i += 1;
            }
        }
    }
    n.deepest
}

/// A closer `c` in CSS code: it closes the innermost bracket still open
/// (`open`, by opener) when that is of its kind, and is a token otherwise
/// — inside a block or a function's arguments only their own closer
/// closes them, and with none open it closes nothing.
fn css_close(c: u8, n: &mut Brackets, open: &mut Vec<u8>) {
    let opener = match c {
        b')' => b'(',
        b']' => b'[',
        _ => b'{',
    };
    if open.last() == Some(&opener) {
        open.pop();
        n.close();
    }
}

/// Whether a CSS function named `name` may take an unquoted URL: `url`,
/// `url-prefix` or `domain` (oxc's `Url`), under a vendor prefix too, or a
/// name with an escape, which may spell one.
fn url_function(name: &[u8]) -> bool {
    if name.contains(&b'\\') {
        return true;
    }
    // oxc's `unvendored`: `-webkit-url` is `url`.
    let base = name
        .strip_prefix(b"-")
        .and_then(|rest| {
            rest.iter()
                .position(|&c| c == b'-')
                .map(|at| &rest[at + 1..])
        })
        .unwrap_or(name);
    [&b"url"[..], b"url-prefix", b"domain"]
        .iter()
        .any(|url| base.eq_ignore_ascii_case(url))
}

/// A CSS escape from `i` (after its `\`); the index after it. As oxc's
/// tokenizer reads one: up to six hex digits and one whitespace byte after
/// them (a line feed too, which then ends nothing), or any one byte.
fn css_escape(b: &[u8], i: usize, n: &mut Brackets) -> usize {
    match b.get(i) {
        None => b.len(),
        Some(c) if c.is_ascii_hexdigit() => {
            let digits = b[i..]
                .iter()
                .take(6)
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            let end = i + digits;
            end + usize::from(b.get(end).is_some_and(u8::is_ascii_whitespace))
        }
        Some(&c) => {
            n.literal(c);
            i + 1
        }
    }
}

/// A CSS string's body from `i` (after its opening `quote`); the index
/// after it. An unescaped line feed ends it unterminated, before the line
/// feed, as oxc's tokenizer ends a bad string (a carriage return alone
/// does not).
fn css_string(b: &[u8], mut i: usize, quote: u8, n: &mut Brackets) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' => i = css_escape(b, i + 1, n),
            b'\n' => return i,
            c if c == quote => return i + 1,
            c => {
                n.literal(c);
                i += 1;
            }
        }
    }
    b.len()
}

/// The arguments of a `url(` from `i` (after the `(`); the index after
/// its `)`. A quoted URL is a string, read as the rest of the text is; an
/// unquoted one is literal text to its `)`. oxc reparses an unquoted body
/// its tokenizer rejects (a quote, a comment) as a function's arguments,
/// so a string and a comment inside are read as such: either way no
/// closer inside counts but the `)`.
fn css_url(b: &[u8], mut i: usize, n: &mut Brackets, open: &mut Vec<u8>) -> usize {
    let start = b[i..]
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .map_or(b.len(), |at| i + at);
    if matches!(b.get(start), Some(b'"' | b'\'')) {
        return i;
    }
    while i < b.len() {
        match b[i] {
            b')' => {
                css_close(b')', n, open);
                return i + 1;
            }
            b'\\' => i = css_escape(b, i + 1, n),
            q @ (b'"' | b'\'') => i = css_string(b, i + 1, q, n),
            b'/' if b.get(i + 1) == Some(&b'*') => i = block_comment(b, i + 2, n),
            c => {
                n.literal(c);
                i += 1;
            }
        }
    }
    b.len()
}

/// A block comment's body from `i` (after its `/*`); the index after its
/// `*/`, or the end.
fn block_comment(b: &[u8], mut i: usize, n: &mut Brackets) -> usize {
    while i < b.len() {
        if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
            return i + 2;
        }
        n.literal(b[i]);
        i += 1;
    }
    b.len()
}

/// [`nesting_depth`] of a script, or of JSON.
fn js_nesting_depth(b: &[u8], visit: &mut impl FnMut(Site, Range<usize>)) -> usize {
    let mut n = Brackets::default();
    // The `{` open in each `${…}` the scan is inside, the innermost's in
    // `braces`: a `}` with none open ends the substitution.
    let mut substitutions = Vec::new();
    let mut braces = 0_usize;
    // The last significant byte of code, for a `/`: none yet, or `"` after
    // a string, a template or a regular expression (an operand).
    let mut prev = 0_u8;
    // Nothing but whitespace since the line began: `-->` is a comment.
    let mut line_start = true;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' | b'\r' => {
                line_start = true;
                i += 1;
                continue;
            }
            b' ' | b'\t' | 0x0b | 0x0c => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let rest = &b[i..];
        if rest.starts_with(b"//")
            || rest.starts_with(b"<!--")
            || (line_start && rest.starts_with(b"-->"))
        {
            let end = line_comment(b, i, &mut n);
            visit(Site::Comment, i..end);
            i = end;
            continue;
        }
        if rest.starts_with(b"/*") {
            let end = block_comment(b, i + 2, &mut n);
            visit(Site::Comment, i..end);
            i = end;
            continue;
        }
        line_start = false;
        i = match c {
            b'"' | b'\'' => {
                prev = b'"';
                let end = js_string(b, i + 1, c, &mut n);
                visit(Site::String(c), i..end);
                end
            }
            b'/' if regex_may_follow(prev) => {
                prev = b'"';
                let end = js_regex(b, i + 1, &mut n);
                visit(Site::Regex, i..end);
                end
            }
            b'(' | b'[' => {
                n.open();
                prev = c;
                i + 1
            }
            b'{' => {
                n.open();
                braces += 1;
                prev = c;
                i + 1
            }
            b')' | b']' => {
                n.close();
                prev = c;
                i + 1
            }
            b'}' if braces > 0 || substitutions.is_empty() => {
                n.close();
                braces = braces.saturating_sub(1);
                prev = c;
                i + 1
            }
            b'`' | b'}' => {
                if c == b'}' {
                    // The end of a `${…}`: back in its template's text.
                    n.close();
                    braces = substitutions.pop().unwrap_or(0);
                }
                let (next, substitution) = template_text(b, i + 1, &mut n);
                visit(Site::Template, i..next);
                if substitution {
                    substitutions.push(braces);
                    braces = 0;
                    prev = b'{';
                } else {
                    prev = b'"';
                }
                next
            }
            _ => {
                prev = c;
                i + 1
            }
        };
    }
    n.deepest
}

/// Whether a `/` after the significant byte `prev` (`0`: none) starts a
/// regular expression rather than divides.
fn regex_may_follow(prev: u8) -> bool {
    prev == 0 || b"(,=:[!&|?{};+-*%<>~^".contains(&prev)
}

/// The rest of the line from `i`: a comment; the index of its line break.
fn line_comment(b: &[u8], mut i: usize, n: &mut Brackets) -> usize {
    while i < b.len() && !matches!(b[i], b'\n' | b'\r') {
        n.literal(b[i]);
        i += 1;
    }
    i
}

/// A JavaScript string's body from `i` (after its opening `quote`); the
/// index after it. An unescaped line break ends it unterminated, a syntax
/// error, which ends oxc's parse.
fn js_string(b: &[u8], mut i: usize, quote: u8, n: &mut Brackets) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' => {
                match b.get(i + 1) {
                    Some(&e) => n.literal(e),
                    None => return b.len(),
                }
                // `\` CR LF continues the string over one line break.
                i += if b.get(i + 1..i + 3) == Some(&b"\r\n"[..]) {
                    3
                } else {
                    2
                };
            }
            b'\n' | b'\r' => return i,
            c if c == quote => return i + 1,
            c => {
                n.literal(c);
                i += 1;
            }
        }
    }
    b.len()
}

/// A regular expression's body and closing `/` from `i` (after its
/// opening `/`), the flags left to the caller as a name; the index after
/// it, or of the line break that ends it unterminated.
fn js_regex(b: &[u8], mut i: usize, n: &mut Brackets) -> usize {
    let mut class = false;
    while i < b.len() {
        match b[i] {
            b'\n' | b'\r' => return i,
            b'\\' => match b.get(i + 1) {
                Some(b'\n' | b'\r') | None => i += 1,
                Some(&e) => {
                    n.literal(e);
                    i += 2;
                }
            },
            b'/' if !class => return i + 1,
            c => {
                n.literal(c);
                class = match c {
                    b'[' => true,
                    b']' => false,
                    _ => class,
                };
                i += 1;
            }
        }
    }
    b.len()
}

/// A template literal's text from `i` (after a backtick or the `}` of a
/// `${…}`): the index after its closing backtick, and `false`; or after
/// the `${` that ends it (counted), and `true`.
fn template_text(b: &[u8], mut i: usize, n: &mut Brackets) -> (usize, bool) {
    while i < b.len() {
        match b[i] {
            b'\\' => {
                if let Some(&e) = b.get(i + 1) {
                    n.literal(e);
                }
                i += 2;
            }
            b'`' => return (i + 1, false),
            b'$' if b.get(i + 1) == Some(&b'{') => {
                n.open();
                return (i + 2, true);
            }
            c => {
                n.literal(c);
                i += 1;
            }
        }
    }
    (b.len(), false)
}

/// The depth of `text`'s JavaScript syntax tree, as [`NESTING_LIMIT`]
/// counts it: the most nodes open at once, the program included (`x = 1;`
/// is 4 deep: the program, the statement, the assignment, the literal); a
/// parenthesis is no node. Parsed as a script, then as a module, as the
/// literal walks try; `None` when `text` parses as neither or the island
/// thread cannot start. The parse and the walk run on the island thread.
pub fn tree_depth(text: &str) -> Option<usize> {
    oxc::tree_depth(text)
}

/// Where an island goes: the option that decides it and the extension of
/// the synthetic path the formatter sees. Whether an island is handed off
/// at all — pure (one text), or holding only text, `##` and `#…#`
/// ([`holes`]) — is the printer's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The option key: `islands.js`, `islands.css` or `islands.json`.
    pub key: &'static str,
    /// The synthetic path's extension: `js`, `mjs`, `json` or `css`.
    pub ext: &'static str,
    /// The language handed off.
    pub lang: Lang,
}

impl Target {
    /// This target's option value.
    pub fn tool(self, opts: &Options) -> IslandPreset {
        match self.key {
            "islands.css" => opts.islands_css,
            "islands.json" => opts.islands_json,
            _ => opts.islands_js,
        }
    }

    /// Whether `opts` formats this target (the option is not `"off"`).
    pub fn enabled(self, opts: &Options) -> bool {
        self.tool(opts) != IslandPreset::Off
    }
}

/// The dispatch table: a `<script>` with no `type` or a JavaScript MIME
/// type → `islands.js` (`.js`), `module` → `islands.js`
/// (`.mjs`), `application/json`, `application/ld+json`, `importmap` and
/// `speculationrules` → `islands.json` (`.json`), a `<style>` → `islands.css` (`.css`). Any other
/// `<script type>`, and every other site (`<cfquery>`, `<cfjava>`, event and
/// style attributes), is `None`: verbatim. Only `site` and `script_type`
/// decide; what the island holds is the caller's question (the printer
/// hands off a pure island and one whose CFML is only `#…#` and `##`, and
/// never a JSON island holding either).
pub fn dispatch(island: &Island) -> Option<Target> {
    let js = |ext| Target {
        key: "islands.js",
        ext,
        lang: Lang::Js,
    };
    // A type that names no language, or one only the server knows, is
    // data: never handed off, whatever the site would say.
    if island.lang == Lang::Unknown {
        return None;
    }
    match island.site {
        IslandSite::ScriptTag => match island.script_type.as_deref() {
            None
            | Some(
                "text/javascript"
                | "application/javascript"
                | "application/x-javascript"
                | "text/ecmascript"
                | "application/ecmascript",
            ) => Some(js("js")),
            Some("module") => Some(js("mjs")),
            Some("application/json" | "application/ld+json" | "importmap" | "speculationrules") => {
                Some(Target {
                    key: "islands.json",
                    ext: "json",
                    lang: Lang::Json,
                })
            }
            Some(_) => None,
        },
        IslandSite::StyleTag => Some(Target {
            key: "islands.css",
            ext: "css",
            lang: Lang::Css,
        }),
        _ => None,
    }
}

/// The text the formatter is handed: the island's text as written but for
/// its leading and trailing blank lines, with one trailing newline; empty
/// when the text is blank. Nothing inside moves —
/// no dedent, no trim — so a template literal or a comment reaches the
/// formatter byte for byte (its first and last non-blank lines hold every
/// opener and closer, so a dropped blank line is never inside one).
pub fn hand_off_text(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let blank = |l: &&str| l.trim().is_empty();
    let Some(first) = lines.iter().position(|l| !blank(l)) else {
        return String::new();
    };
    let last = lines.iter().rposition(|l| !blank(l)).unwrap_or(first);
    let mut out = lines[first..=last].join("\n");
    out.push('\n');
    out
}

/// The lines of `text`, a formatter's output for an island of `lang`, that
/// start inside literal text. In JavaScript: a template literal's quasi (a
/// tagged template's too, and one nested in another's `${…}`), a string
/// literal continued over a line, or a block comment the formatter prints
/// raw (one whose lines after the first do not all start with `*`). In
/// JSON: such a block comment. In CSS: any block comment, since the CSS
/// formatter re-aligns none. 0-based indices, ascending; the first line is
/// never one. The printer joins these lines with `literalline` and prints
/// them as written, so their columns are the text's own: an indented line
/// of a comment would be indented again at every run. No CSS or JSON
/// string spans a line (a CSS `\`-newline body is not handed off,
/// [`keeps_literal_text`]).
///
/// A JavaScript text's lines are found in the parsed text, so one that
/// parses neither as a script nor as a module has none to give: that is
/// [`Refused`] (`internal error: the formatted text does not
/// parse`), and so is an island thread that cannot start, never an empty
/// list, which would re-indent every line of a literal. A CSS or JSON
/// text's are found by the scan [`nesting_depth`] makes, which cannot
/// fail. [`Islands`] runs this on every formatter's output
/// ([`FormattedIsland`]).
pub fn literal_lines(text: &str, lang: Lang) -> Result<Vec<usize>, Refused> {
    oxc::literal_lines(text, lang)
}

/// The literal texts of a JavaScript island ([`literal_texts`]), in source
/// order, each as written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiteralTexts {
    /// Every template-literal quasi's raw text (a tagged template's too):
    /// what is between a backtick or `}` and the next `${` or backtick.
    pub quasis: Vec<String>,
    /// Every block comment, its delimiters included.
    pub comments: Vec<String>,
}

/// The literal texts of `text`, JavaScript parsed as the formatter parses
/// it (a script or a module, whichever parses), or `None` when it does not
/// parse. Formatting an island must leave its quasis as they were, and
/// its block comments too but for the re-alignment of their
/// lines: `tests/invariants.rs` compares this for the input and the output
/// of every island handed off.
pub fn literal_texts(text: &str) -> Option<LiteralTexts> {
    oxc::literal_texts(text)
}

/// Whether an island's source text (`text`, CF tags included) may hold a
/// string or literal that spans lines, so that moving any of its lines could
/// change a value: the verbatim path then prints it byte for byte,
/// and a CSS one is not handed to the formatter. Conservative, by language:
/// JavaScript — a backtick, or a line ending in `\` (a continued string);
/// CSS — a line ending in `\`; SQL — a string, quoted identifier or
/// dollar quote spanning a line, or one the scan cannot close, under any
/// dialect reading ([`sql_literal_spans_lines`]); Java — a `"""` text
/// block. JSON strings cannot span lines, and a body of no language
/// ([`Lang::Unknown`]) always keeps its text.
pub fn keeps_literal_text(text: &str, lang: Lang) -> bool {
    let lines = || text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let continued = || lines().any(|l| l.ends_with('\\'));
    match lang {
        Lang::Js => text.contains('`') || continued(),
        Lang::Css => continued(),
        Lang::Sql => sql_literal_spans_lines(text),
        Lang::Java => text.contains("\"\"\""),
        Lang::Json => false,
        Lang::Unknown => true,
    }
}

/// One way of reading SQL's lexical forms. SQL has no single lexer: the
/// dialects disagree on whether `\` escapes inside a string and whether
/// block comments nest, and a few have forms of their own.
#[derive(Debug, Clone, Copy)]
struct SqlReading {
    /// `\` escapes the next character inside `'…'` and `"…"` (MySQL).
    backslash: bool,
    /// `/*` inside a block comment opens another (PostgreSQL, SQL Server).
    nested: bool,
    /// MySQL's comments: `--` only before whitespace or the end, and
    /// `/*!…*/` is code, not a comment.
    mysql: bool,
    /// Oracle's `q'X…X'` (`[…]`, `(…)`, `{…}`, `<…>` pair up).
    q_quotes: bool,
}

/// The readings [`sql_literal_spans_lines`] runs: the four combinations of
/// `\` and nesting, then MySQL's comment forms and Oracle's `q'…'`, each
/// a run of its own. A body is kept when *any* run finds a literal spanning
/// a line, so a further reading can only keep more bodies, never fewer.
const SQL_READINGS: [SqlReading; 6] = [
    SqlReading {
        backslash: false,
        nested: true,
        mysql: false,
        q_quotes: false,
    },
    SqlReading {
        backslash: false,
        nested: false,
        mysql: false,
        q_quotes: false,
    },
    SqlReading {
        backslash: true,
        nested: true,
        mysql: false,
        q_quotes: false,
    },
    SqlReading {
        backslash: true,
        nested: false,
        mysql: false,
        q_quotes: false,
    },
    SqlReading {
        backslash: true,
        nested: false,
        mysql: true,
        q_quotes: false,
    },
    SqlReading {
        backslash: false,
        nested: false,
        mysql: false,
        q_quotes: true,
    },
];

/// Whether SQL `text` holds a literal — a `'…'` string (`''` inside), a
/// `"…"`, `` `…` `` or `[…]` quoted identifier, a `$tag$…$tag$` dollar
/// quote — that spans a line or that the text ends inside, under any of
/// the readings: the four combinations of `\` escaping and comment nesting,
/// MySQL's comment forms and Oracle's `q'…'` quotes, one pass each. `--` to
/// the end of the line and `/*…*/` are comments, and a comment never counts: `select 1 -- it's` is `false`. A `$` right
/// after an identifier character continues the identifier (`v$session`),
/// as PostgreSQL's lexer reads it; a `$tag$` elsewhere that the scan cannot
/// close keeps the body verbatim, a false positive for a dialect with no
/// dollar quotes.
pub fn sql_literal_spans_lines(text: &str) -> bool {
    SQL_READINGS.iter().any(|&reading| {
        let mut spans = false;
        sql_scan(text, reading, |literal, closed| {
            spans |= !closed || text[literal].contains('\n');
        });
        spans
    })
}

/// The literal texts of SQL `text` — strings, quoted identifiers and dollar
/// quotes, delimiters included — in source order, as the scan of
/// [`sql_literal_spans_lines`] finds them, one list per reading (they pair
/// the same quotes differently: MySQL's `'a\'` ⏎ `b'` is one literal, and
/// PostgreSQL's reading puts that newline between two). An unclosed one
/// runs to the end of the text, its trailing whitespace excluded: that is
/// the closing tag's line, not the literal's. Formatting must leave every
/// list's multiset as it was: `tests/invariants.rs` compares them for the
/// input and the output of every `<cfquery>`.
pub fn sql_literal_texts(text: &str) -> Vec<Vec<&str>> {
    SQL_READINGS
        .iter()
        .map(|&reading| {
            let mut out = Vec::new();
            sql_scan(text, reading, |literal, closed| {
                let literal = &text[literal];
                out.push(if closed { literal } else { literal.trim_end() });
            });
            out
        })
        .collect()
}

/// One linear pass over `text` under `reading`: `visit` gets each literal's
/// range and whether it closed (an unclosed one runs to the end).
fn sql_scan(text: &str, reading: SqlReading, mut visit: impl FnMut(std::ops::Range<usize>, bool)) {
    let b = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let after_ident = i > 0 && ident(b[i - 1]);
        match c {
            b'-' if b.get(i + 1) == Some(&b'-')
                && (!reading.mysql
                    || b.get(i + 2)
                        .is_none_or(|&n| n.is_ascii_whitespace() || n < 0x20)) =>
            {
                i = text[i..].find('\n').map_or(b.len(), |n| i + n);
            }
            b'/' if b.get(i + 1) == Some(&b'*')
                && !(reading.mysql && b.get(i + 2) == Some(&b'!')) =>
            {
                let mut depth = 1;
                i += 2;
                while i < b.len() && depth > 0 {
                    if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                    } else if reading.nested && b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'q' | b'Q'
                if reading.q_quotes
                    && b.get(i + 1) == Some(&b'\'')
                    // `q'…'`, or `nq'…'` (national): not the tail of a name.
                    && (!after_ident
                        || (matches!(b[i - 1], b'n' | b'N') && (i == 1 || !ident(b[i - 2]))))
                    && b.get(i + 2).is_some_and(|d| !d.is_ascii_whitespace()) =>
            {
                let open = text[i + 2..].chars().next().map_or(1, char::len_utf8);
                let d = &text[i + 2..i + 2 + open];
                let close = match d {
                    "[" => "]",
                    "(" => ")",
                    "{" => "}",
                    "<" => ">",
                    _ => d,
                };
                let from = i + 2 + open;
                let end = text[from..]
                    .find(&format!("{close}'"))
                    .map(|n| from + n + close.len() + 1);
                visit(i..end.unwrap_or(b.len()), end.is_some());
                i = end.unwrap_or(b.len());
            }
            b'\'' | b'"' | b'`' | b'[' => {
                let close = if c == b'[' { b']' } else { c };
                let escapes = reading.backslash && matches!(c, b'\'' | b'"');
                let mut j = i + 1;
                let end = loop {
                    match b.get(j) {
                        None => break None,
                        Some(b'\\') if escapes => j += 2,
                        Some(&x) if x == close && b.get(j + 1) == Some(&close) => j += 2,
                        Some(&x) if x == close => break Some(j + 1),
                        Some(_) => j += 1,
                    }
                };
                visit(i..end.unwrap_or(b.len()).min(b.len()), end.is_some());
                i = end.unwrap_or(b.len());
            }
            b'$' if !after_ident => {
                let tag_len = b[i + 1..]
                    .iter()
                    .position(|&x| !(x.is_ascii_alphanumeric() || x == b'_'))
                    .unwrap_or(b.len() - i - 1);
                let tag_ok = tag_len == 0 || !b[i + 1].is_ascii_digit();
                if tag_ok && b.get(i + 1 + tag_len) == Some(&b'$') {
                    let delim = &text[i..i + tag_len + 2];
                    let from = i + delim.len();
                    let end = text[from..].find(delim).map(|n| from + n + delim.len());
                    visit(i..end.unwrap_or(b.len()), end.is_some());
                    i = end.unwrap_or(b.len());
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
}

/// One island handed to a formatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandRequest<'a> {
    /// The island's language.
    pub lang: Lang,
    /// The text, as [`hand_off_text`] makes it.
    pub text: &'a str,
    /// The synthetic path (`page.cfm.js`): its extension picks the parser
    /// and the configuration's `overrides` match it. [`Oxc`] reads only the
    /// extension, and its cache entries are shared across paths; a
    /// formatter of [`Islands::with_formatter`] may read all of it, and its
    /// entries are per path.
    pub path: PathBuf,
    /// `tab_indent` and `indent_size` ([`Options::indent_style`]).
    pub indent: IndentStyle,
    /// The width budget: `max_columns` less the island's indentation, at
    /// least 40.
    pub width: usize,
    /// The project's configuration ([`Islands::project_config`]), which
    /// [`Oxc`] applies; the empty configuration when there is none or
    /// `islands.config` is `"off"`.
    pub config: Arc<IslandConfig>,
}

/// Why a formatter returned no text: it refused the island (it does not
/// parse, or it is past a limit). A warning, and the island prints
/// verbatim; the message is the formatter's first diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

/// An island as [`Islands`] formatted it: the formatter's text and the
/// lines of it that start inside literal text ([`literal_lines`]), which
/// the printer prints as written. One result, validated before the printer
/// sees any of it: a text whose lines cannot be found is no result but a
/// [`Refused`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedIsland {
    /// The formatter's output.
    pub text: String,
    /// [`literal_lines`] of `text`: 0-based, ascending.
    pub literal_lines: Vec<usize>,
}

/// A formatter for island text: [`Oxc`], or an oracle's through
/// [`Islands::with_formatter`].
pub trait IslandFormatter: Send + Sync {
    /// The formatted text, or why there is none.
    fn format(&self, req: &IslandRequest) -> Result<String, Refused>;

    /// The formatted text and its [`literal_lines`], what [`Islands`]
    /// caches: by default [`IslandFormatter::format`], then
    /// [`literal_lines`] of its text, a text whose lines cannot be found
    /// being a refusal. [`Oxc`] runs both on one island thread.
    fn format_island(&self, req: &IslandRequest) -> Result<FormattedIsland, Refused> {
        let text = self.format(req)?;
        literal_lines(&text, req.lang).map(|literal_lines| FormattedIsland {
            text,
            literal_lines,
        })
    }
}

/// How one request to [`Islands`] was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hit {
    /// From the cache (or from another thread's run of the same request).
    pub cached: bool,
    /// Time spent in the formatter and in the walk for its
    /// [`literal_lines`]: zero for a cache hit.
    pub time: Duration,
}

/// Counters over a run or one document (`--timing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IslandStats {
    /// Formatter runs, successful or not.
    pub formatted: usize,
    /// Requests answered from the cache.
    pub cached: usize,
    /// Requests that ended in a warning, cached or not.
    pub warnings: usize,
    /// Time spent in formatter runs.
    pub time: Duration,
}

impl IslandStats {
    /// Counts one request: a formatter run (with its time) or a cache hit,
    /// and a warning when the formatter refused the island.
    pub fn record(&mut self, hit: Hit, refused: bool) {
        if hit.cached {
            self.cached += 1;
        } else {
            self.formatted += 1;
            self.time += hit.time;
        }
        if refused {
            self.warnings += 1;
        }
    }

    /// Islands handed off: runs plus cache hits.
    pub fn islands(self) -> usize {
        self.formatted + self.cached
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    /// The synthetic path, for a formatter of [`Islands::with_formatter`]
    /// (which may read it); `None` for [`Oxc`], which reads only its
    /// extension.
    path: Option<PathBuf>,
    ext: String,
    indent: IndentStyle,
    width: usize,
    config: Arc<IslandConfig>,
    text: String,
}

/// Island formatting for one run: the cache and the counters. Shared by
/// every file of the run (it is `Sync`); never written to disk.
///
/// The cache is keyed by the synthetic path's extension, the indent style
/// and width, the width budget, the project configuration and the text as
/// handed off (as written: the same script at two depths is two entries).
/// For [`Oxc`] not by the path or its directory, so the same island in two
/// directories under the same (or no) configuration is one run; for a
/// formatter of [`Islands::with_formatter`], which may read
/// [`IslandRequest::path`], by the path too. A refusal is cached too. A
/// request already running on another thread is waited for rather than run
/// twice, so the counters do not depend on how many threads share the cache.
///
/// It also holds the run's project configuration for `"oxc"`
/// ([`Islands::project_config`]): which `.oxfmtrc` / `.prettierrc` applies
/// in each directory and what each holds, each read once, and the warnings
/// of the files that could not be read ([`Islands::config_warnings`]).
#[derive(Default)]
pub struct Islands {
    cache: Mutex<HashMap<CacheKey, Slot>>,
    stats: Mutex<IslandStats>,
    config: Mutex<config::ConfigCache>,
    config_warnings: Mutex<Vec<(PathBuf, String)>>,
    /// [`Islands::with_formatter`].
    formatter: Option<Arc<dyn IslandFormatter>>,
}

impl fmt::Debug for Islands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Islands")
            .field("stats", &self.stats())
            .field("formatter", &self.formatter.is_some())
            .finish_non_exhaustive()
    }
}

/// A cached outcome, filled by the first request that runs the formatter.
type Slot = Arc<OnceLock<Result<FormattedIsland, Refused>>>;

impl Islands {
    /// An empty cache.
    pub fn new() -> Self {
        Islands::default()
    }

    /// An empty cache whose every request goes to `formatter` instead of
    /// [`Oxc`]: the seam an oracle plugs another
    /// formatter into (`tests/islands_parity.rs` runs the prettier CLI on
    /// the requests the printer builds). Requests are cached and counted as
    /// for [`Islands::new`], with the request's path in the key.
    pub fn with_formatter(formatter: Arc<dyn IslandFormatter>) -> Self {
        Islands {
            formatter: Some(formatter),
            ..Islands::default()
        }
    }

    /// The project configuration for the island at `path` (its synthetic
    /// path, absolute): with `mode` `"auto"`, the first directory from
    /// `path`'s upward holding `.oxfmtrc.json`, `.oxfmtrc.jsonc`, a
    /// `package.json` with a `"prettier"` key or one of prettier's
    /// `.prettierrc` names supplies it, its `overrides` matched against
    /// `path`; otherwise, or when that file cannot be read, the empty
    /// configuration. Each directory and each file is looked up once per
    /// [`Islands`]; a file's problems are recorded once, for
    /// [`Islands::config_warnings`].
    pub fn project_config(&self, path: &Path, mode: IslandConfigMode) -> Arc<IslandConfig> {
        config::resolve(&self.config, &self.config_warnings, path, mode)
    }

    /// The warnings of every configuration file read so far, in the order
    /// they were found, each file's once: `(file, message)`, printed by the
    /// CLI as `warning: <file>: <message>`.
    pub fn config_warnings(&self) -> Vec<(PathBuf, String)> {
        self.config_warnings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Formats `req` with [`Oxc`] (or the formatter of
    /// [`Islands::with_formatter`]), from the cache when the same request
    /// was seen before. Only an enabled target reaches it: callers check
    /// [`Target::enabled`] first, and `"off"` prints verbatim.
    pub fn format(&self, req: &IslandRequest) -> (Result<FormattedIsland, Refused>, Hit) {
        match &self.formatter {
            Some(formatter) => self.format_with(formatter.as_ref(), req),
            None => self.format_with(&Oxc, req),
        }
    }

    /// Formats `req` with `formatter`, through the cache; the [`Hit`] says
    /// whether the formatter ran for this request and for how long. An
    /// [`Islands`] caches one formatter's results; the path is in the key
    /// when the [`Islands`] was built [`with_formatter`](Islands::with_formatter).
    ///
    /// The formatter's text comes with its [`literal_lines`], whichever
    /// formatter it was ([`IslandFormatter::format_island`]), so the walk is
    /// cached and timed with the format; when it fails the island is refused
    /// like any other.
    pub fn format_with(
        &self,
        formatter: &dyn IslandFormatter,
        req: &IslandRequest,
    ) -> (Result<FormattedIsland, Refused>, Hit) {
        let key = CacheKey {
            path: self.formatter.is_some().then(|| req.path.clone()),
            ext: req
                .path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default(),
            indent: req.indent,
            width: req.width,
            config: Arc::clone(&req.config),
            text: req.text.to_owned(),
        };
        let slot: Slot = Arc::clone(
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key)
                .or_default(),
        );
        // The lock is not held across the run: another thread asking for the
        // same request waits on the slot and counts a hit.
        let mut hit = Hit {
            cached: true,
            time: Duration::ZERO,
        };
        let result = slot
            .get_or_init(|| {
                let started = Instant::now();
                let result = formatter.format_island(req);
                hit = Hit {
                    cached: false,
                    time: started.elapsed(),
                };
                result
            })
            .clone();
        self.stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(hit, result.is_err());
        (result, hit)
    }

    /// Counts a warning for an island the formatter took and the printer
    /// refused after it: an island holding `#…#` whose output did not give
    /// them back as they went ([`holes::Interpolated::restore`]). The run
    /// was counted by [`Islands::format_with`] as it was answered.
    pub(crate) fn record_refusal(&self) {
        self.stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .warnings += 1;
    }

    /// The counters so far.
    pub fn stats(&self) -> IslandStats {
        *self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn req(text: &str) -> IslandRequest<'_> {
        IslandRequest {
            lang: Lang::Js,
            text,
            path: PathBuf::from("stdin.cfm.js"),
            indent: IndentStyle::Spaces(4),
            width: 72,
            config: Arc::default(),
        }
    }

    /// A formatter that takes a millisecond and upper-cases its input.
    struct Slow;

    impl IslandFormatter for Slow {
        fn format(&self, req: &IslandRequest) -> Result<String, Refused> {
            thread::sleep(Duration::from_millis(1));
            Ok(req.text.to_uppercase())
        }
    }

    /// A formatter that upper-cases its input and appends the extension and
    /// the width, and refuses `SYNTAX ERROR`.
    struct Marker;

    impl IslandFormatter for Marker {
        fn format(&self, req: &IslandRequest) -> Result<String, Refused> {
            if req.text.contains("SYNTAX ERROR") {
                return Err(Refused("SyntaxError: fake (1:1)".into()));
            }
            let ext = req.path.extension().unwrap().to_string_lossy();
            Ok(format!(
                "{}/* {ext} {} */\n",
                req.text.to_uppercase(),
                req.width
            ))
        }
    }

    /// A formatter whose output parses as no script: `SYNTAX ERROR` for
    /// JavaScript, its input for anything else.
    struct Unparsable;

    impl IslandFormatter for Unparsable {
        fn format(&self, req: &IslandRequest) -> Result<String, Refused> {
            Ok(match req.lang {
                Lang::Js => "SYNTAX ERROR\n".into(),
                _ => req.text.into(),
            })
        }
    }

    /// The formatted text of a result, or its failure.
    fn text(result: &Result<FormattedIsland, Refused>) -> Result<&str, &Refused> {
        result.as_ref().map(|f| f.text.as_str())
    }

    #[test]
    fn a_hit_says_whether_the_formatter_ran() {
        let islands = Islands::new();
        let (out, hit) = islands.format_with(&Slow, &req("x\n"));
        assert_eq!(text(&out), Ok("X\n"));
        assert!(
            !hit.cached && hit.time >= Duration::from_millis(1),
            "{hit:?}"
        );
        let (again, hit) = islands.format_with(&Slow, &req("x\n"));
        assert_eq!(again, out);
        assert_eq!(
            hit,
            Hit {
                cached: true,
                time: Duration::ZERO
            }
        );
        // Two threads asking for the same new request run the formatter once.
        let (a, b) = thread::scope(|s| {
            let a = s.spawn(|| islands.format_with(&Slow, &req("y\n")).1);
            let b = s.spawn(|| islands.format_with(&Slow, &req("y\n")).1);
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_ne!(a.cached, b.cached);
        let stats = islands.stats();
        assert_eq!((stats.formatted, stats.cached, stats.warnings), (2, 2, 0));
        let mut sum = IslandStats::default();
        sum.record(a, false);
        sum.record(b, true);
        assert_eq!((sum.formatted, sum.cached, sum.warnings), (1, 1, 1));
    }

    #[test]
    fn with_formatter_takes_every_request() {
        let islands = Islands::with_formatter(Arc::new(Slow));
        let (out, hit) = islands.format(&req("a = 1\n"));
        assert_eq!(text(&out), Ok("A = 1\n"));
        assert!(!hit.cached);
        let (again, hit) = islands.format(&req("a = 1\n"));
        assert_eq!((again, hit.cached), (out, true));
        // Such a formatter may read the path: another path is another
        // request.
        let elsewhere = IslandRequest {
            path: PathBuf::from("other/page.cfm.js"),
            ..req("a = 1\n")
        };
        assert!(!islands.format(&elsewhere).1.cached);
        assert!(islands.format(&elsewhere).1.cached);
        // The built-in formatter reads only the extension: one entry.
        let islands = Islands::new();
        assert!(!islands.format(&req("a = 1\n")).1.cached);
        assert!(islands.format(&elsewhere).1.cached);
    }

    #[test]
    fn the_cache_answers_a_repeated_request() {
        let islands = Islands::new();
        let format = |req: &IslandRequest| islands.format_with(&Marker, req).0;
        let first = format(&req("x\n"));
        assert_eq!(text(&first), Ok("X\n/* js 72 */\n"));
        assert_eq!(format(&req("x\n")), first);
        // A different width is a different request.
        let wider = IslandRequest {
            width: 80,
            ..req("x\n")
        };
        assert_eq!(text(&format(&wider)), Ok("X\n/* js 80 */\n"));
        // A refusal is cached and warns every time.
        format(&req("SYNTAX ERROR\n")).unwrap_err();
        format(&req("SYNTAX ERROR\n")).unwrap_err();
        let stats = islands.stats();
        assert_eq!((stats.formatted, stats.cached, stats.warnings), (3, 2, 2));
    }

    #[test]
    fn nesting_depth_counts_brackets_outside_literals() {
        let css = |text: &str| nesting_depth(text, Lang::Css);
        let json = |text: &str| nesting_depth(text, Lang::Json);
        let js = |text: &str| nesting_depth(text, Lang::Js);
        // Every language: never below zero, so closers first do not hide
        // the openers after; an opener in a literal counts (an over-count,
        // which only makes the limit stricter).
        for lang in [Lang::Css, Lang::Json, Lang::Js] {
            assert_eq!(nesting_depth("a(b[c{", lang), 3);
            assert_eq!(nesting_depth(")))(((", lang), 3);
            assert_eq!(nesting_depth("'((('", lang), 3);
            assert_eq!(nesting_depth("\"(((\"", lang), 3);
            assert_eq!(nesting_depth("", lang), 0);
            assert_eq!(nesting_depth("a(b)(c)[d]{e}", lang), 1);
        }
        let media = format!(
            "{}.a{{color:red}}{}",
            "@media screen{".repeat(3),
            "}".repeat(3)
        );
        assert_eq!(css(&media), 4);

        // CSS. A closer in a comment or a string cancels no opener (a
        // byte count would say 1 for the first).
        let masked = format!("{}color:red;{}", ".a{/* } */".repeat(3), "}".repeat(3));
        assert_eq!(css(&masked), 3);
        assert_eq!(css(".a{ content: \"}\" }"), 1);
        assert_eq!(css(".a{ content: '}' }"), 1);
        // An escaped quote does not end the string; an escaped closer
        // outside one is part of a name.
        assert_eq!(css(".a{ content: \"\\\"}\" }"), 1);
        assert_eq!(css(&".a\\}{".repeat(3)), 3);
        // A hex escape takes the whitespace byte after it, a line feed
        // too, which then ends no string.
        let hex = format!("{}{}", ".a{c:\"\\41\n}\";".repeat(3), "}".repeat(3));
        assert_eq!(css(&hex), 3);
        // A line feed ends an unterminated string (a bad string): the `}`
        // on the next line closes; with none, the string runs to the end.
        assert_eq!(css(".a{ content: \"}\n}.b{x:y}"), 1);
        assert_eq!(css(".a{ content: \"}"), 1);
        // An unquoted URL is literal to its `)` (its `(` counts), under a
        // vendor prefix too; a quoted one is a string. Three rules, each
        // with a `url(})`: a byte count would say 2.
        let urls = |f: &str| {
            format!(
                "{}color:red;{}",
                format!(".a{{b:{f}(}});").repeat(3),
                "}".repeat(3)
            )
        };
        assert_eq!(css(&urls("url")), 4);
        assert_eq!(css(&urls("URL")), 4);
        assert_eq!(css(&urls("-webkit-url")), 4);
        assert_eq!(css(&urls("url-prefix")), 4);
        assert_eq!(css(&urls("u\\72 l")), 4);
        assert_eq!(css(".a{b:url( \"})\" )}"), 2);
        // Any other function's arguments run to their `)`: a closer of
        // another kind inside them is a token, as inside a block (counting
        // it, `.a{b:foo(});`×n would be about 2 deep).
        assert_eq!(css(&urls("calc")), 4);
        assert_eq!(css(&urls("foo")), 4);
        assert_eq!(css(&".a{b:foo(]);".repeat(3)), 4);
        assert_eq!(css(&".a{b:foo(bar(}));".repeat(3)), 5);
        assert_eq!(css(&".a{b:[};".repeat(3)), 6);
        assert_eq!(css(".a{b:c)}"), 1);
        assert_eq!(css(".a{b:c]}.b{"), 1);
        // With nothing open a closer closes nothing, whatever its kind.
        assert_eq!(css("}]) .a{b:c}"), 1);
        // A comment's opener counts: over-counted, never under.
        assert!(css("/* { */ .a{b:c}") >= 1);
        // `//` is no CSS comment.
        assert_eq!(css(".a{ // }\n .b{c:d}"), 1);

        // JSON: the JavaScript rules, since oxc parses it with the
        // JavaScript parser.
        let masked = format!("{}0{}", "[\"]\",".repeat(3), "]".repeat(3));
        assert_eq!(json(&masked), 3);
        assert_eq!(json("{\"a\":\"}\"}"), 1);
        // An escaped backslash ends the string; an escaped quote does not.
        assert_eq!(json("[\"\\\\\"],[1]"), 1);
        assert_eq!(json("[\"\\\"],[1]\"]"), 2);
        // A comment inside a string is text; a comment is a comment.
        assert_eq!(json("[\"/* ] */\"]"), 1);
        assert_eq!(json("[\"/*\"],[1]"), 1);
        assert_eq!(json("[[// ]\n]]"), 2);
        assert_eq!(json("[[/* ] */]]"), 2);
        // Single quotes and a template literal, which the JSON formatter
        // prints as written.
        assert_eq!(json(&format!("{}0{}", "[']',".repeat(3), "]".repeat(3))), 3);
        assert_eq!(json(&format!("{}0{}", "[`]`,".repeat(3), "]".repeat(3))), 3);

        // JavaScript: the masked shapes, 3 deep each.
        for unit in ["[\"]\",", "[/]/,", "[`]`,", "[//]\n"] {
            let text = format!("x = {}0{};", unit.repeat(3), "]".repeat(3));
            assert_eq!(js(&text), 3, "{unit:?}");
        }
        let text = format!("x = {}1{};", "{a:\"}\",b:".repeat(3), "}".repeat(3));
        assert_eq!(js(&text), 3);
        let text = format!("x = {}1{};", "(/*)*/".repeat(3), ")".repeat(3));
        assert_eq!(js(&text), 3);
        // A substitution is code, and its `${` an opener: the byte count's
        // 2 and 3, too, for the first and the last.
        assert_eq!(js("`${[1]}`"), 2);
        assert_eq!(js("`${\"}\"}`"), 1);
        assert_eq!(js("`${`${[1]}`}`"), 3);
        // Its `}` is the one no `{` inside it opened; a template's text
        // runs over lines.
        assert_eq!(js("`${ {a:1} }]\n]`; [1]"), 2);
        // A division, then a regular expression after `[`.
        assert_eq!(js("x = a / 2; y = [/]/"), 1);
        // A `/` inside a class does not end one: the class's `[` counts,
        // and so do the two after (the byte count's 2).
        assert_eq!(js("x = /[/]]/; [[1]]"), 3);
        assert_eq!(js("/a/g; ["), 1);
        // A line break ends one unterminated, as it ends a string.
        assert_eq!(js("[/]\n]"), 1);
        assert_eq!(js("['a]\n]"), 1);
        // A script's HTML comments.
        assert_eq!(js("[<!-- ] -->\n]"), 1);
        assert_eq!(js("[\n  --> ]\n]"), 1);
        // The known misreading: after a name, a `/` divides, so the
        // regular expression's `]` counts as code. An under-count; a
        // script's tree is bounded after the parse.
        assert_eq!(js("return /]/"), 0);
        assert_eq!(js("[return /]/, [1]"), 1);
    }

    #[test]
    fn a_text_whose_lines_cannot_be_found_is_refused() {
        // The walk for the literal lines runs on every formatter's output,
        // in the same run: a text that does not parse is a refusal, cached
        // and warned about like the formatter's own.
        let islands = Islands::with_formatter(Arc::new(Unparsable));
        let refused = Err(Refused(
            "internal error: the formatted text does not parse".into(),
        ));
        let (first, hit) = islands.format(&req("var a = 1\n"));
        assert_eq!((first, hit.cached), (refused.clone(), false));
        let (again, hit) = islands.format(&req("var a = 1\n"));
        assert_eq!((again, hit.cached), (refused, true));
        let stats = islands.stats();
        assert_eq!((stats.formatted, stats.cached, stats.warnings), (1, 1, 2));
        // CSS has no literal lines to find.
        let css = IslandRequest {
            lang: Lang::Css,
            path: PathBuf::from("stdin.cfm.css"),
            ..req(".a { color: red }\n")
        };
        assert_eq!(
            islands.format(&css).0,
            Ok(FormattedIsland {
                text: ".a { color: red }\n".into(),
                literal_lines: Vec::new(),
            })
        );
    }

    #[test]
    fn a_refusal_by_the_limits_is_cached_and_warns() {
        let islands = Islands::new();
        let deep = format!("x = {}1{};\n", "(".repeat(501), ")".repeat(501));
        let refused = Err(Refused(
            "nested 501 levels deep, over the limit of 500".into(),
        ));
        let (first, hit) = islands.format(&req(&deep));
        assert_eq!((first, hit.cached), (refused.clone(), false));
        let (again, hit) = islands.format(&req(&deep));
        assert_eq!((again, hit.cached), (refused, true));
        let stats = islands.stats();
        assert_eq!((stats.formatted, stats.cached, stats.warnings), (1, 1, 2));
        // The same for the tree's refusal, after the parse.
        let deep = format!("{}x;\n", "if(a)".repeat(498));
        let refused = Err(Refused(
            "nested 501 levels deep, over the limit of 500".into(),
        ));
        let (first, hit) = islands.format(&req(&deep));
        assert_eq!((first, hit.cached), (refused.clone(), false));
        let (again, hit) = islands.format(&req(&deep));
        assert_eq!((again, hit.cached), (refused, true));
        let stats = islands.stats();
        assert_eq!((stats.formatted, stats.cached, stats.warnings), (2, 2, 4));
    }

    #[test]
    fn dispatch_table() {
        let island = |site, ty: Option<&str>| Island {
            lang: Lang::Js,
            site,
            script_type: ty.map(String::from),
        };
        let ext = |site, ty| dispatch(&island(site, ty)).map(|t| (t.key, t.ext));
        assert_eq!(ext(IslandSite::ScriptTag, None), Some(("islands.js", "js")));
        for ty in [
            "text/javascript",
            "application/javascript",
            "application/x-javascript",
            "text/ecmascript",
            "application/ecmascript",
        ] {
            assert_eq!(
                ext(IslandSite::ScriptTag, Some(ty)),
                Some(("islands.js", "js"))
            );
        }
        assert_eq!(
            ext(IslandSite::ScriptTag, Some("module")),
            Some(("islands.js", "mjs"))
        );
        for ty in [
            "application/json",
            "application/ld+json",
            "importmap",
            "speculationrules",
        ] {
            assert_eq!(
                ext(IslandSite::ScriptTag, Some(ty)),
                Some(("islands.json", "json"))
            );
        }
        for ty in [
            "text/template",
            "text/html",
            "text/x-handlebars-template",
            "text/babel",
        ] {
            assert_eq!(ext(IslandSite::ScriptTag, Some(ty)), None);
        }
        assert_eq!(
            ext(IslandSite::StyleTag, None),
            Some(("islands.css", "css"))
        );
        for site in [
            IslandSite::CfQuery,
            IslandSite::CfJava,
            IslandSite::EventAttribute,
            IslandSite::StyleAttribute,
        ] {
            assert_eq!(ext(site, None), None);
        }
    }

    #[test]
    fn hand_off_text_drops_only_blank_edges() {
        // Only the blank lines at the edges go; everything else is as
        // written, indentation and trailing whitespace included.
        assert_eq!(
            hand_off_text("\n  \n    a\n      b  \n\n    c\n  \n"),
            "    a\n      b  \n\n    c\n"
        );
        // A literal's blank line survives, and so does a tab.
        assert_eq!(
            hand_off_text("\tx = `a\n\n\t\tb`;"),
            "\tx = `a\n\n\t\tb`;\n"
        );
        assert_eq!(hand_off_text("  \n\n"), "");
        assert_eq!(hand_off_text(""), "");
    }

    #[test]
    fn literal_lines_are_the_lines_inside_literal_text() {
        let js = |text: &str| literal_lines(text, Lang::Js).unwrap();
        // A template literal spanning lines: every line after its opener,
        // the closer's included (indenting it would add to the quasi).
        assert_eq!(js("const s = `a\n    b\n`;\nx();\n"), [1, 2]);
        // A blank line inside one is literal too.
        assert_eq!(js("const s = `a\n\nb`;\n"), [1, 2]);
        // A tagged template's quasi, and one nested in another's `${…}`.
        assert_eq!(js("html`<a>\n  </a>`;\n"), [1]);
        assert_eq!(js("const s = `a ${f(`b\n  c`)} d\n  e`;\nx();\n"), [1, 2]);
        // Only the quasis: a line inside `${…}` is code.
        assert_eq!(js("const s = `a${\n  x\n}b`;\n"), Vec::<usize>::new());
        // A string continued over a line.
        assert_eq!(js("const s = \"a\\\n  b\";\n"), [1]);
        // A JSDoc-style comment is re-aligned by the formatter: not literal;
        // a plain block comment is printed raw: literal; a line comment
        // spans nothing.
        assert_eq!(js("/**\n * doc\n */\nx();\n"), Vec::<usize>::new());
        assert_eq!(js("/* a\n   b */\nx();\n"), [1]);
        assert_eq!(js("/* a\n   b\n*/\nx();\n"), [1, 2]);
        assert_eq!(js("// a\n// b\nx();\n"), Vec::<usize>::new());
        // A comment inside `${…}` counts; a `/*` inside a quasi is text.
        assert_eq!(js("`${/* a\n b */ x}`;\n"), [1]);
        assert_eq!(js("`/* a\n b */`;\n"), [1]);
        // A module parses as one.
        assert_eq!(js("import a from \"a\";\nawait `x\ny`;\n"), [2]);
        // CSS and JSON have no string that spans lines.
        assert_eq!(
            literal_lines(".a {\n  content: \"x\\\ny\";\n}\n", Lang::Css),
            Ok(Vec::new())
        );
        assert_eq!(
            literal_lines("{\n  \"a\": 1\n}\n", Lang::Json),
            Ok(Vec::new())
        );
        // A CSS comment is printed as written, whatever its lines start
        // with: every line after its first, wherever the comment sits.
        let css = |text: &str| literal_lines(text, Lang::Css).unwrap();
        assert_eq!(css("/*\n * a\n */\n.a {\n}\n"), [1, 2]);
        assert_eq!(css(".a {\n  /* b\n     c */\n  color: red;\n}\n"), [2]);
        assert_eq!(css("/* one line */\n.a {\n}\n"), Vec::<usize>::new());
        // A `/*` in a string or a quoted URL starts none.
        assert_eq!(
            css(".a {\n  content: \"/*\";\n  background: url(\"/*\");\n}\n/* b */\n"),
            Vec::<usize>::new()
        );
        // In an unquoted URL the scan cannot tell: what it reads over a
        // line from there is kept as written.
        assert_eq!(css(".a {\n  b: url(/*c);\n}\n/* d\n   e */\n"), [2, 3, 4]);
        // The JSON formatter re-aligns a comment whose lines start with
        // `*`, as the JavaScript one does, and prints any other raw.
        let json = |text: &str| literal_lines(text, Lang::Json).unwrap();
        assert_eq!(
            json("{\n  /*\n   * a\n   */\n  \"a\": 1\n}\n"),
            Vec::<usize>::new()
        );
        assert_eq!(json("{\n  /* a\n     b */\n  \"a\": \"/*\"\n}\n"), [2]);
        assert_eq!(json("{\n  // a\n  \"a\": 1\n}\n"), Vec::<usize>::new());
        // A script that does not parse has no lines to give: a failure,
        // not an empty list. Not for CSS, which has none to find.
        assert_eq!(
            literal_lines("SYNTAX ERROR\n", Lang::Js),
            Err(Refused(
                "internal error: the formatted text does not parse".into()
            ))
        );
        assert_eq!(literal_lines("SYNTAX ERROR\n", Lang::Css), Ok(Vec::new()));
    }

    #[test]
    fn keeps_literal_text_by_language() {
        let keeps = |text: &str, lang| keeps_literal_text(text, lang);
        // JavaScript: a backtick anywhere, or a `\`-continued line.
        assert!(keeps("const s = `a`;\n", Lang::Js));
        assert!(keeps("const s = \"a\\\nb\";\n", Lang::Js));
        assert!(keeps("const s = \"a\\\r\nb\";\r\n", Lang::Js));
        assert!(!keeps("const s = 'a';\n  go(s); // \\ x\n", Lang::Js));
        // CSS: a `\`-continued line only.
        assert!(keeps(".a{content:\"x\\\ny\"}\n", Lang::Css));
        assert!(!keeps(".a{content:\"`\"}\n", Lang::Css));
        // SQL: a literal spanning a line ([`sql_literal_spans_lines`]).
        assert!(keeps("select 'a\n  b' as x\n", Lang::Sql));
        assert!(!keeps("select 'a', 'it''s'\nfrom t\n", Lang::Sql));
        // Java: a text block.
        assert!(keeps("String s = \"\"\"\n  a\n  \"\"\";\n", Lang::Java));
        assert!(!keeps("String s = \"a\";\n", Lang::Java));
        // JSON never.
        assert!(!keeps("{\"a\": \"`\\\n\"}\n", Lang::Json));
    }

    #[test]
    fn sql_literals_spanning_lines_under_every_reading() {
        let spans = |text: &str| keeps_literal_text(text, Lang::Sql);
        // A comment's apostrophe cancels nothing (a per-line quote count
        // would let it), and dollar quotes are strings.
        assert!(spans("/* ' */ select 'a\n  b' /* ' */ as x"));
        assert!(spans("select $$a\n  b$$ as x"));
        assert!(spans("select $fn$\n$fn$"));
        assert!(spans("select $fn$ a $$ b\n c $fn$"));
        // A comment never counts: `-- it's` alone keeps nothing now.
        assert!(!spans("select 1 -- it's\nfrom t\n"));
        assert!(!spans("select 1 /* it's\n */ from t\n"));
        // One-line literals.
        assert!(!spans("select 'it''s' as x\nfrom t\n"));
        assert!(!spans("select [it's] from t\nwhere 1 = 1\n"));
        assert!(!spans("select `it's`, \"it's\" from t\nwhere 1 = 1\n"));
        assert!(!spans("select $$a$$\nfrom t\n"));
        assert!(!spans(
            "select a from t where b = $1 and c = $2\n  and d = 1\n"
        ));
        assert!(!spans("select 'a' || 'b'\r\nfrom t\r\n"));
        // `\` escapes under MySQL's reading: this string spans a line there.
        assert!(spans("select 'a\\'\nb' as x"));
        // Comments nest under PostgreSQL's reading, not MySQL's: MySQL ends
        // the comment at the first `*/` and reads a string over the line.
        assert!(spans("select /* a /* b */ 'c\n  d' */ 1"));
        // MySQL's `--` needs whitespace after it, and `/*!` is code.
        assert!(spans("select 1--'a\n  b'"));
        assert!(spans("select /*! 'a\n  b' */ 1"));
        // Oracle's `q'…'` (an apostrophe inside pairs with nothing).
        assert!(spans("select q'{a'b\n'c}' from dual"));
        // Read by the other readings, one with an apostrophe inside leaves
        // one unpaired: kept, a false positive.
        assert!(spans("select q'[it's]' from dual\nwhere 1 = 1\n"));
        // What the scan cannot close keeps the body.
        assert!(spans("select 'a from t\n"));
        assert!(spans("select [a from t\n"));
        assert!(spans("select $x$ a from t\n"));
        // A `$` inside a name continues it (Oracle's `v$session`).
        assert!(!spans(
            "select * from v$session where a = 'x'\n  and b = v$b$c\n"
        ));
        // The literal texts under the PostgreSQL reading.
        // The literal texts, per reading.
        let texts = sql_literal_texts("select 'a''b', \"c\" /* 'd' */ -- 'e'\n, $$f$$, [g], `h`");
        assert_eq!(texts.len(), SQL_READINGS.len());
        assert!(texts
            .iter()
            .all(|t| t == &["'a''b'", "\"c\"", "$$f$$", "[g]", "`h`"]));
        let texts = sql_literal_texts("select 'a\\'\n  b' as x\n    ");
        assert_eq!(texts[0], ["'a\\'", "' as x"]);
        assert_eq!(texts[2], ["'a\\'\n  b'"]);
    }
}
