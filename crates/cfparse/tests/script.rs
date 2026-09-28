//! The script front end parses every fixture — script and tag fixtures alike,
//! as `Mode::Script` — without declining: `Declined` means
//! "not script" and nothing else.

mod common;

use std::path::Path;

use cfparse::{parse_source, script, ElementKind, Mode, Node, TagShape};

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
