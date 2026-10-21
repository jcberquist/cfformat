//! `LICENSE-OXC-THIRD-PARTY` at the repository root is a copy of the oxc
//! repository's `THIRD-PARTY-LICENSE` at the revision the root `Cargo.toml`
//! pins: the notices of the code oxc itself ports (Biome's and Prettier's
//! formatters among them), which the generated `THIRD-PARTY-LICENSES.md`
//! does not carry. The copy must move with the pin, so this test compares
//! it with the checkout cargo built the workspace from: no network, the
//! checkout is there or nothing compiled.
//!
//! The checkout is found by hand, not through `cargo metadata`: that
//! resolves the whole graph for every platform and fetches each package's
//! manifest, so on a cargo home holding only what the host build needed (a
//! CI runner, any fresh machine) it fails offline before comparing
//! anything. Instead the revision is read from `Cargo.lock` (every oxc
//! entry's source is `git+…/oxc?rev=<rev>#<rev>`) and checked against the
//! `rev = "…"` of every oxc line in the root `Cargo.toml`; the cargo home
//! is `$CARGO_HOME`, else `~/.cargo` (`HOME`, or `USERPROFILE` on Windows);
//! the checkout is `<cargo home>/git/checkouts/oxc-<hash>/<rev[..7]>/`,
//! where `<hash>` is cargo's own, so every `oxc-*` directory holding that
//! revision is compared, and at least one must.

use std::path::{Path, PathBuf};

const OXC_GIT: &str = "https://github.com/oxc-project/oxc";

#[test]
fn the_oxc_notices_are_the_pinned_checkouts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ours = root.join("LICENSE-OXC-THIRD-PARTY");
    let rev = pinned_rev(&root);
    let cargo_home = cargo_home();
    let checkouts = cargo_home.join("git").join("checkouts");
    let mut theirs: Vec<PathBuf> = std::fs::read_dir(&checkouts)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("oxc-"))
                .map(|entry| entry.path().join(&rev[..7]).join("THIRD-PARTY-LICENSE"))
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default();
    theirs.sort();
    assert!(
        !theirs.is_empty(),
        "no checkout of oxc at {rev} under {}: expected {}/oxc-*/{}/THIRD-PARTY-LICENSE \
         (cargo clones the pinned oxc there when it builds the workspace; this \
         repository does not vendor its dependencies)",
        cargo_home.display(),
        checkouts.display(),
        &rev[..7]
    );
    for path in &theirs {
        assert!(
            read(&ours) == read(path),
            "{} differs from {}: the pin moved, copy the file over again",
            ours.display(),
            path.display()
        );
    }
}

/// The one revision every oxc entry in `Cargo.lock` is pinned at, which the
/// root `Cargo.toml` names on every oxc line.
fn pinned_rev(root: &Path) -> String {
    let lock = root.join("Cargo.lock");
    let lock = std::fs::read_to_string(&lock).unwrap_or_else(|e| panic!("{}: {e}", lock.display()));
    let prefix = format!("source = \"git+{OXC_GIT}?rev=");
    let mut revs: Vec<&str> = lock
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(|rest| rest.split('#').next().unwrap())
        .collect();
    revs.sort_unstable();
    revs.dedup();
    assert!(
        revs.len() == 1,
        "Cargo.lock pins oxc at {} revisions, expected one: {revs:?}",
        revs.len()
    );
    let rev = revs[0].to_string();
    assert!(
        rev.len() >= 7 && rev.bytes().all(|b| b.is_ascii_hexdigit()),
        "Cargo.lock pins oxc at an odd revision: {rev:?}"
    );

    let manifest = root.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    let mut lines = 0;
    for line in manifest.lines().filter(|line| line.contains(OXC_GIT)) {
        lines += 1;
        let theirs = line
            .split_once("rev = \"")
            .and_then(|(_, rest)| rest.split('"').next())
            .unwrap_or_else(|| panic!("no rev on the Cargo.toml oxc line: {line}"));
        assert!(
            theirs == rev,
            "Cargo.toml pins oxc at {theirs} where Cargo.lock has {rev}: {line}"
        );
    }
    assert!(lines > 0, "no oxc entry in the root Cargo.toml");
    rev
}

/// `$CARGO_HOME`, else `~/.cargo`: cargo's own default, from the platform's
/// home directory variable.
fn cargo_home() -> PathBuf {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return PathBuf::from(home);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .expect("CARGO_HOME, HOME or USERPROFILE set");
    PathBuf::from(home).join(".cargo")
}

/// Byte for byte but for `\r`: a Windows checkout with autocrlf may carry
/// either file with CRLF.
fn read(path: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes.retain(|&b| b != b'\r');
    bytes
}
