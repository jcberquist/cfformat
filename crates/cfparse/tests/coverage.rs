//! Every commandbox-cfformat model parses and its tokens tile the source
//! exactly (fixtures are covered by `snapshots.rs`). Skipped when the
//! `commandbox-cfformat` checkout is not next to the workspace (CI).

mod common;

use cfparse::{parse_source, Mode};

#[test]
fn models_cover_source() {
    let Some(dir) = common::commandbox_dir() else {
        eprintln!("commandbox-cfformat checkout not found next to the workspace; skipping");
        return;
    };
    let dir = dir.join("models");
    let mut n = 0;
    for entry in walk(&dir) {
        let src = std::fs::read_to_string(&entry).unwrap();
        let tree = parse_source(&src, Mode::Auto);
        common::assert_covers_source(&entry.display().to_string(), &tree);
        n += 1;
    }
    assert!(n > 30, "expected the commandbox-cfformat models, found {n}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "cfc") {
            out.push(p);
        }
    }
    out.sort();
    out
}
