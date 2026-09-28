//! The files git reports as changed: the `--git staged|unstaged|all`
//! selection of both binaries.
//!
//! `git` runs from `PATH`: the repository root first (`git rev-parse
//! --show-toplevel`, in the current directory), then the listings with `-C
//! <root>`, so every path they print is relative to the root whatever the
//! current directory, and whatever `diff.relative` says.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use crate::io_message;

/// Which changed files `--git` selects, in the whole repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChanges {
    /// Added, copied, modified or renamed in the index against `HEAD`.
    Staged,
    /// Added, copied, modified or renamed in the working tree against the
    /// index, and the untracked files that are not ignored.
    Unstaged,
    /// Both.
    All,
}

impl GitChanges {
    /// The values `--git` takes, in the order `--help` lists them.
    pub const NAMES: [&str; 3] = ["staged", "unstaged", "all"];
}

impl FromStr for GitChanges {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "staged" => Ok(GitChanges::Staged),
            "unstaged" => Ok(GitChanges::Unstaged),
            "all" => Ok(GitChanges::All),
            _ => Err(format!("expected one of {}", Self::NAMES.join(", "))),
        }
    }
}

/// The files of the repository holding `cwd` that `which` selects, in path
/// order, each once; only regular files of the working tree (a file the
/// index holds but the working tree has deleted, a symlink, a submodule
/// are not selected). A file under `cwd` is named relative to it
/// (`sub/a.cfc`), any other as the root joined with its path in the
/// repository. The caller keeps the extensions it wants.
///
/// An error is git's own message (its `fatal: ` dropped: `not a git
/// repository (or any of the parent directories): .git`), or `git: <the IO
/// error>` when it cannot be run.
pub fn git_changes(which: GitChanges, cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let top = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let top = top.strip_suffix(b"\n").unwrap_or(&top);
    let top = top.strip_suffix(b"\r").unwrap_or(top);
    let root = path_from_bytes(top).ok_or("the repository root is not valid UTF-8")?;
    let listings: &[&[&str]] = match which {
        GitChanges::Staged => &[STAGED],
        GitChanges::Unstaged => &[UNSTAGED, UNTRACKED],
        GitChanges::All => &[STAGED, UNSTAGED, UNTRACKED],
    };
    let mut in_repo = BTreeSet::new();
    for args in listings {
        let mut with_root: Vec<&OsStr> = vec![OsStr::new("-C"), root.as_os_str()];
        with_root.extend(args.iter().map(OsStr::new));
        let listed = git(cwd, &with_root)?;
        // `-z`: each path ends with a NUL, as git holds it (no quoting).
        for entry in listed.split(|&b| b == 0).filter(|e| !e.is_empty()) {
            let path = path_from_bytes(entry).ok_or("a path is not valid UTF-8")?;
            in_repo.insert(path);
        }
    }
    // The current directory within the repository, compared as the file
    // system names both (a symlinked temp directory, say).
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let here = canonical(cwd)
        .strip_prefix(canonical(&root))
        .ok()
        .map(Path::to_path_buf);
    Ok(in_repo
        .into_iter()
        .filter(|p| std::fs::symlink_metadata(root.join(p)).is_ok_and(|m| m.file_type().is_file()))
        .map(|p| match here.as_deref().map(|h| p.strip_prefix(h)) {
            Some(Ok(under)) => under.to_path_buf(),
            _ => root.join(&p),
        })
        .collect())
}

/// Staged: the index against `HEAD` (against nothing before the first
/// commit).
const STAGED: &[&str] = &[
    "diff",
    "--cached",
    "--name-only",
    "--no-color",
    "--diff-filter=ACMR",
    "-z",
];

/// Unstaged: the working tree against the index.
const UNSTAGED: &[&str] = &[
    "diff",
    "--name-only",
    "--no-color",
    "--diff-filter=ACMR",
    "-z",
];

/// Untracked and not ignored.
const UNTRACKED: &[&str] = &["ls-files", "--others", "--exclude-standard", "-z"];

/// Runs `git args` in `cwd`: its stdout, or its message.
fn git<S: AsRef<OsStr>>(cwd: &Path, args: &[S]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("git: {}", io_message(&e)))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty());
    Err(match line {
        Some(l) => l
            .strip_prefix("fatal: ")
            .or_else(|| l.strip_prefix("error: "))
            .unwrap_or(l)
            .to_owned(),
        None => format!("git exited with {}", out.status),
    })
}

/// A path from git's bytes: any bytes on Unix.
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

/// A path from git's bytes: UTF-8 (git's encoding for paths there) off
/// Unix, its `/` separators made the platform's.
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    let s = std::str::from_utf8(bytes).ok()?;
    Some(PathBuf::from(s.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_parse() {
        for name in GitChanges::NAMES {
            assert!(name.parse::<GitChanges>().is_ok(), "{name}");
        }
        assert_eq!("all".parse(), Ok(GitChanges::All));
        assert!("Staged".parse::<GitChanges>().is_err());
    }
}
