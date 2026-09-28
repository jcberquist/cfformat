//! Printer behaviour, case by case.
//!
//! With `CFDOC_PRETTIER_CHECK=1` (and the `npm install` from
//! `tests/parity.rs`), every expected string is also checked against
//! Prettier's `printDocToString`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use cfdoc::builders::*;
use cfdoc::debug::to_prettier_json;
use cfdoc::{print_doc, Doc, IndentStyle, PrintOptions};

const SP2: IndentStyle = IndentStyle::Spaces(2);

struct Case {
    name: &'static str,
    doc: Doc,
    width: usize,
    indent: IndentStyle,
    newline: &'static str,
    expected: &'static str,
}

fn case(name: &'static str, doc: Doc, width: usize, expected: &'static str) -> Case {
    Case {
        name,
        doc,
        width,
        indent: SP2,
        newline: "\n",
        expected,
    }
}

fn with_indent(mut c: Case, indent: IndentStyle) -> Case {
    c.indent = indent;
    c
}

fn with_crlf(mut c: Case) -> Case {
    c.newline = "\r\n";
    c
}

fn run(cases: Vec<Case>) {
    if std::env::var_os("CFDOC_PRETTIER_CHECK").is_some() {
        check_with_prettier(&cases);
    }
    let mut failures = Vec::new();
    for c in cases {
        let opts = PrintOptions {
            width: c.width,
            indent: c.indent,
            newline: c.newline,
        };
        let actual = print_doc(&mut c.doc.clone(), &opts);
        if actual != c.expected {
            failures.push(format!(
                "{}:\n  expected {:?}\n  actual   {:?}",
                c.name, c.expected, actual
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn check_with_prettier(cases: &[Case]) {
    let script_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/doc-parity");
    assert!(
        script_dir.join("node_modules/prettier").is_dir(),
        "run `npm install` in {}",
        script_dir.display()
    );
    let mut input = Vec::new();
    for c in cases {
        let (use_tabs, tab_width) = match c.indent {
            IndentStyle::Tabs(n) => (true, n),
            IndentStyle::Spaces(n) => (false, n),
        };
        input.push(format!(
            r#"{{"doc":{},"printWidth":{},"tabWidth":{tab_width},"useTabs":{use_tabs},"endOfLine":"{}"}}"#,
            to_prettier_json(&c.doc),
            c.width,
            if c.newline == "\r\n" { "crlf" } else { "lf" },
        ));
    }
    let mut child = Command::new("node")
        .arg(script_dir.join("print.mjs"))
        .current_dir(&script_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn node");
    let json = format!("[{}]", input.join(","));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(json.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "print.mjs failed");
    let prettier: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    for (c, theirs) in cases.iter().zip(prettier) {
        assert_eq!(
            c.expected, theirs,
            "{}: expectation differs from Prettier",
            c.name
        );
    }
}

fn t(s: &'static str) -> Doc {
    Doc::from(s)
}

fn ab_group(a: &'static str, b: &'static str) -> Doc {
    group(vec![t(a), line(), t(b)])
}

fn bracket_list(items: &[&'static str]) -> Doc {
    vec![
        t("["),
        indent(vec![
            softline(),
            join(vec![t(","), line()], items.iter().map(|&s| t(s))),
        ]),
        softline(),
        t("]"),
    ]
    .into()
}

#[test]
fn groups_and_fits() {
    run(vec![
        case(
            "flat",
            group(bracket_list(&["1", "2", "3"])),
            9,
            "[1, 2, 3]",
        ),
        case(
            "broken",
            group(bracket_list(&["1", "2", "3"])),
            8,
            "[\n  1,\n  2,\n  3\n]",
        ),
        // `fits` keeps measuring the rest commands up to the next line break.
        case(
            "rest counted",
            vec![ab_group("a", "b"), t("cccc")].into(),
            6,
            "a\nbcccc",
        ),
        case(
            "rest counted fits",
            vec![ab_group("a", "b"), t("ccc")].into(),
            6,
            "a bccc",
        ),
        case(
            "rest stops at break-mode line",
            vec![ab_group("a", "b"), line(), t("cccc")].into(),
            6,
            "a b\ncccc",
        ),
        // A pending space from a flat `line` only counts when text follows.
        case(
            "trailing flat line",
            group(vec![t("aaaaaa"), line()]),
            6,
            "aaaaaa ",
        ),
        case(
            "should_break",
            group_opts(
                vec![t("a"), line(), t("b")],
                GroupOpts {
                    should_break: true,
                    ..Default::default()
                },
            ),
            80,
            "a\nb",
        ),
        case(
            "hardline breaks parents",
            group(vec![
                t("a"),
                line(),
                group(vec![t("b"), hardline(), t("c")]),
            ]),
            80,
            "a\nb\nc",
        ),
        // A hard line without a break parent does not break the group, but the
        // group after it is remeasured (`shouldRemeasure`).
        case(
            "remeasure after forced line",
            group(vec![
                t("x"),
                hardline_without_break_parent(),
                ab_group("aaaa", "bbbb"),
            ]),
            8,
            "x\naaaa\nbbbb",
        ),
        case(
            "no remeasure needed",
            group(vec![
                t("x"),
                hardline_without_break_parent(),
                ab_group("aaa", "bbb"),
            ]),
            8,
            "x\naaa bbb",
        ),
        case(
            "negative remaining width",
            vec![t("0123456789"), ab_group("a", "b")].into(),
            5,
            "0123456789a\nb",
        ),
    ]);
}

#[test]
fn fill_layouts() {
    let words = || fill(vec![t("aaa"), line(), t("bbb"), line(), t("ccc")]);
    run(vec![
        case("content fits", words(), 80, "aaa bbb ccc"),
        case("separator breaks", words(), 7, "aaa bbb\nccc"),
        case("separator breaks every pair", words(), 6, "aaa\nbbb\nccc"),
        case(
            "content breaks",
            fill(vec![ab_group("aaaa", "bbbb"), line(), t("cc")]),
            6,
            "aaaa\nbbbb\ncc",
        ),
        case(
            "single part",
            fill(vec![ab_group("aaaa", "bbbb")]),
            6,
            "aaaa\nbbbb",
        ),
        case("two parts", fill(vec![t("aaaa"), line()]), 6, "aaaa "),
        case("empty", fill(vec![]), 6, ""),
        // `must_be_flat`: a broken group never fits as fill content.
        case(
            "must be flat",
            fill(vec![
                t("xx"),
                line(),
                conditional_group_opts(
                    vec![t("aaaaaaaaaa"), vec![t("b"), line(), t("c")].into()],
                    GroupOpts {
                        should_break: true,
                        ..Default::default()
                    },
                ),
            ]),
            80,
            "xx\nb\nc",
        ),
    ]);
}

#[test]
fn if_break_and_indent_if_break() {
    let id_group = |a, b| {
        group_opts(
            vec![t(a), line(), t(b)],
            GroupOpts {
                id: Some(0),
                ..Default::default()
            },
        )
    };
    run(vec![
        case(
            "if_break flat",
            group(vec![t("a"), line(), t("b"), if_break(",", ";")]),
            80,
            "a b;",
        ),
        case(
            "if_break break",
            group(vec![t("a"), line(), t("b"), if_break(",", ";")]),
            3,
            "a\nb,",
        ),
        case(
            "foreign id flat",
            vec![id_group("aaaa", "bbbb"), if_break_group("B", "F", 0)].into(),
            80,
            "aaaa bbbbF",
        ),
        case(
            "foreign id break",
            vec![id_group("aaaa", "bbbb"), if_break_group("B", "F", 0)].into(),
            9,
            "aaaa\nbbbbB",
        ),
        // Inside a flat group, the foreign group's mode still decides.
        case(
            "foreign id inside other group",
            vec![
                id_group("aaaa", "bbbb"),
                group(vec![t("x"), line(), if_break_group("B", "F", 0)]),
            ]
            .into(),
            9,
            "aaaa\nbbbbx B",
        ),
        // Unknown id: nothing is printed (Prettier's map lookup is undefined).
        case(
            "unprinted id",
            vec![if_break_group("B", "F", 5), t("x")].into(),
            80,
            "x",
        ),
        case(
            "indent_if_break flat",
            vec![
                id_group("a", "b"),
                indent_if_break(vec![hardline(), t("x")], 0, false),
            ]
            .into(),
            80,
            "a b\nx",
        ),
        case(
            "indent_if_break break",
            vec![
                id_group("a", "b"),
                indent_if_break(vec![hardline(), t("x")], 0, false),
            ]
            .into(),
            2,
            "a\nb\n  x",
        ),
        case(
            "indent_if_break negate flat",
            vec![
                id_group("a", "b"),
                indent_if_break(vec![hardline(), t("x")], 0, true),
            ]
            .into(),
            80,
            "a b\n  x",
        ),
        case(
            "indent_if_break negate break",
            vec![
                id_group("a", "b"),
                indent_if_break(vec![hardline(), t("x")], 0, true),
            ]
            .into(),
            2,
            "a\nb\nx",
        ),
    ]);
}

#[test]
fn line_suffixes() {
    run(vec![
        case(
            "flush at hard line",
            vec![t("a"), line_suffix(" // c"), t(";"), hardline(), t("b")].into(),
            80,
            "a; // c\nb",
        ),
        case(
            "flush at end",
            vec![t("a"), line_suffix(" // c"), t(";")].into(),
            80,
            "a; // c",
        ),
        case(
            "flush order",
            vec![
                t("a"),
                line_suffix(" /1"),
                t("b"),
                line_suffix(" /2"),
                hardline(),
                t("c"),
            ]
            .into(),
            80,
            "ab /1 /2\nc",
        ),
        case(
            "flush at broken line",
            group(vec![
                t("a"),
                line_suffix(" // c"),
                line(),
                t("b"),
                break_parent(),
            ]),
            80,
            "a // c\nb",
        ),
        case(
            "boundary flushes",
            vec![t("a"), line_suffix(" // c"), line_suffix_boundary(), t("b")].into(),
            80,
            "a // c\nb",
        ),
        case(
            "boundary without suffix",
            vec![t("a"), line_suffix_boundary(), t("b")].into(),
            80,
            "ab",
        ),
        // In `fits`, a boundary after a pending suffix never fits.
        case(
            "boundary breaks group",
            group(vec![
                t("("),
                indent(vec![
                    softline(),
                    t("a"),
                    line_suffix(" // c"),
                    line_suffix_boundary(),
                ]),
                softline(),
                t(")"),
            ]),
            80,
            "(\n  a // c\n\n)",
        ),
    ]);
}

#[test]
fn indentation() {
    let tabs = IndentStyle::Tabs(4);
    run(vec![
        case(
            "indent",
            indent(vec![t("a"), hardline(), t("b")]),
            80,
            "a\n  b",
        ),
        case(
            "align width",
            align(3, vec![t("a"), hardline(), t("b")]),
            80,
            "a\n   b",
        ),
        case(
            "align string",
            align("> ", vec![t("a"), hardline(), t("b")]),
            80,
            "a\n> b",
        ),
        case(
            "align zero",
            indent(align(0, vec![t("a"), hardline(), t("b")])),
            80,
            "a\n  b",
        ),
        case(
            "dedent",
            indent(indent(dedent(vec![t("a"), hardline(), t("b")]))),
            80,
            "a\n  b",
        ),
        with_indent(
            case(
                "tabs",
                indent(indent(vec![t("a"), hardline(), t("b")])),
                80,
                "a\n\t\tb",
            ),
            tabs,
        ),
        // useTabs: an align followed by an indent becomes a whole tab…
        with_indent(
            case(
                "tabs align then indent",
                indent(align(2, indent(vec![t("a"), hardline(), t("b")]))),
                80,
                "a\n\t\t\tb",
            ),
            tabs,
        ),
        with_indent(
            case(
                "spaces align then indent",
                indent(align(2, indent(vec![t("a"), hardline(), t("b")]))),
                80,
                "a\n          b",
            ),
            IndentStyle::Spaces(4),
        ),
        // …while a trailing align stays spaces.
        with_indent(
            case(
                "tabs trailing align",
                indent(align(2, vec![t("a"), hardline(), t("b")])),
                80,
                "a\n\t  b",
            ),
            tabs,
        ),
        // A tab counts as tab_width columns when measuring.
        with_indent(
            case(
                "tab width measured",
                indent(vec![hardline(), ab_group("aa", "bb")]),
                8,
                "\n\taa\n\tbb",
            ),
            tabs,
        ),
        with_indent(
            case(
                "tab width measured fits",
                indent(vec![hardline(), ab_group("aa", "b")]),
                8,
                "\n\taa b",
            ),
            tabs,
        ),
        case(
            "dedent_to_root",
            indent(vec![
                t("a"),
                indent(vec![t("b"), dedent_to_root(vec![hardline(), t("c")])]),
            ]),
            80,
            "ab\nc",
        ),
        case(
            "dedent_to_root to marked root",
            indent(mark_as_root(indent(vec![
                t("a"),
                dedent_to_root(vec![hardline(), t("b")]),
            ]))),
            80,
            "a\n  b",
        ),
        case(
            "literalline ignores indent",
            indent(vec![t("a"), literalline(), t("b")]),
            80,
            "a\nb",
        ),
        case(
            "literalline under mark_as_root",
            indent(mark_as_root(indent(vec![
                t("a"),
                literalline(),
                t("b"),
                hardline(),
                t("c"),
            ]))),
            80,
            "a\n  b\n    c",
        ),
        case(
            "add_alignment_to_doc",
            indent(vec![
                t("a"),
                add_alignment_to_doc(vec![hardline(), t("b")], 5, 2),
            ]),
            80,
            "a\n     b",
        ),
    ]);
}

#[test]
fn conditional_groups() {
    let states = || {
        conditional_group(vec![
            t("long-long-long"),
            t("mid-mid"),
            vec![t("s"), line(), t("t")].into(),
        ])
    };
    run(vec![
        case("first state fits", states(), 80, "long-long-long"),
        case("middle state fits", states(), 10, "mid-mid"),
        case("last state in break mode", states(), 5, "s\nt"),
        case(
            "broken skips to last",
            conditional_group_opts(
                vec![t("a"), t("b"), vec![t("c"), line(), t("d")].into()],
                GroupOpts {
                    should_break: true,
                    ..Default::default()
                },
            ),
            80,
            "c\nd",
        ),
        // Breaks inside the states do not propagate out of the conditional group.
        case(
            "no propagation out",
            group(vec![
                t("x"),
                line(),
                conditional_group(vec![vec![t("y"), break_parent()].into()]),
            ]),
            80,
            "x y",
        ),
        // `conditional_group_contents`: one state, stored once.
        case(
            "contents state flat",
            conditional_group_contents(vec![t("a"), line(), t("b")]),
            80,
            "a b",
        ),
        case(
            "contents state broken",
            conditional_group_contents(vec![t("aa"), line(), t("b")]),
            2,
            "aa\nb",
        ),
        case(
            "contents state fits up to its hard line",
            group(vec![
                t("x"),
                line(),
                conditional_group_contents(vec![t("("), line(), t("y"), hardline(), t("z")]),
            ]),
            80,
            "x ( y\nz",
        ),
    ]);
}

#[test]
fn output_buffer() {
    run(vec![
        case(
            "trailing whitespace trimmed",
            vec![t("a  "), hardline(), t("b\t "), hardline(), t("c ")].into(),
            80,
            "a\nb\nc ",
        ),
        case(
            "blank line indentation trimmed",
            indent(vec![t("a"), hardline(), hardline(), t("b")]),
            80,
            "a\n\n  b",
        ),
        case(
            "literalline does not trim",
            vec![t("a "), literalline(), t("b")].into(),
            80,
            "a \nb",
        ),
        with_crlf(case("crlf lines", ab_group("a", "b"), 1, "a\r\nb")),
        with_crlf(case(
            "crlf text",
            vec![t("x\ny"), hardline(), t("z"), literalline(), t("w")].into(),
            80,
            "x\r\ny\r\nz\r\nw",
        )),
        with_crlf(case(
            "crlf trim",
            vec![t("a "), hardline(), t("b")].into(),
            80,
            "a\r\nb",
        )),
        case(
            "wide text measured",
            ab_group("日本語", "x"),
            7,
            "日本語\nx",
        ),
        case("wide text fits", ab_group("日本語", "x"), 8, "日本語 x"),
    ]);
}
