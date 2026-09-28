//! `parse_source` over `commandbox-cfformat`.
//!
//! ```text
//! cargo bench -p cfparse --bench parse
//! ```
//!
//! Four groups: `script_parse`, a whole `parse_source` over `models/` (the
//! script front end and the six
//! post-passes); `script_front_end`, the front end alone (the gap to
//! `script_parse` is the post-passes' share); `tags_parse`, a whole
//! `parse_source` over the checkout's tag-mode `.cfc` and its `.cfm` (the tag
//! scanner, the fragments it hands to the script front end and the same six
//! passes); and `tags_front_end`, the scanner alone.

use std::path::{Path, PathBuf};

use cfparse::{parse_source, Mode};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn checkout() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../commandbox-cfformat");
    if !dir.join("models").is_dir() {
        eprintln!(
            "{}: commandbox-cfformat checkout not found next to the workspace; nothing to bench",
            dir.display()
        );
        std::process::exit(0);
    }
    dir
}

fn read(dir: &Path, ext: &str) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect(dir, ext, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let src = std::fs::read_to_string(&p).unwrap();
            (p, src)
        })
        .collect()
}

fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_dir() {
            collect(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

fn bytes(files: &[(PathBuf, String, Mode)]) -> u64 {
    files.iter().map(|(_, s, _)| s.len() as u64).sum()
}

fn bench_script(c: &mut Criterion) {
    let files = read(&checkout().join("models"), "cfc");
    let concatenated: String = files
        .iter()
        .map(|(_, s)| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Refuse to report a number for a corpus the front end does not parse.
    let normalized = cfparse::normalize::normalized_text(&concatenated);
    if let Err(e) = cfparse::script::parse(&normalized, Mode::Auto) {
        eprintln!("models: {e}");
        std::process::exit(1);
    }

    let mut group = c.benchmark_group("script_parse");
    group.throughput(Throughput::Bytes(concatenated.len() as u64));
    group.bench_function("models concatenated", |b| {
        b.iter(|| parse_source(&concatenated, Mode::Auto))
    });
    group.bench_function("models per file", |b| {
        b.iter(|| {
            for (_, src) in &files {
                parse_source(src, Mode::Auto);
            }
        })
    });
    group.finish();

    let mut group = c.benchmark_group("script_front_end");
    group.throughput(Throughput::Bytes(concatenated.len() as u64));
    group.bench_function("models concatenated", |b| {
        b.iter(|| cfparse::script::parse(&normalized, Mode::Auto).unwrap())
    });
    group.finish();
}

fn bench_tags(c: &mut Criterion) {
    let dir = checkout();
    let mut files: Vec<(PathBuf, String, Mode)> = read(&dir, "cfc")
        .into_iter()
        .filter(|(_, src)| parse_source(src, Mode::Auto).mode() == Mode::Tags)
        .map(|(p, s)| (p, s, Mode::Auto))
        .collect();
    let cfc = files.len();
    files.extend(
        read(&dir, "cfm")
            .into_iter()
            .map(|(p, s)| (p, s, Mode::Tags)),
    );
    eprintln!(
        "tags_parse: {cfc} tag-mode .cfc, {} .cfm, {} bytes",
        files.len() - cfc,
        bytes(&files)
    );
    // A whole `parse_source`, and the front end alone (the gap is the
    // post-passes' share).
    let normalized: Vec<String> = files
        .iter()
        .map(|(_, src, _)| cfparse::normalize::normalized_text(src))
        .collect();
    let tag_mode: Vec<&String> = files
        .iter()
        .zip(&normalized)
        .filter(|((_, src, mode), _)| parse_source(src, *mode).mode() == Mode::Tags)
        .map(|(_, text)| text)
        .collect();

    let mut group = c.benchmark_group("tags_parse");
    group.throughput(Throughput::Bytes(bytes(&files)));
    group.bench_function("tag .cfc and .cfm per file", |b| {
        b.iter(|| {
            for (_, src, mode) in &files {
                parse_source(src, *mode);
            }
        })
    });
    group.finish();

    let scanned: u64 = tag_mode.iter().map(|s| s.len() as u64).sum();
    let mut group = c.benchmark_group("tags_front_end");
    group.throughput(Throughput::Bytes(scanned));
    group.bench_function("tag .cfc and .cfm per file", |b| {
        b.iter(|| {
            for text in &tag_mode {
                cfparse::tags::parse(text);
            }
        })
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench_script, bench_tags
}
criterion_main!(benches);
