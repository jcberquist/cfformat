//! `cfformat::arrange`: the fixtures under `tests/arrange` and the
//! invariants.
//!
//! Each fixture directory holds `source.cfc`, `arranged.cfc` (the output
//! without `--properties`) and `arranged-properties.cfc` (with it), compared
//! byte for byte; the fixtures [`FIRST_CASES`] names hold one more expected
//! file per `--first` list. `UPDATE_ARRANGE=1` rewrites the expected files.
//!
//! The invariants run over every arrange fixture and every golden fixture
//! under `tests/fixtures`, each without and with `--properties` (and an
//! arrange fixture with its `--first` lists too):
//! idempotence, permutation, sortedness (`common/arrange.rs`), and
//! commutation with the formatter: `fmt(arrange(x)) == arrange(fmt(x))`
//! for every settings case of a golden fixture, and for the arrange
//! fixtures with the default options and with
//! `alignment.consecutive.properties`.

#[path = "common/arrange.rs"]
mod checks;
mod common;

use std::path::{Path, PathBuf};

use cfformat::arrange::{self, ArrangeOptions};
use cfformat::{Options, RecoveryReason, Skipped};
use cfparse::Mode;
use similar::TextDiff;

/// Commutation cases known to fail, with the reason: `fixture[case]` when
/// both runs fail, `fixture[case]+properties` when only the run with
/// `--properties` does. A listed case that passes fails the test, so a fix
/// is never silent.
const EXPECT_COMMUTE_FAIL: &[(&str, &str)] = &[
    ("alignAttributeRuns[0]+properties", ALIGNMENT),
    ("alignAttributeRuns[1]+properties", ALIGNMENT),
    ("alignAttributeRuns[2]+properties", ALIGNMENT),
    ("alignAttributeRuns[3]+properties", ALIGNMENT),
    ("alignAttributeRuns[4]+properties", ALIGNMENT),
    ("arrange/shared-line[0]", SHARED_LINE),
    ("arrange/shared-line[1]", SHARED_LINE),
];

const ALIGNMENT: &str = "the formatter aligns a property with its neighbours of the same attribute shape and leaves one holding a block comment unaligned, so its alignment runs are narrower than the arrange run and sorting changes the padding";

const SHARED_LINE: &str = "the formatter puts each of two members sharing a line on a line of its own, which makes them units that arrange then sorts";

struct Fixture {
    name: String,
    dir: PathBuf,
    source: String,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/arrange")
}

fn fixtures() -> Vec<Fixture> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("source.cfc").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| Fixture {
            name: dir.file_name().unwrap().to_string_lossy().into_owned(),
            source: std::fs::read_to_string(dir.join("source.cfc")).unwrap(),
            dir,
        })
        .collect()
}

/// The `--first` cases, beside every fixture's two with the default list
/// (`init`): the fixture, the list, and its expected file.
const FIRST_CASES: &[(&str, &[&str], &str)] = &[
    (
        "first",
        &["before", "init"],
        "arranged-first-before-init.cfc",
    ),
    ("first", &[], "arranged-first-none.cfc"),
];

/// The option sets a fixture runs with, and the expected file of each.
fn cases(fixture: &str) -> Vec<(ArrangeOptions, &'static str)> {
    let [plain, properties] = checks::both();
    let mut out = vec![
        (plain, "arranged.cfc"),
        (properties, "arranged-properties.cfc"),
    ];
    for (name, first, file) in FIRST_CASES {
        if *name == fixture {
            let first = first.iter().map(|n| n.to_string()).collect();
            out.push((
                ArrangeOptions {
                    first,
                    ..ArrangeOptions::default()
                },
                file,
            ));
        }
    }
    out
}

#[test]
fn every_first_case_names_a_fixture() {
    let names: Vec<String> = fixtures().into_iter().map(|f| f.name).collect();
    for (name, _, _) in FIRST_CASES {
        assert!(names.iter().any(|n| n == name), "no fixture {name}");
    }
}

#[test]
fn arrange_fixtures() {
    let update = std::env::var_os("UPDATE_ARRANGE").is_some();
    let mut failures = Vec::new();
    for fixture in fixtures() {
        for (opts, file) in cases(&fixture.name) {
            let out = arrange::arrange(&fixture.source, Mode::Auto, &opts);
            let path = fixture.dir.join(file);
            if update {
                std::fs::write(&path, &out.text).unwrap();
                continue;
            }
            let expected = std::fs::read_to_string(&path).unwrap();
            // Bytes, not lines: the CRLF, lone CR and BOM fixtures are
            // about exactly those bytes.
            if out.text.as_bytes() != expected.as_bytes() {
                failures.push(format!(
                    "{}{}:\n{}",
                    fixture.name,
                    checks::suffix(&opts),
                    TextDiff::from_lines(&expected, &out.text)
                        .unified_diff()
                        .header("expected", "arranged")
                ));
            }
            if out.changed != (expected != fixture.source) {
                failures.push(format!(
                    "{}{}: changed is {}",
                    fixture.name,
                    checks::suffix(&opts),
                    out.changed
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_body_the_parse_recovered_in_is_skipped() {
    let src = std::fs::read_to_string(fixtures_dir().join("recovery/source.cfc")).unwrap();
    for opts in checks::both() {
        let out = arrange::arrange(&src, Mode::Auto, &opts);
        assert!(!out.changed);
        assert_eq!(
            out.skipped,
            vec![Skipped {
                line: 1,
                reason: RecoveryReason::Unmatched
            }]
        );
    }
}

#[test]
fn an_arranged_body_is_unchanged() {
    let src = std::fs::read_to_string(fixtures_dir().join("already-arranged/source.cfc")).unwrap();
    for opts in checks::both() {
        let out = arrange::arrange(&src, Mode::Auto, &opts);
        assert!(!out.changed && out.text == src && out.skipped.is_empty());
    }
}

#[test]
fn nothing_outside_a_top_level_component_moves() {
    for src in [
        // A template.
        "<cfset b = 1>\n<cffunction name=\"b\"></cffunction>\n<cffunction name=\"a\"></cffunction>\n",
        // Nested functions.
        "component {\n    function f() {\n        function b() {}\n        function a() {}\n    }\n}\n",
        // A component inside `<cfscript>` in a template.
        "<cfscript>\ncomponent {\n    function b() {}\n    function a() {}\n}\n</cfscript>\n",
    ] {
        for opts in checks::both() {
            assert!(!arrange::arrange(src, Mode::Auto, &opts).changed, "{src}");
        }
    }
}

#[test]
fn invariants() {
    let mut failures = Vec::new();
    let mut sources: Vec<(String, String, Mode, Vec<ArrangeOptions>)> = fixtures()
        .into_iter()
        .map(|f| {
            let opts = cases(&f.name).into_iter().map(|(o, _)| o).collect();
            (format!("arrange/{}", f.name), f.source, Mode::Auto, opts)
        })
        .collect();
    sources.extend(
        common::fixtures()
            .into_iter()
            .map(|f| (f.name, f.source, f.mode, checks::both().to_vec())),
    );
    for (name, src, mode, sets) in &sources {
        for opts in sets {
            let (_, problems) = checks::check(src, *mode, opts);
            failures.extend(
                problems
                    .into_iter()
                    .map(|p| format!("{name}{}: {p}", checks::suffix(opts))),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Whether `fmt(arrange(x)) == arrange(fmt(x))` for one source and one set
/// of options; the two sides on failure.
fn commutes(
    fixture: &common::Fixture,
    opts: &Options,
    arrange_opts: &ArrangeOptions,
) -> Result<(), (String, String)> {
    let fmt = |src: &str| common::format_case(fixture, src, opts);
    let left = fmt(&arrange::arrange(&fixture.source, fixture.mode, arrange_opts).text);
    let right = arrange::arrange(&fmt(&fixture.source), fixture.mode, arrange_opts).text;
    if left == right {
        Ok(())
    } else {
        Err((left, right))
    }
}

#[test]
fn commutation() {
    let mut cases: Vec<(String, common::Fixture, Options)> = Vec::new();
    for f in common::fixtures() {
        for (i, case) in f.cases.iter().enumerate() {
            let fixture = common::Fixture {
                name: f.name.clone(),
                dir: f.dir.clone(),
                path: f.path.clone(),
                source: f.source.clone(),
                mode: f.mode,
                cases: Vec::new(),
            };
            cases.push((format!("{}[{i}]", f.name), fixture, case.options.clone()));
        }
    }
    for f in fixtures() {
        for (i, settings) in [
            r#"{"newline": "\n", "islands.config": "off"}"#,
            r#"{"newline": "\n", "islands.config": "off", "alignment.consecutive.properties": true}"#,
        ]
        .into_iter()
        .enumerate()
        {
            let (opts, _) = Options::from_map(common::settings(settings)).unwrap();
            let fixture = common::Fixture {
                name: f.name.clone(),
                dir: f.dir.clone(),
                path: f.dir.join("source.cfc"),
                source: f.source.clone(),
                mode: Mode::Auto,
                cases: Vec::new(),
            };
            cases.push((format!("arrange/{}[{i}]", f.name), fixture, opts));
        }
    }
    let mut failures = Vec::new();
    for (key, fixture, opts) in &cases {
        for arrange_opts in checks::both() {
            let full = format!("{key}{}", checks::suffix(&arrange_opts));
            let expected = EXPECT_COMMUTE_FAIL
                .iter()
                .find(|(k, _)| *k == key || *k == full)
                .map(|(_, why)| *why);
            match (commutes(fixture, opts, &arrange_opts), expected) {
                (Ok(()), None) | (Err(_), Some(_)) => {}
                (Ok(()), Some(why)) => {
                    failures.push(format!("{full}: listed ({why}) but commutes"))
                }
                (Err((left, right)), None) => failures.push(format!(
                    "{full}:\n{}",
                    TextDiff::from_lines(&left, &right)
                        .unified_diff()
                        .header("fmt(arrange(x))", "arrange(fmt(x))")
                )),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
