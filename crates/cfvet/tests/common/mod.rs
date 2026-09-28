//! Where the test sources are, and the walk over them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The `commandbox-cfformat` checkout next to the workspace, if present.
pub fn commandbox_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../commandbox-cfformat");
    dir.join("models").is_dir().then_some(dir)
}

/// The formatter's fixtures: CFML of every shape the parser knows.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../cfformat/tests/fixtures")
}

/// Every `.cfc` and `.cfm` file under `dir`, hidden entries skipped, in
/// path order.
pub fn cfml_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("cfc") || e.eq_ignore_ascii_case("cfm"))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}
