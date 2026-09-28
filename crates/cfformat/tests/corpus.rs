//! Corpus smoke test: formats every `.cfc` under the sibling
//! `commandbox-cfformat` checkout with default options and asserts no panic,
//! idempotence, the output checks of `common::check_output` (token
//! preservation, reparse, comment count) and literal preservation
//! (`common::check_literals`; islands of no language, `<cfquery>` SQL and
//! Java bodies exactly: `common::check_preserved`). Trailing whitespace (only inside
//! multi-line strings; a line of an island that keeps its literal text is
//! counted apart) and lines over `max_columns` are reported. Files that
//! do not parse are counted, not failed. Script and tag mode are counted
//! separately. Prints how many files the formatter changes (informational).
//!
//! Ignored by default (it needs the checkout and takes a few seconds):
//!
//! ```text
//! cargo test -p cfformat --test corpus -- --ignored --nocapture
//! ```
//!
//! `CFFORMAT_CORPUS=<dir>` scans another directory instead;
//! `CFFORMAT_CORPUS_CFM=1` walks `.cfm` files too (reported on their own line);
//! `CFFORMAT_WIDE=1`
//! prints every line over `max_columns`. `CFFORMAT_CORPUS_OPTIONS='{…}'` merges
//! a settings object over the defaults (`{"alignment.consecutive.assignments":
//! true, "alignment.doc_comments": true}`); `CFFORMAT_CORPUS_DISCOVER=1`
//! resolves each file's options with `Discovery` (the home file disabled, the
//! options object, when given, as the `--config` layer), so a tree formatted by
//! CommandBox with its own `.cfformat.json` is compared with its own settings.
//! One `Islands` serves the whole run, each file passes its real path, every
//! island warning is printed as `file:line: key: message` (first run only)
//! and an `islands:` line reports the formatter runs, the cache hits (text
//! already seen: the same island in another file, or an island the formatter
//! left unchanged, seen again by the idempotence run) and the time spent in
//! the formatter.
//!
//! Every panic prints as
//! `path: internal error: message`; a panic is a failure. With
//! `CFFORMAT_CORPUS_REPORT=1` (the soak report) each mode's line is
//! followed by its throughput (files formatted per second of formatting
//! time), the p50 / p95 / max per-file time (`Formatted.timings`: parse, doc
//! and print; the island formatter runs inside doc) and its ten slowest files
//! with their phase split (`doc` without the formatter, `islands` the
//! formatter).
//!
//! A per-file watchdog prints `still running: <file> (N s)` past
//! 10 s and, past `CFFORMAT_CORPUS_TIMEOUT` seconds (default 60), the file as
//! `<file>: timeout after N s` and the counts so far, and exits with status 1.

mod common;
#[path = "../../cfparse/tests/common/watchdog.rs"]
mod watchdog;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cfformat::options::Discovery;
use cfformat::{format_with, FormatCtx, Formatted, Islands, Options, WarningKind};
use cfparse::Mode;

fn cfc_files(dir: &Path, cfm: bool, out: &mut Vec<PathBuf>) {
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
            cfc_files(&path, cfm, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "cfc" || (cfm && e == "cfm"))
        {
            out.push(path);
        }
    }
}

/// Per-extension and per-mode counters.
#[derive(Default)]
struct Counts {
    files: usize,
    changed: usize,
    unchanged: usize,
    panics: usize,
    wide: usize,
    /// Per formatted file: its name and the first run's phase times.
    times: Vec<(String, FileTime)>,
}

/// One file's first formatting run, split by phase.
#[derive(Clone, Copy)]
struct FileTime {
    parse: Duration,
    /// Building the document, the island formatter excluded.
    doc: Duration,
    print: Duration,
    islands: Duration,
}

impl FileTime {
    fn of(f: &Formatted) -> Self {
        FileTime {
            parse: f.timings.parse,
            doc: f.timings.doc.saturating_sub(f.islands.time),
            print: f.timings.print,
            islands: f.islands.time,
        }
    }

    fn total(self) -> Duration {
        self.parse + self.doc + self.print + self.islands
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl Counts {
    fn line(&self, what: &str) -> String {
        format!(
            "{} {what}: {} changed, {} unchanged, {} lines over max_columns",
            self.files, self.changed, self.unchanged, self.wide
        ) + &if self.panics > 0 {
            format!(", {} panics", self.panics)
        } else {
            String::new()
        }
    }

    /// The soak report's lines for this mode: throughput, percentiles and the
    /// slowest files.
    fn report(&self) -> Vec<String> {
        let mut totals: Vec<Duration> = self.times.iter().map(|(_, t)| t.total()).collect();
        if totals.is_empty() {
            return Vec::new();
        }
        totals.sort();
        let sum: Duration = totals.iter().sum();
        let at = |q: f64| totals[((totals.len() - 1) as f64 * q).round() as usize];
        let mut out = vec![format!(
            "  {:.0} files/s ({} files in {:.2} s), per file p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            totals.len() as f64 / sum.as_secs_f64().max(f64::EPSILON),
            totals.len(),
            sum.as_secs_f64(),
            ms(at(0.5)),
            ms(at(0.95)),
            ms(*totals.last().unwrap())
        )];
        let mut slowest: Vec<&(String, FileTime)> = self.times.iter().collect();
        slowest.sort_by_key(|(_, t)| std::cmp::Reverse(t.total()));
        for (name, t) in slowest.into_iter().take(10) {
            out.push(format!(
                "  {:>9.2} ms  {name}  (parse {:.2} / doc {:.2} / print {:.2} / islands {:.2})",
                ms(t.total()),
                ms(t.parse),
                ms(t.doc),
                ms(t.print),
                ms(t.islands)
            ));
        }
        out
    }
}

/// A panic's message: the payload as `&str` or `String`, else `panic`.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

#[test]
#[ignore]
fn corpus() {
    let dir = std::env::var_os("CFFORMAT_CORPUS")
        .map(PathBuf::from)
        .or_else(common::commandbox_dir);
    let Some(dir) = dir else {
        eprintln!("corpus: ../commandbox-cfformat not found, skipped");
        return;
    };
    let with_cfm = std::env::var_os("CFFORMAT_CORPUS_CFM").is_some();
    let report = std::env::var_os("CFFORMAT_CORPUS_REPORT").is_some();
    let run_started = std::time::Instant::now();
    let mut files = Vec::new();
    cfc_files(&dir, with_cfm, &mut files);
    files.sort();
    let extra = std::env::var("CFFORMAT_CORPUS_OPTIONS").ok();
    let discovery = std::env::var_os("CFFORMAT_CORPUS_DISCOVER").map(|_| {
        let config = extra.as_ref().map(|json| {
            let path = std::env::temp_dir().join(format!(
                "cfformat-corpus-options-{}.json",
                std::process::id()
            ));
            std::fs::write(&path, json).unwrap();
            path
        });
        Discovery::new(None, config)
    });
    let mut base = common::settings(extra.as_deref().unwrap_or("{}"));
    base.entry("newline").or_insert_with(|| "\n".into());
    let (base, _) = Options::from_map(base).expect("CFFORMAT_CORPUS_OPTIONS");
    let (mut with_settings_file, mut settings_errors) = (0, 0);
    let (mut script, mut tags, mut cfm) = (Counts::default(), Counts::default(), Counts::default());
    let (mut trailing_ws, mut trailing_exempt) = (0, 0);
    let mut literal_islands = 0;
    let mut preserved = common::Preserved::default();
    let islands = Islands::new();
    let mut island_warnings = 0;
    // Regions the parse recovered in, printed as written.
    let mut recovered = 0;
    let mut failures = Vec::new();
    let watchdog = watchdog::Watchdog::start("CFFORMAT_CORPUS_TIMEOUT", 1);
    for path in &files {
        let src = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
        let name = path
            .strip_prefix(&dir)
            .unwrap_or(path)
            .display()
            .to_string();
        watchdog.summary(format!(
            "corpus so far: {} script, {} tag, {} .cfm files, {} failures",
            script.files,
            tags.files,
            cfm.files,
            failures.len()
        ));
        let opts = match &discovery {
            None => base.clone(),
            Some(d) => match d.discover(path) {
                Ok(r) => {
                    with_settings_file += usize::from(!r.sources.is_empty());
                    r.options
                }
                Err(e) => {
                    eprintln!("{name}: {e}");
                    settings_errors += 1;
                    continue;
                }
            },
        };
        let opts = &opts;
        let tag_mode = cfparse::parse_source(&src, Mode::Auto).mode() == Mode::Tags;
        let counts = if name.ends_with(".cfm") {
            &mut cfm
        } else if tag_mode {
            &mut tags
        } else {
            &mut script
        };
        counts.files += 1;
        watchdog.begin(0, &name);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let ctx = FormatCtx {
                path: Some(path),
                islands: Some(&islands),
            };
            let once = format_with(&src, Mode::Auto, opts, &ctx);
            let time = FileTime::of(&once);
            for w in &once.warnings {
                eprintln!("{name}:{}: {}: {}", w.line, w.label(), w.message);
            }
            let islands = |w: &&cfformat::Warning| matches!(w.kind, WarningKind::Island { .. });
            island_warnings += once.warnings.iter().filter(islands).count();
            recovered += once.warnings.len() - once.warnings.iter().filter(islands).count();
            let twice = format_with(&once.text, Mode::Auto, opts, &ctx);
            (once.text, twice.text, time)
        }));
        match result {
            Err(payload) => {
                let line = format!(
                    "{name}: internal error: {}",
                    panic_message(payload.as_ref())
                );
                eprintln!("{line}");
                counts.panics += 1;
                failures.push(line);
            }
            Ok((once, twice, time)) => {
                counts.times.push((name.clone(), time));
                if once != twice {
                    let diff = similar::TextDiff::from_lines(&once, &twice);
                    failures.push(format!(
                        "{name}: not idempotent\n{}",
                        diff.unified_diff()
                            .context_radius(2)
                            .header("once", "twice")
                    ));
                }
                for p in common::check_output(&src, &once, Mode::Auto, opts) {
                    failures.push(format!("{name}: {p}"));
                }
                let (checked, problems) = common::check_literals(&src, &once, Mode::Auto, opts);
                literal_islands += checked;
                failures.extend(problems.into_iter().map(|p| format!("{name}: {p}")));
                let (checked, problems) = common::check_preserved(&src, &once, Mode::Auto);
                preserved += checked;
                failures.extend(problems.into_iter().map(|p| format!("{name}: {p}")));
                let exempt = common::literal_island_lines(&once, Mode::Auto);
                for (n, l) in once.lines().enumerate() {
                    if l.ends_with([' ', '\t']) && exempt.contains(&(n + 1)) {
                        trailing_exempt += 1;
                    } else if l.ends_with([' ', '\t']) {
                        trailing_ws += 1;
                        if trailing_ws <= 10 {
                            eprintln!("{name}:{}: trailing whitespace: {l:?}", n + 1);
                        }
                    }
                    if l.chars().count() > opts.max_columns {
                        counts.wide += 1;
                        if std::env::var_os("CFFORMAT_WIDE").is_some() {
                            eprintln!("{name}:{}: over max_columns: {l}", n + 1);
                        }
                    }
                }
                if once.replace("\r\n", "\n") == src.replace("\r\n", "\n") {
                    counts.unchanged += 1;
                } else {
                    counts.changed += 1;
                }
            }
        }
        watchdog.end(0);
    }
    if discovery.is_some() {
        eprintln!(
            "corpus: discovery: {with_settings_file} files under a .cfformat.json, {settings_errors} settings errors"
        );
    }
    let mut modes = vec![
        (&script, ".cfc files, script mode"),
        (&tags, ".cfc files, tag mode"),
    ];
    if with_cfm {
        modes.push((&cfm, ".cfm files"));
    }
    for (counts, what) in modes {
        eprintln!("corpus: {}", counts.line(what));
        if report {
            for line in counts.report() {
                eprintln!("corpus: {line}");
            }
        }
    }
    let stats = islands.stats();
    if stats.islands() > 0 {
        eprintln!(
            "corpus: islands: {} formatted, {} cached, {island_warnings} warnings, {:.2} s",
            stats.formatted,
            stats.cached,
            stats.time.as_secs_f64()
        );
    }
    if recovered > 0 {
        eprintln!("corpus: {recovered} regions not formatted (the parse recovered)");
    }
    eprintln!(
        "corpus: {} files: {trailing_ws} lines with trailing whitespace ({trailing_exempt} more inside islands that keep their literal text), literals preserved in {literal_islands} JavaScript islands, text preserved in {} islands of no language, literals preserved in {} SQL and {} Java bodies, {} failures",
        files.len(),
        preserved.opaque,
        preserved.sql,
        preserved.java,
        failures.len()
    );
    if report {
        eprintln!(
            "corpus: {:.2} s in all, formatting twice and checking",
            run_started.elapsed().as_secs_f64()
        );
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
