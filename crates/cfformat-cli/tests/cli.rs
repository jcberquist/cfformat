//! The command line through the real binary: the island flags (a refused
//! island, `--no-islands`, `--timing`) and the whole surface — inputs,
//! output modes, exit codes, stdin, settings, writes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A scratch directory holding a `.git` (so settings discovery stops there)
/// and a home whose `.cfformat.json` only sets `newline` to `\n` (so the
/// output is the same on every platform). The home file is only the fallback
/// of discovery: a project settings file the tests write sets `newline` too.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cfformat-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::write(dir.join("home/.cfformat.json"), r#"{"newline": "\n"}"#).unwrap();
        Scratch(resolved(&dir))
    }

    fn write(&self, name: &str, text: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Runs `cfformat` in `cwd` with the scratch home.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().unwrap()
    }

    /// Runs `cfformat` in `cwd` with `input` on stdin.
    fn run_stdin(&self, cwd: &Path, args: &[&str], input: impl AsRef<[u8]>) -> Output {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // A run that does not read stdin may exit before the write.
        match child.stdin.take().unwrap().write_all(input.as_ref()) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => panic!("{e}"),
            _ => {}
        }
        child.wait_with_output().unwrap()
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cfformat"));
        cmd.args(args)
            .current_dir(cwd)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"));
        cmd
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.0.join(name)).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A stream as text, with Windows' path separators as `/`.
/// `dir` as a child process running in it reports it: `current_dir` resolves
/// symlinks (macOS's temp directory is under `/var`, a link to `/private/var`),
/// and the paths cfformat prints are built on it, so the scratch root is
/// resolved the same way or the `<tmp>` substitution misses. Windows's
/// canonical form carries a `\\?\` prefix that nothing prints; drop it.
fn resolved(dir: &Path) -> PathBuf {
    let real = std::fs::canonicalize(dir).unwrap();
    match real.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) if cfg!(windows) => PathBuf::from(plain),
        _ => real,
    }
}

fn text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes).into_owned();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s
    }
}

fn stdout(o: &Output) -> String {
    text(&o.stdout)
}

fn stderr(o: &Output) -> String {
    text(&o.stderr)
}

/// Settings under which every file fails, on every platform: an invalid
/// `islands.js` value (formatting itself cannot fail: both parsers are
/// total). The error is the settings file's, printed once however many
/// files it fails.
const INVALID: &str = r#"{"newline": "\n", "islands.js": "oxfmt"}"#;

/// Whether `line` is the settings error of the [`INVALID`] file at `file`
/// (a path ending, `/` separators).
fn invalid_settings_error(line: &str, file: &str) -> bool {
    line.starts_with("error: ") && line.contains(&format!("{file}: `islands.js`: "))
}

/// stderr with the times of the summary (`, 0.02 s`) and of the `islands:`
/// line (`, 0.004s`) masked.
fn stderr_untimed(o: &Output) -> String {
    stderr(o)
        .lines()
        .map(|l| match l.rsplit_once(", ") {
            Some((head, t))
                if (l.contains(" files, ") && t.ends_with(" s"))
                    || (l.starts_with("islands: ") && t.ends_with('s')) =>
            {
                format!("{head}, T s")
            }
            _ => l.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const PAGE: &str = "<div>\n<script>\nvar a;\n</script>\n</div>\n";

#[test]
fn a_removed_islands_value_is_a_settings_error() {
    // The island values of earlier versions (`"prettier"`) are invalid like any other:
    // one error per settings file, every file under it failed, nothing
    // formatted.
    let s = Scratch::new("removed-values");
    s.write("src/a.cfm", PAGE);
    s.write("src/b.cfm", PAGE);
    s.write(
        "src/.cfformat.json",
        r#"{"islands.js": "prettier", "islands.css": "prettier", "islands.json": "prettier"}"#,
    );
    let out = s.run(&s.0, &["--check", "--cfm", "src"]);
    assert_eq!((out.status.code(), stdout(&out)), (Some(2), "".into()));
    let err = stderr_untimed(&out);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 2, "{err}");
    assert!(
        lines[0].starts_with("error: ")
            && lines[0].ends_with(
                "src/.cfformat.json: `islands.js`: unknown variant `prettier`, expected `oxc` or `off`"
            ),
        "{err}"
    );
    assert_eq!(lines[1], "2 files, 0 would change, 2 failed, T s");
}

#[test]
fn islands_are_formatted_by_default() {
    let s = Scratch::new("default-islands");
    s.write("off.json", r#"{"islands.js": "off"}"#);
    let run = |args: &[&str], input: &str| {
        let mut cmd = s.command(&s.0, args);
        cmd.env("PATH", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let src = "<script>var x = {a:1}</script>\n";
    let out = run(&["-", "--tags"], src);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "<script>\nvar x = { a: 1 };\n</script>\n");
    assert_eq!(stderr(&out), "");
    for args in [
        &["-", "--tags", "--no-islands"][..],
        &["-", "--tags", "--config", "off.json"],
    ] {
        let out = run(args, src);
        assert_eq!(stdout(&out), src, "{args:?}");
    }
    // `islandSyntaxError` under the defaults: the warning the golden runner
    // does not see, and the island as written.
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../cfformat/tests/fixtures/islandSyntaxError/source.cfc"
    );
    let page = s.write(
        "page.cfm",
        std::fs::read_to_string(fixture)
            .unwrap()
            .replace("\r\n", "\n"),
    );
    let out = s.run(&s.0, &[page.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    // Both sides through `text`, which reads Windows' `\` as `/`.
    assert_eq!(
        stderr(&out),
        format!(
            "{}:6: islands.js: Expected a semicolon or an implicit semicolon after a statement, but found none\n",
            text(page.display().to_string().as_bytes())
        )
    );
    assert!(
        stdout(&out).contains("    <script>\n    SYNTAX ERROR\n      here\n    </script>\n"),
        "{}",
        stdout(&out)
    );
}

/// Deep embedded code: handed to oxc unbounded, each of the first four
/// aborts the whole process with `has overflowed its stack` (exit 134); the
/// fifth, with no bracket past 1, formats into 24 MB; the next two, a
/// closer hidden in a comment or a string at every level, count 1 to a
/// byte count and format into 2 MB; the last, a `}` inside a CSS function's
/// arguments at every level, a token to the parser, counts about 2 unless
/// brackets are matched by kind and formats into 3.6 MB. Each is refused by
/// the island limits — the fifth after the parse, by its tree, the others
/// before oxc sees them, so this is safe in a debug build — and prints as
/// written with a warning; the page is otherwise clean, so `check` passes.
#[test]
fn a_deep_or_large_island_is_refused_not_an_abort() {
    let s = Scratch::new("deep-islands");
    let cases = [
        (
            "<script>",
            format!("x = {}1{};", "(".repeat(5_000), ")".repeat(5_000)),
            "</script>",
            "islands.js: nested 5000 levels deep, over the limit of 500",
        ),
        // On lines of its own: a one-line page would change anyway, its
        // tag's attribute broken onto a line to fit `max_columns`.
        (
            "<script type=\"application/json\">\n",
            format!("{}0{}", "[".repeat(5_000), "]".repeat(5_000)),
            "\n</script>",
            "islands.json: nested 5000 levels deep, over the limit of 500",
        ),
        (
            "<style>",
            format!(
                "{}.a{{color:red}}{}",
                "@media screen{".repeat(1_000),
                "}".repeat(1_000)
            ),
            "</style>",
            "islands.css: nested 1001 levels deep, over the limit of 500",
        ),
        (
            "<script>",
            format!("x = {}a;", "!".repeat(300_000)),
            "</script>",
            "islands.js: 300007 bytes, over the limit of 262144",
        ),
        (
            "<script>",
            format!("{}x;", "if(a)".repeat(2_000)),
            "</script>",
            "islands.js: nested 2003 levels deep, over the limit of 500",
        ),
        (
            "<style>",
            format!("{}color:red;{}", ".a{/* } */".repeat(600), "}".repeat(600)),
            "</style>",
            "islands.css: nested 600 levels deep, over the limit of 500",
        ),
        (
            "<script type=\"application/json\">\n",
            format!("{}0{}", "[\"]\",".repeat(600), "]".repeat(600)),
            "\n</script>",
            "islands.json: nested 600 levels deep, over the limit of 500",
        ),
        (
            "<style>",
            format!(
                "{}color:red;{}",
                ".a{b:foo(});".repeat(600),
                "}".repeat(600)
            ),
            "</style>",
            "islands.css: nested 601 levels deep, over the limit of 500",
        ),
    ];
    for (open, body, close, warning) in cases {
        let page = format!("{open}{body}{close}\n");
        s.write("page.cfm", &page);
        let line = 1 + open.matches('\n').count();
        let warning = format!("page.cfm:{line}: {warning}\n");
        let out = s.run(&s.0, &["--check", "page.cfm"]);
        assert_eq!(out.status.code(), Some(0), "{warning}{}", out.status);
        assert_eq!(stderr(&out), warning);
        let out = s.run(&s.0, &["page.cfm"]);
        assert_eq!(out.status.code(), Some(0), "{warning}{}", out.status);
        assert!(stdout(&out) == page, "{warning}: not as written");
        assert_eq!(stderr(&out), warning);
        let out = s.run(&s.0, &["page.cfm", "--no-islands"]);
        assert_eq!(out.status.code(), Some(0), "{warning}{}", out.status);
        assert!(stdout(&out) == page, "{warning}: not as written");
        assert_eq!(stderr(&out), "");
    }
}

#[test]
fn oxc_formats_in_process_with_nothing_on_path() {
    let s = Scratch::new("oxc");
    s.write(
        "oxc.json",
        r#"{"newline": "\n", "islands.js": "oxc", "islands.css": "oxc", "islands.json": "oxc"}"#,
    );
    let page =
        "<div>\n<script>\nSYNTAX ERROR\n</script>\n<script>\nvar a = {b:1}\n</script>\n</div>\n";
    s.write("site/page.cfm", page);
    let run = |args: &[&str]| s.command(&s.0, args).env("PATH", "").output().unwrap();
    let formatted = concat!(
        "<div>\n    <script>\n    SYNTAX ERROR\n    </script>\n",
        "    <script>\n    var a = { b: 1 };\n    </script>\n</div>\n"
    );
    let warning = "site/page.cfm:3: islands.js: Expected a semicolon or an implicit semicolon after a statement, but found none\n";
    let out = run(&["site/page.cfm", "--config", "oxc.json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), formatted);
    assert_eq!(stderr(&out), warning);
    // `--quiet` hides the warning; `--no-islands` prints every island as written.
    let out = run(&["site/page.cfm", "--config", "oxc.json", "--quiet"]);
    assert_eq!(
        (stdout(&out).as_str(), stderr(&out).as_str()),
        (formatted, "")
    );
    let out = run(&["site/page.cfm", "--config", "oxc.json", "--no-islands"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        "<div>\n    <script>\n    SYNTAX ERROR\n    </script>\n    <script>\n    var a = {b:1}\n    </script>\n</div>\n"
    );
    assert_eq!(stderr(&out), "");
    // `--timing`: two islands handed off, one refused, in-process time.
    let out = run(&["site/page.cfm", "--config", "oxc.json", "--timing"]);
    let err = stderr(&out);
    let timing = err.lines().nth(1).unwrap();
    assert!(
        timing.starts_with("site/page.cfm: parse ") && timing.contains(" islands 2/1 "),
        "{err}"
    );
    // A directory run prints the `islands:` line.
    let out = run(&["--check", "site", "--cfm", "--config", "oxc.json"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_untimed(&out);
    assert!(
        err.contains("islands: 2 formatted, 0 cached, 1 warnings"),
        "{err}"
    );
}

#[test]
fn oxc_reads_the_project_prettierrc() {
    let s = Scratch::new("config");
    // A trailing comma, as prettier's YAML loader accepts it.
    s.write(
        "proj/.prettierrc",
        r#"{"singleQuote": true, "semi": false,}"#,
    );
    s.write("off.json", r#"{"newline": "\n", "islands.config": "off"}"#);
    let page = "<script>var x = {\"a\": \"b\"};</script>\n";
    s.write("proj/site/page.cfm", page);
    s.write("proj/site/other.cfm", page);
    let configured = "<script>\nvar x = { a: 'b' }\n</script>\n";
    let defaults = "<script>\nvar x = { a: \"b\" };\n</script>\n";
    let out = s.run(&s.0, &["proj/site/page.cfm", "--tags"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        (stdout(&out).as_str(), stderr(&out).as_str()),
        (configured, "")
    );
    let out = s.run(
        &s.0,
        &["proj/site/page.cfm", "--tags", "--config", "off.json"],
    );
    assert_eq!(stdout(&out), defaults);
    let out = s.run(&s.0, &["proj/site/page.cfm", "--tags", "--no-islands"]);
    assert_eq!(stdout(&out), "<script>var x = {\"a\": \"b\"};</script>\n");
    // stdin resolves from the working directory.
    let out = s.run_stdin(&s.0.join("proj/site"), &["-", "--tags"], page);
    assert_eq!(stdout(&out), configured);
    let out = s.run_stdin(&s.0, &["-", "--tags"], page);
    assert_eq!(stdout(&out), defaults);

    // A `package.json` with a "prettier" key comes first in its directory.
    s.write(
        "proj/package.json",
        r#"{"name": "p", "prettier": {"semi": false}}"#,
    );
    let out = s.run(&s.0, &["proj/site/page.cfm", "--tags"]);
    assert_eq!(stdout(&out), "<script>\nvar x = { a: \"b\" }\n</script>\n");
    std::fs::remove_file(s.0.join("proj/package.json")).unwrap();

    // A YAML file is not read: one warning per run, the defaults.
    std::fs::remove_file(s.0.join("proj/.prettierrc")).unwrap();
    s.write("proj/.prettierrc.yml", "singleQuote: true\n");
    let out = s.run(&s.0, &["--check", "proj", "--cfm", "-j", "1"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    // The file as the walk found it: absolute (macOS may name the temp
    // directory through `/private`).
    let first = err.lines().next().unwrap();
    assert!(
        first.starts_with("warning: /") || first.starts_with("warning: ") && first.contains(":/"),
        "{err}"
    );
    assert!(
        first.ends_with(
            "/proj/.prettierrc.yml: not read: YAML is not supported; oxc's defaults apply"
        ),
        "{err}"
    );
    assert_eq!(err.matches("warning: ").count(), 1, "{err}");
    assert!(err.contains("2 files, 2 would change"), "{err}");
    let out = s.run(&s.0, &["--check", "proj", "--cfm", "--quiet"]);
    assert_eq!((out.status.code(), stderr(&out).as_str()), (Some(1), ""));
    let out = s.run(&s.0, &["proj/site/page.cfm", "--tags"]);
    assert_eq!(stdout(&out), defaults);
}

#[test]
fn help_lists_every_subcommand_and_flag() {
    let s = Scratch::new("help");
    let help = |args: &[&str]| {
        let out = s.run(&s.0, args);
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        stdout(&out)
    };
    let top = help(&["--help"]);
    assert!(
        top.contains("Usage: cfformat [OPTIONS] [PATHS]..."),
        "{top}"
    );
    assert!(top.contains("cfformat <COMMAND>"), "{top}");
    for word in ["--check", "doc", "settings", "--version", "Exit codes:"] {
        assert!(top.contains(word), "--help lacks {word}:\n{top}");
    }
    assert!(
        top.contains("Git: `cfformat --git staged -w` formats the staged"),
        "{top}"
    );
    assert!(top.contains("never runs `git add`"), "{top}");
    let format_flags = [
        "--check",
        "--diff",
        "--cfm",
        "--config <FILE>",
        "--stdin ",
        "--stdin-filepath <PATH>",
        "--files-from <FILE|->",
        "--git <WHICH>",
        "[possible values: staged, unstaged, all]",
        "-j, --jobs <N>",
        "--quiet",
        "--timing",
        "--no-islands",
        "--script",
        "--tags",
        "[PATHS]...",
    ];
    for flag in format_flags.iter().chain(&["-w, --write"]) {
        assert!(top.contains(flag), "--help lacks {flag}:\n{top}");
    }
    let doc = help(&["doc", "--help"]);
    for flag in [
        "<FILE|->",
        "--config <FILE>",
        "--no-islands",
        "--timing",
        "--script",
        "--tags",
    ] {
        assert!(doc.contains(flag), "doc --help lacks {flag}:\n{doc}");
    }
    let settings = help(&["settings", "--help"]);
    for flag in [
        "[PATH|-]",
        "--config <FILE>",
        "--defaults",
        "--schema",
        "--migrate",
    ] {
        assert!(
            settings.contains(flag),
            "settings --help lacks {flag}:\n{settings}"
        );
    }
    // `--git` selects inputs; `doc` and `settings` take one path.
    assert!(!doc.contains("--git") && !settings.contains("--git"));
    let version = help(&["--version"]);
    assert_eq!(version, format!("cfformat {}\n", env!("CARGO_PKG_VERSION")));
    // A usage error is exit 2 on stderr.
    let out = s.run(&s.0, &["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).starts_with("error: unexpected argument '--bogus'"));
}

/// A component the formatter changes, and its formatted text.
const UGLY: &str = "component {\nfunction f(){return 1;}\n}\n";
const PRETTY: &str = "component {\n\n    function f() {\n        return 1;\n    }\n\n}\n";

#[test]
fn one_file_prints_several_files_need_a_mode() {
    let s = Scratch::new("stdout");
    s.write("a.cfc", UGLY);
    s.write("b.cfc", PRETTY);
    let out = s.run(&s.0, &["a.cfc"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(0), PRETTY.into(), "".into())
    );
    let out = s.run(&s.0, &["a.cfc", "b.cfc"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr(&out),
        "error: 2 files given; use -w, --check or --diff\n"
    );
    assert_eq!(stdout(&out), "");
}

#[test]
fn write_touches_only_what_changes_then_check_is_clean() {
    let s = Scratch::new("write");
    s.write("src/a.cfc", UGLY);
    s.write("src/b.cfc", PRETTY);
    s.write("src/sub/c.cfc", UGLY);
    let mtime = |name: &str| {
        std::fs::metadata(s.0.join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    let before = mtime("src/b.cfc");

    let out = s.run(&s.0, &["--check", "src"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "src/a.cfc\nsrc/sub/c.cfc\n");
    assert_eq!(
        stderr_untimed(&out),
        "3 files, 2 would change, 0 failed, T s"
    );
    // `fmt --check` is `check`.
    let again = s.run(&s.0, &["--check", "src"]);
    assert_eq!(
        (again.status.code(), stdout(&again)),
        (Some(1), stdout(&out))
    );

    let out = s.run(&s.0, &["-w", "src"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr_untimed(&out), "3 files, 2 written, 0 failed, T s");
    assert_eq!(s.read("src/a.cfc"), PRETTY);
    assert_eq!(s.read("src/sub/c.cfc"), PRETTY);
    assert_eq!(mtime("src/b.cfc"), before);

    let out = s.run(&s.0, &["-w", "src"]);
    assert_eq!(stderr_untimed(&out), "3 files, 0 written, 0 failed, T s");
    let out = s.run(&s.0, &["--check", "src"]);
    assert_eq!((out.status.code(), stdout(&out)), (Some(0), "".into()));
    assert_eq!(
        stderr_untimed(&out),
        "3 files, 0 would change, 0 failed, T s"
    );
}

#[test]
fn diff_prints_a_unified_diff() {
    let s = Scratch::new("diff");
    s.write("page.cfc", UGLY);
    s.write("done.cfc", PRETTY);
    let out = s.run(&s.0, &["--diff", "page.cfc"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        concat!(
            "--- a/page.cfc\n+++ b/page.cfc\n@@ -1,3 +1,7 @@\n component {\n",
            "-function f(){return 1;}\n+\n+    function f() {\n+        return 1;\n+    }\n+\n }\n"
        )
    );
    assert_eq!(stderr(&out), "");
    let out = s.run(&s.0, &["--diff", "done.cfc"]);
    assert_eq!((out.status.code(), stdout(&out)), (Some(0), "".into()));
    // `--check --diff`: the diffs, check's exit code.
    let out = s.run(&s.0, &["--check", "--diff", "page.cfc", "done.cfc"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).starts_with("--- a/page.cfc\n"));
    assert_eq!(
        stderr_untimed(&out),
        "2 files, 1 would change, 0 failed, T s"
    );
}

#[test]
fn directories_honour_gitignore_and_named_files_do_not() {
    let s = Scratch::new("ignore");
    s.write(".gitignore", "src/gen/\n");
    s.write("src/a.cfc", UGLY);
    s.write("src/gen/x.cfc", UGLY);
    s.write("src/.hidden/y.cfc", UGLY);
    s.write("src/v.cfm", "<div><p>x</p></div>\n");
    let out = s.run(&s.0, &["--check", "src"]);
    assert_eq!(stdout(&out), "src/a.cfc\n");
    let out = s.run(&s.0, &["--check", "src/gen/x.cfc"]);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(1), "src/gen/x.cfc\n".into())
    );
    // No summary for one named file.
    assert_eq!(stderr(&out), "");
    // A directory named twice, or through a file inside it, is walked once.
    let out = s.run(&s.0, &["--check", "src/a.cfc", "src", "./src"]);
    assert_eq!(stdout(&out), "src/a.cfc\n");
    assert_eq!(
        stderr_untimed(&out),
        "1 files, 1 would change, 0 failed, T s"
    );
    // --cfm walks .cfm files too, and needs a directory.
    let out = s.run(&s.0, &["--check", "--cfm", "src"]);
    assert_eq!(stdout(&out), "src/a.cfc\nsrc/v.cfm\n");
    let out = s.run(&s.0, &["--cfm", "src/a.cfc"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).starts_with("error: --cfm "),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_file_name_decides_the_mode_where_it_is_certain() {
    let s = Scratch::new("mode-by-name");
    // Plain statements: the source alone reads them as a tag template.
    s.write("a.cfs", "x=1;\n");
    // A leading `//`: the source alone reads it as script.
    s.write("b.cfm", "// c\nx=1;\n");
    let run = |args: &[&str]| {
        let out = s.run(&s.0, args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        stdout(&out)
    };
    assert_eq!(run(&["a.cfs"]), "x = 1;\n");
    assert_eq!(run(&["b.cfm"]), "// c\nx=1;\n");
    // The flags win over the name.
    assert_eq!(run(&["--tags", "a.cfs"]), "x=1;\n");
    assert_eq!(run(&["--script", "b.cfm"]), "// c\nx = 1;\n");
    // `doc` follows the same rule.
    assert_ne!(run(&["doc", "a.cfs"]), run(&["doc", "--tags", "a.cfs"]));
    // stdin: by the name `--stdin-filepath` gives, sniffed without one.
    let piped = |args: &[&str], src: &str| stdout(&s.run_stdin(&s.0, args, src));
    assert_eq!(
        piped(&["--stdin-filepath", "new.cfs"], "x=1;\n"),
        "x = 1;\n"
    );
    assert_eq!(piped(&["-"], "x=1;\n"), "x=1;\n");
    assert_eq!(
        piped(&["--stdin-filepath", "new.CFM"], "// c\nx=1;\n"),
        "// c\nx=1;\n"
    );
    assert_eq!(piped(&["-"], "// c\nx=1;\n"), "// c\nx = 1;\n");
    // A walk takes `.cfs` without `--cfm`, and formats it as script.
    s.write("src/c.cfs", "x=1;\n");
    s.write("src/d.cfm", "// c\nx=1;\n");
    let out = s.run(&s.0, &["--check", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(1), "src/c.cfs\n".into())
    );
    // `arrange` walks it too, and a `.cfs` holds no component to arrange.
    let out = s.run(&s.0, &["arrange", "--check", "src"]);
    assert_eq!((out.status.code(), stdout(&out)), (Some(0), "".into()));
    assert_eq!(
        stderr_untimed(&out),
        "1 files, 0 would change, 0 failed, T s"
    );
}

#[test]
fn globs_match_without_the_extension_filter() {
    let s = Scratch::new("glob");
    s.write("src/a.cfc", UGLY);
    s.write("src/v.cfm", "<div><p>x</p></div>\n");
    s.write("src/deep/w.cfm", "<div><p>x</p></div>\n");
    s.write("other/z.cfm", "<div><p>x</p></div>\n");
    let out = s.run(&s.0, &["--check", "src/**/*.cfm"]);
    assert_eq!(stdout(&out), "src/deep/w.cfm\nsrc/v.cfm\n");
    assert_eq!(
        stderr_untimed(&out),
        "2 files, 2 would change, 0 failed, T s"
    );
    let out = s.run(&s.0, &["--check", "*/*.cfm"]);
    assert_eq!(stdout(&out), "other/z.cfm\nsrc/v.cfm\n");
    let out = s.run(&s.0, &["--check", "src/**/*.cfml"]);
    assert_eq!(out.status.code(), Some(2));
    // Nothing to run: the error and no summary.
    assert_eq!(stderr(&out), "error: no files match src/**/*.cfml\n");
}

#[test]
fn files_from_takes_lines_or_nul_separated_paths() {
    let s = Scratch::new("files-from");
    s.write("a.cfc", UGLY);
    s.write("b.cfc", UGLY);
    s.write("c.cfc", PRETTY);
    let out = s.run_stdin(
        &s.0,
        &["--check", "--files-from", "-"],
        "a.cfc\0b.cfc\0c.cfc\0",
    );
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(1), "a.cfc\nb.cfc\n".into())
    );
    s.write("list.txt", "a.cfc\n\nb.cfc\n");
    let out = s.run(&s.0, &["--check", "--files-from", "list.txt"]);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(1), "a.cfc\nb.cfc\n".into())
    );
    // A directory PATH with a list is not the report: several files need a
    // mode.
    s.write("src/d.cfc", UGLY);
    s.write("src/e.cfc", UGLY);
    let out = s.run_stdin(&s.0, &["src", "--files-from", "-"], "a.cfc\n");
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (
            Some(2),
            "".into(),
            "error: 3 files given; use -w, --check or --diff\n".into()
        )
    );
    // A missing entry is an error; the others are still checked.
    let out = s.run_stdin(&s.0, &["--check", "--files-from", "-"], "nope.cfc\na.cfc\n");
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(2), "a.cfc\n".into())
    );
    let err = stderr(&out);
    assert!(err.starts_with("error: nope.cfc: "), "{err}");
    #[cfg(unix)]
    assert!(
        err.starts_with("error: nope.cfc: No such file or directory\n"),
        "{err}"
    );
    // -w over the list: the git recipe.
    let out = s.run_stdin(&s.0, &["--files-from", "-", "-w"], "a.cfc\0");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(s.read("a.cfc"), PRETTY);
    // A NUL-separated entry is taken as it is: its `\r` is part of the name.
    let out = s.run_stdin(&s.0, &["--check", "--files-from", "-"], "b.cfc\r\0c.cfc\0");
    assert_eq!((out.status.code(), stdout(&out)), (Some(2), "".into()));
    assert!(
        stderr(&out).starts_with("error: b.cfc\r: "),
        "{}",
        stderr(&out)
    );
    // A blank entry is an error naming it; the rest still runs.
    let out = s.run_stdin(&s.0, &["--check", "--files-from", "-"], "  \nb.cfc\n");
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (
            Some(2),
            "b.cfc\n".into(),
            "error: <stdin>: entry 1 is blank\n".into()
        )
    );
    s.write("blank.txt", "a.cfc\0\0");
    let out = s.run(&s.0, &["--check", "--files-from", "blank.txt"]);
    assert_eq!(
        (out.status.code(), stderr(&out)),
        (Some(2), "error: blank.txt: entry 2 is blank\n".into())
    );
}

#[test]
fn a_file_named_dash_in_a_list_is_a_file() {
    // Only a positional `-` is stdin. A list naming `-` names the file `-`:
    // it is read, checked and written like any other, never stdin.
    let s = Scratch::new("dash-file");
    s.write("-", "x=1;\n");
    let out = s.run_stdin(&s.0, &["--check", "--script", "--files-from", "-"], "-\n");
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(1), "-\n".into(), "".into())
    );
    // The list in a file, and something else on stdin: the file `-` gets
    // its own text formatted, not stdin's.
    s.write("list.txt", "-\n");
    let out = s.run_stdin(
        &s.0,
        &["--script", "--files-from", "list.txt", "-w"],
        "y=2;\n",
    );
    assert_eq!((out.status.code(), stderr(&out)), (Some(0), "".into()));
    assert_eq!(s.read("-"), "x = 1;\n");
    // Its errors name it `-`.
    std::fs::remove_file(s.0.join("-")).unwrap();
    let out = s.run_stdin(&s.0, &["--check", "--files-from", "list.txt"], "y=2;\n");
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).starts_with("error: -: "), "{}", stderr(&out));
    #[cfg(unix)]
    assert_eq!(stderr(&out), "error: -: No such file or directory\n");
}

#[test]
fn invalid_utf8_is_an_error_never_replaced() {
    // No replacement character, no transcoding: the file is reported and
    // left alone, stdin prints nothing.
    let s = Scratch::new("utf8");
    let latin1 = b"x='\xff';\n";
    for (args, err) in [
        (
            &["--stdin", "--script"][..],
            "error: <stdin>: not valid UTF-8\n",
        ),
        (
            &["--check", "--stdin", "--script"],
            "<stdin>: not valid UTF-8\n",
        ),
        (
            &["doc", "-", "--script"],
            "error: <stdin>: not valid UTF-8\n",
        ),
    ] {
        let out = s.run_stdin(&s.0, args, latin1);
        assert_eq!(
            (out.status.code(), stdout(&out), stderr(&out)),
            (Some(2), "".into(), err.into()),
            "{args:?}"
        );
    }
    s.write("src/a.cfc", latin1);
    s.write("src/b.cfc", UGLY);
    let out = s.run(&s.0, &["--check", "src/a.cfc"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(2), "".into(), "src/a.cfc: not valid UTF-8\n".into())
    );
    let out = s.run(&s.0, &["--script", "-w", "src"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_untimed(&out),
        "src/a.cfc: not valid UTF-8\n2 files, 1 written, 1 failed, T s"
    );
    assert_eq!(std::fs::read(s.0.join("src/a.cfc")).unwrap(), latin1);
    assert_eq!(s.read("src/b.cfc"), PRETTY);
}

#[test]
fn usage_errors_and_the_explicit_current_directory() {
    let s = Scratch::new("usage");
    s.write("src/a.cfc", UGLY);
    let out = s.run(&s.0, &["--cfm"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr(&out),
        "error: no input given; name files, directories or globs (e.g. `cfformat .`), or use --stdin / --files-from / --git\n"
    );
    for args in [
        &["-w", "--stdin"][..],
        &["-w", "-"],
        &["-w", "--check", "src"],
        &["--cfm", "src/a.cfc"],
        &["-", "src/a.cfc"],
        &["-j", "0", "src"],
        &["--check", "--write", "src"],
    ] {
        let out = s.run_stdin(&s.0, args, "");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&out), "", "{args:?}");
    }
    // `.` is an ordinary directory.
    let out = s.run(&s.0.join("src"), &["-w", "."]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr_untimed(&out), "1 files, 1 written, 0 failed, T s");
    assert_eq!(s.read("src/a.cfc"), PRETTY);
}

#[test]
fn errors_name_the_file_and_quiet_keeps_only_them() {
    let s = Scratch::new("errors");
    s.write("src/a.cfc", UGLY);
    s.write("src/bad/.cfformat.json", INVALID);
    s.write("src/bad/c.cfm", PAGE);
    s.write("src/bad/d.cfm", PAGE);
    s.write("src/b.cfc", PRETTY);
    let out = s.run(&s.0, &["--check", "--cfm", "src"]);
    // 2 wins over 1.
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "src/a.cfc\n");
    let err = stderr_untimed(&out);
    let lines: Vec<&str> = err.lines().collect();
    // The settings error once, both of its files failed.
    assert_eq!(lines.len(), 2, "{err}");
    assert!(
        invalid_settings_error(lines[0], "src/bad/.cfformat.json"),
        "{err}"
    );
    assert_eq!(lines[1], "4 files, 1 would change, 2 failed, T s");
    let out = s.run(&s.0, &["-w", "--quiet", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(0), "".into(), "".into())
    );
    let out = s.run(&s.0, &["-w", "--quiet", "--cfm", "src"]);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(
        invalid_settings_error(&err, "src/bad/.cfformat.json"),
        "{err}"
    );
    // --quiet keeps check's list.
    s.write("src/a.cfc", UGLY);
    let out = s.run(&s.0, &["--check", "--quiet", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(1), "src/a.cfc\n".into(), "".into())
    );
}

#[test]
fn a_view_partial_formats() {
    let s = Scratch::new("partial");
    s.write("views/a.cfm", "  </cfif>\n<div>x</div>\n<cfif y>\n");
    // The stray closer and the unclosed tag are recoveries: each
    // prints as written with a warning, and the file formats around them.
    let out = s.run(&s.0, &["--check", "--cfm", "views"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(1),
            "views/a.cfm\n".into(),
            "views/a.cfm:1: not formatted: a stray closer\n\
             views/a.cfm:3: not formatted: an unclosed block\n\
             1 files, 1 would change, 0 failed, 2 not formatted, T s"
                .into()
        )
    );
    let out = s.run(&s.0, &["-w", "--cfm", "views"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(s.read("views/a.cfm"), "</cfif>\n<div>x</div>\n<cfif y>\n");
    let out = s.run(&s.0, &["--check", "--cfm", "views"]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn stdin_reads_one_source() {
    let s = Scratch::new("stdin");
    for args in [&["-"][..], &["--stdin"]] {
        let out = s.run_stdin(&s.0, args, UGLY);
        assert_eq!(
            (out.status.code(), stdout(&out)),
            (Some(0), PRETTY.into()),
            "{args:?}"
        );
    }
    // Settings come from the current directory; --check names `<stdin>`.
    s.write(
        "sub/.cfformat.json",
        r#"{"newline": "\n", "indent_size": 2}"#,
    );
    let out = s.run_stdin(
        &s.0.join("sub"),
        &["--check", "--stdin", "--script"],
        "x=1;\n",
    );
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(1), "<stdin>\n".into())
    );
    let out = s.run_stdin(&s.0.join("sub"), &["-"], UGLY);
    assert!(
        stdout(&out).contains("\n  function f() {\n"),
        "{}",
        stdout(&out)
    );
    let out = s.run_stdin(&s.0, &["--diff", "-"], UGLY);
    assert!(stdout(&out).starts_with("--- a/<stdin>\n+++ b/<stdin>\n"));
    let out = s.run_stdin(&s.0, &["-", "--timing"], UGLY);
    assert!(
        stderr(&out).starts_with("<stdin>: parse "),
        "{}",
        stderr(&out)
    );
    // The current directory's settings error fails stdin.
    s.write("bad/.cfformat.json", INVALID);
    let out = s.run_stdin(&s.0.join("bad"), &["-", "--tags"], PAGE);
    assert_eq!((out.status.code(), stdout(&out)), (Some(2), "".into()));
    assert!(
        invalid_settings_error(&stderr(&out), "bad/.cfformat.json"),
        "{}",
        stderr(&out)
    );
}

/// Whether `line` is `--timing`'s line for `name`: `name: parse Xms doc Xms
/// print Xms islands N/W Xms total Xms`.
fn is_timing_line(line: &str, name: &str) -> bool {
    let Some(rest) = line.strip_prefix(&format!("{name}: ")) else {
        return false;
    };
    let fields: Vec<&str> = rest.split(' ').collect();
    let time = |t: &str| {
        t.strip_suffix("ms")
            .is_some_and(|n| n.parse::<f64>().is_ok())
    };
    let islands = |t: &str| {
        t.split_once('/')
            .is_some_and(|(n, w)| n.parse::<u32>().is_ok() && w.parse::<u32>().is_ok())
    };
    matches!(
        fields[..],
        ["parse", a, "doc", b, "print", c, "islands", n, d, "total", e]
            if [a, b, c, d, e].into_iter().all(time) && islands(n)
    )
}

#[test]
fn timing_prints_a_total_per_file_and_the_summary() {
    let s = Scratch::new("timing");
    s.write("a.cfc", UGLY);
    let summary =
        "1 files, 1 would change, 0 failed, T s\nislands: 0 formatted, 0 cached, 0 warnings, T s";
    let check = |out: &Output, name: &str, rest: &str| {
        assert_eq!(out.status.code(), Some(0), "{}", stderr(out));
        let err = stderr_untimed(out);
        let (first, others) = err.split_once('\n').unwrap_or((&err, ""));
        assert!(is_timing_line(first, name), "{err}");
        assert_eq!(others, rest, "{err}");
    };
    // One file on stdout, and stdin: the file's line with its total, then
    // the summary, which only `--timing` prints for them.
    let out = s.run(&s.0, &["--timing", "a.cfc"]);
    assert_eq!(stdout(&out), PRETTY);
    check(&out, "a.cfc", summary);
    let out = s.run_stdin(&s.0, &["--timing", "--stdin"], UGLY);
    assert_eq!(stdout(&out), PRETTY);
    check(&out, "<stdin>", summary);
    for args in [&["a.cfc"][..], &["--stdin"]] {
        let out = s.run_stdin(&s.0, args, UGLY);
        assert_eq!((stdout(&out), stderr(&out)), (PRETTY.into(), "".into()));
    }
    // `--quiet` keeps the times alone.
    let out = s.run(&s.0, &["--timing", "--quiet", "a.cfc"]);
    check(&out, "a.cfc", "");
    // Several files: a line each, then the summary as without `--timing`.
    s.write("b.cfc", PRETTY);
    let out = s.run(&s.0, &["--timing", "--check", "a.cfc", "b.cfc"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_untimed(&out);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 4, "{err}");
    assert!(is_timing_line(lines[0], "a.cfc") && is_timing_line(lines[1], "b.cfc"));
    assert_eq!(
        lines[2..],
        [
            "2 files, 1 would change, 0 failed, T s",
            "islands: 0 formatted, 0 cached, 0 warnings, T s"
        ]
    );
    // `doc`: the line with its total, no summary.
    let out = s.run(&s.0, &["doc", "--timing", "a.cfc"]);
    check(&out, "a.cfc", "");
    assert!(!is_timing_line(
        "a.cfc: parse 1.0ms doc 1.0ms print 1.0ms islands 0/0 0.0ms",
        "a.cfc"
    ));
}

#[test]
fn stdin_filepath_names_the_file_it_stands_for() {
    let s = Scratch::new("stdin-filepath");
    s.write(
        "site/.cfformat.json",
        r#"{"newline": "\n", "indent_size": 2}"#,
    );
    let page = "<div>\n<script>\nvar a\n</script>\n<script>\nSYNTAX ERROR\n</script>\n</div>\n";
    // site/page.cfm does not exist: its settings apply, and it names the
    // warnings and the timing line.
    let out = s.run_stdin(
        &s.0,
        &["--stdin-filepath", "site/page.cfm", "-", "--timing"],
        page,
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        concat!(
            "<div>\n  <script>\n  var a;\n  </script>\n",
            "  <script>\n  SYNTAX ERROR\n  </script>\n</div>\n"
        )
    );
    let refused = "islands.js: Expected a semicolon or an implicit semicolon after a statement, but found none";
    let err = stderr(&out);
    let mut lines = err.lines();
    assert_eq!(
        lines.next(),
        Some(format!("site/page.cfm:6: {refused}").as_str())
    );
    assert!(
        lines.next().unwrap().starts_with("site/page.cfm: parse "),
        "{err}"
    );
    // Plain `-` has no settings file here and names `<stdin>`.
    let out = s.run_stdin(
        &s.0.join("site"),
        &["-", "--config", ".cfformat.json"],
        page,
    );
    assert_eq!(stderr(&out), format!("<stdin>:6: {refused}\n"));
    // It reads stdin: no other input, no -w.
    for args in [
        &["--stdin-filepath", "a.cfc", "b.cfc"][..],
        &["-w", "--stdin-filepath", "a.cfc"],
    ] {
        let out = s.run_stdin(&s.0, args, "");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn output_order_does_not_depend_on_the_thread_count() {
    let s = Scratch::new("jobs");
    for d in 0..4 {
        // A removed key: one warning per settings file per run.
        s.write(
            &format!("src/d{d}/.cfformat.json"),
            r#"{"newline": "\n", "keywords.spacing_to_group": true}"#,
        );
        // An invalid value: one error per settings file per run.
        s.write(&format!("src/d{d}/bad/.cfformat.json"), INVALID);
        for f in 0..10 {
            let (dir, text) = match f % 4 {
                0 => ("", UGLY),
                1 => ("", PRETTY),
                2 => ("bad/", PAGE),
                _ => ("", "<div><p>x</p></div>\n"),
            };
            let ext = if f % 4 >= 2 { "cfm" } else { "cfc" };
            s.write(&format!("src/d{d}/{dir}f{f}.{ext}"), text);
        }
    }
    for f in 0..8 {
        let body = if f % 3 == 0 { "SYNTAX ERROR" } else { "var a;" };
        s.write(
            &format!("src/islands/p{f}.cfm"),
            format!("<div>\n<script>\n{body}\n</script>\n</div>\n"),
        );
    }
    let run = |jobs: &str| {
        let out = s.run(&s.0, &["--check", "--cfm", "-j", jobs, "src"]);
        (out.status.code(), stdout(&out), stderr_untimed(&out))
    };
    let serial = run("1");
    assert_eq!(serial.0, Some(2));
    assert_eq!(serial.2.matches("warning: ").count(), 4, "{}", serial.2);
    assert_eq!(serial.2.matches("error: ").count(), 4, "{}", serial.2);
    assert!(serial.2.contains(" 8 failed, "), "{}", serial.2);
    // Identical islands run the formatter once, whatever the threads.
    assert!(
        serial
            .2
            .contains("islands: 2 formatted, 6 cached, 3 warnings"),
        "{}",
        serial.2
    );
    for jobs in ["4", "8"] {
        assert_eq!(run(jobs), serial, "-j {jobs}");
    }
}

#[test]
fn settings_discovery_is_first_found_wins() {
    let s = Scratch::new("discovery");
    // The scratch home's file sets only `newline`; give it a second key.
    s.write(
        "home/.cfformat.json",
        r#"{"newline": "\n", "tab_indent": true}"#,
    );
    s.write("p/.cfformat.json", r#"{"indent_size": 2}"#);
    s.write("p/src/A.cfc", "");
    s.write("loose/B.cfc", "");
    s.write("inline.json", r#"{"max_columns": 80}"#);
    let settings = |args: &[&str]| {
        let out = s.run(&s.0, args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        let options: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let sources = stderr(&out).replace(&text(s.0.as_os_str().as_encoded_bytes()), "<tmp>");
        (sources, options)
    };
    // A project file alone: none of the home file's keys.
    let (sources, o) = settings(&["settings", "p/src/A.cfc"]);
    assert_eq!(sources, "sources:\n  <tmp>/p/.cfformat.json\n");
    assert_eq!(
        (o["indent_size"].as_u64(), o["tab_indent"].as_bool()),
        (Some(2), Some(false))
    );
    // No project file (the scratch `.git` ends the walk): the home file.
    let (sources, o) = settings(&["settings", "loose/B.cfc"]);
    assert_eq!(sources, "sources:\n  <tmp>/home/.cfformat.json\n");
    assert_eq!(o["tab_indent"].as_bool(), Some(true));
    // `--config` over either.
    let (sources, o) = settings(&["settings", "p/src/A.cfc", "--config", "inline.json"]);
    assert_eq!(
        sources,
        "sources:\n  <tmp>/p/.cfformat.json\n  <tmp>/inline.json\n"
    );
    assert_eq!(
        (o["indent_size"].as_u64(), o["max_columns"].as_u64()),
        (Some(2), Some(80))
    );
    let (sources, _) = settings(&["settings", "loose", "--config", "inline.json"]);
    assert_eq!(
        sources,
        "sources:\n  <tmp>/home/.cfformat.json\n  <tmp>/inline.json\n"
    );
}

/// A CommandBox settings file with removed and merged keys (the padding
/// keys `parentheses.padding` replaced, one key no longer configurable).
const OLD_SETTINGS: &str = r#"{
    "array.empty_padding":true,
    "array.padding":true,
    "brackets.padding":true,
    "function_anonymous.padding": true,
    "function_call.padding":true,
    "function_declaration.padding":true,
    "keywords.padding_inside_group":true,
    "parentheses.padding":true,
    "struct.empty_padding":true,
    "struct.padding":true,
    "strings.quote": "single"
}"#;

#[test]
fn settings_migrate_rewrites_an_old_file_once() {
    let s = Scratch::new("migrate");
    let p = s.0.join("p");
    s.write("p/.cfformat.json", OLD_SETTINGS);
    s.write("p/a.cfc", UGLY);
    let modified = |name: &str| {
        std::fs::metadata(s.0.join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    let effective = |cwd: &Path, args: &[&str]| {
        let out = s.run(cwd, args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        let err = stderr(&out).replace(&text(s.0.as_os_str().as_encoded_bytes()), "<tmp>");
        (stdout(&out), err)
    };
    let (options, warned) = effective(&p, &["settings"]);
    assert_eq!(warned.matches("warning: ").count(), 4, "{warned}");
    let (formatted, _) = effective(&p, &["a.cfc"]);

    // FILE defaults to `.cfformat.json` here: each warning as discovery
    // prints it, then the count.
    let out = s.run(&p, &["settings", "--migrate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        "warning: .cfformat.json: `keywords.padding_inside_group`: is no longer configurable; keyword groups follow `parentheses.padding`\n\
         warning: .cfformat.json: `function_anonymous.padding`: is replaced by `parentheses.padding`, which is already set; ignored\n\
         warning: .cfformat.json: `function_call.padding`: is replaced by `parentheses.padding`, which is already set; ignored\n\
         warning: .cfformat.json: `function_declaration.padding`: is replaced by `parentheses.padding`, which is already set; ignored\n\
         .cfformat.json: 4 keys migrated\n"
    );
    // The surviving keys in the file's order, pretty-printed.
    assert_eq!(
        s.read("p/.cfformat.json"),
        "{\n  \"array.empty_padding\": true,\n  \"array.padding\": true,\n  \"brackets.padding\": true,\n  \
         \"parentheses.padding\": true,\n  \"struct.empty_padding\": true,\n  \"struct.padding\": true,\n  \
         \"strings.quote\": \"single\"\n}\n"
    );
    // It now loads without a warning, to the same options and output.
    assert_eq!(
        effective(&p, &["settings"]),
        (options, "sources:\n  <tmp>/p/.cfformat.json\n".to_owned())
    );
    assert_eq!(effective(&p, &["a.cfc"]), (formatted, String::new()));
    // A second run has nothing to do and leaves the file alone.
    let before = modified("p/.cfformat.json");
    let out = s.run(&p, &["settings", "--migrate", ".cfformat.json"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr(&out), ".cfformat.json: nothing to migrate\n");
    assert_eq!(modified("p/.cfformat.json"), before);

    // The per-construct comma keys become one `multiline.comma`; a rename
    // that is only a spelling change migrates silently.
    s.write(
        "q/.cfformat.json",
        r#"{"newline": "\n", "array.multiline.leading_comma": true, "strings.convertNestedQuotes": "never", "struct.multiline.leading_comma": true, "indent_size": 2}"#,
    );
    let (options, warned) = effective(&s.0, &["settings", "q"]);
    let out = s.run(&s.0, &["settings", "--migrate", "q/.cfformat.json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stderr(&out), "q/.cfformat.json: 3 keys migrated\n");
    assert_eq!(
        s.read("q/.cfformat.json"),
        "{\n  \"newline\": \"\\n\",\n  \"indent_size\": 2,\n  \"multiline.comma\": \"leading\",\n  \
         \"strings.convert_nested_quotes\": \"never\"\n}\n"
    );
    assert_eq!(warned, "sources:\n  <tmp>/q/.cfformat.json\n");
    assert_eq!(effective(&s.0, &["settings", "q"]), (options, warned));

    // `strings.convert_nested_quotes` took booleans before it took strings:
    // they load with a warning on every run, and the value is rewritten in
    // place.
    s.write(
        "b/.cfformat.json",
        r#"{"strings.convert_nested_quotes": true, "indent_size": 2}"#,
    );
    let (options, warned) = effective(&s.0, &["settings", "b"]);
    let warning = "`strings.convert_nested_quotes`: `true` is now `\"always\"`";
    assert!(
        warned.contains(&format!("warning: <tmp>/b/.cfformat.json: {warning}\n")),
        "{warned}"
    );
    assert!(
        options.contains("\"strings.convert_nested_quotes\": \"always\""),
        "{options}"
    );
    let out = s.run(&s.0, &["settings", "--migrate", "b/.cfformat.json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        format!("warning: b/.cfformat.json: {warning}\nb/.cfformat.json: 1 key migrated\n")
    );
    assert_eq!(
        s.read("b/.cfformat.json"),
        "{\n  \"strings.convert_nested_quotes\": \"always\",\n  \"indent_size\": 2\n}\n"
    );
    assert_eq!(
        effective(&s.0, &["settings", "b"]),
        (options, "sources:\n  <tmp>/b/.cfformat.json\n".to_owned())
    );

    // `-`: the object from stdin, migrated, on stdout.
    let out = s.run_stdin(
        &s.0,
        &["settings", "--migrate", "-"],
        r#"{"function_call.padding": true}"#,
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "{\n  \"parentheses.padding\": true\n}\n");
    assert_eq!(
        stderr(&out),
        "warning: <stdin>: `function_call.padding`: is now `parentheses.padding`\n<stdin>: 1 key migrated\n"
    );

    // A file that does not load is a settings error, exit 2, left as it was.
    for (name, json, error) in [
        (
            "bogus.json",
            r#"{"bogus": 1, "function_call.padding": true}"#,
            "error: bogus.json: unknown field `bogus`",
        ),
        (
            "value.json",
            r#"{"indent_size": "x", "function_call.padding": true}"#,
            "error: value.json: `indent_size`: invalid type",
        ),
        (
            "quotes.json",
            r#"{"strings.convert_nested_quotes": "sometimes", "function_call.padding": true}"#,
            "error: quotes.json: `strings.convert_nested_quotes`: unknown variant `sometimes`",
        ),
        (
            "array.json",
            "[1]",
            "error: array.json: expected a JSON object",
        ),
        ("broken.json", "{", "error: broken.json: EOF while parsing"),
    ] {
        s.write(name, json);
        let out = s.run(&s.0, &["settings", "--migrate", name]);
        assert_eq!(out.status.code(), Some(2), "{name}");
        assert!(stderr(&out).starts_with(error), "{name}: {}", stderr(&out));
        assert_eq!(s.read(name), json);
    }
    let out = s.run(&s.0, &["settings", "--migrate", "missing.json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).starts_with("error: missing.json: "));
    // `--migrate` names a file to rewrite; the other modes print.
    for flag in ["--defaults", "--schema", "--config"] {
        let out = s.run(&s.0, &["settings", "--migrate", flag, "x.json"]);
        assert_eq!(out.status.code(), Some(2), "{flag}");
        assert!(stderr(&out).contains("cannot be used with"), "{flag}");
    }
}

/// CommandBox's attribute alignments are options, not removed keys: a file
/// naming them loads without a warning and has nothing to migrate.
#[test]
fn settings_migrate_keeps_the_attribute_alignments() {
    let s = Scratch::new("migrate-alignments");
    let json = "{\n  \"alignment.consecutive.params\": true,\n  \"alignment.consecutive.properties\": true\n}\n";
    s.write("p/.cfformat.json", json);
    let p = s.0.join("p");
    let out = s.run(&p, &["settings"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!stderr(&out).contains("warning"), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("\"alignment.consecutive.params\": true"),
        "{}",
        stdout(&out)
    );
    let out = s.run(&p, &["settings", "--migrate"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stderr(&out), ".cfformat.json: nothing to migrate\n");
    assert_eq!(s.read("p/.cfformat.json"), json);
}

#[test]
fn settings_schema_describes_every_key() {
    let s = Scratch::new("schema");
    let out = s.run(
        &s.0,
        &["settings", "--schema", "nowhere", "--config", "nope.json"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // The bytes, not `stdout()`: its separator normalisation would break the
    // JSON's own escapes on Windows.
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    let properties = schema["properties"].as_object().unwrap();
    let out = s.run(&s.0, &["settings", "--defaults"]);
    let defaults: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&stdout(&out)).unwrap();
    let mut keys: Vec<&String> = properties.keys().collect();
    let mut default_keys: Vec<&String> = defaults.keys().collect();
    keys.sort();
    default_keys.sort();
    assert_eq!(keys, default_keys);
    assert_eq!(keys.len(), 47);
    for (key, p) in properties {
        assert!(
            p["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{key}"
        );
        assert_eq!(p["default"], defaults[key], "{key}");
    }
    assert_eq!(properties["indent_size"]["minimum"], 1);
    assert_eq!(properties["struct.separator"]["pattern"], "^ ?[:=] ?$");
    assert_eq!(
        properties["islands.js"]["enum"],
        serde_json::json!(["oxc", "off"])
    );
    assert_eq!(
        properties["strings.convert_nested_quotes"]["type"],
        "string"
    );
    assert_eq!(
        properties["strings.convert_nested_quotes"]["enum"],
        serde_json::json!(["always", "never", "fewer_escapes"])
    );
    assert!(!properties.contains_key("islands.timeout_ms"));

    // Every fixture's settings (written in the current keys, which the
    // golden test checks) use only schema keys, with values of the
    // schema's type. The library's fixtures, next door: the CLI package has
    // none.
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cfformat/tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path().join("settings.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let cases = match serde_json::from_str(&text).unwrap() {
            serde_json::Value::Array(cases) => cases,
            object => vec![object],
        };
        for case in cases {
            for (key, value) in case.as_object().unwrap() {
                let p = properties
                    .get(key)
                    .unwrap_or_else(|| panic!("{}: `{key}` is not in the schema", path.display()));
                let ok = match p["type"].as_str() {
                    Some("boolean") => value.is_boolean(),
                    Some("integer") => value
                        .as_u64()
                        .is_some_and(|n| n >= p["minimum"].as_u64().unwrap()),
                    Some("string") => value.as_str().is_some_and(|v| {
                        p["enum"].as_array().is_none_or(|e| e.contains(value))
                            && (key != "struct.separator"
                                || [":", "=", " :", ": ", " : ", " =", "= ", " = "].contains(&v))
                    }),
                    other => panic!("`{key}`: type {other:?}"),
                };
                assert!(
                    ok,
                    "{}: `{key}`: {value} does not fit the schema",
                    path.display()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "{checked}");
    eprintln!("schema: {checked} fixture settings checked");
}

/// A region the parse recovered in prints as written with a warning,
/// `file:line: not formatted: reason`; `--quiet` hides it, the summary
/// counts it, and the exit code is what it would be without it.
#[test]
fn a_recovery_warns_without_changing_the_exit_code() {
    let s = Scratch::new("recovered");
    let out = s.run_stdin(&s.0, &["-", "--script"], "a=1;\nb = @;\n");
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (
            Some(0),
            "a = 1;\nb = @;\n".into(),
            "<stdin>:2: not formatted: an unmatched run\n".into()
        )
    );
    let out = s.run_stdin(&s.0, &["-", "--script", "--quiet"], "b = @;\n");
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(0), "b = @;\n".into(), "".into())
    );
    // Nothing else to change: `check` passes. (The comment makes `.cfc`
    // script.)
    s.write("src/a.cfc", "// a\na = 1;\n}\nb = 2;\n");
    let out = s.run(&s.0, &["--check", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(0),
            "".into(),
            "src/a.cfc:3: not formatted: a stray closer\n\
             1 files, 0 would change, 0 failed, 1 not formatted, T s"
                .into()
        )
    );
    // Something else changes: `check` fails for that, `fmt -w` does not.
    s.write("src/a.cfc", "// a\na=1;\n}\nb = 2;\n");
    let out = s.run(&s.0, &["--check", "--quiet", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(1), "src/a.cfc\n".into(), "".into())
    );
    let out = s.run(&s.0, &["-w", "src"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(s.read("src/a.cfc"), "// a\na = 1;\n}\nb = 2;\n");
    assert!(
        stderr_untimed(&out).ends_with("1 files, 1 written, 0 failed, 1 not formatted, T s"),
        "{}",
        stderr(&out)
    );
}

/// Runs `cfformat` with stdout on `out` (a file or a pipe end) and `input`
/// on stdin: the exit code and stderr.
#[cfg(unix)]
fn run_into(
    s: &Scratch,
    args: &[&str],
    out: impl Into<Stdio>,
    input: &str,
) -> (Option<i32>, String) {
    let mut child = s
        .command(&s.0, args)
        .stdin(Stdio::piped())
        .stdout(out)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A run that does not read stdin may exit before the write.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    let o = child.wait_with_output().unwrap();
    (o.status.code(), stderr_untimed(&o))
}

#[test]
#[cfg(unix)]
fn a_closed_pipe_ends_the_text_quietly_and_anything_else_is_an_error() {
    // A pipe whose reader is gone: every write is `BrokenPipe`.
    let closed = || {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        writer
    };
    let s = Scratch::new("closed-pipe");
    s.write("src/a.cfc", UGLY);
    s.write("src/b.cfc", UGLY);
    // The formatted text to a reader that stopped (`| head`): exit 0,
    // nothing said, not even the summary.
    for args in [&["src/a.cfc"][..], &["-"], &["doc", "src/a.cfc"]] {
        assert_eq!(
            run_into(&s, args, closed(), UGLY),
            (Some(0), String::new()),
            "{args:?}"
        );
    }
    // Anything else a script reads is an error, printed once; nothing more
    // follows it, the summary included.
    let broken = "error: stdout: Broken pipe";
    for args in [
        &["--check", "src"][..],
        &["--diff", "src"],
        &["settings", "--defaults"],
        &["settings", "--schema"],
    ] {
        assert_eq!(
            run_into(&s, args, closed(), ""),
            (Some(2), broken.into()),
            "{args:?}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_full_stdout_is_an_error() {
    let full = || {
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap()
    };
    let s = Scratch::new("full");
    s.write("src/a.cfc", UGLY);
    s.write("src/b.cfc", UGLY);
    let error = "error: stdout: No space left on device";
    for args in [
        &["src/a.cfc"][..],
        &["-"],
        &["--check", "src"],
        &["doc", "src/a.cfc"],
        &["settings", "--defaults"],
    ] {
        assert_eq!(
            run_into(&s, args, full(), UGLY),
            (Some(2), error.into()),
            "{args:?}"
        );
    }
}

/// The names in `dir`, sorted.
#[cfg(unix)]
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
#[cfg(unix)]
fn write_replaces_a_file_whole_and_keeps_its_mode_and_links() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let s = Scratch::new("atomic");
    // The mode bits survive the rename, and no temp file is left.
    let a = s.write("src/a.cfc", UGLY);
    std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o640)).unwrap();
    let out = s.run(&s.0, &["-w", "src/a.cfc"]);
    assert_eq!((out.status.code(), stderr(&out)), (Some(0), "".into()));
    assert_eq!((s.read("src/a.cfc"), mode(&a)), (PRETTY.into(), 0o640));
    assert_eq!(listing(&s.0.join("src")), ["a.cfc"]);
    // A symlink is written through: still a link, its target formatted.
    s.write("real/b.cfc", UGLY);
    std::os::unix::fs::symlink("../real/b.cfc", s.0.join("src/link.cfc")).unwrap();
    let out = s.run(&s.0, &["-w", "src/link.cfc"]);
    assert_eq!((out.status.code(), stderr(&out)), (Some(0), "".into()));
    let link = std::fs::symlink_metadata(s.0.join("src/link.cfc")).unwrap();
    assert!(link.file_type().is_symlink());
    assert_eq!(s.read("real/b.cfc"), PRETTY);
    assert_eq!(listing(&s.0.join("real")), ["b.cfc"]);
    // A read-only file fails as it did before the rename: untouched.
    let c = s.write("src/c.cfc", UGLY);
    std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o444)).unwrap();
    // A writable file in a read-only directory: the temp file cannot be
    // made, so the write fails and the file is untouched.
    s.write("locked/d.cfc", UGLY);
    std::fs::set_permissions(s.0.join("locked"), std::fs::Permissions::from_mode(0o500)).unwrap();
    let out = s.run(&s.0, &["-w", "src/c.cfc", "locked/d.cfc"]);
    std::fs::set_permissions(s.0.join("locked"), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_untimed(&out),
        "src/c.cfc: Permission denied\n\
         locked/d.cfc: Permission denied\n\
         2 files, 0 written, 2 failed, T s"
    );
    assert_eq!((s.read("src/c.cfc"), mode(&c)), (UGLY.into(), 0o444));
    assert_eq!(s.read("locked/d.cfc"), UGLY);
    assert_eq!(listing(&s.0.join("locked")), ["d.cfc"]);
}

/// A component `arrange` changes, and its arranged text. Neither is
/// formatted: arranging never formats.
const UNARRANGED: &str = "component {\n    private function b() {}\n    function a() {}\n}\n";
const ARRANGED: &str = "component {\n    function a() {}\n    private function b() {}\n}\n";

#[test]
fn arrange_help_lists_its_flags() {
    let s = Scratch::new("arrange-help");
    let out = s.run(&s.0, &["--help"]);
    assert!(stdout(&out).contains("arrange"), "{}", stdout(&out));
    let out = s.run(&s.0, &["arrange", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = stdout(&out);
    for flag in [
        "Usage: cfformat arrange [OPTIONS] [PATHS]...",
        "-w, --write",
        "--check",
        "--diff",
        "--stdin ",
        "--stdin-filepath <PATH>",
        "--files-from <FILE|->",
        "--git <WHICH>",
        "-j, --jobs <N>",
        "--quiet",
        "--properties",
        "--script",
        "--tags",
        "Exit codes:",
    ] {
        assert!(help.contains(flag), "arrange --help lacks {flag}:\n{help}");
    }
    for absent in ["--config", "--cfm", "--no-islands", "--timing"] {
        assert!(!help.contains(absent), "arrange --help has {absent}");
    }
}

#[test]
fn arrange_check_write_and_exit_codes() {
    let s = Scratch::new("arrange");
    s.write("src/a.cfc", UNARRANGED);
    s.write("src/b.cfc", ARRANGED);
    s.write("src/sub/c.cfc", UNARRANGED);
    // A template is not walked; a broken settings file is not read.
    s.write("src/page.cfm", UNARRANGED);
    s.write("src/.cfformat.json", "{ not json");
    let mtime = |name: &str| {
        std::fs::metadata(s.0.join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    let before = mtime("src/b.cfc");

    let out = s.run(&s.0, &["arrange", "--check", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(1),
            "src/a.cfc\nsrc/sub/c.cfc\n".into(),
            "3 files, 2 would change, 0 failed, T s".into()
        )
    );
    let out = s.run(&s.0, &["arrange", "--diff", "src/a.cfc"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).starts_with("--- a/src/a.cfc\n+++ b/src/a.cfc\n"),
        "{}",
        stdout(&out)
    );
    // A missing file is exit 2, which wins over the 1 of a change.
    let out = s.run(&s.0, &["arrange", "--check", "src/a.cfc", "src/nope.cfc"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "src/a.cfc\n");

    let out = s.run(&s.0, &["arrange", "-w", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(0),
            "".into(),
            "3 files, 2 written, 0 failed, T s".into()
        )
    );
    assert_eq!(s.read("src/a.cfc"), ARRANGED);
    assert_eq!(s.read("src/sub/c.cfc"), ARRANGED);
    assert_eq!(s.read("src/page.cfm"), UNARRANGED);
    assert_eq!(mtime("src/b.cfc"), before);
    let out = s.run(&s.0, &["arrange", "--check", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(0),
            "".into(),
            "3 files, 0 would change, 0 failed, T s".into()
        )
    );
    // A template named explicitly is read, and holds no component.
    let template = "<cffunction name=\"b\"></cffunction>\n<cffunction name=\"a\"></cffunction>\n";
    s.write("src/view.cfm", template);
    let out = s.run(&s.0, &["arrange", "src/view.cfm"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(0), template.into(), "".into())
    );
    // One file prints; several need a mode.
    let out = s.run(&s.0, &["arrange", "src/a.cfc", "src/b.cfc"]);
    assert_eq!(
        (out.status.code(), stderr(&out)),
        (
            Some(2),
            "error: 2 files given; use -w, --check or --diff\n".into()
        )
    );
}

#[test]
fn arrange_reads_stdin_and_sorts_properties_on_request() {
    let s = Scratch::new("arrange-stdin");
    let out = s.run_stdin(&s.0, &["arrange", "-"], UNARRANGED);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (Some(0), ARRANGED.into(), "".into())
    );
    let out = s.run_stdin(&s.0, &["arrange", "--stdin"], UNARRANGED);
    assert_eq!(
        (out.status.code(), stdout(&out)),
        (Some(0), ARRANGED.into())
    );
    let properties = "component {\n    property name=\"b\";\n    property name=\"a\";\n}\n";
    let out = s.run_stdin(&s.0, &["arrange", "-"], properties);
    assert_eq!(stdout(&out), properties);
    let out = s.run_stdin(&s.0, &["arrange", "--properties", "-"], properties);
    assert_eq!(
        stdout(&out),
        "component {\n    property name=\"a\";\n    property name=\"b\";\n}\n"
    );
    // A body the parse recovered in: left as written, a warning naming the
    // file stdin stands for, exit 0.
    let recovered = "component {\n    function b() {}\n    function a() { x = @; }\n}\n";
    let out = s.run_stdin(
        &s.0,
        &["arrange", "--stdin-filepath", "src/x.cfc"],
        recovered,
    );
    assert_eq!(
        (out.status.code(), stdout(&out), stderr(&out)),
        (
            Some(0),
            recovered.into(),
            "src/x.cfc:1: not arranged: an unmatched run\n".into()
        )
    );
    s.write("src/x.cfc", recovered);
    s.write("src/y.cfc", UNARRANGED);
    let out = s.run(&s.0, &["arrange", "--check", "src"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(1),
            "src/y.cfc\n".into(),
            "src/x.cfc:1: not arranged: an unmatched run\n\
             2 files, 1 would change, 0 failed, 1 not arranged, T s"
                .into()
        )
    );
    let out = s.run_stdin(&s.0, &["arrange", "-w", "-"], UNARRANGED);
    assert_eq!(
        (out.status.code(), stderr(&out)),
        (Some(2), "error: -w cannot write to stdin\n".into())
    );
}

#[test]
fn arrange_usage_errors() {
    let s = Scratch::new("arrange-usage");
    s.write("a.cfc", UNARRANGED);
    let out = s.run(&s.0, &["arrange"]);
    assert_eq!(
        (out.status.code(), stderr(&out)),
        (
            Some(2),
            "error: no input given; name files, directories or globs \
             (e.g. `cfformat arrange .`), or use --stdin / --files-from / --git\n"
                .into()
        )
    );
    for flag in [
        &["--config", "x.json"][..],
        &["--cfm"],
        &["--no-islands"],
        &["--timing"],
    ] {
        let mut args = vec!["arrange"];
        args.extend_from_slice(flag);
        args.push("a.cfc");
        let out = s.run(&s.0, &args);
        assert_eq!(out.status.code(), Some(2), "{flag:?}");
        assert!(
            stderr(&out).starts_with(&format!("error: unexpected argument '{}'", flag[0])),
            "{}",
            stderr(&out)
        );
    }
    // A file called `arrange` is formatted when written as a path.
    s.write("arrange", UGLY);
    let out = s.run(&s.0, &["./arrange"]);
    assert_eq!((out.status.code(), stdout(&out)), (Some(0), PRETTY.into()));
}

/// Whether `git` runs; a test that needs a repository skips without it (CI
/// has it on every platform).
fn git_available() -> bool {
    let ok = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("--git tests skipped: `git` is not on PATH");
    }
    ok
}

impl Scratch {
    /// The environment every git run of a `--git` test gets, the tests' own
    /// and cfformat's: an empty global configuration, no system one, and
    /// the discovery stopped at the scratch directory, so the developer's
    /// settings and any repository around the temp directory stay out.
    fn git_env(&self, cmd: &mut Command) {
        let global = self.0.join("gitconfig");
        if !global.exists() {
            std::fs::write(&global, "").unwrap();
        }
        cmd.env("GIT_CONFIG_GLOBAL", global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.0);
    }

    /// Runs `git args` in `cwd`, which must succeed.
    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "user.name=cfformat",
            "-c",
            "user.email=cfformat@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .current_dir(cwd);
        self.git_env(&mut cmd);
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        stdout(&out)
    }

    /// Runs `cfformat` in `cwd` under [`Scratch::git_env`].
    fn run_git(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = self.command(cwd, args);
        self.git_env(&mut cmd);
        cmd.output().unwrap()
    }

    /// A repository at `repo/` whose first commit holds `src/mod.cfc` and
    /// `gone.cfc`, both formatted.
    fn repo(&self) -> PathBuf {
        let repo = self.0.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        self.git(&repo, &["init", "-q", "."]);
        self.write("repo/src/mod.cfc", PRETTY);
        self.write("repo/gone.cfc", PRETTY);
        self.git(&repo, &["add", "."]);
        self.git(&repo, &["commit", "-q", "-m", "one"]);
        repo
    }
}

#[test]
fn git_selects_the_staged_the_unstaged_or_all() {
    if !git_available() {
        return;
    }
    let s = Scratch::new("git");
    let repo = s.repo();
    // Nothing changed: no file, as a walk that finds none; not an error.
    let out = s.run_git(&repo, &["--git", "all", "--check"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(0),
            "".into(),
            "0 files, 0 would change, 0 failed, T s".into()
        )
    );
    // Committed then modified, a staged new file, an untracked one, an
    // untracked template, a committed file deleted, and a staged file
    // deleted from the working tree.
    s.write("repo/src/mod.cfc", UGLY);
    s.write("repo/src/new.cfc", UGLY);
    s.write("repo/src/ghost.cfc", UGLY);
    s.git(&repo, &["add", "src/new.cfc", "src/ghost.cfc"]);
    std::fs::remove_file(repo.join("src/ghost.cfc")).unwrap();
    std::fs::remove_file(repo.join("gone.cfc")).unwrap();
    s.write("repo/untracked.cfc", UGLY);
    s.write("repo/views/page.cfm", "<cfset x=1>\n");
    let check = |cwd: &Path, args: &[&str]| {
        let mut all = vec!["--check"];
        all.extend_from_slice(args);
        let out = s.run_git(cwd, &all);
        (out.status.code(), stdout(&out))
    };
    assert_eq!(
        check(&repo, &["--git", "staged"]),
        (Some(1), "src/new.cfc\n".into())
    );
    assert_eq!(
        check(&repo, &["--git", "unstaged"]),
        (Some(1), "src/mod.cfc\nuntracked.cfc\n".into())
    );
    assert_eq!(
        check(&repo, &["--git", "unstaged", "--cfm"]),
        (
            Some(1),
            "src/mod.cfc\nuntracked.cfc\nviews/page.cfm\n".into()
        )
    );
    let out = s.run_git(&repo, &["--git", "all", "--check"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(1),
            "src/mod.cfc\nsrc/new.cfc\nuntracked.cfc\n".into(),
            "3 files, 3 would change, 0 failed, T s".into()
        )
    );
    // Alone, a selection reports like a directory.
    let out = s.run_git(&repo, &["--git", "staged"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(0),
            "".into(),
            "1 files, 3 lines, 1 would change, 0 failed, T s".into()
        )
    );

    // From a subdirectory: the whole repository, a file under the current
    // directory named relative to it, any other by its full path.
    let src = repo.join("src");
    let (code, listed) = check(&src, &["--git", "all", "--cfm"]);
    assert_eq!(code, Some(1));
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 4, "{listed}");
    assert_eq!(lines[..2], ["mod.cfc", "new.cfc"]);
    for (line, name) in lines[2..]
        .iter()
        .zip(["repo/untracked.cfc", "repo/views/page.cfm"])
    {
        assert!(
            Path::new(line).is_absolute() && line.ends_with(name),
            "{line}"
        );
    }

    // `-w` writes the working tree and adds nothing to the index.
    let out = s.run_git(&src, &["--git", "staged", "-w"]);
    assert_eq!(
        (out.status.code(), stderr_untimed(&out)),
        (Some(0), "1 files, 1 written, 0 failed, T s".into())
    );
    assert_eq!(s.read("repo/src/new.cfc"), PRETTY);
    assert_eq!(
        s.git(&repo, &["diff", "--name-only", "--", "src/new.cfc"]),
        "src/new.cfc\n"
    );
    let out = s.run_git(&src, &["--git", "all", "-w", "--cfm"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(s.read("repo/untracked.cfc"), PRETTY);
    assert_eq!(s.read("repo/views/page.cfm"), "<cfset x = 1>\n");
    assert_eq!(
        check(&src, &["--git", "all", "--cfm"]),
        (Some(0), "".into())
    );
}

#[test]
fn git_errors_and_conflicts() {
    if !git_available() {
        return;
    }
    let s = Scratch::new("git-errors");
    s.write("a.cfc", UGLY);
    // The scratch directory's `.git` is empty, not a repository.
    for args in [&["--git", "all"][..], &["arrange", "--git", "staged"]] {
        let out = s.run_git(&s.0, args);
        assert_eq!((out.status.code(), stdout(&out)), (Some(2), "".into()));
        assert!(
            stderr(&out).starts_with("error: --git: not a git repository"),
            "{}",
            stderr(&out)
        );
    }
    // `git` not on PATH.
    let mut cmd = s.command(&s.0, &["--git", "all"]);
    cmd.env("PATH", "");
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).starts_with("error: --git: git: "),
        "{}",
        stderr(&out)
    );
    // Another input source is a usage error.
    for args in [
        &["--git", "all", "a.cfc"][..],
        &["--git", "all", "--files-from", "list.txt"],
        &["--git", "all", "--stdin"],
        &["--git", "all", "--stdin-filepath", "a.cfc"],
        &["--git", "some", "--check"],
        &["arrange", "--git", "all", "a.cfc"],
    ] {
        let out = s.run_git(&s.0, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(stderr(&out).starts_with("error: "), "{}", stderr(&out));
    }
    let out = s.run_git(&s.0, &["--git", "all", "a.cfc"]);
    assert!(
        stderr(&out).starts_with("error: the argument '--git <WHICH>' cannot be used with"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn arrange_takes_the_git_selection() {
    if !git_available() {
        return;
    }
    let s = Scratch::new("git-arrange");
    let repo = s.repo();
    s.write("repo/src/b.cfc", UNARRANGED);
    s.write("repo/src/page.cfm", UNARRANGED);
    s.git(&repo, &["add", "."]);
    s.write("repo/src/c.cfc", UNARRANGED);
    let out = s.run_git(&repo, &["arrange", "--git", "staged", "--check"]);
    assert_eq!(
        (out.status.code(), stdout(&out), stderr_untimed(&out)),
        (
            Some(1),
            "src/b.cfc\n".into(),
            "1 files, 1 would change, 0 failed, T s".into()
        )
    );
    let out = s.run_git(&repo, &["arrange", "--git", "all", "-w"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(s.read("repo/src/b.cfc"), ARRANGED);
    assert_eq!(s.read("repo/src/c.cfc"), ARRANGED);
    assert_eq!(s.read("repo/src/page.cfm"), UNARRANGED);
}
