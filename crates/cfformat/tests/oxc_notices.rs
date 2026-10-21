//! `LICENSE-OXC-THIRD-PARTY` at the repository root is a copy of the oxc
//! repository's `THIRD-PARTY-LICENSE` at the revision the root `Cargo.toml`
//! pins: the notices of the code oxc itself ports (Biome's and Prettier's
//! formatters among them), which the generated `THIRD-PARTY-LICENSES.md`
//! does not carry. The copy must move with the pin, so this test compares
//! it with the checkout cargo built the workspace from, found through
//! `cargo metadata`: no network, the checkout is there or nothing compiled.

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn the_oxc_notices_are_the_pinned_checkouts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ours = root.join("LICENSE-OXC-THIRD-PARTY");
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // The checkout root is two levels above `crates/oxc_formatter/Cargo.toml`.
    let theirs = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "oxc_formatter")
        .map(|p| PathBuf::from(p["manifest_path"].as_str().unwrap()))
        .expect("oxc_formatter in the graph")
        .parent()
        .unwrap()
        .join("../../THIRD-PARTY-LICENSE");
    let read = |path: &Path| {
        // Byte for byte but for `\r`: a Windows checkout with autocrlf may
        // carry either file with CRLF.
        let mut bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        bytes.retain(|&b| b != b'\r');
        bytes
    };
    assert!(
        read(&ours) == read(&theirs),
        "{} differs from {}: the pin moved, copy the file over again",
        ours.display(),
        theirs.display()
    );
}
