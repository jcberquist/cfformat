//! `cfvet` binary: the command line.
//!
//! ```text
//! cfvet [PATHS...] [--git staged|unstaged|all] [--script|--tags] [-q|--quiet]
//! ```
//!
//! File IO, walking and printing live here, in `cli`; the library lints a
//! string and never touches a file.

mod cli;

fn main() -> std::process::ExitCode {
    cli::main()
}
