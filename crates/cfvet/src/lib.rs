//! Two CFML checks on the `cfparse` tree.
//!
//! Two rules. `missing-var` ([`MISSING_VAR`]): a write inside a function to
//! a name with no scope and no `var` lands in the component's (or the
//! page's) `variables` scope in classic CFML, where it outlives the call and
//! is shared by every call on the same instance. `unevaluated-call`
//! ([`UNEVALUATED_CALL`]): a `#…#` that calls a function, in template text
//! outside `<cfoutput>`, is output as written and the call never runs.
//!
//! The library lints a string and never touches a file; the `cfvet` binary
//! walks paths, prints and sets the exit code.

mod rules;
mod scope;
mod suppress;

use cfparse::{parse_source, Mode, Tree};

pub use rules::missing_var::RULE as MISSING_VAR;
pub use rules::unevaluated_call::RULE as UNEVALUATED_CALL;

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// 1-based line of the written name, or of the `#…#`'s opening `#`.
    pub line: usize,
    /// 1-based column of the same, in characters.
    pub column: usize,
    /// The rule's id, `missing-var` or `unevaluated-call`.
    pub rule: &'static str,
    /// The written name as the source spells it, or the `#…#` with its
    /// arguments elided.
    pub name: String,
    /// The function declares the name, but no declaration of it covers this
    /// write: each is below it, or in a block the write is not inside (an
    /// `if` branch, a loop body). On Lucee a `var` declares only when its
    /// line runs, so the write may land in the `variables` scope. The
    /// message says so. Always `false` for `unevaluated-call`.
    pub var_not_run: bool,
    pub message: String,
}

impl Report {
    /// The report's output line: `path:line:column: rule: message`.
    pub fn render(&self, path: &str) -> String {
        format!(
            "{path}:{}:{}: {}: {}",
            self.line, self.column, self.rule, self.message
        )
    }
}

/// A region the parser did not understand, which is not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// 1-based line where the region starts.
    pub line: usize,
    pub message: String,
}

impl Note {
    /// The note's output line: `path:line: note: message`.
    pub fn render(&self, path: &str) -> String {
        format!("{path}:{}: note: {}", self.line, self.message)
    }
}

/// What linting one source found.
#[derive(Debug, Clone, Default)]
pub struct Lint {
    /// Sorted by line, then column; one per rule, name and line.
    pub reports: Vec<Report>,
    /// One per recovered region, in source order.
    pub notes: Vec<Note>,
    /// How many function bodies were checked (closures included).
    pub functions: usize,
}

/// Parses `src` in `mode` and lints it.
pub fn lint_source(src: &str, mode: Mode) -> Lint {
    lint_tree(&parse_source(src, mode))
}

/// Lints a parsed tree.
pub fn lint_tree(tree: &Tree) -> Lint {
    let units = scope::units(tree);
    let suppressed = suppress::suppressed_lines(tree);
    let mut reports = rules::missing_var::check(tree, &units);
    reports.extend(rules::unevaluated_call::check(tree));
    reports.retain(|r| !suppressed.contains(&r.line));
    reports.sort_by_key(|r| (r.line, r.column));
    // A line is fixed once per rule and name: `for (i = 1; i <= n; i++)` is
    // one report, at the first write.
    let mut seen = std::collections::HashSet::new();
    reports.retain(|r| seen.insert((r.line, r.rule, r.name.to_lowercase())));
    let notes = tree
        .recoveries
        .iter()
        .map(|r| Note {
            line: tree.line_of(r.span.start),
            message: format!(
                "parse recovery ({}); the region is not checked",
                r.reason.name()
            ),
        })
        .collect();
    Lint {
        reports,
        notes,
        functions: units.len(),
    }
}

/// 1-based line and column (in characters) of a byte offset in the tree's
/// normalised source.
pub(crate) fn position(tree: &Tree, offset: u32) -> (usize, usize) {
    let offset = (offset as usize).min(tree.source.len());
    let before = &tree.source[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (
        tree.line_of(offset as u32),
        before[line_start..].chars().count() + 1,
    )
}
