//! The settings reference: one entry per [`Options`](super::Options)
//! key with its type, default, description and an example source.
//! `tests/reference.rs` renders `SETTINGS.md` from this table, formatting
//! each example with `format_source` once per value, so the reference cannot
//! disagree with the formatter. The descriptions and example sources started
//! from CommandBox cfformat's reference; the formatted examples are generated
//! here, never stored.

/// The type of an option's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// A non-negative integer.
    Integer,
    /// One of these strings.
    Enum(&'static [&'static str]),
    /// `struct.separator`: `:` or `=` with at most one space on each side.
    Separator,
}

/// An example of an option's effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    /// The source, formatted once per shown value.
    pub source: &'static str,
    /// How the source is parsed: script unless the example is tag mode.
    pub mode: ExampleMode,
    /// Other settings the example needs (a JSON object), under the shown value.
    pub settings: &'static str,
    /// The values shown, as JSON literals; empty for every value of a
    /// [`Kind::Bool`], [`Kind::Enum`] or [`Kind::Separator`] domain.
    pub values: &'static [&'static str],
}

/// One option of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionInfo {
    /// The dotted key.
    pub key: &'static str,
    /// Its type.
    pub kind: Kind,
    /// The default, as a JSON literal.
    pub default: &'static str,
    /// What it does.
    pub description: &'static str,
    /// An example source, when one can be shown.
    pub example: Option<Example>,
}

/// How an [`Example::source`] is parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExampleMode {
    /// CFScript (the default).
    #[default]
    Script,
    /// A tag-mode template.
    Tags,
}

/// The `struct.separator` values the reference shows an example for.
pub const SEPARATOR_EXAMPLES: &[&str] = &[r#"": ""#, r#"" = ""#, r#"" : ""#, r#""=""#];

const fn info(
    key: &'static str,
    kind: Kind,
    default: &'static str,
    description: &'static str,
) -> OptionInfo {
    OptionInfo {
        key,
        kind,
        default,
        description,
        example: None,
    }
}

const fn ex(
    key: &'static str,
    kind: Kind,
    default: &'static str,
    description: &'static str,
    source: &'static str,
    settings: &'static str,
    values: &'static [&'static str],
) -> OptionInfo {
    OptionInfo {
        key,
        kind,
        default,
        description,
        example: Some(Example {
            source,
            mode: ExampleMode::Script,
            settings,
            values,
        }),
    }
}

const fn tag_ex(
    key: &'static str,
    kind: Kind,
    default: &'static str,
    description: &'static str,
    source: &'static str,
) -> OptionInfo {
    OptionInfo {
        key,
        kind,
        default,
        description,
        example: Some(Example {
            source,
            mode: ExampleMode::Tags,
            settings: "{}",
            values: &[],
        }),
    }
}

const fn island_ex(
    key: &'static str,
    description: &'static str,
    source: &'static str,
    settings: &'static str,
) -> OptionInfo {
    OptionInfo {
        key,
        kind: Kind::Enum(&["oxc", "off"]),
        default: r#""oxc""#,
        description,
        // Both values: the default, then verbatim.
        example: Some(Example {
            source,
            mode: ExampleMode::Tags,
            settings,
            values: &[],
        }),
    }
}

const QUOTES: Kind = Kind::Enum(&["single", "double", "ignored"]);

const REFERENCE: &[OptionInfo] = &[
    ex(
        "alignment.consecutive.assignments",
        Kind::Bool,
        "false",
        "When true, consecutive variable assignments (`var`, `param`, and assignments to a \
         variable, member or index), named function call arguments, parameter defaults, struct \
         members and attributes are aligned: the `=` or key-value separator of each is padded to \
         the widest left side in the run. A run ends at a blank line or at a line that is not an \
         assignment; comments do not end it. A struct, argument list or attribute list is only \
         aligned when it is printed on multiple lines.",
        "var a = 1;\nvar ab = 2;\nmyStruct = {a: 1, abc: 2};",
        r#"{"struct.multiline.element_count": 2, "struct.multiline.min_item_length": 0}"#,
        &[],
    ),
    ex(
        "alignment.consecutive.params",
        Kind::Bool,
        "false",
        "When true, the attributes of consecutive `param` statements written with attributes \
         (`param name=\"a\" type=\"string\";`) are aligned: every attribute but the last is \
         padded to the widest of its column in the run. A run is a sequence of such statements \
         with the same attribute names in the same order (compared without regard to case); a \
         blank line, any other statement, a statement with other attribute names or a comment \
         inside a statement ends it, and a line comment between two statements does not. A \
         statement that does not fit on one line is printed one attribute per line, unpadded, \
         and sets none of the widths the others align to.",
        "param name=\"a\" type=\"string\";\nparam name=\"abcdefg\" type=\"string\";",
        "{}",
        &[],
    ),
    ex(
        "alignment.consecutive.properties",
        Kind::Bool,
        "false",
        "When true, the attributes of consecutive `property` statements are aligned: every \
         attribute but the last is padded to the widest of its column in the run. A run is a \
         sequence of `property` statements with the same attribute names in the same order \
         (compared without regard to case); a blank line, any other statement, a statement with \
         other attribute names or a comment inside a statement ends it, and a line comment \
         between two statements does not. A statement that does not fit on one line is printed \
         one attribute per line, unpadded, and sets none of the widths the others align to.",
        "property name=\"requestService\" inject=\"coldbox:requestService\";\nproperty \
         name=\"log\" inject=\"logbox:logger:{this}\";",
        "{}",
        &[],
    ),
    ex(
        "alignment.doc_comments",
        Kind::Bool,
        "false",
        "When true, the tags of doc comments are aligned: the text after `@param`-style tags is \
         padded to the widest tag, `@throws Name` descriptions to the widest name, and the \
         blocks are ordered description, params, `@return` (`@returns` is renamed), `@throws`, \
         separated by one empty line. A doc comment without tags is left as it is.",
        "/**\n * @name test\n * @b another param\n * @returns something\n */",
        "{}",
        &[],
    ),
    ex(
        "array.empty_padding",
        Kind::Bool,
        "false",
        "When true, an empty array is padded with a space: `[ ]`; so is an empty array \
         destructuring pattern.",
        "myArray = [];",
        "{}",
        &[],
    ),
    ex(
        "array.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces an array onto multiple lines when it has at least this many elements and their \
         one-line widths average more than `array.multiline.min_item_length` columns; an array \
         destructuring pattern (`[a, b] = x`) too. At 0, the default, nothing is forced, as \
         Prettier has no such rule: it breaks only when it does not fit within `max_columns`.",
        "myArray = [1, 2, 3];",
        r#"{"array.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "array.multiline.min_item_length",
        Kind::Integer,
        "8",
        "How wide an array's elements must be, on average, before `array.multiline.element_count` \
         forces it onto multiple lines: each element's one-line width, commas and padding not \
         counted. A list of short elements stays on one line however many there are. No effect \
         while `array.multiline.element_count` is 0.",
        "myArray = ['alpha', 'bravo', 'charlie', 'delta'];",
        r#"{"array.multiline.element_count": 4}"#,
        &["8", "6"],
    ),
    ex(
        "array.padding",
        Kind::Bool,
        "false",
        "When true, non-empty arrays are padded with spaces, array destructuring patterns \
         (`[ a, b ] = x`) included. An empty array follows `array.empty_padding`.",
        "myArray = [1,2];",
        "{}",
        &[],
    ),
    ex(
        "attributes.key_value.padding",
        Kind::Bool,
        "false",
        "Whether to pad the key value separator of attributes in script: component and function \
         metadata, `property`, `param` and tag statements such as `http url=\"x\";` (formerly \
         `metadata.key_value.padding`, `param.key_value.padding` and \
         `property.key_value.padding`). A script tag call (`cfhttp(url = \"x\")`) is always \
         padded, like named arguments. The attributes of tags in tag mode are never padded.",
        "component extends=\"base.component\" output=false {\n    property name=\"test\";\n}",
        "{}",
        &[],
    ),
    ex(
        "brackets.padding",
        Kind::Bool,
        "false",
        "When true, non-empty index brackets are padded with spaces while they print on one \
         line (`a[ 'key' ]`, `foo()[ 1 ]`). An array literal follows `array.padding`; empty \
         brackets are always `[]`.",
        "a['mykey'][1]=7;\nx.y[i+1].z();",
        "{}",
        &[],
    ),
    ex(
        "comment.asterisks",
        Kind::Enum(&["align", "indent", "ignored"]),
        r#""align""#,
        "When enabled, if every line after the first of a block comment starts with a `*`, they \
         will be aligned. Setting this to \"ignored\" means no alignment will be done.",
        "{\n/**\n      * a comment\n    */\n}",
        "{}",
        &[],
    ),
    ex(
        "function_anonymous.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces an anonymous function's parameter list onto multiple lines when it has at least \
         this many parameters and their one-line widths average more than \
         `function_anonymous.multiline.min_item_length` columns. At 0, the default, nothing is \
         forced, as Prettier has no such rule: it breaks only when it does not fit within \
         `max_columns`.",
        "f = function(a, b, c) {};",
        r#"{"function_anonymous.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "function_anonymous.multiline.min_item_length",
        Kind::Integer,
        "8",
        "How wide an anonymous function's parameter list's parameters must be, on average, before \
         `function_anonymous.multiline.element_count` forces it onto multiple lines: each \
         parameter's one-line width, commas and padding not counted. A list of short parameters \
         stays on one line however many there are. No effect while \
         `function_anonymous.multiline.element_count` is 0.",
        "f = function(a, b, c, d) {};",
        r#"{"function_anonymous.multiline.element_count": 4}"#,
        &["8", "0"],
    ),
    ex(
        "function_call.casing.builtin",
        Kind::Enum(&["cfdocs", "pascal", "ignored"]),
        r#""cfdocs""#,
        "Formats builtin function call casing. The default is to match cfdocs.org data. An \
         alternative is to always capitalize the first letter (pascal). Set this setting to \
         \"ignored\" to leave casing as is.",
        "ARRAYAPPEND(myarray, 1);",
        "{}",
        &[],
    ),
    ex(
        "function_call.casing.userdefined",
        Kind::Enum(&["ignored", "camel", "pascal"]),
        r#""ignored""#,
        "Formats user defined function call casing. The default is to leave as is (this is set \
         to \"ignored\"). Alternatives are to always capitalize the first letter (pascal), or \
         always lower case it (camel).",
        "myFunc();\nOtherFunc();",
        "{}",
        &[],
    ),
    ex(
        "function_call.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces a function call's argument list onto multiple lines when it has at least this many \
         arguments and their one-line widths average more than \
         `function_call.multiline.min_item_length` columns. At 0, the default, nothing is forced, \
         as Prettier has no such rule: it breaks only when it does not fit within `max_columns`.",
        "myFunc(a, b, c);",
        r#"{"function_call.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "function_call.multiline.min_item_length",
        Kind::Integer,
        "8",
        "How wide a function call's argument list's arguments must be, on average, before \
         `function_call.multiline.element_count` forces it onto multiple lines: each argument's \
         one-line width, commas and padding not counted. A list of short arguments stays on one \
         line however many there are. No effect while `function_call.multiline.element_count` is \
         0.",
        "myFunc(firstName, lastName, emailAddress, phone);",
        r#"{"function_call.multiline.element_count": 4}"#,
        &["8", "10"],
    ),
    ex(
        "function_declaration.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces a function declaration's parameter list onto multiple lines when it has at least \
         this many parameters and their one-line widths average more than \
         `function_declaration.multiline.min_item_length` columns. At 0, the default, nothing is \
         forced, as Prettier has no such rule: it breaks only when it does not fit within \
         `max_columns`.",
        "function example(a, b, c) {}",
        r#"{"function_declaration.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "function_declaration.multiline.min_item_length",
        Kind::Integer,
        "8",
        "How wide a function declaration's parameter list's parameters must be, on average, before \
         `function_declaration.multiline.element_count` forces it onto multiple lines: each \
         parameter's one-line width, commas and padding not counted. A list of short parameters \
         stays on one line however many there are. No effect while \
         `function_declaration.multiline.element_count` is 0.",
        "function example(required string a, string b, numeric c, d) {}",
        r#"{"function_declaration.multiline.element_count": 4}"#,
        &["8", "12"],
    ),
    ex(
        "indent_size",
        Kind::Integer,
        "4",
        "Each indent level or tab is equivalent to this many spaces.",
        "do {myFunc();}",
        "{}",
        &["4", "2"],
    ),
    info(
        "islands.config",
        Kind::Enum(&["auto", "off"]),
        r#""auto""#,
        "Whether \"oxc\" reads the project's formatter configuration: \"auto\" looks for \
         `.oxfmtrc.json` / `.oxfmtrc.jsonc`, then prettier's `.prettierrc` family and the \
         `\"prettier\"` key of `package.json`, from the file's directory upward (the first \
         directory holding one wins), and applies the options oxc understands (`singleQuote`, \
         `jsxSingleQuote`, `semi`, `trailingComma`, `arrowParens`, `quoteProps`, \
         `bracketSpacing`, `bracketSameLine`, `singleAttributePerLine`, `objectWrap`, \
         `experimentalOperatorPosition`, and `overrides` by file pattern: `*`, `**`, `?`, \
         `[…]` and `{a,b}`; a micromatch extglob such as `*.@(js|mjs)` matches nothing and \
         warns). `printWidth`, \
         `tabWidth` and `useTabs` are not read: `max_columns`, `indent_size` and `tab_indent` \
         apply. Plugins are not loaded. A YAML, JSON5, TOML or JavaScript configuration is not \
         read and warns once. \"off\" uses oxc's defaults, which are prettier's.",
    ),
    island_ex(
        "islands.css",
        "How pure `<style>` islands are formatted in tag mode (`.css`; see `islands.js` and \
         `islands.config`). A body with a line ending in `\\` (a CSS string continued over a \
         line) prints as it is, byte for byte.",
        "<div>\n<style>\n.a { color: red; }\n  .b { color: blue; }\n</style>\n</div>\n",
        "{}",
    ),
    island_ex(
        "islands.js",
        "How pure JavaScript `<script>` islands are formatted in tag mode: no `type`, a \
         JavaScript MIME type (`.js`) or `module` (`.mjs`). An island is pure when it holds no \
         CFML tag, `#expr#` or tag comment. \"oxc\", the default, formats it in process with \
         the oxc formatter, which prints what prettier prints with its default options (double \
         quotes, semicolons, trailing commas) or with the options of the project's \
         `.prettierrc` (see `islands.config`): nothing needs to be installed. The island's text \
         is handed over as written (blank lines before and after dropped), the indentation \
         comes from `tab_indent` and `indent_size`, the width is `max_columns` less the \
         island's indentation, at least 40, and the output is printed at the tag's indent — \
         except a line inside a template literal, a continued string or a block comment oxc \
         prints raw, which keeps its source columns and its trailing whitespace: formatting \
         never changes a string. An island that does not parse is a warning and prints as it \
         is. \"off\" prints the island as it is, shifted as a whole so that no line sits left \
         of the tag, or byte for byte, not shifted at all, when it holds a backtick or a line \
         ending in `\\`; an impure island and one inside a code fence always print that way. \
         A `type` holding CFML (`type=\"#kind#\"`) names no language: the body is text.",
        "<div>\n<script>\nvar a = 1;\nif (a) {\n  b();\n}\n</script>\n</div>\n",
        "{}",
    ),
    island_ex(
        "islands.json",
        "How pure JSON `<script>` islands are formatted in tag mode: `application/json`, \
         `application/ld+json`, `importmap` and `speculationrules` (`.json`; see `islands.js` \
         and `islands.config`).",
        "<script type=\"application/json\">\n{\"a\": 1}\n</script>\n",
        "{}",
    ),
    ex(
        "max_columns",
        Kind::Integer,
        "120",
        "The line width the formatter tries to stay within: a struct, array, argument or \
         parameter list, member chain or expression that does not fit on its line is printed on \
         multiple lines. Strings, comments and other unbreakable text can still make a line \
         longer.",
        "result = someFunction(argumentOne, argumentTwo);",
        "{}",
        &["120", "40"],
    ),
    ex(
        "metadata.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces the metadata attributes of a component or function declaration onto multiple lines \
         when it has at least this many attributes and their one-line widths average more than \
         `metadata.multiline.min_item_length` columns. At 0, the default, nothing is forced, as \
         Prettier has no such rule: they break only when they do not fit within `max_columns`.",
        "function example() output=false access=\"public\" {}",
        r#"{"metadata.multiline.min_item_length": 0}"#,
        &["0", "2"],
    ),
    ex(
        "metadata.multiline.min_item_length",
        Kind::Integer,
        "12",
        "How wide the metadata attributes of a component or function declaration must be, on \
         average, before `metadata.multiline.element_count` forces them onto multiple lines: each \
         attribute's one-line width, alignment padding not counted. A list of short attributes \
         stays on one line however many there are. No effect while \
         `metadata.multiline.element_count` is 0.",
        "function example() access=\"public\" output=false returnformat=\"json\" hint=\"An example\" {}",
        r#"{"metadata.multiline.element_count": 4}"#,
        &["12", "16"],
    ),
    ex(
        "method_call.chain.multiline",
        Kind::Integer,
        "0",
        "When above 0, a method call chain with at least this many method calls always prints \
         one call per line. At 0 the chain follows Prettier's rules: it stays on one line while \
         it fits (its last call's arguments may still break), and breaks one call per line when \
         it does not, or when it has more than two calls and one of them takes an argument \
         that is not simple (a function, an operator expression, a deeply nested call).",
        "result = query.select('a').from('b').where('c');",
        "{}",
        &["0", "3"],
    ),
    ex(
        "multiline.comma",
        Kind::Enum(&[
            "trailing",
            "dangling",
            "dangling_all",
            "leading",
            "leading_tight",
        ]),
        r#""trailing""#,
        "Where the commas go when a struct, array, argument list, parameter list or script-tag \
         attribute list prints one item per line: `\"trailing\"` after every item but the last; \
         `\"dangling\"` after the last item too in struct literals (`{…}` and ordered \
         `[a: 1]`) and array literals, while argument lists (calls, `new`, script-tag calls such \
         as `cfhttp(url = \"x\")`) and parameter lists (named functions, anonymous functions, \
         arrows) print `\"trailing\"`; `\"dangling_all\"` after the last item of every one of \
         those lists; `\"leading\"` before every item but the first (`, b`, the first item \
         spaced by two so the items align); `\"leading_tight\"` the same without the space \
         (`,b`). A list on one line is `a, b` under every value. A destructuring pattern takes \
         the style of the literal it resembles; its dangling comma is ColdFusion 2025 syntax, and \
         none is written after a rest item (`...r`). Engines differ on a comma after \
         the last item: Lucee 6 accepts one in a struct, array or parameter list and rejects one \
         in an argument list, and Adobe ColdFusion releases before 2025 reject it everywhere. \
         Replaces the per-construct `*.multiline.comma_dangle`, `*.multiline.leading_comma` and \
         `*.multiline.leading_comma.padding` keys: each construct's keys resolve to one style \
         (leading commas never dangle), and the styles merge silently when one value gives \
         them: all trailing is `\"trailing\"`; struct and array literals dangling with \
         argument and parameter lists trailing is `\"dangling\"`; all dangling is \
         `\"dangling_all\"` (`\"dangling\"` when only literals set a key); all one leading \
         style is that style. Otherwise the first construct in the file decides, with a warning.",
        "myArray = [1,2,3,4];\nmyFunction(1,2,3,4);",
        r#"{"array.multiline.element_count": 4, "array.multiline.min_item_length": 0,
            "function_call.multiline.element_count": 4, "function_call.multiline.min_item_length": 0}"#,
        &[],
    ),
    info(
        "newline",
        Kind::Enum(&["os", "\n", "\r\n", "auto"]),
        r#""os""#,
        "The new line character(s) to use. The default is \"os\", which uses \\r\\n on Windows, \
         and \\n otherwise; \"auto\" uses the line ending found in the source.",
    ),
    ex(
        "param.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces the attribute list of a `param` statement onto multiple lines when it has at least \
         this many attributes and their one-line widths average more than \
         `param.multiline.min_item_length` columns. At 0, the default, nothing is forced, as \
         Prettier has no such rule: it breaks only when it does not fit within `max_columns`.",
        "param name=\"a\" type=\"string\" default=\"\";",
        r#"{"param.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "param.multiline.min_item_length",
        Kind::Integer,
        "12",
        "How wide the attributes of a `param` statement must be, on average, before \
         `param.multiline.element_count` forces it onto multiple lines: each attribute's one-line \
         width, alignment padding not counted. A list of short attributes stays on one line \
         however many there are. No effect while `param.multiline.element_count` is 0.",
        "param name=\"abc\" type=\"string\" default=\"\" required=true;",
        r#"{"param.multiline.element_count": 4}"#,
        &["12", "10"],
    ),
    ex(
        "parentheses.padding",
        Kind::Bool,
        "false",
        "Whether to pad the contents of non-empty parentheses with spaces: groups, keyword \
         groups (`for` headers included), function call arguments and function parameters \
         (formerly also `function_call.padding`, `function_declaration.padding` and \
         `function_anonymous.padding`). Empty parentheses are always `()`, and so is \
         `for (;;)`.",
        "a=(1+2);\nif(a){myFunc(1,2);}\nfor(i=1;i<=a;i++){}\nfunction example(a,b) {}",
        "{}",
        &[],
    ),
    ex(
        "property.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces the attribute list of a `property` statement onto multiple lines when it has at \
         least this many attributes and their one-line widths average more than \
         `property.multiline.min_item_length` columns. At 0, the default, nothing is forced, as \
         Prettier has no such rule: it breaks only when it does not fit within `max_columns`.",
        "property name=\"a\" type=\"string\" default=\"\";",
        r#"{"property.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "property.multiline.min_item_length",
        Kind::Integer,
        "12",
        "How wide the attributes of a `property` statement must be, on average, before \
         `property.multiline.element_count` forces it onto multiple lines: each attribute's \
         one-line width, alignment padding not counted. A list of short attributes stays on one \
         line however many there are. No effect while `property.multiline.element_count` is 0.",
        "property name=\"abc\" type=\"string\" default=\"\" inject=\"x\";",
        r#"{"property.multiline.element_count": 4}"#,
        &["12", "10"],
    ),
    ex(
        "strings.attributes.quote",
        QUOTES,
        r#""double""#,
        "Whether to use a single or double quote for attribute values: those of CF tags, in tag \
         mode and in script, and of component and function metadata, `property` and `param`. \
         If set to \"ignored\", leaves attribute value quotes as they are found. The values of \
         HTML tag attributes are never re-quoted.",
        "http url='www.google.com';\nparam name=\"key\";",
        "{}",
        &[],
    ),
    ex(
        "strings.convert_nested_quotes",
        Kind::Enum(&["always", "never", "fewer_escapes"]),
        r#""fewer_escapes""#,
        "The quote of a string whose text contains a quote character (formerly \
         `strings.convertNestedQuotes`). \"always\" uses `strings.quote` \
         (`strings.attributes.quote` for attribute values), doubling it inside the text; \
         \"never\" keeps the quote as written; \"fewer_escapes\" uses the configured quote unless \
         the text contains more of it than of the other quote, as Prettier does. Quotes inside \
         `#…#` are code and do not count. The old values `true` and `false` load as \"always\" \
         and \"never\", with a warning.",
        "a = \"it's\";\nb = 'it''s';\nc = \"say \"\"hi\"\"\";\nd = \"#fn( 'x' )#\";",
        "{}",
        &[],
    ),
    ex(
        "strings.quote",
        QUOTES,
        r#""single""#,
        "Whether to use a single or double quote for strings. If set to \"ignored\", leaves \
         string quotes as they are found.",
        "a=\"One\";\nb='Two';",
        "{}",
        &[],
    ),
    ex(
        "struct.empty_padding",
        Kind::Bool,
        "false",
        "When true, an empty struct is padded with a space: `{ }` (an empty struct \
         destructuring pattern too), and an empty ordered struct prints `[ : ]`.",
        "myStruct = {};\nmyOrdered = [:];",
        "{}",
        &[],
    ),
    ex(
        "struct.multiline.element_count",
        Kind::Integer,
        "0",
        "Forces a struct onto multiple lines when it has at least this many members and their \
         one-line widths average more than `struct.multiline.min_item_length` columns; a struct \
         destructuring pattern (`({a, b} = x)`) too. At 0, the default, nothing is forced, as \
         Prettier has no such rule: it breaks only when it does not fit within `max_columns`.",
        "myStruct = {a: 1, b: 2, c: 3};",
        r#"{"struct.multiline.min_item_length": 0}"#,
        &["0", "3"],
    ),
    ex(
        "struct.multiline.min_item_length",
        Kind::Integer,
        "8",
        "How wide a struct's members must be, on average, before `struct.multiline.element_count` \
         forces it onto multiple lines: each member's one-line width (`key: value`), commas and \
         padding not counted. A list of short members stays on one line however many there are. No \
         effect while `struct.multiline.element_count` is 0.",
        "myStruct = {name: 'Ann', city: 'Paris', role: 'admin', team: 'core'};",
        r#"{"struct.multiline.element_count": 4}"#,
        &["8", "16"],
    ),
    ex(
        "struct.padding",
        Kind::Bool,
        "false",
        "Whether to pad non-empty structs with spaces, struct destructuring patterns (`({ a, b } = \
         x)`) included. An empty struct follows `struct.empty_padding`.",
        "myStruct={a:1,b:2};",
        "{}",
        &[],
    ),
    ex(
        "struct.quote_keys",
        Kind::Bool,
        "false",
        "When true, struct keys are quoted. A destructuring pattern's keys are names and never \
         quoted.",
        "myStruct={a: 1, 'b': 2};",
        "{}",
        &[],
    ),
    ex(
        "struct.separator",
        Kind::Separator,
        r#"": ""#,
        "The key value separator to use in structs - it must contain either a single `:` or `=` \
         with at most one space on each side. A destructuring pattern's rename is always \
         `key: target` (`=` there is a default).",
        "myStruct={a:1,b:2};",
        "{}",
        &[],
    ),
    info(
        "tab_indent",
        Kind::Bool,
        "false",
        "Whether to indent using tab characters or not.",
    ),
    tag_ex(
        "tags.body.indent",
        Kind::Enum(&["always", "cfml"]),
        r#""always""#,
        "With \"always\", every broken tag body is indented one level. With \"cfml\", a paired \
         CF tag whose body starts with HTML — its first node, after whitespace and comments, is \
         not a CF tag — prints that body at the tag's own indent; the tag is judged once, from \
         the part before any `<cfelse>` / `<cfelseif>`, and HTML tag bodies always indent.",
        "<cfif a>\n<div>x</div>\n<cfelse>\n<div>y</div>\n</cfif>\n",
    ),
    tag_ex(
        "tags.lowercase",
        Kind::Bool,
        "true",
        "When true, tag names are lowercased. If false, tag name case is left as is. Attribute \
         names are never changed, and `<!DOCTYPE …>` keeps its case either way.",
        "<CFIF a EQ b>\n<DIV CLASS=\"x\"></DIV>\n</CFIF>\n",
    ),
];

/// Every option, sorted by key.
pub fn reference() -> &'static [OptionInfo] {
    REFERENCE
}
