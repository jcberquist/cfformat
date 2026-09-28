//! The formatter's phases over the golden fixtures: every
//! `tests/fixtures/*/source.cfc` concatenated per mode — script sources (a
//! first line `//`, dropped) and tag sources — then `parse`
//! (`cfparse::parse_source`), `to_doc` (`tree_to_doc` on the parsed tree),
//! `print` (`print_doc` on the built document) and `format`
//! (`format_source`, the three together), with the default options.
//! Informative only, not run in CI.
//!
//! ```text
//! cargo bench -p cfformat --bench format
//! ```

use std::path::Path;

use cfdoc::{print_doc, IndentStyle, PrintOptions};
use cfformat::{format_source, tree_to_doc, Options};
use cfparse::{parse_source, Mode};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

/// The fixture sources joined per mode: `(script, tags)`.
fn corpus() -> (String, String) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut dirs: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("source.cfc").is_file())
        // Unclosed on purpose: concatenated, it would swallow every script
        // fixture after it into one unmatched run.
        .filter(|p| !p.ends_with("callUnclosed"))
        .collect();
    dirs.sort();
    let (mut script, mut tags) = (String::new(), String::new());
    for d in dirs {
        let raw = std::fs::read_to_string(d.join("source.cfc")).unwrap();
        match raw.strip_prefix("//") {
            Some(rest) => {
                script.push_str(rest.split_once('\n').map_or("", |(_, body)| body));
                script.push('\n');
            }
            None => {
                tags.push_str(&raw);
                tags.push('\n');
            }
        }
    }
    (script, tags)
}

fn bench(c: &mut Criterion) {
    let (script, tags) = corpus();
    let opts = Options::default();
    let print_opts = PrintOptions {
        width: opts.max_columns,
        indent: IndentStyle::Spaces(opts.indent_size),
        newline: "\n",
    };
    for (name, src, mode) in [
        ("script", &script, Mode::Script),
        ("tags", &tags, Mode::Tags),
    ] {
        let tree = parse_source(src, mode);
        let doc = tree_to_doc(&tree, &opts);
        let out = format_source(src, mode, &opts);
        eprintln!(
            "{name}: {} source lines, {} bytes; {} output lines",
            src.lines().count(),
            src.len(),
            out.lines().count()
        );
        c.bench_function(&format!("{name}/parse"), |b| {
            b.iter(|| parse_source(src, mode))
        });
        c.bench_function(&format!("{name}/to_doc"), |b| {
            b.iter(|| tree_to_doc(&tree, &opts))
        });
        c.bench_function(&format!("{name}/print"), |b| {
            b.iter_batched(
                || doc.clone(),
                |mut doc| print_doc(&mut doc, &print_opts),
                BatchSize::LargeInput,
            )
        });
        c.bench_function(&format!("{name}/format"), |b| {
            b.iter(|| format_source(src, mode, &opts))
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
