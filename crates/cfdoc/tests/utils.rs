//! `utils` against the behaviour of `prettier/src/document/utilities/index.js`.

use cfdoc::builders::*;
use cfdoc::utils::*;
use cfdoc::{Doc, FlatWidth, LineKind};

fn broke(doc: &Doc) -> bool {
    matches!(doc, Doc::Group(g) if g.should_break)
}

fn group_parts(doc: &Doc) -> &cfdoc::Group {
    match doc {
        Doc::Group(g) => g,
        other => panic!("not a group: {other:?}"),
    }
}

#[test]
fn propagate_breaks_through_contents() {
    // group([ "a", group([ "b", group(["c", hardline]) ]) ])
    let mut doc = group(vec![
        "a".into(),
        group(vec!["b".into(), group(vec!["c".into(), hardline()])]),
    ]);
    assert!(propagate_breaks(&mut doc));
    let outer = group_parts(&doc);
    assert!(outer.should_break && outer.break_propagated);
    let Doc::Concat(parts) = &outer.contents else {
        unreachable!()
    };
    assert!(broke(&parts[1]));
}

#[test]
fn propagate_breaks_line_without_break_parent_does_not_break() {
    let mut doc = group(vec!["a".into(), hardline_without_break_parent()]);
    assert!(!propagate_breaks(&mut doc));
    assert!(!broke(&doc));
}

#[test]
fn propagate_breaks_stops_at_conditional_group() {
    // group(conditionalGroup([ [ "x", breakParent ], group(["y", hardline]) ]))
    let mut doc = group(conditional_group(vec![
        vec!["x".into(), break_parent()].into(),
        group(vec!["y".into(), hardline()]),
    ]));
    assert!(!propagate_breaks(&mut doc));
    let outer = group_parts(&doc);
    assert!(!outer.should_break);
    let cond = group_parts(&outer.contents);
    assert!(!cond.should_break);
    // The states themselves are still walked.
    assert!(broke(&cond.expanded_states.as_ref().unwrap()[1]));

    // An explicitly broken conditional group does break its parent.
    let mut doc = group(conditional_group_opts(
        vec!["x".into()],
        GroupOpts {
            should_break: true,
            ..Default::default()
        },
    ));
    assert!(propagate_breaks(&mut doc));
    assert!(broke(&doc));
}

#[test]
fn propagate_breaks_keeps_contents_copy_in_step() {
    let mut doc = conditional_group(vec![group(vec!["a".into(), hardline()]), "b".into()]);
    propagate_breaks(&mut doc);
    let cond = group_parts(&doc);
    assert_eq!(&cond.contents, &cond.expanded_states.as_ref().unwrap()[0]);
    assert!(broke(&cond.contents));
}

#[test]
fn will_break_and_can_break() {
    let cases: Vec<(Doc, bool, bool)> = vec![
        ("a".into(), false, false),
        (softline(), false, true),
        (group(line()), false, true),
        (hardline(), true, true),
        (hardline_without_break_parent(), true, true),
        (literalline_without_break_parent(), true, true),
        (break_parent(), true, false),
        (
            group_opts(
                "a",
                GroupOpts {
                    should_break: true,
                    ..Default::default()
                },
            ),
            true,
            false,
        ),
        (indent(if_break(hardline(), "")), true, true),
        // Conditional groups are only looked into through `contents`.
        (
            conditional_group(vec!["a".into(), hardline()]),
            false,
            false,
        ),
    ];
    for (doc, will, can) in cases {
        assert_eq!((will_break(&doc), can_break(&doc)), (will, can), "{doc:?}");
    }
}

#[test]
fn clean_doc_cases() {
    let cases: Vec<(Doc, Doc)> = vec![
        // Flatten nested concats, drop "", merge adjacent strings.
        (
            vec![
                "a".into(),
                vec!["b".into(), "".into(), "c".into()].into(),
                line(),
                "d".into(),
            ]
            .into(),
            vec!["abc".into(), line(), "d".into()].into(),
        ),
        (vec!["".into(), "".into()].into(), "".into()),
        (vec![vec![line()].into()].into(), line()),
        (vec![].into(), "".into()),
        // Empty containers.
        (indent(""), "".into()),
        (align(2, vec!["".into()]), "".into()),
        (line_suffix(""), "".into()),
        (indent_if_break("", 0, false), "".into()),
        (if_break("", vec![]), "".into()),
        (if_break("", "a"), if_break("", "a")),
        (group(""), "".into()),
        (
            group_opts(
                "",
                GroupOpts {
                    id: Some(1),
                    ..Default::default()
                },
            ),
            group_opts(
                "",
                GroupOpts {
                    id: Some(1),
                    ..Default::default()
                },
            ),
        ),
        // Fill.
        (fill(vec!["".into(), "".into()]), "".into()),
        (fill(vec![group("a")]), group("a")),
        (
            fill(vec!["a".into(), line(), "b".into()]),
            fill(vec!["a".into(), line(), "b".into()]),
        ),
        // Nested only group.
        (
            group(group(vec!["a".into(), line()])),
            group(vec!["a".into(), line()]),
        ),
        (
            group(group_opts(
                "a",
                GroupOpts {
                    id: Some(3),
                    ..Default::default()
                },
            )),
            group(group_opts(
                "a",
                GroupOpts {
                    id: Some(3),
                    ..Default::default()
                },
            )),
        ),
        // Conditional-group states are cleaned and `contents` follows.
        (
            conditional_group(vec![vec!["a".into(), "b".into()].into(), indent("")]),
            conditional_group(vec!["ab".into(), "".into()]),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(clean_doc(input.clone()), expected, "{input:?}");
    }
}

#[test]
fn strip_trailing_hardline_cases() {
    let cases: Vec<(Doc, Doc)> = vec![
        (vec!["a".into(), hardline()].into(), vec!["a".into()].into()),
        (
            vec!["a".into(), hardline(), hardline()].into(),
            vec!["a".into()].into(),
        ),
        (
            vec!["a".into(), literalline()].into(),
            vec!["a".into()].into(),
        ),
        (vec!["a\n\r\n".into()].into(), "a".into()),
        (
            group(indent(vec!["a".into(), hardline()])),
            group(indent(vec!["a".into()])),
        ),
        (
            vec!["a".into(), indent(vec!["b".into(), hardline()])].into(),
            vec!["a".into(), indent(vec!["b".into()])].into(),
        ),
        // A line without a break parent is not stripped; align is left alone.
        (
            vec!["a".into(), hardline_without_break_parent()].into(),
            vec!["a".into(), hardline_without_break_parent()].into(),
        ),
        (
            align(2, vec!["a".into(), hardline()]),
            align(
                2,
                vec!["a".into(), Doc::Line(LineKind::Hard), break_parent()],
            ),
        ),
        (
            if_break(vec!["a".into(), hardline()], vec!["b\n".into()]),
            if_break(vec!["a".into()], "b"),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            strip_trailing_hardline(input.clone()),
            expected,
            "{input:?}"
        );
    }
}

#[test]
fn replace_end_of_line_cases() {
    let doc = group(vec!["x".into(), "a\nb\n".into()]);
    assert_eq!(
        replace_end_of_line(doc, literalline()),
        group(vec![
            "x".into(),
            vec![
                "a".into(),
                literalline(),
                "b".into(),
                literalline(),
                "".into()
            ]
            .into(),
        ])
    );
    assert_eq!(replace_end_of_line("a".into(), hardline()), "a".into());
}

#[test]
fn remove_lines_cases() {
    let doc = group(vec![
        "a".into(),
        line(),
        "b".into(),
        softline(),
        if_break(",", "!"),
        hardline(),
    ]);
    assert_eq!(
        remove_lines(doc),
        group(vec![
            "a".into(),
            " ".into(),
            "b".into(),
            "".into(),
            "!".into(),
            vec![Doc::Line(LineKind::Hard), break_parent()].into(),
        ])
    );
}

#[test]
fn map_keeps_a_conditional_group_contents_body() {
    // `conditional_group_contents` stores its one state in `contents` with an
    // empty `expanded_states`; mapping must not read `contents` back out of
    // those states (which would clear it).
    let doc = conditional_group_contents(vec!["a".into(), line(), "b".into()]);
    let mapped = remove_lines(doc);
    assert_eq!(
        group_parts(&mapped).contents,
        Doc::from(vec!["a".into(), Doc::from(" "), "b".into()])
    );
    assert_eq!(group_parts(&mapped).expanded_states, Some(Vec::new()));
    // A real conditional group still copies its first mapped state.
    let doc = conditional_group(vec![vec!["a".into(), line()].into(), "bb".into()]);
    let mapped = remove_lines(doc);
    let g = group_parts(&mapped);
    assert_eq!(g.contents, g.expanded_states.as_ref().unwrap()[0]);
    assert_eq!(g.contents, Doc::from(vec!["a".into(), Doc::from(" ")]));
}

#[test]
fn is_empty_cases() {
    let cases: Vec<(Doc, bool)> = vec![
        ("".into(), true),
        (vec!["".into(), group(indent(""))].into(), true),
        (if_break("", ""), true),
        ("a".into(), false),
        (group(softline()), false),
        (break_parent(), false),
        (line_suffix_boundary(), false),
    ];
    for (doc, empty) in cases {
        assert_eq!(doc.is_empty(), empty, "{doc:?}");
    }
}

#[test]
fn flat_widths() {
    use FlatWidth::{Finite, Infinite};
    let broken = || {
        group_opts(
            vec!["a".into(), line(), "b".into()],
            GroupOpts {
                should_break: true,
                ..Default::default()
            },
        )
    };
    let cases: Vec<(Doc, FlatWidth)> = vec![
        ("".into(), Finite(0)),
        ("abc".into(), Finite(3)),
        ("日本".into(), Finite(4)),
        ("a\nb".into(), Infinite),
        (softline(), Finite(0)),
        (line(), Finite(1)),
        (hardline(), Infinite),
        (hardline_without_break_parent(), Infinite),
        (literalline_without_break_parent(), Infinite),
        (vec!["a".into(), hardline(), "b".into()].into(), Infinite),
        (fill(vec!["a".into(), literalline()]), Infinite),
        // A break parent or a broken group is measured all the same; only a
        // line that is always a newline cannot be.
        (break_parent(), Finite(0)),
        (broken(), Finite(3)),
        (line_suffix("// long comment"), Finite(0)),
        (line_suffix(hardline()), Finite(0)),
        (line_suffix_boundary(), Finite(0)),
        (if_break("broken", "f"), Finite(1)),
        (if_break_group(hardline(), "", 3), Finite(0)),
        (if_break("", hardline()), Infinite),
        (indent(vec!["ab".into(), line(), "c".into()]), Finite(4)),
        (align(4, "ab"), Finite(2)),
        (align("> ", "ab"), Finite(2)),
        (dedent_to_root("ab"), Finite(2)),
        (indent_if_break("abc", 0, false), Finite(3)),
        (fill(vec!["a".into(), line(), "b".into()]), Finite(3)),
        // A conditional group is its first state.
        (
            conditional_group(vec!["abcd".into(), hardline()]),
            Finite(4),
        ),
        (conditional_group(vec![hardline(), "abcd".into()]), Infinite),
        (
            conditional_group_contents(vec!["ab".into(), line()]),
            Finite(3),
        ),
        (
            group(vec!["a".into(), group(vec![line(), "b".into()])]),
            Finite(3),
        ),
    ];
    for (doc, expected) in cases {
        assert_eq!(flat_width(&doc), expected, "{doc:?}");
    }
    assert!(Finite(usize::MAX) < Infinite);
}
