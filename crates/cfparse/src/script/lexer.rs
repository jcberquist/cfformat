//! Byte-oriented scanners and the word lists the CFScript parser matches on.
//!
//! Every scanner takes the source and a byte offset and returns the offset
//! just past what it matched, so the parser can try alternatives in order,
//! the first that matches winning. Nothing here allocates.
//!
//! The long word lists are `data/*.json`, from which `build.rs` generates
//! [`SUPPORT_FUNCTIONS`] and [`TAGS_IN_SCRIPT`].

use crate::tree::{Literal, Operator, Prec, TokenKind};

include!(concat!(env!("OUT_DIR"), "/name_lists.rs"));

// ---------------------------------------------------------------------------
// Word lists written out here (short enough that generating them would hide
// them from the reader)
// ---------------------------------------------------------------------------

/// The CFML scope names (`variables`, `url`, …): a bare one is an
/// `Ident::ScopeVar` token.
pub(crate) const SCOPE_VARIABLES: &[&str] = &[
    "application",
    "argumentcollection",
    "arguments",
    "attributes",
    "caller",
    "cgi",
    "client",
    "cookie",
    "flash",
    "form",
    "local",
    "request",
    "self",
    "server",
    "session",
    "static",
    "thistag",
    "url",
    "variables",
];

/// Access modifiers of a function declaration.
pub(crate) const ACCESS_MODIFIERS: &[&str] = &["package", "private", "public", "remote"];

/// Storage modifiers of a declaration.
pub(crate) const STORAGE_MODIFIERS: &[&str] = &["abstract", "final", "static"];

/// Reserved words (`default` is not reserved: outside a `switch` it is an
/// ordinary identifier, `var default = 1`).
pub(crate) const RESERVED_WORDS: &[&str] = &[
    "break", "case", "catch", "continue", "do", "else", "false", "finally", "for", "function",
    "if", "import", "in", "new", "null", "return", "super", "switch", "this", "true", "try", "var",
    "void", "while",
];

/// The attribute names `param x` stops its inline name at.
pub(crate) const PARAM_ATTRIBUTES: &[&str] = &[
    "default",
    "max",
    "maxlength",
    "min",
    "name",
    "pattern",
    "type",
];

/// ASCII-case-insensitive membership in a sorted list.
pub(crate) fn in_list(list: &[&str], name: &str) -> bool {
    list.binary_search_by(|probe| {
        let mut p = probe.bytes();
        let mut n = name.bytes().map(|b| b.to_ascii_lowercase());
        loop {
            match (p.next(), n.next()) {
                (None, None) => return std::cmp::Ordering::Equal,
                (None, Some(_)) => return std::cmp::Ordering::Less,
                (Some(_), None) => return std::cmp::Ordering::Greater,
                (Some(a), Some(b)) if a == b => {}
                (Some(a), Some(b)) => return a.cmp(&b),
            }
        }
    })
    .is_ok()
}

pub(crate) fn is_support_function(name: &str) -> bool {
    in_list(SUPPORT_FUNCTIONS, name)
}

/// A script tag name, with or without the `cf` prefix.
pub(crate) fn is_tag_in_script(name: &str) -> bool {
    in_list(TAGS_IN_SCRIPT, name)
        || name
            .get(..2)
            .is_some_and(|p| p.eq_ignore_ascii_case("cf") && in_list(TAGS_IN_SCRIPT, &name[2..]))
}

/// A `cf`-prefixed script tag name only.
pub(crate) fn is_cf_tag_in_script(name: &str) -> bool {
    name.get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("cf") && in_list(TAGS_IN_SCRIPT, &name[2..]))
}

// ---------------------------------------------------------------------------
// Character classes
// ---------------------------------------------------------------------------

/// `[_$[:alpha:]]`, the first character of an identifier.
pub(crate) fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

/// `[_$[:alnum:]]`, a later character of an identifier.
pub(crate) fn is_ident_part(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

/// `\b` immediately before `at`: a word boundary by the identifier's own
/// rule, [`is_ident_part`]. Unlike a regex `\b`, whose word
/// characters exclude `$`, `$` is a word character here, so `var$foo` is one
/// identifier and `and$` is not the operator `and`.
pub(crate) fn word_boundary_before(src: &str, at: usize) -> bool {
    let before = src[..at].chars().next_back().is_some_and(is_ident_part);
    let after = src[at..].chars().next().is_some_and(is_ident_part);
    before != after
}

/// `\B` immediately before `at`.
pub(crate) fn no_word_boundary_before(src: &str, at: usize) -> bool {
    !word_boundary_before(src, at)
}

// ---------------------------------------------------------------------------
// Scanners
// ---------------------------------------------------------------------------

/// An identifier: [`is_ident_start`], then [`is_ident_part`]s.
pub(crate) fn identifier(src: &str, at: usize) -> Option<usize> {
    let mut it = src[at..].char_indices();
    let (_, first) = it.next()?;
    if !is_ident_start(first) {
        return None;
    }
    let mut end = at + first.len_utf8();
    for (i, c) in it {
        if !is_ident_part(c) {
            return Some(at + i);
        }
        end = at + i + c.len_utf8();
    }
    Some(end)
}

/// An attribute name: a letter or `_`, then letters, digits, `_`, `-` and
/// `:`, ending on its last [`is_ident_part`] (a trailing `-` or `:` is not
/// part of the name, so `access:"remote"` names the attribute `access`).
pub(crate) fn attribute_name(src: &str, at: usize) -> Option<usize> {
    let first = src[at..].chars().next()?;
    if first != '_' && !first.is_alphabetic() {
        return None;
    }
    let mut end = at + first.len_utf8();
    let mut last_word = end;
    for c in src[end..].chars() {
        if !(c.is_alphanumeric() || matches!(c, '_' | '-' | ':')) {
            break;
        }
        end += c.len_utf8();
        if is_ident_part(c) {
            last_word = end;
        }
    }
    Some(last_word)
}

/// A dotted path: `[_$[:alnum:]][_$[:alnum:].]*`.
pub(crate) fn dot_path(src: &str, at: usize) -> Option<usize> {
    let first = src[at..].chars().next()?;
    if !is_ident_part(first) {
        return None;
    }
    let end = at + first.len_utf8();
    for (i, c) in src[end..].char_indices() {
        if !is_ident_part(c) && c != '.' {
            return Some(end + i);
        }
    }
    Some(src.len())
}

/// A number literal: `0x` and hex digits, `.5`, or digits with an optional
/// fraction, then an optional exponent ([`exponent`]). A sign is never part
/// of it: in expression position `-` and `+` are prefix operators. `0x1e3` is
/// one hex literal.
pub(crate) fn number(src: &str, at: usize) -> Option<usize> {
    let b = src.as_bytes();
    if b.get(at) == Some(&b'0')
        && matches!(b.get(at + 1), Some(b'x' | b'X'))
        && word_boundary_before(src, at)
    {
        let mut end = at + 2;
        while b.get(end).is_some_and(|c| c.is_ascii_hexdigit()) {
            end += 1;
        }
        return Some(end);
    }
    if b.get(at) == Some(&b'.') {
        // `\B\.[0-9]+`
        if !no_word_boundary_before(src, at) || !b.get(at + 1).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        let mut end = at + 1;
        while b.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        return Some(exponent(b, end));
    }
    // `\b[0-9]+(\.[0-9]*)?`
    if !b.get(at).is_some_and(u8::is_ascii_digit) || !word_boundary_before(src, at) {
        return None;
    }
    let mut end = at;
    while b.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if b.get(end) == Some(&b'.') {
        end += 1;
        while b.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    Some(exponent(b, end))
}

/// `([eE][+-]?[0-9]+)?` after a number's digits, so `1e3` is one number, not
/// `1` and the identifier `e3`. The exponent is taken only when a digit
/// follows the `e` and its optional sign, so `1e`, `1ex` and `1e+` stay the
/// number `1` and what follows.
fn exponent(b: &[u8], end: usize) -> usize {
    if !matches!(b.get(end), Some(b'e' | b'E')) {
        return end;
    }
    let mut digits = end + 1;
    if matches!(b.get(digits), Some(b'+' | b'-')) {
        digits += 1;
    }
    if !b.get(digits).is_some_and(u8::is_ascii_digit) {
        return end;
    }
    while b.get(digits).is_some_and(u8::is_ascii_digit) {
        digits += 1;
    }
    digits
}

/// A keyword at `at`, ASCII case-insensitively, with `\b` on both sides.
/// Every keyword rule uses it: CFML keywords are case-insensitive, so `IF`,
/// `Var` and `RETURN` are keywords, and the word lists are searched with
/// [`in_list`].
pub(crate) fn keyword_at(src: &str, at: usize, word: &str) -> bool {
    let Some(head) = src.get(at..at + word.len()) else {
        return false;
    };
    head.eq_ignore_ascii_case(word)
        && word_boundary_before(src, at)
        && !src[at + word.len()..].starts_with(is_ident_part)
}

/// The end of one whitespace token: a run of whitespace up to and including
/// the first newline, or to the end of the run: a whitespace token never
/// continues past a `\n`.
pub(crate) fn whitespace(src: &str, at: usize) -> Option<usize> {
    let b = src.as_bytes();
    if !b.get(at).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let mut end = at;
    while let Some(&c) = b.get(end) {
        if !c.is_ascii_whitespace() {
            break;
        }
        end += 1;
        if c == b'\n' {
            break;
        }
    }
    Some(end)
}

/// End of the line at `at`, newline included.
pub(crate) fn line_end(src: &str, at: usize) -> usize {
    match src[at..].find('\n') {
        Some(i) => at + i + 1,
        None => src.len(),
    }
}

/// The comments [`skip_trivia`] looks past besides whitespace.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comments {
    /// `/* … */` only: a `//` ends the skip.
    Block,
    /// `/* … */` and `//` to the end of its line.
    All,
}

/// Skips whitespace and complete comments, for the lookaheads that choose
/// between forms. A `/*` that never closes runs to the end of `src`.
pub(crate) fn skip_trivia(src: &str, mut at: usize, comments: Comments) -> usize {
    loop {
        if src.as_bytes().get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        } else if src[at..].starts_with("/*") {
            match src[at + 2..].find("*/") {
                Some(i) => at += 2 + i + 2,
                None => return src.len(),
            }
        } else if comments == Comments::All && src[at..].starts_with("//") {
            at = line_end(src, at);
        } else {
            return at;
        }
    }
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

/// One row of an operator table: the spelling and the token kind it gets. Word
/// operators need `\b`; symbol operators are matched in table order, longest
/// first within a group.
pub(crate) struct Op {
    pub(crate) text: &'static str,
    pub(crate) word: bool,
    pub(crate) kind: TokenKind,
}

const fn sym(text: &'static str, kind: TokenKind) -> Op {
    Op {
        text,
        word: false,
        kind,
    }
}

const fn word(text: &'static str, kind: TokenKind) -> Op {
    Op {
        text,
        word: true,
        kind,
    }
}

/// The binary operators, in matching order, each with its kind: the
/// expression pass reads a binary operator's level from it, never from the
/// text. `=` and `,` are handled by the parser (`=` must not be followed by
/// `=` or `>`, and whether `,` is an operator depends on where it sits), `?`
/// by the ternary rule.
pub(crate) const BINARY_OPERATORS: &[Op] = &[
    word("in", TokenKind::Operator(Operator::In)),
    sym("&&", binary(Prec::And)),
    sym("||", binary(Prec::Or)),
    word("and", binary(Prec::And)),
    word("or", binary(Prec::Or)),
    word("xor", binary(Prec::Xor)),
    word("eqv", binary(Prec::Eqv)),
    word("imp", binary(Prec::Imp)),
    sym("%=", aug_assign(Prec::Modulus)),
    sym("&=", aug_assign(Prec::Concat)),
    sym("*=", aug_assign(Prec::Multiplicative)),
    sym("+=", aug_assign(Prec::Additive)),
    sym("-=", aug_assign(Prec::Additive)),
    sym("/=", aug_assign(Prec::Multiplicative)),
    sym("&", binary(Prec::Concat)),
    sym("===", binary(Prec::Comparison)),
    sym("!==", binary(Prec::Comparison)),
    sym("==", binary(Prec::Comparison)),
    word("neq", binary(Prec::Comparison)),
    sym("!=", binary(Prec::Comparison)),
    sym("<>", binary(Prec::Comparison)),
    word("eq", binary(Prec::Comparison)),
    // CFML's `equal` (= `eq`).
    word("equal", binary(Prec::Comparison)),
    sym("<=", binary(Prec::Comparison)),
    word("lte", binary(Prec::Comparison)),
    word("le", binary(Prec::Comparison)),
    sym(">=", binary(Prec::Comparison)),
    word("gte", binary(Prec::Comparison)),
    word("ge", binary(Prec::Comparison)),
    sym("<", binary(Prec::Comparison)),
    word("lt", binary(Prec::Comparison)),
    sym(">", binary(Prec::Comparison)),
    word("gt", binary(Prec::Comparison)),
    word("mod", binary(Prec::Modulus)),
    sym("^", binary(Prec::Exponent)),
    sym("/", binary(Prec::Multiplicative)),
    sym("\\", binary(Prec::Multiplicative)),
    sym("%", binary(Prec::Modulus)),
    sym("*", binary(Prec::Multiplicative)),
    sym("+", binary(Prec::Additive)),
    sym("-", binary(Prec::Additive)),
];

const fn binary(prec: Prec) -> TokenKind {
    TokenKind::Operator(Operator::Binary(prec))
}

const fn aug_assign(prec: Prec) -> TokenKind {
    TokenKind::Operator(Operator::AugAssign(prec))
}

/// Multi-word operators, matched before [`BINARY_OPERATORS`]: any whitespace
/// run (`\s+`) separates the words. Every one is a comparison; the `bool` is
/// whether the phrase must be followed by whitespace.
pub(crate) const PHRASE_OPERATORS: &[(&[&str], bool)] = &[
    (&["is", "not"], false),
    // CFML's `not equal` (= `neq`).
    (&["not", "equal"], false),
    (&["less", "than", "or", "equal", "to"], false),
    (&["greater", "than", "or", "equal", "to"], false),
    (&["less", "than"], true),
    (&["greater", "than"], true),
    (&["does", "not", "contain"], false),
];

/// `is` and `contains`: comparisons on their own.
pub(crate) const WORD_COMPARISONS: &[Op] = &[
    word("is", binary(Prec::Comparison)),
    word("contains", binary(Prec::Comparison)),
];

/// The literal kind of a bare `true`, `false` or `null`, if the word is one;
/// case-insensitively, like every keyword (`TRUE`, `Null`).
pub(crate) fn special_name(word: &str) -> Option<TokenKind> {
    if word.eq_ignore_ascii_case("true") || word.eq_ignore_ascii_case("false") {
        Some(TokenKind::Literal(Literal::Bool))
    } else if word.eq_ignore_ascii_case("null") {
        Some(TokenKind::Literal(Literal::Null))
    } else {
        None
    }
}

/// The prefix operators, in matching order.
pub(crate) const PREFIX_OPERATORS: &[Op] = &[
    sym("--", TokenKind::Operator(Operator::Increment)),
    sym("++", TokenKind::Operator(Operator::Increment)),
    sym("...", TokenKind::Operator(Operator::Spread)),
    sym("+", TokenKind::Operator(Operator::Sign)),
    sym("-", TokenKind::Operator(Operator::Sign)),
];

/// One row of an operator table at `at`: the end of the match.
pub(crate) fn op_at(src: &str, at: usize, op: &Op) -> Option<usize> {
    let end = at + op.text.len();
    let hit = if op.word {
        keyword_at(src, at, op.text)
    } else {
        src.get(at..end) == Some(op.text)
    };
    hit.then_some(end)
}

/// A [`PHRASE_OPERATORS`] row at `at`: the end of its last word. The words
/// are separated by `\s+`; `less than` / `greater than` must also be
/// followed by a whitespace character (`less\s+than\s`), which is not part
/// of the match.
pub(crate) fn phrase_operator(src: &str, at: usize) -> Option<usize> {
    let spaces = |mut i: usize| {
        while src.as_bytes().get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        i
    };
    'rows: for (words, trailing_space) in PHRASE_OPERATORS {
        let mut pos = at;
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                let next = spaces(pos);
                if next == pos {
                    continue 'rows;
                }
                pos = next;
            }
            if !keyword_at(src, pos, w) {
                continue 'rows;
            }
            pos += w.len();
        }
        if *trailing_space && spaces(pos) == pos {
            continue;
        }
        return Some(pos);
    }
    None
}

/// Does a new line starting at `at` continue the previous statement? Yes when
/// it starts with a binary operator — any row of [`PHRASE_OPERATORS`],
/// [`BINARY_OPERATORS`] or [`WORD_COMPARISONS`], the tables the expression
/// parser reads, so the two cannot disagree — or with `=`, an index or call, a
/// member access, a separator or a ternary. `++` / `--` start a new statement,
/// and so do the prefix-only operators (`not`, `!`), which are in none of the
/// tables.
pub(crate) fn continues_statement(src: &str, at: usize) -> bool {
    let rest = &src[at..];
    if rest.starts_with("++") || rest.starts_with("--") {
        return false;
    }
    if rest.starts_with(['=', '[', '(', ';', ',', '.', ':', '?']) {
        return true;
    }
    phrase_operator(src, at).is_some()
        || BINARY_OPERATORS
            .iter()
            .chain(WORD_COMPARISONS)
            .any(|op| op_at(src, at, op).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_forms() {
        assert_eq!(number("123", 0), Some(3));
        assert_eq!(number("123.45x", 0), Some(6));
        assert_eq!(number("999.", 0), Some(4));
        assert_eq!(number("0xFF", 0), Some(4));
        assert_eq!(number("0x", 0), Some(2));
        assert_eq!(number(" .5", 1), Some(3));
        // `a.5` is a member access: `\B` fails before the dot.
        assert_eq!(number("a.5", 1), None);
        // `\b` before the digits: `a1` is an identifier.
        assert_eq!(number("a1", 1), None);
        assert_eq!(number("x", 0), None);
    }

    #[test]
    fn number_exponents_need_a_digit() {
        for (src, end) in [
            ("1e3", 3),
            ("1.2e-3", 6),
            ("1E+5", 4),
            ("1.e3", 4),
            ("1e3x", 3),
            ("1e", 1),
            ("1ex", 1),
            ("1e+", 1),
            ("1e+x", 1),
            ("1.5e", 3),
            ("0x1e3", 5),
        ] {
            assert_eq!(number(src, 0), Some(end), "{src}");
        }
        assert_eq!(number(" .5e2", 1), Some(5));
    }

    #[test]
    fn identifiers_and_paths() {
        assert_eq!(identifier("foo.bar", 0), Some(3));
        assert_eq!(identifier("$_a1 ", 0), Some(4));
        assert_eq!(identifier("1abc", 0), None);
        assert_eq!(dot_path("foo.bar baz", 0), Some(7));
        assert_eq!(dot_path("1foo.bar", 0), Some(8));
    }

    #[test]
    fn whitespace_stops_after_a_newline() {
        assert_eq!(whitespace("  \n  x", 0), Some(3));
        assert_eq!(whitespace("  \n  x", 3), Some(5));
        assert_eq!(whitespace("x", 0), None);
    }

    #[test]
    fn lists_are_sorted_and_searchable() {
        for list in [
            SUPPORT_FUNCTIONS,
            TAGS_IN_SCRIPT,
            SCOPE_VARIABLES,
            ACCESS_MODIFIERS,
            STORAGE_MODIFIERS,
            RESERVED_WORDS,
            PARAM_ATTRIBUTES,
        ] {
            assert!(
                list.windows(2).all(|w| w[0] < w[1]),
                "unsorted: {:?}",
                &list[..3]
            );
        }
        assert!(is_support_function("ArrayAppend"));
        assert!(is_support_function("writeOutput"));
        assert!(!is_support_function("notAFunction"));
        assert!(is_tag_in_script("http"));
        assert!(is_tag_in_script("cfhttp"));
        assert!(!is_tag_in_script("nope"));
        assert!(is_cf_tag_in_script("cffile"));
        assert!(!is_cf_tag_in_script("file"));
    }

    #[test]
    fn keyword_needs_word_boundaries() {
        assert!(keyword_at("var x", 0, "var"));
        assert!(!keyword_at("variable", 0, "var"));
        assert!(keyword_at("a.var", 2, "var"));
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert!(keyword_at("var x", 0, "var"));
        assert!(keyword_at("Var x", 0, "var"));
        assert!(keyword_at("IF (x)", 0, "if"));
        assert!(!keyword_at("Variable", 0, "var"));
        assert!(in_list(RESERVED_WORDS, "Return"));
        assert!(special_name("true").is_some());
        assert!(special_name("True").is_some());
        assert!(special_name("NULL").is_some());
        assert!(continues_statement("and b", 0));
        assert!(continues_statement("AND b", 0));
    }

    /// The statement count and the significant tokens of a script.
    fn statements_and_tokens(src: &str) -> (usize, Vec<String>) {
        let tree = crate::parse_source(src, crate::Mode::Script);
        let statements = tree
            .root
            .children
            .iter()
            .filter(|n| n.as_element().is_some_and(|e| e.kind.is_statement()))
            .count();
        let tokens = tree
            .root
            .tokens()
            .iter()
            .filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline))
            .map(|t| format!("{:?} {}", t.kind, tree.text(t)))
            .collect();
        (statements, tokens)
    }

    /// Generated from the tables: every binary operator, in either case, at
    /// the start of a line continues the statement the previous line began,
    /// and parses to the same tokens as on one line.
    #[test]
    fn every_binary_operator_continues_a_statement() {
        let mut ops: Vec<String> = BINARY_OPERATORS
            .iter()
            .chain(WORD_COMPARISONS)
            .map(|op| op.text.to_string())
            .collect();
        ops.extend(PHRASE_OPERATORS.iter().map(|(words, _)| words.join(" ")));
        assert!(ops.len() > 45, "{ops:?}");
        for op in ops.iter().flat_map(|op| [op.clone(), op.to_uppercase()]) {
            let one_line = statements_and_tokens(&format!("x = a {op} b;"));
            let two_lines = statements_and_tokens(&format!("x = a\n{op} b;"));
            assert_eq!(one_line.0, 1, "{op}: {:?}", one_line.1);
            assert_eq!(two_lines, one_line, "{op}");
            let op_token = &one_line.1[3];
            assert!(op_token.starts_with("Operator("), "{op}: {op_token}");
        }
        // Prefix-only operators and `++` / `--` start a new statement.
        for next in ["not b;", "NOT b;", "!b;", "++b;", "--b;"] {
            let (statements, _) = statements_and_tokens(&format!("x = a\n{next}"));
            assert_eq!(statements, 2, "{next}");
        }
    }
}
