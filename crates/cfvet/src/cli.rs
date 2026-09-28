//! The command line: paths in, reports out.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cfcli::{io_message, Dedupe, GitChanges, Input};
use cfparse::Mode;
use cfvet::{lint_source, Lint};
use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::Parser;

const AFTER_HELP: &str = "\
A `cfvet-ignore` comment silences the reports on its own line and on the
line after it.

`--git staged` checks the staged .cfc, .cfs and .cfm files of the whole
repository; `unstaged` the files changed since the index and the untracked
ones; `all` both.

Exit codes:
  0  no reports
  1  at least one report
  2  a usage error, a missing or unreadable path, a file that is not UTF-8,
     a failed write to stdout, git failing under --git (2 wins over 1)";

/// Two checks for CFML: missing-var and unevaluated-call.
// One line of doc on purpose: a second paragraph would print as `--help`'s
// long description. Nothing at all prints the help and exits 2.
#[derive(Debug, Parser)]
#[command(
    name = "cfvet",
    version,
    after_help = AFTER_HELP,
    arg_required_else_help = true
)]
struct Cli {
    /// Files or directories (walked for .cfc, .cfs and .cfm, honouring
    /// .gitignore); `-` reads stdin
    #[arg(value_name = "PATHS")]
    paths: Vec<PathBuf>,
    /// The files git reports as changed in the whole repository, .cfc, .cfs
    /// and .cfm only
    #[arg(
        long,
        value_name = "WHICH",
        value_parser = PossibleValuesParser::new(GitChanges::NAMES)
            .map(|name| name.parse::<GitChanges>().expect("one of GitChanges::NAMES")),
        conflicts_with = "paths"
    )]
    git: Option<GitChanges>,
    /// Parse as CFScript, whatever the file's name (by default a .cfs is
    /// script, a .cfm tags, and anything else is read from the source)
    #[arg(long, conflicts_with = "tags")]
    script: bool,
    /// Parse as a tag template, whatever the file's name
    #[arg(long)]
    tags: bool,
    /// Print the reports only: no notes, no summary
    #[arg(short, long)]
    quiet: bool,
}

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let mode = if cli.script {
        Mode::Script
    } else if cli.tags {
        Mode::Tags
    } else {
        Mode::Auto
    };
    let mut failed = false;
    let mut error = |message: String| {
        eprintln!("error: {message}");
        failed = true;
    };
    let inputs = match cli.git {
        // The files git lists, kept by the walk's extensions; git's failure
        // (not a repository, not on PATH) ends the run like a usage error.
        Some(which) => match std::env::current_dir()
            .map_err(|e| format!("the current directory: {}", io_message(&e)))
            .and_then(|cwd| cfcli::git_changes(which, &cwd))
        {
            Ok(files) => files.into_iter().filter(|f| is_cfml(f)).collect(),
            Err(e) => {
                eprintln!("error: --git: {e}");
                return ExitCode::from(2);
            }
        },
        None => resolve(&cli.paths, &mut error),
    };
    let (mut reports, mut files, mut functions) = (Vec::new(), 0, 0);
    for path in &inputs {
        let input = Input::positional(path);
        let name = input.name();
        // By name where it is certain (`.cfs` script, `.cfm` tags), as for
        // `cfformat`; the flags win.
        let mode = mode.or_for_path(match input {
            Input::File(p) => Some(p),
            Input::Stdin => None,
        });
        let src = match input.read_utf8() {
            Ok(src) => src,
            Err(e) => {
                error(format!("{name}: {}", io_message(&e)));
                continue;
            }
        };
        let Lint {
            reports: found,
            notes,
            functions: checked,
        } = lint_source(&src, mode);
        if !cli.quiet {
            for note in &notes {
                eprintln!("{}", note.render(&name));
            }
        }
        files += 1;
        functions += checked;
        reports.extend(found.into_iter().map(|r| (name.clone(), r)));
    }
    reports.sort_by(|(a, r), (b, s)| (a, r.line, r.column).cmp(&(b, s.line, s.column)));
    let written = (|| {
        let mut out = std::io::stdout().lock();
        for (name, report) in &reports {
            writeln!(out, "{}", report.render(name))?;
        }
        out.flush()
    })();
    // A failed write ends the run: the reports are what a script reads, so
    // the summary would mislead.
    if let Err(e) = written {
        eprintln!("error: stdout: {}", io_message(&e));
        return ExitCode::from(2);
    }
    if !cli.quiet {
        eprintln!(
            "{} in {} ({} checked)",
            count(reports.len(), "report"),
            count(files, "file"),
            count(functions, "function")
        );
    }
    ExitCode::from(if failed {
        2
    } else if reports.is_empty() {
        0
    } else {
        1
    })
}

/// `1 report`, `2 reports`.
fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// The inputs `paths` name, in order, each once: `-` is stdin (kept as
/// `-`, which [`Input::positional`] reads from stdin; no walked file is
/// called `-`), a file is taken whatever its extension, a directory is
/// walked for `.cfc`, `.cfs` and `.cfm` files (a template can declare
/// functions too).
fn resolve(paths: &[PathBuf], error: &mut dyn FnMut(String)) -> Vec<PathBuf> {
    let mut inputs = Vec::new();
    let mut seen = Dedupe::default();
    let mut add = |inputs: &mut Vec<PathBuf>, file: PathBuf| {
        if seen.first(&file) {
            inputs.push(file);
        }
    };
    let mut stdin = false;
    for path in paths {
        if path.as_os_str() == "-" {
            if !std::mem::replace(&mut stdin, true) {
                inputs.push(path.clone());
            }
            continue;
        }
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => {
                for entry in cfcli::walk(path) {
                    match entry {
                        Ok(file) if is_cfml(&file) => add(&mut inputs, file),
                        Ok(_) => {}
                        Err(e) => error(e),
                    }
                }
            }
            Ok(_) => add(&mut inputs, path.clone()),
            Err(e) => error(format!("{}: {}", path.display(), io_message(&e))),
        }
    }
    inputs
}

/// `.cfc`, `.cfs` or `.cfm`, in any case.
fn is_cfml(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["cfc", "cfs", "cfm"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}
