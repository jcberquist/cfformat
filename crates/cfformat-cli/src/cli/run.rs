//! Running the command line: formatting, `arrange`, `doc` and `settings`.

use std::any::Any;
use std::collections::BTreeMap;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use cfcli::{io_message, Input, STDIN};
use cfformat::options::{Discovery, OptionsError, Resolved, SETTINGS_FILE};
use cfformat::{
    format_with, ArrangeOptions, FormatCtx, Formatted, IslandStats, Islands, Mode, Options,
};
use similar::TextDiff;

use super::args::{self, ArrangeArgs, Command, DocArgs, FormatArgs, InputArgs, SettingsArgs};
use super::output::{stdout_failed, FileError, FileOutcome, Output, Printer, Verb, Warned};
use super::{inputs, schema};

/// Runs the command line.
pub fn main() -> ExitCode {
    let cli = args::parse();
    match cli.command {
        None => fmt(&cli.format),
        Some(Command::Arrange(a)) => arrange(&a),
        Some(Command::Doc(a)) => doc(&a),
        Some(Command::Settings(a)) => settings(&a),
    }
}

/// `error: message`, exit 2.
fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::from(2)
}

/// Settings resolution for one run: discovery plus the files whose warnings
/// were already printed, so each file warns once per run.
struct Settings {
    discovery: Discovery,
    warned: Warned,
}

impl Settings {
    fn new(config: Option<&Path>) -> Self {
        Settings {
            discovery: Discovery::new(Discovery::home_file(), config.map(PathBuf::from)),
            warned: Warned::default(),
        }
    }

    /// The options for `path` (a file, a directory, or `-` for the current
    /// directory), printing the warnings of settings files not seen before.
    fn resolve(&mut self, path: &Path) -> Result<Resolved, String> {
        let resolved = if path == Path::new("-") {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            self.discovery.discover_in(&cwd)
        } else if path.is_dir() {
            self.discovery.discover_in(path)
        } else {
            self.discovery.discover(path)
        }
        .map_err(|e| e.to_string())?;
        self.warned.print(&resolved.warnings, false);
        Ok(resolved)
    }
}

/// `cfformat [OPTIONS] [PATHS]...`: to stdout, in place (`-w`), or as a
/// check (`--check`, `--diff`).
fn fmt(args: &FormatArgs) -> ExitCode {
    let discovery = Discovery::new(Discovery::home_file(), args.config.clone());
    let islands = Islands::new();
    let run = Run {
        input: &args.input,
        cfm: args.cfm,
        timing: args.timing,
        mode: args.engine.mode(),
    };
    run_files(
        &run,
        Task::Format {
            discovery: &discovery,
            islands: &islands,
            use_islands: !args.engine.no_islands,
        },
    )
}

/// `cfformat arrange [OPTIONS] [PATHS]...`: the bare command's inputs,
/// output modes and exit codes, with arranging in place of formatting. It
/// reads no settings, and its walk keeps `.cfc` and `.cfs` files (a `.cfs`
/// holds no component, so arranging one changes nothing).
fn arrange(args: &ArrangeArgs) -> ExitCode {
    let run = Run {
        input: &args.input,
        cfm: false,
        timing: false,
        mode: args.mode.mode(),
    };
    run_files(
        &run,
        Task::Arrange(ArrangeOptions {
            properties: args.properties,
        }),
    )
}

/// The inputs and output flags the bare command and `arrange` share.
struct Run<'a> {
    input: &'a InputArgs,
    /// `--cfm`: false for `arrange`.
    cfm: bool,
    /// `--timing`: false for `arrange`.
    timing: bool,
    mode: Mode,
}

/// What a run does to each file.
#[derive(Clone, Copy)]
enum Task<'a> {
    /// Discover the file's settings and format it.
    Format {
        discovery: &'a Discovery,
        islands: &'a Islands,
        /// False under `--no-islands`: every island prints as written.
        use_islands: bool,
    },
    /// Arrange it with these options.
    Arrange(ArrangeOptions),
}

impl Task<'_> {
    fn verb(self) -> Verb {
        match self {
            Task::Format { .. } => Verb::Format,
            Task::Arrange(_) => Verb::Arrange,
        }
    }
}

/// Resolves the inputs and the output mode, runs `task` over every file
/// and prints the outcomes and the summary.
fn run_files(args: &Run, task: Task) -> ExitCode {
    let input = args.input;
    let write = input.write;
    let started = Instant::now();
    let dash = input.paths.iter().any(|p| p == Path::new("-"));
    if dash && (input.paths.len() > 1 || input.files_from.is_some()) {
        return fail("`-` (stdin) cannot be combined with other inputs");
    }
    if input.stdin_filepath.is_some() && !input.paths.is_empty() && !dash {
        return fail("--stdin-filepath reads stdin; name no other input");
    }
    // `--stdin-filepath` implies `--stdin`; `-` is `--stdin`.
    let stdin = dash || input.stdin || input.stdin_filepath.is_some();
    if stdin && write {
        return fail("-w cannot write to stdin");
    }
    let mode_output = if write {
        Some(Output::Write)
    } else if input.diff {
        Some(Output::Diff)
    } else if input.check {
        Some(Output::Check)
    } else {
        None
    };
    let mut paths = input.paths.to_vec();
    // A list's own errors (a blank or non-UTF-8 entry): printed with the
    // inputs' errors, the rest of the list still runs.
    let mut list_errors = Vec::new();
    if let Some(list) = input.files_from.as_deref() {
        let list = Input::positional(list);
        match list.bytes() {
            Ok(bytes) => {
                let (entries, errors) = inputs::parse_list(&bytes);
                paths.extend(entries);
                list_errors.extend(errors.iter().map(|e| format!("{}: {e}", list.name())));
            }
            Err(e) => return fail(format!("{}: {}", list.name(), io_message(&e))),
        }
    }
    if !stdin && paths.is_empty() && input.files_from.is_none() && input.git.is_none() {
        return fail(no_input(task.verb()));
    }
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => return fail(format!("the current directory: {}", io_message(&e))),
    };
    let (items, output, summary, mut printer) = if stdin {
        let output = mode_output.unwrap_or(Output::Stdout);
        (
            vec![Item::Stdin(input.stdin_filepath.clone())],
            output,
            false,
            Printer::new(output, task.verb(), input.quiet),
        )
    } else {
        let inputs = match input.git {
            // `--git` takes no PATHS or list (clap's conflicts).
            Some(which) => match inputs::git(which, args.cfm, &cwd) {
                Ok(inputs) => inputs,
                Err(e) => return fail(format!("--git: {e}")),
            },
            None => inputs::resolve(&paths, args.cfm),
        };
        if args.cfm && inputs.directories == 0 && input.git.is_none() {
            return fail("--cfm applies to the directories walked and to --git; name a directory");
        }
        // The report is `cfformat DIR` alone (one PATH, a directory, no
        // list) or `cfformat --git WHICH` alone: a selection, like a walk.
        let report = input.git.is_some()
            || (input.paths.len() == 1 && inputs.directories == 1 && input.files_from.is_none());
        let output = mode_output.unwrap_or(if report {
            Output::Report
        } else {
            Output::Stdout
        });
        let mut printer = Printer::new(output, task.verb(), input.quiet);
        for e in list_errors.iter().chain(&inputs.errors) {
            printer.input_error(e);
        }
        // Inputs that resolved to no file at all: their errors, no summary.
        if inputs.files.is_empty() && (!inputs.errors.is_empty() || !list_errors.is_empty()) {
            return ExitCode::from(printer.exit_code());
        }
        if output == Output::Stdout && inputs.files.len() > 1 {
            return fail(format!(
                "{} files given; use -w, --check or --diff",
                inputs.files.len()
            ));
        }
        let summary = inputs.expanded || inputs.files.len() > 1;
        let items = inputs.files.into_iter().map(Item::File).collect();
        (items, output, summary, printer)
    };
    let pool = match rayon::ThreadPoolBuilder::new()
        // 0 is rayon's default: the available cores.
        .num_threads(input.jobs.map_or(0, |j| j as usize))
        .thread_name(|i| format!("{WORKER}{i}"))
        // The printer is bounded against a 2 MB stack (`print::MAX_DEPTH`);
        // this is headroom, and costs only address space.
        .stack_size(WORKER_STACK)
        .build()
    {
        Ok(pool) => pool,
        Err(e) => return fail(format!("cannot start the worker threads: {e}")),
    };
    let job = Job {
        task,
        mode: args.mode,
        output,
        timing: args.timing,
        cwd: &cwd,
    };
    let mut open = true;
    run_ordered(
        &pool,
        &items,
        |item| guarded(&item.name(), || run_item(&job, item)),
        // Once stdout takes nothing more, the workers finish unheard.
        |outcome| {
            if open {
                open = printer.print(outcome);
            }
        },
    );
    let islands = match task {
        Task::Format { islands, .. } => islands.stats(),
        Task::Arrange(_) => IslandStats::default(),
    };
    printer.summary(summary, started.elapsed(), islands, args.timing);
    ExitCode::from(printer.exit_code())
}

/// A worker's stack: 8 MB, where the platform default is 2 MB.
const WORKER_STACK: usize = 8 << 20;

/// The prefix of the pool's thread names, which the panic hook recognises.
const WORKER: &str = "cfformat-worker-";

/// Runs `work` over `items` on `pool` and hands each result to `print` on
/// the calling thread, in the items' order, as soon as every earlier one is
/// done: workers send `(index, result)` over a channel and a reorder buffer
/// holds what arrives early. Output streams during a long run instead of
/// waiting for the last file, and does not depend on the thread count.
fn run_ordered<T, R>(
    pool: &rayon::ThreadPool,
    items: &[T],
    work: impl Fn(&T) -> R + Sync,
    mut print: impl FnMut(R),
) where
    T: Sync,
    R: Send,
{
    use rayon::prelude::*;
    let _quiet = QuietPanics::install();
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        let work = &work;
        scope.spawn(move || {
            pool.install(|| {
                items
                    .par_iter()
                    .enumerate()
                    .for_each_with(tx, |tx, (i, item)| {
                        let _ = tx.send((i, work(item)));
                    });
            });
        });
        let mut early = BTreeMap::new();
        let mut next = 0;
        for (i, result) in rx {
            early.insert(i, result);
            while let Some(result) = early.remove(&next) {
                print(result);
                next += 1;
            }
        }
    });
}

/// Runs one file's work; a panic becomes the file's error (`path: internal
/// error: message`) and the run goes on.
fn guarded(name: &str, work: impl FnOnce() -> FileOutcome) -> FileOutcome {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| FileOutcome {
        name: name.to_owned(),
        error: Some(FileError::Panic(panic_message(payload.as_ref()))),
        ..FileOutcome::default()
    })
}

/// A panic's message: the payload as `&str` or `String`, else `panic`.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

type Hook = Arc<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

/// While alive, a panic on a pool thread prints nothing (the file's error
/// line reports it), nor one on an island thread (`ISLAND_THREAD`: the
/// island's warning reports it); any other thread's panic goes to the
/// previous hook, which is restored on drop.
struct QuietPanics(Hook);

impl QuietPanics {
    fn install() -> Self {
        let previous: Hook = Arc::from(std::panic::take_hook());
        let chained = Arc::clone(&previous);
        std::panic::set_hook(Box::new(move |info| {
            if !thread::current()
                .name()
                .is_some_and(|n| n.starts_with(WORKER) || n == cfformat::islands::ISLAND_THREAD)
            {
                chained(info);
            }
        }));
        QuietPanics(previous)
    }
}

impl Drop for QuietPanics {
    fn drop(&mut self) {
        let previous = Arc::clone(&self.0);
        std::panic::set_hook(Box::new(move |info| previous(info)));
    }
}

/// The usage error for a run given no input: never the current directory,
/// so an accidental `cfformat -w` cannot rewrite a tree.
fn no_input(verb: Verb) -> String {
    let example = match verb {
        Verb::Format => "cfformat .",
        Verb::Arrange => "cfformat arrange .",
    };
    format!(
        "no input given; name files, directories or globs (e.g. `{example}`), or use --stdin / --files-from / --git"
    )
}

/// What a run formats.
enum Item {
    /// A file, named as given (a file called `-` included).
    File(PathBuf),
    /// The source on stdin, formatted as if it were the file given with
    /// `--stdin-filepath` (which need not exist).
    Stdin(Option<PathBuf>),
}

impl Item {
    /// The item as messages name it: the path as given, `<stdin>` for stdin
    /// without `--stdin-filepath`.
    fn name(&self) -> String {
        match self {
            Item::File(p) | Item::Stdin(Some(p)) => p.display().to_string(),
            Item::Stdin(None) => STDIN.to_owned(),
        }
    }

    fn input(&self) -> Input<'_> {
        match self {
            Item::File(p) => Input::File(p),
            Item::Stdin(_) => Input::Stdin,
        }
    }
}

/// What every file of a run shares.
struct Job<'a> {
    task: Task<'a>,
    /// The requested mode: `Auto` unless `--script` / `--tags`, resolved
    /// per file by its name ([`Mode::or_for_path`]).
    mode: Mode,
    output: Output,
    timing: bool,
    cwd: &'a Path,
}

/// A file's task once its settings are resolved: the options and the
/// islands to format with, or the options to arrange with.
enum Ready<'a> {
    Format(Options, Option<&'a Islands>),
    Arrange(ArrangeOptions),
}

/// Runs the task on one file (settings resolved, read, formatted or
/// arranged) and compares the result with the source.
fn run_item(job: &Job, item: &Item) -> FileOutcome {
    let path = match item {
        Item::File(p) | Item::Stdin(Some(p)) => Some(p.as_path()),
        Item::Stdin(None) => None,
    };
    let mut outcome = FileOutcome {
        name: item.name(),
        ..FileOutcome::default()
    };
    // `.cfs` is script and `.cfm` tags by name unless a flag says otherwise;
    // a `.cfc`, any other name and bare stdin are resolved from the source.
    let mode = job.mode.or_for_path(path);
    // Settings first: a settings error is reported without reading the file.
    let ready = match job.task {
        Task::Format {
            discovery,
            islands,
            use_islands,
        } => {
            let resolved = match path {
                Some(p) => discovery.discover(p),
                None => discovery.discover_in(job.cwd),
            };
            match resolved {
                Ok(r) => {
                    outcome.settings_warnings = r.warnings;
                    Ready::Format(r.options, use_islands.then_some(islands))
                }
                Err(e) => {
                    outcome.settings_error = Some(e.to_string());
                    return outcome;
                }
            }
        }
        Task::Arrange(opts) => Ready::Arrange(opts),
    };
    let src = match item.input().read_utf8() {
        Ok(s) => s,
        Err(e) => {
            outcome.error = Some(FileError::Read(io_message(&e)));
            return outcome;
        }
    };
    // Only the report's summary counts lines.
    if job.output == Output::Report {
        outcome.lines = src.lines().count();
    }
    let text = match ready {
        Ready::Format(opts, islands) => {
            let ctx = FormatCtx { path, islands };
            let started = Instant::now();
            let formatted = format_with(&src, mode, &opts, &ctx);
            let total = started.elapsed();
            if let Some(islands) = islands {
                outcome.config_warnings = islands.config_warnings();
            }
            outcome.timing = job
                .timing
                .then(|| timing_line(&outcome.name, &formatted, total));
            outcome.warnings = formatted.warnings;
            formatted.text
        }
        Ready::Arrange(opts) => {
            let arranged = cfformat::arrange(&src, mode, &opts);
            // The warning's path stays `None`: the CLI prints a warning
            // with the file's name as given (`Outcome::name`).
            outcome.warnings = arranged
                .skipped
                .iter()
                .map(|s| cfformat::Warning::recovered(None, s.line, s.reason))
                .collect();
            arranged.text
        }
    };
    outcome.changed = text != src;
    match job.output {
        Output::Stdout => outcome.text = Some(text),
        Output::Report | Output::Check => {}
        Output::Diff if outcome.changed => {
            outcome.diff = Some(
                TextDiff::from_lines(&src, &text)
                    .unified_diff()
                    .context_radius(3)
                    .header(
                        &format!("a/{}", outcome.name),
                        &format!("b/{}", outcome.name),
                    )
                    .to_string(),
            );
        }
        Output::Diff => {}
        // Only a file whose text differs is written, so an unchanged file
        // keeps its mtime.
        // `-w` with stdin is a usage error.
        Output::Write if outcome.changed => {
            if let Item::File(p) = item {
                if let Err(e) = write_atomic(p, &text) {
                    outcome.changed = false;
                    outcome.error = Some(FileError::Write(io_message(&e)));
                }
            }
        }
        Output::Write => {}
    }
    outcome
}

/// Replaces the contents of the file at `path` with `text`, all or nothing:
/// the text goes to a sibling temp file (`.<name>.cfformat-<pid>-<n>.tmp`,
/// created new, given the original's permissions) that is then renamed over
/// the original. Any failure removes the temp file and leaves the original
/// as it was. A symlink is written through: the rename happens at the end of
/// its chain (`canonicalize`), so the link stays a link. A file the user
/// cannot write fails as `std::fs::write` would, although the rename alone
/// could replace it.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let target = std::fs::canonicalize(path)?;
    // Opening for writing (without truncating) is the permission check.
    drop(std::fs::OpenOptions::new().write(true).open(&target)?);
    let permissions = std::fs::metadata(&target)?.permissions();
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(std::io::Error::other("not a file"));
    };
    let mut temp = std::ffi::OsString::from(".");
    temp.push(name);
    temp.push(format!(
        ".cfformat-{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = dir.join(temp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.set_permissions(permissions))
        .and_then(|()| {
            drop(file);
            std::fs::rename(&temp, &target)
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// `cfformat doc`: the doc as `print_doc` sees it (breaks propagated),
/// islands spliced ([`cfformat::debug_doc`]).
fn doc(args: &DocArgs) -> ExitCode {
    let mut settings = Settings::new(args.config.as_deref());
    let path = args.path.as_path();
    let opts = match settings.resolve(path) {
        Ok(r) => r.options,
        Err(e) => return fail(e),
    };
    let input = Input::positional(path);
    let name = input.name();
    let src = match input.read_utf8() {
        Ok(s) => s,
        Err(e) => return fail(format!("{name}: {}", io_message(&e))),
    };
    let islands = Islands::new();
    let ctx = FormatCtx {
        path: match input {
            Input::File(p) => Some(p),
            Input::Stdin => None,
        },
        islands: (!args.engine.no_islands).then_some(&islands),
    };
    let mode = args.engine.mode().or_for_path(ctx.path);
    let started = Instant::now();
    let doc = cfformat::debug_doc(&src, mode, &opts, &ctx);
    let total = started.elapsed();
    settings.warned.print(&islands.config_warnings(), false);
    report(&name, &doc, args.timing.then_some(total));
    // Like a formatted file's text, a pager that stops reading is not an error.
    write_stdout(&doc.text, true)
}

/// Writes `text` to stdout and flushes: success, or `error: stdout: …` and
/// exit 2 (a closed pipe when `pager` is quiet and exit 0).
fn write_stdout(text: &str, pager: bool) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if stdout_failed(&e, pager) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(2),
    }
}

/// Prints a file's warnings and, with `--timing` (the total time given),
/// its times.
fn report(name: &str, f: &Formatted, timing: Option<Duration>) {
    for w in &f.warnings {
        eprintln!("{}", warning_line(name, w));
    }
    if let Some(total) = timing {
        eprintln!("{}", timing_line(name, f, total));
    }
}

/// `file:line: key: message` (`file:line: not formatted: reason` for a
/// recovery), naming the file as it was given.
fn warning_line(name: &str, w: &cfformat::Warning) -> String {
    format!("{name}:{}: {}: {}", w.line, w.label(), w.message)
}

/// `path: parse Xms doc Yms print Zms islands N/W Tms total Ams`: the three
/// phases of [`Formatted::timings`], the island formatter's share of `doc`,
/// and `total`, the caller's measurement around the whole call, which
/// includes whatever the phases do not (freeing the tree and the document,
/// for one).
fn timing_line(name: &str, f: &Formatted, total: Duration) -> String {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    format!(
        "{name}: parse {:.1}ms doc {:.1}ms print {:.1}ms islands {}/{} {:.1}ms total {:.1}ms",
        ms(f.timings.parse),
        ms(f.timings.doc),
        ms(f.timings.print),
        f.islands.islands(),
        f.islands.warnings,
        ms(f.islands.time),
        ms(total)
    )
}

/// `cfformat settings`: the settings files on stderr, the effective options
/// as one JSON object with sorted keys on stdout. `--defaults` reads nothing;
/// `--migrate` rewrites a settings file instead ([`migrate`]).
fn settings(args: &SettingsArgs) -> ExitCode {
    if args.migrate {
        return migrate(args.path.as_deref());
    }
    if args.schema {
        let schema = serde_json::to_string_pretty(&schema::schema()).expect("schema serialises");
        return write_stdout(&format!("{schema}\n"), false);
    }
    let opts = if args.defaults {
        Options::default()
    } else {
        let mut settings = Settings::new(args.config.as_deref());
        let path = args.path.clone().unwrap_or_else(|| PathBuf::from("-"));
        if path != Path::new("-") && !path.exists() {
            return fail(format!(
                "{} is not a valid file or directory",
                path.display()
            ));
        }
        match settings.resolve(&path) {
            Ok(r) => {
                if r.sources.is_empty() {
                    eprintln!("sources: none");
                } else {
                    eprintln!("sources:");
                    for s in &r.sources {
                        eprintln!("  {}", s.display());
                    }
                }
                r.options
            }
            Err(e) => return fail(e),
        }
    };
    let value = serde_json::to_value(&opts).expect("options serialise");
    let sorted: BTreeMap<String, serde_json::Value> = match value {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => unreachable!("options serialise to an object"),
    };
    let json = serde_json::to_string_pretty(&sorted).expect("options serialise");
    write_stdout(&format!("{json}\n"), false)
}

/// `cfformat settings --migrate [FILE|-]`: rewrites a settings file for the
/// current key set, as loading it does in memory ([`Options::migrate`]).
/// Each migration warning prints as discovery prints it, then `FILE: N keys
/// migrated`; a file with nothing to migrate is not written (`FILE: nothing
/// to migrate`), so a second run leaves it alone. A file that does not load
/// — unreadable, not a JSON object, a key that is no setting, a wrong value
/// — is the settings error it is anywhere else, exit 2, and stays as it was.
/// The object is pretty-printed with a final newline, the surviving keys in
/// the file's order and the renamed ones after them, and replaces the file
/// whole (`write_atomic`). FILE defaults to `.cfformat.json` in the current
/// directory; `-` reads stdin and writes the object to stdout, migrated or
/// not.
fn migrate(path: Option<&Path>) -> ExitCode {
    let input = Input::positional(path.unwrap_or(Path::new(SETTINGS_FILE)));
    let name = input.name();
    let text = match input.read_utf8() {
        Ok(t) => t,
        Err(e) => return fail(format!("{name}: {}", io_message(&e))),
    };
    let original = match serde_json::from_str(&text) {
        Ok(serde_json::Value::Object(map)) => map,
        Ok(_) => return fail(format!("{name}: expected a JSON object")),
        Err(e) => return fail(format!("{name}: {e}")),
    };
    let (migrated, warnings) = Options::migrate(original.clone());
    if let Err(e) = Options::validate(&migrated) {
        return fail(match e {
            OptionsError::Invalid(message) => format!("{name}: {message}"),
            other => other.to_string(),
        });
    }
    for w in &warnings {
        eprintln!("warning: {name}: {w}");
    }
    // A key is migrated when it does not survive under its own name with
    // its own value: removed, renamed (a spelling change too) or merged.
    let count = original
        .iter()
        .filter(|&(key, value)| migrated.get(key) != Some(value))
        .count();
    let summary = match count {
        0 => format!("{name}: nothing to migrate"),
        1 => format!("{name}: 1 key migrated"),
        n => format!("{name}: {n} keys migrated"),
    };
    let json = serde_json::to_string_pretty(&migrated).expect("a JSON object serialises") + "\n";
    match input {
        Input::Stdin => {
            eprintln!("{summary}");
            write_stdout(&json, false)
        }
        Input::File(_) if count == 0 => {
            eprintln!("{summary}");
            ExitCode::SUCCESS
        }
        Input::File(p) => match write_atomic(p, &json) {
            Ok(()) => {
                eprintln!("{summary}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(format!("{name}: {}", io_message(&e))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_the_file_s_error() {
        let ok = guarded("a.cfc", || FileOutcome {
            name: "a.cfc".into(),
            changed: true,
            ..FileOutcome::default()
        });
        assert!(ok.changed && ok.error.is_none());
        let message = |o: FileOutcome| {
            Printer::new(Output::Check, Verb::Format, false).error_line(&o.name, o.error.unwrap())
        };
        assert_eq!(
            message(guarded("src/a.cfc", || panic!("boom"))),
            "src/a.cfc: internal error: boom"
        );
        let what = String::from("boom");
        assert_eq!(
            message(guarded("src/a.cfc", || panic!("{what}"))),
            "src/a.cfc: internal error: boom"
        );
        assert_eq!(
            message(guarded("b.cfc", || std::panic::panic_any(42))),
            "b.cfc: internal error: panic"
        );
    }

    #[test]
    fn results_print_in_order_whatever_the_threads() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .thread_name(|i| format!("{WORKER}{i}"))
            // The printer is bounded against a 2 MB stack (`print::MAX_DEPTH`);
            // this is headroom, and costs only address space.
            .stack_size(WORKER_STACK)
            .build()
            .unwrap();
        let items: Vec<u64> = (0..64).collect();
        let mut seen = Vec::new();
        run_ordered(
            &pool,
            &items,
            |&i| {
                // Early items finish last.
                thread::sleep(Duration::from_micros((64 - i) * 50));
                if i == 7 {
                    let caught = guarded("x", || panic!("quiet"));
                    assert!(caught.error.is_some());
                }
                i
            },
            |i| seen.push(i),
        );
        assert_eq!(seen, items);
    }
}
