//! Golden fixtures ported from CommandBox cfformat, and cfformat's own.
//!
//! Every fixture under `tests/fixtures` is formatted once per settings case
//! and compared with its expectation; a mismatch is reported as a unified
//! diff. `UPDATE_GOLDENS=1` rewrites the expectations of every fixture that
//! differs; add `GOLDENS=name,name` to limit a run (or an update) to some
//! fixtures.

mod common;

use cfformat::format_source;
use cfparse::{ElementKind, Mode};
use similar::TextDiff;

fn selected(name: &str) -> bool {
    match std::env::var("GOLDENS") {
        Ok(list) if !list.is_empty() => list.split(',').any(|n| n == name),
        _ => true,
    }
}

#[test]
fn goldens() {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some();
    let mut failures = Vec::new();
    let (mut passed, mut cases) = (0, 0);
    for fixture in common::fixtures() {
        if !selected(&fixture.name) {
            continue;
        }
        let mut report = String::new();
        let mut outputs = Vec::new();
        for (i, case) in fixture.cases.iter().enumerate() {
            cases += 1;
            let actual = common::format_case(&fixture, &fixture.source, &case.options);
            if actual != case.expected {
                let diff = TextDiff::from_lines(&case.expected, &actual);
                report.push_str(&format!(
                    "case {i}:\n{}",
                    diff.unified_diff()
                        .context_radius(3)
                        .header("expected", "actual")
                ));
            }
            outputs.push(actual);
        }
        if report.is_empty() {
            passed += 1;
        } else if update {
            std::fs::write(
                fixture.dir.join("formatted.txt"),
                common::join_expectations(&outputs),
            )
            .unwrap();
            eprintln!("updated {}", fixture.name);
            passed += 1;
        } else {
            failures.push(format!("{}:\n{report}", fixture.name));
        }
    }
    eprintln!("goldens: {passed} passing, {cases} cases");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The `exprTests` sources have no expectations: formatting must be
/// idempotent and keep the number of top-level statements.
#[test]
fn expr_tests() {
    let opts = cfformat::Options::default();
    let mut failures = Vec::new();
    for (name, source) in common::expr_tests() {
        if let Some(p) = check_expr_test(&source, &opts) {
            failures.push(format!("{name}: {p}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn check_expr_test(source: &str, opts: &cfformat::Options) -> Option<String> {
    let statements = |src: &str| {
        cfparse::parse_source(src, Mode::Script)
            .root
            .children
            .iter()
            .filter(|n| {
                n.as_element()
                    .is_some_and(|e| matches!(e.kind, ElementKind::Statement(_)))
            })
            .count()
    };
    let once = format_source(source, Mode::Script, opts);
    let twice = format_source(&once, Mode::Script, opts);
    if twice != once {
        return Some(format!("not idempotent:\n{once}\n---\n{twice:?}"));
    }
    if statements(source) != statements(&once) {
        return Some(format!("statement count changed:\n{once}"));
    }
    None
}
