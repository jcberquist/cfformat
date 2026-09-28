//! `debug::format_doc` notation (Prettier's `printDocToDebug`) and the
//! Prettier JSON shape.

use cfdoc::builders::*;
use cfdoc::debug::{format_doc, to_prettier_json};
use cfdoc::utils::propagate_breaks;
use cfdoc::{Align, Doc};

fn representative() -> Doc {
    group_opts(
        vec![
            "call(".into(),
            indent(vec![
                softline(),
                join(
                    vec![",".into(), line()],
                    vec![
                        "\"quoted\"\tx".into(),
                        group(vec!["[".into(), softline(), "1".into(), "]".into()]),
                        conditional_group(vec!["a".into(), vec!["b".into(), hardline()].into()]),
                        fill(vec!["w1".into(), line(), "w2".into()]),
                    ],
                ),
            ]),
            if_break(",", ""),
            if_break_group(";", "", 7),
            indent_if_break("x", 7, true),
            softline(),
            ")".into(),
            line_suffix(" // c"),
            line_suffix_boundary(),
            vec!["".into(), align(2, "a"), align("> ", "b"), dedent("c")].into(),
            dedent_to_root(mark_as_root(literalline())),
            hardline_without_break_parent(),
            break_parent(),
            group_opts(
                "z",
                GroupOpts {
                    should_break: true,
                    ..Default::default()
                },
            ),
        ],
        GroupOpts {
            id: Some(7),
            ..Default::default()
        },
    )
}

#[test]
fn format_doc_matches_prettier_notation() {
    let mut doc = representative();
    // Checked against prettier's `debug.js`. Propagated breaks are not
    // printed as `shouldBreak: true`, and a hardline without break parent
    // followed by a break parent reads as `hardline` once flattened.
    propagate_breaks(&mut doc);
    let expected = concat!(
        r#"group(["call(", indent([softline, "\"quoted\"\tx", ",", line, "#,
        r#"group(["[", softline, "1", "]"]), ",", line, "#,
        r#"conditionalGroup(["a",["b", hardline]]), ",", line, "#,
        r#"fill(["w1", line, "w2"])]), ifBreak(","), ifBreak(";", "", { groupId: "7" }), "#,
        r#"indentIfBreak("x", { negate: true, groupId: "7" }), softline, ")", "#,
        r#"lineSuffix(" // c"), lineSuffixBoundary, align(2, "a"), align("> ", "b"), "#,
        r#"dedent("c"), dedentToRoot(markAsRoot(literalline)), hardline, "#,
        r#"group("z", { shouldBreak: true })], { id: "7" })"#,
    );
    assert_eq!(format_doc(&doc), expected);
}

#[test]
fn format_doc_edge_cases() {
    let cases: Vec<(Doc, &str)> = vec![
        ("".into(), r#""""#),
        (vec![].into(), "[]"),
        (vec!["".into()].into(), "[]"),
        (vec![vec!["a".into()].into()].into(), r#""a""#),
        (hardline(), "hardline"),
        (
            Doc::Line(cfdoc::LineKind::Hard),
            "hardlineWithoutBreakParent",
        ),
        (if_break("", vec![]), r#"ifBreak("", [])"#),
        (
            if_break_group("a", vec![], 1),
            r#"ifBreak("a", [], { groupId: "1" })"#,
        ),
        (
            indent_if_break("a", 0, false),
            r#"indentIfBreak("a", { groupId: "0" })"#,
        ),
        (align(Align::Width(0), "a"), r#"align(0, "a")"#),
        (
            fill(vec!["a".into(), hardline()]),
            r#"fill(["a", hardline])"#,
        ),
        ("\u{1}é".into(), r#""\u0001é""#),
    ];
    for (doc, expected) in cases {
        assert_eq!(format_doc(&doc), expected, "{doc:?}");
    }
}

#[test]
fn prettier_json_shape() {
    let doc = group_opts(
        vec![
            "a".into(),
            indent(softline()),
            align(Align::DedentToRoot, line()),
            if_break_group("x", "", 2),
            conditional_group(vec!["s".into()]),
        ],
        GroupOpts {
            id: Some(2),
            should_break: true,
        },
    );
    assert_eq!(
        to_prettier_json(&doc),
        concat!(
            r#"{"type":"group","contents":["a",{"type":"indent","contents":{"type":"line","soft":true}},"#,
            r#"{"type":"align","contents":{"type":"line"},"n":null},"#,
            r#"{"type":"if-break","breakContents":"x","flatContents":"","groupId":"g2"},"#,
            r#"{"type":"group","contents":"s","break":false,"expandedStates":["s"]}],"break":true,"id":"g2"}"#,
        )
    );
}
