//! What a run prints: each file's outcome, in the order the files
//! were given, then the summary; and the exit code.
//!
//! stdout carries only what a script consumes: the formatted text (one
//! file), the paths `--check` lists (one per line) or the `--diff` diffs.
//! Everything else goes to stderr.
//!
//! A write to stdout that fails is an error (`error: stdout: message`, exit
//! 2) and ends the output: nothing more is printed. The one exception is a
//! closed pipe under [`Output::Stdout`] (`cfformat a.cfc | head`): the
//! reader has what it wanted, so the run ends quietly with exit 0.

use std::collections::HashSet;
use std::fmt::Display;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use cfformat::options::Warning as SettingsWarning;
use cfformat::{IslandStats, Warning, WarningKind};

use cfcli::io_message;

/// What happens to the formatted text: exactly one of these per run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// One file: the text on stdout.
    Stdout,
    /// `cfformat DIR` without a mode flag: every file formatted, nothing written,
    /// a summary with the line count.
    Report,
    /// `-w`: a file that changes is written in place.
    Write,
    /// `--check`: the path of a file that would change on stdout; exit 1.
    Check,
    /// `--diff` (also with `--check`): a unified diff on stdout; exit 1.
    Diff,
}

/// What the run does to each file: the word its messages use for a region
/// left as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// The bare command.
    Format,
    /// `cfformat arrange`.
    Arrange,
}

impl Verb {
    /// `not formatted` / `not arranged`: a region the parse recovered in,
    /// left as written.
    pub fn not_done(self) -> &'static str {
        match self {
            Verb::Format => "not formatted",
            Verb::Arrange => "not arranged",
        }
    }
}

/// Why a file was not formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    /// It could not be read.
    Read(String),
    /// `-w` could not write it.
    Write(String),
    /// Formatting panicked: the panic's message.
    Panic(String),
}

/// Everything a file's run prints, collected by the worker and printed in
/// order by [`Printer::print`].
#[derive(Debug, Default)]
pub struct FileOutcome {
    /// The file as messages name it.
    pub name: String,
    /// The warnings of every settings file that applied.
    pub settings_warnings: Vec<(PathBuf, SettingsWarning)>,
    /// The warnings of the island configuration files (`.prettierrc`, …)
    /// read so far in the run ([`cfformat::Islands::config_warnings`]).
    pub config_warnings: Vec<(PathBuf, String)>,
    /// The settings could not be resolved (the message names the file).
    pub settings_error: Option<String>,
    /// What printed as written: islands the island formatter refused and
    /// regions the parse recovered in.
    pub warnings: Vec<Warning>,
    /// The file failed.
    pub error: Option<FileError>,
    /// Source lines, counted for the report ([`Output::Report`]) only.
    pub lines: usize,
    /// The output differs from the source (under `-w`: and was written).
    pub changed: bool,
    /// [`Output::Stdout`]: the formatted text.
    pub text: Option<String>,
    /// [`Output::Diff`]: the unified diff of a file that would change.
    pub diff: Option<String>,
    /// The `--timing` line.
    pub timing: Option<String>,
}

/// The settings and island configuration files whose warnings were printed:
/// each file warns once per run, whichever command reads it.
#[derive(Debug, Default)]
pub struct Warned(HashSet<PathBuf>);

impl Warned {
    /// `warning: <file>: <message>` for each warning of a file not seen
    /// before (all of that file's warnings in `warnings`), unless `quiet`;
    /// every file in `warnings` is seen afterwards.
    pub fn print(&mut self, warnings: &[(PathBuf, impl Display)], quiet: bool) {
        for (file, message) in warnings {
            if !quiet && !self.0.contains(file) {
                eprintln!("warning: {}: {message}", file.display());
            }
        }
        self.0.extend(warnings.iter().map(|(f, _)| f.clone()));
    }
}

/// Whether stdout still takes output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stdout {
    Open,
    /// The reader of [`Output::Stdout`]'s text went away: quiet, exit 0.
    Closed,
    /// A write failed and was reported: exit 2.
    Failed,
}

/// Prints outcomes and keeps the counts.
#[derive(Debug)]
pub struct Printer {
    output: Output,
    verb: Verb,
    stdout: Stdout,
    quiet: bool,
    /// Settings and island configuration files whose warnings were printed.
    warned: Warned,
    /// Settings errors already printed.
    settings_errors: HashSet<String>,
    files: usize,
    lines: usize,
    changed: usize,
    failed: usize,
    /// Errors outside any file (a missing PATH, a glob matching nothing).
    input_errors: usize,
    /// Regions printed as written because the parse recovered in them.
    recovered: usize,
}

impl Printer {
    /// A printer for `output`; `quiet` drops the summary and the warnings.
    pub fn new(output: Output, verb: Verb, quiet: bool) -> Self {
        Printer {
            output,
            verb,
            stdout: Stdout::Open,
            quiet,
            warned: Warned::default(),
            settings_errors: HashSet::new(),
            files: 0,
            lines: 0,
            changed: 0,
            failed: 0,
            input_errors: 0,
            recovered: 0,
        }
    }

    /// `error: message` for a problem outside any file; the exit code is 2.
    pub fn input_error(&mut self, message: &str) {
        eprintln!("error: {message}");
        self.input_errors += 1;
    }

    /// Prints one file's outcome; false once stdout takes nothing more (a
    /// write failed, or the reader went away), after which the caller
    /// stops printing.
    pub fn print(&mut self, o: FileOutcome) -> bool {
        self.files += 1;
        self.lines += o.lines;
        self.warned.print(&o.settings_warnings, self.quiet);
        self.warned.print(&o.config_warnings, self.quiet);
        if let Some(e) = o.settings_error {
            self.failed += 1;
            if self.settings_errors.insert(e.clone()) {
                eprintln!("error: {e}");
            }
            return true;
        }
        self.recovered += o
            .warnings
            .iter()
            .filter(|w| matches!(w.kind, WarningKind::Recovered(_)))
            .count();
        if !self.quiet {
            for w in &o.warnings {
                let label = match w.kind {
                    WarningKind::Recovered(_) => self.verb.not_done(),
                    WarningKind::Island { .. } => w.label(),
                };
                eprintln!("{}:{}: {label}: {}", o.name, w.line, w.message);
            }
        }
        if let Some(e) = o.error {
            self.failed += 1;
            eprintln!("{}", self.error_line(&o.name, e));
            return true;
        }
        self.changed += usize::from(o.changed);
        let written = (|| {
            let mut stdout = io::stdout().lock();
            if let Some(text) = o.text {
                stdout.write_all(text.as_bytes())?;
            }
            if self.output == Output::Check && o.changed {
                writeln!(stdout, "{}", o.name)?;
            }
            if let Some(diff) = o.diff {
                stdout.write_all(diff.as_bytes())?;
            }
            stdout.flush()
        })();
        if let Err(e) = written {
            self.stdout = if stdout_failed(&e, self.output == Output::Stdout) {
                Stdout::Closed
            } else {
                Stdout::Failed
            };
            return false;
        }
        if let Some(t) = o.timing {
            eprintln!("{t}");
        }
        true
    }

    /// `path: message`; one file on stdout reads `error: path: message`.
    pub fn error_line(&self, name: &str, e: FileError) -> String {
        let standalone = self.output == Output::Stdout;
        match e {
            FileError::Panic(m) if standalone => format!("error: {name}: internal error: {m}"),
            FileError::Panic(m) => format!("{name}: internal error: {m}"),
            FileError::Read(m) | FileError::Write(m) if standalone => {
                format!("error: {name}: {m}")
            }
            FileError::Read(m) | FileError::Write(m) => {
                format!("{name}: {m}")
            }
        }
    }

    /// The summary and the `islands:` line on stderr, when `summary` (a run
    /// over several files or a walked input) or `timing` (`--timing`: one
    /// file and stdin too), and not quiet. The report always has one:
    ///
    /// ```text
    /// 12 files, 3 written, 0 failed, 0.02 s                  (-w)
    /// 12 files, 3 would change, 0 failed, 0.02 s             (--check, --diff)
    /// 12 files, 840 lines, 3 would change, 0 failed, 0.02 s  (cfformat DIR)
    /// islands: 4 formatted, 0 cached, 0 warnings, 0.120s     (an island handed off, or --timing)
    /// ```
    ///
    /// When the parse recovered in any file, the first line gains
    /// `, R not formatted` (`not arranged` under `arrange`) before the time:
    /// the regions the parse did not understand, left as written. They
    /// never change the exit code.
    pub fn summary(&self, summary: bool, elapsed: Duration, islands: IslandStats, timing: bool) {
        let summary = timing
            || match self.output {
                Output::Stdout => false,
                Output::Report => true,
                Output::Write | Output::Check | Output::Diff => summary,
            };
        if self.quiet || !summary || self.stdout != Stdout::Open {
            return;
        }
        let changed = match self.output {
            Output::Write => format!("{} written", self.changed),
            _ => format!("{} would change", self.changed),
        };
        let lines = match self.output {
            Output::Report => format!("{} lines, ", self.lines),
            _ => String::new(),
        };
        let recovered = match self.recovered {
            0 => String::new(),
            n => format!(", {n} {}", self.verb.not_done()),
        };
        eprintln!(
            "{} files, {lines}{changed}, {} failed{recovered}, {:.2} s",
            self.files,
            self.failed,
            elapsed.as_secs_f64()
        );
        if timing || islands.islands() > 0 {
            eprintln!(
                "islands: {} formatted, {} cached, {} warnings, {:.3}s",
                islands.formatted,
                islands.cached,
                islands.warnings,
                islands.time.as_secs_f64()
            );
        }
    }

    /// 2 when anything failed (stdout included); else 1 when `--check` /
    /// `--diff` found a change; else 0.
    pub fn exit_code(&self) -> u8 {
        if self.failed > 0 || self.input_errors > 0 || self.stdout == Stdout::Failed {
            2
        } else if self.changed > 0 && matches!(self.output, Output::Check | Output::Diff) {
            1
        } else {
            0
        }
    }
}

/// Handles a failed write to stdout: a closed pipe when `pager` (the
/// formatted text of one file, or `doc`'s) is quiet and returns true; any
/// other failure prints `error: stdout: message` and returns false.
pub fn stdout_failed(e: &io::Error, pager: bool) -> bool {
    if pager && e.kind() == io::ErrorKind::BrokenPipe {
        return true;
    }
    eprintln!("error: stdout: {}", io_message(e));
    false
}
