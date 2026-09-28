//! The inspect JSON (no spans) of every vendored fixture source
//! (`tests/fixtures`, copied from commandbox-cfformat). This is the parse
//! contract; regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p cfparse
//! --test snapshots` and review the diff.

mod common;

use std::path::PathBuf;

use cfparse::json::{to_json, to_string_pretty, JsonOpts};
use cfparse::parse_source;

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

#[test]
fn fixture_snapshots() {
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let dir = snapshot_dir();
    std::fs::create_dir_all(&dir).unwrap();

    let fixtures = common::fixtures();
    assert_eq!(fixtures.len(), 128, "120 fixtures + 8 exprTests expected");
    // A deleted fixture must take its snapshot with it.
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert!(
            fixtures.iter().any(|f| f.name == stem),
            "orphan snapshot {}",
            path.display()
        );
    }

    let mut failures = Vec::new();
    for f in &fixtures {
        let tree = parse_source(&f.source, f.mode);
        common::assert_covers_source(&f.name, &tree);
        let actual = to_string_pretty(&to_json(&tree, JsonOpts::default()));
        let path = dir.join(format!("{}.json", f.name));
        if update {
            std::fs::write(&path, &actual).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => {
                failures.push(format!("{}\n{}", f.name, first_diff(&expected, &actual)))
            }
            Err(_) => failures.push(format!("{}: missing snapshot {}", f.name, path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} snapshot(s) differ (UPDATE_SNAPSHOTS=1 to accept):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// First differing line with a little context.
fn first_diff(expected: &str, actual: &str) -> String {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let i = e
        .iter()
        .zip(&a)
        .position(|(x, y)| x != y)
        .unwrap_or(e.len().min(a.len()));
    let from = i.saturating_sub(3);
    let mut out = String::new();
    for (n, line) in e.iter().enumerate().skip(from).take(i - from) {
        out.push_str(&format!("  {:>5} | {line}\n", n + 1));
    }
    if let Some(line) = e.get(i) {
        out.push_str(&format!("- {:>5} | {line}\n", i + 1));
    }
    if let Some(line) = a.get(i) {
        out.push_str(&format!("+ {:>5} | {line}\n", i + 1));
    }
    out
}
