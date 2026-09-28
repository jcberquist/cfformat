# cfparse

CFML parse tree from two hand-written front ends, neither with a
dependency: a CFScript parser (`src/script/`) and a tag scanner
(`src/tags/`). `build.rs` compiles `data/*.json` (the script parser's
built-in function and script tag lists) into tables. Both
front ends are total, so parsing cannot fail.

```rust
let tree = cfparse::parse_source(src, cfparse::Mode::Auto);
println!("{}", cfparse::debug::format_tree(&tree, Default::default()));
```

The tree keeps delimiters (`open`/`close`), splits delimited elements into
`items` with their separators, wraps CFScript statements, and wraps embedded
JS/CSS/SQL/Java/JSON in `Island` elements whose host text is coalesced while
CFML tags and `#expr#` stay structured. A `<script>` / `<style>` body's
language comes from its `type`: absent or empty is JavaScript (CSS for a
`<style>`), a JavaScript MIME type or `module` JavaScript, a JSON one
(`application/json`, `+json`, `importmap`, `speculationrules`) JSON,
`text/html` HTML; any other value — and a value holding CFML, such as
`type="#kind#"` inside `<cfoutput>`, which only the server knows, or CFML
where the server could emit a `type` (`<script #attrs#>`, `src=#url#`) —
makes the body an island of no language (`Lang::Unknown`): its text with
CFML still live. A newline around the `=` is whitespace.

The post-passes run inside `parse_source` (`src/postpass/`), each because
it needs the whole tree, in this order: tag pairing, the file-wide CF tag
walk that marks unpaired CF tags as recovered regions ("Recovered
regions" below), item comments, the expression pass (below), and last the
fold of nested regions into `Tree::recoveries`. The first and third:

- **Tag pairing**: an opening and closing tag with the same name become a
  `tag-body` element whose children are `[open tag, …body…, close tag]`
  (`Element::{open_tag, body, close_tag}`). CF tags pair first and apart
  from HTML tags, as the engines read them: each list is walked for
  CF tags with every HTML tag as content, then for HTML tags over the result
  and inside every CF body, with CF bodies as content — so a CF body survives
  an HTML tag that crosses it (`<html>…<cfoutput>…</html></cfoutput>`: the
  `<html>` and `</html>` stay bare). Unmatched opening and closing tags stay
  bare (a view partial, a CF / HTML mis-nesting), and the body of a closing
  tag that never pairs is paired again one level out. A bodyless form
  (`<cftransaction action="commit">`, `<cfthread action="join">`:
  `BODYLESS_WHEN`) is never a pairing candidate, and pairing nests at most
  `MAX_TAG_DEPTH` (1000) bodies in one list, CF and HTML together.
- **Item comments**: line comments around a delimited item move into the
  item's `leading` / `trailing` (JSON `"leading"` / `"trailing"` arrays,
  `leading:` / `trailing:` in `--tree`), with the whitespace around them.
  `trailing` can span the separator, so order an item's nodes by span to
  recover source order.

A comment after a statement, `if`/`try` chain or `case` is always a sibling
of it, and HTML attribute values are `string` elements (`in_tag: true`).

**Key-values** are built by the front ends, as the element holding them is
finished (`src/key_value.rs`): `key-value` elements for struct members, named
arguments, parameter defaults and `cffile(…)` attributes (the script parser,
per item) and for tag and declaration attributes (the tag scanner for a
tag, the script parser for a header or a tag in script); the separator is
always `punct.key-value`. Pairing and recovery read a tag's attributes
through them.

**Declarations** are the script parser's too: a header and the body it
reads after it become one `function` (`arrow` for `=>`), `class`,
`interface` or `static-block` element, trivia between them inside; a
body-less `function f();` stays a `function-decl`.

**Statement kinds** are the script parser's: `statement` carries `stmt`
(`assignment`, `expression`, `declaration`, `keyword`, `flow`, `function`,
`class`, `interface`, `static-block`, `property`, `param`, `script-tag`,
`import`, `empty`), the kind of the rule that read it. An expression
statement is `function` while its run is a lone function, `expression`
otherwise, and `assignment` when the expression pass builds its run into
one. A lone `static` / `final` / `abstract` statement the parser split off
a declaration on the same line is fused into it: `static foo = 1;` is one
`declaration { static, assignment }`.

The expression pass gives expressions structure, the only pass that needs
precedence:

- **Expressions**: a Pratt parse of every run of operands and operators into
  `assignment`, `ternary`, `binary` (`prec`; one n-ary node per
  same-precedence run), `unary` (`postfix`), `call-expr`, `new`, `chain` and
  `segment` (`property` / `method` / `index`, `safe`, `static`). Whitespace
  and comments between a node's first and last part are its children. A run
  that does not parse stays flat; a lone operand gets no wrapper.

```text
x = a.b(1) + 2;

statement assignment terminator=";"
  assignment
    ident.variable "x"
    ws " "
    op.assign "="
    ws " "
    binary additive
      chain
        ident.variable "a"
        segment method
          punct.accessor "."
          ident.call "b"
          call open="(" close=")"
            item
              lit.number "1"
      ws " "
      op.additive "+"
      ws " "
      lit.number "2"
```

`cfparse::nodes` has a view per kind (`Element::as_assignment`,
`as_binary`, `as_chain`, …) that names the parts and skips trivia. The
tokenizer merges `?:` into `op.elvis` and keeps whitespace out of struct key
tokens. No token spans whitespace between words: a multi-word operator
(`less than`, `is not`, `does not contain`) is a `phrase` element of word
tokens and the whitespace between them, read as one operator, and `else if`
is the tokens `kw.else` and `kw.if`. A call's arguments are a `call` element; what is called is its
callee's kind: `ident.builtin` for a built-in function, `ident.call` for
a user-defined function or a method.

### Kinds

A kind exists because a reader tells it apart — the printer (`cfformat`),
`cfformat arrange`, `cfvet` — or building the tree needs it (the key-value
rules, a post-pass); a distinction nothing reads is not a kind (a tag name
is `ident.tag-name` whatever the tag).

| token kind | holds | read by |
|---|---|---|
| `Whitespace`, `Newline` | trivia; a run ends after its first `\n` | every reader that skips trivia |
| `Keyword(_)` | one variant per keyword word | the expression pass (`new`, `component`), the printer (`var`, `=>`, `return` / `throw`, `component`), cfvet (`var`, `default`), arrange (`function`) |
| `Operator(_)` | `Binary(Prec)` and `AugAssign(Prec)` with the level the lexer gives them; `In`, `Assign`, `TernaryQ`, `TernaryColon`, `Sign`, `Increment`, `Postfix`, `Not { word }`, `Spread` | the expression pass (levels, prefix, postfix), the printer (`in`, a word `not`, a simple unary, `=`), cfvet (`in`) |
| `Punct(_)` | terminators, commas, `KeyValue`, accessors, `Open` / `Close(Delim)` | the key-value rules (`KeyValue`), the expression pass (accessors), the printer; a `Delim` by the recovery check (an unclosed `Paren`, `Brace`, `Bracket`, `String` or `Template` is a region, a `Fence`, `Tag` or comment is not) and the printer (`TagComment`: a `<!--- --->`) |
| `Ident(_)` | a name by its role | the key-value rules (keys, attribute names), the expression pass (operands), the printer (`Call` / `Builtin`, `This`, tag and attribute names), cfvet (variables, scopes, parameters), arrange (function and property names) |
| `Literal(_)` | numbers, booleans, null, unquoted values, string text, escapes | the key-value rules (`Unquoted`), the expression pass (operands), the printer, arrange, cfvet |
| `Storage(Type \| Modifier)` | a type name; an access or storage modifier | the expression pass (`new java`), arrange (access), cfvet (`var` as a type) |
| `CommentText`, `DocTag` | comment text, one token per line; a doc comment line's leading `@tag` | arrange (`@access`) |
| `Text` | host text (an island's, HTML's) | cfvet |
| `Ignore`, `Invalid`, `Other` | a `cfformat-ignore` region's text; text no rule read | the printer (as written), the recovery pass |

Element kinds are the delimited forms (`struct`, `array`, `typed-array`,
`call`, `parameters`, `block`, `group`, `brackets`, `string`,
`template-expression`), the declaration headers and the statement and
clause elements the script parser builds, comments, the tag elements
(`cf-tag`, `html-tag`, `doctype`, `tag-body`, `island`, `tag-island`),
`ignore` and `recovered`, and the expression structure above, `phrase`
included; each is read by the printer's dispatch, and the expression pass
builds its kinds from the rest.

A `cf-tag` also carries its `CfKind` (JSON `cf_kind`, `--tree` after the
shape; `Element::cf_kind` reads it through a `tag-body` too): how the tag
scanner read it, and the branch tags of a `<cfif>`, so that readers match
the kind rather than the name.

| `CfKind` | tags | read by |
|---|---|---|
| `Expression` | `<cfset>`, `<cfreturn>`, `<cfif>`: script after the name | the scanner, the printer (script, not attributes) |
| `ElseIf`, `Else` | `<cfelseif>` (script after the name, as `Expression`), `<cfelse>` | the printer (a branch splits its body), cfvet (`<cfif>` branches) |
| `Script` | `<cfscript>` | the scanner, the printer (statements at the tag's indent) |
| `Query`, `Java` | `<cfquery>`, `<cfjava>` | the scanner (island bodies), the printer (the island's owner) |
| `Function` | `<cffunction>` | the scanner (`name`, `access`, `returntype`; an HTML body), arrange, cfvet (a function unit) |
| `Property` | `<cfproperty>` | the scanner (`name`), arrange |
| `Output` | `<cfoutput>`, `<cfmail>` | the scanner (a body where `#…#` is read) |
| `Class` | `<cfcomponent>` / `<cfinterface>` at the head of the file, and its closing tag | the scanner (`extends`) |
| `Generic` | every other tag, custom (`<cf_x>`, `<p:x>`) and extension (`<cfx_x>`) ones included | — |

A closing tag has the kind of an opening tag of its name. The kind is the
scanner's reading, not the name's: `<cfset2>` and `<cfoutputs>` (no `\b`
after the tag's name) and `<cfelse:x>` (a custom tag) are `Generic`, and
so is a `<cfcomponent>` past the head of the file, which arrange and cfvet
therefore still find by name. The HTML tags have no kind: the printer tells
`<script>`, `<style>`, `<pre>`, `<textarea>` and its block tags apart by
name.

```text
cfparse parse <FILE|-> [--script|--tags] [--json|--tree] [--spans]
cfparse parse <DIR> [--cfm]
```

Snapshots: `UPDATE_SNAPSHOTS=1 cargo test -p cfparse --test snapshots`.

Two readers for the tree's consumers (`cfformat arrange` and `cfvet`):
`nodes::plain_text` (`KeyValue::plain_text` for an attribute) is the text
of a value written as text alone — a string with no `#…#` and no escapes,
or an unquoted word that is not a variable — and `script::is_scope_name`
says whether a word written as text names a scope, as the script lexer
reads one.

Corpus scan (`Other`/`Invalid` tokens, terminator-only empty statements,
comments left at the tail of a statement/chain/case, bare closing tags,
operators outside an expression node, unparsed expression runs, with source
lines, plus statement-kind / chain / binary histograms; needs the sibling
`commandbox-cfformat` checkout):
`cargo test -p cfparse --test scan -- --ignored --nocapture`.

## Front ends

`parse_source(src, mode)` sends script — `Mode::Script`, and `Mode::Auto`
when the source starts like a component, an interface, an `import` or a
`//` / `/*` comment (a licence in a `<!--- --->` above `component` included)
— to `script::parse`, and tag mode to `tags::parse`; the same
post-passes run on either root. Two consequences for callers: leading
script comments followed by a tag (`// a` then `<cfcomponent>`, a licence
block above `<cfscript>`) select tag mode, not script; and an ordinary
script snippet with no such start (`x = 1;`) is tag mode under `Mode::Auto`
— pass `Mode::Script` (the CLI's `--script`) for it. `parse_source` reads
the text alone; a caller with a file name resolves the mode first with
`Mode::for_path` (or `Mode::or_for_path` over a requested mode): `.cfs` is
`Script` and `.cfm` `Tags`, as the engines run them, and anything else,
`.cfc` included, stays `Auto`. Both binaries do.

- **Script**: `src/script/` is a hand-written recursive-descent parser
  (`parser.rs` the statements, `expressions.rs` expression runs, literals
  and calls, `tags.rs` the tag-in-script forms, `comments.rs` trivia,
  `lexer.rs` the byte scanners and word lists). It never fails: text no
  rule matches is an `other` run, or a recovered region (below); past 100
  nested levels the rest of the file is one such run. `script::parse` returns `Err(Declined)`
  only for a source that is not script — the one way either front end
  returns no tree.
- **Tag mode**: `src/tags/` keeps the HTML / CFML interleaving and delimits
  the script inside it — a `<cfscript>` body, the expression of `<cfset>` /
  `<cfreturn>` / `<cfif>` / `<cfelseif>`, a `#…#` in text, an attribute or
  SQL — with the routines that find where each such region ends
  (`islands.rs`, over `scan.rs`). Each region goes to
  `script::parse_fragment` and its nodes are spliced in.

### The tag front end

`src/tags/` is a hand-written, dependency-free byte scanner over the whole
normalised source: `mod.rs` the two entry points, `scanner.rs` the state, the
content loop and the emit helpers, `html.rs` the HTML side, `cf.rs` the CFML
side, `islands.rs` the islands and the script regions. Its output is
line-at-a-time: no region spans a line end.

`tags::parse(source)` (public, but hidden from the docs: the modules
`script`, `tags` and `postpass` are there for the crate's own tests and
benches, not the API) returns the pre-post-pass root, and
`tags::parse_fragment(source, range, depth)` (internal, `pub(crate)`) the
nodes of a ```` ``` ```` tag island in script, at the script parser's
depth. Neither can fail. `script_body_end` and `angle_end` find where a
script span ends and hand it to `script::parse_fragment`, using the shared
string-, comment- and `#…#`-aware scanners of `src/scan.rs` (`comment_end`,
`tag_comment_end`, and `Bounded` for strings and `#…#`). `Bounded` treats
the region's boundary — `</cfscript>`, or a tag expression's `>` / `/>` —
as hard inside a `#…#`: a `#` or a string that does not close before it is a
bare character, so `x = "price #";` or `<cfset x = "price #">` ends at its
own closing tag and the script parser reports the broken statement, where
the unbounded `hash_end` / `string_end` read on to the next `#` or the end of
the file. A closed string still hides the boundary (`<cfset x = "<b>">`,
`#"</cfscript>"#`) unless it is inside a `#…#` — or holds `</cfscript>` —
and runs past the end of the line it holds the boundary on. Failed
constructs are remembered, a `#…#` fails at `MAX_DEPTH`, and a budget of 32
reads per byte stops what those miss, so the retries stay linear. The same
module holds what the script parser's lookaheads use — `balanced_code` (the
arrow and `for-in` lookaheads), `java_literal_end` (a `type="java"` body) —
`closes_tag` (`</name\s*>`, any case, every CF closing-tag check) and the
ignore markers every reader of script or tags goes through: `script_marker`
(`//`, spaces or tabs, the word, spaces or tabs and the line's newline; or
`/*`, the word and `*/` with any whitespace around it) and `tag_marker`
(`<!---`, the word and `--->`, any whitespace around the word). An
event or style attribute island is parsed to its end as any quoted value is:
the first quote outside a `#…#` (in a `<cfoutput>`-like context), a CF tag or
a `<!--- --->` comment, so `style="a;<cfif x eq "b">c</cfif>"` is one value.

Deliberate lexical rules: keywords and `true` / `false` / `null` are
case-insensitive (`IF`, `Var`), text printed as written; `$` is a word
character for every `\b` (`var$foo` is one identifier); a number takes an
exponent when a digit follows the `e` (`1.2e-3`); custom-tag names take digits
(`<cf_foo2>`, `<ns:foo2>`); a line starting with any binary operator of the
expression tables continues the statement (`lte`, `does not contain`, …), and
the tables add `equal` / `not equal`.

`MAX_DEPTH` (100, `lib.rs`) is one budget for both front ends and the
boundary scanners: the script parser counts statement lists,
expressions, binding patterns, unbraced bodies (`if (a) if (b) …`, `else
if`, `while`, `for`, `do`), labels and statements directly in a `switch`
block; the tag scanner counts content runs and every CF tag an island body
opens (`<cfquery>` in `<cfquery>`); a fragment one front end hands the other
starts at its caller's depth. The `scan.rs` scanners (`hash_end`,
`bracket_end`, `string_end`, `balanced_code`) take the
caller's depth and, at the bound, scan flat to their own closer (brackets
by counting their own pair, a `#`, quote or other bracket then a plain
character). A statement that reads on after `abort` or `cffile(…)` without
a `;` (neither ends its statement) does so in a loop, into
the same element, so a run of them takes no depth. The expression post-pass collects a prefix chain in a loop and
folds an assignment chain from the right, and leaves a run flat — a
`TooDeep` region — rather than build a tree deeper than `MAX_TREE_DEPTH`
(410: a level of the budget adds at most four elements, `switch (x) { case
1:` or `a.b(`). So no input overflows a 2 MB stack in release (debug frames are larger, on Windows past 2 MB at the bound, so the debug tests run with headroom),
and every recursive walk of a tree — the passes, the printer, `Drop` —
stays within `MAX_TREE_DEPTH` (tag pairing nests up to its own
`MAX_TAG_DEPTH`, 1000). At the
bound the content loop does what `script`'s `too_deep` does: the rest of the
source up to its trailing whitespace becomes unmatched text — one region per
line, `text` inside HTML and `other` inside `source.cfml` — the source ends
there for every enclosing routine, every open element closes without a
closing tag, and the entry point takes the trailing whitespace. A `<!--- --->`
comment is the exception: it stops *nesting* at the bound and a deeper
comment is comment text, because the printer writes `--->` for a comment the
source never closed, so cutting inside one would add a level on every run.

### Recovered regions

`Tree::recoveries` lists, in source order, the regions the parse did not
understand — `Recovery { span, reason }` — and the tree holds each as an
`ElementKind::Recovered(reason)` element around its nodes, which still tile
the source. `RecoveryReason::Unmatched`: text no rule matched (`b =
@;`, an invalid character in a tag's attributes); `StrayCloser`: a `)`
`}` `]` with nothing open, or a CF closing tag no opening tag took;
`Unclosed`: a block, parenthesis, bracket, string or `#…#` still open where
its list or fragment ends,
or an opening tag of `postpass::NEEDS_CLOSING_TAG` (`<cfif>`, `<cfloop>`,
`<cfoutput>`, …) left unpaired. For CF tags "unpaired" is a file-wide walk's:
every CF tag in source order, whatever element holds it (a tag body,
an island, a `<cfscript>` body, a code fence), HTML tags ignored, paired as
above; a pair the walk makes but the tree cannot hold — a `<cfoutput>`
opened inside a `<style>` island and closed after it, a `<cfif>` closed
inside a `<cfquery>` — is two bare tags, not a region. `TooDeep`: nesting past `MAX_DEPTH` (the cut
text) or a run the expression pass left flat. The region is the statement
the parse gave up in (in script; it takes in, on its line, an unterminated
statement before it and a lone `;` after it: `b = @;` is one region), the
innermost tag body around the problem (in tag mode), or — outside any — the
run, the tag or the unclosed element itself; an unclosed tag outside any
body runs to the end of its list. A region inside another is folded into
the outer one, which takes `TooDeep` when the inner had it. The post-passes
do not build inside a region, though what the front ends built as they
read it (key-values, declarations) is there; `debug` prints `recovered <reason>`, the JSON
a `reason` and, when there are any, a top-level `recoveries` list. A
formatter prints a region as written (`cfformat` does, with a warning).
`Tree::line_of(offset)` is the 1-based line of an offset, for a warning: it
counts the newlines before the offset on every call, a scan of the source
that is fine at one call per warning and not meant for a loop over nodes.

Two fuzz passes run over every fixture and every `.cfc` and `.cfm` of every
corpus, on a thread per corpus with a 2 MB stack in release (`cfformat` is a dev-dependency for
this). The **fixed cases** run first (`tests/common/mod.rs` `generators`):
every input shape that once overflowed the stack — `!` × 20,000, `x=` ×
20,000, `if(x) ` × 20,000, 400,000 brackets in a `#…#`, `<cfquery>` ×
20,000, `a()` × 200,000, the string ↔ `#…#` cycle, `abort ` and
`cffile(…) ` × 4,000 in a `<cfscript>`, … — and the shapes that were once
quadratic — unclosed `#`s retried, `<cfif a>` × 20,000 then `</cfoutput>` ×
20,000 (and `<div>` / `</span>`, in and out of a CF body) — at the size
of the first fuzz cases and ten times it (about a minute and a half of the
passes' time in release). The truncation pass cuts each source at each eighth of its length and
parses each cut in its own mode and as script, then formats it. The mutation
pass makes `CFPARSE_FUZZ_MUTATIONS` (8) single edits per source from a
seeded xorshift — `é`, `💩`, U+0301 or a delimiter (`"`, `'`, `/*`, `*/`,
`<!---`, `--->`, `#`, brackets) inserted at a character boundary, or a
character deleted — and also checks that both parses tile the source with
every token boundary on a character boundary and every recovered region on
token boundaries. The truncation pass checks that every recovered region
reached the output as written (its text without whitespace is a run of the
output's). Both: no panic, and — a stack overflow being an abort that kills
the process — a finished run:

```text
cargo test --release -p cfparse --test fuzz -- --ignored --nocapture
CFPARSE_CORPUS=/a:/b …         # more corpora, colon-separated
CFPARSE_FUZZ_TRACE=1 …         # print each case before it runs
CFPARSE_FUZZ_TIMEOUT=60 …      # seconds one case may take
CFPARSE_FUZZ_SEED=… …          # replay the mutation pass (the seed is printed)
CFPARSE_FUZZ_MUTATIONS=8 …     # mutations per source
```

`tests/lexical.rs` holds the lexical cases: token kinds where a rule is
lexical, formatted text where it is visible, and the UTF-8 twins rule (a
multibyte character formats exactly like an ASCII placeholder in its place).

`cargo bench -p cfparse --bench parse` has four groups: `script_parse`
(`parse_source` over `commandbox-cfformat/models`), `script_front_end` (the
front end alone), `tags_parse` (`parse_source` over the checkout's tag-mode
`.cfc` and `.cfm`) and `tags_front_end` (the scanner alone over the same
files).
