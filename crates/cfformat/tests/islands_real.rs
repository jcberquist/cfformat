//! Island formatting over real files with `"oxc"`, the only island
//! formatter: in process, on every platform, with nothing on `PATH`. It
//! checks idempotence and the invariants (token preservation, reparse,
//! width as a reported count, no trailing whitespace; a file that
//! does not parse is skipped; a line of an island that keeps its literal
//! text may keep trailing whitespace) and literal preservation
//! (`common::check_literals`), and prints the counts, every warning and the
//! time. Two sweeps:
//!
//! - every case of the island fixtures (`islandScript`, `islandModule`,
//!   `islandJson`, `islandStyle`, `islandTemplateLiteral`, `islandWidth`,
//!   `islandLdJson`, `islandLiteralNested`, `islandDynamicType`,
//!   `islandVerbatimLiterals`, `islandInterpolated`) with its own settings (`tab_indent`,
//!   `max_columns`, an `islands.*` key `"off"`);
//! - every island fixture's source and every `../commandbox-cfformat` file
//!   holding a `<script>` or `<style>`, with the default options.
//!
//! Ignored by default (it reads `../commandbox-cfformat`):
//!
//! ```text
//! cargo test -p cfformat --test islands_real -- --ignored --nocapture
//! ```

mod common;

use std::path::{Path, PathBuf};

use cfformat::{format_with, FormatCtx, Islands, Options};
use cfparse::Mode;

/// The fixtures whose cases run with their own settings.
const FIXTURES: &[&str] = &[
    "islandScript",
    "islandModule",
    "islandJson",
    "islandStyle",
    "islandTemplateLiteral",
    "islandWidth",
    "islandLdJson",
    "islandLiteralNested",
    "islandDynamicType",
    "islandVerbatimLiterals",
    "islandInterpolated",
];

fn sources() -> Vec<(PathBuf, String, Mode)> {
    let mut out: Vec<(PathBuf, String, Mode)> = common::fixtures()
        .into_iter()
        .filter(|f| f.name.starts_with("island") || f.name == "tagHTMLScriptIndent")
        .map(|f| (f.path, f.source, f.mode))
        .collect();
    out.extend(commandbox_sources());
    out
}

fn commandbox_sources() -> Vec<(PathBuf, String, Mode)> {
    let mut out = Vec::new();
    if let Some(dir) = common::commandbox_dir() {
        let mut files = Vec::new();
        walk(&dir, &mut files);
        files.sort();
        for path in files {
            let src = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
            let lower = src.to_ascii_lowercase();
            if lower.contains("<script") || lower.contains("<style") {
                out.push((path, src, Mode::Auto));
            }
        }
    }
    out
}

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
        } else if path.extension().is_some_and(|e| e == "cfc" || e == "cfm") {
            out.push(path);
        }
    }
}

/// One sweep: counts and failures.
#[derive(Default)]
struct Sweep {
    files: usize,
    wide: usize,
    /// JavaScript islands whose literals were compared.
    literal_islands: usize,
    /// Trailing-whitespace lines inside an island that keeps its literal
    /// text.
    exempt: usize,
    failures: Vec<String>,
}

impl Sweep {
    /// Formats `src` twice with `opts` and checks the output.
    fn check(
        &mut self,
        label: &str,
        path: &Path,
        src: &str,
        mode: Mode,
        opts: &Options,
        islands: &Islands,
    ) {
        let name = format!("{label}: {}", path.display());
        let ctx = FormatCtx {
            path: Some(path),
            islands: Some(islands),
        };
        let once = format_with(src, mode, opts, &ctx);
        self.files += 1;
        for w in &once.warnings {
            eprintln!(
                "islands_real: {name}:{}: {}: {}",
                w.line,
                w.label(),
                w.message
            );
        }
        let twice = format_with(&once.text, mode, opts, &ctx);
        if twice.text != once.text {
            let diff = similar::TextDiff::from_lines(&once.text, &twice.text);
            self.failures.push(format!(
                "{name}: not idempotent\n{}",
                diff.unified_diff()
                    .context_radius(2)
                    .header("once", "twice")
            ));
        }
        for p in common::check_output(src, &once.text, mode, opts) {
            self.failures.push(format!("{name}: {p}"));
        }
        let (islands, problems) = common::check_literals(src, &once.text, mode, opts);
        self.literal_islands += islands;
        self.failures
            .extend(problems.into_iter().map(|p| format!("{name}: {p}")));
        let exempt = common::literal_island_lines(&once.text, mode);
        for (n, line) in once.text.lines().enumerate() {
            if line.ends_with([' ', '\t']) {
                if exempt.contains(&(n + 1)) {
                    self.exempt += 1;
                } else {
                    self.failures
                        .push(format!("{name}:{}: trailing whitespace", n + 1));
                }
            }
            if line.chars().count() > opts.max_columns {
                self.wide += 1;
            }
        }
        if !once.text.is_empty() && !once.text.ends_with('\n') {
            self.failures.push(format!("{name}: no trailing newline"));
        }
    }
}

#[test]
#[ignore]
fn real_island_formatters() {
    let tool = "oxc";
    eprintln!("islands_real: {tool}: in process (oxfmt_v0.72.0)");
    let islands = Islands::new();
    let started = std::time::Instant::now();

    // The fixtures' cases, with their own settings.
    let mut cases = Sweep::default();
    for fixture in common::fixtures() {
        if !FIXTURES.contains(&fixture.name.as_str()) {
            continue;
        }
        let path = fixture.path.clone();
        for (i, case) in fixture.cases.iter().enumerate() {
            cases.check(
                &format!("{tool}: {}[{i}]", fixture.name),
                &path,
                &fixture.source,
                fixture.mode,
                &case.options,
                &islands,
            );
        }
    }
    let fixture_stats = islands.stats();

    // The sources, with the defaults.
    let (opts, _) = Options::from_json(r#"{"newline": "\n"}"#).unwrap();
    let mut files = Sweep::default();
    let mut commandbox = 0;
    let fixture_sources = sources().len() - commandbox_sources().len();
    for (i, (path, src, mode)) in sources().into_iter().enumerate() {
        commandbox += usize::from(i >= fixture_sources);
        files.check(tool, &path, &src, mode, &opts, &islands);
    }
    let stats = islands.stats();
    let failures: Vec<String> = cases.failures.into_iter().chain(files.failures).collect();
    eprintln!(
        "islands_real: {tool}: {} fixture cases with their own settings ({} islands, {} warnings, {} lines over max_columns); {} sources with the defaults ({} island fixtures, {commandbox} commandbox files, {} lines over max_columns); literals preserved in {} JavaScript islands, {} trailing-whitespace lines exempted; islands: {} formatted, {} cached, {} warnings, {:.2} s in the tool, {:.2} s total; {} failures",
        cases.files,
        fixture_stats.islands(),
        fixture_stats.warnings,
        cases.wide,
        files.files,
        fixture_sources,
        files.wide,
        cases.literal_islands + files.literal_islands,
        cases.exempt + files.exempt,
        stats.formatted,
        stats.cached,
        stats.warnings,
        stats.time.as_secs_f64(),
        started.elapsed().as_secs_f64(),
        failures.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
