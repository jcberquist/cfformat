//! String-, comment- and `#…#`-aware delimiter scanning, shared by the two
//! front ends.
//!
//! They find where a `<cfscript>` body, a tag's expression or a `#…#` ends
//! without parsing the script inside: the tag scanner (`tags/islands.rs`)
//! uses them for that, and the script parser's lookaheads use them to count
//! delimiters by the same rules ([`balanced_code`]).
//!
//! Every function takes the source and a byte offset and returns an offset;
//! the caller passes the source already cut where scanning must stop (the
//! tag scanner's `end`), and an unterminated construct runs to `src.len()`.
//! [`Bounded`] is the exception: for where a `<cfscript>` body or a CF
//! tag's expression ends, a string or `#…#` that does not close before
//! the region's boundary is a bare character instead.
//!
//! The scanners recurse (a `#…#` inside a string inside a `#…#`), so each
//! that does takes the caller's `depth` and counts against
//! [`MAX_DEPTH`]. At the bound a scanner stops recursing
//! and scans **flat** to its own closer: brackets by
//! counting their own pair, a string or a `#…#` to its first closing
//! character, a nested `#`, quote or other bracket read as a plain
//! character. Real code never nests that deep; a pathological input gets a
//! region end that may differ from the recursive one, never an overflow.

use std::collections::HashSet;

use crate::MAX_DEPTH;

/// The next character's width at `at` (1 past the end).
fn char_len(src: &str, at: usize) -> usize {
    src[at..].chars().next().map_or(1, char::len_utf8)
}

/// Where a `#…#` whose opener is just before `from` ends — the first `#`
/// outside a string, a comment or brackets. At [`MAX_DEPTH`], the first `#`.
pub(crate) fn hash_end(src: &str, from: usize, depth: u32) -> usize {
    if depth >= MAX_DEPTH {
        return src[from..].find('#').map_or(src.len(), |i| from + i);
    }
    let mut at = from;
    while at < src.len() {
        let rest = &src[at..];
        match rest.as_bytes()[0] {
            b'#' => return at,
            b'(' | b'[' | b'{' => at = bracket_end(src, at, depth + 1),
            b'<' if rest.starts_with("<!---") => at = tag_comment_end(src, at),
            b'\'' | b'"' => at = string_end(src, at, depth + 1),
            b'/' => at = comment_end(src, at),
            _ => at += char_len(src, at),
        }
    }
    src.len()
}

/// [`hash_end`] with the closing `#` consumed: a `#…#` nested in brackets
/// or a string.
pub(crate) fn nested_hash_end(src: &str, from: usize, depth: u32) -> usize {
    let end = hash_end(src, from, depth);
    if end < src.len() {
        end + 1
    } else {
        end
    }
}

/// The end of the `( )`, `[ ]` or `{ }` opening at `from`, past its closer,
/// inside a `#…#`: inside the brackets a `#` opens another `#…#` instead of
/// ending the enclosing one. At [`MAX_DEPTH`], the closer that balances the
/// opener's own pair.
fn bracket_end(src: &str, from: usize, depth: u32) -> usize {
    let open = src.as_bytes()[from];
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        _ => b'}',
    };
    if depth >= MAX_DEPTH {
        let mut count = 0usize;
        for (i, &b) in src.as_bytes()[from..].iter().enumerate() {
            if b == open {
                count += 1;
            } else if b == close {
                count -= 1;
                if count == 0 {
                    return from + i + 1;
                }
            }
        }
        return src.len();
    }
    let mut at = from + 1;
    while at < src.len() {
        let rest = &src[at..];
        let b = rest.as_bytes()[0];
        if b == close {
            return at + 1;
        }
        match b {
            b'(' | b'[' | b'{' => at = bracket_end(src, at, depth + 1),
            b'#' => at = nested_hash_end(src, at + 1, depth + 1),
            b'<' if rest.starts_with("<!---") => at = tag_comment_end(src, at),
            b'\'' | b'"' => at = string_end(src, at, depth + 1),
            b'/' => at = comment_end(src, at),
            _ => at += char_len(src, at),
        }
    }
    src.len()
}

/// The end of the CFML string opening at `from`, past its closing quote. A
/// doubled quote (`''`, `""`) is an escape, `##` an escaped hash, and a `#…#`
/// inside may hold strings with the same quote. At [`MAX_DEPTH`] a `#` is a
/// plain character.
pub(crate) fn string_end(src: &str, from: usize, depth: u32) -> usize {
    let quote = src.as_bytes()[from];
    let mut at = from + 1;
    while at < src.len() {
        let b = src.as_bytes()[at];
        if b == quote {
            if src.as_bytes().get(at + 1) == Some(&quote) {
                at += 2;
                continue;
            }
            return at + 1;
        }
        if b == b'#' && depth < MAX_DEPTH {
            if src.as_bytes().get(at + 1) == Some(&b'#') {
                at += 2;
                continue;
            }
            at = nested_hash_end(src, at + 1, depth + 1);
            continue;
        }
        at += char_len(src, at);
    }
    src.len()
}

/// At a `/`: the end of a `cfformat-ignore` region, a block comment or a line
/// comment (its newline excluded); `from + 1` when the `/` opens none of them.
pub(crate) fn comment_end(src: &str, from: usize) -> usize {
    let rest = &src[from..];
    if let Some(n) = script_marker(rest, Marker::Start) {
        let mut at = from + n;
        while at < src.len() {
            if let Some(n) = script_marker(&src[at..], Marker::End) {
                return at + n;
            }
            at += char_len(src, at);
        }
        return src.len();
    }
    if rest.starts_with("/*") {
        return match src[from + 2..].find("*/") {
            Some(n) => from + 2 + n + 2,
            None => src.len(),
        };
    }
    if rest.starts_with("//") {
        return match rest.find('\n') {
            Some(n) => from + n,
            None => src.len(),
        };
    }
    from + 1
}

/// The end of the `<!--- … --->` opening at `from`, nested comments
/// included.
pub(crate) fn tag_comment_end(src: &str, from: usize) -> usize {
    let mut at = from + 5;
    let mut depth = 1usize;
    while at < src.len() {
        let rest = &src[at..];
        if rest.starts_with("--->") {
            at += 4;
            depth -= 1;
            if depth == 0 {
                return at;
            }
            continue;
        }
        if rest.starts_with("<!---") {
            depth += 1;
            at += 5;
            continue;
        }
        at += char_len(src, at);
    }
    src.len()
}

/// The end of a balanced `open`…`close` run of script starting at `at`
/// (which must hold `open`), or `None` when it never closes. Delimiters
/// count only outside strings ([`string_end`]: `''` / `""` doubling, `#…#`
/// with its own strings), `/* */`, `//` to the end of the line and
/// `<!--- --->`, so the script parser's lookaheads do not read
/// `f=(x /* ) */)=>x` or `f=(x=")")=>x` as closing early.
/// `depth` is the caller's, for [`string_end`].
pub(crate) fn balanced_code(
    src: &str,
    at: usize,
    open: u8,
    close: u8,
    depth: u32,
) -> Option<usize> {
    if src.as_bytes().get(at) != Some(&open) {
        return None;
    }
    let mut count = 0usize;
    let mut i = at;
    while i < src.len() {
        let rest = &src[i..];
        match rest.as_bytes()[0] {
            b if b == open => {
                count += 1;
                i += 1;
            }
            b if b == close => {
                count -= 1;
                i += 1;
                if count == 0 {
                    return Some(i);
                }
            }
            // An unterminated string or comment runs to the end: nothing
            // closes.
            b'\'' | b'"' => i = string_end(src, i, depth + 1),
            b'/' if rest.starts_with("/*") || rest.starts_with("//") => {
                i = comment_end(src, i);
            }
            b'<' if rest.starts_with("<!---") => i = tag_comment_end(src, i),
            _ => i += char_len(src, i),
        }
    }
    None
}

/// The text that ends the region a [`Bounded`] scan is finding, even
/// inside a `#…#` left open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    /// `</cfscript>`: a `<cfscript>` body's end. Never script, so it ends a
    /// `#…#` inside brackets too, and a string holding it must close on
    /// the line it is on (see [`Bounded::string_end`]).
    ScriptClose,
    /// `>` or `/>`: a CF tag expression's end (`<cfset …>`). A comparison
    /// in a call's arguments is script (`#f(a > b)#`), so inside brackets
    /// it is not a boundary, and `=>` never is.
    TagClose,
}

impl Boundary {
    /// Whether `rest` starts with the boundary, read inside brackets or not.
    fn at(self, rest: &str, in_brackets: bool) -> bool {
        match self {
            Boundary::ScriptClose => closes_tag(rest, "cfscript").is_some(),
            Boundary::TagClose => !in_brackets && (rest.starts_with('>') || rest.starts_with("/>")),
        }
    }
}

/// What a [`Bounded`] scan remembers did not close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Construct {
    Hash,
    Bracket,
    /// A string, and whether it was read `strict`.
    String(bool),
}

/// The string and `#…#` scanners for finding where a `<cfscript>` body or a
/// CF tag's expression ends, bounded by its [`Boundary`]: a `#` whose
/// `#…#` meets the boundary before its closing `#` is a bare character, and
/// so is a quote whose string never closes. [`string_end`] and
/// [`hash_end`] read an unclosed `#` on to the next `#` or the end of the
/// file, so `x = "price #"; … </cfscript>` or `<cfset x = "price #">` took
/// the closing tag and every tag after it into one expression, an unclosed
/// block or no warning at all.
///
/// The rule, for each level:
///
/// - **Inside a `#…#`** (and its brackets, for [`Boundary::ScriptClose`])
///   the boundary is hard: meeting it, or a string or bracket in it that
///   does not close, makes the `#…#` unclosed. A string inside that closes
///   hides the boundary — `#f(">")#`, `#"</cfscript>"#` — unless it holds
///   the boundary and runs past the end of its line, which in a
///   one-expression `#…#` is a string that closed on some later quote, not
///   one that was written.
/// - **In a string**, a `#…#` that does not close is a bare `#` and the
///   string reads on: `"price #"` is a string. A closed string hides the
///   boundary (`<cfset x = "<b>">`, `x = "</cfscript>";`), except that
///   under [`Boundary::ScriptClose`] a string holding `</cfscript>` must
///   close on that line too.
/// - **At the region's own level** (the callers' loops), a string or
///   `#…#` that does not close is a bare character and the scan goes on,
///   so the boundary after it ends the region and the script parser reports
///   the broken expression inside it.
///
/// A closed form scans exactly as with the unbounded scanners.
///
/// **Cost.** The unbounded scanners never retry, so each character is read
/// once per level; here a failure is retried from the character after its
/// opener, which could read the same text again and again. Three things
/// keep it linear:
///
/// - a construct that fails is remembered by start (and kind), so a retry
///   that meets it again takes it as bare without scanning it;
/// - at [`MAX_DEPTH`] a `#…#` or bracket fails outright — remembered too —
///   where the unbounded scanners scan flat: a flat scan runs to the end
///   of the region, and each retry started one (`x="#f("#f("…` × 40,000
///   took four seconds). Code nested a hundred deep in a `#…#` is not real
///   code; it ends the region like any other unclosed `#`;
/// - a budget of [`BUDGET_PER_BYTE`] reads per byte of the region: what
///   the memo does not cover (a `/*` never closed inside each of many
///   `#…#`s, read to the end once per `#`) spends it, and once it is spent
///   every string and `#…#` still to scan is unclosed, so the region's own
///   loop reads the rest as bare characters up to the first boundary. A
///   real body or tag expression reads each byte a few times at most.
pub(crate) struct Bounded<'a> {
    src: &'a str,
    boundary: Boundary,
    failed: HashSet<(usize, Construct)>,
    /// Reads left (see [`Bounded`]).
    budget: usize,
}

/// A [`Bounded`] scan's budget, in reads per byte of the region, and the
/// fixed part of it.
const BUDGET_PER_BYTE: usize = 32;
const BUDGET_BASE: usize = 4096;

impl<'a> Bounded<'a> {
    /// A scan of the region of `src` that starts at `from`.
    pub(crate) fn new(src: &'a str, from: usize, boundary: Boundary) -> Self {
        let len = src.len().saturating_sub(from);
        Bounded {
            src,
            boundary,
            failed: HashSet::new(),
            budget: len
                .saturating_mul(BUDGET_PER_BYTE)
                .saturating_add(BUDGET_BASE),
        }
    }

    /// Takes `reads` from the budget; `false` once it is spent.
    fn spend(&mut self, reads: usize) -> bool {
        match self.budget.checked_sub(reads) {
            Some(left) => {
                self.budget = left;
                true
            }
            None => {
                self.budget = 0;
                false
            }
        }
    }

    /// The end of the string opening at `from`, past its closing quote, or
    /// `None` when it does not close (see [`Bounded`]). `strict`: the
    /// string must close on the line where it holds the boundary — always
    /// inside a `#…#`, and at the region's level under
    /// [`Boundary::ScriptClose`].
    pub(crate) fn string_end(&mut self, from: usize, depth: u32, strict: bool) -> Option<usize> {
        if self.failed.contains(&(from, Construct::String(strict))) {
            return None;
        }
        let end = self.string_end_uncached(from, depth, strict);
        if end.is_none() {
            self.failed.insert((from, Construct::String(strict)));
        }
        end
    }

    fn string_end_uncached(&mut self, from: usize, depth: u32, strict: bool) -> Option<usize> {
        let src = self.src;
        let b = src.as_bytes();
        let quote = b[from];
        let mut holds_boundary = false;
        let mut at = from + 1;
        while at < src.len() {
            if !self.spend(1) {
                return None;
            }
            let c = b[at];
            if c == quote {
                if b.get(at + 1) == Some(&quote) {
                    at += 2;
                    continue;
                }
                return Some(at + 1);
            }
            if strict {
                if c == b'\n' && holds_boundary {
                    return None;
                }
                holds_boundary |= self.boundary.at(&src[at..], false);
            }
            if c == b'#' {
                if b.get(at + 1) == Some(&b'#') {
                    at += 2;
                    continue;
                }
                // An unclosed `#…#` is a bare `#`: the string reads on.
                at = self
                    .hash_end(at + 1, depth + 1)
                    .map_or(at + 1, |end| end + 1);
                continue;
            }
            at += char_len(src, at);
        }
        None
    }

    /// The offset of the `#` closing the `#…#` whose opener is just before
    /// `from`, or `None` when the boundary, a string or bracket that does
    /// not close, or the end comes first, or at [`MAX_DEPTH`].
    pub(crate) fn hash_end(&mut self, from: usize, depth: u32) -> Option<usize> {
        if self.failed.contains(&(from, Construct::Hash)) {
            return None;
        }
        let end = if depth < MAX_DEPTH {
            self.code_end(from, depth, None)
        } else {
            None
        };
        if end.is_none() {
            self.failed.insert((from, Construct::Hash));
        }
        end
    }

    /// The end of the bracket opening at `from`, past its closer, or `None`
    /// (at [`MAX_DEPTH`] too).
    fn bracket_end(&mut self, from: usize, depth: u32) -> Option<usize> {
        if self.failed.contains(&(from, Construct::Bracket)) {
            return None;
        }
        let close = match self.src.as_bytes()[from] {
            b'(' => b')',
            b'[' => b']',
            _ => b'}',
        };
        let end = if depth < MAX_DEPTH {
            self.code_end(from + 1, depth, Some(close))
                .map(|end| end + 1)
        } else {
            None
        };
        if end.is_none() {
            self.failed.insert((from, Construct::Bracket));
        }
        end
    }

    /// The script inside a `#…#` (`close` `None`, ending at a `#`) or a
    /// bracket (ending at `close`): the offset of the closer, or `None`.
    fn code_end(&mut self, from: usize, depth: u32, close: Option<u8>) -> Option<usize> {
        let src = self.src;
        let in_brackets = close.is_some();
        let closer = close.unwrap_or(b'#');
        let mut at = from;
        while at < src.len() {
            if !self.spend(1) {
                return None;
            }
            let rest = &src[at..];
            if self.boundary.at(rest, in_brackets) {
                return None;
            }
            // A comment is read to its end by the unbounded scanner: what
            // it read is spent.
            let comment = match rest.as_bytes()[0] {
                c if c == closer => return Some(at),
                b'=' if rest.starts_with("=>") => {
                    at += 2;
                    continue;
                }
                b'(' | b'[' | b'{' => {
                    at = self.bracket_end(at, depth + 1)?;
                    continue;
                }
                b'#' => {
                    at = self.hash_end(at + 1, depth + 1)? + 1;
                    continue;
                }
                b'\'' | b'"' => {
                    at = self.string_end(at, depth + 1, true)?;
                    continue;
                }
                b'<' if rest.starts_with("<!---") => tag_comment_end(src, at),
                b'/' => comment_end(src, at),
                _ => {
                    at += char_len(src, at);
                    continue;
                }
            };
            if !self.spend(comment - at) {
                return None;
            }
            at = comment;
        }
        None
    }
}

/// The length of the closing tag `</name\s*>` at the start of `rest` —
/// `</`, `name` ASCII case-insensitively, any whitespace (newlines
/// included), `>` — or `None`. Every CF closing-tag check uses it, so
/// `</cfscript >` closes a script body as `</cfscript>` does.
pub(crate) fn closes_tag(rest: &str, name: &str) -> Option<usize> {
    let b = rest.as_bytes();
    if !rest.starts_with("</")
        || !b
            .get(2..2 + name.len())?
            .eq_ignore_ascii_case(name.as_bytes())
    {
        return None;
    }
    let mut at = 2 + name.len();
    while b.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    (b.get(at) == Some(&b'>')).then_some(at + 1)
}

/// The end of the Java string, char or text-block literal opening at `from`
/// (a `"` or `'`), past its closing delimiter: `"…"` and `'…'` with `\`
/// escapes, `"""…"""` to the next `"""` that no `\` escapes. A `"…"` or
/// `'…'` left open ends at its line's newline (excluded), which Java does
/// not allow inside one, so the rest of the body is still read; an open
/// text block runs to the end (a `\` as its last character included).
pub(crate) fn java_literal_end(src: &str, from: usize) -> usize {
    let b = src.as_bytes();
    if src[from..].starts_with("\"\"\"") {
        let mut at = from + 3;
        while at < b.len() {
            match b[at] {
                b'\\' => at += 2,
                b'"' if src[at..].starts_with("\"\"\"") => return at + 3,
                _ => at += 1,
            }
        }
        return src.len();
    }
    let quote = b[from];
    let mut at = from + 1;
    while at < b.len() {
        match b[at] {
            b'\\' => at += 1 + b.get(at + 1).map_or(0, |_| char_len(src, at + 1)),
            b'\n' => return at,
            c if c == quote => return at + 1,
            _ => at += char_len(src, at),
        }
    }
    src.len()
}

/// One end of an ignore region, the comment that opens or closes it.
#[derive(Clone, Copy)]
pub(crate) enum Marker {
    Start,
    End,
}

impl Marker {
    /// The words that mark this end: cfformat's own, and `@formatter:off` /
    /// `@formatter:on`, the formatter markers of JetBrains IDEs and Eclipse
    /// (which BoxLang's formatter reads too). Either end word closes either
    /// start word.
    fn words(self) -> [&'static str; 2] {
        match self {
            Marker::Start => ["cfformat-ignore-start", "@formatter:off"],
            Marker::End => ["cfformat-ignore-end", "@formatter:on"],
        }
    }

    /// `text` past a leading marker word, in any ASCII case, or `None`.
    pub(crate) fn strip(self, text: &str) -> Option<&str> {
        self.words().into_iter().find_map(|w| {
            let head = text.get(..w.len())?;
            head.eq_ignore_ascii_case(w).then(|| &text[w.len()..])
        })
    }
}

/// The ignore marker comment at the start of `text` in script, its length:
/// `//`, spaces or tabs, the marker word, spaces or tabs, and the newline
/// (a line of its own after the `//`); or `/*`, whitespace, the word,
/// whitespace and `*/`. Every reader of script — the script parser, and
/// [`comment_end`] for the scanners that find where script ends — reads
/// the markers through this.
pub(crate) fn script_marker(text: &str, which: Marker) -> Option<usize> {
    if let Some(rest) = text.strip_prefix("//") {
        let body = rest.trim_start_matches([' ', '\t']);
        let body = which.strip(body)?;
        let body = body.trim_start_matches([' ', '\t']);
        let body = body.strip_prefix('\n')?;
        return Some(text.len() - body.len());
    }
    let rest = text.strip_prefix("/*")?;
    let body = which.strip(rest.trim_start())?;
    let body = body.trim_start().strip_prefix("*/")?;
    Some(text.len() - body.len())
}

/// The ignore marker comment at the start of `text` in a template, its
/// length: `<!---`, whitespace, the marker word, whitespace and `--->`.
/// The tag scanner and the script parser's mode check read it through this.
pub(crate) fn tag_marker(text: &str, which: Marker) -> Option<usize> {
    let rest = text.strip_prefix("<!---")?;
    let body = which.strip(rest.trim_start())?;
    let body = body.trim_start().strip_prefix("--->")?;
    Some(text.len() - body.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_end_skips_an_ignore_region_of_either_marker_in_any_case() {
        for src in [
            "// cfformat-ignore-start\n)\n// cfformat-ignore-end\n",
            "// @formatter:off\n)\n// @formatter:on\n",
            "/* @formatter:off */)/* @formatter:on */",
            "// @formatter:off\n)\n// cfformat-ignore-end\n",
            // In any case.
            "// CFFORMAT-IGNORE-START\n)\n// Cfformat-Ignore-End\n",
            "/* @Formatter:Off */)/* @FORMATTER:ON */",
        ] {
            assert_eq!(comment_end(src, 0), src.len(), "{src:?}");
        }
        // A longer word is a line comment, its newline excluded.
        let src = "// @formatter:offset\n)";
        assert_eq!(comment_end(src, 0), src.find('\n').unwrap());
    }

    #[test]
    fn markers_take_spaces_or_tabs_on_a_line_and_whitespace_in_a_block() {
        let marker = "// \tcfformat-ignore-start\t \n";
        assert_eq!(script_marker(marker, Marker::Start), Some(marker.len()));
        assert_eq!(
            script_marker("//\x0ccfformat-ignore-start\n", Marker::Start),
            None
        );
        assert_eq!(
            script_marker("// cfformat-ignore-start x\n", Marker::Start),
            None
        );
        assert_eq!(
            script_marker("/*\n@formatter:on\n*/x", Marker::End),
            Some(19)
        );
        assert_eq!(
            tag_marker("<!---\n@formatter:on --->x", Marker::End),
            Some(24)
        );
        assert_eq!(tag_marker("<!--- @formatter:onx --->", Marker::End), None);
        // A form feed after `//` is a line comment to the boundary scanners.
        let src = "//\x0ccfformat-ignore-start\n)";
        assert_eq!(comment_end(src, 0), src.find('\n').unwrap());
    }

    #[test]
    fn balanced_code_skips_strings_and_comments() {
        for (src, end) in [
            ("(a)", Some(3)),
            ("(a(b))c", Some(6)),
            ("(x /* ) */)=>", Some(11)),
            ("(x // )\n)", Some(9)),
            ("(x <!--- ) --->)", Some(16)),
            ("(x=\")\")", Some(7)),
            ("(x=')')", Some(7)),
            ("(x='it''s)')", Some(12)),
            ("(x=\"#a(\")\")#\")", Some(14)),
            ("(x=\"é)\")", Some(9)),
            ("(a", None),
            ("(x=\")", None),
            ("(x /* )", None),
        ] {
            assert_eq!(balanced_code(src, 0, b'(', b')', 0), end, "{src}");
        }
        assert_eq!(balanced_code("[a, \"]\"]", 0, b'[', b']', 0), Some(8));
    }

    #[test]
    fn strings_and_hashes() {
        assert_eq!(string_end("'a''b'x", 0, 0), 6);
        assert_eq!(string_end("\"a##b\"x", 0, 0), 6);
        assert_eq!(string_end("\"#f(\"x\")#\"y", 0, 0), 10);
        assert_eq!(hash_end("f(\"##\")#", 0, 0), 7);
        assert_eq!(tag_comment_end("<!---a<!---b--->c--->d", 0), 21);
    }

    /// At `MAX_DEPTH` a scanner reads flat: a quote or a `#` inside is a
    /// plain character, brackets count their own pair.
    #[test]
    fn scanning_at_the_bound_is_flat() {
        assert_eq!(bracket_end("(')')x", 0, 0), 5);
        assert_eq!(bracket_end("(')')x", 0, MAX_DEPTH), 3);
        assert_eq!(bracket_end("([)]", 0, MAX_DEPTH), 3);
        assert_eq!(string_end("'#'#'", 0, 0), 5);
        assert_eq!(string_end("'#'#'", 0, MAX_DEPTH), 3);
        assert_eq!(hash_end("('#')#", 0, MAX_DEPTH), 2);
        // 100,000 brackets: the recursion stops at the bound, the flat scan
        // finds the same end.
        let n = 100_000;
        let src = format!("{}x{}#", "(".repeat(n), ")".repeat(n));
        assert_eq!(hash_end(&src, 0, 0), src.len() - 1);
    }

    /// Where a region's own loop (as `script_body_end` / `angle_end` run
    /// it, comments and fences aside) ends: the first boundary outside a
    /// string or `#…#` that closes, an unclosed one read as bare.
    fn region_end(src: &str, boundary: Boundary) -> usize {
        let mut scan = Bounded::new(src, 0, boundary);
        let mut at = 0;
        while at < src.len() {
            let rest = &src[at..];
            if boundary.at(rest, false) {
                return at;
            }
            at = match rest.as_bytes()[0] {
                b'\'' | b'"' => scan
                    .string_end(at, 0, boundary == Boundary::ScriptClose)
                    .unwrap_or(at + 1),
                b'#' => scan.hash_end(at + 1, 0).map_or(at + 1, |end| end + 1),
                b'/' => comment_end(src, at),
                _ => at + char_len(src, at),
            };
        }
        src.len()
    }

    #[test]
    fn bounded_scans_end_at_the_boundary() {
        fn script(src: &str) -> &str {
            &src[..region_end(src, Boundary::ScriptClose)]
        }
        fn tag(src: &str) -> &str {
            &src[..region_end(src, Boundary::TagClose)]
        }
        // An unclosed `#`, in a string or bare, is a character: the
        // boundary after it ends the region.
        for (src, body) in [
            (
                "x = \"price #\"; y=1; </cfscript><p>ok</p>\n",
                "x = \"price #\"; y=1; ",
            ),
            ("x = #; y=1; </cfscript><p>ok</p>\n", "x = #; y=1; "),
            ("x = 1 # 2; y=1; </cfscript><p>ok</p>\n", "x = 1 # 2; y=1; "),
            // A later `#` or quote does not close it across the boundary.
            ("x = #; </cfscript><p>#y#</p>\n", "x = #; "),
            (
                "x = \"price #\"; y=1; </cfscript>\n<p class=\"ok\">#y#</p>\n",
                "x = \"price #\"; y=1; ",
            ),
            // Nor quotes that pair up to a `#` after it: without the line
            // rule `"; y=1; </cfscript>` ⏎ `<p class="` would be a closed
            // string, and `#"…"ok" id="#` a closed `#…#`.
            (
                "x = \"price #\"; y=1; </cfscript>\n<p class=\"ok\" id=\"#y#\">\n",
                "x = \"price #\"; y=1; ",
            ),
            // A string holding `</cfscript>` closes on its line or not at all.
            ("x = \"a </cfscript>\n<p class=\"b\">\n", "x = \"a "),
        ] {
            assert_eq!(script(src), body, "{src:?}");
        }
        for (src, expr) in [
            (" x = \"price #\"><cfset y=1>\n", " x = \"price #\""),
            (
                " x = \"price #\"><cfset y=\"1\"><p class=\"a\">#z#</p>\n",
                " x = \"price #\"",
            ),
            (" x = \"#f(\"><cfset y=1>\n", " x = \"#f(\""),
        ] {
            assert_eq!(tag(src), expr, "{src:?}");
        }
        // Closed forms scan as the unbounded scanners do.
        for src in [
            "x = \"price ##\"; y = \"hi #name#\"; z = \"a#f(\">\")#b\"; ",
            "x = \"a#\"</cfscript>\"#b\"; y = #\"</cfscript>\"#; ",
            "x = \"</cfscript>\"; y = #a#; // # </cfscript>\n/* # </cfscript> */ ",
            "x = \"#f(\n  a,\n  b\n)#\"; y = #(a)#; ",
        ] {
            let full = format!("{src}</cfscript><p>#x#</p>\n");
            assert_eq!(script(&full), src, "{src:?}");
        }
        for src in [
            " x = \"#f(\">\")#\"",
            " x = \"a#f(\">\")#b\"",
            " x = \"<b>bold</b>\"",
            " x = \"<b>\n  bold\n</b>\"",
            " x = \"#f(a > b)#\"",
            " x = #arr.map((a) => a)#",
            " x = \"#y#\" & \"price ##\"",
        ] {
            let full = format!("{src}><cfset y=1>\n");
            assert_eq!(tag(&full), src, "{src:?}");
        }
    }

    /// Retrying from the character after a bare opener stays linear: the
    /// memo, the depth bound and, for what they miss, the budget.
    #[test]
    fn bounded_scans_stay_linear() {
        for unit in ["#\"#(", "\"#f(\"#", "'#\"", "#(", "#[{(", "\"# ("] {
            let src = format!("x={}</cfscript>\n", unit.repeat(40_000));
            let mut scan = Bounded::new(&src, 0, Boundary::ScriptClose);
            let _ = region_end(&src, Boundary::ScriptClose);
            // The memo alone keeps these well inside the budget.
            let mut at = 2;
            while at < src.len() && !src[at..].starts_with("</cfscript>") {
                at = match src.as_bytes()[at] {
                    b'\'' | b'"' => scan.string_end(at, 0, true).unwrap_or(at + 1),
                    b'#' => scan.hash_end(at + 1, 0).map_or(at + 1, |end| end + 1),
                    _ => at + 1,
                };
            }
            assert!(scan.budget > 0, "{unit:?}: budget spent");
            assert_eq!(
                region_end(&src, Boundary::ScriptClose),
                src.len() - 12,
                "{unit:?}"
            );
        }
        // A line comment in each `#…#` is read to the end of the line once
        // per `#`, which the memo does not cover: the budget ends it, and
        // every `#…#` after is bare.
        let src = format!("x=\"{}</cfscript>\n", "#\"//".repeat(40_000));
        let mut scan = Bounded::new(&src, 0, Boundary::ScriptClose);
        assert_eq!(scan.string_end(2, 0, true), None);
        assert_eq!(scan.budget, 0);
        assert_eq!(scan.hash_end(4, 0), None);
    }

    #[test]
    fn closing_tags() {
        for (rest, len) in [
            ("</cfscript>x", Some(11)),
            ("</CFScript>", Some(11)),
            ("</cfscript >", Some(12)),
            ("</cfscript\n\t>", Some(13)),
            ("</cfscriptx>", None),
            ("</cfscript", None),
            ("</cfscr", None),
            ("< /cfscript>", None),
            ("</cfscripté>", None),
        ] {
            assert_eq!(closes_tag(rest, "cfscript"), len, "{rest:?}");
        }
    }

    #[test]
    fn java_literals() {
        for (src, end) in [
            ("\"}\";", 3),
            ("\"\\\"}\";", 5),
            ("'{';", 3),
            ("'\\'';", 4),
            ("\"é}\"x", 5),
            ("\"\"\"a\n\"}\"\n\"\"\"x", 12),
            ("\"open\n}", 5),
            // A text block honours `\`: an escaped quote is no terminator,
            // an escaped backslash is.
            ("\"\"\"\na\\\"\"\" }\nx\n\"\"\"; y", 17),
            ("\"\"\"a\\\\\"\"\"x", 9),
            ("\"\"\"a\\\"\"\" and more", 17),
            ("\"\"\"a\\", 5),
        ] {
            assert_eq!(java_literal_end(src, 0), end, "{src:?}");
        }
    }
}
