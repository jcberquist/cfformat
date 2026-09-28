# Working in this repository

A CFML formatter (`cfformat`) and a two-check companion (`cfvet`) in Rust. The user-facing
documentation is the root `README.md`; the developer documentation is each
crate's README, which is the contract for that crate:

- `crates/cfparse/README.md` — the parse tree, both front ends, recovered
  regions, the fuzz passes.
- `crates/cfdoc/README.md` — the port of Prettier's document printer and its
  parity test.
- `crates/cfformat/README.md` — the library API, what each printer module
  prints, every test and how to update its expectations, the full command-line
  contract.
- `crates/cfformat-cli` — the `cfformat` binary over the library (no README of
  its own; the crate README's CLI section is its contract).
- `crates/cfvet/README.md` — `cfvet`'s two checks and its exit codes.
- `crates/cfcli/README.md` — the file plumbing both binaries share: the
  directory walk, the dedupe, reading a source as UTF-8, IO messages.

Read the README of the crate you are changing before changing it, and keep it
true: a change in behaviour is a change in the README and, for a user-visible
one, in `CHANGELOG.md` too.

## The gate

Every change passes all of this before it is committed; CI
(`.github/workflows/ci.yml`) runs the same on Linux, macOS and Windows:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --release -p cfparse --locked --test fuzz -- --ignored
cargo test --release -p cfformat --locked --test islands_deep -- every_chain_at_the_size_limit_is_refused
```

`Cargo.lock` is committed and must not change under `--locked`. The oxc
formatter crates are a git dependency pinned to one revision, written in
several places in the root `Cargo.toml` and in the two READMEs that quote it;
bump them together.

## Tests and their expectations

- The golden fixtures (`crates/cfformat/tests/fixtures/`) are compared byte
  for byte. Never edit an expectation by hand: run `UPDATE_GOLDENS=1 cargo
  test -p cfformat --test goldens`, then review the diff of every regenerated
  file as you would review code. The same for `UPDATE_ARRANGE=1` (the arrange
  fixtures), `UPDATE_REFERENCE=1` (`SETTINGS.md`, generated
  from the option definitions), `UPDATE_SNAPSHOTS=1` (`cfparse`) and
  `UPDATE_CASES=1` (`cfvet`).
- `crates/cfformat/tests/invariants.rs` checks every golden case for
  idempotence, token preservation and literal preservation; a formatter change
  that breaks one of these is wrong, not the invariant.
- The corpus, soak and parity tests are `#[ignore]` and read a sibling
  checkout next to this repository (`../commandbox-cfformat`, or the directory
  a `*_CORPUS` variable names) and, for the parity tests, `prettier` on
  `PATH`. A test that runs by default never needs them; an ignored one either
  skips with a message or fails at once naming what it needs. See the crate
  READMEs for how to run them.
- Test data keeps its line endings: the Windows CI job sets
  `core.autocrlf false`, and a fixture with CRLF is meant to have it.

## Rules

- Both parsers are total and the formatter never fails a file: what it cannot
  read prints as written, with a warning. Keep it that way; a new construct is
  parsed or recovered, never an error.
- Formatting never changes the text of a string, and never drops or adds a
  token. `cfformat arrange` never formats.
- Every dependency is pure Rust: the release builds musl and macOS targets
  with no C toolchain. Adding a dependency with a C build is a design change.
- Comments explain what the code does and why, for a reader of this
  repository alone. They do not cite documents outside it. The exception is
  `crates/cfdoc`, a port of Prettier's document printer pinned to a named
  version with its licence shipped: its comments name the Prettier file and
  line each piece follows, which is the port's contract.
- Commit messages describe the change and its reason in their own words. A
  commit written by an AI agent ends with a `Co-Authored-By:` trailer naming
  the agent.
