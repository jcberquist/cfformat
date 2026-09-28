//! The command line, parsed with `clap`'s derive API.

use std::path::PathBuf;

use cfcli::GitChanges;
use cfformat::Mode;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

const AFTER_HELP: &str = "\
Exit codes:
  0  nothing to change, or every change written
  1  --check / --diff: a file would change
  2  a usage error, an unreadable, non-UTF-8 or unwritable file, a failed
     write to stdout, a settings error, an internal error (2 wins over 1)

Git: `cfformat --git staged -w` formats the staged .cfc and .cfs files of
the whole repository (with --cfm, the .cfm files too); `unstaged` takes the
files changed since the index and the untracked ones, `all` both. It writes
the working tree's copy and never runs `git add`: a pre-commit hook adds
what it wrote. Any other list of files goes to --files-from.";

const ARRANGE_AFTER_HELP: &str = "\
Exit codes:
  0  nothing to change, or every change written
  1  --check / --diff: a file would change
  2  a usage error, an unreadable, non-UTF-8 or unwritable file, a failed
     write to stdout, an internal error (2 wins over 1)

Arranging never formats: run `cfformat arrange -w` and `cfformat -w` as two
steps, and commit each on its own.";

/// A CFML formatter.
// Formatting is the command itself: `cfformat [OPTIONS] [PATHS]...`. The
// introspection commands, `doc` and `settings`, and `arrange` are
// subcommands, and a subcommand takes none of the formatting flags
// (`args_conflicts_with_subcommands`). A path that happens to be called
// `doc`, `settings` or `arrange` is written `./doc`. Nothing at all prints the help
// and exits 2 (`arg_required_else_help`). The doc comment is one line on
// purpose: a second paragraph would print as `--help`'s long description.
#[derive(Debug, Parser)]
#[command(
    name = "cfformat",
    after_help = AFTER_HELP,
    args_conflicts_with_subcommands = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// The formatting inputs and flags.
    #[command(flatten)]
    pub format: FormatArgs,
    /// `doc`, `settings` or `arrange`; `None` formats.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the formatter's document for one file (debug)
    Doc(DocArgs),
    /// Print the effective settings for a file or directory as JSON
    Settings(SettingsArgs),
    /// Order a component's functions by access, then name; never formats
    #[command(after_help = ARRANGE_AFTER_HELP)]
    Arrange(ArrangeArgs),
}

/// The formatting inputs and flags: to stdout, in place (`-w`), or as a
/// check (`--check`, `--diff`).
#[derive(Debug, Args)]
pub struct FormatArgs {
    /// The inputs and output flags, shared with `arrange`.
    #[command(flatten)]
    pub input: InputArgs,
    /// Walk .cfm files as well as .cfc and .cfs in directories
    #[arg(long)]
    pub cfm: bool,
    /// A settings file applied over the discovered one
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// The parse, doc, print, island and total times of each file, then the
    /// run's summary, on stderr (with --quiet, the times alone)
    #[arg(long)]
    pub timing: bool,
    /// The mode and island flags, shared with `doc`.
    #[command(flatten)]
    pub engine: EngineArgs,
}

/// The inputs, the output mode and the run's flags that formatting and
/// `arrange` share. The help is formatting's; `arrange` rewords three of
/// them in [`parse`].
#[derive(Debug, Args)]
pub struct InputArgs {
    /// Files, directories (walked for .cfc and .cfs, honouring .gitignore)
    /// or globs; `-` reads stdin. A path named like a subcommand is written
    /// ./doc
    #[arg(value_name = "PATHS")]
    pub paths: Vec<PathBuf>,
    /// Write each file that changes in place
    #[arg(short = 'w', long, conflicts_with_all = ["check", "diff", "stdin", "stdin_filepath"])]
    pub write: bool,
    /// List the files that would change, one per line; exit 1 if any
    #[arg(long)]
    pub check: bool,
    /// Print a unified diff for each file that would change; exit 1 if any
    #[arg(long)]
    pub diff: bool,
    /// Read the source from stdin and print the result on stdout
    #[arg(long, conflicts_with_all = ["paths", "files_from"])]
    pub stdin: bool,
    /// Read stdin as if it were this file: its settings, its island
    /// configuration, its mode, its name in messages (the file need not
    /// exist)
    #[arg(long, value_name = "PATH", conflicts_with = "files_from")]
    pub stdin_filepath: Option<PathBuf>,
    /// Read the paths from a file (`-` for stdin), one per line or
    /// NUL-separated
    #[arg(long, value_name = "FILE|-")]
    pub files_from: Option<PathBuf>,
    /// The files git reports as changed in the whole repository, walked
    /// extensions only (--cfm applies); -w never runs `git add`
    #[arg(
        long,
        value_name = "WHICH",
        value_parser = git_changes(),
        conflicts_with_all = ["paths", "files_from", "stdin", "stdin_filepath"]
    )]
    pub git: Option<GitChanges>,
    /// Format N files at a time (default: the number of cores)
    #[arg(short = 'j', long = "jobs", value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub jobs: Option<u32>,
    /// Print only errors, the --check list and the --diff diffs
    #[arg(long)]
    pub quiet: bool,
}

/// `--no-islands`, `--script`, `--tags`.
#[derive(Debug, Clone, Copy, Args)]
pub struct EngineArgs {
    /// Print every <script> / <style> island verbatim, whatever islands.*
    /// says
    #[arg(long)]
    pub no_islands: bool,
    /// The mode flags, shared with `arrange`.
    #[command(flatten)]
    pub mode: ModeArgs,
}

impl EngineArgs {
    /// The requested parse mode: `Auto` unless `--script` or `--tags`
    /// (each file then resolves `Auto` by its name, `Mode::or_for_path`).
    pub fn mode(self) -> Mode {
        self.mode.mode()
    }
}

/// `--script`, `--tags`.
#[derive(Debug, Clone, Copy, Args)]
pub struct ModeArgs {
    /// Parse as CFScript, whatever the file's name (by default a .cfs is
    /// script, a .cfm tags, and anything else is read from the source)
    #[arg(long, conflicts_with = "tags")]
    pub script: bool,
    /// Parse as a tag template, whatever the file's name
    #[arg(long)]
    pub tags: bool,
}

impl ModeArgs {
    /// The requested parse mode: `Auto` unless `--script` or `--tags`.
    pub fn mode(self) -> Mode {
        if self.script {
            Mode::Script
        } else if self.tags {
            Mode::Tags
        } else {
            Mode::Auto
        }
    }
}

/// `cfformat doc`.
#[derive(Debug, Args)]
pub struct DocArgs {
    /// The file, or `-` for stdin
    #[arg(value_name = "FILE|-")]
    pub path: PathBuf,
    /// A settings file applied over the discovered one
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// The parse, doc, print, island and total times, on stderr
    #[arg(long)]
    pub timing: bool,
    /// The mode and island flags, shared with formatting.
    #[command(flatten)]
    pub engine: EngineArgs,
}

/// `cfformat arrange`: the bare command's inputs, output modes and exit
/// codes, without its settings, `--cfm`, islands or timing (arranging reads
/// no settings, and its walk skips templates).
#[derive(Debug, Args)]
pub struct ArrangeArgs {
    /// The inputs and output flags, shared with formatting.
    #[command(flatten)]
    pub input: InputArgs,
    /// Also sort properties by name (their order is visible to
    /// getMetadata, ORM mappings and serialisation)
    #[arg(long)]
    pub properties: bool,
    /// The mode flags, shared with formatting.
    #[command(flatten)]
    pub mode: ModeArgs,
}

/// `cfformat settings`.
#[derive(Debug, Args)]
pub struct SettingsArgs {
    /// A file or directory (`-` or nothing: the current directory); with
    /// --migrate, the settings file (`-`: stdin to stdout)
    #[arg(value_name = "PATH|-")]
    pub path: Option<PathBuf>,
    /// A settings file applied over the discovered one
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Print the defaults; read no settings file
    #[arg(long)]
    pub defaults: bool,
    /// Print a JSON Schema for .cfformat.json (ignores PATH and --config)
    #[arg(long)]
    pub schema: bool,
    /// Rewrite an old settings file (default ./.cfformat.json) with its
    /// removed and renamed keys migrated
    #[arg(long, conflicts_with_all = ["config", "defaults", "schema"])]
    pub migrate: bool,
}

/// `--git`'s values, listed by `--help`.
fn git_changes() -> impl TypedValueParser<Value = GitChanges> {
    PossibleValuesParser::new(GitChanges::NAMES)
        .map(|name| name.parse().expect("one of GitChanges::NAMES"))
}

/// Parses the process arguments; a usage error, `--help` and `--version`
/// exit here (clap: 2 on stderr, 0 on stdout). Usage lines say `cfformat`
/// whatever the executable is called (`cfformat_linux_x86_64`).
pub fn parse() -> Cli {
    let matches = Cli::command()
        .version(env!("CARGO_PKG_VERSION"))
        .bin_name("cfformat")
        // `InputArgs`' help is formatting's; arranging has no subcommands to
        // shadow, no settings or islands, and arranges.
        .mut_subcommand("arrange", |arrange| {
            arrange
                .mut_arg("paths", |a| {
                    a.help(
                        "Files, directories (walked for .cfc and .cfs, honouring .gitignore) or \
                         globs; `-` reads stdin",
                    )
                })
                .mut_arg("stdin_filepath", |a| {
                    a.help(
                        "Read stdin as if it were this file: its mode, its name in messages \
                         (the file need not exist)",
                    )
                })
                .mut_arg("git", |a| {
                    a.help(
                        "The files git reports as changed in the whole repository, .cfc and \
                         .cfs only; -w never runs `git add`",
                    )
                })
                .mut_arg("jobs", |a| {
                    a.help("Arrange N files at a time (default: the number of cores)")
                })
        })
        .get_matches();
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}
