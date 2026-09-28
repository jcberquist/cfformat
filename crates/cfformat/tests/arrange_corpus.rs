//! The arrange soak: every `.cfc` under the sibling `commandbox-cfformat`
//! checkout (or `CFFORMAT_CORPUS=<dir>`) is arranged without and with
//! `--properties`, and each result is held to the invariants of
//! `tests/arrange.rs`: idempotence, permutation, sortedness, and
//! commutation with the formatter (`fmt(arrange(x)) == arrange(fmt(x))`)
//! at the default options, or with `CFFORMAT_CORPUS_OPTIONS='{…}'` merged
//! over them (`{"alignment.consecutive.properties": true}`).
//!
//! Ignored by default (it needs a corpus):
//!
//! ```text
//! cargo test --release -p cfformat --test arrange_corpus -- --ignored --nocapture
//! ```
//!
//! It prints, per option set, the files and bodies seen, the bodies skipped
//! (the parse recovered in them), the bodies with a run and those changed,
//! the runs, the units in runs, moved and alone, and the fixed elements by
//! reason. `CFFORMAT_ARRANGE_LIST=1` also prints every changed file, for
//! reading the diffs (`cfformat arrange --diff FILE`). A broken invariant
//! or a panic fails the run. A commutation failure is printed with the file
//! and counted: each one read so far is a formatter behaviour that changes
//! what attaches to a member (the causes `tests/arrange.rs` lists in
//! `EXPECT_COMMUTE_FAIL`), so it fails the run only with
//! `CFFORMAT_ARRANGE_STRICT=1`. The per-file watchdog is the corpus test's
//! (`CFFORMAT_CORPUS_TIMEOUT`, default 60 s).

#[path = "common/arrange.rs"]
mod checks;
mod common;
#[path = "../../cfparse/tests/common/watchdog.rs"]
mod watchdog;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use cfformat::arrange::{self, Fixed, Ineligible};
use cfformat::{format_source, Options};
use cfparse::Mode;

fn cfc_files(dir: &Path, out: &mut Vec<PathBuf>) {
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
            cfc_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cfc"))
        {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct Counts {
    files: usize,
    bodies: usize,
    skipped: usize,
    with_runs: usize,
    changed_bodies: usize,
    changed_files: Vec<String>,
    runs: usize,
    in_runs: usize,
    moved: usize,
    alone: usize,
    fixed: BTreeMap<&'static str, usize>,
    commute_failures: Vec<String>,
}

fn reason(f: Fixed) -> &'static str {
    match f {
        Fixed::Comment => "standalone comment",
        Fixed::Banner => "banner",
        Fixed::Statement => "statement",
        Fixed::Property => "property",
        Fixed::Ignore => "ignore",
        Fixed::Other => "text or tag",
        Fixed::SharedLine => "shared line",
        Fixed::Ineligible(Ineligible::Name) => "ineligible: name",
        Fixed::Ineligible(Ineligible::Access) => "ineligible: access",
        Fixed::Ineligible(Ineligible::Disagree) => "ineligible: disagree",
        Fixed::Ineligible(Ineligible::DocAccess) => "ineligible: @access",
        Fixed::Ineligible(Ineligible::PropertyName) => "ineligible: property name",
        Fixed::Ineligible(Ineligible::TwoNames) => "ineligible: two names",
    }
}

#[test]
#[ignore = "needs a corpus: CFFORMAT_CORPUS=<dir> or the commandbox-cfformat checkout"]
fn arrange_corpus() {
    let dir = std::env::var_os("CFFORMAT_CORPUS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(common::commandbox_dir)
        .expect("CFFORMAT_CORPUS or the commandbox-cfformat checkout");
    let list = std::env::var_os("CFFORMAT_ARRANGE_LIST").is_some();
    let strict = std::env::var_os("CFFORMAT_ARRANGE_STRICT").is_some();
    let mut files = Vec::new();
    cfc_files(&dir, &mut files);
    files.sort();
    let mut settings =
        common::settings(&std::env::var("CFFORMAT_CORPUS_OPTIONS").unwrap_or_else(|_| "{}".into()));
    settings.entry("newline").or_insert_with(|| "\n".into());
    settings
        .entry("islands.config")
        .or_insert_with(|| "off".into());
    let (fmt_opts, _) = Options::from_map(settings).expect("CFFORMAT_CORPUS_OPTIONS");
    let mut counts = [Counts::default(), Counts::default()];
    let mut failures = Vec::new();
    let watchdog = watchdog::Watchdog::start("CFFORMAT_CORPUS_TIMEOUT", 1);
    for path in &files {
        let Ok(src) = String::from_utf8(std::fs::read(path).unwrap()) else {
            continue;
        };
        let name = path
            .strip_prefix(&dir)
            .unwrap_or(path)
            .display()
            .to_string();
        watchdog.summary(format!(
            "arrange soak so far: {} files, {} failures",
            counts[0].files,
            failures.len()
        ));
        watchdog.begin(0, &name);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let formatted = format_source(&src, Mode::Auto, &fmt_opts);
            let tree = cfparse::parse_source(&src, Mode::Auto);
            for (opts, c) in checks::BOTH.iter().zip(counts.iter_mut()) {
                c.files += 1;
                let (out, problems) = checks::check(&src, Mode::Auto, opts);
                for p in problems {
                    failures.push(format!("{name}{}: {p}", checks::suffix(opts)));
                }
                let bodies = arrange::bodies(&tree, opts);
                for b in &bodies {
                    c.bodies += 1;
                    c.skipped += usize::from(b.skipped.is_some());
                    c.with_runs += usize::from(!b.runs.is_empty());
                    c.alone += b.alone;
                    let mut moved_here = 0;
                    for run in &b.runs {
                        c.runs += 1;
                        c.in_runs += run.units.len();
                        let mut order: Vec<usize> = (0..run.units.len()).collect();
                        order.sort_by(|&x, &y| run.units[x].order(&run.units[y]));
                        moved_here += order.iter().enumerate().filter(|(s, u)| s != *u).count();
                    }
                    c.moved += moved_here;
                    c.changed_bodies += usize::from(moved_here > 0);
                    for (_, f) in &b.fixed {
                        *c.fixed.entry(reason(*f)).or_default() += 1;
                    }
                }
                if out.changed {
                    c.changed_files.push(name.clone());
                }
                let left = if out.changed {
                    format_source(&out.text, Mode::Auto, &fmt_opts)
                } else {
                    formatted.clone()
                };
                let right = arrange::arrange(&formatted, Mode::Auto, opts).text;
                if left != right {
                    c.commute_failures.push(name.clone());
                }
            }
        }));
        if let Err(e) = result {
            let message = e
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            failures.push(format!("{name}: internal error: {message}"));
        }
        watchdog.end(0);
    }
    for (opts, c) in checks::BOTH.iter().zip(&counts) {
        let with = if opts.properties {
            "with --properties"
        } else {
            "without --properties"
        };
        eprintln!(
            "arrange {with}: {} files, {} bodies, {} skipped, {} with a run, {} changed \
             ({} files); {} runs, {} units in runs, {} moved, {} alone",
            c.files,
            c.bodies,
            c.skipped,
            c.with_runs,
            c.changed_bodies,
            c.changed_files.len(),
            c.runs,
            c.in_runs,
            c.moved,
            c.alone
        );
        let fixed: Vec<String> = c.fixed.iter().map(|(k, v)| format!("{k} {v}")).collect();
        eprintln!("  fixed: {}", fixed.join(", "));
        eprintln!("  {} files do not commute", c.commute_failures.len());
        if list {
            for f in &c.changed_files {
                eprintln!("  changed: {f}");
            }
        }
        for f in &c.commute_failures {
            eprintln!("  does not commute: {f}");
        }
        if strict {
            failures.extend(
                c.commute_failures
                    .iter()
                    .map(|f| format!("{f}{}: does not commute", checks::suffix(opts))),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
