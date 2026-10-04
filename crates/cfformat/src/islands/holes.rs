//! Islands holding `#…#`: a `<script>` or `<style>` inside `<cfoutput>`
//! whose CFML is only `#expr#` (a *hole*) and `##`.
//!
//! The formatter cannot read CFML, so it is handed a stand-in text
//! ([`substitute`]): each `##` as the `#` the page receives, and each hole
//! as a placeholder — an identifier in both languages, as wide as the hole
//! prints, built on a stem the island's text does not hold. A hole inside a
//! `'…'` or `"…"` string gets the other quote character inside its
//! placeholder, enough of them that the formatter, which keeps the quote
//! that needs fewer escapes, keeps the string's quote whatever the
//! project's preference: the formatter cannot know which quotes the hole
//! emits.
//!
//! The formatter's output is checked and put back ([`Interpolated::restore`]):
//! every placeholder must come back once, in order, at the same kind of
//! place (code, a string with the same quote, a template literal's text, a
//! comment, a regular expression, a `url(…)` body), and a hole in code —
//! one identifier to the formatter, anything at all on the page — with the
//! same bytes around it: the same non-blank byte before it, the same after
//! it (or a statement's `;`, or a property's trailing `,` before `}`, each
//! added in front of what followed), as many parentheses around it, the
//! same byte where it touched a word in the source, no
//! operator newly against it (`- #x#` printed `-#x#`), and nothing but a
//! closing bracket or a separator brought up from a later line to its own
//! (`#x#` ⏎ `(f)();` printed `#x#(f)();`). Then each `#`
//! becomes `##` again and each placeholder its hole's printed text. An
//! island that fails a check is refused, and the printer prints it as
//! written with a warning naming the check and the hole's line.
//!
//! Where a hole sits is read by the lexical scan [`super::nesting_depth`]
//! makes (its regions: strings, comments, template literals' text, regular
//! expressions, `url(…)` bodies), on the text handed off and again on the
//! output. Its regular-expression rule is a heuristic: a `/` after a name
//! divides and after an operator starts a regular expression. A misread
//! reads both texts the same way, and a hole in a regular expression is
//! checked as a hole in code is, so a regular expression read as code
//! costs nothing; code read as a regular expression (`a++ / (#x#) / 2`)
//! puts the hole in a "regular expression" the formatter may reformat, and
//! only the checks on its neighbours, which it gets too, stand behind it.

use std::ops::Range;

use cfdoc::width::str_width;
use cfparse::Lang;

use super::{hand_off_text, scan, Refused};

/// Where a byte of an island's text sits, lexically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Site {
    /// Outside every literal.
    Code,
    /// A string with this quote.
    String(u8),
    /// A template literal's text (not the code of a `${…}`).
    Template,
    /// A comment, block or line (and a script's `<!--` / `-->` lines).
    Comment,
    /// A JavaScript regular expression.
    Regex,
    /// The unquoted body of a CSS `url(…)`.
    Url,
}

impl Site {
    /// How a warning names it.
    fn name(self) -> &'static str {
        match self {
            Site::Code => "code",
            Site::String(b'\'') => "a '…' string",
            Site::String(_) => "a \"…\" string",
            Site::Template => "a template literal",
            Site::Comment => "a comment",
            Site::Regex => "a regular expression",
            Site::Url => "a url(…)",
        }
    }

    /// Whether a hole here has its neighbours checked: code, and the
    /// literal text the formatter prints as written but a scan may have
    /// misread (a regular expression, a `url(…)` body), where the checks
    /// cost nothing.
    fn checked(self) -> bool {
        matches!(self, Site::Code | Site::Regex | Site::Url)
    }
}

/// One piece of an island's children, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    /// Text, as written.
    Text(&'a str),
    /// `##`.
    Hash,
    /// `#expr#`.
    Hole(Hole),
}

/// A `#expr#` in an island.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hole {
    /// Its printed text, delimiters included, on one line.
    pub text: String,
    /// Its source line, 1-based, for a warning.
    pub line: usize,
}

/// An island ready to hand off: the text and what is needed to put the
/// holes back in the formatter's output.
#[derive(Debug)]
pub(crate) struct Interpolated {
    /// The text the formatter is handed ([`hand_off_text`] of the
    /// substitution).
    pub text: String,
    lang: Lang,
    stem: &'static str,
    holes: Vec<Placed>,
}

/// A hole and its stand-in.
#[derive(Debug)]
struct Placed {
    hole: Hole,
    placeholder: String,
    /// Where the placeholder is in [`Interpolated::text`].
    at: usize,
    site: Site,
}

/// The stems a placeholder is built on, tried in order: the first that the
/// island's text does not hold is used. Pairs of letters English and code
/// seldom put together.
const STEMS: [&str; 6] = ["zq", "qx", "jq", "vq", "xj", "qj"];

/// The text an island of `lang` holding `pieces` is handed, or `None` when
/// the island is not one to hand off: a text holding a `#` (a lone hash the
/// parse recovered, which the restoration could not tell from a `##`), no
/// text outside the holes but whitespace (`<style>#expr#</style>`), or
/// every stem in the text.
///
/// Placeholder `i` is the stem, `i` in decimal and `_` (so none is a
/// prefix of another), then for the first hole of a string the other
/// quote character as many times as the string's own quote less the other
/// one's, plus one (a quote needing fewer escapes is kept: the string's
/// own needs strictly fewer), then `_` up to the hole's printed width.
pub(crate) fn substitute(pieces: Vec<Piece<'_>>, lang: Lang) -> Option<Interpolated> {
    let mut plain = String::new();
    for p in &pieces {
        match p {
            Piece::Text(t) if t.contains('#') => return None,
            Piece::Text(t) => plain.push_str(t),
            Piece::Hash => plain.push('#'),
            Piece::Hole(_) => plain.push(' '),
        }
    }
    if plain.trim().is_empty() {
        return None;
    }
    for stem in STEMS {
        if plain.contains(stem) {
            continue;
        }
        if let Some((text, placed)) = place(&pieces, stem, lang) {
            let holes = pieces
                .into_iter()
                .filter_map(|p| match p {
                    Piece::Hole(h) => Some(h),
                    _ => None,
                })
                .zip(placed)
                .map(|(hole, (placeholder, at, site))| Placed {
                    hole,
                    placeholder,
                    at,
                    site,
                })
                .collect();
            return Some(Interpolated {
                text,
                lang,
                stem,
                holes,
            });
        }
    }
    None
}

/// The hand-off text for `stem` and each placeholder with its position and
/// site, or `None` when a placeholder is not in the text exactly once.
#[allow(clippy::type_complexity)]
fn place(
    pieces: &[Piece<'_>],
    stem: &str,
    lang: Lang,
) -> Option<(String, Vec<(String, usize, Site)>)> {
    let holes: Vec<&Hole> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Hole(h) => Some(h),
            _ => None,
        })
        .collect();
    let make = |i: usize, pins: &str| {
        let mut p = format!("{stem}{i}_{pins}");
        let width = str_width(&holes[i].text);
        while p.len() < width {
            p.push('_');
        }
        p
    };
    let mut placeholders: Vec<String> = (0..holes.len()).map(|i| make(i, "")).collect();
    let text = hand_off_text(&join(pieces, &placeholders));
    let at = once_each(&text, stem, &placeholders)?;
    let regions = regions_of(&text, lang);
    let sites: Vec<(Site, Option<Range<usize>>)> =
        at.iter().map(|&p| site_at(&regions, p)).collect();
    // Pin the quote of each string holding a hole, in its first hole.
    let mut pinned = false;
    let mut last = None;
    for (i, (site, region)) in sites.iter().enumerate() {
        let (Site::String(q), Some(r)) = (site, region) else {
            continue;
        };
        if last == Some(r.start) {
            continue;
        }
        last = Some(r.start);
        let other = if *q == b'\'' { b'"' } else { b'\'' };
        let body = &text.as_bytes()[r.start + 1..r.end];
        let closed = r.end > r.start + 1 && text.as_bytes()[r.end - 1] == *q;
        let own = body.iter().filter(|&&c| c == *q).count() - usize::from(closed);
        let others = body.iter().filter(|&&c| c == other).count();
        let pins = (own + 1).saturating_sub(others);
        if pins > 0 {
            placeholders[i] = make(i, &(other as char).to_string().repeat(pins));
            pinned = true;
        }
    }
    let sites: Vec<Site> = sites.into_iter().map(|(s, _)| s).collect();
    if !pinned {
        return Some((text, zip3(placeholders, at, sites)));
    }
    // A quote of the other kind inside a string ends nothing, so the sites
    // are the same; checked rather than assumed.
    let text = hand_off_text(&join(pieces, &placeholders));
    let at = once_each(&text, stem, &placeholders)?;
    let regions = regions_of(&text, lang);
    let again: Vec<Site> = at.iter().map(|&p| site_at(&regions, p).0).collect();
    (again == sites).then(|| (text, zip3(placeholders, at, sites)))
}

fn zip3(a: Vec<String>, b: Vec<usize>, c: Vec<Site>) -> Vec<(String, usize, Site)> {
    a.into_iter()
        .zip(b)
        .zip(c)
        .map(|((a, b), c)| (a, b, c))
        .collect()
}

/// The island's text with each `##` as `#` and each hole as its
/// placeholder.
fn join(pieces: &[Piece<'_>], placeholders: &[String]) -> String {
    let mut out = String::new();
    let mut holes = placeholders.iter();
    for p in pieces {
        match p {
            Piece::Text(t) => out.push_str(t),
            Piece::Hash => out.push('#'),
            Piece::Hole(_) => out.push_str(holes.next().map_or("", String::as_str)),
        }
    }
    out
}

/// Where each placeholder is in `text`: every place it starts, found in
/// one pass over the stem's occurrences.
fn occurrences(text: &str, stem: &str, placeholders: &[String]) -> Vec<Vec<usize>> {
    let mut found = vec![Vec::new(); placeholders.len()];
    for (at, _) in text.match_indices(stem) {
        let rest = &text.as_bytes()[at + stem.len()..];
        let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        let Some(i) = std::str::from_utf8(&rest[..digits])
            .ok()
            .and_then(|d| d.parse::<usize>().ok())
        else {
            continue;
        };
        if placeholders
            .get(i)
            .is_some_and(|p| text[at..].starts_with(p.as_str()))
        {
            found[i].push(at);
        }
    }
    found
}

/// Each placeholder's one position in `text`, or `None` when one is not
/// there exactly once.
fn once_each(text: &str, stem: &str, placeholders: &[String]) -> Option<Vec<usize>> {
    occurrences(text, stem, placeholders)
        .into_iter()
        .map(|f| (f.len() == 1).then(|| f[0]))
        .collect()
}

/// The literal regions of `text` read as `lang`, in order.
fn regions_of(text: &str, lang: Lang) -> Vec<(Site, Range<usize>)> {
    let mut out = Vec::new();
    scan(text.as_bytes(), lang, &mut |site, range| {
        out.push((site, range))
    });
    out
}

/// The site of the byte at `at`, and the region holding it.
fn site_at(regions: &[(Site, Range<usize>)], at: usize) -> (Site, Option<Range<usize>>) {
    let i = regions.partition_point(|(_, r)| r.end <= at);
    match regions.get(i) {
        Some((site, r)) if r.start <= at => (*site, Some(r.clone())),
        _ => (Site::Code, None),
    }
}

/// A byte that continues a word: a hole written against one is part of a
/// name, a number, a selector or a value (`function #name#Callback`,
/// `#w#px`, `.#cls#`, `#a##b#`).
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '-' | '%' | '.' | '#') || !c.is_ascii()
}

/// A character that can run into what a `#…#` emits when nothing separates
/// them: an operator's.
fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '%' | '<' | '>' | '=' | '&' | '|' | '^' | '!' | '~' | '?'
    )
}

/// The parentheses open in code at each of `at` (ascending), `regions`
/// being `text`'s literal regions: a `(` or `)` inside a string, a comment,
/// a template literal's text or a regular expression counts for nothing,
/// and an unquoted `url(…)` body holds its function's `)`.
fn depths(
    text: &str,
    regions: &[(Site, Range<usize>)],
    at: impl Iterator<Item = usize>,
) -> Vec<isize> {
    let b = text.as_bytes();
    let (mut depth, mut i, mut r) = (0_isize, 0, 0);
    let mut out = Vec::new();
    for to in at {
        while i < to {
            match regions.get(r) {
                Some((site, range)) if range.start <= i => {
                    if *site == Site::Url && b.get(range.end - 1) == Some(&b')') {
                        depth -= 1;
                    }
                    i = range.end.max(i + 1);
                    r += 1;
                }
                _ => {
                    match b[i] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
        out.push(depth);
    }
    out
}

/// The nearest character before `at` that is not ASCII whitespace, and
/// where it is.
fn before(text: &str, at: usize) -> Option<(usize, char)> {
    let rest = text[..at].trim_end_matches(|c: char| c.is_ascii_whitespace());
    let c = rest.chars().next_back()?;
    Some((rest.len() - c.len_utf8(), c))
}

/// The nearest character from `at` on that is not ASCII whitespace, and
/// where it is.
fn after(text: &str, at: usize) -> Option<(usize, char)> {
    let rest = &text[at..];
    let trimmed = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let c = trimmed.chars().next()?;
    Some((at + rest.len() - trimmed.len(), c))
}

/// Whether `text` from `at` on is an `else`, whitespace skipped.
fn before_else(text: &str, at: usize) -> bool {
    text[at..]
        .trim_start_matches(|c: char| c.is_ascii_whitespace())
        .strip_prefix("else")
        .is_some_and(|rest| !rest.starts_with(is_word))
}

/// A character as a warning shows it.
fn shown(c: Option<char>) -> String {
    match c {
        Some(c) => format!("`{c}`"),
        None => "nothing".into(),
    }
}

impl Interpolated {
    /// The formatter's output `out` with the holes put back, or why the
    /// island is refused: a placeholder not there once or out of order, a
    /// hole at another kind of place, or a hole in code whose neighbours
    /// changed (the module docs).
    pub(crate) fn restore(&self, out: &str) -> Result<String, Refused> {
        let placeholders: Vec<String> = self.holes.iter().map(|h| h.placeholder.clone()).collect();
        let mut at = Vec::with_capacity(self.holes.len());
        for (h, found) in self
            .holes
            .iter()
            .zip(occurrences(out, self.stem, &placeholders))
        {
            let line = h.hole.line;
            match found.len() {
                1 => {}
                0 => {
                    return Err(Refused(format!(
                        "the #…# on line {line} is missing from the formatted text"
                    )))
                }
                n => {
                    return Err(Refused(format!(
                        "the #…# on line {line} is in the formatted text {n} times"
                    )))
                }
            }
            if at.last().is_some_and(|&prev| prev > found[0]) {
                return Err(Refused(format!(
                    "the #…# on line {line} moved past another #…#"
                )));
            }
            at.push(found[0]);
        }
        let regions = regions_of(out, self.lang);
        // The parentheses open at each hole, in the text handed off and in
        // the output.
        let was = depths(
            &self.text,
            &regions_of(&self.text, self.lang),
            self.holes.iter().map(|h| h.at),
        );
        let is = depths(out, &regions, at.iter().copied());
        for (i, &o) in at.iter().enumerate() {
            self.check(i, out, &at, site_at(&regions, o).0, (was[i], is[i]))?;
        }
        let mut restored = String::with_capacity(out.len() + out.len() / 8);
        let mut from = 0;
        for (h, &o) in self.holes.iter().zip(&at) {
            restored.push_str(&out[from..o].replace('#', "##"));
            restored.push_str(&h.hole.text);
            from = o + h.placeholder.len();
        }
        restored.push_str(&out[from..].replace('#', "##"));
        Ok(restored)
    }

    /// The checks on hole `n`: its site, then for a hole in code its
    /// neighbours. `at` is every placeholder's position in `out`; `depth`
    /// the parentheses open at the hole before and after ([`depths`]).
    fn check(
        &self,
        n: usize,
        out: &str,
        at: &[usize],
        site: Site,
        depth: (isize, isize),
    ) -> Result<(), Refused> {
        let (h, o) = (&self.holes[n], at[n]);
        let line = h.hole.line;
        // A neighbour as a message shows it: the page's character, so a
        // byte of another hole's placeholder is that hole's `#`.
        let was_face = |i: usize, c: char| {
            let held = |p: &Placed| (p.at..p.at + p.placeholder.len()).contains(&i);
            if self.holes.iter().any(held) {
                '#'
            } else {
                c
            }
        };
        let is_face = |i: usize, c: char| {
            let held = |(p, &a): (&Placed, &usize)| (a..a + p.placeholder.len()).contains(&i);
            if self.holes.iter().zip(at).any(held) {
                '#'
            } else {
                c
            }
        };
        let refuse = |message: String| Err(Refused(message));
        if site != h.site {
            return refuse(format!(
                "the #…# on line {line} moved from {} into {}",
                h.site.name(),
                site.name()
            ));
        }
        if !site.checked() {
            return Ok(());
        }
        let text = &self.text;
        let len = h.placeholder.len();
        let (was, is) = (before(text, h.at), before(out, o));
        if was.map(|w| w.1) != is.map(|i| i.1) {
            return refuse(format!(
                "what precedes the #…# on line {line} changed from {} to {}",
                shown(was.map(|(i, c)| was_face(i, c))),
                shown(is.map(|(i, c)| is_face(i, c)))
            ));
        }
        let (was, is) = (after(text, h.at + len), after(out, o + len));
        let (was_c, is_c) = (was.map(|w| w.1), is.map(|i| i.1));
        if was_c != is_c {
            // Added, not put in place of something: what follows the `;` or
            // the `,` is what followed the hole (`(y + #x#);` printed
            // `y + #x#;` lost a parenthesis, not gained a `;`).
            let then = |i: usize| after(out, i + 1).map(|(_, c)| c);
            let allowed = match is {
                // A statement's terminator added; not before an `else`,
                // which a `#…#` emitting a whole statement could not take.
                Some((i, ';')) if before_else(out, i + 1) => {
                    return refuse(format!(
                        "a `;` was added after the #…# on line {line}, before `else`"
                    ))
                }
                Some((i, ';')) => then(i) == was_c,
                // A trailing comma after a property value.
                Some((i, ',')) => was_c == Some('}') && then(i) == was_c,
                _ => false,
            };
            if !allowed {
                let was = shown(was.map(|(i, c)| was_face(i, c)));
                return refuse(match is {
                    Some((_, ',')) => {
                        format!("a `,` was added after the #…# on line {line}, before {was}")
                    }
                    _ => format!(
                        "what follows the #…# on line {line} changed from {was} to {}",
                        shown(is.map(|(i, c)| is_face(i, c)))
                    ),
                });
            }
        }
        // What followed it on a later line is not brought up to its line:
        // a `#…#` on a line of its own may emit whole statements, or end in
        // a `//` comment, and `#lib#` ⏎ `(function () {…})();` printed
        // `#lib#(function () {…})();` is then another program. A closing
        // bracket and a separator end what the `#…#` is in, and may join it.
        if let (Some((w, _)), Some((i, c))) = (was, is) {
            let broke = |t: &str, from: usize, to: usize| t[from..to].contains('\n');
            if broke(text, h.at + len, w)
                && !broke(out, o + len, i)
                && !matches!(c, ')' | ']' | '}' | ',' | ';')
            {
                return refuse(format!(
                    "the #…# on line {line} was joined to the `{}` on the line after it",
                    is_face(i, c)
                ));
            }
        }
        // The parentheses around it, of which the checks above see only
        // those against it: `foo((#x#))` printed `foo(#x#)`, or
        // `(y + #x# + w) || z` printed `y + #x# + w || z`, passes them, and
        // changes what an emitted `1, 2` means.
        if depth.0 != depth.1 {
            return refuse(format!(
                "the parentheses around the #…# on line {line} changed"
            ));
        }
        // `i` is where the source's neighbour is, for its face.
        let glued = |side: &str, i: usize, was: Option<char>, is: Option<char>| match (was, is) {
            (Some(c), _) if is_word(c) && is != was => refuse(format!(
                "the #…# on line {line} is no longer joined to the `{}` {side} it",
                was_face(i, c)
            )),
            // Newly joined to an operator: `- #x#` printed `-#x#` reads
            // `--1` when the `#…#` emits `-1`.
            (Some(w), Some(c)) if w.is_ascii_whitespace() && is_operator(c) => refuse(format!(
                "the #…# on line {line} is now joined to the `{c}` {side} it"
            )),
            _ => Ok(()),
        };
        glued(
            "before",
            h.at.saturating_sub(1),
            text[..h.at].chars().next_back(),
            out[..o].chars().next_back(),
        )?;
        glued(
            "after",
            h.at + len,
            text[h.at + len..].chars().next(),
            out[o + len..].chars().next(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pieces of `src`, an island's text written as in a template:
    /// `##` is a hash, `#…#` a hole (printed as written, on line 1).
    fn pieces(src: &str) -> Vec<Piece<'_>> {
        let mut out = Vec::new();
        let mut rest = src;
        while let Some(at) = rest.find('#') {
            if at > 0 {
                out.push(Piece::Text(&rest[..at]));
            }
            if rest[at..].starts_with("##") {
                out.push(Piece::Hash);
                rest = &rest[at + 2..];
                continue;
            }
            let end = at + 1 + rest[at + 1..].find('#').expect("an unclosed hole");
            out.push(Piece::Hole(Hole {
                text: rest[at..=end].to_owned(),
                line: 1,
            }));
            rest = &rest[end + 1..];
        }
        if !rest.is_empty() {
            out.push(Piece::Text(rest));
        }
        out
    }

    fn js(src: &str) -> Interpolated {
        substitute(pieces(src), Lang::Js).expect("eligible")
    }

    fn css(src: &str) -> Interpolated {
        substitute(pieces(src), Lang::Css).expect("eligible")
    }

    /// `src` handed off, its hand-off text edited by `edit` as a formatter
    /// might, and put back.
    fn round(i: &Interpolated, edit: impl Fn(&str) -> String) -> Result<String, String> {
        i.restore(&edit(&i.text)).map_err(|Refused(m)| m)
    }

    #[test]
    fn the_hand_off_text() {
        // `##` is `#`; a hole an identifier as wide as it prints, at least
        // the stem, its number and `_`; the blank edge lines dropped as for
        // a pure island.
        assert_eq!(js("\n$( \"##a\" );\n").text, "$( \"#a\" );\n");
        assert_eq!(js("x = #a#;").text, "x = zq0_;\n");
        assert_eq!(js("x = #prc.name#;").text, "x = zq0_______;\n");
        // Numbered in order, none a prefix of another.
        let many: String = (0..12).map(|i| format!("f(#v{i}#);\n")).collect();
        let text = js(&many).text;
        assert!(text.starts_with("f(zq0_);\nf(zq1_);\n"), "{text}");
        assert!(text.ends_with("f(zq10_);\nf(zq11_);\n"), "{text}");
        // The width is the printed text's, in columns.
        assert_eq!(js("x = #é.ü.ö.ä#;").text, "x = zq0______;\n");
        // A stem the text holds is passed over.
        assert_eq!(js("zq = #a#;").text, "zq = qx0_;\n");
        let all: String = STEMS.join(" ");
        assert!(substitute(pieces(&format!("{all} #a#")), Lang::Js).is_none());
    }

    #[test]
    fn not_eligible() {
        // A lone hash in a text, nothing but whitespace outside the holes.
        let lone = vec![
            Piece::Text("a # b "),
            Piece::Hole(Hole {
                text: "#x#".into(),
                line: 1,
            }),
        ];
        assert!(substitute(lone, Lang::Js).is_none());
        assert!(substitute(pieces("  #x#  \n"), Lang::Css).is_none());
        assert!(substitute(pieces("#a##b#"), Lang::Css).is_none());
        // `##` is text.
        assert!(substitute(pieces("##"), Lang::Css).is_some());
        assert!(substitute(pieces("x #a#"), Lang::Css).is_some());
    }

    #[test]
    fn a_string_holding_a_hole_pins_its_quote() {
        // The other quote, once more than the string's own holds.
        assert_eq!(js("x = '#a#';").text, "x = 'zq0_\"';\n");
        assert_eq!(js("x = \"#a#\";").text, "x = \"zq0_'\";\n");
        assert_eq!(js("x = 'it\\'s #a#';").text, "x = 'it\\'s zq0_\"\"';\n");
        assert_eq!(js("x = 'say \"hi\" #a#';").text, "x = 'say \"hi\" zq0_';\n");
        // Inside the width when the hole is wide enough.
        assert_eq!(js("x = '#abcdef#';").text, "x = 'zq0_\"___';\n");
        // Once per string, in its first hole.
        assert_eq!(js("x = '#a# #b#';").text, "x = 'zq0_\" zq1_';\n");
        assert_eq!(
            css("a { content: '#a#' }").text,
            "a { content: 'zq0_\"' }\n"
        );
        // A template literal, a comment and a regular expression need none.
        assert_eq!(js("x = `#a#`;").text, "x = `zq0_`;\n");
        assert_eq!(js("// #a#\nx = /#a#/;").text, "// zq0_\nx = /zq1_/;\n");
    }

    #[test]
    fn restoring_puts_back_holes_and_hashes() {
        let i = js("$( \"##a\" ).on(#ev#, '#sel#');\n");
        assert_eq!(i.text, "$( \"#a\" ).on(zq0_, 'zq1_\"');\n");
        let out = round(&i, |t| {
            t.replace("$( ", "$(")
                .replace(" )", ")")
                .replace('\n', ";\n")
        });
        assert_eq!(out.as_deref(), Ok("$(\"##a\").on(#ev#, '#sel#');;\n"));
        // A hole that prints wider than its placeholder's minimum keeps its
        // own text: the placeholder is only its width.
        let i = js("f( #  a  # );");
        assert_eq!(i.text, "f( zq0____ );\n");
        assert_eq!(
            round(&i, |t| t.replace("( ", "(").replace(" )", ")")).as_deref(),
            Ok("f(#  a  #);\n")
        );
    }

    #[test]
    fn each_placeholder_comes_back_once_in_order() {
        let i = js("f(#a#, #b#);");
        assert_eq!(
            round(&i, |t| t.replace("zq0_", "")),
            Err("the #…# on line 1 is missing from the formatted text".into())
        );
        assert_eq!(
            round(&i, |t| t.replace("zq0_", "zq0_, zq0_")),
            Err("the #…# on line 1 is in the formatted text 2 times".into())
        );
        assert_eq!(
            round(&i, |_| "f(zq1_, zq0_);\n".into()),
            Err("the #…# on line 1 moved past another #…#".into())
        );
        assert!(round(&i, |t| t.replace(", ", ",")).is_ok());
    }

    #[test]
    fn a_hole_stays_where_it_was() {
        // The quote of its string: a pinned placeholder is not found in a
        // string of the other quote (its pin would be escaped), and one with
        // no pin is found, at another site.
        let i = js("x = '#a#';");
        assert_eq!(
            round(&i, |_| "x = \"zq0_\\\"\";\n".into()),
            Err("the #…# on line 1 is missing from the formatted text".into())
        );
        assert!(round(&i, |t| t.replace(" = ", "=")).is_ok());
        let i = js("x = 'say \"hi\" #a#';");
        assert_eq!(
            round(&i, |_| "x = \"say \\\"hi\\\" zq0_\";\n".into()),
            Err("the #…# on line 1 moved from a '…' string into a \"…\" string".into())
        );
        // Out of a comment, a template literal.
        let i = js("x = 1; // #a#");
        assert_eq!(
            round(&i, |_| "x = 1;\nzq0_;\n".into()),
            Err("the #…# on line 1 moved from a comment into code".into())
        );
        let i = js("x = `#a#`;");
        assert_eq!(
            round(&i, |_| "x = zq0_;\n".into()),
            Err("the #…# on line 1 moved from a template literal into code".into())
        );
        // Into a string: `quoteProps` quoting a key.
        let i = js("x = { #k#: 1, \"a-b\": 2 };");
        assert_eq!(
            round(&i, |t| t.replace("zq0_", "\"zq0_\"")),
            Err("the #…# on line 1 moved from code into a \"…\" string".into())
        );
        // CSS: a `url(…)` body and a string.
        let i = css("a { b: url(#u#/x.png); c: \"#s#\" }");
        assert_eq!(i.text, "a { b: url(zq0_/x.png); c: \"zq1_'\" }\n");
        assert!(round(&i, |t| t.replace("{ ", "{\n  ")).is_ok());
        assert_eq!(
            round(&i, |t| t.replace("url(zq0_/x.png)", "url(\"zq0_/x.png\")")),
            Err("the #…# on line 1 moved from a url(…) into a \"…\" string".into())
        );
    }

    #[test]
    fn a_hole_in_code_keeps_what_precedes_it() {
        // The parenthesis guard.
        for (src, edited) in [
            ("(#x#).call();", "zq0_.call();\n"),
            ("(#x#.foo)();", "zq0_.foo();\n"),
            ("return (#x#);", "return zq0_;\n"),
        ] {
            let i = js(src);
            let out = round(&i, |_| edited.into());
            assert!(
                out.as_ref().is_err_and(
                    |m| m.starts_with("what precedes the #…# on line 1 changed from `(` to")
                ),
                "{src}: {out:?}"
            );
        }
        // Whitespace around it may change.
        assert!(round(&js("foo( #x# );"), |_| "foo(zq0_);\n".into()).is_ok());
        assert!(round(&js("if ( #x# ) {}"), |_| "if (zq0_) {\n}\n".into()).is_ok());
        // At the start of the text there is nothing before it.
        let i = js("#x#\nf();");
        assert_eq!(
            round(&i, |_| ";zq0_\nf();\n".into()),
            Err("what precedes the #…# on line 1 changed from nothing to `;`".into())
        );
    }

    #[test]
    fn a_hole_in_code_keeps_what_follows_it() {
        // A statement's terminator, a trailing comma before `}`.
        assert_eq!(
            round(&js("#x#\nvar a = 1"), |_| "zq0_;\nvar a = 1;\n".into()).as_deref(),
            Ok("#x#;\nvar a = 1;\n")
        );
        assert!(round(&js("f();\n#x#"), |_| "f();\nzq0_;\n".into()).is_ok());
        assert!(round(&js("o = { a: 1,\n b: #x#\n};"), |_| {
            "o = {\n  a: 1,\n  b: zq0_,\n};\n".into()
        })
        .is_ok());
        // A comma before `]` or `)` is not.
        assert_eq!(
            round(&js("ids = [#l#];"), |_| "ids = [\n  zq0_,\n];\n".into()),
            Err("a `,` was added after the #…# on line 1, before `]`".into())
        );
        assert_eq!(
            round(&js("f(a, #x#)"), |_| "f(a, zq0_,);\n".into()),
            Err("a `,` was added after the #…# on line 1, before `)`".into())
        );
        // Anything else.
        assert_eq!(
            round(&js("(#x#).call();"), |_| "(zq0_.call());\n".into()),
            Err("what follows the #…# on line 1 changed from `)` to `.`".into())
        );
        // A `;` before an `else`: `if (a) { … }; else` does not parse.
        assert_eq!(
            round(&js("if (a) #x#\nelse b();"), |_| {
                "if (a) zq0_;\nelse b();\n".into()
            }),
            Err("a `;` was added after the #…# on line 1, before `else`".into())
        );
        assert!(round(&js("#x#\nelsewhere();"), |_| "zq0_;\nelsewhere();\n".into()).is_ok());
    }

    #[test]
    fn a_message_shows_a_neighbouring_hole_as_a_hash() {
        // Not a byte of its placeholder.
        let i = js("#a#\n#b#\nvar c = 1;");
        assert_eq!(
            round(&i, |_| "zq0_;\nzq1_;\nvar c = 1;\n".into()),
            Err("what precedes the #…# on line 1 changed from `#` to `;`".into())
        );
        assert_eq!(
            round(&i, |_| "zq0_(zq1_);\nvar c = 1;\n".into()),
            Err("what follows the #…# on line 1 changed from `#` to `(`".into())
        );
        assert_eq!(
            round(&js("x = #a##b#;"), |t| t.replace("_zq", "_ zq")),
            Err("the #…# on line 1 is no longer joined to the `#` after it".into())
        );
    }

    #[test]
    fn a_hole_is_not_joined_to_the_line_after_it() {
        // What the source had on the next line stays there, or the `#…#`
        // gains a separator.
        let i = js("#lib#\n(function () {})();");
        assert_eq!(
            round(&i, |t| t.replace('\n', "")),
            Err("the #…# on line 1 was joined to the `(` on the line after it".into())
        );
        assert!(round(&i, |t| t.to_owned()).is_ok());
        assert_eq!(
            round(&js("x = #o#\n  .a();"), |_| "x = zq0_.a();\n".into()),
            Err("the #…# on line 1 was joined to the `.` on the line after it".into())
        );
        assert_eq!(
            round(&css(".a { }\n#rules#\n.b { }"), |_| {
                ".a {\n}\nzq0____ .b {\n}\n".into()
            }),
            Err("the #…# on line 1 was joined to the `.` on the line after it".into())
        );
        // A closing bracket or a separator may come up to its line.
        assert!(round(&js("f(\n  a,\n  #x#\n);"), |_| "f(a, zq0_);\n".into()).is_ok());
        assert!(round(&js("a = [\n  #x#\n  , 1];"), |_| "a = [zq0_, 1];\n".into()).is_ok());
        assert!(round(&js("o = {\n  #x#\n};"), |_| "o = { zq0_ };\n".into()).is_ok());
        // On one line in the source, there is nothing to keep.
        assert!(round(&js("x = #o# .a();"), |_| "x = zq0_.a();\n".into()).is_ok());
    }

    #[test]
    fn a_hole_glued_to_a_word_stays_glued() {
        let i = js("function #name#Callback() {}");
        assert_eq!(i.text, "function zq0___Callback() {}\n");
        assert_eq!(
            round(&i, |t| t.replace("_Callback", "_ Callback")),
            Err("the #…# on line 1 is no longer joined to the `C` after it".into())
        );
        assert!(round(&i, |t| t.replace("{}", "{\n}")).is_ok());
        // `.#cls#`, `#w#px`, `margin-#side#`, adjacent holes, `###x#`.
        let i = css(".#cls# { width: #w#px; margin-#s#: 0; color: ###c#; }");
        assert_eq!(
            i.text,
            ".zq0__ { width: zq1_px; margin-zq2_: 0; color: #zq3_; }\n"
        );
        assert!(round(&i, |t| t.replace("{ ", "{\n  ").replace("; ", ";\n  ")).is_ok());
        assert_eq!(
            round(&i, |t| t.replace(".zq0__", ". zq0__")),
            Err("the #…# on line 1 is no longer joined to the `.` before it".into())
        );
        assert_eq!(
            round(&i, |t| t.replace("zq1_px", "zq1_ px")),
            Err("the #…# on line 1 is no longer joined to the `p` after it".into())
        );
        let i = js("x = #a##b#;");
        assert_eq!(i.text, "x = zq0_zq1_;\n");
        assert!(round(&i, |t| t.to_owned()).is_ok());
        // Not glued: whitespace between may come and go, but a `#…#` newly
        // joined to an operator could run into it.
        assert!(round(&js("f( #a#, 1 )"), |_| "f(zq0_,1);\n".into()).is_ok());
        assert_eq!(
            round(&js("x = - #a#;"), |t| t.replace("- ", "-")),
            Err("the #…# on line 1 is now joined to the `-` before it".into())
        );
        assert_eq!(
            round(&js("x = #a# + 1;"), |t| t.replace(" + ", "+")),
            Err("the #…# on line 1 is now joined to the `+` after it".into())
        );
    }

    #[test]
    fn a_hole_in_code_keeps_its_parentheses() {
        // The innermost pair is the neighbours' business; the others are
        // counted, wherever they are, and not those in literals.
        assert_eq!(
            round(&js("foo((#x#));"), |_| "foo(zq0_);\n".into()),
            Err("the parentheses around the #…# on line 1 changed".into())
        );
        assert_eq!(
            round(&js("x = (y + #x# + w) || z;"), |_| {
                "x = y + zq0_ + w || z;\n".into()
            }),
            Err("the parentheses around the #…# on line 1 changed".into())
        );
        assert!(round(&js("f(')', /(/, `(`); g(a)(#x#);"), |t| t
            .replace("; ", ";\n"))
        .is_ok());
        assert!(round(&css("a { b: url(c.png) #x#; }"), |t| t.replace("{ ", "{\n")).is_ok());
        assert_eq!(
            round(&js("x = a && #x# || c;"), |_| "x = (a && zq0_) || c;\n"
                .into()),
            Err("what follows the #…# on line 1 changed from `|` to `)`".into())
        );
        assert_eq!(
            round(&js("foo(( #x# ));"), |_| "foo( ( zq0_ ) );\n".into()),
            Ok("foo( ( #x# ) );\n".into())
        );
        // A `;` in place of a `)`, not added: a parenthesis lost.
        assert_eq!(
            round(&js("x = (y + #x#);"), |_| "x = y + zq0_;\n".into()),
            Err("what follows the #…# on line 1 changed from `)` to `;`".into())
        );
        // A `,` before `}` in place of something else.
        assert_eq!(
            round(&js("o = { a: (y + #x#) }"), |_| "o = { a: y + zq0_, }\n"
                .into()),
            Err("a `,` was added after the #…# on line 1, before `)`".into())
        );
    }

    #[test]
    fn the_scan_finds_the_sites() {
        let sites = |src: &str, lang| {
            let i = substitute(pieces(src), lang).unwrap();
            i.holes.iter().map(|h| h.site).collect::<Vec<_>>()
        };
        assert_eq!(
            sites(
                "f(#a#, '#b#', \"#c#\", `x ${#d#} #e#`, /#f#/); // #g#\n/* #h# */",
                Lang::Js
            ),
            [
                Site::Code,
                Site::String(b'\''),
                Site::String(b'"'),
                Site::Code,
                Site::Template,
                Site::Regex,
                Site::Comment,
                Site::Comment,
            ]
        );
        assert_eq!(
            sites(
                ".#a# { b: url(#c#); d: url('#e#'); f: \"#g#\" } /* #h# */",
                Lang::Css
            ),
            [
                Site::Code,
                Site::Url,
                Site::String(b'\''),
                Site::String(b'"'),
                Site::Comment,
            ]
        );
    }
}
