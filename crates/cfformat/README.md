# cfformat

The CFML formatter: parses with `cfparse`, builds a `cfdoc` document from the
tree and prints it — CFScript, tag-mode CFML and the `<script>` / `<style>`
islands inside it (formatted in process by oxc, `islands.*: "oxc"`, or
printed as written, `"off"`). This is the developer README: the library API,
what each printer module handles, the tests and the full CLI contract. The
[root README](../../README.md) is the user's: install, quick use and the
limitations.
The command line is its own package, `cfformat-cli` in
`crates/cfformat-cli` (binary `cfformat`), so the library's dependency graph
holds no clap, ignore, rayon or similar; its contract is the [CLI](#cli)
section below.

```rust
use cfformat::{format_source, Mode, Options};

let (opts, warnings) = Options::from_json(r#"{"max_columns": 80, "newline": "\n"}"#).unwrap();
assert!(warnings.is_empty());
let out = format_source("if(a==b){x=1;}", Mode::Script, &opts);
assert_eq!(out, "if (a == b) {\n    x = 1;\n}\n");
```

## Install

See the [root README](../../README.md#install): release binaries, or
`cargo install --locked --path crates/cfformat-cli` from a checkout (Rust 1.96 or
newer and the platform's usual linker and build tools; every dependency is
pure Rust, so no other native library is needed; the oxc crates are a git
dependency, so `cfformat` is not on crates.io). Nothing needs to be on
`PATH` at run time.

## API

The crate's API is the formatter's. The `cfparse` and `cfdoc` types its
items take or return are re-exported at the root, so the entry points below
need no other dependency: `Mode`, `Tree`, `Node` / `Element` / `Token` (the
printer's input), `Island` / `Lang` (the island functions),
`RecoveryReason` (`WarningKind::Recovered`), `Newline`
(`Options::newline_str`), `Doc` and `GroupIdGen` (the printer's output and
its group ids), `IndentStyle` (`Options::indent_style`, an island
request's indentation). The crates themselves are not re-exported: a caller that
builds a tree for `tree_to_doc` (`cfparse::parse_source`) or runs the
document printer's passes itself (`cfdoc::print_doc`) depends on `cfparse`
or `cfdoc` directly, by the same path or git source.

- `format_source(src, mode, &Options) -> String` — parse, build the doc,
  print at `max_columns`, re-emit the BOM. `Mode::Auto`
  resolves script or tag mode from the source, and both print; a caller
  with a file name resolves it first (`Mode::for_path`: `.cfs` script,
  `.cfm` tags, else `Auto`), as the CLI does.
- `format_with(src, mode, &Options, &FormatCtx { path, islands }) ->
  Formatted { text, warnings, timings, islands }` — the real
  entry: `path` is the file (an island is formatted as `<file>.js` next to
  it, under the configuration found from its directory; `None` is stdin),
  `islands` the run's `islands::Islands` (cache and counters, `Sync`, one per
  process) or `None` to print every island verbatim. `Formatted.warnings`
  lists, in source order, what printed as written instead of formatted —
  `Warning { path, line, kind, message }`, `WarningKind::Island { key }` for
  an island oxc refuses (a parse error: `file:line: islands.js: message`)
  and `WarningKind::Recovered(reason)` for a region the parse did not
  understand (`cfparse::Tree::recoveries`: `file:line: not formatted: an
  unmatched run` / `a stray closer` / `an unclosed block` / `nesting past
  the limit`). Neither is an error, and nothing else is: both front ends
  are total, so formatting never fails and neither function returns a
  `Result`.
  `format_source` is `format_with` with no path and a fresh `Islands`; it
  drops the warnings.
  `Formatted.islands` is this file's `IslandStats` (formatter runs, cache
  hits, refusals, time in the island formatter), right whatever thread formatted it;
  `Islands::stats()` is the run's total. `Islands` also caches the
  project configuration `"oxc"` reads (`islands.config`: each directory's
  `.oxfmtrc` / `.prettierrc` looked up once per run, from the file's
  directory, or the current directory's for `path: None`);
  `Islands::config_warnings()` lists `(file, message)` for every
  configuration file that could not be read (each file once). The library
  never reads or writes the source file, prints, or starts a thread pool
  (that is the binary's job, `crates/cfformat-cli`); `islands.config: "auto"` (the
  default) reads `.oxfmtrc` / `.prettierrc` files from the formatted file's
  directory upward — set `IslandConfigMode::Off` (`"off"`) for a run that
  touches nothing but the source.
- `arrange(src, mode, &ArrangeOptions { properties, first }) -> Arranged {
  text, changed, skipped }` — `cfformat arrange`: the functions of each
  top-level component and interface body (script or tags) in order (the
  functions `first` names, in its order, whatever their access — `["init"]`
  by default, `[]` for none; then `remote`, `public`, `package`, `private`,
  then the name, ASCII case-insensitively, `first` matched the same way),
  and with `properties` the properties by name within
  each group (a blank line between two properties ends a run; functions
  cross blank lines). The
  output is the input's bytes with the members permuted (line endings and
  the BOM as they were); it never formats and reads no `Options`.
  `skipped` lists each body left as written because the parse recovered in
  it: `Skipped { line, reason }`. The module `arrange` has the pieces the
  engine decides with, for tests and tools: `bodies(&Tree, &ArrangeOptions)
  -> Vec<Body { span, skipped, runs, alone, fixed }>` and `runs(&Tree,
  &ArrangeOptions) -> Vec<Run { kind, units, separators }>`, where a `Unit
  { span, kind, name, access }` is a member with its attached comments and
  `Unit::order(&Unit, &First)` the comparator (`First::new(&opts.first)`
  lowercases the list once); `fixed` gives each element that does not
  move and why (`Fixed`, `Ineligible`). The rules are the root README's
  "Arranging members".
- `tree_to_doc(&Tree, &Options) -> Doc` — the document, before the
  printer's break propagation; `tree_to_doc_with` takes a `FormatCtx` and
  returns the `PrintReport { warnings, stats }` too: the island formatter's
  warnings and numbers, and a warning per recovered region.
- `debug_doc(src, mode, &Options, &FormatCtx) -> Formatted` — what
  `cfformat doc` prints: the document `format_with` would print, breaks
  propagated, as `cfdoc`'s debug text in `text`, with `format_with`'s
  warnings, timings and island numbers.
- `islands` — `dispatch(&Island) -> Option<Target>` (`None` for a body of
  no language, `Lang::Unknown`), `hand_off_text` (the
  island as written, blank edge lines dropped), `literal_lines` (the lines
  of the formatter's output inside a template literal, a continued string
  or a raw block comment, spliced as written; a text it cannot parse is a
  `Refused`, not an empty list), `nesting_depth` / `tree_depth` (the
  island limits' two counts), `keeps_literal_text` (whether
  a verbatim island keeps its lines as written, `Verbatim::Raw`), `sql_literal_spans_lines` (the
  SQL rule of it: a string- and comment-aware scan under each dialect
  reading), `literal_texts` and `sql_literal_texts` (the JavaScript quasis
  and block comments, and the SQL literals, the invariants compare),
  `trait IslandFormatter` (`format`, the text, and `format_island`, the
  text with its `literal_lines`: by default `format` and then the walk, for
  `Oxc` both on one island thread), `Oxc` (the island formatter, `"oxc"`;
  `src/islands/oxc.rs` is the only file naming an oxc type), `Islands`
  (`format` / `format_with` return a `FormattedIsland { text,
  literal_lines }` — the walk run on every formatter's output, in the same
  cached run — or the refusal, `Refused(message)`, and a `Hit { cached,
  time }`;
  `Islands::with_formatter` sends every request to another
  `IslandFormatter`, the seam `tests/islands_parity.rs` runs the prettier
  CLI through), `IslandStats`.
- `Options` — the 47 keys ([`SETTINGS.md`](../../SETTINGS.md), at the
  repository root) as a flat serde struct; the keys come from CommandBox
  cfformat, the defaults are the ones `SETTINGS.md` lists;
  `Options::from_json` / `from_map` migrate removed and renamed keys and
  the old boolean values of `strings.convert_nested_quotes`
  (`Options::migrate`, warnings; `Warning::chose_comma` marks the one
  whose comma value the migration chose), reject unknown ones and validate values
  (`struct.separator`, `indent_size`, `max_columns`); `Options::validate`
  is the second half alone, for an object already migrated.
- `options::Discovery` — settings discovery, first found wins:
  `Discovery::new(home, config)` (`Discovery::home_file()` is
  `~/.cfformat.json`, the fallback when no `.cfformat.json` is found),
  `discover(file)` / `discover_in(dir)` → `Resolved { options, sources,
  warnings }`. Settings files are read once per `Discovery`; see "CLI" for
  the rule.
- `options::reference()` — every option as `OptionInfo { key, kind, default,
  description, example }`, the source of `SETTINGS.md`; an
  `Example` carries its `ExampleMode` (script or tags).
- `print::Printer` — one walk over the tree: `Printer::new(&tree,
  &opts).document()` (islands verbatim) or `Printer::with_ctx`, then
  `finish()` for the `PrintReport`. The per-node printers inside it
  (`Printer::node` dispatches on the element kind) are the crate's own.

**Depending on the library.** `cfformat` is not on crates.io; depend on it by
path or git. The oxc formatter crates are a git dependency, and the published
`oxc-css-parser` that `oxc_formatter_css` parses with depends on the
*published* `oxc_allocator`; this workspace redirects it to the same git rev
in `[patch.crates-io]`. Cargo reads that table from the root manifest of the
build only, so a project that depends on `cfformat` must copy the line into
its own root `Cargo.toml`:

```toml
[patch.crates-io]
oxc_allocator = { git = "https://github.com/oxc-project/oxc", rev = "288d8cc77984b0a3851c58c423ffe9e6edc79f2e" }
```

Without it the two allocators are two crates and the build fails in
`oxc_formatter_css` with `error[E0308]: mismatched types … expected
oxc_css_parser::Allocator, found oxc_allocator::Allocator`. The rev is the
one in this workspace's `Cargo.toml`; keep the two in step.

## What prints

| Module | Constructs |
|---|---|
| `print/statements.rs` | script root, blocks, statement lists (a run of blank lines kept as one, semicolons preserved), every statement kind, `if` / `else` / `for` / `while` / `do` / `switch` / `case` / `try` / `catch` / `finally`, `Printer::sequence` for runs the tree leaves unstructured |
| `print/expressions.rs` | groups (a ternary, struct, array or destructuring assignment alone inside hugs the parentheses: `({` ⏎ … ⏎ `} = x)`), assignments (Prettier's `printAssignment` layouts: break after the operator, never, or fluid, and `break-lhs` for a struct pattern of more than two items with a rename or a default, which breaks inside the pattern before the value; shared with struct members; with one line comment before the value, on one line with the comment at its end while the value fits there, else the comment on its own line after the operator and the value under it, as always when the value holds a forced break), binary chains, ternaries, unary operators, `new` |
| `print/delimited.rs` | structs, arrays, parameters and `cfhttp(…)` attributes: one threshold per list (`*.multiline.element_count` / `min_item_length`, padding, empty padding, `multiline.comma`: trailing, dangling or leading commas, applied per kind of list by `CommaStyle::literal` / `list`), item comments, `KeyValue` separators, index brackets (`brackets.padding`); destructuring patterns with the settings of the literal they resemble (`struct.*` for `{…}`, `array.*` for `[…]`, `multiline.comma` as for literals), every item kept (a skipped element prints its comma alone: `[a, , c]`, broken on a line of its own), no dangling comma after a rest item, the rename always `key: target` (never `struct.separator`, the key never quoted), a default `target = value` on one line; a struct pattern that renames to a nested pattern breaks, except as a parameter or a default's target (Prettier's `ObjectPattern`) |
| `print/accessors.rs` | member chains as Prettier's `printMemberChain`: groups (`.a.b()` runs), the merge of a short or factory-like head, one line while it fits with the last call's arguments free to break, else a group per line; `method_call.chain.multiline` above 0 forces the break; a chain with comments keeps the CommandBox line layout |
| `print/calls.rs` | call callees and arguments, with Prettier's argument hugging (last / first function, struct or array; a single string that spans lines) |
| `print/casing.rs` | `function_call.casing.builtin` (`data/functions.json`: the cfdocs spelling of every builtin `cfparse`' `data/support_functions.json` lists — `tests/data.rs` keeps the two sets equal) and `.userdefined` |
| `print/functions.rs` | function declarations, anonymous functions, arrows, metadata attributes; a sole pattern parameter (bare, or with a name, `{}` or `[]` as its default, no comment) hugs the parentheses and breaks inside itself (Prettier's `shouldHugTheOnlyFunctionParameter`), with no parameter threshold |
| `print/components.rs` | `component` / `interface` headers, `static { }`, `import` (a line comment after the path prints after the `;`, as after any statement: `import a.b; // c`), `property`, `param`, tags in script (both forms); Lucee's inline `new component attrs { }` prints through the same component rules |
| `print/attributes.rs` | the attribute group shared by metadata, `property`, `param`, script tags and tags (a tag holding only a comment after its name keeps it: `<p <!--- c --->>`; a CF tag or `#expr#` among an HTML tag's attributes stays glued to what precedes it — the tag's name, an attribute, a value — where the source has it so, since a space there shows on the page: `<td<cfif x> class="a"</cfif>>`, `class=a<cfif x>b</cfif>`) |
| `print/comments.rs` | line, block and doc comments (`comment.asterisks`); `alignment.doc_comments`: tags padded, `@return` and `@throws` blocks ordered. A comment after code on its line prints there: a block comment in place, a line comment as a line suffix that breaks the enclosing group where the layout has a line break right after it (`Printer::same_line_comment`: `a && // c` ⏎ `b`), and that breaks nothing where it has none — after a ternary's `?` or `:`, inside a unary, a member access, an attribute or an assignment that holds comments, after the last attribute of a `property`, `param` or script tag, before or after a function's parameters, after a statement's last part — so it ends the line the code ends on (`Printer::deferred_comment`: `a ? b : // c` ⏎ `d;` prints `a ? b : d; // c`), which is where the next run reads it |
| `print/alignment.rs` | `alignment.consecutive.assignments`: the run partition and padding used by statement lists (`x = …`, `var x = …`, `param x = …`), delimited lists (struct members, named arguments, parameter defaults, `cfhttp(…)` attributes; only when the list breaks) and attribute groups; `alignment.consecutive.properties` / `.params`: the attribute runs of a statement list (`property …;` and attribute-form `param …;` statements with the same attribute names, case-insensitive), every attribute but the last padded to the widest of its column, only when the statement prints on one line |
| `print/strings.rs` | quote style, the quote of a string holding one (Prettier's fewer escapes by default) and its re-escaping, `#expr#` |
| `print/tags.rs` | tag mode: the document root, tags and tag bodies (`tags.lowercase`; `tags.body.indent`, under `"cfml"` a paired CF tag body that starts with HTML at the tag's own indent; attributes through the shared group, never padded (`attributes.key_value.padding` is for script), a CF tag's quoted values in `strings.attributes.quote`, an HTML tag's as written, as are those in a CF tag body inside an HTML tag's attribute list (`<div <cfif x>id="y"</cfif>>`); `<cfset>` / `<cfreturn>` / `<cfif>` / `<cfelseif>` script with `>` on its own line when it breaks, unless it ends on a call's, struct's, array's or index's bracket, and after a `//` comment that ends the script's last line (`<cfset s = // c` ⏎ `{ a: 1 }>` prints `<cfset s = {a: 1} // c` ⏎ `>`: after `>` the comment would be text), `<cfelse>` / `<cfelseif>` on their own line at the tag's indent, the segments between them indented, block tags from `data/tags.json`; a body on its tag's line keeps whitespace at either end as one space when the page can show it — an HTML body whose tag is not a block tag (`<span>hello </span>world`), or a CF body, which the browser never sees (`<cfoutput>#a# </cfoutput>`) — and drops it at the ends of a block tag's body (`<div> a </div>` is `<div>a</div>`); whitespace here is ASCII, as in HTML: a no-break space is text and is never trimmed; a body that breaks has line breaks at its ends instead, except an inline HTML body, which breaks at an end only where the source had whitespace there (`<a href="x"><img></a>` stays glued); for whitespace, a tag whose surrounding and edge whitespace a browser never renders counts as a block tag too (`spacelessTags` in `data/tags.json`, this project's own list beside the vendored `blockTags`: `<html>`, `<head>`, `<title>`, the parts of a table, `<select>`, `<option>`, …), without the line break after it; a line break follows a block tag's body and a CF tag body, unless what follows the CF body is glued to it in the source (`</cfif>more`) and is not such a tag; a paired `<pre>` / `<textarea>` (any case) prints its body exactly as written between its two formatted tags, every byte but the line endings (`newline`), CF tags in it left as they are (still parsed: they run on the server) — one whose tags the tree left bare, because they cross a CF body, is not recognised and its text is laid out as any other; a tag the tree left bare — an HTML tag that crosses a CF body, a CF tag paired across an island, an unmatched HTML tag — prints inline, on its own line when it had one, at the indentation around it; a CF tag the file-wide CF walk cannot pair makes its region print as written, with a warning), `<!--- --->` comments, `<cfscript>` bodies and ```` ``` ```` code fences |
| `print/islands.rs` | islands: a pure `<script>` / `<style>` (JavaScript, a module, JSON — `application/json`, `application/ld+json`, `importmap`, `speculationrules` — or CSS) whose `islands.*` option is not `"off"` is handed to oxc as written and its output printed at the tag's indent, a line inside a template literal, a continued string or a raw block comment (`islands::literal_lines`) as written; every other island (`<cfquery>`, `<cfjava>`, impure or `"off"` islands, islands inside a code fence) is verbatim (`TagCtx::verbatim`), its lines keeping their indentation relative to each other and shifted as a whole so that none sits left of the tag (`Verbatim::Shift`) — or, when its text may hold a string spanning lines (`islands::keeps_literal_text`), every line as written, not shifted, trimmed or re-indented, but whitespace-only edge lines dropped (the last line's trailing whitespace goes with the `hardline` before the closing tag) and the closing tag on its own line (`Verbatim::Raw`); a `<script>` / `<style>` body of no language (`Lang::Unknown`: a type the formatter does not know, or one CFML supplies) is never handed off and prints every line as written, its whitespace-only edge lines included, and the closing tag right after the last one (`Verbatim::Exact`); in every mode a line ending is the output's (`newline`), not the source's; a tag inside an island (`<cfqueryparam>` in a SQL line) never breaks, whatever the width, unless it holds a forced break (a `//` comment, a function body), when it breaks as it does outside an island; a `<script>` / `<style>` whose body starts with CFML and holds no island prints as written |
| `print/mod.rs` | dispatch, ignore regions (`cfformat-ignore`, `@formatter:off`) |

There is no fallback printer any more: `Printer::element` has an arm for every
element kind, and a misparse (a string without both quotes) prints its source
text through `Printer::as_written`. A region the parse recovered in
(`ElementKind::Recovered`: a statement in script, a tag body in tag
mode) prints as written too — its first line where the printer puts it,
every other line as in the source (tabs and trailing whitespace included) —
and everything around it is formatted; `Printer::finish` adds its warning.
Nesting is
bounded like the parser's: past `print::MAX_DEPTH` (260, half the depth at
which a 2 MB thread overflowed) an element prints as written, so the CFML
side of `format_source` is safe on any default thread; the CLI's workers
have 8 MB. The oxc parsers and formatters bound nothing — they recurse
once per level with no counter — and neither do the walks over their trees
(`literal_lines`, `literal_texts`, the depth walk), so islands are bounded
around them, in `islands.rs` and `islands/oxc.rs`. `Oxc::format` refuses,
before anything else, a hand-off text over `SIZE_LIMIT` (256 KB) or nested
deeper than `NESTING_LIMIT` (500) by `nesting_depth`: a `Refused`
like a parse error, so the island prints as written with a warning
(`islands.js: nested 501 levels deep, over the limit of 500`, `islands.js:
300007 bytes, over the limit of 262144`). `nesting_depth` is lexical, per
language, and may only over-count: an opener counts wherever it is, a
closer only where the scan is sure it is code, so a closer inside a string
or a comment cannot cancel a real opener. CSS: strings (a line feed ends
one, where oxc's tokenizer ends a bad string), comments, escapes (a hex
escape takes the whitespace byte after it) and an unquoted `url(` body
(under a vendor prefix too, and `url-prefix(`, `domain(`); and its
brackets are matched by kind, as the parser matches them — a block or a
function's arguments run to their own closer, and a closer of another
kind inside them is a token (`foo(})` closes no rule). JavaScript, and
JSON, which oxc parses with its JavaScript parser: strings, `/* */` and
`//` comments, the script's HTML comments, template literals (code inside
`${…}`), and regular expressions, told from a division by the byte before
the `/`. That last is a heuristic, and its misreadings (`return /]/`,
`a++ / b`, JSX text) may count below the parser's nesting: for a script
the tree bound stands behind it, and JSON with a `/` outside its strings
and comments is refused by the JSON formatter after the parse. So the
count is the whole bound for CSS and for JSON, which nest by brackets
alone, and the parser's protection for JavaScript, whose nesting it cannot
all see: an unbraced `if` / loop / `do` body, a JSX child, a label, a
chain of operators, calls or members each close their bracket at once, or
have none. A script is then parsed with the formatter's own
`parse_for_format` (any diagnostic refuses, as `oxc_formatter::format`
does), its depth measured by `tree_depth` — a `Visit` counting every node
it enters, whatever its kind, so a parenthesis counts nothing and an
object literal two a level — and refused past the same limit with the same
message; only a tree within it reaches `format_program`. The formatter's
text then goes through `literal_lines` inside `Islands::format_with`
(`IslandFormatter::format_island`), in the same cached, timed run and for
every formatter: the result is a `FormattedIsland`, the text and its
literal lines together, and a text the walk cannot parse (or a thread that
cannot start) is a refusal, never a text with no literal lines. Within the
limits, the three calls — `Oxc::format` (with that walk of its output, on
the same thread, for `Islands`), `literal_lines`, `literal_texts` — each
run on a thread of their own (`ISLAND_THREAD`) with an `ISLAND_STACK`
(1 GB) stack,
whatever the caller's thread is: address space, not memory, touched only
as deep as the island goes. What is bounded by construction: the
formatter and both literal walks (they parse the formatter's output of a
tree at most 500 deep), and every stylesheet, and every JSON text the JSON
formatter takes, before its parser sees it. What is measured, under the
size limit: a script's parse, which runs before the tree can be counted,
and the depth walk after it (recursive, a frame per level, at most about a
level per byte: a frame smaller than the parser's that built the tree) —
and the parse of a JSON text holding a `/`. At the limit, on a Linux x86_64
development machine with these oxc crates, parse and walk together peak at about 110 MB in
release for the deepest (`x=` and `a=>` chains), 71 MB for brackets the
pre-scan could not see (a closer hidden in a string at every level, should
its reading of a script be fooled), about 60 MB or less for every other
shape tried, and a debug build's at most 134 MB (a `!` chain) — the
size-limit group of `tests/islands_deep.rs`, which runs by default,
prints each. Those numbers move with the oxc crates.

**Memory.** Nothing streams: a file's source, tree, document and output are
all held at once, so memory grows with the file. The release binary's peak
resident size is 14 MB for an empty file, 18 MB for a real 175 KB component
(about 25× the source above that base), and 1.0 GB for a synthetic 4.9 MB
run of statements, the densest in tokens (about 210× the source; a tree
node is 16 bytes, an element boxed behind it). No real file comes near
that. The island cache (`Islands`) lives for the run and is never emptied:
it holds one entry, text and result, per distinct island request, so it is
bounded by the distinct islands a run meets, not by the number of files.

## Tests

```text
cargo test -p cfformat                                   # options, goldens, invariants, printer cases, comment variations
GOLDENS=switch,keywords cargo test -p cfformat --test goldens
UPDATE_GOLDENS=1 cargo test -p cfformat --test goldens   # rewrite the expectations that differ
cargo test --release -p cfformat --test corpus -- --ignored --nocapture
CFFORMAT_CORPUS=~/src cargo test --release -p cfformat --test corpus -- --ignored --nocapture
CFFORMAT_WIDE=1 …                                          # also print every corpus line over max_columns
CFFORMAT_CORPUS_OPTIONS='{"alignment.consecutive.assignments": true}' …   # merge settings over the defaults
CFFORMAT_CORPUS_DISCOVER=1 …                               # each file's options from its .cfformat.json (home file off)
CFFORMAT_CORPUS_CFM=1 …                                    # walk .cfm files too (counted on their own line)
UPDATE_REFERENCE=1 cargo test -p cfformat --test reference # rewrite SETTINGS.md
CFFORMAT_CORPUS_REPORT=1 …                                 # the soak report: files/s, p50 / p95 / max, the ten slowest files
cargo test -p cfformat --test islands_real -- --ignored --nocapture   # oxc over the island fixtures and commandbox
cargo test --release -p cfformat --test islands_parity -- --ignored --nocapture   # oxc against the prettier CLI (the test spawns it; prettier on PATH), file by file (minutes over a directory of real projects)
cargo bench -p cfformat --bench format                    # parse / to_doc / print / format over the fixtures
cargo test -p cfformat-cli                               # the command line through the built binary (crates/cfformat-cli/tests/cli.rs)
cargo test -p cfformat --test arrange                    # arrange: the fixtures under tests/arrange and the invariants
UPDATE_ARRANGE=1 cargo test -p cfformat --test arrange   # rewrite their expected files
cargo test --release -p cfformat --test arrange_corpus -- --ignored --nocapture   # the arrange soak (CFFORMAT_CORPUS, CFFORMAT_CORPUS_OPTIONS, CFFORMAT_ARRANGE_LIST=1, CFFORMAT_ARRANGE_STRICT=1)
```

The corpus test prints every panic (`path: internal error: message`, a
failure); both front ends are total, so a file it can read parses. With
`CFFORMAT_CORPUS_REPORT=1` each mode's counts are followed by its throughput
(files per second of formatting time), the p50 / p95 / max per-file time
(parse + doc + print, the island formatter inside doc) and its ten slowest
files with a `parse / doc / print / islands` split.

Island formatting needs nothing installed, so every golden runs on every
platform (the Windows and macOS CI jobs included): the `island*` goldens have
an `"off"` case (the verbatim shift, or byte for byte) and a case with the
defaults (oxc's output, each reviewed against prettier 3.9.6);
`islandTemplateLiteral`, `islandLiteralNested` and `islandVerbatimLiterals`
pin literal preservation on both paths, `islandDynamicType` a `type` holding
CFML, `islandOpaqueBody` the bodies of no language (a newline around `type=`,
generated attributes, static unknown types) byte for byte,
`cfquerySqlLiterals` the SQL scan and `functionTypeJavaTextBlock` a Java
text block with an escaped `"""`. The goldens and the invariants
format each case as its fixture's `source.cfc` (`common::format_case`), and
every case that does not name `islands.config` runs with it `"off"`, so no
`.prettierrc` above the checkout can move an expectation (`tests/printer.rs`
and `tests/reference.rs` format with it `"off"` too). Two fixtures carry a
configuration file and an `"auto"` and an `"off"` case: `islandConfig` (a
`.prettierrc` with a trailing comma, a comment, `printWidth` / `tabWidth` /
`useTabs` that do not apply and `overrides`; its `"auto"` case is what the
prettier CLI prints in that directory) and `islandConfigOxfmt` (an
`.oxfmtrc.jsonc` beside a `.prettierrc`: the oxfmt file wins).
`tests/islands_real.rs` runs oxc over real files: every case of the island
fixtures with its own settings, then every island fixture source and
`../commandbox-cfformat` file holding a `<script>` / `<style>` with the
defaults, checking idempotence and the invariants. `tests/islands_parity.rs`
formats every file of the island fixtures, `../commandbox-cfformat`,
`CFFORMAT_CORPUS` (a path list of real projects; none when unset) holding a
`<script>` / `<style>` twice, as cfformat does
and with every island handed to the prettier CLI instead (the test spawns
it, `prettier --stdin-filepath <path> --use-tabs … --tab-width …
--print-width …` in the file's directory, through `Islands::with_formatter`;
it needs `prettier` on `PATH` and is skipped otherwise), compares the
outputs byte for byte and fails on a fixture or commandbox difference (`islandConfigOxfmt` is left out: prettier
does not read `.oxfmtrc`); `CFFORMAT_PARITY_CLASSIFY=1` sorts the other
differences into those a part of the project's prettier configuration that
oxc does not read explains (plugins, ignore files, a YAML file) and the rest.

`tests/comment_variation.rs` formats a fixed list of small script and
tag-mode snippets (statements, expressions, calls, structs and arrays,
closures, components with properties and functions, imports, tags with
attributes and bodies) with a comment inserted after each significant
token in turn — `/* c */` and `// c` with its newline, and in tag mode
`<!--- c --->` too — about 1,900 variants in a fraction of a second: each
must format, its output must parse with no recovered region and keep the
number of comments, and a second run must reproduce the output. A
variant where the inserted text is no comment, or that parses with a
recovered region, is skipped. It runs at the default settings: at a
narrow width a line comment printed at the end of its line can land
after a bracket that breaks (`foo( // c`), and the next run moves it onto
a line of its own.

`tests/invariants.rs` checks every passing golden case, and `tests/corpus.rs`
every corpus file, for: idempotence; token preservation (the
normalised significant-token streams of input and output are equal:
`common::token_stream`; host text is compared with each whitespace run as
one space, its leading and trailing space dropped only at the edges of the
document, a code fence, an island, a CF tag body and a block tag's body,
next to a block tag and next to a `<cfelse>` / `<cfelseif>` — a block tag
here is one of `data/tags.json`'s `blockTags` or `spacelessTags` (whose
edge whitespace a browser does not render either: `<tr>`, `<head>`, …) —
and a `<pre>` /
`<textarea>` body exactly; in an HTML tag's attribute list, whether a CF
tag or `#expr#` is glued to what precedes it; separator commas are kept, a trailing one and
the comma after an empty item dropped; comments are compared on their own,
in order, each line of their text trimmed, a doc comment's text not at all
under `alignment.doc_comments`, which rewrites it; the test
`the_token_stream_tells_significant_changes_apart` pins what it catches);
reparse (no new `invalid` / `other` token, the same
number of top-level statements, the same number of comments); literal
preservation (every JavaScript island handed off keeps its template-literal
quasis and its block comments, a comment's lines compared without their
leading whitespace: `common::check_literals`, also run by
`tests/islands_real.rs`); exact preservation (an island of no language
keeps its text byte for byte, a `<cfquery>` the multiset of its SQL
literals under each dialect reading, a Java block or `<cfjava>` the
multiset of its Java literals: `common::check_preserved`, counted in both
summary lines); no trailing whitespace outside an island whose text
may hold a string spanning lines and outside a `<pre>` / `<textarea>` body
(`common::literal_island_lines`; the exempted lines are counted). A golden line over `max_columns` fails unless it is on the
`WIDE` allow-list in `tests/invariants.rs`. `tests/printer.rs` holds the cases
the goldens do not pin (chain merge boundaries, index / safe / static
segments, chains inside calls and keyword groups, chain comments, source line
breaks that must not matter, alignment runs and doc comments, and the tag-mode
layouts: block vs inline bodies, blank-line capping, `<cfelse>`, attribute
breaking, `tags.lowercase`, `tags.body.indent`, tag comments, islands, `<cfscript>` and code
fences) and one case per soak regression. `tests/reference.rs` renders the root `SETTINGS.md` from
`options::reference()`, formatting every example with the formatter itself,
and fails when the checked-in file differs.

The golden fixtures in `tests/fixtures/<name>/` started as CommandBox
cfformat's `tests/data`, minus the dropped cases and with the edited cases
listed in `tests/fixtures/DROPPED.md`, plus fixtures of this project's own
(among them `tagPreformatted`, `<pre>` / `<textarea>` bodies as written,
`tagWhitespaceInline`, the whitespace a page shows around inline, CF and
spaceless tag bodies, and the `destructuring` family: `destructuring`,
`destructuringPadding`, `destructuringCommas` and `destructuringWide`, every
pattern form Adobe ColdFusion 2025 runs, each golden source runnable as is). `formatted.txt` holds one expectation per
settings case, separated by lines holding only `~`. Every `settings.json`
case is written in the current keys: the fixture loader fails on a case the
migration would change (`Options::migrate`), so no golden depends on it.

Every settings case of every fixture must match its expectation; a mismatch
fails the golden test with a unified diff. When a change is meant to alter
output, regenerate with `UPDATE_GOLDENS=1` and review each rewritten
`formatted.txt` as a diff.

## CLI

The `cfformat-cli` package, `crates/cfformat-cli`: `src/main.rs` and
`src/cli/` (arguments, inputs, output, the run and the settings schema) over
the library's public API, with the directory walk, the dedupe and the
UTF-8 read from `crates/cfcli`, which `cfvet` shares; `tests/cli.rs` drives
the built binary.

```text
cfformat [PATHS...] [-w|--write] [--check] [--diff] [--stdin]
         [--stdin-filepath PATH] [--files-from FILE|-]
         [--git staged|unstaged|all] [-j N] [--quiet]
         [--cfm] [--config FILE] [--timing] [--no-islands] [--script|--tags]
cfformat doc <FILE|-> [--config FILE] [--no-islands] [--timing] [--script|--tags]
cfformat settings [PATH|-] [--config FILE] [--defaults] [--schema]
cfformat settings --migrate [FILE|-]
cfformat arrange [PATHS...] [-w|--write] [--check] [--diff] [--stdin]
         [--stdin-filepath PATH] [--files-from FILE|-]
         [--git staged|unstaged|all] [-j N] [--quiet]
         [--script|--tags] [--properties] [--first NAMES]
cfformat --version                   # cfformat 0.2.0
```

**Inputs.** A PATH is a file (taken as given, any extension, ignored or
not), a directory or a glob. A directory is walked for `.cfc` and `.cfs`
files — and `.cfm` with `--cfm` — honouring `.gitignore` (inside a git
repository), the global gitignore, `.git/info/exclude` and `.ignore`,
skipping hidden entries and not following symlinks, in path order. A PATH holding `*`, `?`, `[` or
`{` that does not exist is a glob (`**` crosses directories, `*` does not):
its literal prefix directory is walked the same way and every file it
matches is kept, whatever its extension (`cfformat --check 'src/**/*.cfm'`).
`--files-from FILE|-` adds the list's entries after the PATHS: NUL-separated
when the list holds a NUL (`git … -z`; nothing is stripped, so a `\r` is part
of a name), else one per line (a trailing `\r` dropped, an empty line
skipped). An entry keeps its bytes (on Unix any bytes are a name; elsewhere
an entry that is not UTF-8 is an error), and a blank or whitespace-only entry
is an error naming it (`error: list.txt: entry 3 is blank`); the rest of the
list still runs. `-` inside a list is the file named `-`, never stdin: only
a positional `-` reads stdin. Each file is formatted once, in the order given. A PATH that does not exist
and a glob that matches nothing are errors (`error: nope.cfc: No such file
or directory`, `error: no files match src/**/*.cfml`); the rest still runs.
`--git staged|unstaged|all` takes the inputs from git instead (below); it
conflicts with PATHS, `--files-from`, `--stdin` and `--stdin-filepath`
(a usage error, exit 2).
A flag with no input is a usage error: name `.` to format the current
directory (`cfformat` alone prints the help and exits 2). `--cfm` needs a
directory or `--git`. A view partial that closes a
tag opened elsewhere formats like any other file. A stray HTML closing tag
prints bare where it is; a CF closing tag the file cannot pair (`</cfif>`),
or a CF tag it opens and never closes, makes its region a recovered one:
printed as written, with a `path:line: not formatted: a stray closer` (or
`an unclosed block`) warning, and the rest of the file formatted around it
(`crates/cfformat-cli/tests/cli.rs` `a_view_partial_formats`).

**Encoding.** A source is UTF-8 (a BOM is kept as it is). A file that is not
valid UTF-8 fails (`path: not valid UTF-8`, exit 2) and is never written;
stdin that is not prints `error: <stdin>: not valid UTF-8` and nothing on
stdout. Nothing is transcoded.

**Output.** One file → the formatted text on stdout. Several files need a
mode (`error: 2 files given; use -w, --check or --diff`), except `cfformat DIR`
or `cfformat --git WHICH` alone, which formats without writing and reports
`N files, L lines, M would change, K failed, T s`.

- `-w` writes each file whose text changes (an unchanged file keeps its
  mtime) and prints nothing per file. A write replaces the file whole: the
  text goes to a temp file beside it (`.<name>.cfformat-<pid>-<n>.tmp`, the
  original's permissions) that is renamed over it, so a failure leaves the
  original as it was; a symlink is written through (the link stays a link, its
  target changes). A file the user cannot write fails as before, and so does
  one in a directory the user cannot write (the temp file cannot be made).
- `--check` prints each file that would change, one path per
  line on stdout; exit 1 if any.
- `--diff` prints a unified diff per changed file on stdout (`--- a/path`,
  `+++ b/path`); exit 1 if any. With `--check` too: the diffs, exit 1.

A run over several files or a walked input, and any run with `--timing`
(one file or stdin too), ends with a summary on stderr —
`12 files, 3 written, 0 failed, 0.02 s` or `12 files, 3 would change, 0
failed, 0.02 s`, with `, R not formatted` before the time when the parse
recovered in R regions — then, when an island was handed off or with
`--timing`, `islands: N formatted, M cached, K warnings, Ts`. Each region
the parse recovered in prints `path:line: not formatted: <reason>` on
stderr and is printed as written; it never changes the exit code. A file that fails prints
`path: message` (it could not be read or written) or `path: internal error:
message` (a panic, caught per file) and the run goes on; a single file on
stdout reads `error: path: …`.
A write to stdout that fails is `error: stdout: <message>`, exit 2, and
nothing more is printed; the one exception is the formatted text of one file
(or `doc`'s) to a reader that stopped (`cfformat a.cfc | head`), which ends
quietly with exit 0.
`--quiet` drops the summary, the `islands:` line and every warning; errors,
the `--check` list, the diffs and the `--timing` lines still print. `-j N`
formats N files at a time (default: the available cores); the output is in
the order the files were given whatever N is.

**Exit codes**: 0 nothing to change or everything written; 1 `--check` /
`--diff` found a change; 2 a usage error (git failing under `--git`
included), an unreadable, non-UTF-8 or unwritable file, a failed write to
stdout, a settings error, an internal error — 2 wins over 1. There is no parse error: both front ends are total.

**Arrange.** `cfformat arrange` runs `arrange` through the same driver
(`run_files` over a `Task`): the same inputs, output modes, messages and
exit codes (`--git` included), with three differences. A directory is walked for `.cfc` and
`.cfs` files only (no `--cfm`; a `.cfs` holds no component, so it never
changes); no settings are read (no `--config`, no discovery, so a
broken `.cfformat.json` cannot fail it); there are no islands and no
`--timing`. A body the parse recovered in prints `path:line: not arranged:
<reason>` and counts in the summary's `, R not arranged`; it never changes
the exit code. `--properties` sets `ArrangeOptions::properties`;
`--first NAMES` (comma-separated or repeated) sets `ArrangeOptions::first`,
an empty name dropped, so `--first=` sets it to none; without the flag it
is the default, `init`.

**Git.** `--git WHICH` selects the files git reports as changed, in the
whole repository whatever the current directory (`cfcli::git_changes`):
`staged` is `git diff --cached --name-only --diff-filter=ACMR` (the index
against `HEAD`, or against nothing before the first commit); `unstaged` is
`git diff --name-only --diff-filter=ACMR` (the working tree against the
index) plus `git ls-files --others --exclude-standard` (untracked, not
ignored); `all` is the union. `git` runs from `PATH`: `git rev-parse
--show-toplevel` in the current directory finds the root, and each listing
runs with `-C <root>` and `-z`, so its paths are root-relative whatever the
directory or `diff.relative`. The paths are deduplicated, sorted in path
order, and kept only when the working tree holds them as a regular file (a
file staged then deleted, a symlink, a submodule are not selected); then
the walk's extension filter applies (`.cfc`, `.cfs`, and `.cfm` with
`--cfm`; `arrange`: `.cfc` and `.cfs`). The hidden-entry and ignore rules do
not: git's listing is the selection. A file under the current directory is
named relative to it (`sub/a.cfc`), any other as the root joined with its
path. The run counts as a walk: it ends with a summary, and nothing
selected is `0 files, …`, exit 0 (`--check` too). A failure is `error:
--git: <git's message, its "fatal: " dropped>` (`error: --git: not a git
repository (or any of the parent directories): .git`), or `error: --git:
git: <the IO error>` when git cannot be run (`git: No such file or
directory`); exit 2, nothing formatted. `-w` writes the working tree's copy
and never runs `git add`: a pre-commit hook that wants the index formatted
adds the files it wrote. Any other list goes to `--files-from`, whose
entries are relative to the current directory, where `git diff
--name-only` prints them relative to the root.

**stdin.** `--stdin` (or `-` as the only PATH) reads the source from stdin
and prints the result on stdout (`--check` / `--diff` work too, `-w` does
not); settings come from the current directory and messages name it
`<stdin>`. `--stdin-filepath PATH` implies `--stdin` and formats the source
as if it were PATH, which need not exist: PATH's settings apply, an island
is formatted as `PATH.js` under the configuration found from PATH's
directory, and messages name PATH. PATH decides the mode as a file's name
does (below).

**Mode.** Unless `--script` or `--tags` forces one, each file's name
decides where the engines make it certain: a `.cfs` is CFScript and a
`.cfm` a tag template (ASCII case-insensitive; Lucee runs a `.cfs` as
script, and Lucee and Adobe print the statements of a `.cfm` as text). A
`.cfc` (a script component or a `<cfcomponent>`), any other name, and
stdin without `--stdin-filepath` go as `Mode::Auto`, resolved from the
source. The rule is `Mode::or_for_path` (`cfparse`), which `format`,
`arrange`, `doc` and `cfvet` all apply to the name as given: a file named
on the command line, walked, from a list, or `--stdin-filepath`'s PATH.

**Settings** are discovered per file, first found wins: the nearest
`.cfformat.json` walking up from the file's directory (stopping after a
directory that contains `.git`) is the project's settings, whole; only when
the walk finds none is `~/.cfformat.json` (`%USERPROFILE%` on Windows) used,
if it exists, so a project's output never depends on the machine's home
file. `--config FILE` is merged over whichever file was found, key by key.
`cfformat settings PATH` lists the files used (`sources:`). There are 47
keys (`cfformat settings --schema`, `SETTINGS.md`); CommandBox's 77-key
files load as they are. `multiline.comma` (`"trailing"`, `"dangling"`,
`"dangling_all"`, `"leading"`, `"leading_tight"`) is one comma setting for
every delimited list, where `"dangling"` dangles struct and array literals
only and `"dangling_all"` argument and parameter lists too; it replaces the
per-construct `*.multiline.comma_dangle`, `*.multiline.leading_comma` and
`*.multiline.leading_comma.padding` keys, which still load: each
construct's keys resolve to one style, and the styles merge silently when
one value gives them (literals dangling with argument and parameter lists
trailing is `"dangling"`, all dangling `"dangling_all"`); any other
disagreement warns once and the first construct in the file decides
(`settings --migrate` then names the value it wrote), and
`multiline.comma` itself wins over all of them. The list thresholds stay per construct: a list with at least
`*.multiline.element_count` items whose items average more than
`*.multiline.min_item_length` columns flat prints one item per line; an
`element_count` of 0, every construct's default, turns the threshold off, so
a list breaks only when it does not fit. CommandBox's `*.multiline.min_length`
measured the whole list's width (its defaults were 4 / 40, and a count of 0
meant "at least 0 items"); it is removed with a warning, since a width has
no exact equivalent as an average. `tags.body.indent` (`"always"`, `"cfml"`) says whether a broken
paired CF tag body is always indented one level (the default) or, under
`"cfml"`, printed at the tag's own indent when its first node — whitespace
and comments skipped — is not a CF tag (HTML, text, `#expr#`); each tag is
judged on its own, once, from the part before any `<cfelse>`, and HTML tag
bodies always indent. `strings.convert_nested_quotes` is `"always"`,
`"never"` or `"fewer_escapes"`; its old values `true` and `false` still load,
as `"always"` and `"never"`, with a warning (`` `true` is now `"always"` ``).
A removed or renamed key, or an old value, warns once per settings file per
run (`warning: <file>: …` on stderr); an unreadable or invalid settings file
is an error naming it, printed once, and fails every file it applies to.

**Island formatting** (`islands.js`, `islands.css`, `islands.json`: `"oxc"`
or `"off"`). The default, `"oxc"`, formats a pure island in process with the
oxc formatter crates (a fraction of a millisecond an island, nothing on
`PATH`); its output
is what prettier prints with its default options (double quotes, semicolons,
trailing commas `all`, arrow parens always), at cfformat's `indent_size`,
`tab_indent` and width, or with the options of the project's formatter
configuration (`islands.config`, `"auto"` by default): from the file's
directory upward (stdin: the current directory's), the first directory
holding `.oxfmtrc.json`, `.oxfmtrc.jsonc`, a `package.json` with a
`"prettier"` key or one of prettier's `.prettierrc` names (prettier's search
order) supplies it. The file is read as JSON with comments and trailing
commas (the extensionless `.prettierrc` prettier accepts as YAML is almost
always that); the options oxfmt maps onto the oxc formatter apply —
`singleQuote`, `jsxSingleQuote`, `quoteProps`, `trailingComma`, `semi`,
`arrowParens`, `bracketSpacing`, `bracketSameLine`, `singleAttributePerLine`,
`objectWrap`, `experimentalOperatorPosition` — with `overrides` matched
against the island's synthetic path (`page.cfm.js`) relative to the
configuration's directory, prettier's way (a pattern without `/` matches the
basename). The glob syntax read is `*`, `**`, `?`, `[…]` and `{a,b}`;
micromatch's extglobs (`@(…)`, `+(…)`, `!(…)`, `?(…)`, `*(…)`) are not: such
a pattern warns and matches nothing, the entry's other patterns still apply.
A `package.yaml` counts only when its `prettier:` key has a value other than
`false` or `null` (and is then unread YAML); an unclosed `/*` makes a
configuration file not JSON. cfformat's layout wins:
`printWidth`, `tabWidth`, `useTabs` and `endOfLine` are not read, and
neither is `.editorconfig`. Plugins are never loaded (the tailwind class
sorter is JavaScript), and ignore files do not apply (an island is not a
file). A YAML, JSON5, TOML or JavaScript configuration, a shared
configuration package (`"prettier": "@acme/config"`) or a file that cannot
be read prints `warning: <file>: <message>` once per run (`--quiet` drops
it) and oxc's defaults apply; the walk does not go on to another directory's
file. `"islands.config": "off"` uses oxc's defaults everywhere.
The island is handed over as written (blank lines before and after
dropped), and its output is printed at the tag's indent, except the lines
that start inside a template literal, a string continued over a line or a
block comment oxc prints raw (one whose lines do not all start with `*`):
those keep their source columns and trailing whitespace, so formatting never
changes a string. An island oxc cannot parse prints
`file:line: islands.js: <oxc's message>` and stays verbatim; the exit code is
unaffected. `"off"` prints the island as written, shifted as a whole to the
tag's indent — unless its text may hold a string spanning lines (JS: a
backtick or a line ending in `\`; CSS: a line ending in `\`, which is not
handed to oxc either; `<cfquery>`: a string, quoted identifier or dollar
quote spanning a line, or one the scan cannot close, under any dialect
reading — a comment's quote counts for nothing; `<cfjava>`: `"""`), which
keeps every line as written, not shifted, trimmed or re-indented (only its
whitespace-only edge lines are dropped); the same rule
holds for every verbatim island (impure, refused, in a code fence,
`<cfquery>`, `<cfjava>`). A `type` the formatter does not know, one holding
CFML (`type="#kind#"`), or a tag whose attributes CFML can emit (a `#…#` or
CF tag where an attribute name or an unquoted value stands) names no
language: the body is data, never handed off, printed line for line (its
edge lines and trailing spaces too) whatever the options. Neither keeps the
source's line endings: every line break is the output's (`newline`), as
everywhere else, so a CRLF source under `"newline": "\n"` loses its CRs
inside these bodies too. One cache per process, shared by every worker:
identical requests run once, whatever the directory, when the configuration
that applies is the same and the island sits at the same indentation (the
text is handed over as written); a formatter plugged in with
`Islands::with_formatter` is cached per path too. A refusal is cached like
a result, a parse error and an island past the limits alike (`NESTING_LIMIT`,
`SIZE_LIMIT`), and warns every time it is met. `--no-islands` prints every
island verbatim whatever the options say. `--timing` prints `path: parse Xms
doc Yms print Zms islands N/W Tms total Ams` per file (`N` islands handed
off, `W` warnings, `T` in the island formatter, `A` the whole format, which
includes what the three phases do not, such as freeing the tree and the
document), then the run's summary.

`doc` prints the document for one file as the printer sees it, breaks
propagated (Prettier's `--debug-print-doc` notation). `settings` resolves the options for a file, a
directory (as for a file in it) or `-` (the current directory; also the
default), prints `sources:` and one settings file per line (or `sources:
none`) to stderr and the effective options as one JSON object with sorted
keys to stdout; `--defaults` prints the defaults and reads nothing;
`--schema` prints a JSON Schema (draft 2020-12) for `.cfformat.json`.
`settings --migrate [FILE]` rewrites a settings file (default
`./.cfformat.json`) with its removed and renamed keys and old values
migrated, so that it loads without a warning to the same options: each
migration warning prints as a run prints it (`warning: FILE: key: message`),
then `FILE: N keys migrated`, followed by `; multiline.comma: "VALUE" chosen,
see the warning` when old comma keys disagreed, since the rewrite silences
that warning. A file with nothing to migrate is not written
(`FILE: nothing to migrate`); one that does not load (unreadable, not a JSON
object, a key that is no setting, a wrong value) is the settings error it
always is, exit 2, and stays as it was. The object is pretty-printed, the
surviving keys in the file's order (an old value rewritten in place) and the
renamed ones after them; a comment cannot survive (the file is JSON). `-`
reads the object from stdin and writes it to stdout.
`--migrate` takes no `--config`, `--defaults` or `--schema`.
