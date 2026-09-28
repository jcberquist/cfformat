# Dropped and edited fixture cases

The port removed some CommandBox options whose behaviour is now fixed. A settings case whose
expectation depends on a removed option being set to something other than the
behaviour the formatter now fixes was deleted from `settings.json` and
`formatted.txt`. Indexes are the
0-based positions in the original CommandBox `settings.json` (ba56074).

Of 115 fixtures and 173 settings cases, 17 cases were dropped (2 fixtures
entirely), leaving 113 fixtures and 156 cases. Later changes restored 10 of them and
the 2 removed fixtures (below), so 5 cases stay dropped. Cases that set
a removed key to its fixed behaviour (`binary_operators.padding: true`,
`wordOperators`' word operators that stay spaced anyway) stay, with the
key removed. Every case is written in the current keys (the last row of
"Edited cases"); the golden test fails on one the migration would change.

| Fixture | Case | Keys | Reason |
|---|---|---|---|
| `binaryOperators` | 1 | `binary_operators.padding: false` | binary operators are always padded |
| `binaryOperatorsMultiline` | 0 | `binary_operators.newline_indent: false` | continuations always indent; the source's newline is not preserved (case 1 stays, with an edited source: see below) |
| `functionSpacingToGroup` | 1 | `function_declaration.spacing_to_group: true` | always `function f(` |
| `functionSpacingToGroup` | 2 | `function_anonymous.spacing_to_group: true` | always `function(` |
| `keywords` | 1 | `keywords.*` spacing keys (Allman, no space before groups) | always `if (x) {` / `} else {` |

## Restored options

A later change brought back `brackets.padding`,
`struct.empty_padding`, `array.empty_padding` and leading commas
(`multiline.comma`: `"leading"` / `"leading_tight"`), so these cases are back
with CommandBox's expectations; the restored leading-comma cases' settings
name `multiline.comma` instead of the per-construct keys (`leading_comma:
true` → `"leading"`, with `.padding: false` → `"leading_tight"`), which is
what the migration makes of them.

| Fixture | Case | Keys (as restored) |
|---|---|---|
| `alignFunctionParams` | 1, 2 | `multiline.comma: "leading"` / `"leading_tight"` |
| `arrayEmpty` | 0 | `array.empty_padding: true` |
| `arrayMultiline` | 1, 2 | `multiline.comma: "leading"` / `"leading_tight"` |
| `brackets` | 1 | `brackets.padding: true` |
| `functionCall` | 2 | `multiline.comma: "leading_tight"` (and see "Edited cases") |
| `functionCallCommentsLeadingComma` | 1 | `multiline.comma: "leading"` (and see "Edited cases") |
| `structEmpty` | 0 | `struct.empty_padding: true` |
| `structMultiline` | 1 | `multiline.comma: "leading_tight"` |

## Restored with the alignment options

`alignment.consecutive.params` and `alignment.consecutive.properties` are
options again because users relied on them, so the two
fixtures are back byte for byte from CommandBox (`source.cfc`,
`settings.json`, `formatted.txt`), with no edit to the expectation.
`alignAttributeRuns` pins the rest of the rule.

| Fixture | Case | Keys |
|---|---|---|
| `alignParamAttributes` | 0 | `alignment.consecutive.params: true` |
| `alignPropertyAttributes` | 0 | `alignment.consecutive.properties: true` |

## Edited cases

Cases kept with an edit to their source or settings, because the expectation
depends on something the formatter no longer does.

| Fixture | Case | Edit | Reason |
|---|---|---|---|
| `binaryOperatorsMultiline` | 1 (all) | `;` after each of the three `( … )` groups in `source.cfc`; expectation regenerated | without semicolons the source is one call chain `(…)(…)(…)` in every CFML parser; a callee/argument newline is never preserved, so the fixture now pins the binary layout of three statements |
| `functionCall` | 1 | `parentheses.padding: true` removed from `settings.json` | CommandBox's `parentheses.padding` padded only `( … )` groups (the source has none) while calls followed `function_call.padding` (off); the two keys are merged, so the case would contradict case 0 |
| `methodCallComments` | 0, third statement | expectation regenerated: `a.test() // comment` newline `|| b` → `a.test() || // comment` newline `b` | the comment sits inside the `Binary`, between an operand and its operator, not in the chain; a comment there prints after the operator (as `binaryOperatorsMultiline` pins); the first two statements are unchanged |
| `tagWordOperators` | 0 | expectation regenerated: `<cfif a` newline `AND b></cfif>` → `<cfif a AND b></cfif>` | the source newline sits inside the `Binary` of the `<cfif>` condition; source whitespace is trivia and the binary printer joins operands, exactly as `binaryOperatorsMultiline` pinned for script mode |
| `cfqueryIndent` | 0 | expectation regenerated: `SELECT` 8 → 12, `FROM` 4 → 8, `WHERE` 0 → 4 | an island keeps its shape: the least-indented line after the first (`WHERE`, column 0) is shifted to the tag's indent (4) and every other line moves by the same amount, replacing an earlier per-line clamp, which flattened relative indentation; the island printer and the script hand-off dedent make the same measurement (the least leading indent of the non-empty lines) |
| `functionCall` | 2 | `parentheses.padding: true` removed from `settings.json` | as case 1: the case expects unpadded calls, which CommandBox printed because `parentheses.padding` did not reach calls |
| `structOrdered` | 0 | expectation edited: `var t = [:];` → `var t = [ : ];` | the case sets `struct.empty_padding: true`; `[:]` is the empty state of an ordered struct and pads like `{ }`. CommandBox's `Structs.cfc` printed the `:` as the struct's one item, so `[:]` never reached the empty branch that pads (its lines 64–66); the one restored case where the fixture does not win |
| `arrayTrailingComma` | 0, 1, 2 | `settings.json`: `array.multiline.comma_dangle: true` / `false` → `multiline.comma: "dangling"` / `"trailing"` | mechanical: the per-construct key is replaced by `multiline.comma`; no expectation moves (the migration itself is pinned by `options` unit tests) |
| `functionCallCommentsLeadingComma` | 1, statements 3–4 | expectation edited: `, from` ⏎ `, // comment a` ⏎ `// comment b` ⏎ `body` → `, from` ⏎ `// comment a` ⏎ `// comment b` ⏎ `, body`, and `, from // comment a` ⏎ `, // comment b` ⏎ `body` → `, from // comment a` ⏎ `// comment b` ⏎ `, body` | an item's own-line leading comments print before its leading comma. CommandBox's layout is not idempotent here: re-parsed, a comment that follows a `,` on its line belongs to the previous item (the after-comma rule), so the second pass moved it (statement 3 → statement 4's shape → statement 2's). The edited expectations are statements 1–2's layout, which is stable |
| `blocks` | 0 | expectation edited: the two blank lines before `b = 2;` → one | a run of blank lines prints as one; CommandBox kept every blank line between statements |
| `componentAttrs`, `componentAttrsShort`, `componentAbstract`, `alignDocCommentsIgnore`, `luceeInlineComponent` | 0–3, 0–1, 0, 0, 0 | expectation edited: an empty component body `{` + two blank lines + `}` → one blank line | never more than one blank line in a row; CommandBox's `CFComponent.cfc` padded an empty body twice |
| `groupWithComment` | 0 | expectation regenerated: `(` ⏎ `a && b // test` ⏎ `)` → `(` ⏎ `a &&` ⏎ `b // test` ⏎ `)` | a binary that is a group's only content breaks with the group (Prettier's `isInsideParenthesis`); the line comment breaks the group, so the chain breaks too, as Prettier prints `if (a && b // test` ⏎ `)` |
| `methodCalls` | 0 | expectation regenerated: each three-method chain broken a method per line → one line (`testObj.methodOne().methodTwo().methodThree();`) | chains follow Prettier's member-chain rules and `method_call.chain.multiline` defaults to 0: a chain with simple arguments that fits stays on one line |
| `methodCallsLineLength` | 0 | expectation regenerated: `getInstance('entityService:Task').list().reduce(…)` on one line → a call per line | three calls, one with a function argument, break a call per line (Prettier's member chain); CommandBox's one-line form ran past `max_columns` |
| `methodWithTagName` | 0 | expectation regenerated: `query` ⏎ `.where(…)` ⏎ `.where(` … → `query.where(…).where(` ⏎ … ⏎ `);` | the last call's arguments (forced to break by the `function_call` threshold) break under a one-line chain, Prettier's `oneLine` state |
| `structMultiline`, `functionCall`, `functionMetadata`, `componentAttrsShort`, `scriptPropertyShort` | every case that sets a `*.multiline.min_length` | `settings.json`: the construct's `*.multiline.element_count: 4` added | every `element_count` defaults to 0 now, which turns its threshold off; these cases test the threshold with CommandBox's default count of 4, so they name it; no expectation moves |
| `alignAttributeRuns`, `alignPropertyAttributeAssignments` | all | `settings.json`: `property.multiline.element_count: 4` added | the cases align the attributes of `property` statements that the old default threshold broke one attribute per line; naming the count keeps what they test; no expectation moves |
| `functionAnon` | 0 | `settings.json`: `function_declaration.multiline.element_count` 0 → 1 | the case forces every declaration's parameters to break (0 / 0 under the old reading, "at least 0 parameters"); 0 now means off, and 1 / 0 is the same rule; no expectation moves |
| `luceeInlineComponent`, `methodWithTagName`, `scriptProperty` | 0; 0; 0, 2 | expectation regenerated: a `new component` header, a call's four arguments, a `property`'s five attributes broken one per line → one line | the default thresholds (4 items wider than 40 columns) forced those breaks; at the new default of 0 a list breaks only when it does not fit, as Prettier's |
| `structMultiline`, `functionCall`, `arrayMultiline`, `commaStyles`, `functionCallStructOrArray`, `arrayTrailingComma`, `alignAssignments`, `functionAnon`, `componentAttrsShort`, `scriptPropertyShort` | every case that sets a `*.multiline.min_length` of 0 or 1 | `settings.json`: the key becomes `*.multiline.min_item_length: 0` | `min_length` is removed (the threshold measures the items' average width); a width of 0 or 1 meant "any list", which is an average above 0; no expectation moves |
| `arrayMultilineShort`, `structMultilineShort`, `functionMetadata` | 0; 0; 1 | `settings.json`: `*.multiline.min_length: 40` / `40` / `30` removed; `functionMetadata` case 1 sets `metadata.multiline.min_item_length: 8` | the two short-list cases hold at the default average (8); `functionMetadata`'s function attributes (`abcd="efgh" ijklmnop="qrst" uvw=xyz bcde`, 37 columns) broke past a width of 30 and average 9.25; no expectation moves |
| `alignAttributeRuns` | all | `settings.json`: `property.multiline.min_item_length: 10` added | the cases need the `createdDate` property broken and the `id` one flat, as 4 / 40 had them; they average 11.5 and 9.25, so 10 keeps both; no expectation moves |
| `strings`, `stringsNestedQuotes` | 0, 1; 0 | `settings.json`: `strings.convert_nested_quotes: "always"` added | the cases test CommandBox's conversion (`"te""s''t"` → `'te"s''''t'`), its default until `"fewer_escapes"` became the default; no expectation moves |
| `luceeInlineComponent` | 0 | expectation regenerated: `javaSettings="{""maven"":[""a:b:1""]}"` → `javaSettings='{"maven":["a:b:1"]}'` | the text holds six `"` and no `'`, so the fewer-escapes default takes the other quote, as Prettier does |
| `binaryOperators`, `binaryOperatorsMultiline`, `wordOperators`, `componentAttrs`, `scriptParam`, `scriptProperty`, `functionCall`, `alignFunctionParams`, `functionCallCommentsLeadingComma`, `stringsNestedQuotes` | 0; 0; 0; 1; 1; 2; 0; 0; 0; 1 | `settings.json` rewritten in the current keys, each case exactly what `cfformat settings --migrate` makes of it: `binary_operators.padding` / `.newline_indent` removed (the case becomes `{}`); `metadata.` / `param.` / `property.key_value.padding` → `attributes.key_value.padding`; `function_call.padding` removed beside `parentheses.padding`; `*.multiline.leading_comma: false` → `multiline.comma: "trailing"`; `strings.convertNestedQuotes` → `strings.convert_nested_quotes`, its `false` now `"never"` | the cases no longer pass through the migration on every run (the migration is pinned by the `options` unit tests and `tests/cli.rs`); no expectation moves |
