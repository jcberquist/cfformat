//! Two fuzz passes over every fixture and every `.cfc` and
//! `.cfm` of every corpus.
//!
//! **Truncation**: each source cut at each eighth of its length must parse
//! without a panic in its own mode and as script, and must format — with
//! default options, as its own mode, islands on — without a panic. Output quality is not
//! checked: a truncated file is not valid CFML, so neither idempotence nor
//! the token checks are required — except that every region the parse
//! recovered in prints as written: its text without whitespace is a
//! run of the output's.
//!
//! **Mutation**: each source gets `CFPARSE_FUZZ_MUTATIONS` (default 8)
//! single edits from a seeded xorshift — a multibyte character (`é`, `💩`,
//! U+0301) or a delimiter (`"`, `'`, `/*`, `*/`, `<!---`, `--->`, `#`, `(`,
//! `)`, `{`, `}`) inserted at a random character boundary, or one character
//! deleted. Each mutated source must parse in its own mode and as script
//! without a panic, the tokens of both parses must tile it with every
//! boundary on a character boundary (and every recovered region on token
//! boundaries), and it must format without a panic.
//! The seed is printed; `CFPARSE_FUZZ_SEED` replays it.
//!
//! Both passes format with islands on: one `Islands` per corpus, so
//! every cut and every mutation of a `<script>` or `<style>` body goes
//! through the hand-off to oxc, the island limits and the island thread,
//! as the CLI's would.
//!
//! ```text
//! cargo test --release -p cfparse --test fuzz -- --ignored --nocapture
//! CFPARSE_CORPUS=/a:/b …        # corpora besides ../commandbox-cfformat
//! CFPARSE_FUZZ_TIMEOUT=60 …     # seconds one case may take (the watchdog)
//! CFPARSE_FUZZ_TRACE=1 …        # print each case before it runs
//! CFPARSE_FUZZ_SEED=… …         # the mutation pass's seed (default: the clock)
//! CFPARSE_FUZZ_MUTATIONS=8 …    # mutations per source
//! ```
//!
//! **Fixed cases** run first in both passes: every input shape that once
//! overflowed the stack (`common::generators`: deep unary and assignment
//! chains, unbraced bodies, brackets in a `#…#`, nested `<cfquery>`, runs
//! of `abort` or `cffile(…)` with no `;`, …) at
//! the size that overflowed and at ten times each; the shapes that were
//! once quadratic (unclosed `#`s retried, opening tags then as many
//! closing tags of another name) at a size that took seconds and at ten
//! times it; the deep `<script>` / `<style>` bodies (JS, JSON and CSS
//! brackets) at the size each first overflowed the CLI's 8 MB worker and
//! at ten times it — all past the
//! island nesting limit, so refused; the nesting with no bracket to count
//! (unbraced `if` / loop / `do` bodies, JSX, labels, `else if`) at 501
//! levels and 5,010: parsed on the island thread and refused by the tree
//! bound; and CSS and JSON brackets with a closer hidden in a comment or a
//! string at every level, and a `}` inside a CSS function's arguments at
//! every level, at 600 and 6,000: refused by the lexical bracket count.
//!
//! Each corpus runs on its own `common::SMALL_STACK` thread (in release the
//! 2 MB a test or a default thread has): the *caller's* stack (oxc has a thread of its own, so an
//! island is bounded the same whatever this one is). A stack overflow is
//! an abort, not a panic: it kills the
//! process, no `catch_unwind` sees it and the run does not finish — so a
//! finished run is also the count of aborts, 0, and `CFPARSE_FUZZ_TRACE`
//! names the case that took the process (on stderr, flushed before it runs).
//!
//! `cfformat` is a dev-dependency here (a cycle Cargo allows for tests) so
//! the printer is fuzzed over the same cases as the parser.

mod common;

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use common::watchdog::Watchdog;

use cfparse::{parse_source, Mode};

/// A source to cut: its name, text and mode.
type Source = (String, String, Mode);

#[derive(Default)]
struct Counts {
    files: usize,
    cases: usize,
    parse_panics: Vec<String>,
    format_panics: Vec<String>,
    /// Recovered regions checked, and the ones whose text did not reach
    /// the output as written.
    recoveries: usize,
    not_as_written: Vec<String>,
}

#[test]
#[ignore = "runs over every corpus; needs --release to be quick"]
fn truncated_sources_neither_panic_nor_abort() {
    let watchdog = Watchdog::start("CFPARSE_FUZZ_TIMEOUT", 1);
    let trace = std::env::var_os("CFPARSE_FUZZ_TRACE").is_some();

    let mut total = Counts::default();
    for (name, sources) in sets() {
        let watchdog = watchdog.clone();
        let counts = std::thread::Builder::new()
            .name(format!("fuzz {name}"))
            .stack_size(common::SMALL_STACK)
            .spawn(move || fuzz(&sources, &watchdog, trace))
            .unwrap()
            .join()
            .unwrap();
        println!(
            "== {name} == {} files, {} cases, {} parse panics, {} format panics, \
             {} of {} recoveries not as written",
            counts.files,
            counts.cases,
            counts.parse_panics.len(),
            counts.format_panics.len(),
            counts.not_as_written.len(),
            counts.recoveries
        );
        total.files += counts.files;
        total.cases += counts.cases;
        total.parse_panics.extend(counts.parse_panics);
        total.format_panics.extend(counts.format_panics);
        total.recoveries += counts.recoveries;
        total.not_as_written.extend(counts.not_as_written);
    }
    println!(
        "== truncation fuzz == {} files, {} cases, {} parse panics, {} format panics, \
         {} of {} recoveries not as written, 0 aborts",
        total.files,
        total.cases,
        total.parse_panics.len(),
        total.format_panics.len(),
        total.not_as_written.len(),
        total.recoveries
    );
    for p in total
        .parse_panics
        .iter()
        .chain(&total.format_panics)
        .chain(&total.not_as_written)
        .take(10)
    {
        println!("  {p}");
    }
    assert!(
        total.parse_panics.is_empty()
            && total.format_panics.is_empty()
            && total.not_as_written.is_empty(),
        "a truncated source panicked or reformatted a recovered region"
    );
}

/// The fixed cases (every input shape that once overflowed the stack, at
/// its sizes and ten times each — [`common::fixed_cases`]), the
/// fixtures, then every corpus: each a name and its sources.
fn sets() -> Vec<(String, Vec<Source>)> {
    let mut sets: Vec<(String, Vec<Source>)> = vec![
        ("fixed cases".into(), common::fixed_cases()),
        (
            "fixtures".into(),
            common::fixtures()
                .into_iter()
                .map(|f| (f.name, f.source, f.mode))
                .collect(),
        ),
    ];
    for (name, dir) in corpora() {
        let mut files = Vec::new();
        for ext in ["cfc", "cfm"] {
            collect(&dir, ext, &mut files);
        }
        files.sort();
        let sources = files
            .iter()
            .filter_map(|p| {
                let source = std::fs::read_to_string(p).ok()?;
                // As `cfformat`'s `run.rs`: `.cfm` is tag mode, `.cfc` auto.
                let mode = if p.extension().is_some_and(|e| e == "cfm") {
                    Mode::Tags
                } else {
                    Mode::Auto
                };
                Some((p.display().to_string(), source, mode))
            })
            .collect();
        sets.push((name, sources));
    }
    sets
}

/// Every source cut at each eighth: parsed as its mode and as script, then
/// formatted as its mode.
fn fuzz(sources: &[Source], watchdog: &Watchdog, trace: bool) -> Counts {
    let mut counts = Counts::default();
    let opts = cfformat::Options::default();
    let islands = cfformat::Islands::new();
    let ctx = cfformat::FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    for (name, source, mode) in sources {
        counts.files += 1;
        for eighth in 1..8 {
            let mut cut = source.len() * eighth / 8;
            while cut > 0 && !source.is_char_boundary(cut) {
                cut -= 1;
            }
            counts.cases += 1;
            let text = &source[..cut];
            let case = format!("{name} (truncated at {cut} of {})", source.len());
            if trace {
                eprintln!("{case}");
                let _ = std::io::stderr().flush();
            }
            watchdog.begin(0, &case);
            let tree = match catch_unwind(AssertUnwindSafe(|| {
                let _ = parse_source(text, Mode::Script);
                parse_source(text, *mode)
            })) {
                Ok(tree) => Some(tree),
                Err(panic) => {
                    counts
                        .parse_panics
                        .push(format!("parse: {case}: {}", panic_message(&panic)));
                    None
                }
            };
            match catch_unwind(AssertUnwindSafe(|| {
                cfformat::format_with(text, *mode, &opts, &ctx).text
            })) {
                Ok(out) => {
                    // A recovered region prints as written: its text
                    // without whitespace is a run of the output's.
                    let bare = |s: &str| s.split_whitespace().collect::<String>();
                    let out = bare(&out);
                    for r in tree.iter().flat_map(|t| &t.recoveries) {
                        counts.recoveries += 1;
                        let region = bare(tree.as_ref().unwrap().slice(r.span.clone()));
                        if !out.contains(&region) {
                            counts.not_as_written.push(format!(
                                "{case}: {:?} region {:?} is not in the output as written",
                                r.reason, r.span
                            ));
                        }
                    }
                }
                Err(panic) => counts
                    .format_panics
                    .push(format!("format: {case}: {}", panic_message(&panic))),
            }
            watchdog.end(0);
        }
    }
    counts
}

#[derive(Default)]
struct MutationCounts {
    files: usize,
    cases: usize,
    parse_panics: Vec<String>,
    format_panics: Vec<String>,
    boundary_failures: Vec<String>,
}

#[test]
#[ignore = "runs over every corpus; needs --release to be quick"]
fn mutated_sources_tile_and_neither_panic_nor_abort() {
    let watchdog = Watchdog::start("CFPARSE_FUZZ_TIMEOUT", 1);
    let trace = std::env::var_os("CFPARSE_FUZZ_TRACE").is_some();
    let seed = std::env::var("CFPARSE_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64)
        });
    let mutations = std::env::var("CFPARSE_FUZZ_MUTATIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8);
    println!(
        "== mutation fuzz == seed {seed} (CFPARSE_FUZZ_SEED), {mutations} mutations per source"
    );

    let mut total = MutationCounts::default();
    for (name, sources) in sets() {
        let watchdog = watchdog.clone();
        let counts = std::thread::Builder::new()
            .name(format!("mutate {name}"))
            .stack_size(common::SMALL_STACK)
            .spawn(move || mutate(&sources, seed, mutations, &watchdog, trace))
            .unwrap()
            .join()
            .unwrap();
        println!(
            "== {name} == {} files, {} cases, {} parse panics, {} format panics, {} boundary failures",
            counts.files,
            counts.cases,
            counts.parse_panics.len(),
            counts.format_panics.len(),
            counts.boundary_failures.len()
        );
        total.files += counts.files;
        total.cases += counts.cases;
        total.parse_panics.extend(counts.parse_panics);
        total.format_panics.extend(counts.format_panics);
        total.boundary_failures.extend(counts.boundary_failures);
    }
    println!(
        "== mutation fuzz == {} files, {} cases, {} parse panics, {} format panics, \
         {} boundary failures, 0 aborts (seed {seed})",
        total.files,
        total.cases,
        total.parse_panics.len(),
        total.format_panics.len(),
        total.boundary_failures.len()
    );
    for p in total
        .parse_panics
        .iter()
        .chain(&total.format_panics)
        .chain(&total.boundary_failures)
        .take(10)
    {
        println!("  {p}");
    }
    assert!(
        total.parse_panics.is_empty()
            && total.format_panics.is_empty()
            && total.boundary_failures.is_empty(),
        "a mutated source failed (seed {seed})"
    );
}

/// xorshift64* (no dependency): a seeded stream of `u64`s.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// `0..n` (`n > 0`).
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// FNV-1a, to give each file its own stream from the run's seed: a failure
/// replays with the same seed whatever else is in the run.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

const INSERTS: &[&str] = &[
    "é", "💩", "\u{301}", "\"", "'", "/*", "*/", "<!---", "--->", "#", "(", ")", "{", "}",
];

/// One random edit of `source`: the mutated text, what was done, and where.
fn mutation(source: &str, rng: &mut Rng) -> (String, String, usize) {
    let mut at = rng.below(source.len() + 1);
    while !source.is_char_boundary(at) {
        at -= 1;
    }
    let pick = rng.below(INSERTS.len() + 2);
    let mut text = source.to_string();
    if pick >= INSERTS.len() && at < source.len() {
        let c = source[at..].chars().next().unwrap();
        text.replace_range(at..at + c.len_utf8(), "");
        return (text, format!("delete {c:?} at {at}"), at);
    }
    let insert = INSERTS[pick % INSERTS.len()];
    text.insert_str(at, insert);
    (text, format!("insert {insert:?} at {at}"), at)
}

/// The line of `text` holding byte `at`, for a failure message.
fn line_at(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let number = text[..at].matches('\n').count() + 1;
    format!("line {number}: {:?}", &text[start..end])
}

/// Every source, `mutations` times: parsed as its mode and as script (both
/// trees must tile the source on character boundaries), then formatted.
fn mutate(
    sources: &[Source],
    seed: u64,
    mutations: usize,
    watchdog: &Watchdog,
    trace: bool,
) -> MutationCounts {
    let mut counts = MutationCounts::default();
    let opts = cfformat::Options::default();
    let islands = cfformat::Islands::new();
    let ctx = cfformat::FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    for (name, source, mode) in sources {
        counts.files += 1;
        let mut rng = Rng::new(seed ^ fnv1a(name));
        for _ in 0..mutations {
            let (text, what, at) = mutation(source, &mut rng);
            counts.cases += 1;
            let case = format!("{name} (seed {seed}, {what})");
            if trace {
                eprintln!("{case}");
                let _ = std::io::stderr().flush();
            }
            let failure = |panic: &Box<dyn std::any::Any + Send>| {
                format!(
                    "{case}: {}\n    {}",
                    panic_message(panic),
                    line_at(&text, at)
                )
            };
            watchdog.begin(0, &case);
            let trees = catch_unwind(AssertUnwindSafe(|| {
                (
                    parse_source(&text, *mode),
                    parse_source(&text, Mode::Script),
                )
            }));
            match trees {
                Err(panic) => counts
                    .parse_panics
                    .push(format!("parse: {}", failure(&panic))),
                Ok((own, script)) => {
                    if let Err(panic) = catch_unwind(AssertUnwindSafe(|| {
                        common::assert_covers_source(name, &own);
                        common::assert_covers_source(name, &script);
                    })) {
                        counts
                            .boundary_failures
                            .push(format!("tiling: {}", failure(&panic)));
                    }
                }
            }
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| {
                let _ = cfformat::format_with(&text, *mode, &opts, &ctx);
            })) {
                counts
                    .format_panics
                    .push(format!("format: {}", failure(&panic)));
            }
            watchdog.end(0);
        }
    }
    counts
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = panic.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic>".into()
}

/// `../commandbox-cfformat` plus every directory in `CFPARSE_CORPUS`.
fn corpora() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Some(dir) = common::commandbox_dir() {
        out.push(("../commandbox-cfformat".into(), dir));
    }
    if let Some(list) = std::env::var_os("CFPARSE_CORPUS") {
        for part in list.to_string_lossy().split(':').filter(|s| !s.is_empty()) {
            let path = PathBuf::from(part);
            if path.is_dir() {
                out.push((part.to_string(), path));
            } else {
                eprintln!("CFPARSE_CORPUS: {part} is not a directory");
            }
        }
    }
    out
}

/// Every `*.<ext>` under `dir`, symlinks and `.git` skipped.
fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            collect(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}
