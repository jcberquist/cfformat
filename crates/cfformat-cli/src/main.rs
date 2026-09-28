//! `cfformat` binary: the command line.
//!
//! ```text
//! cfformat [PATHS...] [-w|--write] [--check] [--diff] [--cfm] [--config FILE]
//!          [--stdin] [--stdin-filepath PATH] [--files-from FILE|-]
//!          [--git staged|unstaged|all] [-j N] [--quiet]
//!          [--timing] [--no-islands] [--script|--tags]
//! cfformat doc <FILE|-> [--config FILE] [--no-islands] [--timing] [--script|--tags]
//! cfformat settings [PATH|-] [--config FILE] [--defaults] [--schema]
//! cfformat settings --migrate [FILE|-]
//! cfformat arrange [PATHS...] [-w|--write] [--check] [--diff] [--stdin]
//!          [--stdin-filepath PATH] [--files-from FILE|-]
//!          [--git staged|unstaged|all] [-j N] [--quiet]
//!          [--script|--tags] [--properties]
//! ```
//!
//! The `cfformat-cli` package: file IO, walking, diffs, parallelism and
//! printing live here, in `cli`, over the `cfformat` library's public API;
//! the library formats a string and never touches a file. A package of its
//! own so that clap, ignore, rayon and similar stay out of the library's
//! dependency graph.

mod cli {
    pub mod args;
    pub mod inputs;
    pub mod output;
    pub mod run;
    pub mod schema;
}

fn main() -> std::process::ExitCode {
    cli::run::main()
}
