//! The script front end parses every fixture — script and tag fixtures alike,
//! as `Mode::Script` — without declining: `Declined` means
//! "not script" and nothing else.

mod common;

use std::path::Path;

use cfparse::{
    parse_source, script, BlockKind, Element, ElementKind, Mode, Node, StatementKind, TagShape,
    TokenKind,
};

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

/// An arrow function whose parameters a stray `}` ends is a recovered region
/// that ends before `=>`; the arrow is one token, and the statement after it
/// is the body.
#[test]
fn a_recovered_arrow_keeps_its_arrow_whole() {
    let src = "g = (a}) => a+b;";
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

/// Every element under `el` (itself included) whose kind `pred` accepts,
/// outermost first.
fn elements_where<'t>(el: &'t Element, pred: &dyn Fn(&ElementKind) -> bool) -> Vec<&'t Element> {
    let mut out = Vec::new();
    collect_where(el, pred, &mut out);
    out
}

fn collect_where<'t>(
    el: &'t Element,
    pred: &dyn Fn(&ElementKind) -> bool,
    out: &mut Vec<&'t Element>,
) {
    if pred(&el.kind) {
        out.push(el);
    }
    for child in el.nodes().filter_map(Node::as_element) {
        collect_where(child, pred, out);
    }
}

fn is_pattern(kind: &ElementKind) -> bool {
    matches!(kind, ElementKind::Pattern { .. })
}

/// Each `Pattern` of `src`, parsed as script with no recovered region:
/// whether it is an array pattern, and how many items it has.
fn patterns(src: &str) -> Vec<(bool, usize)> {
    let tree = parse_source(src, Mode::Script);
    assert!(tree.recoveries.is_empty(), "{src}: {:?}", tree.recoveries);
    elements_where(&tree.root, &is_pattern)
        .iter()
        .map(|e| {
            (
                e.kind == ElementKind::Pattern { array: true },
                e.items.len(),
            )
        })
        .collect()
}

/// The first significant node of `nodes`, as an element.
fn first_element(nodes: &[Node]) -> &Element {
    nodes
        .iter()
        .find(|n| !n.is_trivia())
        .and_then(Node::as_element)
        .expect("an element")
}

#[test]
fn a_var_binding_is_a_pattern() {
    let src = "var [a, , c] = x;";
    assert_eq!(patterns(src), [(true, 3)]);
    let tree = parse_source(src, Mode::Script);
    let found = elements_where(&tree.root, &is_pattern);
    assert_eq!(found[0].items[1].significant().count(), 0);

    let src = "var {p, q: {r, s = 1}, ...t} = st;";
    assert_eq!(patterns(src), [(false, 3), (false, 2)]);
    let tree = parse_source(src, Mode::Script);
    let found = elements_where(&tree.root, &is_pattern);
    let default = first_element(&found[1].items[1].children);
    assert_eq!(default.kind, ElementKind::Assignment);
}

#[test]
fn a_pattern_at_a_statement_start_is_assigned_to() {
    let src = "[a, b] = [b, a];";
    assert_eq!(patterns(src), [(true, 2)]);
    let tree = parse_source(src, Mode::Script);
    let statement = first_element(&tree.root.children);
    assert_eq!(
        statement.kind,
        ElementKind::Statement(StatementKind::Assignment)
    );
    let assignment = first_element(&statement.children);
    assert_eq!(assignment.kind, ElementKind::Assignment);
    let parts: Vec<&ElementKind> = assignment
        .children
        .iter()
        .filter_map(Node::as_element)
        .map(|e| &e.kind)
        .collect();
    assert_eq!(
        parts,
        [&ElementKind::Pattern { array: true }, &ElementKind::Array]
    );
}

#[test]
fn a_pattern_in_a_group_is_assigned_to() {
    let src = "({a, b: c, d = 1} = st) && y;";
    assert_eq!(patterns(src), [(false, 3)]);
    let tree = parse_source(src, Mode::Script);
    let groups = elements_where(&tree.root, &|k| *k == ElementKind::Group);
    assert_eq!(groups.len(), 1);
    let assignment = first_element(&groups[0].children);
    assert_eq!(assignment.kind, ElementKind::Assignment);
}

#[test]
fn a_literal_not_assigned_to_is_no_pattern() {
    for src in [
        "[1, 2].each(f);",
        "[a][1] = x;",
        "x = [1, 2];",
        "if ([1, 2] == x) {}",
        "x = {a: 1};",
        "{ a = 1; }",
    ] {
        let tree = parse_source(src, Mode::Script);
        assert!(elements_where(&tree.root, &is_pattern).is_empty(), "{src}");
    }
}

#[test]
fn a_for_in_binding_is_a_pattern() {
    for (src, array) in [
        ("for ([k, v] in pairs) {}", true),
        ("for ({a, b} in s) {}", false),
        ("for (var [k, v] in pairs) {}", true),
    ] {
        assert_eq!(patterns(src), [(array, 2)], "{src}");
        let tree = parse_source(src, Mode::Script);
        let header = elements_where(&tree.root, &|k| *k == ElementKind::Group)[0];
        let in_item = header.items[0]
            .children
            .iter()
            .filter_map(Node::as_element)
            .flat_map(|e| elements_where(e, &is_pattern))
            .count();
        assert_eq!(in_item, 1, "{src}");
    }
}

#[test]
fn a_parameter_can_be_a_pattern() {
    // The source, its patterns, and how many are parameters of their own
    // (the rest are nested).
    for (src, expected, parameters) in [
        (
            "function f(x, {a, b = 9, p: {q}, ...r}, {c}) {}",
            &[(false, 4), (false, 1), (false, 1)][..],
            2,
        ),
        ("g = ({a, b}) => a + b;", &[(false, 2)][..], 1),
        ("h = function([a, b]) {};", &[(true, 2)][..], 1),
    ] {
        assert_eq!(patterns(src), expected, "{src}");
        let tree = parse_source(src, Mode::Script);
        let params = elements_where(&tree.root, &|k| *k == ElementKind::Parameters);
        assert_eq!(params.len(), 1, "{src}");
        let own = params[0]
            .items
            .iter()
            .flat_map(|i| i.children.iter())
            .filter_map(Node::as_element)
            .filter(|e| is_pattern(&e.kind))
            .count();
        assert_eq!(own, parameters, "{src}");
        let bodies = elements_where(&tree.root, &|k| {
            *k == ElementKind::Block(BlockKind::Function)
        });
        assert_eq!(bodies.len(), 1, "{src}");
    }
}

/// No engine runs `{a, b} = x;`, but it is read as the pattern assignment it
/// would be, not as a block followed by `= x;`; after `static`, not as a
/// static block.
#[test]
fn a_bare_struct_pattern_is_assigned_to() {
    for src in ["{a, b} = x;", "static {a, b} = x;"] {
        assert_eq!(patterns(src), [(false, 2)], "{src}");
        let tree = parse_source(src, Mode::Script);
        let blocks = elements_where(&tree.root, &|k| {
            matches!(k, ElementKind::Block(_) | ElementKind::StaticBlock)
        });
        assert!(blocks.is_empty(), "{src}");
    }
}

/// A parameter no rule reads is skipped an operator at a time: `?.` stays
/// one token.
#[test]
fn a_parameter_run_keeps_an_operator_whole() {
    let tree = parse_source("function f(?.x) {}", Mode::Script);
    let texts: Vec<&str> = tree.root.tokens().iter().map(|t| tree.text(t)).collect();
    assert!(texts.contains(&"?."), "{texts:?}");
}
