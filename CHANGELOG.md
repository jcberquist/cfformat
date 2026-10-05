# Changelog

## Unreleased

**Changed.** The body of a `<cfscript>`, `<script>`, `<style>`, `<cfquery>`
or `<cfjava>` tag is indented one level inside the tag, as Prettier indents
a `<script>` or `<style>` body; it used to print at the tag's own indent.
This changes the output of most templates that hold one of these tags.
Before:
`<script>` ⏎ `var a = 1;` ⏎ `</script>`, the statement at the tag's
column; now: `<script>` ⏎ `    var a = 1;` ⏎ `</script>`, one indent in,
the closing tag where it was. A `<cfscript>` body's statements move the
same way, those of a top-level `<cfscript>` too. A formatted
block is handed to the formatter with that much less width, so a line that
just fitted may now break. A block printed as written (a `<cfquery>`, a
`<cfjava>`, a `<script>` or `<style>` holding CFML, refused, or under
`"islands.*": "off"`) is raised as a whole until no line sits left of the
new indent, and never moved left: SQL written flush with its `<cfquery>`
moves one indent in, SQL written deeper stays where it is. The lines a CF
tag inside such a block breaks onto (`<cfset x = {` ⏎ …) follow that
indent. What does not move: lines inside a template literal or another
string spanning lines, and a block that keeps its text line for line (a
`<cfquery>` whose SQL holds a string spanning lines). The new key
`tags.islands.indent` (default `true`) set to `false` gives back the
previous layout, byte for byte; on a file already formatted with `true` it
brings `<cfscript>` bodies and formatted blocks back to the tag's indent,
but leaves a raised block as written where it is.

**Changed.** A `<script>` or `<style>` block inside `<cfoutput>` whose only
CFML is `#expr#` and `##` is now formatted; it used to print as written.
The formatter is handed each `##` as `#` and each `#expr#` as a name as wide
as it prints, and both come back as they were. Before:
`<script>` ⏎ `var api = { url : '/sites/#prc.slug#', token : #toJson( prc.jwt )# }` ⏎
`$( '##form' ).validate()` ⏎ `</script>`; now:
`<script>` ⏎ `    var api = { url: '/sites/#prc.slug#', token: #toJson(prc.jwt)# };` ⏎
`    $("##form").validate();` ⏎ `</script>`. What is kept: the `#expr#`
themselves (printed as any `#expr#` is), and the quote of a string holding
one, whatever the project's `singleQuote` says, since what the expression
emits may hold either quote; a string without one is the formatter's, as in
any block. A block whose `#expr#` would not come back as they went prints
as written with a warning, such as ``islands.js: what precedes the #…# on
line 12 changed from `(` to nothing`` — oxc drops the parentheses of
`(#expr#).call()`, which breaks the page when the expression emits a
function — or ``a `,` was added after the #…# on line 12, before `]` ``;
so does one oxc cannot parse. Left as written with no warning: a block
holding a CF tag or a tag comment, a `#expr#` that prints over more than
one line, a block with nothing but whitespace around its `#expr#`
(`<style>#css#</style>`), and a JSON block holding `#expr#` or `##`. This
changes the output of templates holding such blocks, typically a view's
JavaScript. The new key `islands.interpolated` (default `true`) set to
`false` prints them as written, as before, byte for byte and with no
warning, while blocks with no `#expr#` or `##` are still formatted;
`"islands.js": "off"` and `"islands.css": "off"` print them as written too,
but every other block of their language as well.

**Builtins.** `configImport`, `dbPoolClear`, `isInThread` and `queryLazy`
(Lucee), which cfdocs now lists, are builtins. Their calls are cased by
`function_call.casing.builtin` instead of `.userdefined`: with the default
`"cfdocs"`, `QUERYLAZY(…)` prints as `queryLazy(…)`.

**Changed.** `<script>` and `<style>` blocks are formatted by the oxc
formatter crates of oxfmt 0.72.0 (was 0.70.0). No block of the projects
tried formats differently; what can move is rarer code. A CSS comma list
with a signed item after the first stays on one line when it fits, where
Prettier breaks it: `font-family: system-ui, -apple-system, sans-serif;`
used to print one item per line. Comments around an `=`, between a callee
and its `(`, or among a test call's arguments keep their place and order,
where one could be dropped or repeated: `const b /* c */` ⏎ `= // d` ⏎
`1;` printed `const b = // d` ⏎ `// d` ⏎ `1;` and now prints
`const b /* c */ = // d` ⏎ `1;`; `foo /* c */ (a)` printed
`foo(/* c */ a)` and now prints `foo /* c */(a)`. A block comment whose
other lines are printed as written keeps the trailing spaces of its first
line. A JSON array or object holding only a block comment prints on one
line: `"a": [/* c */]`. Building from source needs Rust 1.97, the new oxc
crates' minimum.

## 0.2.0 — 2026-09-30

**Fixed.** A region printed as written could end inside a two-character
operator: in an arrow function whose parameters the parser gave up on
(`g = ({a,b}) => a+b;` until the fix below), the region ended after the `=`
of `=>`, the `>` started the next statement on a line of its own, and
`cfformat -w` wrote a file no engine parses. The same held for `+=` and
`-=` after a region, printed as `+ =`. An operator after a region is now
one token and prints whole.

**Fixed.** Destructuring (Adobe ColdFusion) is read as a pattern wherever it
can stand, not as a literal or as text the parser gave up on. A struct
pattern as a parameter of a function, closure or arrow (Adobe ColdFusion
2025: `function f({a, b = 2}, c) {}`, `({a, b}) => a + b`) printed as
written with a warning, and the function's body brace still moved to a line
of its own. Two forms printed changed what the code does: a default inside
a parenthesised struct pattern became a rename (`({m, n = 1} = st);`
printed `( { m, n: 1 } = st );`), and a skipped element of an array pattern
was dropped (`[a, , c] = x;` printed `[ a, c ] = x;`, `c` then taking the
second element; the same in `for ([k, , v] in x)`). `{a, b} = x;`, which no
engine runs, printed as a block and then `= x;` with no warning, and
`static {a, b} = x;` (which none runs either) as a static block. A pattern
now prints and breaks like the literal it resembles, under that literal's
settings (`struct.padding` / `array.padding`, the empty padding, the
`*.multiline.*` thresholds, `multiline.comma`), as Prettier prints
patterns: `[a, , c] = x;` keeps its skip, `({m, n = 1} = st);` its
default, a comma after a rest item is never written, and the rename is
always `key: target` (`var {p, q: {r}} = st;`, where a `var` pattern used to
print `q : {r}`), whatever `struct.separator` and `struct.quote_keys` say. A
sole pattern parameter hugs the parentheses and breaks inside itself
(`function f({` ends the first line, `}) {` starts the last), and a struct
pattern that renames to a nested pattern breaks where it is assigned to.

**`cfvet`.** `missing-var` reads destructuring patterns (Adobe ColdFusion).
Before, a pattern's names were not writes (`[a, b] = x;` in a function was
silent, though `a` and `b` land in the variables scope) while its defaults
were (`var [p, q = 1] = x;` reported `q`, which `var` declares). Now a
pattern writes every name it binds, `var` declares every name its pattern
binds, a default's value is read, and a destructuring parameter
(`function f({a, b = 1})`, Adobe ColdFusion 2025) is reported name by name
with a message of its own: the engine binds those names in the variables
scope on every call, not as arguments.

## 0.1.0 — 2026-09-29

The first release: a CFML formatter in one binary, a rewrite in Rust of the
CommandBox cfformat module. It reads CommandBox's `.cfformat.json` files;
what changed in the settings and in the output is in the README's
"Migrating from CommandBox" section.

**Formatting.** CFScript (components, interfaces, `.cfs` script files) and
tag CFML (`.cfm` templates, tag components) from one parse tree. The mode
comes from the file name where the engines make it certain, `.cfs` script
and `.cfm` tags; a `.cfc` is script or tags by how its source starts, and
`--script` / `--tags` force either. Script follows
Prettier's layout rules: a list breaks when it does not fit, call arguments
hug a trailing function or struct, and a broken binary expression, ternary
or `for` header breaks as Prettier's does. Spacing, padding, comma style,
quotes, the casing of built-in functions and the alignment of consecutive
assignments, of `property` and `param` attributes and of doc-comment tags
are settings. Tags: CF and HTML tags and their attributes, tag bodies
indented or flush (`tags.body.indent`), `<!--- --->` comments and
`<cfscript>` bodies. `<pre>` and `<textarea>` bodies print as written, and
the whitespace at the edges of an inline element or between two pieces
written together follows the source, so the page shows the same. CF tags
pair apart from HTML tags, as the engines read them, so a CF body survives
HTML that crosses it. A run of blank lines prints as one. A region between
`cfformat-ignore-start` and `cfformat-ignore-end` comments (or
`@formatter:off` / `@formatter:on`) prints as written. Output is
idempotent, keeps every token and comment, and never changes the text of a
string.

**`<script>` and `<style>` blocks.** A block holding no CFML — JavaScript, a
module, JSON, CSS — is formatted in process by the oxc formatter crates, at
cfformat's width and indentation, with the options of the project's
`.prettierrc` / `.oxfmtrc` when there is one (`islands.config`); nothing
needs to be installed. `islands.js`, `islands.css` and `islands.json` turn
each kind off. A block of a `type` the formatter does not know, or one CFML
supplies, keeps every line as written, as does a `<cfquery>` whose SQL
holds a string spanning lines. A block larger than 256 KB or nested more
than 500 levels deep prints as written with a warning.

**What cannot be read prints as written.** Both parsers are total: any
input formats. A region the parser cannot read — a statement in a script, a
tag body in a template — prints exactly as written, the rest of the file is
formatted around it, and stderr names it (`path:line: not formatted:
reason`); the exit code does not change. The README's Limitations section
lists the valid code this happens to today.

**The command line.** `cfformat [PATHS]...` prints, writes (`-w`), checks
(`--check`) or diffs (`--diff`) files, directories (walked for `.cfc` and
`.cfs`, and `.cfm` with `--cfm`, honouring `.gitignore` and `.ignore`), globs,
`--files-from` lists, the files git reports as changed (`--git
staged|unstaged|all`, the whole repository, no `git add`) and stdin
(`--stdin`, `--stdin-filepath` for editors), in parallel (`-j`), with
output in the order given. `cfformat settings` shows the options that apply
to a path, `--schema` prints a JSON Schema for `.cfformat.json`, and
`--migrate` rewrites a CommandBox settings file for the current keys;
`cfformat doc` prints the document the printer sees. Exit codes: 0, 1 for
a file that would change, 2 for any error. A write replaces a file whole,
and only when its text changes; a source that is not UTF-8 is an error,
never a silent change.

**Settings.** 47 keys in `.cfformat.json`: the nearest file up to the
repository root applies (else `~/.cfformat.json`), with `--config` merged
over it. Every key, its default and an example of each value are in
`SETTINGS.md`. CommandBox's files load as they are; a removed or renamed
key warns on every run until `--migrate` rewrites it.

**`cfformat arrange`.** Orders the functions of each component and
interface, script or tags (`init` first, or the functions `--first`
names, then by access, then by name) and, with `--properties`, the
properties within the groups their blank lines make. A member moves with
its attached comments; banners, other comments, statements and other
tags stay where they are and divide the body into sections ordered on
their own. It moves text as written and never formats.

**`cfvet`.** A second binary in every release, on the same parse tree, with
two checks. `missing-var`: an unscoped write inside a function, which in
classic CFML lands in the `variables` scope; a `var` counts from where it
runs, as on Lucee, so it covers only what follows it in its own block.
`unevaluated-call`: a `#…#` that calls a function in template text the
engine outputs as written, outside `<cfoutput>` and the other places it
evaluates. A `cfvet-ignore` comment silences its own line and the next.
It checks paths, stdin or the `--git` selection. Exit codes 0, 1 with a
report, 2 on an error.

**Limitations.** UTF-8 sources only. Glob syntax is `globset`'s. Not on
crates.io: the oxc formatter crates are a git dependency, and a project
that uses the library must copy the `oxc_allocator` patch into its own
`Cargo.toml`. The README's Limitations section is the full list.
