//! The invariants of `cfformat::arrange`, shared by `tests/arrange.rs` and
//! the corpus soak (`tests/arrange_corpus.rs`).

use std::cmp::Ordering;

use cfformat::arrange::{self, Access, ArrangeOptions, Arranged, MemberKind, Run, Unit};
use cfparse::{Mode, TokenKind, Tree};

/// Both option sets every invariant runs with.
pub const BOTH: [ArrangeOptions; 2] = [
    ArrangeOptions { properties: false },
    ArrangeOptions { properties: true },
];

/// `+properties` or nothing, for messages and expected-failure keys.
pub fn suffix(opts: &ArrangeOptions) -> &'static str {
    if opts.properties {
        "+properties"
    } else {
        ""
    }
}

/// Arranges `src` and checks invariants 1, 2 and 4 on the result:
/// idempotence, permutation (the units of each run are the input's as a
/// multiset, the text outside them is identical, the output reparses with
/// the same recoveries and invalid tokens) and sortedness (every run of the
/// output is in order, by a comparator written apart from the engine's).
/// Returns the arranged source and the problems found.
pub fn check(src: &str, mode: Mode, opts: &ArrangeOptions) -> (Arranged, Vec<String>) {
    let mut problems = Vec::new();
    let out = arrange::arrange(src, mode, opts);
    if out.changed != (out.text != src) {
        problems.push(format!(
            "changed is {} but the texts say otherwise",
            out.changed
        ));
    }
    let again = arrange::arrange(&out.text, mode, opts);
    if again.changed || again.text != out.text {
        problems.push("not idempotent: a second arrange changes the output".into());
    }
    let before = cfparse::parse_source(src, mode);
    let after = cfparse::parse_source(&out.text, mode);
    let runs_before = arrange::runs(&before, opts);
    let runs_after = arrange::runs(&after, opts);
    if runs_before.len() != runs_after.len() {
        problems.push(format!(
            "{} runs before, {} after",
            runs_before.len(),
            runs_after.len()
        ));
    } else {
        for (i, (b, a)) in runs_before.iter().zip(&runs_after).enumerate() {
            let texts = |tree: &Tree, run: &Run| {
                let mut t: Vec<String> = run
                    .units
                    .iter()
                    .map(|u| tree.slice(u.span.clone()).to_owned())
                    .collect();
                t.sort();
                t
            };
            if b.kind != a.kind || texts(&before, b) != texts(&after, a) {
                problems.push(format!("run {i}: the units differ as a multiset"));
            }
        }
    }
    if masked(&before, &runs_before) != masked(&after, &runs_after) {
        problems.push("the text outside the units differs".into());
    }
    if before.recoveries != after.recoveries {
        problems.push(format!(
            "recoveries differ: {:?} before, {:?} after",
            before.recoveries, after.recoveries
        ));
    }
    if invalid(&before) != invalid(&after) {
        problems.push("invalid tokens differ".into());
    }
    for (i, run) in runs_after.iter().enumerate() {
        if let Some(w) = run
            .units
            .windows(2)
            .find(|w| expected_order(&w[0], &w[1]) == Ordering::Greater)
        {
            problems.push(format!(
                "run {i} of the output is out of order: {} before {}",
                w[0].name, w[1].name
            ));
        }
    }
    (out, problems)
}

/// The normalised source with every unit replaced by a marker.
fn masked(tree: &Tree, runs: &[Run]) -> String {
    let mut spans: Vec<_> = runs
        .iter()
        .flat_map(|r| r.units.iter().map(|u| u.span.clone()))
        .collect();
    spans.sort_by_key(|s| s.start);
    let mut out = String::new();
    let mut at = 0;
    for s in spans {
        out.push_str(tree.slice(at..s.start));
        out.push_str("\u{0}unit\u{0}");
        at = s.end;
    }
    out.push_str(tree.slice(at..tree.source.len() as u32));
    out
}

/// The texts of the tree's invalid tokens, sorted.
fn invalid(tree: &Tree) -> Vec<String> {
    let mut out: Vec<String> = tree
        .root
        .tokens()
        .iter()
        .filter(|t| t.kind == TokenKind::Invalid)
        .map(|t| tree.text(t).to_owned())
        .collect();
    out.sort();
    out
}

/// The documented order, written apart from [`Unit::order`]: `init`
/// first, then remote, public, package, private, then the name, ASCII
/// case-insensitively; properties by name alone.
fn expected_order(a: &Unit, b: &Unit) -> Ordering {
    let lower = |u: &Unit| {
        u.name
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .collect::<Vec<u8>>()
    };
    match a.kind {
        MemberKind::Property => lower(a).cmp(&lower(b)),
        MemberKind::Function => {
            let rank = |u: &Unit| {
                if lower(u) == b"init" {
                    return 0;
                }
                match u.access {
                    Access::Remote => 1,
                    Access::Public => 2,
                    Access::Package => 3,
                    Access::Private => 4,
                }
            };
            rank(a).cmp(&rank(b)).then_with(|| lower(a).cmp(&lower(b)))
        }
    }
}
