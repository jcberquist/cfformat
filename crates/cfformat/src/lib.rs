//! CFML formatter: parses with [`cfparse`], builds a [`cfdoc`] document and
//! prints it. `README.md` describes the API, what prints and the CLI.
//!
//! ```
//! use cfformat::options::NewlineStyle;
//! use cfformat::{format_source, Mode, Options};
//!
//! // The default newline is the platform's (`\r\n` on Windows); `Lf` makes
//! // the output the same everywhere.
//! let opts = Options { newline: NewlineStyle::Lf, ..Options::default() };
//! let out = format_source("x=1;", Mode::Script, &opts);
//! assert_eq!(out, "x = 1;\n");
//! ```

pub mod arrange;
pub mod islands;
pub mod options;
pub mod print;
mod warning;

use std::path::Path;
use std::time::{Duration, Instant};

use cfdoc::{print_doc, PrintOptions};

// The `cfparse` and `cfdoc` types this crate's public items take or return,
// and no more: the crate's API is the formatter's, not the union of the
// three crates'. A caller that parses (`cfparse::parse_source`, for
// `tree_to_doc`) or prints a document itself depends on those crates
// directly, as the README says.
pub use arrange::{arrange, ArrangeOptions, Arranged, Skipped};
pub use cfdoc::{Doc, GroupIdGen, IndentStyle};
pub use cfparse::{Element, Island, Lang, Mode, Newline, Node, RecoveryReason, Token, Tree};
pub use islands::{IslandStats, Islands};
pub use options::Options;
pub use print::PrintReport;
pub use warning::{Warning, WarningKind};

/// What [`format_with`] needs beyond the source and the options.
#[derive(Debug, Clone, Copy, Default)]
pub struct FormatCtx<'a> {
    /// The file being formatted: an island is formatted as a synthetic path
    /// next to it (`page.cfm.js`), under the configuration found from its
    /// directory. `None` for stdin (`stdin.cfm.js` in the current
    /// directory).
    pub path: Option<&'a Path>,
    /// Island formatting for this run; `None` prints every island
    /// verbatim whatever the `islands.*` options say (`--no-islands`).
    pub islands: Option<&'a Islands>,
}

/// Time spent in each phase of [`format_with`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timings {
    /// Normalising and parsing.
    pub parse: Duration,
    /// Building the document, island formatting included.
    pub doc: Duration,
    /// Printing the document.
    pub print: Duration,
}

/// A formatted source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    /// The output.
    pub text: String,
    /// What printed as written instead of formatted, in source order:
    /// islands the island formatter refused, and regions the parse did not
    /// understand ([`cfparse::Tree::recoveries`]).
    pub warnings: Vec<Warning>,
    /// Where the time went.
    pub timings: Timings,
    /// Island formatting for this file alone: formatter runs, cache hits,
    /// refusals and the time in the formatter.
    pub islands: IslandStats,
}

/// Formats `src`: normalise and parse, build the document, print it at
/// `max_columns`, re-emit the BOM. The output ends with one newline unless it
/// is empty. Island formatting runs with a cache of its own and no file path;
/// warnings — a refused island, a region the parse recovered in, each printed
/// as written — are dropped (see [`format_with`]).
///
/// Formatting never fails: both front ends are total, and what the
/// formatter cannot read prints as written with a warning.
pub fn format_source(src: &str, mode: Mode, opts: &Options) -> String {
    let islands = Islands::new();
    let ctx = FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    format_with(src, mode, opts, &ctx).text
}

/// Formats `src` like [`format_source`], with a file path and a shared
/// [`Islands`]. An island the formatter refuses (oxc's parse error) and a
/// region the parse did not understand ([`cfparse::Recovery`]) print as
/// written, each a warning in [`Formatted::warnings`]; neither is an
/// error.
pub fn format_with(src: &str, mode: Mode, opts: &Options, ctx: &FormatCtx) -> Formatted {
    run(src, mode, opts, ctx, |tree, doc| {
        let print_opts = PrintOptions {
            width: opts.max_columns,
            indent: opts.indent_style(),
            newline: opts.newline_str(tree.newline),
        };
        let mut out = print_doc(doc, &print_opts);
        // Exactly one newline at the end, even when the last node's own text
        // ends with blank lines (a string left open at the end of the file).
        let nl = print_opts.newline;
        while out.len() > nl.len() && out.ends_with(nl) && out[..out.len() - nl.len()].ends_with(nl)
        {
            out.truncate(out.len() - nl.len());
        }
        if tree.bom {
            format!("\u{FEFF}{out}")
        } else {
            out
        }
    })
}

/// What `cfformat doc` prints: the document [`format_with`] would print,
/// in [`cfdoc`]'s debug form after the printer's break-propagation pass,
/// with a final newline. The warnings, timings and island numbers are
/// [`format_with`]'s; `timings.print` is the propagation and the debug
/// text.
pub fn debug_doc(src: &str, mode: Mode, opts: &Options, ctx: &FormatCtx) -> Formatted {
    run(src, mode, opts, ctx, |_, doc| {
        cfdoc::utils::propagate_breaks(doc);
        cfdoc::debug::format_doc(doc) + "\n"
    })
}

/// Parses `src`, builds its document and turns it into text with `print`,
/// timing each phase.
fn run(
    src: &str,
    mode: Mode,
    opts: &Options,
    ctx: &FormatCtx,
    print: impl FnOnce(&Tree, &mut Doc) -> String,
) -> Formatted {
    let started = Instant::now();
    let tree = cfparse::parse_source(src, mode);
    let parsed = Instant::now();
    let (mut doc, report) = tree_to_doc_with(&tree, opts, ctx);
    let built = Instant::now();
    let text = print(&tree, &mut doc);
    Formatted {
        text,
        warnings: report.warnings,
        islands: report.stats,
        timings: Timings {
            parse: parsed - started,
            doc: built - parsed,
            print: built.elapsed(),
        },
    }
}

/// The document for a parsed tree, before `print_doc`'s
/// break-propagation pass ([`debug_doc`] shows it after). Islands are
/// handed off as in [`format_source`].
pub fn tree_to_doc(tree: &Tree, opts: &Options) -> Doc {
    let islands = Islands::new();
    let ctx = FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    tree_to_doc_with(tree, opts, &ctx).0
}

/// [`tree_to_doc`] with a [`FormatCtx`], returning what the printing
/// reported too: the island formatter's warnings and numbers, and a
/// warning per recovered region.
///
/// The printers return a [`Doc`], not a `Result`: an island's outcome is
/// collected on the printer while the document is built and returned here,
/// so no tag printer threads a `Result`.
pub fn tree_to_doc_with(tree: &Tree, opts: &Options, ctx: &FormatCtx) -> (Doc, PrintReport) {
    let printer = print::Printer::with_ctx(tree, opts, ctx);
    let doc = printer.document();
    (doc, printer.finish())
}
