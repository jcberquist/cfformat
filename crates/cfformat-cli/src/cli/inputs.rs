//! From PATHS to the list of files to format.
//!
//! A PATH is a file (taken as given, whatever its extension, ignored or
//! not), a directory (walked for `.cfc` and `.cfs`, and `.cfm` with
//! `--cfm`, honouring `.gitignore`, the global gitignore,
//! `.git/info/exclude` and `.ignore`, hidden entries skipped, symlinks not
//! followed, in path order) or a glob (its literal prefix directory walked
//! the same way, every file the pattern matches kept). The list keeps the PATHS' order, each expanded in place,
//! and names a file once (by absolute path; the first occurrence wins).
//! `--git` replaces the PATHS: the files git reports as changed, kept by
//! the walk's extensions.

use std::path::{Component, Path, PathBuf};

use cfcli::{io_message, Dedupe, GitChanges};
use globset::GlobBuilder;

/// The resolved inputs of a run.
#[derive(Debug, Default)]
pub struct Inputs {
    /// The files, in order, each once, named as the user gave them (a walked
    /// file as its directory joined with the walk's relative path).
    pub files: Vec<PathBuf>,
    /// PATHS that could not be resolved: `nope.cfc: No such file or
    /// directory`, `no files match src/**/*.cfml`, a walk error.
    pub errors: Vec<String>,
    /// How many PATHS were directories.
    pub directories: usize,
    /// Whether a PATH was a directory or a glob.
    pub expanded: bool,
}

/// Whether a PATH is a glob: it holds `*`, `?`, `[` or `{`.
pub fn is_glob(path: &str) -> bool {
    path.contains(['*', '?', '[', '{'])
}

/// The entries of a `--files-from` list, and its errors (`entry N is
/// blank`, `entry N is not valid UTF-8`, N counting from 1 in the list's
/// order).
///
/// NUL-separated when the input holds a NUL (`git … -z`): nothing is
/// stripped, a trailing NUL ends the last entry, and an empty or
/// whitespace-only entry is an error. Else one entry per line: a trailing
/// `\r` is dropped, an empty line skipped, and a whitespace-only line is an
/// error. The entries keep their bytes: on Unix any bytes are a path; on
/// other platforms an entry that is not UTF-8 is an error. The rest of the
/// list is kept either way.
pub fn parse_list(bytes: &[u8]) -> (Vec<PathBuf>, Vec<String>) {
    let nul = bytes.contains(&0);
    let mut pieces: Vec<&[u8]> = bytes.split(|&b| b == if nul { 0 } else { b'\n' }).collect();
    // The terminator of the last entry leaves an empty piece.
    if pieces.last().is_some_and(|p| p.is_empty()) {
        pieces.pop();
    }
    let (mut entries, mut errors) = (Vec::new(), Vec::new());
    for (i, piece) in pieces.into_iter().enumerate() {
        let n = i + 1;
        let entry = if nul {
            piece
        } else {
            match piece.strip_suffix(b"\r").unwrap_or(piece) {
                [] => continue,
                line => line,
            }
        };
        if entry.iter().all(u8::is_ascii_whitespace) {
            errors.push(format!("entry {n} is blank"));
            continue;
        }
        match path_from_bytes(entry) {
            Some(path) => entries.push(path),
            None => errors.push(format!("entry {n} is not valid UTF-8")),
        }
    }
    (entries, errors)
}

/// A path from a list entry's bytes: any bytes on Unix.
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

/// A path from a list entry's bytes: UTF-8 only off Unix.
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(bytes).ok().map(PathBuf::from)
}

/// Whether a walked file is formatted: `.cfc` and `.cfs`, and `.cfm` with
/// `--cfm` (ASCII case-insensitive).
fn walked_extension(path: &Path, cfm: bool) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        e.eq_ignore_ascii_case("cfc")
            || e.eq_ignore_ascii_case("cfs")
            || (cfm && e.eq_ignore_ascii_case("cfm"))
    })
}

/// Resolves `paths` in order.
pub fn resolve(paths: &[PathBuf], cfm: bool) -> Inputs {
    let mut inputs = Inputs::default();
    let mut seen = Dedupe::default();
    let mut add = |inputs: &mut Inputs, file: PathBuf| {
        if seen.first(&file) {
            inputs.files.push(file);
        }
    };
    for path in paths {
        let text = path.to_string_lossy();
        // A path that exists is never a glob, whatever characters it holds.
        let metadata = std::fs::metadata(path);
        if metadata.is_err() && is_glob(&text) {
            inputs.expanded = true;
            match glob(&text) {
                Ok((files, errors)) if files.is_empty() && errors.is_empty() => {
                    inputs.errors.push(format!("no files match {text}"));
                }
                Ok((files, errors)) => {
                    inputs.errors.extend(errors);
                    for f in files {
                        add(&mut inputs, f);
                    }
                }
                Err(e) => inputs.errors.push(e),
            }
            continue;
        }
        match metadata {
            Ok(m) if m.is_dir() => {
                inputs.directories += 1;
                inputs.expanded = true;
                for entry in cfcli::walk(path) {
                    match entry {
                        Ok(f) if walked_extension(&f, cfm) => add(&mut inputs, f),
                        Ok(_) => {}
                        Err(e) => inputs.errors.push(e),
                    }
                }
            }
            Ok(_) => add(&mut inputs, path.clone()),
            Err(e) => inputs
                .errors
                .push(format!("{}: {}", path.display(), io_message(&e))),
        }
    }
    inputs
}

/// The files `--git` selects ([`cfcli::git_changes`], run from `cwd`),
/// kept as a walk keeps them: `.cfc` and `.cfs`, and `.cfm` with `--cfm`.
/// Counted as expanded, so the run ends with a summary even when git
/// reports nothing.
pub fn git(which: GitChanges, cfm: bool, cwd: &Path) -> Result<Inputs, String> {
    let files = cfcli::git_changes(which, cwd)?
        .into_iter()
        .filter(|f| walked_extension(f, cfm))
        .collect();
    Ok(Inputs {
        files,
        expanded: true,
        ..Inputs::default()
    })
}

/// The files `pattern` matches, relative to the current directory
/// (`literal_separator`: `*` stops at `/`, `**` crosses it), walking its
/// literal prefix directory; and the walk's errors.
fn glob(pattern: &str) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let pattern = if cfg!(windows) {
        pattern.replace('\\', "/")
    } else {
        pattern.to_owned()
    };
    let pattern = pattern.strip_prefix("./").unwrap_or(&pattern);
    let matcher = GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| format!("invalid glob {pattern}: {}", e.kind()))?
        .compile_matcher();
    let prefix: PathBuf = Path::new(pattern)
        .components()
        .take_while(|c| !matches!(c, Component::Normal(n) if is_glob(&n.to_string_lossy())))
        .collect();
    let (root, dot) = if prefix.as_os_str().is_empty() {
        (PathBuf::from("."), true)
    } else {
        (prefix, false)
    };
    let (mut files, mut errors) = (Vec::new(), Vec::new());
    if root.is_dir() {
        for entry in cfcli::walk(&root) {
            match entry {
                Ok(f) => {
                    let candidate = if dot {
                        f.strip_prefix(".").unwrap_or(&f)
                    } else {
                        &f
                    };
                    if matcher.is_match(candidate) {
                        files.push(candidate.to_path_buf());
                    }
                }
                Err(e) => errors.push(e),
            }
        }
    }
    Ok((files, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_forms() {
        let paths = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
        // Lines: `\r` dropped, an empty line skipped, a blank one an error
        // that names it; the rest kept.
        assert_eq!(
            parse_list(b"a.cfc\nb c.cfc\r\n\n  \nd.cfc"),
            (
                paths(&["a.cfc", "b c.cfc", "d.cfc"]),
                vec!["entry 4 is blank".into()]
            )
        );
        assert_eq!(parse_list(b"-\n"), (paths(&["-"]), vec![]));
        // NUL: nothing stripped, the trailing NUL ends the last entry, an
        // empty entry between two NULs is an error.
        assert_eq!(
            parse_list(b"a.cfc\0b\nc.cfc\0\0"),
            (
                paths(&["a.cfc", "b\nc.cfc"]),
                vec!["entry 3 is blank".into()]
            )
        );
        assert_eq!(
            parse_list(b"a.cfc\r\0 b.cfc \0\t\0"),
            (
                paths(&["a.cfc\r", " b.cfc "]),
                vec!["entry 3 is blank".into()]
            )
        );
        assert_eq!(parse_list(b""), (vec![], vec![]));
        // Bytes that are not UTF-8: a path on Unix, an error elsewhere.
        let (entries, errors) = parse_list(b"a\xff.cfc\nb.cfc\n");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(entries[0].as_os_str().as_bytes(), b"a\xff.cfc");
            assert_eq!((entries.len(), errors.len()), (2, 0));
        }
        #[cfg(not(unix))]
        assert_eq!(
            (entries, errors),
            (paths(&["b.cfc"]), vec!["entry 1 is not valid UTF-8".into()])
        );
    }

    /// A scratch tree: a git root holding `src/{a.cfc, b.cfm, c.cfs,
    /// gen/x.cfc, .hidden/h.cfc, notes.txt}` with `gen/` ignored.
    fn tree(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cfformat-inputs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in [
            "src/a.cfc",
            "src/b.cfm",
            "src/c.cfs",
            "src/gen/x.cfc",
            "src/.hidden/h.cfc",
            "src/notes.txt",
        ] {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x = 1;\n").unwrap();
        }
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".gitignore"), "gen/\n").unwrap();
        dir
    }

    fn names(dir: &Path, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|f| {
                f.strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn directories_files_and_globs() {
        let dir = tree("resolve");
        let src = dir.join("src");
        // A directory: .cfc and .cfs only, the ignored and the hidden
        // skipped.
        let inputs = resolve(std::slice::from_ref(&src), false);
        assert_eq!(names(&dir, &inputs.files), ["src/a.cfc", "src/c.cfs"]);
        assert_eq!((inputs.directories, inputs.expanded), (1, true));
        let inputs = resolve(std::slice::from_ref(&src), true);
        assert_eq!(
            names(&dir, &inputs.files),
            ["src/a.cfc", "src/b.cfm", "src/c.cfs"]
        );
        // A file named explicitly is taken whatever it is; a repeat is
        // dropped, the first occurrence kept.
        let inputs = resolve(
            &[
                src.join("gen/x.cfc"),
                src.join("notes.txt"),
                src.clone(),
                src.join("a.cfc"),
            ],
            false,
        );
        assert_eq!(
            names(&dir, &inputs.files),
            ["src/gen/x.cfc", "src/notes.txt", "src/a.cfc", "src/c.cfs"]
        );
        assert_eq!(inputs.directories, 1);
        // A glob keeps what it matches, without the extension filter, and
        // walks like a directory.
        let pattern = format!("{}/**/*.cf?", src.display());
        let inputs = resolve(&[PathBuf::from(&pattern)], false);
        assert_eq!(
            names(&dir, &inputs.files),
            ["src/a.cfc", "src/b.cfm", "src/c.cfs"]
        );
        assert_eq!((inputs.directories, inputs.expanded), (0, true));
        // Errors name the PATH; the rest still resolves.
        let nothing = format!("{}/**/*.cfml", src.display());
        let inputs = resolve(
            &[
                PathBuf::from(&nothing),
                src.join("nope.cfc"),
                src.join("a.cfc"),
            ],
            false,
        );
        assert_eq!(names(&dir, &inputs.files), ["src/a.cfc"]);
        assert_eq!(inputs.errors[0], format!("no files match {nothing}"));
        assert!(inputs.errors[1].starts_with(&format!("{}: ", src.join("nope.cfc").display())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn globs_and_extensions() {
        assert!(is_glob("src/**/*.cfm") && is_glob("a?.cfc") && is_glob("{a,b}.cfc"));
        assert!(is_glob("[ab].cfc") && !is_glob("src/a.cfc"));
        assert!(walked_extension(Path::new("A.CFC"), false));
        assert!(
            walked_extension(Path::new("a.cfs"), false)
                && walked_extension(Path::new("A.CfS"), false)
        );
        assert!(!walked_extension(Path::new("a.cfm"), false));
        assert!(walked_extension(Path::new("a.Cfm"), true));
        assert!(!walked_extension(Path::new("a.cfml"), true));
    }
}
