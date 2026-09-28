//! Byte parity with Prettier's `printDocToString` on random docs.
//!
//! Dev-only and ignored: needs `node` and a one-time
//! `npm install` in `crates/cfdoc/scripts/doc-parity`. Run with
//!
//! ```text
//! cargo test -p cfdoc --test parity -- --ignored --nocapture
//! ```
//!
//! `CFDOC_PARITY_CASES` (default 50) and `CFDOC_PARITY_SEED` override the
//! number of docs and the generator seed.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use cfdoc::builders::*;
use cfdoc::debug::{format_doc, to_prettier_json};
use cfdoc::{print_doc, Align, Doc, GroupIdGen, IndentStyle, PrintOptions};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

const WORDS: &[&str] = &[
    "a",
    "foo",
    "barbaz",
    "quux",
    "lorem ipsum",
    "x",
    "(",
    ")",
    "[",
    "]",
    ",",
    ";",
    "=>",
    "longer_identifier_name",
    "trailing  ",
    " ",
    "\t",
    "tab\tin",
    "two\nlines",
    "",
];

struct Gen {
    rng: Rng,
    ids: GroupIdGen,
    allocated: u32,
    budget: usize,
}

impl Gen {
    fn id_ref(&mut self) -> u32 {
        // Mostly groups already allocated (possibly printed), sometimes one
        // that does not exist yet.
        self.rng.below(self.allocated as usize + 2) as u32
    }

    fn maybe_id(&mut self) -> Option<u32> {
        self.rng.chance(35).then(|| {
            self.allocated += 1;
            self.ids.next_id()
        })
    }

    fn leaf(&mut self) -> Doc {
        match self.rng.below(20) {
            0..=7 => Doc::from(self.rng.pick(WORDS)),
            8..=10 => line(),
            11..=12 => softline(),
            13 => hardline(),
            14 => literalline(),
            15 => hardline_without_break_parent(),
            16 => break_parent(),
            17 => line_suffix_boundary(),
            _ => Doc::from(self.rng.pick(WORDS)),
        }
    }

    fn doc(&mut self, depth: usize) -> Doc {
        if depth == 0 || self.budget == 0 || self.rng.chance(30) {
            return self.leaf();
        }
        self.budget -= 1;
        let d = depth - 1;
        match self.rng.below(15) {
            0..=2 => {
                let n = 1 + self.rng.below(5);
                Doc::Concat((0..n).map(|_| self.doc(d)).collect())
            }
            3..=5 => {
                let id = self.maybe_id();
                let should_break = self.rng.chance(10);
                group_opts(self.doc(d), GroupOpts { id, should_break })
            }
            6 => {
                let id = self.maybe_id();
                let should_break = self.rng.chance(10);
                let n = 1 + self.rng.below(3);
                let states = (0..n).map(|_| self.doc(d)).collect();
                conditional_group_opts(states, GroupOpts { id, should_break })
            }
            7 => {
                let n = 1 + self.rng.below(8);
                let parts = (0..n)
                    .map(|i| {
                        if i % 2 == 0 {
                            self.doc(d)
                        } else {
                            match self.rng.below(6) {
                                0..=2 => line(),
                                3 => softline(),
                                4 => hardline(),
                                _ => self.doc(d),
                            }
                        }
                    })
                    .collect();
                fill(parts)
            }
            8 => {
                let (b, f) = (self.doc(d), self.doc(d));
                if self.rng.chance(50) {
                    let id = self.id_ref();
                    if_break_group(b, f, id)
                } else {
                    if_break(b, f)
                }
            }
            9 => {
                let id = self.id_ref();
                let negate = self.rng.chance(50);
                indent_if_break(self.doc(d), id, negate)
            }
            10 | 11 => indent(self.doc(d)),
            12 => {
                let n = match self.rng.below(7) {
                    0 => Align::Width(0),
                    1 | 2 => Align::Width(1 + self.rng.below(4)),
                    3 => Align::from(self.rng.pick(&["> ", "\t", "--"])),
                    4 => Align::Dedent,
                    5 => Align::DedentToRoot,
                    _ => Align::MarkRoot,
                };
                align(n, self.doc(d))
            }
            13 => line_suffix(self.doc(d.min(2))),
            _ => {
                let n = 2 + self.rng.below(4);
                join(
                    vec![",".into(), line()],
                    (0..n).map(|_| self.doc(d)).collect::<Vec<_>>(),
                )
            }
        }
    }
}

struct Case {
    doc: Doc,
    width: usize,
    tab_width: usize,
    use_tabs: bool,
    crlf: bool,
}

fn cases(count: usize, seed: u64) -> Vec<Case> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|_| {
            let mut gen = Gen {
                rng: Rng(rng.next() | 1),
                ids: GroupIdGen::new(),
                allocated: 0,
                budget: 120,
            };
            let doc = Doc::Concat((0..3).map(|_| gen.doc(6)).collect());
            Case {
                doc,
                width: rng.pick(&[20, 40, 80]),
                tab_width: rng.pick(&[2, 4]),
                use_tabs: rng.chance(40),
                crlf: rng.chance(20),
            }
        })
        .collect()
}

#[test]
#[ignore = "needs node and `npm install` in crates/cfdoc/scripts/doc-parity"]
fn parity() {
    let script_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/doc-parity");
    if !script_dir.join("node_modules/prettier").is_dir() {
        eprintln!(
            "parity skipped: run `npm install` in {} first",
            script_dir.display()
        );
        return;
    }
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("parity skipped: `node` is not on PATH");
        return;
    }

    let count = std::env::var("CFDOC_PARITY_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let seed = std::env::var("CFDOC_PARITY_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed_cfd0c);

    let cases = cases(count, seed);
    let mut input = String::from("[");
    let mut expected_rust = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        if i > 0 {
            input.push(',');
        }
        input.push_str(&format!(
            r#"{{"doc":{},"printWidth":{},"tabWidth":{},"useTabs":{},"endOfLine":"{}"}}"#,
            to_prettier_json(&case.doc),
            case.width,
            case.tab_width,
            case.use_tabs,
            if case.crlf { "crlf" } else { "lf" },
        ));
        let opts = PrintOptions {
            width: case.width,
            indent: if case.use_tabs {
                IndentStyle::Tabs(case.tab_width)
            } else {
                IndentStyle::Spaces(case.tab_width)
            },
            newline: if case.crlf { "\r\n" } else { "\n" },
        };
        expected_rust.push(print_doc(&mut case.doc.clone(), &opts));
    }
    input.push(']');

    let mut child = Command::new("node")
        .arg(script_dir.join("print.mjs"))
        .current_dir(&script_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn node");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("node output");
    assert!(output.status.success(), "print.mjs failed");
    let prettier: Vec<String> = serde_json::from_slice(&output.stdout).expect("print.mjs JSON");
    assert_eq!(prettier.len(), cases.len());

    let mut failures = 0;
    for (i, (ours, theirs)) in expected_rust.iter().zip(&prettier).enumerate() {
        if ours != theirs {
            failures += 1;
            if failures <= 3 {
                let case = &cases[i];
                eprintln!(
                    "case {i} differs (width {}, tab {}, tabs {}, crlf {})\ndoc: {}\ncfdoc:    {:?}\nprettier: {:?}\n",
                    case.width,
                    case.tab_width,
                    case.use_tabs,
                    case.crlf,
                    format_doc(&case.doc),
                    ours,
                    theirs
                );
            }
        }
    }
    let lines: usize = prettier.iter().map(|s| s.lines().count()).sum();
    println!(
        "parity: {}/{} byte-identical ({} output lines, seed {seed:#x})",
        cases.len() - failures,
        cases.len(),
        lines
    );
    assert_eq!(failures, 0);
}
