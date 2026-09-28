//! The soak run: lint a corpus and print what the rule reports, to read by
//! hand.
//!
//! ```text
//! cargo test -p cfvet --test soak -- --ignored --nocapture
//! ```
//!
//! `CFVET_CORPUS=<dir>` lints another directory instead of the
//! `commandbox-cfformat` checkout next to the workspace. Every `.cfc` and
//! `.cfm` file is linted in `Mode::Auto`; each report prints as the binary
//! prints it, then the totals.

mod common;

use std::path::PathBuf;

use cfparse::Mode;
use cfvet::lint_source;

#[test]
#[ignore = "reads a corpus outside the workspace; run by hand"]
fn soak() {
    let dir = match std::env::var_os("CFVET_CORPUS") {
        Some(dir) => PathBuf::from(dir),
        None => common::commandbox_dir().expect("no CFVET_CORPUS and no commandbox-cfformat"),
    };
    let files = common::cfml_files(&dir);
    let (mut reports, mut notes, mut functions, mut flagged) = (0, 0, 0, 0);
    for file in &files {
        let Ok(src) = std::fs::read_to_string(file) else {
            println!("{}: not UTF-8, skipped", file.display());
            continue;
        };
        let lint = lint_source(&src, Mode::Auto);
        let name = file
            .strip_prefix(&dir)
            .unwrap_or(file)
            .display()
            .to_string();
        for note in &lint.notes {
            println!("{}", note.render(&name));
        }
        for report in &lint.reports {
            println!("{}", report.render(&name));
        }
        reports += lint.reports.len();
        notes += lint.notes.len();
        functions += lint.functions;
        flagged += usize::from(!lint.reports.is_empty());
    }
    println!(
        "{reports} reports in {flagged} of {} files ({functions} functions checked, {notes} notes)",
        files.len()
    );
}
