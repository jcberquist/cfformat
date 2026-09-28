# Changelog

## 0.1.0 — unreleased

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
interface, script or tags (`init` first, then by access, then by name)
and, with `--properties`, the properties within the groups their blank
lines make. A member moves with its attached comments; banners, other
comments, statements and other tags stay where they are and divide the
body into sections ordered on their own. It moves text as written and
never formats.

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
