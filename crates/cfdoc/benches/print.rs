//! `print_doc` on a synthetic ~5k-line document: statements assigning nested
//! struct/array literals, printed at width 120.
//!
//! ```text
//! cargo bench -p cfdoc --bench print
//! ```

use cfdoc::builders::*;
use cfdoc::{print_doc, Doc, IndentStyle, PrintOptions};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

/// `open`, indented items joined by `, line`, optional trailing comma,
/// `close`, as one group.
fn delimited(open: &'static str, items: Vec<Doc>, close: &'static str) -> Doc {
    group(vec![
        open.into(),
        indent(vec![softline(), join(vec![",".into(), line()], items)]),
        if_break(",", ""),
        softline(),
        close.into(),
    ])
}

/// A small deterministic generator so the doc shape is fixed.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn value(rng: &mut Lcg, depth: u32) -> Doc {
    match (depth, rng.below(6)) {
        (0, _) | (_, 0..=2) => match rng.below(3) {
            0 => format!("\"value_{}\"", rng.below(10_000)).into(),
            1 => rng.below(1_000_000).to_string().into(),
            _ => "true".into(),
        },
        (_, 3 | 4) => {
            let n = 1 + rng.below(6);
            let items = (0..n)
                .map(|i| vec![format!("key{i}").into(), ": ".into(), value(rng, depth - 1)].into())
                .collect();
            delimited("{", items, "}")
        }
        _ => {
            let n = 1 + rng.below(8);
            let items = (0..n).map(|_| value(rng, depth - 1)).collect();
            delimited("[", items, "]")
        }
    }
}

fn document() -> Doc {
    let mut rng = Lcg(42);
    let statements = (0..350).map(|i| {
        group(vec![
            format!("local.variable{i} = ").into(),
            value(&mut rng, 4),
            ";".into(),
        ])
    });
    vec![join(hardline(), statements), hardline()].into()
}

fn bench(c: &mut Criterion) {
    let doc = document();
    let opts = PrintOptions {
        width: 120,
        indent: IndentStyle::Spaces(4),
        newline: "\n",
    };
    let out = print_doc(&mut doc.clone(), &opts);
    eprintln!(
        "synthetic doc: {} output lines, {} bytes",
        out.lines().count(),
        out.len()
    );

    c.bench_function("print_doc width 120", |b| {
        b.iter_batched(
            || doc.clone(),
            |mut doc| print_doc(&mut doc, &opts),
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
