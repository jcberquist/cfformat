//! The parity oracle for `"oxc"`: every file is formatted twice with
//! `format_with`, the file's real path and the default options,
//! once as cfformat does (`"oxc"`, one `Islands` for the whole run) and once
//! with every island handed to the prettier CLI instead
//! ([`Islands::with_formatter`] and this file's [`PrettierCli`], one
//! `Islands` per directory, since prettier's configuration is the
//! directory's), and the two outputs are compared byte for byte. The printer
//! builds both sides' requests (the synthetic path, cfformat's indent, the
//! width budget), so only the formatter differs. A file that holds no
//! `<script` or `<style` (any case) cannot differ and is not formatted.
//!
//! The corpora, each reported on its own line as `N files, I islands (oxc) /
//! J islands (prettier), D different, Ro / Rp refused`: the island fixtures
//! (and `tagHTMLScriptIndent`; not `islandConfigOxfmt`, whose `.oxfmtrc.jsonc`
//! prettier does not read), `../commandbox-cfformat`, and the directories of
//! `CFFORMAT_CORPUS` (a path list; no other corpus when unset). `.cfm` parses
//! as tags, `.cfc` as `Auto`. The first ten differences print as unified diffs, with each side's
//! warnings for that file; `CFFORMAT_PARITY_OUT=<file>` writes all of them.
//!
//! The test spawns prettier itself (cfformat spawns no formatter): the
//! island on stdin, cfformat's layout as prettier's options
//! ([`PrettierCli`]). The prettier CLI runs in the file's directory and
//! resolves the project's configuration from there (`.prettierrc`,
//! `.editorconfig`, plugins, `.prettierignore` / `.gitignore`); `"oxc"`
//! reads the `.prettierrc` too (`islands.config`) but not the rest. With
//! `CFFORMAT_PARITY_CLASSIFY=1` every differing file is formatted a third
//! time, by the prettier CLI with
//! `--no-config --no-editorconfig` and no ignore file, and counted as a
//! *configuration* difference when that output is oxc's (a configuration
//! effect oxc did not reproduce); the rest are printed first, as
//! `unexplained`.
//! **Fails when a fixture or `../commandbox-cfformat` file differs**; the
//! other corpora's counts are reported only.
//!
//! Ignored by default; skipped with a message when `prettier` is not on
//! `PATH`. The prettier side runs one CLI per island, sequentially (≈ 245 ms
//! each): minutes over a directory of real projects, and classification
//! adds a CLI run per differing file:
//!
//! ```text
//! cargo test --release -p cfformat --test islands_parity -- --ignored --nocapture
//! ```

mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use cfformat::islands::{IslandFormatter, IslandRequest, Refused};
use cfformat::{format_with, FormatCtx, Formatted, IndentStyle, IslandStats, Islands, Options};
use cfparse::Mode;

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|dir| dir.join(tool).is_file()))
}

/// The prettier CLI on one island: the text on stdin, in the directory of
/// the file the synthetic path stands next to, with `extra` before
/// cfformat's layout as prettier's options:
/// `prettier --stdin-filepath {path} --use-tabs={use_tabs} --tab-width
/// {indent_width} --print-width {width}`. A non-zero exit or empty output is
/// a refusal naming stderr's first line; stderr with exit 0 (prettier's
/// `[warn] Ignored unknown option …`) is not.
struct PrettierCli {
    extra: Vec<String>,
}

impl IslandFormatter for PrettierCli {
    fn format(&self, req: &IslandRequest) -> Result<String, Refused> {
        let (tabs, width) = match req.indent {
            IndentStyle::Tabs(n) => (true, n),
            IndentStyle::Spaces(n) => (false, n),
        };
        let mut cmd = Command::new("prettier");
        cmd.args(&self.extra)
            .arg("--stdin-filepath")
            .arg(&req.path)
            .arg(format!("--use-tabs={tabs}"))
            .args(["--tab-width", &width.to_string()])
            .args(["--print-width", &req.width.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = req.path.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().expect("prettier is on PATH");
        let mut stdin = child.stdin.take().unwrap();
        let text = req.text.to_owned();
        let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
        let out = child.wait_with_output().unwrap();
        let _ = writer.join();
        let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
        if out.status.success() && !stdout.trim().is_empty() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(Refused(
            stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map_or_else(|| format!("exited with {}", out.status), str::to_owned),
        ))
    }
}

/// One side of the comparison: the options and island formatting.
struct Side {
    opts: Options,
    /// `None`: cfformat's own (`"oxc"`), one `Islands` for the run.
    prettier: Option<Arc<PrettierCli>>,
    /// The run's `Islands`, keyed by directory for the prettier sides.
    islands: HashMap<PathBuf, Islands>,
}

impl Side {
    fn new(prettier: Option<PrettierCli>) -> Self {
        Side {
            opts: Options::from_json(r#"{"newline": "\n"}"#).unwrap().0,
            prettier: prettier.map(Arc::new),
            islands: HashMap::new(),
        }
    }

    fn format(&mut self, path: &Path, src: &str, mode: Mode) -> Formatted {
        let dir = match self.prettier {
            Some(_) => path.parent().map(PathBuf::from).unwrap_or_default(),
            None => PathBuf::new(),
        };
        let prettier = self.prettier.clone();
        let islands = self.islands.entry(dir).or_insert_with(|| match prettier {
            Some(p) => Islands::with_formatter(p),
            None => Islands::new(),
        });
        let ctx = FormatCtx {
            path: Some(path),
            islands: Some(islands),
        };
        format_with(src, mode, &self.opts, &ctx)
    }

    /// Requests and refusals so far, over every `Islands`.
    fn stats(&self) -> (usize, usize) {
        self.islands
            .values()
            .map(Islands::stats)
            .fold((0, 0), |(n, w), s: IslandStats| {
                (n + s.islands(), w + s.warnings)
            })
    }
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

/// The files of `dir` holding a `<script` or `<style`, with their mode.
fn corpus(dir: &Path) -> Vec<(PathBuf, String, Mode)> {
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let src = String::from_utf8_lossy(&std::fs::read(&path).ok()?).into_owned();
            let lower = src.to_ascii_lowercase();
            if !lower.contains("<script") && !lower.contains("<style") {
                return None;
            }
            let mode = Mode::for_path(&path);
            Some((path, src, mode))
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    files: usize,
    different: usize,
    configuration: usize,
}

struct Run {
    oxc: Side,
    prettier: Side,
    /// The prettier CLI without the project's configuration, when classifying.
    bare: Option<Side>,
    diffs: Vec<String>,
}

/// The prettier CLI ignoring every configuration and ignore file (`ignore`
/// names a file that does not exist).
fn bare_prettier(ignore: &Path) -> PrettierCli {
    PrettierCli {
        extra: vec![
            "--no-config".into(),
            "--no-editorconfig".into(),
            format!("--ignore-path={}", ignore.display()),
        ],
    }
}

impl Run {
    fn corpus(&mut self, name: &str, files: &[(PathBuf, String, Mode)]) -> Tally {
        let (o0, p0) = (self.oxc.stats(), self.prettier.stats());
        let mut tally = Tally::default();
        for (path, src, mode) in files {
            let a = self.oxc.format(path, src, *mode);
            let b = self.prettier.format(path, src, *mode);
            tally.files += 1;
            if a.text == b.text {
                continue;
            }
            tally.different += 1;
            if let Some(bare) = &mut self.bare {
                if bare.format(path, src, *mode).text == a.text {
                    tally.configuration += 1;
                    self.diffs
                        .push(format!("{name}: configuration: {}\n", path.display()));
                    continue;
                }
            }
            let mut report = format!("{name}: unexplained: {}\n", path.display());
            for (side, f) in [("oxc", &a), ("prettier", &b)] {
                for w in &f.warnings {
                    let _ = writeln!(report, "  {side} warning: line {}: {}", w.line, w.message);
                }
            }
            let diff = similar::TextDiff::from_lines(&b.text, &a.text);
            let _ = write!(
                report,
                "{}",
                diff.unified_diff()
                    .context_radius(2)
                    .header("prettier", "oxc")
            );
            self.diffs.push(report);
        }
        let (o, p) = (self.oxc.stats(), self.prettier.stats());
        let classified = match self.bare {
            Some(_) => format!(
                " ({} configuration, {} unexplained)",
                tally.configuration,
                tally.different - tally.configuration
            ),
            None => String::new(),
        };
        eprintln!(
            "islands_parity: {name}: {} files, {} islands (oxc) / {} islands (prettier), {} different{classified}, {} / {} refused",
            tally.files,
            o.0 - o0.0,
            p.0 - p0.0,
            tally.different,
            o.1 - o0.1,
            p.1 - p0.1,
        );
        tally
    }
}

#[test]
#[ignore]
fn oxc_matches_prettier() {
    if !on_path("prettier") {
        eprintln!("islands_parity: `prettier` is not on PATH, skipped");
        return;
    }
    let no_ignore = std::env::temp_dir().join("cfformat-parity-no-such-ignore-file");
    let mut run = Run {
        oxc: Side::new(None),
        prettier: Side::new(Some(PrettierCli { extra: Vec::new() })),
        bare: std::env::var_os("CFFORMAT_PARITY_CLASSIFY")
            .map(|_| Side::new(Some(bare_prettier(&no_ignore)))),
        diffs: Vec::new(),
    };

    let fixtures: Vec<(PathBuf, String, Mode)> = common::fixtures()
        .into_iter()
        .filter(|f| f.name.starts_with("island") || f.name == "tagHTMLScriptIndent")
        // prettier reads the `.prettierrc` there, oxc the `.oxfmtrc.jsonc`.
        .filter(|f| f.name != "islandConfigOxfmt")
        .map(|f| (f.path, f.source, f.mode))
        .collect();
    let mut must_match = run.corpus("fixtures", &fixtures).different;
    if let Some(dir) = common::commandbox_dir() {
        must_match += run.corpus("commandbox-cfformat", &corpus(&dir)).different;
    }
    let corpora = std::env::var_os("CFFORMAT_CORPUS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();
    for dir in corpora.iter().filter(|d| d.is_dir()) {
        run.corpus(&dir.display().to_string(), &corpus(dir));
    }

    // The unexplained differences first.
    run.diffs.sort_by_key(|d| {
        d.lines()
            .next()
            .is_some_and(|l| l.contains(": configuration: "))
    });
    for d in run.diffs.iter().take(10) {
        eprintln!("{d}");
    }
    if let Some(out) = std::env::var_os("CFFORMAT_PARITY_OUT") {
        std::fs::write(out, run.diffs.join("\n")).unwrap();
    }
    assert_eq!(
        must_match, 0,
        "a fixture or ../commandbox-cfformat file formats differently under oxc and prettier"
    );
}
