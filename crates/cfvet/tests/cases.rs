//! Each `cases/NAME.cfc`, `cases/NAME.cfs` or `cases/NAME.cfm` is linted in
//! the mode the binary gives its name ([`Mode::for_path`]) and its report
//! lines (path printed as `cases/NAME.ext`) compared with
//! `cases/NAME.expected`. `UPDATE_CASES=1` rewrites the expected files
//! instead.

use std::path::Path;

use cfparse::Mode;
use cfvet::lint_source;

#[test]
fn cases() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let update = std::env::var_os("UPDATE_CASES").is_some();
    let mut sources: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == "cfc" || e == "cfs" || e == "cfm")
        })
        .collect();
    sources.sort();
    assert!(!sources.is_empty());
    let mut failures = Vec::new();
    for source in &sources {
        let file = source.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(source).unwrap();
        let lint = lint_source(&src, Mode::for_path(source));
        let actual: String = lint
            .reports
            .iter()
            .map(|r| r.render(&format!("cases/{file}")) + "\n")
            .collect();
        let expected_path = source.with_extension("expected");
        if update {
            std::fs::write(&expected_path, &actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            failures.push(format!(
                "{file}:\n--- expected\n{expected}--- actual\n{actual}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
