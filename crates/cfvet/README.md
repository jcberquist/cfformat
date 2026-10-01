# cfvet

Two checks on the `cfparse` tree, in one binary: `missing-var` and
`unevaluated-call`. It is not a general linter; each check is one that
needs the parse tree to get right.

## `missing-var`

An unscoped write inside a function lands in the `variables` scope in
classic CFML: `x = 1` in a function, with no `var x` and no `local.x`,
sets `variables.x`, which outlives the call and is shared by every call
on the same component instance. `cfvet` reports each such write:

```text
models/Report.cfc:12:9: missing-var: `total` is written without `var` or a scope in function `build`; it lands in the variables scope
```

A **write** is an assignment (`x = 1`, `x += 1`, `x.y = 1`, `x[1] = 1`,
`<cfset x = 1>`), an increment or decrement (`x++`, `--x`), a `for (x in
…)` loop variable, a `param` (`param name="x"`, `param string x = 1`,
`<cfparam name="x">`), or the attribute of a tag that names the variable
the tag creates: `<cfquery name="q">`, `http result="r";`,
`cfhttp(result="r")`, `<cfloop index="i">`, `<cfsavecontent
variable="s">` and the rest of the table in
[`src/rules/result_attributes.rs`](src/rules/result_attributes.rs). The
name written is what the target starts with, up to its first `.` or `[`.
A destructuring pattern (Adobe ColdFusion: `[a, b] = x`, `({a, b: c, d =
1, ...r} = x)`, `for ([k, v] in …)`) writes every name it binds — a
rename's target, a default's target (its value is read, not written), a
rest's name, a nested pattern's names — at each name.

A name is **declared** in a function by `var x` (a `for (var …)`, `<cfset
var x = …>` and Lucee's `var x.y = …` included; `var [a, b = 1] = x` and
`var {a, b: c} = x` declare every name the pattern binds), by a write to `local.x` or
`local["x"]` (an attribute value `local.x` too), by a parameter or
`<cfargument name="x">` (an unscoped write to a parameter's name sets the
argument), or as a `catch` variable. Names compare without case.

A declaration covers only the code that runs after it. The engines
differ, checked on 2026-09-24:

| Engine | When a `var` declares | `if (false) { var y = 1; } y = 2;` |
|---|---|---|
| Lucee 7 | when its line runs | `y` lands in `variables` |
| Adobe | for the whole function | `y` is local |

`cfvet` follows Lucee, where a misplaced `var` leaks; on Adobe the same
code is a smell. The static model is **block dominance**: a declaration
covers a write that comes after it in the source and lies inside the block
that holds the declaration, since that is the code that can only run once
the declaration has. A block is a run of code that may be skipped: an `if`
/ `else if` / `else` branch (a brace-less body is its one statement), a
loop body, a `case` / `default`, a `catch` body, and in tags a `<cfif>`
branch (split at `<cfelseif>` / `<cfelse>`), a `<cfcatch>`, `<cfloop>`,
`<cfcase>` and `<cfdefaultcase>` body, and the body of a tag with a
`query` attribute (`<cfoutput query>`). A `var` at the function's top
level covers the rest of the function, nested blocks included. Code that
runs whenever the code around it runs stays in the enclosing block: a
`try` / `<cftry>` body, a `do … while` body, a `finally` / `<cffinally>`
body, a `<cfscript>`, `lock` or `<cfsavecontent>` body, and a `for (var i
…)` header, so `i` is local after the loop. A write that its function
declares, but where no declaration covers it, is reported with a message
of its own:

```text
models/Report.cfc:4:5: missing-var: `y` is written where its `var` may not have run in function `a`; on Lucee the write lands in the variables scope
```

The same message covers a write above the `var` (`x = 1; var x = 2;`). The
fix is the same: declare the name above the write, outside the branch.

A `try` body always starts, so a `var` in it has run by the time its
`catch`, its `finally` or the code after the `try` runs. Both engines keep
`x` local here (checked on 2026-09-24):

```cfml
try {
    var x = 1;
} catch (any e) {
}
x = 2;
```

A `var` in a `catch` covers that `catch` only.

A conditional that runs exactly one of its branches declares a name that
**every** branch declares, from its end on: an `if` with an `else` (any
`else if` between), a `switch` with a `default`, a `<cfif>` with a
`<cfelse>`, a `<cfswitch>` with a `<cfdefaultcase>`. So `if (a) { var x =
1; } else { var x = 2; } x = 3;` is silent. A branch declares a name when a
declaration covers the rest of that branch, a nested conditional's
included. An `if` without an `else`, a `switch` without a `default` and a
loop may run no branch, so they declare nothing after themselves. In a
script `switch`, an empty `case` that falls through to the next is part of
that next one.

What the model cannot see: in `try { var s = init(); } catch (any e) {
s.error = e; }`, if `init()` (or any statement above the `var` in the
`try`) throws, the `var` has not run and the `catch` writes to
`variables`; the rule does not guess which statements can throw, so this
is silent, and `var s = ""` above the `try` closes it. A branch that
always exits (`else { return; }`) still counts as one that does not
declare, and a `var` inside a loop body that ran is local after the loop
at run time; both are reported, and a single `var` above the `if` or the
loop fixes both the report and the Lucee edge.

Parameters and `<cfargument>` names exist on entry, wherever the tag
stands. A **destructuring parameter** (`function f({a, b = 1})`, `({a}) =>
…`, Adobe ColdFusion 2025) declares nothing: the engine binds its names in
the `variables` scope on every call, and `arguments` holds one generated
entry in their place (checked on Adobe ColdFusion 2025 on 2026-09-30), so
each name is reported with a message of its own, and an unscoped write to
the name afterwards is reported as any other:

```text
models/Report.cfc:8:16: missing-var: `rows` is bound by a destructuring parameter of function `build`; the engine puts it in the variables scope
```

No `var` fixes it; the fix is a plain parameter destructured with `var` in
the body. A `catch` variable exists in its catch block only. A closure sees
every name its enclosing function declares, wherever it is declared: it
runs when it is called, after the enclosing function's declarations.

**Not reported:** a write to any scope (`variables.x`, `arguments.x`,
`request.x`, `local.x`, …), `this` or `super`; a target that does not
start with a name (`foo().x = 1`, `"x" = 1`); an attribute value that is
computed (`name="#prefix#Query"`).

**Not checked:** reads; code outside every function (a component's
pseudo-constructor, a `.cfm` template's top level, where writing to
`variables` is the point); a `thread` or `<cfthread>` body, which has its
own scopes; a region the parser could not read (noted on stderr);
`setVariable()`, `evaluate()` and `<cfinclude>`, which write names the
source does not spell. A project on Lucee's modern local mode
(`localMode="modern"`), where an unscoped write in a function is local,
does not need the rule.

## `unevaluated-call`

Outside `<cfoutput>`, a `#…#` in a template is text: the engine sends it to
the browser as written. `cfvet` reports each one that calls a function,
since the call was meant to run and never does:

```text
views/detail.cfm:3:39: unevaluated-call: `#showDetail(…)#` is not inside `<cfoutput>`: it is output as written and the call never runs
```

A `#…#` is **live** in a CF tag's attributes and script (`<cfset x =
"#f()#">`), in the body of `<cfoutput>`, `<cfmail>` and `<cfquery>`, and in
the body of a `<cffunction output="true">` (or `"yes"`), or of a
`<cffunction>` with no `output` in a `<cfcomponent output="true">`.
Anywhere else in a template, HTML text, an HTML attribute value and the
text of a `<script>` or `<style>` body or a `style` / `on…` value included,
it is output as written: inside `<cfsavecontent>`, `<cfif>`, `<cfloop>`,
`<cfxml>` and `<cftimer>` too, and in a template included from inside a
`<cfoutput>`, which does not inherit it (all checked on Lucee 6 and Adobe
2021 on 2026-09-25). A `<cfoutput>` pairs with its `</cfoutput>` whatever
HTML lies between, as the engines pair it: one opened in a `<style>` body
runs past the `</style>`.

A **call** is a name, then any member names (`.b`), index brackets
(`[1]`) and argument lists, at least one argument list, then the closing
`#`: `#f()#`, `#rc.user.getName()#`, `#a.b(1).c[2]()#`. The argument lists
balance their brackets and quotes; a `#…#` longer than 500 bytes is not
read.

**Not reported:** a `#…#` that calls nothing (`#name#`, `#rc.user.name#`,
which is as likely a bug but also an HTML colour or anchor); a comment,
`<!-- -->` included; a component's pseudo-constructor under
`output="true"` (Lucee reads its `#…#`, Adobe refuses to compile it); a
region the parser could not read. A page that shows CFML on purpose
(documentation) silences its example with `cfvet-ignore`.

## Silencing a report

A comment containing `cfvet-ignore` silences the reports on its own line
and on the line after it, in any comment form: `//`, `/* */`, `/** */`,
`<!--- --->`.

```cfml
counter++; // cfvet-ignore: a component-wide counter
<!--- cfvet-ignore --->
<cfset cache = {}>
```

There is no range form and no file-level form.

## Usage

```text
cargo install --locked --path crates/cfvet

cfvet src                  # every .cfc, .cfs and .cfm file under src
cfvet Foo.cfc views/a.cfm  # files, whatever their extension
cat Foo.cfc | cfvet -      # stdin, reported as <stdin>
cfvet -q src               # the reports only
cfvet --git staged         # the staged .cfc, .cfs and .cfm files
```

A directory is walked for `.cfc`, `.cfs` and `.cfm` files (a template can
declare functions), skipping hidden entries and whatever `.gitignore`
(inside a git repository) and `.ignore` exclude: the walk `cfformat` uses
(`crates/cfcli`). The parse mode is chosen as `cfformat` chooses it: a
`.cfs` is script and a `.cfm` a template by name, and a `.cfc`, any other
name and stdin are read from the source; `--script` and `--tags` force
either.

`--git staged|unstaged|all` takes the files git reports as changed in
place of PATHS (it conflicts with them), as `cfformat --git` does
(`cfcli::git_changes`, see `crates/cfformat/README.md`): `staged` the
index against `HEAD`, `unstaged` the working tree against the index plus
the untracked files that are not ignored, `all` both; the whole repository,
files under the current directory named relative to it; kept by the walk's
extensions, `.cfc`, `.cfs` and `.cfm`. Nothing selected is `0 reports in 0
files`, exit 0. Outside a repository, or with `git` not on `PATH`, it
prints `error: --git: <git's message>` (`error: --git: git: No such file or
directory`) and exits 2 without checking anything.

Reports go to stdout, one per line, sorted by path, line and column; a
name written twice on one line is reported once. Notes (a region the
parser recovered from, which is not checked) and the summary (`N reports
in M files (K functions checked)`) go to stderr, and `-q` drops both. An
error goes to stderr as `error: message`, as `cfformat` prints it: a
missing path (`error: nope.cfc: No such file or directory`), a file that
cannot be read or is not UTF-8 (`error: a.cfc: not valid UTF-8`, the rest
still linted), or a failed write to stdout (`error: stdout: Broken pipe`,
which ends the run without the summary).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | no reports |
| 1 | at least one report |
| 2 | a usage error, a missing or unreadable path, a file that is not UTF-8, a failed write to stdout, git failing under `--git` (2 wins over 1) |

## Library

`cfvet::lint_source(src, mode)` lints a string and returns the reports of
both rules,
the recovery notes and the number of functions checked; `Report::render`
and `Note::render` give the output lines. The library never reads a file.
