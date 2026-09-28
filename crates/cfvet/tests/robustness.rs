//! Lints every source the formatter's fixtures hold and, when the
//! `commandbox-cfformat` checkout sits next to the workspace, every CFML
//! file in it: no panic, and every report inside its file.

mod common;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use cfparse::Mode;
use cfvet::lint_source;

fn check(files: &[PathBuf]) -> Vec<String> {
    let mut failures = Vec::new();
    for file in files {
        let Ok(src) = std::fs::read_to_string(file) else {
            continue;
        };
        let name = file.display();
        match catch_unwind(AssertUnwindSafe(|| lint_source(&src, Mode::Auto))) {
            Err(_) => failures.push(format!("{name}: panicked")),
            Ok(lint) => {
                // The parser normalises `\r\n`, so its lines are `\n` lines.
                let lines = src.replace("\r\n", "\n").replace('\r', "\n");
                let lines: Vec<&str> = lines.split('\n').collect();
                for r in &lint.reports {
                    let fits = r.line >= 1
                        && r.column >= 1
                        && lines
                            .get(r.line - 1)
                            .is_some_and(|l| r.column <= l.chars().count());
                    if !fits {
                        failures.push(format!("{name}: report outside the file: {r:?}"));
                    }
                }
            }
        }
    }
    failures
}

#[test]
fn fixtures_lint_without_panicking() {
    let files = common::cfml_files(&common::fixtures_dir());
    assert!(files.len() > 100, "{} fixtures", files.len());
    let failures = check(&files);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_corpus_lints_without_panicking() {
    let Some(dir) = common::commandbox_dir() else {
        eprintln!("no commandbox-cfformat checkout; skipped");
        return;
    };
    let failures = check(&common::cfml_files(&dir));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
