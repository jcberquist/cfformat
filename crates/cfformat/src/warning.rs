//! What printed as written instead of formatted, and why.

use std::fmt;
use std::path::{Path, PathBuf};

use cfparse::RecoveryReason;

/// Something printed as written instead of formatted: an island the island
/// formatter refused, or a region the parse did not understand. Neither
/// fails the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The file, when there is one.
    pub path: Option<PathBuf>,
    /// The first line of what printed as written, 1-based.
    pub line: usize,
    /// What it was.
    pub kind: WarningKind,
    /// The formatter's first diagnostic, or the recovery's reason in words.
    pub message: String,
}

/// What a [`Warning`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// The island formatter refused the island `key` dispatches to.
    Island {
        /// The island's option (`islands.js`).
        key: &'static str,
    },
    /// The parse recovered ([`cfparse::Recovery`]).
    Recovered(RecoveryReason),
}

impl Warning {
    /// What the line names after the line number: the island's option, or
    /// `not formatted` for a recovery.
    pub fn label(&self) -> &'static str {
        match self.kind {
            WarningKind::Island { key } => key,
            WarningKind::Recovered(_) => "not formatted",
        }
    }

    /// The warning for a recovered region starting on `line`.
    pub fn recovered(path: Option<PathBuf>, line: usize, reason: RecoveryReason) -> Self {
        let message = match reason {
            RecoveryReason::Unmatched => "an unmatched run",
            RecoveryReason::StrayCloser => "a stray closer",
            RecoveryReason::Unclosed => "an unclosed block",
            RecoveryReason::TooDeep => "nesting past the limit",
        };
        Warning {
            path,
            line,
            kind: WarningKind::Recovered(reason),
            message: message.into(),
        }
    }
}

/// `{path|<stdin>}:{line}: {key}: {message}` for an island,
/// `{path|<stdin>}:{line}: not formatted: {reason}` for a recovery.
impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self
            .path
            .as_deref()
            .map_or_else(|| "<stdin>".into(), Path::to_string_lossy);
        write!(
            f,
            "{path}:{}: {}: {}",
            self.line,
            self.label(),
            self.message
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_message() {
        let w = Warning {
            path: Some(PathBuf::from("page.cfm")),
            line: 3,
            kind: WarningKind::Island { key: "islands.js" },
            message: "SyntaxError: fake (1:1)".into(),
        };
        assert_eq!(
            w.to_string(),
            "page.cfm:3: islands.js: SyntaxError: fake (1:1)"
        );
        let lines = [
            (RecoveryReason::Unmatched, "an unmatched run"),
            (RecoveryReason::StrayCloser, "a stray closer"),
            (RecoveryReason::Unclosed, "an unclosed block"),
            (RecoveryReason::TooDeep, "nesting past the limit"),
        ];
        for (reason, text) in lines {
            let w = Warning::recovered(None, 2, reason);
            assert_eq!(w.to_string(), format!("<stdin>:2: not formatted: {text}"));
        }
    }
}
