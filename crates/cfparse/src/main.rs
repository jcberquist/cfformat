//! Thin debugging binary over the `cfparse` library.
//!
//! ```text
//! cfparse parse <FILE|-> [--script|--tags] [--json|--tree] [--spans]
//! cfparse parse <DIR> [--cfm]     # parse + serialise every .cfc (.cfm), print a summary
//! ```

use std::io::{Read, Write};
use std::process::ExitCode;

use cfparse::json::{self, JsonOpts};
use cfparse::Mode;

const USAGE: &str = "usage:
  cfparse parse <FILE|-> [--script|--tags] [--json|--tree] [--spans]
  cfparse parse <DIR> [--cfm]
  cfparse --version";

struct Args {
    path: String,
    mode: Mode,
    tree: bool,
    cfm: bool,
    opts: JsonOpts,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut it = argv.iter();
    match it.next().map(String::as_str) {
        Some("parse") => {}
        Some(other) => return Err(format!("unknown command `{other}`")),
        None => return Err("missing command".into()),
    }
    let mut path = None;
    let mut mode = Mode::Auto;
    let mut tree = false;
    let mut cfm = false;
    let mut opts = JsonOpts::default();
    for arg in it {
        match arg.as_str() {
            "--script" => mode = Mode::Script,
            "--tags" => mode = Mode::Tags,
            "--json" => tree = false,
            "--tree" => tree = true,
            "--spans" => opts.spans = true,
            "--cfm" => cfm = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            p if path.is_none() => path = Some(p.to_string()),
            extra => return Err(format!("unexpected argument `{extra}`")),
        }
    }
    let path = path.ok_or("missing FILE (use `-` for stdin)")?;
    Ok(Args {
        path,
        mode,
        tree,
        cfm,
        opts,
    })
}

fn read_input(path: &str) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    if path == "-" {
        std::io::stdin().read_to_end(&mut bytes)?;
    } else {
        bytes = std::fs::read(path)?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("cfparse {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if argv.is_empty() || argv.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if std::path::Path::new(&args.path).is_dir() {
        return parse_dir(&args);
    }
    let src = match read_input(&args.path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {e}", args.path);
            return ExitCode::from(2);
        }
    };

    // Both front ends are total: a file that can be read parses.
    let tree = cfparse::parse_source(&src, args.mode);
    let out = if args.tree {
        cfparse::debug::format_tree(&tree, args.opts)
    } else {
        json::to_string_pretty(&json::to_json(&tree, args.opts))
    };
    let _ = std::io::stdout().lock().write_all(out.as_bytes());
    ExitCode::SUCCESS
}

/// Parse and serialise every `.cfc` (and `.cfm` with `--cfm`) under a
/// directory in one process; prints failures and a summary to stderr.
fn parse_dir(args: &Args) -> ExitCode {
    let mut files = Vec::new();
    collect(std::path::Path::new(&args.path), args.cfm, &mut files);
    files.sort();
    let started = std::time::Instant::now();
    let (mut lines, mut failed) = (0usize, 0usize);
    for path in &files {
        let src = match read_input(&path.to_string_lossy()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failed += 1;
                continue;
            }
        };
        lines += src.lines().count();
        let tree = cfparse::parse_source(&src, args.mode);
        let out = json::to_string_pretty(&json::to_json(&tree, args.opts));
        std::hint::black_box(out);
    }
    eprintln!(
        "{} files, {} lines, {} failed, {:.3}s",
        files.len(),
        lines,
        failed,
        started.elapsed().as_secs_f64()
    );
    if failed > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn collect(dir: &std::path::Path, cfm: bool, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            collect(&path, cfm, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "cfc" || (cfm && e == "cfm"))
        {
            out.push(path);
        }
    }
}
