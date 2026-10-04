# cfformat

`cfformat` is a code formatter for CFML written in Rust. It will format both
script and tag based code (`.cfc`, `.cfs`, `.cfm`). It reads a file, prints
it again at a column width with consistent spacing, indentation, quotes and
line breaks, and leaves what it cannot format untouched. In template files,
any `<script>` and `<style>` blocks that do not contain CFML code are
formatted too, in process, by the [oxc](https://oxc.rs) formatter crates:
the output is Prettier's, at cfformat's width and indentation and with the
options of the project's `.prettierrc` when there is one, and nothing needs
to be installed besides the one binary.

It is a rewrite of the [CommandBox cfformat
module](https://github.com/jcberquist/commandbox-cfformat) and reads its
`.cfformat.json` settings files. Please note that some settings have been
changed or updated (see [Migrating from
CommandBox](#migrating-from-commandbox)).

## How it was built

The code in this repository was written by Claude, working from a design plan
and a log of decisions. Every commit Claude wrote carries a `Co-Authored-By`
trailer naming it.

Prettier's document printer is ported into
`crates/cfdoc` (see [Licence](#licence)).

## Quick use

```text
cfformat --check .               # list the .cfc and .cfs files under . that would change (exit 1 if any)
cfformat --check --cfm .         # the .cfm files too
cfformat --diff src              # show the changes as unified diffs
cfformat -w src                  # write them
cfformat -w --cfm .              # every .cfc, .cfs and .cfm file under .
cfformat Foo.cfc                 # one file: the formatted text on stdout
cfformat 'views/**/*.cfm' -w     # a glob (quoted, so the shell leaves it alone)
cfformat --git staged -w         # the staged .cfc/.cfs files (see "Git")
cfformat arrange -w src          # order each component's functions (see "Arranging members")
```

A directory is walked for `.cfc` and `.cfs` files, and `.cfm` files when
`--cfm` is specified, skipping hidden entries and whatever `.gitignore`
(inside a git repository) and `.ignore` exclude. A file named on the
command line is formatted whatever its extension. A flag with no input is an error — name `.`
to format the current directory — so a stray `cfformat -w` never rewrites a
tree; `cfformat` alone prints the help.

From an editor, pass the text on stdin and the file's path with
`--stdin-filepath`: the settings that apply to that path are used, the path
need not exist, and the result is printed on stdout.

```text
cfformat --stdin-filepath src/models/User.cfc < src/models/User.cfc
```

`--stdin` (or `-`) reads stdin without a path. `cfformat --help` lists every
flag.

The mode — CFScript or tags — comes from the file name where the engines
make it certain: a `.cfs` file is script and a `.cfm` file is a tag-mode
template, whatever they start with. A `.cfc` (which holds either a script
component or a `<cfcomponent>`), a file of any other name, and stdin
without a path are read from the source: script when it starts with a
`component` or `interface` declaration (`abstract` / `final` included), an
`import`, or a `//` or `/*` comment that no tag follows (`<!--- --->`
comments above any of these do not count); anything else is a tag-mode
template. `--stdin-filepath` counts as the name. `--script` and `--tags`
force either mode, over the name too; a script snippet on stdin without
such a start (`x = 1;`) needs `--script`.

## Install

Download the binary for your platform from the [latest
release](https://github.com/jcberquist/cfformat/releases/latest), save it
as `cfformat` (`cfformat.exe` on Windows) in a directory on your `PATH`,
and make it executable. The commands below all run it as `cfformat`. Every
release carries `cfvet`, a second binary, as a second set of assets with
the same names (`cfvet_linux_x86_64`, …); see [cfvet](#cfvet). For example,
on Linux x86_64:

```text
curl -fL -o ~/.local/bin/cfformat \
  https://github.com/jcberquist/cfformat/releases/latest/download/cfformat_linux_x86_64
chmod +x ~/.local/bin/cfformat
```

On Windows, in PowerShell (then add `%USERPROFILE%\bin` to `PATH` if it
is not there already):

```text
mkdir -Force $HOME\bin
Invoke-WebRequest -OutFile $HOME\bin\cfformat.exe `
  https://github.com/jcberquist/cfformat/releases/latest/download/cfformat_windows_x86_64.exe
```

The assets:

| Asset                         | Platform                                            |
| ----------------------------- | --------------------------------------------------- |
| `cfformat_linux_x86_64`       | Linux x86_64 (glibc)                                |
| `cfformat_linux_x86_64_musl`  | Linux x86_64, static (musl)                         |
| `cfformat_linux_arm64`        | Linux arm64 (glibc)                                 |
| `cfformat_linux_arm64_musl`   | Linux arm64, static (musl)                          |
| `cfformat_windows_x86_64.exe` | Windows x86_64                                      |
| `cfformat_macos`              | macOS, universal (arm64 and x86_64, 10.13 or newer) |

The same six exist for `cfvet` (`cfvet_linux_x86_64`, `cfvet_macos`, …).
Each release also carries `LICENSE` and `LICENSE-PRETTIER` (see
[Licence](#licence)).

You can also build and install it with Cargo. It requires Rust 1.96 or newer
and what Rust itself needs on your platform to link a program: a C linker and
the platform's build tools (`build-essential` or similar on Linux, the Xcode
command line tools on macOS, the Visual Studio C++ build tools for the MSVC
toolchain on Windows). Every dependency is pure Rust, so no other native
library is needed:

```text
cargo install --locked --git https://github.com/jcberquist/cfformat cfformat-cli
cargo install --locked --path crates/cfformat-cli  # from a checkout
```

The package is `cfformat-cli` (the command line) and the binary it installs is
`cfformat`; the `cfformat` package is the library (see [Library](#library)).
`cfformat` is not on crates.io: it depends on the oxc formatter crates, which
are not published there either, by git.

## Configuration

Settings live in `.cfformat.json`. For each file, the nearest one walking up
from the file's directory — stopping at the repository root, the first
directory holding `.git` — applies, whole; only when there is none is
`~/.cfformat.json` used. `--config FILE` is merged over it, key by key.

```text
cfformat settings src/Foo.cfc    # which files apply, and the options they make
cfformat settings --schema       # a JSON Schema for .cfformat.json
cfformat settings --migrate      # rewrite an old ./.cfformat.json for the current keys
cfformat doc src/Foo.cfc         # the document the printer sees (debug)
```

Every option, its default and an example of each
value: [`SETTINGS.md`](SETTINGS.md).
See [Migrating from CommandBox](#migrating-from-commandbox) for what has
changed in the settings.

The `<script>` / `<style>` formatting reads the project's Prettier or oxfmt
configuration (`.prettierrc`, `.oxfmtrc.json`, a `package.json` `"prettier"`
key), from the file's directory upward, for the options that change a
program's style (`singleQuote`, `semi`, `trailingComma`, …); the width and
indentation are always cfformat's. `"islands.config": "off"` ignores those
files, and `"islands.js"`, `"islands.css"` or `"islands.json": "off"` prints
that kind of block as written.

## Migrating from CommandBox

cfformat reads CommandBox cfformat's `.cfformat.json` files, but the output
is not the same: the script layout now follows Prettier's rules, some
defaults changed, and settings files are found differently. Expect a diff
the first time you run it over a formatted project.

**The commands.**

| CommandBox                                       | cfformat                                                                             |
| ------------------------------------------------ | ------------------------------------------------------------------------------------ |
| `cfformat run Foo.cfc` / `fmt Foo.cfc`           | `cfformat Foo.cfc` (stdout)                                                          |
| `cfformat run dir/` (overwrites, after a prompt) | `cfformat -w dir` (no prompt; without `-w`, nothing is written)                      |
| `cfformat run … --cfm`                           | `--cfm`                                                                              |
| `cfformat run PATH SETTINGS_FILE`                | `cfformat --config SETTINGS_FILE PATH`                                               |
| `cfformat check dir/` (`--verbose`)              | `cfformat --check dir` (`--diff`)                                                    |
| `cfformat git staged` / `unstaged` / `all`       | `cfformat -w --git staged` / `unstaged` / `all` (no `git add`; see [Git](#git))      |
| `cfformat tag-check dir/`                        | `cfformat --check --cfm dir`: an unbalanced tag prints `path:line: not formatted: …` |
| `cfformat settings show PATH`                    | `cfformat settings PATH`                                                             |
| `cfformat settings info KEY`                     | [`SETTINGS.md`](SETTINGS.md)                                                         |
| `cfformat settings wizard`                       | none; `cfformat settings --schema` gives editors completion for `.cfformat.json`     |
| `cfformat watch`, `stats`, `server`              | none                                                                                 |

**Which settings files apply.** CommandBox merged every layer: its
defaults, then `~/.cfformat.json` (or the `cfformat.settings` config
setting), then the project's file, then the file named on the command line.
cfformat uses the nearest project file _alone_; `~/.cfformat.json` is read
only when there is no project file, and `cfformat.settings` is not read.
A project that relied on keys from someone's home file must now set them
in its own file. `--config` is still merged over whatever was found.

**Keys.** An old file loads with a warning for each removed or renamed key
and each old value, on every run. `cfformat settings --migrate [FILE]`
(default `./.cfformat.json`) rewrites it once to the current keys, so that
it loads silently with the same options:

- **Removed**, now fixed behavior: `binary_operators.padding` and
  `.newline_indent`, `for_loop_semicolons.padding`, every `keywords.*` key,
  `function_*.spacing_to_group`, `function_*.group_to_block_spacing` and
  `function_*.empty_padding`. The output is always `if (x) {`, `} else {`,
  `function f() {`, `a + b` and `()`; the one exception is `(`/`)` padding
  inside keyword groups, which now follows `parentheses.padding` (so
  `for ( … )` pads too when it is on).
- **Merged**: `function_call.padding`, `function_declaration.padding` and
  `function_anonymous.padding` are `parentheses.padding`;
  `metadata.key_value.padding`, `param.key_value.padding` and
  `property.key_value.padding` are `attributes.key_value.padding`; each
  construct's `multiline.comma_dangle`, `multiline.leading_comma` and
  `multiline.leading_comma.padding` are one `multiline.comma`
  (`"trailing"`, `"dangling"`, `"dangling_all"`, `"leading"`,
  `"leading_tight"`): `"dangling"` puts a comma after the last item of
  struct and array literals only, `"dangling_all"` of argument and parameter
  lists too. Old keys that dangle literals and not argument or parameter
  lists become `"dangling"`, keys that dangle all of them `"dangling_all"`;
  any other disagreement warns, and the first key in the file wins.
- **New values**: `strings.convert_nested_quotes` takes `"always"`,
  `"never"` or `"fewer_escapes"`; its old `true` and `false` load as
  `"always"` and `"never"` with a warning, and `--migrate` rewrites them.
- **Replaced**: `*.multiline.min_length`, which measured the whole list, is
  `*.multiline.min_item_length`, which measures the items' average
  width (see below).

**Output that changes.** Each of these moves code on the first run. Where
there is a setting that gives back CommandBox's output, it is named; a file
written by CommandBox's settings wizard names every key, so it already sets
the first three.

- **Lists break only when they do not fit.** CommandBox broke a struct, array,
  argument or parameter list of 4 or more items one per line once the list
  was over 40 columns. Every `*.multiline.element_count` now defaults to 0,
  which turns that threshold off. Setting a count back to 4 restores it,
  but measured by the items' average width (`*.multiline.min_item_length`,
  8 columns; 12 for metadata, `param` and `property` attributes), so `[1,
  2, 3, 4, 5, 6]` stays on one line where CommandBox broke it.
- **Method chains stay on one line while they fit.**
  `method_call.chain.multiline` defaults to 0 (CommandBox: 3, which broke
  every chain of three calls a call per line). At 0 a chain follows
  Prettier's rules; set 3 to get CommandBox's.
- **A quote inside a string no longer forces the other quote.**
  `strings.convert_nested_quotes` defaults to `"fewer_escapes"`: `"it's"`
  stays as it is, where CommandBox printed `'it''s'`. `"always"` is
  CommandBox's.
- **The layout is Prettier's**, with no setting: call arguments hug a trailing
  function or struct, and a broken binary expression, ternary or `for`
  header breaks as Prettier's does.
- **`<cfscript>`, `<script>`, `<style>`, `<cfquery>` and `<cfjava>` bodies
  are indented one level inside their tag**, as Prettier indents `<script>`
  and `<style>`. CommandBox printed these bodies at the tag's own indent;
  `"tags.islands.indent": false` keeps that.
- **A run of blank lines prints as one**, in script as in tags; CommandBox
  kept every blank line between statements.
- **Templates are formatted in full.** `cfformat` formats the CF and HTML tags
  of any template, and the `<script>` and `<style>` blocks in it with
  Prettier's rules. Two new keys choose how tag bodies indent:
  `tags.body.indent` for CF tag bodies, and `tags.islands.indent`
  for the bodies of `<cfscript>`, `<script>`, `<style>`, `<cfquery>` and
  `<cfjava>`; `"islands.js"`, `"islands.css"` and `"islands.json": "off"`
  leave those blocks as written.

## Ignoring a region

Code between a `cfformat-ignore-start` comment and a `cfformat-ignore-end`
comment prints as written, markers included (only its line breaks follow
the `newline` setting, like the rest of the file); everything around it is
formatted. In script, the markers are `//` comments on lines of their
own, or `/* … */` comments:

```cfc
x = {a: 1, b: 2};
// cfformat-ignore-start
matrix = [
    1, 0, 0,
    0, 1, 0,
    0, 0, 1
];
// cfformat-ignore-end
```

In a template, they are CFML comments:

```cfm
<!--- cfformat-ignore-start --->
<cfif   x><td>#a#</td><cfelse><td>#b#</td></cfif>
<!--- cfformat-ignore-end --->
```

A start marker with no end marker after it runs to the end of the file.

`@formatter:off` and `@formatter:on`, the formatter markers of JetBrains
IDEs and Eclipse, work the same way, in the same comment forms, so a
region marked for the IDE is left alone here too:

```cfc
// @formatter:off
matrix = [1, 0,
          0, 1];
// @formatter:on
```

Either end marker closes either start marker, and the markers match in
any case (`// CFFORMAT-IGNORE-START`).

## Arranging members

`cfformat arrange` orders the members of each component and interface,
script or tags:

- **Functions** always: the functions named by `--first` first, in that
  order, whatever their access (`init` by default; `--first before,init`
  keeps `before` above it; `--first=` names none), then by access, widest
  first (`remote`, `public`, `package`, `private`), then by name, ignoring
  case. The access is the `public` / `private` / `package` / `remote`
  modifier or the `access` attribute, and `public` when neither is given;
  `static`, `abstract` and `final` do not change the order.
- **Properties** with `--properties`: by name, ignoring case, within each
  group: a blank line between two properties ends a group, so properties
  grouped by hand (injections, then data) are ordered among themselves and
  the groups stay where they are. Functions cross blank lines.

```text
cfformat arrange --check src     # list the components out of order (exit 1 if any)
cfformat arrange --diff src      # show the moves
cfformat arrange -w src          # write them
cfformat arrange -w --properties src
cfformat arrange -w --first before,init src   # `before`, then `init`, then the rest
```

It takes the bare command's inputs, output modes and exit codes (`-`,
`--stdin-filepath`, `--files-from`, `--git`, `-j`, `--quiet`, `--script` /
`--tags`); it walks `.cfc` and `.cfs` files (a `.cfs` holds no component,
so it is left as it is) and reads no settings.

A member moves with the comments attached to it: those directly above it
with no blank line between, and one at the end of its last line. Anything
else stays where it is, and a member only moves among the members next to
it, so these divide a component into sections that are each ordered on
their own:

- a comment not attached to a member: one followed by a blank line, or a
  banner such as `// ---- PRIVATE ----` (a one-line comment framed by
  rule characters never attaches, even with no blank line below it);
- a statement (pseudo-constructor code, `static { }`), an ignored region,
  `<cfscript>` or any other tag, text;
- a property without `--properties`, and a function among properties;
- a blank line between two properties (a blank line between two functions
  does not);
- a member written on a line it shares with something else;
- a member whose order cannot be read with confidence: an `access` that
  is not a plain word, a modifier and an attribute that disagree, an
  `@access` tag in its doc comment, a property without one plain name;
- any component in which the parse did not understand a region (reported
  as `path:line: not arranged: reason`).

Arranging never formats: it moves the members' text as written, and the
lines between them stay where they were. Run it as its own step, each
in its own commit, and list both commits in `.git-blame-ignore-revs`:

```text
cfformat arrange -w src && git commit -am "arrange members"
cfformat -w src && git commit -am "format"
```

Arrange first, then format: formatting the arranged text realigns what the
move put side by side (aligned properties, for one). `cfformat arrange
--check` in CI keeps the order once it is in place.

**At runtime,** function order does not change what a component does: its
functions exist before its pseudo-constructor runs. It does change the
order of `getMetadata().functions`, so a test suite that depends on the
order of its tests will see them run in another order. Property order is
visible to `getMetadata().properties`, to ORM mappings (column order) and
to serialisation, which is why `--properties` is opt-in.

## Git

`--git` formats the files git reports as changed, in place of PATHS:

```text
cfformat --git staged -w         # the staged .cfc and .cfs files
cfformat --git unstaged --check  # the changed and untracked ones that would change
cfformat --git all -w --cfm      # both, .cfm files included
```

- `staged`: the files added, copied, modified or renamed in the index
  since the last commit.
- `unstaged`: the files modified in the working tree since the index, and
  the untracked files that are not ignored.
- `all`: both.

The selection is the whole repository, from whichever directory it runs
in; a file under the current directory is printed relative to it. It keeps
what a directory walk keeps — `.cfc` and `.cfs` files, and `.cfm` files
with `--cfm` — and skips a file that is gone from the working tree. No file
selected is not an error. Without `-w`, `--check` or `--diff` it reports
like a directory. `git` must be on `PATH`; outside a repository the run
fails with git's message (exit 2).

`-w` formats the working tree's copy and never runs `git add` (CommandBox's
`git staged` did): a pre-commit hook that wants the commit formatted adds
the files it wrote.

```sh
#!/bin/sh
# .git/hooks/pre-commit
cfformat --git staged --check
```

Any other list of files goes to `--files-from`. `git diff --name-only`
prints paths relative to the repository root, so a pipe like this one works
from the root only:

```text
git diff --name-only --diff-filter=ACMR -z main... -- '*.cfc' '*.cfs' | cfformat --files-from - --check
```

## Exit codes

| Code | Meaning                                                                                                                                     |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| 0    | nothing to change, or every change written                                                                                                  |
| 1    | `--check` or `--diff`: a file would change                                                                                                  |
| 2    | a usage error, a file that cannot be read, is not UTF-8 or cannot be written, a failed write to stdout, a settings error, an internal error |

2 wins over 1. A file that fails is reported (`path: message`) and the run
goes on with the others. `cfformat arrange` exits the same way.

## Limitations

- **What cfformat does not understand prints as written.** A region the
  parser cannot read — a statement in a script, a tag body in a template —
  is printed as written (only its line endings follow `newline`),
  everything around it is formatted, and
  stderr says where: `path:line: not formatted: <reason>` (`an unclosed
block`, `a stray closer`, `an unmatched run`, `nesting past the limit`).
  The exit code does not change. Besides genuine errors (an unclosed
  `<cfif>` in a view partial), valid code that does this today:
  - a function parameter named `required` (`required required=true`);
  - `var` inside an expression (`while ((var x = next()) != -1)`, Lucee);
  - an arrow function whose body starts with `return` without braces
    (`(s, i) => return {…}`);
  - quoted named arguments with `:` (`query('a': […], 'b': […])`);
  - an array literal as a script `property` attribute value
    (`property string p preanno=["a", "b"];`);
  - a CF tag inside an unquoted attribute value
    (`<option value="#x#" <cfif y>selected=true</cfif>>`).
- **CF tags inside HTML tags print bare.** A CF tag opened or closed inside
  an HTML tag's attributes (`<a <cfif x>class="on"</cfif> href="…">`,
  `class="<cfif …>active"</cfif>"`) or inside a `style="…"` attribute, or a
  `<cfoutput>` opened inside a `<style>` block and closed after it, is paired
  correctly but printed where it is, without indenting what it wraps.
- **`<script>` / `<style>` blocks are formatted only when they are pure**:
  no CFML inside (a CF tag, or a `#expr#` inside `<cfoutput>`, makes the
  block print as written), and a `type` the formatter knows (JavaScript, a
  module, JSON, CSS). A block oxc cannot parse prints as written with a
  warning (`path:line: islands.js: <message>`).
- **Some blocks keep their text line for line.** Formatting never moves a
  line that starts inside a multi-line string: in a formatted `<script>`,
  the lines inside a template literal or a string continued with `\` keep
  their columns. A block printed as written is otherwise shifted as a whole
  so that no line sits left of where its body belongs (one level inside
  its tag, or at the tag's indent with `"tags.islands.indent": false`), and
  never to the left, except:
  - a `<script>` / `<style>` whose `type` the formatter does not know
    (`text/plain`, `text/x-template`), holds CFML (`type="#kind#"`), or
    whose attributes CFML can emit (`<script #attrs#>`) is data: every line
    of its body is kept, blank lines at its start and end and trailing
    spaces included, and the closing tag follows the last line directly,
    whatever `islands.*` or `--no-islands` say. Only the CF tags and `#…#`
    inside it are formatted, in place.
  - a `<cfquery>`, or a block printed as written (impure, refused, or under
    `"islands.*": "off"`), whose text may hold a string spanning lines — for
    SQL, a string, quoted identifier or dollar quote that spans a line or
    that the formatter cannot see closed — keeps each line's text,
    indentation and trailing spaces, and is not shifted or re-indented.
    Blank lines at the start and end of its body are dropped, as are the
    last line's trailing spaces, and the closing tag starts its own line.

  In both cases, and in a CFML string spanning lines, the line breaks
  themselves follow the `newline` setting like the rest of the file: a
  CRLF source formatted with `"newline": "\n"` (the default on Linux and
  macOS) comes out with LF inside these blocks and strings too.

- **Very large or deeply nested embedded code prints as written.** A
  `<script>` / `<style>` block larger than 256 KB or nested more than 500
  levels deep is not formatted: it prints as written with a warning
  (`path:line: islands.js: nested 2003 levels deep, over the limit of
500`). A formatted block whose template-literal lines cannot be found
  again prints as written with an `internal error` warning rather than
  re-indented. How the nesting is counted, and the stack it needs, are in
  the [developer README](crates/cfformat/README.md#what-prints).
- **A file is formatted in memory, whole.** Peak memory grows with the
  file: about 18 MB for a 175 KB component, and up to about 200 times the
  source for generated, token-dense script (1 GB for 5 MB of statements).
  The island cache holds every distinct `<script>` / `<style>` block of a
  run until it ends. The [developer
  README](crates/cfformat/README.md#what-prints) has the numbers.
- **UTF-8 only.** A file that is not valid UTF-8 is an error and is never
  written; there is no `--encoding` and no transcoding.
- **Globs** are [`globset`](https://docs.rs/globset)'s: `*` (not across
  `/`), `**`, `?`, `[…]` and `{a,b}`. A path that exists is never read as a
  glob.
- **Not on crates.io** (see [Install](#install)).

## Library

The `cfformat` crate is also a library: `cfformat::format_source(src, mode,
&options)` formats a string, and `format_with` takes a context (the file's
path, for island configuration, and a shared `Islands` cache) and returns the
warnings too — `format_source` drops them. Options come from
`Options::from_json` or `options::Discovery`, which resolves `.cfformat.json`
files as the CLI does. The library never reads or writes the source file,
prints, or starts a thread pool; set `islands.config` to `"off"` for a run
that reads no configuration file either. The `cfparse` and `cfdoc` types
the API takes or returns (`Mode`, `Tree`, `Doc`, …) are re-exported from
`cfformat`; to parse a tree or print a document yourself, depend on those
crates too. `cfformat::arrange(src, mode, &ArrangeOptions)` is
`cfformat arrange` over a string. `crates/cfformat/README.md` is the full
API.

Depend on it by path or git, and copy the workspace's `oxc_allocator` line
from `[patch.crates-io]` into your own root `Cargo.toml` — Cargo reads patches
only from the root of the build, and without it the build fails with
`error[E0308]: mismatched types` in `oxc_formatter_css`:

```toml
[patch.crates-io]
oxc_allocator = { git = "https://github.com/oxc-project/oxc", rev = "288d8cc77984b0a3851c58c423ffe9e6edc79f2e" }
```

The workspace's other crates are `cfformat-cli` (the `cfformat` command
line, over this library), `cfparse` (the CFML parse tree), `cfdoc` (a port
of Prettier's document printer), `cfvet` (below) and `cfcli`
(the file walking and reading the two binaries share).

## cfvet

`cfvet` is not a general linter: it is two checks that need a parse tree
to get right, in a second binary on the same one. `missing-var` reports
every unscoped write inside a function — in classic CFML it lands in the
`variables` scope — and `unevaluated-call` every `#…#` that calls a
function outside `<cfoutput>`, which the page shows as written. Every
release carries it next to `cfformat`, under the same asset
names with the `cfvet_` prefix (`cfvet_linux_x86_64`, `cfvet_macos`,
`cfvet_windows_x86_64.exe`, …), or it is built from source:

```text
cargo install --locked --git https://github.com/jcberquist/cfformat cfvet
cargo install --locked --path crates/cfvet      # from a checkout
```

```text
$ cfvet src
src/Report.cfc:12:9: missing-var: `total` is written without `var` or a scope in function `build`; it lands in the variables scope
1 report in 14 files (63 functions checked)
```

A `cfvet-ignore` comment silences its own line and the next. What counts
as a write and a declaration, where a `#…#` is live, the flags and the exit codes are in
[`crates/cfvet/README.md`](crates/cfvet/README.md).

## Licence

MIT: [`LICENSE`](LICENSE). `crates/cfdoc` is a port of Prettier's document
printer and keeps Prettier's MIT notice,
[`crates/cfdoc/LICENSE-PRETTIER`](crates/cfdoc/LICENSE-PRETTIER).
