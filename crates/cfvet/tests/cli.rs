//! The binary: exit codes, stdin, `--quiet`, the directory walk.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// A scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cfvet-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_stdin(args, "")
    }

    fn run_with_stdin(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cfvet"))
            .args(args)
            .current_dir(&self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // A run that does not read stdin may exit before the write.
        match child.stdin.take().unwrap().write_all(input.as_bytes()) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => panic!("{e}"),
            _ => {}
        }
        child.wait_with_output().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const CLEAN: &str = "component {\n    function f() {\n        var x = 1;\n    }\n}\n";
const LEAKS: &str = "component {\n    function f() {\n        x = 1;\n    }\n}\n";
const LEAKS_CFM: &str = "<cffunction name=\"g\">\n    <cfset y = 1>\n</cffunction>\n";
const LEAKS_CFS: &str = "function h() {\n    z = 1;\n}\n";

#[test]
fn exit_codes() {
    let s = Scratch::new("exit");
    s.write("clean.cfc", CLEAN);
    s.write("leaks.cfc", LEAKS);
    let o = s.run(&["clean.cfc"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    assert_eq!(stderr(&o), "0 reports in 1 file (1 function checked)\n");

    let o = s.run(&["leaks.cfc"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        "leaks.cfc:3:9: missing-var: `x` is written without `var` or a scope in function `f`; \
         it lands in the variables scope\n"
    );

    // A missing path is an error, and 2 wins over 1; the rest is linted.
    let o = s.run(&["nope.cfc", "leaks.cfc"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).starts_with("error: nope.cfc: "),
        "{}",
        stderr(&o)
    );
    assert!(stdout(&o).starts_with("leaks.cfc:3:9: "));

    // Not UTF-8.
    std::fs::write(s.0.join("latin1.cfc"), b"x = \"\xe9\";\n").unwrap();
    let o = s.run(&["latin1.cfc"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("latin1.cfc: not valid UTF-8"));

    // No arguments: the help, exit 2; an unknown flag, exit 2.
    assert_eq!(s.run(&[]).status.code(), Some(2));
    assert_eq!(s.run(&["--fix"]).status.code(), Some(2));
    assert_eq!(
        s.run(&["--script", "--tags", "clean.cfc"]).status.code(),
        Some(2)
    );
}

#[test]
fn stdin_and_modes() {
    let s = Scratch::new("stdin");
    let o = s.run_with_stdin(&["-"], LEAKS);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).starts_with("<stdin>:3:9: missing-var: `x`"),
        "{}",
        stdout(&o)
    );

    // `--tags` reads script as template text: no function, nothing to report.
    let o = s.run_with_stdin(&["--tags", "-"], LEAKS);
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
    let o = s.run_with_stdin(&["--script", "-"], "function f() { z = 1; }\n");
    assert_eq!(o.status.code(), Some(1));
    assert!(stdout(&o).starts_with("<stdin>:1:16: missing-var: `z`"));
}

#[test]
fn quiet_prints_the_reports_only() {
    let s = Scratch::new("quiet");
    s.write("leaks.cfc", LEAKS);
    s.write(
        "broken.cfc",
        "component {\n    function f() {\n        a = @;\n    }\n}\n",
    );
    let o = s.run(&["leaks.cfc", "broken.cfc"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stderr(&o),
        "broken.cfc:3: note: parse recovery (unmatched); the region is not checked\n\
         1 report in 2 files (2 functions checked)\n"
    );
    let o = s.run(&["-q", "leaks.cfc", "broken.cfc"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o), "");
    assert_eq!(stdout(&o).lines().count(), 1);
    assert_eq!(s.run(&["--quiet", "leaks.cfc"]).stderr, b"");
}

#[test]
fn a_directory_walk_finds_every_extension() {
    let s = Scratch::new("walk");
    s.write("src/a.cfc", LEAKS);
    s.write("src/views/b.cfm", LEAKS_CFM);
    // Script by its name: the source alone would read it as a template.
    s.write("src/c.cfs", LEAKS_CFS);
    s.write("src/notes.txt", LEAKS);
    s.write("src/.hidden/h.cfc", LEAKS);
    s.write("src/gen/x.cfc", LEAKS);
    std::fs::create_dir_all(s.0.join(".git")).unwrap();
    s.write(".gitignore", "gen/\n");
    let o = s.run(&["src"]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let files: Vec<String> = stdout(&o)
        .lines()
        .map(|l| l.split(':').next().unwrap().replace('\\', "/"))
        .collect();
    assert_eq!(files, ["src/a.cfc", "src/c.cfs", "src/views/b.cfm"]);
    assert!(stderr(&o).ends_with("3 reports in 3 files (3 functions checked)\n"));
}

#[test]
#[cfg(unix)]
fn a_failed_stdout_write_is_an_error() {
    // A pipe whose reader is gone: the reports cannot be written, which is
    // said once and exits 2, with no summary after it.
    let s = Scratch::new("stdout");
    s.write("leaks.cfc", LEAKS);
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let o = Command::new(env!("CARGO_BIN_EXE_cfvet"))
        .arg("leaks.cfc")
        .current_dir(&s.0)
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stderr(&o), "error: stdout: Broken pipe\n");
}

/// Runs `git args` (or `cfvet`, with `cfvet` as the program) in `cwd` with
/// an empty global configuration, no system one, and the repository
/// discovery stopped at the scratch directory.
fn git_env(s: &Scratch, program: &str, cwd: &std::path::Path, args: &[&str]) -> Output {
    let global = s.0.join("gitconfig");
    if !global.exists() {
        std::fs::write(&global, "").unwrap();
    }
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", global)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CEILING_DIRECTORIES", &s.0)
        .output()
        .unwrap()
}

#[test]
fn git_selects_the_changed_files() {
    if !Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("--git tests skipped: `git` is not on PATH");
        return;
    }
    let s = Scratch::new("git");
    let cfvet = env!("CARGO_BIN_EXE_cfvet");
    // Not a repository: exit 2, git's message.
    let o = git_env(&s, cfvet, &s.0, &["--git", "all"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).starts_with("error: --git: not a git repository"),
        "{}",
        stderr(&o)
    );
    let repo = s.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let mut all = vec![
            "-c",
            "user.name=cfvet",
            "-c",
            "user.email=cfvet@example.com",
            "-c",
            "commit.gpgsign=false",
        ];
        all.extend_from_slice(args);
        let o = git_env(&s, "git", &repo, &all);
        assert!(o.status.success(), "git {args:?}: {}", stderr(&o));
    };
    git(&["init", "-q", "."]);
    s.write("repo/clean.cfc", CLEAN);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "one"]);
    s.write("repo/src/leaks.cfc", LEAKS);
    git(&["add", "src/leaks.cfc"]);
    s.write("repo/views/page.cfm", LEAKS_CFM);
    let src = repo.join("src");
    let o = git_env(&s, cfvet, &src, &["--git", "staged"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).starts_with("leaks.cfc:3:9: missing-var: "),
        "{}",
        stdout(&o)
    );
    assert_eq!(stderr(&o), "1 report in 1 file (1 function checked)\n");
    // `all`: the untracked template too, named from the root.
    let o = git_env(&s, cfvet, &repo, &["--git", "all", "-q"]);
    assert_eq!(o.status.code(), Some(1));
    let reported: Vec<String> = stdout(&o)
        .lines()
        .map(|l| l.split(':').next().unwrap().replace('\\', "/"))
        .collect();
    assert_eq!(reported, ["src/leaks.cfc", "views/page.cfm"]);
    // Paths and --git conflict.
    let o = git_env(&s, cfvet, &repo, &["--git", "all", "clean.cfc"]);
    assert_eq!(o.status.code(), Some(2));
}
