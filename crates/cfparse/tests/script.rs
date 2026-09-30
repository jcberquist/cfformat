//! The script front end parses every fixture — script and tag fixtures alike,
//! as `Mode::Script` — without declining: `Declined` means
//! "not script" and nothing else.

mod common;

use std::path::Path;

use cfparse::{parse_source, script, ElementKind, Mode, Node, TagShape, TokenKind};

/// Sources of `cfformat`'s golden fixtures (`tests/fixtures/*/source.cfc`),
/// the first line dropped when it is the `//` script marker.
fn golden_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cfformat/tests/fixtures");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path().join("source.cfc");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let source = match raw.strip_prefix("//") {
            Some(rest) => rest.split_once('\n').map_or("", |(_, r)| r).to_string(),
            None => raw,
        };
        out.push((path.display().to_string(), source));
    }
    out.sort();
    out
}

#[test]
fn script_parser_never_declines_script() {
    let mut sources: Vec<(String, String)> = common::fixtures()
        .into_iter()
        .map(|f| (f.name, f.source))
        .collect();
    sources.extend(golden_sources());
    assert!(sources.len() > 250, "{} sources", sources.len());
    let mut declined = Vec::new();
    for (name, source) in &sources {
        let text = cfparse::normalize::normalized_text(source);
        if let Err(e) = script::parse(&text, Mode::Script) {
            declined.push(format!("{name}: {e}"));
        }
    }
    assert!(declined.is_empty(), "{}", declined.join("\n"));
}

/// A `//` comment in a tag's expression runs to the end of the line, past a
/// `>`: the tag ends at the first `>` after the comment. The printer
/// keeps the comment in the tag (`cfformat`'s fixture
/// `tagExpressionLineComment`).
#[test]
fn a_line_comment_in_a_tag_expression_runs_past_a_gt() {
    let src = "<cfset y = 1 // runs past > to the end\n>\n<cfset z = 2>";
    let tree = parse_source(src, Mode::Tags);
    let tags: Vec<_> = tree
        .root
        .children
        .iter()
        .filter_map(Node::as_element)
        .filter(|e| matches!(e.kind, ElementKind::CfTag(TagShape::Open, _)))
        .collect();
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0].span, 0..40);
    assert!(tags[0]
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::LineComment)));
}

/// The symbol operators, each of which must stay one token wherever a
/// recovered region ends before it.
const SYMBOL_OPERATORS: &[&str] = &[
    "&&", "||", "%=", "&=", "*=", "+=", "-=", "/=", "===", "!==", "==", "!=", "<>", "<=", ">=",
    "--", "++", "...", "=>", "?:", "?.", "::",
];

/// After a stray `)` each symbol operator is one token, never a shorter
/// operator and the rest: whichever rule reads it, the text after the region
/// must not print apart.
#[test]
fn an_operator_after_a_recovered_region_is_one_token() {
    for op in SYMBOL_OPERATORS {
        let src = format!(") {op} a;");
        let tree = parse_source(&src, Mode::Script);
        assert!(!tree.recoveries.is_empty(), "{op}");
        let at_op: Vec<&str> = tree
            .root
            .tokens()
            .iter()
            .filter(|t| t.span.start == 2)
            .map(|t| tree.text(t))
            .collect();
        assert_eq!(at_op, [*op], "{op}");
    }
}

/// An arrow function with a pattern parameter is a recovered region that
/// ends before `=>`; the arrow is one token, and the statement after it is
/// the body.
#[test]
fn a_recovered_arrow_keeps_its_arrow_whole() {
    let src = "g = ({a,b}) => a+b;";
    let tree = parse_source(src, Mode::Script);
    let tokens = tree.root.tokens();
    let texts: Vec<&str> = tokens.iter().map(|t| tree.text(t)).collect();
    assert_eq!(texts.iter().filter(|t| **t == "=>").count(), 1, "{texts:?}");
    assert!(!texts.contains(&">"), "{texts:?}");
    let last = tree
        .root
        .children
        .iter()
        .filter_map(Node::as_element)
        .rfind(|e| e.kind.is_statement())
        .unwrap();
    let first = last
        .tokens()
        .into_iter()
        .find(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline))
        .unwrap();
    assert_eq!(tree.text(&first), "a");
}
