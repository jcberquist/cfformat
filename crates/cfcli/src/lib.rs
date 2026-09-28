//! The file plumbing shared by the `cfformat` and `cfvet` binaries: the
//! directory walk, the files git reports as changed (`--git`), the dedupe
//! of files named twice, reading a source as UTF-8 and the message of an
//! IO error. Each binary chooses which walked or listed files it keeps and
//! prints its own messages.

use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

mod git;

pub use git::{git_changes, GitChanges};

/// The name stdin goes by in messages.
pub const STDIN: &str = "<stdin>";

/// Where a source comes from. A file is a file whatever its name: a file
/// called `-` (from a list of paths) is read from disk; only a positional
/// `-` is stdin, and each command maps it here itself
/// ([`Input::positional`]).
#[derive(Debug, Clone, Copy)]
pub enum Input<'a> {
    Stdin,
    File(&'a Path),
}

impl<'a> Input<'a> {
    /// A positional argument: `-` is stdin, anything else a file.
    pub fn positional(path: &'a Path) -> Self {
        if path == Path::new("-") {
            Input::Stdin
        } else {
            Input::File(path)
        }
    }

    /// The input as messages name it: [`STDIN`], or the path as given.
    pub fn name(self) -> String {
        match self {
            Input::Stdin => STDIN.to_owned(),
            Input::File(p) => p.display().to_string(),
        }
    }

    /// The input's bytes.
    pub fn bytes(self) -> io::Result<Vec<u8>> {
        match self {
            Input::Stdin => {
                let mut bytes = Vec::new();
                io::stdin().read_to_end(&mut bytes)?;
                Ok(bytes)
            }
            Input::File(p) => std::fs::read(p),
        }
    }

    /// The input's text. Bytes that are not UTF-8 are an error (`not valid
    /// UTF-8`, of kind `InvalidData`), never replaced: a file in a legacy
    /// encoding is reported and left alone.
    pub fn read_utf8(self) -> io::Result<String> {
        String::from_utf8(self.bytes()?)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not valid UTF-8"))
    }
}

/// An `io::Error`'s message without the `(os error N)` suffix.
pub fn io_message(e: &io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_owned(),
        _ => s,
    }
}

/// Every file under `root`, in path order, and the walk's errors as
/// messages: `.gitignore`, the global gitignore, `.git/info/exclude` and
/// `.ignore` honoured (a `.gitignore` inside a git repository only),
/// hidden entries skipped, symlinks not followed. The caller keeps the
/// files it wants.
pub fn walk(root: &Path) -> impl Iterator<Item = Result<PathBuf, String>> {
    ignore::WalkBuilder::new(root)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .follow_links(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .build()
        .filter_map(|entry| match entry {
            Ok(e) if e.file_type().is_some_and(|t| t.is_file()) => Some(Ok(e.into_path())),
            Ok(_) => None,
            Err(e) => Some(Err(e.to_string())),
        })
}

/// The files of a run seen so far, by absolute path (not canonicalised):
/// a file named twice, directly or through a walk, is taken once, the
/// first time.
#[derive(Debug, Default)]
pub struct Dedupe(HashSet<PathBuf>);

impl Dedupe {
    /// Whether `file` is named for the first time in the run.
    pub fn first(&mut self, file: &Path) -> bool {
        let key = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        self.0.insert(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_messages_drop_the_os_error() {
        let e = io::Error::from_raw_os_error(2);
        assert!(!io_message(&e).contains("os error"), "{}", io_message(&e));
        assert_eq!(io_message(&io::Error::other("plain")), "plain");
    }

    #[test]
    fn a_file_is_first_once() {
        let mut seen = Dedupe::default();
        assert!(seen.first(Path::new("a.cfc")));
        assert!(!seen.first(Path::new("./a.cfc")));
        assert!(seen.first(Path::new("b.cfc")));
    }

    #[test]
    fn a_dash_is_stdin_only_as_a_positional() {
        assert!(matches!(Input::positional(Path::new("-")), Input::Stdin));
        assert_eq!(Input::positional(Path::new("-")).name(), STDIN);
        assert_eq!(Input::File(Path::new("-")).name(), "-");
    }
}
