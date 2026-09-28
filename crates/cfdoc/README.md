# cfdoc

A Rust port of Prettier's document IR and printer
(`prettier/src/document`, 3.10.0-dev; the printer is identical in the
published 3.9.6), plus two additions (**Added** below). It knows nothing about
the language being formatted. The port keeps Prettier's MIT licence:
its notice (Copyright © James Long and contributors) is
[`LICENSE-PRETTIER`](LICENSE-PRETTIER), and ships with every release.

```rust
use cfdoc::builders::{group, indent, join, line, softline};
use cfdoc::{print_doc, Doc, PrintOptions};

let items = ["alpha", "beta", "gamma"].map(Doc::from);
let mut doc = group(vec![
    "[".into(),
    indent(vec![softline(), join(vec![",".into(), line()], items)]),
    softline(),
    "]".into(),
]);
let out = print_doc(&mut doc, &PrintOptions { width: 16, ..Default::default() });
assert_eq!(out, "[\n  alpha,\n  beta,\n  gamma\n]");
```

| Module | Prettier |
|---|---|
| `Doc`, `Group`, `Align`, `LineKind` | `builders/*.js` doc objects (owned tree, no shared subtrees) |
| `builders` | `group`, `conditionalGroup`, `indent`, `align`, `dedent`, `dedentToRoot`, `markAsRoot`, `addAlignmentToDoc`, `line`, `softline`, `hardline`, `literalline`, `*WithoutBreakParent`, `ifBreak`, `indentIfBreak`, `fill`, `lineSuffix`, `lineSuffixBoundary`, `breakParent`, `join` (snake_case) |
| `print_doc`, `PrintOptions` | `printDocToString` (`printer/printer.js`) |
| `indent` | `printer/indent.js` |
| `utils` | `propagateBreaks`, `willBreak`, `canBreak`, `findInDoc`, `cleanDoc`, `stripTrailingHardline`, `replaceEndOfLine`, `removeLines` |
| `Doc::walk`, `Doc::walk_mut`, `Doc::map`, `Doc::is_empty` | `traverseDoc`, `mapDoc`, `isEmptyDoc` |
| `width::str_width` | `utilities/get-string-width.js` |
| `debug::format_doc` | `debug.js` `printDocToDebug` |

**Added**: `conditional_group_contents(doc)` is `conditionalGroup([doc])`
stored once (a group with empty `expanded_states`, read as `[contents]`):
flat when it fits, else broken, never broken by its children. Prettier shares
the state object; an owned tree would copy it at every nesting level.
`utils::flat_width(doc)` is the width the doc takes printed on one line, a
`FlatWidth`: `Finite(columns)`, or `Infinite` when it holds a hard or literal
line (or a string with a newline). A group counts as its contents whether or
not it is broken, an `if_break` as its flat side, a line suffix and a break
parent as 0; a caller that needs a broken group to count as unmeasurable
asks `utils::will_break` as well.

**Not ported**: the `cursor`, `trim` and `label` docs and cursor-position
tracking. `str_width` uses `unicode-width`, which differs from Prettier's
emoji handling at the margins.

## Tests

```text
cargo test -p cfdoc                                              # unit tests + doctest
cargo bench -p cfdoc --bench print                               # print_doc, synthetic 5k-line doc
```

Parity with Prettier (dev-only; needs Node). Once:

```text
cd crates/cfdoc/scripts/doc-parity && npm install
```

Then:

```text
cargo test -p cfdoc --test parity -- --ignored --nocapture       # 50 random docs, byte-identical
CFDOC_PARITY_CASES=3000 CFDOC_PARITY_SEED=7 cargo test -p cfdoc --test parity -- --ignored --nocapture
CFDOC_PRETTIER_CHECK=1 cargo test -p cfdoc --test printer        # unit-test expectations vs Prettier
```

The parity test builds seeded random ASCII docs (every node kind, nesting
depth ≤ 6, widths 20/40/80, tabs and spaces, `\n` and `\r\n`), serialises
them with `debug::to_prettier_json` and compares with
`scripts/doc-parity/print.mjs`, which runs the published `prettier/doc`
`printDocToString`. It skips with a message when `node` or `node_modules` is
missing. Run it before committing changes to `printer.rs`, `indent.rs` or
`utils.rs`.
