//! Printer cases beyond the golden fixtures: layouts the fixtures do not
//! pin down (or pin down on a shape the parser reads differently).

mod common;

use cfformat::options::IslandConfigMode;
use cfformat::print::MAX_DEPTH;
use cfformat::{format_with, FormatCtx, Islands, Options};
use cfparse::Mode;

/// [`cfformat::format_source`] with `islands.config` `"off"`: no
/// `.prettierrc` above the checkout (or in the home directory) moves an
/// island case.
fn format_source(src: &str, mode: Mode, opts: &Options) -> String {
    let opts = Options {
        islands_config: IslandConfigMode::Off,
        ..opts.clone()
    };
    cfformat::format_source(src, mode, &opts)
}

fn fmt_with(src: &str, settings: &str) -> String {
    let (opts, _) = Options::from_json(settings).unwrap();
    let out = format_source(src, Mode::Script, &opts);
    let again = format_source(&out, Mode::Script, &opts);
    assert_eq!(again, out, "not idempotent for {src:?}");
    out
}

fn fmt(src: &str) -> String {
    fmt_with(src, r#"{"newline": "\n"}"#)
}

#[test]
fn binary_breaks_after_the_operator_and_indents_once() {
    // A binary that is a group's only content is not indented again
    // (binaryOperatorsMultiline pins the three shapes).
    assert_eq!(
        fmt("(\na // test comment\n    &&\nb\n);"),
        "(\n    a && // test comment\n    b\n);\n"
    );
    assert_eq!(
        fmt_with(
            "if (aaaa && bbbb) {}",
            r#"{"newline": "\n", "max_columns": 12}"#
        ),
        "if (\n    aaaa &&\n    bbbb\n) {\n}\n"
    );
    assert_eq!(
        fmt_with(
            "x = aaaa + bbbb * cccc;",
            r#"{"newline": "\n", "max_columns": 16}"#
        ),
        "x =\n    aaaa +\n    bbbb * cccc;\n"
    );
    // The first operand stays outside the indent: its own lines (a broken
    // argument list, chain segments) are not indented twice. A single binary
    // groups its operator and right operand, so a
    // short right part stays on the first operand's last line.
    assert_eq!(
        fmt_with(
            "someFunction(argumentOne, argumentTwo, argumentThree) && anotherCondition;",
            r#"{"newline": "\n", "max_columns": 40}"#
        ),
        "someFunction(\n    argumentOne,\n    argumentTwo,\n    argumentThree\n) && anotherCondition;\n"
    );
    assert_eq!(
        fmt_with(
            "someFunction(argumentOne, argumentTwo, argumentThree) && anotherConditionThatIsLongerThanForty;",
            r#"{"newline": "\n", "max_columns": 40}"#
        ),
        "someFunction(\n    argumentOne,\n    argumentTwo,\n    argumentThree\n) &&\n    anotherConditionThatIsLongerThanForty;\n"
    );
    assert_eq!(
        fmt_with(
            "x = a.b().c().d() && e.f().g().h();",
            r#"{"newline": "\n", "method_call.chain.multiline": 3}"#
        ),
        "x =\n    a\n        .b()\n        .c()\n        .d() &&\n    e\n        .f()\n        .g()\n        .h();\n"
    );
}

#[test]
fn the_output_ends_with_one_newline() {
    // A string left open at the end of the file ends with the file's
    // newline; the output still ends with exactly one, run after run.
    let once = fmt("x = 1;\ny = \"abc\n}\n");
    assert_eq!(once, "x = 1;\ny = \"abc\n}\n");
    assert_eq!(fmt(&once), once);
    assert_eq!(
        fmt_with("x = 1;\r\ny = \"abc\r\n}\r\n\r\n", r#"{"newline": "\r\n"}"#),
        "x = 1;\r\ny = \"abc\r\n}\r\n"
    );
}

#[test]
fn verbatim_text_takes_the_output_newline() {
    // A `cfformat-ignore` region and a Java body keep their text but not
    // the source's line endings: the file never comes out mixed.
    let ignore = "a={x:1};\r\n// cfformat-ignore-start\r\nb={x:1};  \r\nc\rd\r\n// \
                  cfformat-ignore-end\r\n";
    assert_eq!(
        fmt(ignore),
        "a = {x: 1};\n// cfformat-ignore-start\nb={x:1};  \nc\nd\n// cfformat-ignore-end\n"
    );
    assert_eq!(
        fmt_with(
            "a={x:1};\n// cfformat-ignore-start\nb={x:1};\n// cfformat-ignore-end\n",
            r#"{"newline": "\r\n"}"#
        ),
        "a = {x: 1};\r\n// cfformat-ignore-start\r\nb={x:1};\r\n// cfformat-ignore-end\r\n"
    );
    assert_eq!(
        fmt("function f() type=\"java\" {\r\n  return 1;\r\n}\r\n"),
        "function f() type=\"java\" {\n  return 1;\n}\n"
    );
}

#[test]
fn stray_tag_tokens_in_script_stay_on_their_line() {
    // Invalid script (a component brace left open, then `</cfscript>`): the
    // component is a recovered region, so it prints as written, never as
    // `< / cfscript >`.
    let src = "component {\n    function f() {\n    }\n</cfscript>\n";
    assert_eq!(fmt(src), src);
}

#[test]
fn operators() {
    assert_eq!(fmt("x = not   a eq b;"), "x = not a eq b;\n");
    assert_eq!(fmt("y = ! a && - b ^ 2;"), "y = !a && -b ^ 2;\n");
    assert_eq!(fmt("z = a is  not b;"), "z = a is not b;\n");
    assert_eq!(fmt("w = new   Foo(1);"), "w = new Foo(1);\n");
    assert_eq!(fmt("x = a/* c */+b;"), "x = a /* c */ + b;\n");
    assert_eq!(fmt("x=a?:b;"), "x = a ?: b;\n");
}

#[test]
fn ternary_breaks_before_its_operators() {
    assert_eq!(
        fmt_with(
            "x = cond ? first : second;",
            r#"{"newline": "\n", "max_columns": 20}"#
        ),
        "x = cond\n    ? first\n    : second;\n"
    );
}

#[test]
fn nested_ternaries_break_as_one_chain() {
    // Prettier's `printTernaryOld`: only the outermost ternary is a group; a
    // `:` branch's ternary lines up under the branch, a `?` branch's one
    // indent in from the outer `?`.
    let narrow = r#"{"newline": "\n", "max_columns": 30}"#;
    assert_eq!(
        fmt_with("x = aaaaaa ? bbbbbb : cccccc ? dddddd : eeeeee;", narrow),
        "x = aaaaaa\n    ? bbbbbb\n    : cccccc\n      ? dddddd\n      : eeeeee;\n"
    );
    assert_eq!(
        fmt_with("x = aaaaaa ? bbbbbb ? cccccc : dddddd : eeeeee;", narrow),
        "x = aaaaaa\n    ? bbbbbb\n        ? cccccc\n        : dddddd\n    : eeeeee;\n"
    );
    // Parentheses stay: the chain prints inside them.
    assert_eq!(
        fmt_with("x = aaaaaa ? bbbbbb : (cccccc ? dddddd : eeeeee);", narrow),
        "x = aaaaaa\n    ? bbbbbb\n    : (cccccc\n      ? dddddd\n      : eeeeee);\n"
    );
    // A chain that fits stays on one line.
    assert_eq!(fmt("x = a ? b : c ? d : e;"), "x = a ? b : c ? d : e;\n");
}

#[test]
fn ternary_branches_align_after_their_operator() {
    // Prettier's `align(2)`: a branch's own lines continue from the column
    // after `? ` / `: `, one indent under tabs.
    let narrow = r#"{"newline": "\n", "max_columns": 30}"#;
    assert_eq!(
        fmt_with("x = aaaaaa ? bbbbbb : foo({kkkkkk: 1, llllll: 2});", narrow),
        "x = aaaaaa\n    ? bbbbbb\n    : foo({\n          kkkkkk: 1,\n          llllll: 2\n      });\n"
    );
    assert_eq!(
        fmt_with(
            "x = aaaaaa ? bbbbbb : foo({kkkkkk: 1, llllll: 2});",
            r#"{"newline": "\n", "max_columns": 30, "tab_indent": true}"#
        ),
        "x = aaaaaa\n\t? bbbbbb\n\t: foo({\n\t\t\tkkkkkk: 1,\n\t\t\tllllll: 2\n\t\t});\n"
    );
    // A binary branch or condition does not indent its continuation lines
    // (Prettier's `shouldNotIndent`)…
    assert_eq!(
        fmt_with(
            "x = aaaaaa ? bbbbbbbbb & cccccccccc : d;",
            r#"{"newline": "\n", "max_columns": 24}"#
        ),
        "x = aaaaaa\n    ? bbbbbbbbb &\n      cccccccccc\n    : d;\n"
    );
    assert_eq!(
        fmt_with("x = aaaaaaaaaaaaaa && bbbbbbbbbbbbbbbb ? c : d;", narrow),
        "x =\n    aaaaaaaaaaaaaa &&\n    bbbbbbbbbbbbbbbb\n        ? c\n        : d;\n"
    );
    // …except in a ternary that is a call argument or a `return` value.
    assert_eq!(
        fmt_with("foo(aaaaaaaaaaaaaa && bbbbbbbbbbbbbbbb ? c : d);", narrow),
        "foo(\n    aaaaaaaaaaaaaa &&\n        bbbbbbbbbbbbbbbb\n        ? c\n        : d\n);\n"
    );
    assert_eq!(
        fmt_with("return aaaaaaaaaaaaaa && bbbbbbbbbbbbbbbb ? c : d;", narrow),
        "return aaaaaaaaaaaaaa &&\n    bbbbbbbbbbbbbbbb\n    ? c\n    : d;\n"
    );
}

#[test]
fn keyword_chains() {
    assert_eq!(
        fmt("if (a) x = 1; else if (b) y; else z;"),
        "if (a) x = 1;\nelse if (b) y;\nelse z;\n"
    );
    assert_eq!(
        fmt("if (a) {} // c\nelse {}"),
        "if (a) {\n} // c\nelse {\n}\n"
    );
    assert_eq!(
        fmt("try {}\n// own\ncatch (e) {} finally {}"),
        "try {\n}\n// own\ncatch (e) {\n} finally {\n}\n"
    );
    assert_eq!(fmt("do x++; while (a);"), "do x++;\nwhile (a);\n");
    assert_eq!(fmt("for(;;){}"), "for (;;) {\n}\n");
    assert_eq!(
        fmt("foo: while (true) { break foo; }"),
        "foo: while (true) {\n    break foo;\n}\n"
    );
}

#[test]
fn for_in_collection_breaks_on_its_own() {
    // Prettier's `ForInStatement`: never a break after `in`.
    let narrow = r#"{"newline": "\n", "max_columns": 60}"#;
    assert_eq!(
        fmt_with(
            "for ( var key in ['AAAAAAA', 'BBBBBBBBB', 'CCCCCCCC', 'DDDDDDDDDDDD', 'EEEEEEEEEE'] ) {}",
            narrow
        ),
        "for (var key in [\n    'AAAAAAA',\n    'BBBBBBBBB',\n    'CCCCCCCC',\n    'DDDDDDDDDDDD',\n    'EEEEEEEEEE'\n]) {\n}\n"
    );
    assert_eq!(
        fmt_with(
            "for (key in foo('AAAAAAAAAA', 'BBBBBBBBBBBB', 'CCCCCCCCCCCC', 'DDDDDDDDDDDDDD')) {}",
            narrow
        ),
        "for (key in foo(\n    'AAAAAAAAAA',\n    'BBBBBBBBBBBB',\n    'CCCCCCCCCCCC',\n    'DDDDDDDDDDDDDD'\n)) {\n}\n"
    );
}

#[test]
fn a_list_threshold_is_off_at_zero() {
    // Every `element_count` defaults to 0: a list that fits stays on one
    // line however many items it has, as Prettier's; a count above 0 keeps
    // the threshold.
    let src = "x = ['aaaaaaaaaa', 'bbbbbbbbbb', 'cccccccccc', 'dddddddddd'];";
    assert_eq!(
        fmt(src),
        "x = ['aaaaaaaaaa', 'bbbbbbbbbb', 'cccccccccc', 'dddddddddd'];\n"
    );
    assert_eq!(
        fmt_with(
            src,
            r#"{"newline": "\n", "array.multiline.element_count": 4}"#
        ),
        "x = [\n    'aaaaaaaaaa',\n    'bbbbbbbbbb',\n    'cccccccccc',\n    'dddddddddd'\n];\n"
    );
    assert_eq!(
        fmt("f(aaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd);"),
        "f(aaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd);\n"
    );
}

#[test]
fn a_list_threshold_measures_the_average_item() {
    // With a count set, a list breaks when its items average more than
    // `min_item_length` columns (8 by default): many tiny items stay on one
    // line, and one long item among short ones does not force a break.
    let s = r#"{"newline": "\n", "array.multiline.element_count": 4, "function_call.multiline.element_count": 4}"#;
    assert_eq!(
        fmt_with("x = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];", s),
        "x = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];\n"
    );
    assert_eq!(
        fmt_with("x = ['january', 'february', 'march', 'april'];", s),
        "x = [\n    'january',\n    'february',\n    'march',\n    'april'\n];\n"
    );
    assert_eq!(
        fmt_with("x = replace(arguments.argValue, '|', '\\|', 'all');", s),
        "x = replace(arguments.argValue, '|', '\\|', 'all');\n"
    );
    // Fewer items than the count: never forced.
    assert_eq!(
        fmt_with("x = ['alphabetical', 'bravissimo', 'charlestown'];", s),
        "x = ['alphabetical', 'bravissimo', 'charlestown'];\n"
    );
}

#[test]
fn a_number_index_never_breaks() {
    // Prettier's `printMemberLookup`: every other index breaks inside `[ ]`.
    // (A member access is poorly breakable: it moves after `=` first.)
    let long = "x".repeat(120);
    assert_eq!(
        fmt(&format!("a = {long}[1];")),
        format!("a =\n    {long}[1];\n")
    );
    assert_eq!(
        fmt(&format!("a = {long}[k];")),
        format!("a =\n    {long}[\n        k\n    ];\n")
    );
}

#[test]
fn a_broken_group_breaks_its_binary() {
    // Prettier's `isInsideParenthesis`: the operators' lines are the
    // group's, so a broken `if (` never keeps `a && b` on one line.
    let (a, b) = ("a".repeat(60), "b".repeat(60));
    assert_eq!(
        fmt(&format!("if ({a} && {b}) {{}}")),
        format!("if (\n    {a} &&\n    {b}\n) {{\n}}\n")
    );
    assert_eq!(fmt("if (a && b) {}"), "if (a && b) {\n}\n");
}

#[test]
fn a_parenthesised_ternary_hugs_its_parentheses() {
    // As Prettier prints a parenthesised conditional: no lines of their own
    // for `(` and `)`.
    let (c, t, e) = ("c".repeat(40), "t".repeat(40), "e".repeat(40));
    assert_eq!(
        fmt(&format!("return ({c} ? {t} : {e});")),
        format!("return ({c}\n    ? {t}\n    : {e});\n")
    );
    assert_eq!(
        fmt_with(
            &format!("return ({c} ? {t} : {e});"),
            r#"{"newline": "\n", "parentheses.padding": true}"#
        ),
        format!("return ( {c}\n    ? {t}\n    : {e} );\n")
    );
    assert_eq!(fmt("return (c ? t : e);"), "return (c ? t : e);\n");
}

#[test]
fn a_lone_string_argument_breaks_like_any_other() {
    // Prettier: only a string that spans lines hugs the parentheses
    // (`isTemplateOnItsOwnLine`).
    let long = "s".repeat(120);
    assert_eq!(
        fmt(&format!("foo('{long}');")),
        format!("foo(\n    '{long}'\n);\n")
    );
    assert_eq!(
        fmt(&format!("foo('a\n{long}');")),
        format!("foo('a\n{long}');\n")
    );
}

#[test]
fn a_parenthesised_binary_operand_hugs_its_parentheses() {
    // Prettier prints the parentheses around an operand: `(b ||` ⏎ `c)`,
    // the continuation indented once.
    let (a, b, c) = ("a".repeat(60), "b".repeat(60), "c".repeat(60));
    assert_eq!(
        fmt(&format!("if ({a} && ({b} || {c})) {{}}")),
        format!("if (\n    {a} &&\n    ({b} ||\n        {c})\n) {{\n}}\n")
    );
    // A unary's operand keeps `(` and `)` on lines of their own.
    assert_eq!(
        fmt(&format!("x = !({a} || {b} || {c});")),
        format!("x = !(\n    {a} ||\n    {b} ||\n    {c}\n);\n")
    );
}

#[test]
fn a_ternary_hugs_only_while_its_binary_condition_fits() {
    // A broken binary condition would continue at the indent of `?`.
    // (After `return`, where nothing breaks before the value.)
    let (a, b) = ("a".repeat(30), "b".repeat(30));
    assert_eq!(
        fmt(&format!("return ({a} && {b} ? {a} : {b});")),
        format!("return ({a} && {b}\n    ? {a}\n    : {b});\n")
    );
    // Too long after `return (`: the parentheses take lines of their own.
    let (a, b) = ("a".repeat(55), "b".repeat(55));
    assert_eq!(
        fmt(&format!("return ({a} && {b} ? c : d);")),
        format!("return (\n    {a} && {b}\n        ? c\n        : d\n);\n")
    );
}

#[test]
fn an_assignment_breaks_after_the_operator_as_prettier_does() {
    // A binary, a plain string, a poorly breakable chain or call: the
    // value moves to the next line, a binary not indented again.
    let (a, b) = ("a".repeat(60), "b".repeat(60));
    assert_eq!(
        fmt(&format!("x = {a} && {b};")),
        format!("x =\n    {a} &&\n    {b};\n")
    );
    let long = "s".repeat(120);
    assert_eq!(
        fmt(&format!("x = '{long}';")),
        format!("x =\n    '{long}';\n")
    );
    let name = "n".repeat(70);
    assert_eq!(
        fmt(&format!(
            "{name} = someService.getSomethingByIdentifier(identifierValue);"
        )),
        format!("{name} =\n    someService.getSomethingByIdentifier(identifierValue);\n")
    );
    // Fluid: a call with several arguments keeps its first line.
    assert_eq!(
        fmt(&format!("x = foo({a}, {b});")),
        format!("x = foo(\n    {a},\n    {b}\n);\n")
    );
    // Never after the operator: a multi-line or interpolated string, a
    // number, a boolean.
    assert_eq!(fmt("a = \"one\n  two\";"), "a = 'one\n  two';\n");
    // A struct member: the same layouts; a short key never breaks.
    assert_eq!(
        fmt(&format!("x = {{key: {a} && {b}}};")),
        format!("x = {{\n    key:\n        {a} &&\n        {b}\n}};\n")
    );
    assert_eq!(
        fmt(&format!("x = {{id: foo({a}, {b})}};")),
        format!("x = {{\n    id: foo(\n        {a},\n        {b}\n    )\n}};\n")
    );
}

#[test]
fn a_hugged_arrow_breaks_after_the_arrow() {
    // Prettier's `expandLastArg`: the parameters never break; the body
    // starts the next line and `)` gets a line of its own.
    let (a, b) = ("a".repeat(50), "b".repeat(50));
    assert_eq!(
        fmt(&format!("x = foo({a}, (k) => {b}.find(k));")),
        format!("x = foo({a}, (k) =>\n    {b}.find(k)\n);\n")
    );
    assert_eq!(
        fmt_with(
            &format!("x = foo({a}, (k) => {b}.find(k));"),
            r#"{"newline": "\n", "parentheses.padding": true}"#
        ),
        format!("x = foo( {a}, ( k ) =>\n    {b}.find( k )\n);\n")
    );
    // A struct or array body stays on the arrow's line.
    assert_eq!(
        fmt(&format!("x = foo((k) => ({{a: {a}, b: {b}}}));")),
        format!("x = foo((k) => ({{\n    a: {a},\n    b: {b}\n}}));\n")
    );
}

#[test]
fn an_arrow_hugs_only_a_call_ternary_struct_or_array_body() {
    // Prettier's `couldExpandArg`: any other body puts the arguments one
    // per line, and the arrow breaks after `=>` with its binary unindented.
    let (a, b) = ("a".repeat(60), "b".repeat(60));
    assert_eq!(
        fmt(&format!("x = foo((k) => {a} && {b});")),
        format!("x = foo(\n    (k) =>\n        {a} &&\n        {b}\n);\n")
    );
    assert_eq!(fmt("x = foo((k) => a && b);"), "x = foo((k) => a && b);\n");
}

#[test]
fn an_arrow_whose_parameters_break_is_not_hugged() {
    // Prettier's `ArgExpansionBailout`: four parameters past the
    // `function_anonymous` threshold break, so every argument does.
    let p = ["a", "b", "c", "d"].map(|c| c.repeat(12)).join(", ");
    assert_eq!(
        fmt_with(
            &format!("x = foo(({p}) => k.find(k));"),
            r#"{"newline": "\n", "function_anonymous.multiline.element_count": 4}"#
        ),
        format!(
            "x = foo(\n    (\n        {}\n    ) => k.find(k)\n);\n",
            p.replace(", ", ",\n        ")
        )
    );
}

#[test]
fn a_for_header_breaks_one_clause_per_line() {
    // Prettier's `ForStatement`: a header too wide for its line puts each
    // clause on its own line, never a break inside one.
    assert_eq!(
        fmt_with(
            "for (var i = 1 + (a == 2 ? 4 : 0); i <= 4 + (a == 2 ? 4 : 0); i++) {}",
            r#"{"newline": "\n", "max_columns": 40}"#
        ),
        "for (\n    var i = 1 + (a == 2 ? 4 : 0);\n    i <= 4 + (a == 2 ? 4 : 0);\n    i++\n) {\n}\n"
    );
    assert_eq!(
        fmt_with(
            "for (var i = 1; i <= len(aaaaaaaaaaaa); i++) {}",
            r#"{"newline": "\n", "max_columns": 40, "parentheses.padding": true}"#
        ),
        "for (\n    var i = 1;\n    i <= len( aaaaaaaaaaaa );\n    i++\n) {\n}\n"
    );
    // A header that fits stays on one line, `for (;;)` tight.
    assert_eq!(
        fmt("for (i = 1; i <= 3; i++) {}"),
        "for (i = 1; i <= 3; i++) {\n}\n"
    );
    assert_eq!(fmt("for (;;) {}"), "for (;;) {\n}\n");
}

#[test]
fn comments_keep_their_line() {
    assert_eq!(fmt("{ // open\n\n  a;\n\n}"), "{ // open\n    a;\n}\n");
    assert_eq!(
        fmt("switch (x) { case 1: // c\n a(); break; }"),
        "switch (x) {\n    case 1: // c\n        a();\n        break;\n}\n"
    );
    assert_eq!(fmt("if ( a // x\n) {}"), "if (\n    a // x\n) {\n}\n");
    // A statement's terminator follows its last part, before a comment that
    // trails it: `;` never lands inside a line comment.
    assert_eq!(fmt("foo()\n// c\n;"), "foo();\n// c\n");
    assert_eq!(fmt("x = 1 // c\n;"), "x = 1; // c\n");
    assert_eq!(fmt("x = 1 /* c */ ;"), "x = 1; /* c */\n");
}

#[test]
fn blank_lines_and_trailing_newline() {
    // A run of blank lines is one blank line, wherever the source keeps
    // them: between statements, after a block comment inside a statement,
    // after a brace-less arrow body, between members and in a case body.
    assert_eq!(fmt("\n\nx = 1;\n\n\n\ny = 2;\n\n\n"), "x = 1;\n\ny = 2;\n");
    assert_eq!(fmt("x = 1;\n\ny = 2;"), "x = 1;\n\ny = 2;\n");
    assert_eq!(fmt("var /* c */\n\n\n\nx = 1;"), "var /* c */\n\nx = 1;\n");
    assert_eq!(
        fmt("f = (a) => a\n\n\n\ng = 1;"),
        "f = (a) => a\n\ng = 1;\n"
    );
    assert_eq!(
        fmt("component {\n    function a() {}\n\n\n\n    function b() {}\n}"),
        "component {\n\n    function a() {\n    }\n\n    function b() {\n    }\n\n}\n"
    );
    assert_eq!(
        fmt("switch (a) {\n    case 1:\n        x = 1;\n\n\n\n        y = 2;\n}"),
        "switch (a) {\n    case 1:\n        x = 1;\n\n        y = 2;\n}\n"
    );
    assert_eq!(fmt(""), "");
    assert_eq!(fmt("x = 1"), "x = 1\n");
}

#[test]
fn blank_lines_after_a_brace_less_arrow_body() {
    // The body holds the newlines that end its statement and, in a block,
    // the next line's indentation: the blank line is still the statement
    // list's, kept inside a block as at the root.
    assert_eq!(
        fmt("function a() {\n    f = (a) => a\n\n    g = 1;\n}"),
        "function a() {\n    f = (a) => a\n\n    g = 1;\n}\n"
    );
    assert_eq!(
        fmt("function a() {\n    f = (a) => a\n\n\n\n    g = 1;\n    h = (a) => a\n    i = 1;\n}"),
        "function a() {\n    f = (a) => a\n\n    g = 1;\n    h = (a) => a\n    i = 1;\n}\n"
    );
    // It ends an alignment run.
    assert_eq!(
        fmt_with(
            "f = (a) => a\n\ngg = 1;\nh = 2;",
            r#"{"newline": "\n", "alignment.consecutive.assignments": true}"#
        ),
        "f = (a) => a\n\ngg = 1;\nh  = 2;\n"
    );
    // Before a list's closer or comma it is nothing.
    assert_eq!(fmt("foo((a) => a\n\n\n);"), "foo((a) => a);\n");
    assert_eq!(fmt("foo((a) => a\n\n\n, b);"), "foo((a) => a, b);\n");
    assert_eq!(fmt("x = [(a) => a\n\n\n];"), "x = [(a) => a];\n");
}

#[test]
fn strings() {
    assert_eq!(fmt(r#"a = "x #y+1# ##z";"#), "a = 'x #y + 1# ##z';\n");
    assert_eq!(fmt("a = \"one\n  two\";"), "a = 'one\n  two';\n");
    assert_eq!(
        fmt_with(
            r#"a = 'it''s';"#,
            r#"{"newline": "\n", "strings.quote": "double"}"#
        ),
        "a = \"it's\";\n"
    );
    assert_eq!(
        fmt_with(
            "a = \"#aVeryLongName + anotherVeryLongName#\";",
            r#"{"newline": "\n", "max_columns": 10}"#
        ),
        "a = '#aVeryLongName + anotherVeryLongName#';\n"
    );
}

#[test]
fn unstructured_runs_are_spaced_by_the_printer() {
    assert_eq!(
        fmt("try {} catch(\"x\"   e) {}"),
        "try {\n} catch (\"x\" e) {\n}\n"
    );
    assert_eq!(
        fmt("foo:while (true) { break  foo ; }"),
        "foo: while (true) {\n    break foo;\n}\n"
    );
    assert_eq!(fmt("final   var y=2;"), "final var y = 2;\n");
}

#[test]
fn soak_regressions() {
    // Two line comments never share one line suffix.
    assert_eq!(
        fmt("return (\n// a\nb OR\n// c\nd\n);"),
        "return (\n    // a\n    b OR // c\n    d\n);\n"
    );
    assert_eq!(
        fmt("x = (\n// a\n// b\nc);"),
        "x = (\n    // a\n    // b\n    c\n);\n"
    );
    // `static foo = 1;` is one statement, the modifier part of it.
    assert_eq!(
        fmt("component {\n    static {\n        static foo = 9000; final brad = 'wood'\n    }\n}"),
        "component {\n\n    static {\n        static foo = 9000;\n        final brad = 'wood'\n    }\n\n}\n"
    );
    // A brace-less arrow body keeps the newline after it.
    assert_eq!(fmt("f = () => 1\nb"), "f = () => 1\nb\n");
}

#[test]
fn delimited_items_keep_their_comments() {
    assert_eq!(
        fmt("x = [\n 1, // a\n // b\n];"),
        "x = [\n    1, // a\n    // b\n];\n"
    );
    assert_eq!(fmt("x = [1 /* b */, 2];"), "x = [1 /* b */, 2];\n");
    assert_eq!(
        fmt("x = {\n a: 1 // c\n , b: 2\n};"),
        "x = {\n    a: 1, // c\n    b: 2\n};\n"
    );
    assert_eq!(
        fmt_with(
            "x = [\n 1,\n 2 // two\n];",
            r#"{"newline": "\n", "multiline.comma": "dangling"}"#
        ),
        "x = [\n    1,\n    2, // two\n];\n"
    );
    assert_eq!(fmt("x = { // only\n};"), "x = {\n    // only\n};\n");
    // An own-line comment between two parts of an item stays between them.
    assert_eq!(
        fmt("x = [\n /* a */\n // b\n {c: 1}\n];"),
        "x = [\n    /* a */\n    // b\n    {c: 1}\n];\n"
    );
}

#[test]
fn ignore_regions_own_their_newline() {
    let src =
        "if (a) {\n}\n// cfformat-ignore-start\nx=1;\n// cfformat-ignore-end\n// after\ny = 2;\n";
    assert_eq!(
        fmt(src),
        "if (a) {\n}\n// cfformat-ignore-start\nx=1;\n// cfformat-ignore-end\n// after\ny = 2;\n"
    );
}

#[test]
fn ignore_region_after_unterminated_statement() {
    // Without a `;` the statement ends before the region, as it does before
    // a comment: the region is the list's, not the statement's trailing
    // comment.
    let src = "component {\n\n    this.test='abc'\n    // cfformat-ignore-start\n    \
               this.foo = [1, 2,3];\n    // cfformat-ignore-end\n\n}\n";
    assert_eq!(
        fmt(src),
        "component {\n\n    this.test = 'abc'\n    // cfformat-ignore-start\n    \
         this.foo = [1, 2,3];\n    // cfformat-ignore-end\n\n}\n"
    );
    assert_eq!(
        fmt("a = 1\n/* cfformat-ignore-start */\nb=2;\n/* cfformat-ignore-end */\nc = 3;\n"),
        "a = 1\n/* cfformat-ignore-start */\nb=2;\n/* cfformat-ignore-end */\nc = 3;\n"
    );
}

#[test]
fn call_arguments_hug() {
    let narrow = r#"{"newline": "\n", "max_columns": 30}"#;
    // The last argument breaks while the others stay on the call's line.
    assert_eq!(
        fmt_with("foo(alpha, {a: 1, bb: 2, ccc: 3});", narrow),
        "foo(alpha, {\n    a: 1,\n    bb: 2,\n    ccc: 3\n});\n"
    );
    // A first function argument followed by one more.
    assert_eq!(
        fmt("setTimeout(function() { go(); }, 10);"),
        "setTimeout(function() {\n    go();\n}, 10);\n"
    );
    // Every argument a function.
    assert_eq!(
        fmt("then(function() { a(); }, function() { b(); });"),
        "then(function() {\n    a();\n}, function() {\n    b();\n});\n"
    );
    // A named argument's function value hugs too.
    assert_eq!(
        fmt("describe(title = 'x', body = function() { a(); });"),
        "describe(title = 'x', body = function() {\n    a();\n});\n"
    );
    // A comment in the argument list: the delimited layout only.
    assert_eq!(
        fmt("foo(a, // c\nfunction() { b(); });"),
        "foo(\n    a, // c\n    function() {\n        b();\n    }\n);\n"
    );
    // Head arguments that do not fit: all broken.
    assert_eq!(
        fmt_with(
            "foo(aaaaaaaaaa, bbbbbbbbbbbb, function() { b(); });",
            narrow
        ),
        "foo(\n    aaaaaaaaaa,\n    bbbbbbbbbbbb,\n    function() {\n        b();\n    }\n);\n"
    );
}

#[test]
fn nested_callbacks_stay_linear() {
    let depth = 10;
    let mut src = String::new();
    for i in 0..depth {
        src.push_str(&format!("describe('{i}', function() {{\n"));
    }
    src.push_str("x(1);\n");
    for _ in 0..depth {
        src.push_str("});\n");
    }
    let start = std::time::Instant::now();
    let out = fmt_with(&src, r#"{"newline": "\n", "max_columns": 1000}"#);
    assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
    assert!(out.starts_with("describe('0', function() {\n    describe('1', function() {\n"));
}

#[test]
fn deep_nesting_prints_as_written() {
    // `MAX_DEPTH` + 50 levels on a 2 MB thread, as a test or a library
    // user's thread has: below the bound an element prints as written.
    // Script nests at most the parser's 100 on its own, so each script
    // shape is also put under enough tags to cross the bound.
    let n = MAX_DEPTH as usize + 50;
    let tags = |n: usize, inner: &str| {
        format!("{}{inner}{}", "<cfif a>\n".repeat(n), "</cfif>\n".repeat(n))
    };
    let parens = format!("x = {}1{};", "(".repeat(n), ")".repeat(n));
    let structs = format!("x = {}1{};", "{a: ".repeat(n), "}".repeat(n));
    let blocks = format!("{}x = 1;{}", "if (a) {\n".repeat(n), "}\n".repeat(n));
    let cases = [
        (tags(n, "x"), Mode::Tags),
        (parens.clone(), Mode::Script),
        (structs.clone(), Mode::Script),
        (blocks.clone(), Mode::Script),
        (
            tags(n - 95, &format!("<cfscript>{parens}</cfscript>")),
            Mode::Tags,
        ),
        (
            tags(n - 95, &format!("<cfscript>{structs}</cfscript>")),
            Mode::Tags,
        ),
        (
            tags(n - 95, &format!("<cfscript>{blocks}</cfscript>")),
            Mode::Tags,
        ),
    ];
    let deep = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let opts = Options::default();
            for (src, mode) in &cases {
                let out = format_source(src, *mode, &opts);
                let again = format_source(&out, *mode, &opts);
                assert!(again == out, "not idempotent: {:?}", &src[..40]);
                let problems = common::check_output(src, &out, *mode, &opts);
                assert!(problems.is_empty(), "{:?}: {problems:?}", &src[..40]);
            }
        })
        .unwrap();
    deep.join().unwrap();
    // The bound itself: `MAX_DEPTH` bodies are laid out, the next one starts
    // at their indent and keeps the source's own layout.
    let lf = Options {
        newline: cfformat::options::NewlineStyle::Lf,
        ..Options::default()
    };
    let out = format_source(&tags(n, "x"), Mode::Tags, &lf);
    let indent = " ".repeat(4 * MAX_DEPTH as usize);
    assert!(
        out.contains(&format!("\n{indent}<cfif a>\n<cfif a>\n")),
        "{}",
        &out[..200]
    );
}

#[test]
fn unclassified_runs_never_break() {
    // `sequence` prints the gaps inside an unclassified run as written:
    // joined with one space, a run holding a string's delimiter would change
    // the string's text (and a misparse could break inside it). No source
    // reaches a multi-token run with a delimiter, so the tree is built by
    // hand: the statement's tokens after `x =` retagged `other`.
    use cfdoc::{print_doc, IndentStyle, PrintOptions};
    use cfformat::print::Printer;
    use cfparse::{Node, TokenKind};

    let src = "x = foo \"a  b\n    c\" y;\n";
    let mut tree = cfparse::parse_source(src, Mode::Script);
    // The parser reads several statements; keep the first, give it the
    // last one's terminator and the file's last newline.
    let terminator = tree
        .root
        .children
        .iter()
        .filter_map(Node::as_element)
        .find_map(|e| e.terminator().cloned())
        .unwrap();
    let newline = tree.root.children.last().unwrap().clone();
    tree.root.children.truncate(1);
    tree.root.children.push(newline);
    let Node::Element(statement) = &mut tree.root.children[0] else {
        panic!("a statement")
    };
    statement.close = Some(terminator);
    // `foo "a`, `  `, `b`, newline, `    `, `c"`, ` `, `y`: flat tokens.
    let (start, end) = (4u32, src.find(';').unwrap() as u32);
    let mut tokens = Vec::new();
    let mut at = start;
    for piece in ["foo \"a", "  ", "b", "\n", "    ", "c\"", " ", "y"] {
        let kind = match piece {
            "\n" => TokenKind::Newline,
            p if p.trim().is_empty() => TokenKind::Whitespace,
            _ => TokenKind::Other,
        };
        let mut t = statement.tokens()[0].clone();
        t.span = at..at + piece.len() as u32;
        t.kind = kind;
        tokens.push(Node::Token(t));
        at += piece.len() as u32;
    }
    assert_eq!(at, end);
    let head: Vec<Node> = statement
        .tokens()
        .into_iter()
        .filter(|t| t.span.end <= start)
        .map(Node::Token)
        .collect();
    statement.children = head.into_iter().chain(tokens).collect();
    statement.span = 0..end + 1;
    let opts = Options::default();
    let mut doc = Printer::new(&tree, &opts).document();
    let out = print_doc(
        &mut doc,
        &PrintOptions {
            width: 10,
            indent: IndentStyle::Spaces(4),
            newline: "\n",
        },
    );
    assert_eq!(out, src);
}

#[test]
fn function_headers_and_arrow_bodies() {
    // The comment and blank line a brace-less arrow body ends with keep
    // their lines.
    assert_eq!(
        fmt("g = a => a\n// c\n\nh = 1;"),
        "g = a => a\n// c\n\nh = 1;\n"
    );
    assert_eq!(fmt("f = () => 1\n\nb"), "f = () => 1\n\nb\n");
    assert_eq!(fmt("foo(() => 1\n);"), "foo(() => 1);\n");
    // Comments in a header stay in it.
    assert_eq!(
        fmt("x = function /* c */ (a) // d\n{ };"),
        "x = function /* c */ (a) { // d\n};\n"
    );
    // Metadata that breaks puts `{` on its own line.
    assert_eq!(
        fmt_with(
            "function f() hint=\"a long hint\" output=false {}",
            r#"{"newline": "\n", "max_columns": 30}"#
        ),
        "function f()\n    hint=\"a long hint\"\n    output=false\n{\n}\n"
    );
    // A body-less declaration keeps its terminator.
    assert_eq!(
        fmt("interface { function f(a,b); }"),
        "interface {\n\n    function f(a, b);\n\n}\n"
    );
}

#[test]
fn components_properties_and_script_tags() {
    assert_eq!(fmt("import  a.b.*;"), "import a.b.*;\n");
    assert_eq!(fmt("import \"a.b\";"), "import \"a.b\";\n");
    assert_eq!(
        fmt("lock name=\"x\" timeout=10 { a = 1; }"),
        "lock name=\"x\" timeout=10 {\n    a = 1;\n}\n"
    );
    assert_eq!(
        fmt("application action=\"update\" mappings=getApplicationSettings().mappings;"),
        "application action=\"update\" mappings=getApplicationSettings().mappings;\n"
    );
    assert_eq!(
        fmt("component { static { static x = 1; } }"),
        "component {\n\n    static {\n        static x = 1;\n    }\n\n}\n"
    );
    assert_eq!(
        fmt("property name=\"a\" /* c */ type=\"b\"; // d"),
        "property name=\"a\" /* c */ type=\"b\"; // d\n"
    );
    // An ACF-form tag's attribute strings follow strings.attributes.quote.
    assert_eq!(
        fmt("cfhttp(url='x', result = 'r');"),
        "cfhttp(url = \"x\", result = \"r\");\n"
    );
}

#[test]
fn soak_regressions_2() {
    // A line comment inside a template expression keeps its lines.
    assert_eq!(
        fmt("var test = \"#foo\n// c\n# true\";"),
        "var test = '#foo\n// c\n# true';\n"
    );
    // `IsNull (x)` is the builtin whatever the space before `(`.
    assert_eq!(fmt("if (IsNull (x)) {}"), "if (isNull(x)) {\n}\n");
    // Brackets after a type stay glued to it.
    assert_eq!(
        fmt("component { public User[] function f() {} }"),
        "component {\n\n    public User[] function f() {\n    }\n\n}\n"
    );
    // Unclassified text in an attribute list prints as written.
    let out = fmt("property string p preanno=[\"a\", \"b\"];");
    assert!(out.starts_with("property string p preanno="), "{out}");
    // A body-less header before `}` leaves the `}` to the class, never
    // `function f() }`.
    assert_eq!(
        fmt("abstract component { function f() }"),
        "abstract component {\n\n    function f()\n\n}\n"
    );
}

#[test]
fn a_threshold_broken_argument_is_never_hugged_around() {
    // A non-hugged argument that breaks only through its own
    // threshold counts as a broken argument, like one with a callback body.
    let settings = r#"{"newline": "\n", "struct.multiline.element_count": 4, "struct.multiline.min_item_length": 0, "function_call.multiline.element_count": 4, "function_call.multiline.min_item_length": 0}"#;
    assert_eq!(
        fmt_with(
            "foo({a: 1, b: 2, c: 3, d: 4}, function() { return 1; });",
            settings
        ),
        "foo(\n    {\n        a: 1,\n        b: 2,\n        c: 3,\n        d: 4\n    },\n    function() {\n        return 1;\n    }\n);\n"
    );
    assert_eq!(
        fmt_with(
            "foo(bar(a, b, c, d), function() { return 1; });",
            settings
        ),
        "foo(\n    bar(\n        a,\n        b,\n        c,\n        d\n    ),\n    function() {\n        return 1;\n    }\n);\n"
    );
    // A single threshold-broken struct still hugs.
    assert_eq!(
        fmt_with("foo({a: 1, b: 2, c: 3, d: 4});", settings),
        "foo({\n    a: 1,\n    b: 2,\n    c: 3,\n    d: 4\n});\n"
    );
}

/// `method_call.chain.multiline: 3`: every chain of three methods breaks, so
/// the chain tests below see the broken layout.
const CHAINS: &str = r#"{"newline": "\n", "method_call.chain.multiline": 3}"#;

#[test]
fn chain_first_method_merges_with_a_short_head() {
    // Prettier's `shouldNotWrap`: at the start of an expression statement
    // a head no longer than `indent_size` keeps the first call.
    assert_eq!(
        fmt_with("aaaa.b().c().d();", CHAINS),
        "aaaa.b()\n    .c()\n    .d();\n"
    );
    assert_eq!(
        fmt_with("aaaaa.b().c().d();", CHAINS),
        "aaaaa\n    .b()\n    .c()\n    .d();\n"
    );
    assert_eq!(
        fmt_with("this.a().b().c();", CHAINS),
        "this.a()\n    .b()\n    .c();\n"
    );
    let two = r#"{"newline": "\n", "method_call.chain.multiline": 2}"#;
    assert_eq!(fmt_with("this.a().b();", two), "this.a()\n    .b();\n");
    // A first group with properties merges only on a factory-like last
    // property.
    assert_eq!(
        fmt_with("this.x.y().z();", two),
        "this.x\n    .y()\n    .z();\n"
    );
    assert_eq!(fmt_with("this.X.y().z();", two), "this.X.y()\n    .z();\n");
    assert_eq!(fmt("this.x.y().z();"), "this.x.y().z();\n");
    // A factory-like head merges anywhere.
    assert_eq!(
        fmt("x = Object.keys(items).filter((x) => x).map((x) => x);"),
        "x = Object.keys(items)\n    .filter((x) => x)\n    .map((x) => x);\n"
    );
    // Heads that are not names: a `New`, a call, a string, an array.
    assert_eq!(
        fmt_with("new X(1).init().run().go();", CHAINS),
        "new X(1)\n    .init()\n    .run()\n    .go();\n"
    );
    assert_eq!(
        fmt_with("foo().bar().baz().qux();", CHAINS),
        "foo()\n    .bar()\n    .baz()\n    .qux();\n"
    );
    assert_eq!(
        fmt_with("\"str\".len().a().b();", CHAINS),
        "'str'\n    .len()\n    .a()\n    .b();\n"
    );
    assert_eq!(
        fmt("[1, 2].map((i) => i).filter((i) => i).a();"),
        "[1, 2]\n    .map((i) => i)\n    .filter((i) => i)\n    .a();\n"
    );
    // A head that holds a forced break never merges.
    assert_eq!(
        fmt("foo(function() { x(); }).bar().baz();"),
        "foo(function() {\n    x();\n})\n    .bar()\n    .baz();\n"
    );
}

#[test]
fn a_chain_stays_on_one_line_while_it_fits() {
    // Prettier: short chains with simple arguments never break, however
    // many calls; a longer one breaks a call per line.
    assert_eq!(fmt("t.a().b().c();"), "t.a().b().c();\n");
    assert_eq!(
        fmt("framework.renderData('json').data({'success': true}).statusCode(200);"),
        "framework.renderData('json').data({'success': true}).statusCode(200);\n"
    );
    let long = "x".repeat(60);
    assert_eq!(
        fmt(&format!("result = q.select('{long}').from('{long}');")),
        format!("result = q\n    .select('{long}')\n    .from('{long}');\n")
    );
    // More than two calls with a function argument always break.
    assert_eq!(
        fmt("x = a.b((i) => i).c().d();"),
        "x = a\n    .b((i) => i)\n    .c()\n    .d();\n"
    );
    // A property run stays with its method.
    assert_eq!(
        fmt_with(
            "items.last().children.append(x);",
            CHAINS.replace("3", "2").as_str()
        ),
        "items\n    .last()\n    .children.append(x);\n"
    );
}

#[test]
fn chain_segments() {
    // An index stays on the line of what it indexes.
    assert_eq!(
        fmt_with("a[1].b().c().d();", CHAINS),
        "a[1]\n    .b()\n    .c()\n    .d();\n"
    );
    assert_eq!(
        fmt_with("a.b()[1].c().d();", CHAINS),
        "a.b()[1]\n    .c()\n    .d();\n"
    );
    // Safe and static accessors print as written.
    assert_eq!(
        fmt_with("a?.b?.c().d()?.e();", CHAINS),
        "a?.b\n    ?.c()\n    .d()\n    ?.e();\n"
    );
    assert_eq!(fmt("A::b().c().d;"), "A::b().c().d;\n");
    // A call around a chain, a numeric member.
    assert_eq!(fmt("a.b()();"), "a.b()();\n");
    assert_eq!(fmt("a[1](2);"), "a[1](2);\n");
    assert_eq!(fmt("a.b.1234.c;"), "a.b.1234.c;\n");
    // Member names are never cased; the space before `(` goes.
    assert_eq!(fmt("obj.ArrayAppend(1);"), "obj.ArrayAppend(1);\n");
    assert_eq!(fmt("x.delete ('a');"), "x.delete('a');\n");
    // `parentheses.padding` reaches a segment's arguments.
    let padded =
        r#"{"newline": "\n", "parentheses.padding": true, "method_call.chain.multiline": 3}"#;
    assert_eq!(
        fmt_with("a.b(1).c(2).d();", padded),
        "a.b( 1 )\n    .c( 2 )\n    .d();\n"
    );
}

#[test]
fn chains_inside_expressions() {
    // The first method joins line 0 only when the chain starts an expression
    // statement: after `return`, `x = `, `(` the head stands alone. A
    // hugged call's argument, a delimited argument, a keyword group, a
    // return value, an assignment.
    assert_eq!(
        fmt_with("foo(function() { return a.b().c().d(); });", CHAINS),
        "foo(function() {\n    return a\n        .b()\n        .c()\n        .d();\n});\n"
    );
    assert_eq!(
        fmt_with("foo(function() { a.b().c().d(); });", CHAINS),
        "foo(function() {\n    a.b()\n        .c()\n        .d();\n});\n"
    );
    assert_eq!(
        fmt_with("foo(a.b().c().d(), 1);", CHAINS),
        "foo(\n    a\n        .b()\n        .c()\n        .d(),\n    1\n);\n"
    );
    assert_eq!(
        fmt_with("if (a.b().c().d()) { x = 1; }", CHAINS),
        "if (\n    a\n        .b()\n        .c()\n        .d()\n) {\n    x = 1;\n}\n"
    );
    // A member chain after `=` is fluid: its head keeps the operator's line.
    assert_eq!(
        fmt_with("x = foo.b().c().d();", CHAINS),
        "x = foo\n    .b()\n    .c()\n    .d();\n"
    );
    assert_eq!(
        fmt_with("var x = t.b().c().d();", CHAINS),
        "var x = t\n    .b()\n    .c()\n    .d();\n"
    );
    assert_eq!(
        fmt_with("x = testObj.b().c().d();", CHAINS),
        "x = testObj\n    .b()\n    .c()\n    .d();\n"
    );
    // A chain wrapped in a call is not the statement's chain.
    assert_eq!(
        fmt_with("t.b().c().d()();", CHAINS),
        "t\n    .b()\n    .c()\n    .d()();\n"
    );
    assert_eq!(
        fmt_with("x = foo(a).bar(b);", CHAINS),
        "x = foo(a).bar(b);\n"
    );
    assert_eq!(
        fmt_with("x = [1, 2, 3].map((i) => i * 2);", CHAINS),
        "x = [1, 2, 3].map((i) => i * 2);\n"
    );
}

#[test]
fn chain_callbacks() {
    // Two methods and a callback: the chain stays on one line and the
    // callback breaks inside it.
    assert_eq!(
        fmt("a.b(function() { x(); }).c();"),
        "a.b(function() {\n    x();\n}).c();\n"
    );
    // The last call's callback breaks under a one-line chain.
    assert_eq!(
        fmt("x = rc.user.getItems().find(function(item) { return item.y == 1; });"),
        "x = rc.user.getItems().find(function(item) {\n    return item.y == 1;\n});\n"
    );
    assert_eq!(
        fmt("items.filter(function(i) { return i; }).map(function(i) { return i; });"),
        "items\n    .filter(function(i) {\n        return i;\n    })\n    .map(function(i) {\n        return i;\n    });\n"
    );
    // One method: flat, the callback hugged.
    assert_eq!(
        fmt("arguments.x.each(function(i) { x(); });"),
        "arguments.x.each(function(i) {\n    x();\n});\n"
    );
}

#[test]
fn chain_comments() {
    // An own-line comment before the first segment, inside the indent.
    assert_eq!(
        fmt("testing\n// boo\n.a();"),
        "testing\n    // boo\n    .a();\n"
    );
    // A same-line comment after the head, no methods.
    assert_eq!(fmt("a // c\n.b;"), "a // c\n    .b;\n");
    assert_eq!(fmt("a\n// c\n.b\n.c;"), "a\n    // c\n    .b.c;\n");
    // After a comment line, a first method after a property starts a line.
    assert_eq!(
        fmt("a.b // c\n.c.d().e;"),
        "a.b // c\n    .c\n    .d()\n    .e;\n"
    );
    // A block comment stays in place.
    assert_eq!(fmt("a /* c */ .b();"), "a /* c */ .b();\n");
}

#[test]
fn chain_layout_ignores_source_line_breaks() {
    // The layout comes from structure: the same chain broken or not in the
    // source prints the same.
    for src in [
        "getInstance('x').list().reduce((r, row) => r.append(row), []);",
        "getInstance('x')\n    .list()\n    .reduce((r, row) => r.append(row), []);",
        "getInstance('x')\n.list().reduce(\n(r, row) => r.append(row),\n[]\n);",
    ] {
        assert_eq!(
            fmt(src),
            "getInstance('x')\n    .list()\n    .reduce((r, row) => r.append(row), []);\n"
        );
    }
    for src in ["t.a().b().c();", "t\n.a()\n.b()\n.c();"] {
        assert_eq!(fmt(src), "t.a().b().c();\n");
    }
}

fn aligned(src: &str, extra: &str) -> String {
    let settings =
        format!(r#"{{"newline": "\n", "alignment.consecutive.assignments": true{extra}}}"#);
    fmt_with(src, &settings)
}

#[test]
fn statement_runs() {
    // An augmented operator and a call in the target end a run.
    assert_eq!(
        aligned("x = 1;\ny.z = 2;\nabc += 3;\nd = 4;", ""),
        "x   = 1;\ny.z = 2;\nabc += 3;\nd = 4;\n"
    );
    assert_eq!(
        aligned("aa = 1;\na.b().c = 2;\nb = 3;\na[1].bbbb = 4;", ""),
        "aa = 1;\na.b().c = 2;\nb         = 3;\na[1].bbbb = 4;\n"
    );
    // A blank line ends a run; a comment, own-line or trailing, does not.
    assert_eq!(
        aligned("a = 1;\n\nbbb = 2; // t\n// own\ncc = 3;", ""),
        "a = 1;\n\nbbb = 2; // t\n// own\ncc  = 3;\n"
    );
    // A value that breaks does not end the run: a run is of statements, not
    // of lines.
    assert_eq!(
        aligned(
            "var a = 1;\nvar s = {\n    k: 1\n};\nvar bb = 2;",
            r#", "struct.multiline.min_item_length": 0, "struct.multiline.element_count": 1"#
        ),
        "var a  = 1;\nvar s  = {\n    k: 1\n};\nvar bb = 2;\n"
    );
    // `var`, bare and `param` statements share a run; the left side is
    // everything before the operator, printed.
    assert_eq!(
        aligned(
            "param string a = \"x\";\nvar bbbbb = 1;\nccc.d = 2;\nslide[ \"k\" ] = 3;",
            ""
        ),
        "param string a = 'x';\nvar bbbbb      = 1;\nccc.d          = 2;\nslide['k']     = 3;\n"
    );
    // Inside blocks too; padding is spaces under `tab_indent`.
    assert_eq!(
        aligned(
            "function f() {\na = 1;\nbbb = 2;\n}",
            r#", "tab_indent": true"#
        ),
        "function f() {\n\ta   = 1;\n\tbbb = 2;\n}\n"
    );
}

#[test]
fn item_runs() {
    let call = r#", "function_call.multiline.element_count": 2, "function_call.multiline.min_item_length": 0"#;
    // A flat list never pads.
    assert_eq!(aligned("f(a = 1, bb = 2);", ""), "f(a = 1, bb = 2);\n");
    assert_eq!(aligned("s = {a: 1, bb: 2};", ""), "s = {a: 1, bb: 2};\n");
    // Positional arguments split a named-argument run into singletons.
    assert_eq!(
        aligned("f(a = 1, b, cc = 2);", call),
        "f(\n    a = 1,\n    b,\n    cc = 2\n);\n"
    );
    // `new` arguments and method arguments.
    assert_eq!(
        aligned("x = new X(a = 1, bb = 2);", call),
        "x = new X(\n    a  = 1,\n    bb = 2\n);\n"
    );
    assert_eq!(
        aligned("x.m(a = 1, bb = 2);", call),
        "x.m(\n    a  = 1,\n    bb = 2\n);\n"
    );
    // A hugged call keeps its named arguments on one line: no padding. Broken
    // out, they pad.
    assert_eq!(
        aligned("foo(a = 1, bbb = function() { return 1; });", ""),
        "foo(a = 1, bbb = function() {\n    return 1;\n});\n"
    );
    assert_eq!(
        aligned("foo(a = 1, bb = 2, ccc = function() { return 1; });", call),
        "foo(\n    a   = 1,\n    bb  = 2,\n    ccc = function() {\n        return 1;\n    }\n);\n"
    );
    // Anonymous function parameters: modifiers and type are part of the left
    // side; a parameter without a default ends the run.
    assert_eq!(
        aligned(
            "g = function(aaaa = 1, required string bb = 2, c, dd = 3) {};",
            r#", "function_anonymous.multiline.element_count": 1, "function_anonymous.multiline.min_item_length": 0"#
        ),
        "g = function(\n    aaaa               = 1,\n    required string bb = 2,\n    c,\n    dd = 3\n) {\n};\n"
    );
    // A member whose value is a function (no `KeyValue` in the tree) aligns
    // like one.
    assert_eq!(
        aligned("s = {\nudf: function() { return 1; },\nlonger: 2\n};", ""),
        "s = {\n    udf   : function() {\n        return 1;\n    },\n    longer: 2\n};\n"
    );
    // A comment-only item and item comments do not end a run.
    assert_eq!(
        aligned("s = {\na: 1, // t\n// own\nbbb: 2\n};", ""),
        "s = {\n    a  : 1, // t\n    // own\n    bbb: 2\n};\n"
    );
}

#[test]
fn attribute_runs() {
    let narrow = r#", "max_columns": 40"#;
    // A property on one line never pads; broken, the keys pad before `=`.
    assert_eq!(
        aligned("property name=\"x\" type=\"y\";", ""),
        "property name=\"x\" type=\"y\";\n"
    );
    assert_eq!(
        aligned(
            "property name=\"name\" fieldtype=\"id\" ormType=\"string\";",
            narrow
        ),
        "property\n    name     =\"name\"\n    fieldtype=\"id\"\n    ormType  =\"string\";\n"
    );
    assert_eq!(
        aligned(
            "property name=\"name\" fieldtype=\"id\" ormType=\"string\";",
            r#", "max_columns": 40, "attributes.key_value.padding": true"#
        ),
        "property\n    name      = \"name\"\n    fieldtype = \"id\"\n    ormType   = \"string\";\n"
    );
    // Function metadata; a bare attribute ends a run.
    assert_eq!(
        aligned(
            "function h() output=false description=\"A description\" abstract access=\"public\" {}",
            narrow
        ),
        "function h()\n    output     =false\n    description=\"A description\"\n    abstract\n    access=\"public\"\n{\n}\n"
    );
}

#[test]
fn alignment_off_prints_as_before() {
    let src = "var a = 1;\nvar bbb = 2;\nf(a = 1, bb = 2);\nproperty name=\"name\" fieldtype=\"id\" ormType=\"string\";";
    assert_eq!(
        fmt_with(
            src,
            r#"{"newline": "\n", "max_columns": 40, "function_call.multiline.element_count": 1, "function_call.multiline.min_item_length": 0}"#
        ),
        "var a = 1;\nvar bbb = 2;\nf(\n    a = 1,\n    bb = 2\n);\nproperty\n    name=\"name\"\n    fieldtype=\"id\"\n    ormType=\"string\";\n"
    );
}

#[test]
fn doc_comment_alignment() {
    let on = r#"{"newline": "\n", "alignment.doc_comments": true}"#;
    // No tags: unchanged, empty lines included.
    let untagged = "/**\n * Hello\n *\n *\n * world\n */\nx = 1;\n";
    assert_eq!(fmt_with(untagged, on), untagged);
    // A continuation line stays after its tag; `@returns` becomes `@return`
    // and moves before `@throws`; one empty line before each block.
    assert_eq!(
        fmt_with(
            "/**\n * Does things.\n * @returns The thing\n * @param a The first\n *   continued here\n * @abc Second\n * @throws Foo when\n * @throws LongerName when else\n */\nx = 1;",
            on
        ),
        "/**\n * Does things.\n *\n * @param a The first\n *   continued here\n * @abc   Second\n *\n * @return The thing\n *\n * @throws Foo        when\n * @throws LongerName when else\n */\nx = 1;\n"
    );
    // Throws only, no description: no empty line first.
    assert_eq!(
        fmt_with("/**\n * @throws A x\n * @throws Bbb y\n */\n", on),
        "/**\n * @throws A   x\n * @throws Bbb y\n */\n"
    );
    // The empty separators between blocks do not become a description
    // (CommandBox printed them as a leading ` *` line); an empty line
    // before the first tag is description and stays, as in CommandBox.
    assert_eq!(
        fmt_with("/**\n * @param a x\n *\n * @return y\n *\n */\n", on),
        "/**\n * @param a x\n *\n * @return y\n */\n"
    );
    let leading = "/**\n *\n * @param a x\n */\n";
    assert_eq!(fmt_with(leading, on), leading);
    assert_eq!(
        fmt_with("/**\n *\n *\n * Desc\n * @param a x\n */\n", on),
        "/**\n *\n * Desc\n *\n * @param a x\n */\n"
    );
    // A tag with no text prints alone.
    assert_eq!(
        fmt_with("/**\n * @aaa\n * @b x\n */\n", on),
        "/**\n * @aaa\n * @b   x\n */\n"
    );
    // The asterisk re-indent still applies.
    let src = "{\n/**\n      * Desc\n      * @b x\n   * @aaa y\n    */\n}";
    assert_eq!(
        fmt_with(
            src,
            r#"{"newline": "\n", "alignment.doc_comments": true, "comment.asterisks": "indent"}"#
        ),
        "{\n    /**\n    * Desc\n    *\n    * @b   x\n    * @aaa y\n    */\n}\n"
    );
    assert_eq!(
        fmt_with(
            src,
            r#"{"newline": "\n", "alignment.doc_comments": true, "comment.asterisks": "ignored"}"#
        ),
        "{\n    /**\n      * Desc\n   *\n      * @b   x\n   * @aaa y\n    */\n}\n"
    );
    // A CRLF source keeps CRLF with `newline: auto`.
    assert_eq!(
        fmt_with(
            "/**\r\n * @b x\r\n * @aaa y\r\n */\r\nx = 1;\r\n",
            r#"{"newline": "auto", "alignment.doc_comments": true}"#
        ),
        "/**\r\n * @b   x\r\n * @aaa y\r\n */\r\nx = 1;\r\n"
    );
    // A block comment is not a doc comment.
    let block = "/*\n * @b x\n * @aaa y\n */\n";
    assert_eq!(fmt_with(block, on), block);
}

#[test]
fn a_template_expression_keeps_a_hugging_call_s_arguments() {
    // `#expr#` prints flat through `remove_lines`, which rebuilds a
    // conditional group from its states: the argument-hugging group must
    // not store an empty first state, or the arguments vanish.
    assert_eq!(fmt("x = \"#f(a, {b: 1})#\";"), "x = '#f(a, {b: 1})#';\n");
    // `remove_lines` leaves hard lines alone, as Prettier's `removeLines`
    // does, so a body inside `#expr#` still breaks.
    assert_eq!(
        fmt("x = \"#f(a, function() { g(); })#\";"),
        "x = '#f(a, function() {\n    g();\n})#';\n"
    );
}

#[test]
fn a_block_comment_keeps_the_line_breaks_after_it() {
    // As `Printer::statements` keeps the line breaks between statements.
    assert_eq!(fmt("/* c */\n\n's';"), "/* c */\n\n's';\n");
    assert_eq!(fmt("/* c */\n's';"), "/* c */\n's';\n");
    // On one line it still shares it.
    assert_eq!(fmt("/* c */ 's';"), "/* c */ 's';\n");
    // A doc comment before a struct member is on its own line.
    assert_eq!(
        fmt("x = {\n/** d */\n'a': 1\n};"),
        "x = {\n    /** d */\n    'a': 1\n};"[..].to_owned() + "\n"
    );
}

// ---------------------------------------------------------------------------
// Tag mode
// ---------------------------------------------------------------------------

fn tag_with(src: &str, settings: &str) -> String {
    let (opts, _) = Options::from_json(settings).unwrap();
    let out = format_source(src, Mode::Auto, &opts);
    let again = format_source(&out, Mode::Auto, &opts);
    assert_eq!(again, out, "not idempotent for {src:?}");
    out
}

fn tag(src: &str) -> String {
    tag_with(src, r#"{"newline": "\n"}"#)
}

/// Every `islands.*` key `"off"`: islands print as written, shifted.
const ISLANDS_OFF: &str =
    r#"{"newline": "\n", "islands.js": "off", "islands.css": "off", "islands.json": "off"}"#;

#[test]
fn a_block_tag_body_forces_a_line_break_after_it() {
    assert_eq!(tag("<p>a</p><p>b</p>\n"), "<p>a</p>\n<p>b</p>\n");
    assert_eq!(
        tag("<span>a</span><span>b</span>\n"),
        "<span>a</span><span>b</span>\n"
    );
    // A CF tag body breaks after it where whitespace follows it or a block
    // tag does; what is glued to it stays glued. A bare tag never breaks.
    assert_eq!(
        tag("<cfoutput>#a#</cfoutput> x\n"),
        "<cfoutput>#a#</cfoutput>\nx\n"
    );
    assert_eq!(
        tag("<cfoutput>#a#</cfoutput>x\n"),
        "<cfoutput>#a#</cfoutput>x\n"
    );
    assert_eq!(
        tag("<cfoutput>#a#</cfoutput><p>x</p>\n"),
        "<cfoutput>#a#</cfoutput>\n<p>x</p>\n"
    );
    assert_eq!(tag("<br>x\n"), "<br>x\n");
}

/// A `<pre>` / `<textarea>` body keeps every byte but its line endings,
/// which are the output's.
#[test]
fn a_pre_body_takes_the_output_newline() {
    assert_eq!(
        tag("<div>\r\n<pre>\r\n  a  \r\n\r\n\r\n\tb</pre>\r\n</div>\r\n"),
        "<div>\n    <pre>\n  a  \n\n\n\tb</pre>\n</div>\n"
    );
    assert_eq!(
        tag_with("<textarea>\n a\n</textarea>\n", r#"{"newline": "\r\n"}"#),
        "<textarea>\r\n a\r\n</textarea>\r\n"
    );
}

/// A broken inline HTML body has a line break at an end only where the
/// source had whitespace there; a table row, whose edge whitespace a browser
/// does not render, breaks as a block tag's body does.
#[test]
fn an_inline_body_breaks_only_where_it_had_whitespace() {
    assert_eq!(
        tag("<a href=\"x\"><img src=\"y\"></a>\n"),
        "<a href=\"x\"><img src=\"y\"></a>\n"
    );
    assert_eq!(
        tag("<span> a <b>x</b></span>\n"),
        "<span>\n    a <b>x</b></span>\n"
    );
    assert_eq!(
        tag("<span>\n<b>x</b>\n</span>\n"),
        "<span>\n    <b>x</b>\n</span>\n"
    );
    assert_eq!(tag("<span>a\nb</span>\n"), "<span>a\n    b</span>\n");
    assert_eq!(
        tag("<tr><td>a</td><td> b </td></tr>\n"),
        "<tr>\n    <td>a</td><td>b</td>\n</tr>\n"
    );
}

#[test]
fn a_tag_body_breaks_when_it_holds_a_tag_or_a_newline() {
    assert_eq!(tag("<p>testing</p>\n"), "<p>testing</p>\n");
    assert_eq!(tag("<p>testing\n</p>\n"), "<p>\n    testing\n</p>\n");
    assert_eq!(tag("<div><br></div>\n"), "<div>\n    <br>\n</div>\n");
    assert_eq!(tag("<DIV></DIV>\n"), "<div></div>\n");
    // A body of nothing but a newline keeps the two tags on their own lines.
    assert_eq!(tag("<div>\n</div>\n"), "<div>\n</div>\n");
}

#[test]
fn tags_body_indent_cfml_keeps_a_cf_body_that_starts_with_html_flush() {
    const CFML: &str = r#"{"newline": "\n", "islands.js": "off", "tags.body.indent": "cfml"}"#;
    // Judged once, from the first segment: the `<cfelse>` segment follows it.
    assert_eq!(
        tag_with(
            "<cfif a>\n<p>x</p>\n<cfelse>\n<cfset y = 1>\n</cfif>\n",
            CFML
        ),
        "<cfif a>\n<p>x</p>\n<cfelse>\n<cfset y = 1>\n</cfif>\n"
    );
    assert_eq!(
        tag_with(
            "<cfif a>\n<cfset y = 1>\n<cfelse>\n<p>x</p>\n</cfif>\n",
            CFML
        ),
        "<cfif a>\n    <cfset y = 1>\n<cfelse>\n    <p>x</p>\n</cfif>\n"
    );
    // An HTML tag body always indents.
    assert_eq!(
        tag_with("<div>\n<p>x</p>\n</div>\n", CFML),
        "<div>\n    <p>x</p>\n</div>\n"
    );
    // A `<script>` in the flush body sits at its indent, 0 here (4 by
    // default), and its island one level inside it whatever this key says.
    let src = "<cfif a>\n<script>\nvar b = 1;\n</script>\n</cfif>\n";
    assert_eq!(
        tag_with(src, CFML),
        "<cfif a>\n<script>\n    var b = 1;\n</script>\n</cfif>\n"
    );
    assert_eq!(
        tag_with(src, ISLANDS_OFF),
        "<cfif a>\n    <script>\n        var b = 1;\n    </script>\n</cfif>\n"
    );
}

/// `tags.islands.indent: false`.
const FLUSH: &str = r#"{"newline": "\n", "tags.islands.indent": false}"#;

#[test]
fn a_verbatim_island_body_is_raised_to_one_level_in_never_lowered() {
    // A tag comment makes the island impure: verbatim, shifted.
    let flush = "<cfoutput>\n<script>\nvar a = 1; <!--- c --->\nif (a) {\n  b();\n}\n</script>\n</cfoutput>\n";
    let raised = concat!(
        "<cfoutput>\n    <script>\n",
        "        var a = 1; <!--- c --->\n        if (a) {\n          b();\n        }\n",
        "    </script>\n</cfoutput>\n"
    );
    // `tag` checks that a second run changes nothing.
    assert_eq!(tag(flush), raised);
    // Written deeper than the floor: kept where it is.
    let deeper = concat!(
        "<cfoutput>\n    <script>\n",
        "              var a = 1; <!--- c --->\n              if (a) {\n                b();\n              }\n",
        "    </script>\n</cfoutput>\n"
    );
    assert_eq!(tag(deeper), deeper);
    // Under `false` the floor is the tag's indent, and a body already raised
    // stays raised: nothing is ever lowered.
    assert_eq!(
        tag_with(flush, FLUSH),
        concat!(
            "<cfoutput>\n    <script>\n",
            "    var a = 1; <!--- c --->\n    if (a) {\n      b();\n    }\n",
            "    </script>\n</cfoutput>\n"
        )
    );
    assert_eq!(tag_with(raised, FLUSH), raised);
    // A body on the tag's own line stays there.
    let inline = "<cfoutput>\n    <script>var a = 1; <!--- c ---></script>\n</cfoutput>\n";
    assert_eq!(tag(inline), inline);
    // A `<cfquery>` and a `<cfjava>` body have the same floor.
    let query = "<div>\n<cfquery name=\"q\">\nSELECT 1\n</cfquery>\n</div>\n";
    let query_raised =
        "<div>\n    <cfquery name=\"q\">\n        SELECT 1\n    </cfquery>\n</div>\n";
    assert_eq!(tag(query), query_raised);
    assert_eq!(
        tag_with(query, FLUSH),
        "<div>\n    <cfquery name=\"q\">\n    SELECT 1\n    </cfquery>\n</div>\n"
    );
    assert_eq!(tag_with(query_raised, FLUSH), query_raised);
    assert_eq!(
        tag("<div>\n<cfjava handle=\"h\">\npublic class A {\n  int b;\n}\n</cfjava>\n</div>\n"),
        concat!(
            "<div>\n    <cfjava handle=\"h\">\n",
            "        public class A {\n          int b;\n        }\n",
            "    </cfjava>\n</div>\n"
        )
    );
}

#[test]
fn islands_indent_round_trips() {
    let src = concat!(
        "<div>\n<cfscript>\nif (a) {\nb = 1;\n}\n</cfscript>\n",
        "<script>\nvar t = `x\n  y`;\nif (t) {go()}\n</script>\n",
        "<style>\n.a{color:red}\n</style>\n</div>\n"
    );
    let indented = concat!(
        "<div>\n    <cfscript>\n        if (a) {\n            b = 1;\n        }\n    </cfscript>\n",
        "    <script>\n        var t = `x\n  y`;\n        if (t) {\n            go();\n        }\n    </script>\n",
        "    <style>\n        .a {\n            color: red;\n        }\n    </style>\n</div>\n"
    );
    let flush = concat!(
        "<div>\n    <cfscript>\n    if (a) {\n        b = 1;\n    }\n    </cfscript>\n",
        "    <script>\n    var t = `x\n  y`;\n    if (t) {\n        go();\n    }\n    </script>\n",
        "    <style>\n    .a {\n        color: red;\n    }\n    </style>\n</div>\n"
    );
    assert_eq!(tag(src), indented);
    // A template literal's lines keep their source columns either way.
    assert_eq!(tag_with(indented, FLUSH), flush);
    assert_eq!(tag(flush), indented);
    // An empty `<cfscript>` is the two tags on two lines either way.
    for settings in [r#"{"newline": "\n"}"#, FLUSH] {
        assert_eq!(
            tag_with("<cfscript></cfscript>\n", settings),
            "<cfscript>\n</cfscript>\n"
        );
    }
}

#[test]
fn a_cfscript_in_a_code_fence_follows_islands_indent() {
    // The fence's tags are laid out by the doc printer at the fence's own
    // indentation, so a `<cfscript>` body there indents like any other.
    let src = "{\n```\n<cfif a>\n<cfscript>\nx = 1;\n</cfscript>\n</cfif>\n```\n}";
    assert_eq!(
        fmt(src),
        concat!(
            "{\n    ```\n    <cfif a>\n        <cfscript>\n            x = 1;\n",
            "        </cfscript>\n    </cfif>\n    ```\n}\n"
        )
    );
    assert_eq!(
        fmt_with(src, FLUSH),
        concat!(
            "{\n    ```\n    <cfif a>\n        <cfscript>\n        x = 1;\n",
            "        </cfscript>\n    </cfif>\n    ```\n}\n"
        )
    );
}

#[test]
fn tag_body_lines_lose_their_edges_and_cap_blank_lines() {
    assert_eq!(
        tag("<div>\n\n\n  a  \n\n\n\n  b\n\n</div>\n"),
        "<div>\n    a\n\n    b\n</div>\n"
    );
    // A `ws` run between two inline nodes is one space.
    assert_eq!(
        tag("<cfoutput>\n#a#   #b#\n</cfoutput>\n"),
        "<cfoutput>\n    #a# #b#\n</cfoutput>\n"
    );
    // An inline tag keeps the space that follows it.
    assert_eq!(
        tag("<p>\na <span>b</span> c\n</p>\n"),
        "<p>\n    a <span>b</span> c\n</p>\n"
    );
}

#[test]
fn cfelse_and_cfelseif_are_dedented() {
    assert_eq!(
        tag("<cfif x>\n<cfset y = 1>\n<cfelseif z>\n<cfset y = 2>\n<cfelse>\n<cfset y = 3>\n</cfif>\n"),
        "<cfif x>\n    <cfset y = 1>\n<cfelseif z>\n    <cfset y = 2>\n<cfelse>\n    <cfset y = 3>\n</cfif>\n"
    );
    // An empty clause keeps its tags on their own lines.
    assert_eq!(
        tag("<cfif x>\n<cfelse>\n<cfset y = 1>\n</cfif>\n"),
        "<cfif x>\n<cfelse>\n    <cfset y = 1>\n</cfif>\n"
    );
}

#[test]
fn tag_attributes_break_one_per_line_with_the_delimiter_on_its_own() {
    assert_eq!(
        tag_with(
            "<cfloop from=\"1\" to=\"10\" index=\"i\">x</cfloop>\n",
            r#"{"newline": "\n", "max_columns": 30}"#
        ),
        "<cfloop\n    from=\"1\"\n    to=\"10\"\n    index=\"i\"\n>x</cfloop>\n"
    );
    // A tag inside an attribute list is an entry of its own.
    assert_eq!(
        tag_with(
            "<div <cfoutput>#a#</cfoutput> class=\"test\">x</div>\n",
            r#"{"newline": "\n", "max_columns": 20}"#
        ),
        "<div\n    <cfoutput>#a#</cfoutput>\n    class=\"test\"\n>x</div>\n"
    );
    // A node glued to an unquoted value stays glued.
    assert_eq!(
        tag("<cfhttp url=www.#a#></cfhttp>\n"),
        "<cfhttp url=www.#a#></cfhttp>\n"
    );
    // A long condition breaks inside the tag, as script does, and `>`
    // takes a line of its own.
    assert_eq!(
        tag_with(
            "<cfif aaaaaaaaaa and bbbbbbbbbb>x</cfif>\n",
            r#"{"newline": "\n", "max_columns": 20}"#
        ),
        "<cfif aaaaaaaaaa and\n    bbbbbbbbbb\n>x</cfif>\n"
    );
}

#[test]
fn script_tag_delimiter_on_its_own_unless_a_bracket_closes() {
    let narrow = r#"{"newline": "\n", "max_columns": 40}"#;
    // A broken binary, ternary or string value: `>` on its own line.
    assert_eq!(
        tag_with(
            "<cfset s &= aaaaaaaaaa & bbbbbbbbbb & cccccccccc & dd>\n",
            narrow
        ),
        "<cfset s &=\n    aaaaaaaaaa &\n    bbbbbbbbbb &\n    cccccccccc &\n    dd\n>\n"
    );
    assert_eq!(
        tag_with(
            "<cfreturn aaaaaaaaaaaaaa ? bbbbbbbbbbbbbb : cccccccccccc>\n",
            narrow
        ),
        "<cfreturn aaaaaaaaaaaaaa\n    ? bbbbbbbbbbbbbb\n    : cccccccccccc\n>\n"
    );
    assert_eq!(
        tag_with("<cfset variables.value = 'a long string value'>\n", narrow),
        "<cfset variables.value =\n    'a long string value'\n>\n"
    );
    // A call, struct, index, `not` call or whole parenthesised condition
    // ends on its own bracket, and `>` follows it.
    assert_eq!(
        tag_with(
            "<cfset x = foo(aaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbb)>\n",
            narrow
        ),
        "<cfset x = foo(\n    aaaaaaaaaaaaaa,\n    bbbbbbbbbbbbbbbbbb\n)>\n"
    );
    assert_eq!(
        tag_with(
            "<cfset x = { aaaaaaaaaa: 1, bbbbbbbbbbbbbbbbbbbb: 2 }>\n",
            narrow
        ),
        "<cfset x = {\n    aaaaaaaaaa: 1,\n    bbbbbbbbbbbbbbbbbbbb: 2\n}>\n"
    );
    assert_eq!(
        tag_with(
            "<cfif not reFind(aaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbb)>x</cfif>\n",
            narrow
        ),
        "<cfif not reFind(\n    aaaaaaaaaaaaaa,\n    bbbbbbbbbbbbbbbbbb\n)>x</cfif>\n"
    );
    assert_eq!(
        tag_with(
            "<cfif (aaaaaaaaaaaaaa and bbbbbbbbbbbbbbbbbb)>x</cfif>\n",
            narrow
        ),
        "<cfif (\n    aaaaaaaaaaaaaa and\n    bbbbbbbbbbbbbbbbbb\n)>x</cfif>\n"
    );
    // A bracketed value that moved after the operator is not closing on
    // the tag's first line.
    assert_eq!(
        tag_with("<cfset variables.value = service.get(id)>\n", narrow),
        "<cfset variables.value =\n    service.get(id)\n>\n"
    );
    // A tag that fits is unchanged.
    assert_eq!(tag("<cfset s &= a & b>\n"), "<cfset s &= a & b>\n");
}

#[test]
fn cf_tags_in_style_and_event_values_print_as_in_an_island() {
    // Flat whatever the width, the body's text as written: the space in
    // the JS string is output.
    assert_eq!(
        tag_with(
            "<b onclick=\"alert('<cfif x eq \"b\"> b</cfif>')\">x</b>\n",
            r#"{"newline": "\n", "max_columns": 20}"#
        ),
        "<b\n    onclick=\"alert('<cfif x eq 'b'> b</cfif>')\"\n>x</b>\n"
    );
    assert_eq!(
        tag_with(
            "<td style=\"a;<cfif x eq \"b\"> c;</cfif>\">x</td>\n",
            r#"{"newline": "\n", "max_columns": 20}"#
        ),
        "<td\n    style=\"a;<cfif x eq 'b'> c;</cfif>\"\n>x</td>\n"
    );
}

#[test]
fn tags_lowercase_and_tab_indent() {
    assert_eq!(
        tag_with(
            "<CFIF a>\n<DIV></DIV>\n</CFIF>\n",
            r#"{"newline": "\n", "tags.lowercase": false}"#
        ),
        "<CFIF a>\n    <DIV></DIV>\n</CFIF>\n"
    );
    // Attribute names are never cased.
    assert_eq!(
        tag("<CFHTTP URL=\"x\"></CFHTTP>\n"),
        "<cfhttp URL=\"x\"></cfhttp>\n"
    );
    assert_eq!(
        tag_with(
            "<div>\n<p>x</p>\n</div>\n",
            r#"{"newline": "\n", "tab_indent": true}"#
        ),
        "<div>\n\t<p>x</p>\n</div>\n"
    );
}

#[test]
fn tag_comments() {
    // The dashed banner keeps its dashes and never breaks.
    assert_eq!(
        tag_with(
            "<!--------- HEADER --------->\n",
            r#"{"newline": "\n", "max_columns": 10}"#
        ),
        "<!--------- HEADER --------->\n"
    );
    // A nested comment is a child element, re-indented with the outer one.
    assert_eq!(
        tag("<!--- <!--- nested ---> --->\n"),
        "<!--- <!--- nested ---> --->\n"
    );
    // A comment at depth 2 that does not fit breaks into three lines.
    assert_eq!(
        tag_with(
            "<div>\n<p>\n<!--- a fairly long comment --->\n</p>\n</div>\n",
            r#"{"newline": "\n", "max_columns": 30}"#
        ),
        "<div>\n    <p>\n        <!---\n            a fairly long comment\n        --->\n    </p>\n</div>\n"
    );
}

#[test]
fn a_tag_comment_outside_tag_mode_stays_verbatim() {
    // A `<!--- --->` banner at the top of a script file (Lucee's gateway
    // components) is verbatim text, not a tag comment: the tag printer is not
    // the enclosing one.
    let banner = "/*\n * a\n *\n */\nx = 1;\n"
        .replace("/*", "<!---")
        .replace(" */", " --->");
    assert_eq!(fmt(&banner), banner);
    // Inside a `<cfscript>` body too: its first line at the body's indent,
    // the others as written.
    let script = "<cfscript>\n    <!---\n * a\n *\n --->\n    x = 1;\n</cfscript>\n";
    assert_eq!(tag(script), script);
    // In a tag body it is a tag comment, and re-flows.
    assert_eq!(
        tag("<div>\n<!---\n   a\n   b\n--->\n</div>\n"),
        "<div>\n    <!---\n        a\n        b\n    --->\n</div>\n"
    );
}

#[test]
fn islands_keep_their_shape() {
    // With `islands.*` "off", a `<script>` at depth 2 with column-0 JS moves
    // right as a whole: the least-indented line lands one level inside the
    // tag and `b();` stays two columns deeper than `if`.
    assert_eq!(
        tag_with(
            "<div>\n<div>\n<script>\nvar a = 1;\nif (a) {\n  b();\n}\n</script>\n</div>\n</div>\n",
            ISLANDS_OFF
        ),
        concat!(
            "<div>\n    <div>\n        <script>\n",
            "            var a = 1;\n            if (a) {\n              b();\n            }\n",
            "        </script>\n    </div>\n</div>\n"
        )
    );
    // Lines partly under that indent shift by the same amount.
    assert_eq!(
        tag_with(
            "<div>\n<p>\n<script>\nlet a = 1;\n  if (a) {\n            b();\n  }\n</script>\n</p>\n</div>\n",
            ISLANDS_OFF
        ),
        "<div>\n    <p>\n        <script>\n            let a = 1;\n              if (a) {\n                        b();\n              }\n        </script>\n    </p>\n</div>\n"
    );
    // A `<style>` is verbatim too (CommandBox re-indents it).
    assert_eq!(
        tag_with(
            "<div>\n<style>\n.a { color: red; }\n</style>\n</div>\n",
            ISLANDS_OFF
        ),
        "<div>\n    <style>\n        .a { color: red; }\n    </style>\n</div>\n"
    );
    // An empty island keeps the two tags together.
    assert_eq!(tag("<script></script>\n"), "<script></script>\n");
    assert_eq!(tag("<script>\n</script>\n"), "<script>\n</script>\n");
    // Tabs inside SQL count as `indent_size` columns: at depth 0 the
    // least-indented line moves one level in and the other keeps its tab.
    assert_eq!(
        tag_with(
            "<cfquery name=\"q\">\n\tSELECT 1\nFROM t\n</cfquery>\n",
            r#"{"newline": "\n", "tab_indent": true}"#
        ),
        "<cfquery name=\"q\">\n\t\tSELECT 1\n\tFROM t\n</cfquery>\n"
    );
}

#[test]
fn an_island_already_at_or_beyond_its_tag_indent_is_untouched() {
    // SQL at 10 columns under a tag at 4: `min` is beyond `least`, no shift.
    let src = concat!(
        "<cffunction name=\"get\">\n    <cfquery name=\"q\">\n",
        "          SELECT a\n            FROM t\n    </cfquery>\n</cffunction>\n"
    );
    assert_eq!(tag(src), src);
}

#[test]
fn an_island_shifts_with_tabs_and_a_space_remainder() {
    // `least` is 8 (two tabs), `min` is 2: every line moves 6 columns, so 8
    // columns print as two tabs and 10 as two tabs plus two spaces.
    assert_eq!(
        tag_with(
            "<div>\n<cfquery name=\"q\">\n  SELECT a\n    FROM t\n</cfquery>\n</div>\n",
            r#"{"newline": "\n", "tab_indent": true}"#
        ),
        "<div>\n\t<cfquery name=\"q\">\n\t\tSELECT a\n\t\t  FROM t\n\t</cfquery>\n</div>\n"
    );
}

#[test]
fn an_island_s_first_line_does_not_count_toward_its_shift() {
    // The first line continues the opening tag's line and keeps its source
    // indentation; `min` is measured over the lines after it.
    assert_eq!(
        tag("<div>\n<cfquery name=\"q\">        SELECT a\nFROM t\n  WHERE b\n</cfquery>\n</div>\n"),
        "<div>\n    <cfquery name=\"q\">        SELECT a\n        FROM t\n          WHERE b\n    </cfquery>\n</div>\n"
    );
}

#[test]
fn a_query_inside_a_function_keeps_its_sql() {
    // `min` is 0 (`SELECT`, `WHERE`) and `least` 12: `FROM` moves from 12 to
    // 24 and a tag inside the SQL counts like any other line.
    assert_eq!(
        tag(concat!(
            "<cffunction name=\"get\">\n<cfif x>\n<cfquery name=\"q\">\n",
            "SELECT a\n            FROM t\nWHERE b = #c# <cfqueryparam value=\"#d#\">\n",
            "</cfquery>\n</cfif>\n</cffunction>\n"
        )),
        concat!(
            "<cffunction name=\"get\">\n    <cfif x>\n        <cfquery name=\"q\">\n",
            "            SELECT a\n                        FROM t\n",
            "            WHERE b = #c# <cfqueryparam value=\"#d#\">\n",
            "        </cfquery>\n    </cfif>\n</cffunction>\n"
        )
    );
}

#[test]
fn a_cfscript_body_is_a_statement_list_one_level_in() {
    assert_eq!(
        tag("<div>\n<cfscript>\nx = 1;\n\ny = 2;\n</cfscript>\n</div>\n"),
        "<div>\n    <cfscript>\n        x = 1;\n\n        y = 2;\n    </cfscript>\n</div>\n"
    );
    // The block-comment rule of `a_block_comment_keeps_the_line_breaks_after_it`
    // holds inside a `<cfscript>` body too.
    assert_eq!(
        tag("<cfscript>\n/* c */\n\n's';\n</cfscript>\n"),
        "<cfscript>\n    /* c */\n\n    's';\n</cfscript>\n"
    );
}

#[test]
fn a_code_fence_prints_its_tags_at_the_fence_indent() {
    assert_eq!(
        fmt("{\n```\n<cfset a=1>\n```\nb=2;\n}"),
        "{\n    ```\n    <cfset a = 1>\n    ```\n    b = 2;\n}\n"
    );
    assert_eq!(fmt("{\n```\n```\n}"), "{\n    ```\n    ```\n}\n");
    // An island inside a fence keeps its source columns exactly: the fence's
    // own column is not knowable at doc-build time, so `least` is 0 there and
    // nothing shifts.
    assert_eq!(
        fmt("{\n```\n<cfquery name=\"q\">\nSELECT a\n  FROM t\n</cfquery>\n```\n}"),
        "{\n    ```\n    <cfquery name=\"q\">\nSELECT a\n  FROM t\n    </cfquery>\n    ```\n}\n"
    );
}

// In-process island formatting (oxc), every platform
// ---------------------------------------------------------------------------

const OXC: &str = r#"{"newline": "\n", "islands.js": "oxc", "islands.css": "oxc", "islands.json": "oxc", "islands.config": "off"}"#;

#[test]
fn oxc_formats_a_pure_island_one_level_inside_the_tag() {
    assert_eq!(
        tag_with(
            "<div>\n<script>\nvar x = {a:1}\nif(x){go()}\n</script>\n<style>\n.a{color:red}\n</style>\n</div>\n",
            OXC
        ),
        concat!(
            "<div>\n    <script>\n        var x = { a: 1 };\n        if (x) {\n            go();\n        }\n",
            "    </script>\n    <style>\n        .a {\n            color: red;\n        }\n    </style>\n</div>\n"
        )
    );
}

#[test]
fn oxc_leaves_a_body_that_is_not_one_island_as_written() {
    // `-->` on its own line after a `<!--` is not JavaScript the island can
    // hold: it is a `text` token outside the island, so the body is not an
    // island and trivia, and prints as written (`scriptIslandLeadingTag`).
    let src = "<div>\n<script>\n<!--\nlegacy();\n-->\n</script>\n</div>\n";
    let verbatim =
        "<div>\n    <script>\n        <!--\n        legacy();\n        -->\n    </script>\n</div>\n";
    assert_eq!(tag_with(src, OXC), verbatim);
    assert_eq!(tag(src), verbatim);
    // `//-->` is a line comment inside the island: formatted.
    assert_eq!(
        tag_with("<script>\n<!--\nlegacy()\n//-->\n</script>\n", OXC),
        "<script>\n    <!--\n    legacy();\n    //-->\n</script>\n"
    );
}

#[test]
fn oxc_refusal_prints_the_island_verbatim_with_a_warning() {
    let (opts, _) = Options::from_json(OXC).unwrap();
    let islands = Islands::new();
    let ctx = FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    let out = format_with(
        "<div>\n<script>\nSYNTAX ERROR\n</script>\n</div>\n",
        Mode::Auto,
        &opts,
        &ctx,
    );
    assert_eq!(
        out.text,
        "<div>\n    <script>\n        SYNTAX ERROR\n    </script>\n</div>\n"
    );
    let warnings: Vec<String> = out.warnings.iter().map(|w| w.to_string()).collect();
    assert_eq!(
        warnings,
        ["<stdin>:3: islands.js: Expected a semicolon or an implicit semicolon after a statement, but found none"]
    );
    assert_eq!((out.islands.formatted, out.islands.warnings), (1, 1));
}

/// A formatter whose output does not parse: the literal walk over it
/// fails, and the island prints as written with a warning rather than the
/// output with every line re-indented.
#[test]
fn a_failed_literal_walk_prints_the_island_verbatim_with_a_warning() {
    use std::sync::Arc;

    use cfformat::islands::{IslandFormatter, IslandRequest, Refused};

    struct Unparsable;

    impl IslandFormatter for Unparsable {
        fn format(&self, _: &IslandRequest) -> Result<String, Refused> {
            Ok("SYNTAX ERROR\n".into())
        }
    }

    let (opts, _) = Options::from_json(OXC).unwrap();
    let islands = Islands::with_formatter(Arc::new(Unparsable));
    let ctx = FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    let out = format_with(
        "<div>\n<script>\nvar a = 1\n</script>\n</div>\n",
        Mode::Auto,
        &opts,
        &ctx,
    );
    assert_eq!(
        out.text,
        "<div>\n    <script>\n        var a = 1\n    </script>\n</div>\n"
    );
    let warnings: Vec<String> = out.warnings.iter().map(|w| w.to_string()).collect();
    assert_eq!(
        warnings,
        ["<stdin>:3: islands.js: internal error: the formatted text does not parse"]
    );
    assert_eq!((out.islands.formatted, out.islands.warnings), (1, 1));
}

#[test]
fn oxc_leaves_the_islands_the_hand_off_does_not_take_alone() {
    let (opts, _) = Options::from_json(OXC).unwrap();
    let verbatim = Options::from_json(ISLANDS_OFF).unwrap().0;
    for src in [
        // An empty island, a blank one.
        "<script></script>\n",
        "<script>\n\n</script>\n",
        // A type no formatter takes; an island holding a CF tag or a tag
        // comment; in `<cfoutput>`, a JSON island holding `#x#` or `##`, a
        // body that is only `#x#`, a `#x#` that prints over lines.
        "<script type=\"text/template\">\n<b>x</b>\n</script>\n",
        "<script>\nvar a;\n<cfif x>b();</cfif>\n</script>\n",
        "<cfoutput><script>\nvar a;   <!--- c --->\n</script></cfoutput>\n",
        "<cfoutput><script type=\"application/json\">\n{\"a\":#x#}\n</script></cfoutput>\n",
        "<cfoutput><script type=\"application/json\">\n{\"a\":\"##\"}\n</script></cfoutput>\n",
        "<cfoutput><style>#x#</style><style>\n  #x#  \n</style></cfoutput>\n",
        "<cfoutput><script>\nvar f =   #function() { return 1; }#;\n</script></cfoutput>\n",
        "<cfoutput><script>\nvar a =   #f( 1, // c\n2 )#;\n</script></cfoutput>\n",
        // SQL and an event attribute are never handed off.
        "<cfquery name=\"q\">\nSELECT 1\n</cfquery>\n",
        "<a onclick=\"go( 1 )\">x</a>\n",
    ] {
        assert_eq!(
            format_source(src, Mode::Auto, &opts),
            format_source(src, Mode::Auto, &verbatim),
            "{src:?}"
        );
    }
    // Inside a code fence the island's column is unknown: verbatim.
    let fence = "{\n```\n<script>\nvar a;\n</script>\n```\n}";
    assert_eq!(
        format_source(fence, Mode::Script, &opts),
        "{\n    ```\n    <script>\nvar a;\n    </script>\n    ```\n}\n"
    );
    // `islands: None` (`--no-islands`) prints verbatim whatever the options
    // say, raised to the body's indent.
    let src = "<style>\n.a{color:red}\n</style>\n";
    let out = format_with(src, Mode::Auto, &opts, &FormatCtx::default());
    assert_eq!(out.text, "<style>\n    .a{color:red}\n</style>\n");
}

/// A `<script>` / `<style>` in `<cfoutput>` holding only text, `##` and
/// `#…#` is formatted: `##` handed off as `#`, each `#…#` as a placeholder
/// as wide as it prints, and put back. Each group checks its second run.
#[test]
fn oxc_formats_an_island_holding_hashes() {
    let fmt = |src: &str| tag_with(src, OXC);
    // `##` alone; the same script outside `<cfoutput>` is pure.
    assert_eq!(
        fmt("<cfoutput><script>\n$( \"##a\" ).focus()\n</script></cfoutput>\n<script>\n$( \"#a\" ).focus()\n</script>\n"),
        concat!(
            "<cfoutput>\n    <script>\n        $(\"##a\").focus();\n    </script>\n</cfoutput>\n",
            "<script>\n    $(\"#a\").focus();\n</script>\n"
        )
    );
    // In strings, which keep their quote; in a template literal, comments
    // and a regular expression.
    assert_eq!(
        fmt(concat!(
            "<cfoutput><script>\n",
            "var a = '#x#', b = \"#y#\", c = 'it\\'s #z#', d = '#f( \"q\" & 'r' )#', e = \"#p# #q#\"\n",
            "var t = `a #x#\n  b`; // #c#\n/* #d# */ var r = /^#re#$/\n",
            "</script></cfoutput>\n"
        )),
        concat!(
            "<cfoutput>\n    <script>\n",
            "        var a = '#x#',\n            b = \"#y#\",\n            c = 'it\\'s #z#',\n",
            "            d = '#f('q' & 'r')#',\n            e = \"#p# #q#\";\n",
            "        var t = `a #x#\n  b`; // #c#\n        /* #d# */ var r = /^#re#$/;\n",
            "    </script>\n</cfoutput>\n"
        )
    );
    // In code: a value, the last property, an argument, a statement, part
    // of a name, a key, two together.
    assert_eq!(
        fmt(concat!(
            "<cfoutput><script>\n",
            "var o = {\n  a : #x#,\n  b : #y#\n}\nshow( #n# );\n#stmt()#\n",
            "function #name#Callback() {}\nvar k = { #key#: 1 }, ab = #a##b#\n",
            "</script></cfoutput>\n"
        )),
        concat!(
            "<cfoutput>\n    <script>\n",
            "        var o = {\n            a: #x#,\n            b: #y#,\n        };\n",
            "        show(#n#);\n        #stmt()#;\n        function #name#Callback() {}\n",
            "        var k = { #key#: 1 },\n            ab = #a##b#;\n",
            "    </script>\n</cfoutput>\n"
        )
    );
    // A `#…#` is measured as it prints: `#   f( a )   #` is `#f(a)#`, so
    // this line fits at 112 columns (its source would not).
    let line = format!(
        "var fits = computeTheResult(firstArgument, {}, #f(a)#);",
        "s".repeat(112 - 53)
    );
    assert_eq!(line.len(), 112);
    assert_eq!(
        fmt(&format!(
            "<cfoutput><script>\n{}\n</script></cfoutput>\n",
            line.replace("#f(a)#", "#   f( a )   #")
        )),
        format!("<cfoutput>\n    <script>\n        {line}\n    </script>\n</cfoutput>\n")
    );
    // CSS: `##` colours and selectors; a value, a `url(…)`, a string, and
    // `#…#` glued to a unit, a class and a property name.
    assert_eq!(
        fmt(concat!(
            "<cfoutput><style>\n##main{color:##FFF}\n",
            ".#c#{width:#w#px;background:url(#u#);content:\"#l#\";margin-#s#:#m#}\n",
            "</style></cfoutput>\n"
        )),
        concat!(
            "<cfoutput>\n    <style>\n",
            "        ##main {\n            color: ##fff;\n        }\n",
            "        .#c# {\n            width: #w#px;\n            background: url(#u#);\n",
            "            content: \"#l#\";\n            margin-#s#: #m#;\n        }\n",
            "    </style>\n</cfoutput>\n"
        )
    );
}

/// What the island formatter of a refusal test returns: its input with
/// `edit` applied, the edit standing for a formatter that moved a
/// placeholder.
struct Edit(fn(&str) -> String);

impl cfformat::islands::IslandFormatter for Edit {
    fn format(
        &self,
        req: &cfformat::islands::IslandRequest,
    ) -> Result<String, cfformat::islands::Refused> {
        Ok((self.0)(req.text))
    }
}

/// `src` formatted with `islands` (oxc when `None`): the text, the
/// warnings and the island counters (runs, warnings).
fn with_islands(src: &str, edit: Option<Edit>) -> (String, Vec<String>, (usize, usize)) {
    let (opts, _) = Options::from_json(OXC).unwrap();
    let islands = match edit {
        Some(edit) => Islands::with_formatter(std::sync::Arc::new(edit)),
        None => Islands::new(),
    };
    let ctx = FormatCtx {
        path: None,
        islands: Some(&islands),
    };
    let out = format_with(src, Mode::Auto, &opts, &ctx);
    let warnings = out.warnings.iter().map(|w| w.to_string()).collect();
    let run = islands.stats();
    assert_eq!(
        (out.islands.formatted, out.islands.warnings),
        (run.formatted, run.warnings),
        "the file's counters and the run's"
    );
    (out.text, warnings, (run.formatted, run.warnings))
}

/// An island whose `#…#` do not come back as they went prints as written
/// (raised to the floor, as any verbatim island), with a warning naming the
/// check and the `#…#`'s line, counted as an island warning; so does one
/// oxc refuses. The output is stable.
#[test]
fn an_island_whose_holes_do_not_come_back_is_refused_with_a_warning() {
    let refused = |src: &str, edit: Option<Edit>| {
        let (text, warnings, counts) = with_islands(src, edit);
        let (opts, _) = Options::from_json(ISLANDS_OFF).unwrap();
        assert_eq!(text, format_source(src, Mode::Auto, &opts), "{src:?}");
        assert_eq!(counts, (1, 1), "{src:?}");
        assert_eq!(warnings.len(), 1, "{src:?}");
        warnings.into_iter().next().unwrap()
    };
    // Through oxc: the parenthesis guard, a comma before `]`, a parse error.
    assert_eq!(
        refused(
            "<cfoutput><script>\n(#f()#).call();\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: what precedes the #…# on line 2 changed from `(` to nothing"
    );
    let long = format!(
        "<cfoutput><script>\nvar ids = [#valueList( q.{} )#];\n</script></cfoutput>\n",
        "a".repeat(100)
    );
    assert_eq!(
        refused(&long, None),
        "<stdin>:2: islands.js: a `,` was added after the #…# on line 2, before `]`"
    );
    // Parentheses oxc drops that the neighbours do not show, a `;` in place
    // of a `)`, a `#…#` newly joined to an operator.
    assert_eq!(
        refused(
            "<cfoutput><script>\nfoo((#a#));\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: the parentheses around the #…# on line 2 changed"
    );
    assert_eq!(
        refused(
            "<cfoutput><script>\nx = (y + #b# + w) || z;\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: the parentheses around the #…# on line 2 changed"
    );
    assert_eq!(
        refused(
            "<cfoutput><script>\nx = (y + #b#);\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: what follows the #…# on line 2 changed from `)` to `;`"
    );
    assert_eq!(
        refused(
            "<cfoutput><script>\nx = - #c#;\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: the #…# on line 2 is now joined to the `-` before it"
    );
    // A statement-level `#…#` after a statement with no `;`: the one oxc
    // adds before it would end a statement the `#…#` may continue.
    assert_eq!(
        refused(
            "<cfoutput><script>\nf()\n#g()#\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: what precedes the #…# on line 3 changed from `)` to `;`"
    );
    assert_eq!(
        refused(
            "<cfoutput><script>\nvar a = #x#;\nvar b = ;\n</script></cfoutput>\n",
            None
        ),
        "<stdin>:2: islands.js: Unexpected token"
    );
    // A formatter that moves its placeholders: dropped, doubled, swapped,
    // into a comment, a neighbour changed, unglued.
    let two = "<cfoutput><script>\nf(#a#,\n#b#);\n</script></cfoutput>\n";
    for (edit, message) in [
        (
            Edit(|t| t.replace("zq0_", "x")),
            "the #…# on line 2 is missing from the formatted text",
        ),
        (
            Edit(|t| t.replace("zq0_", "zq0_, zq0_")),
            "the #…# on line 2 is in the formatted text 2 times",
        ),
        (
            Edit(|t| {
                t.replace("zq0_", "TMP")
                    .replace("zq1_", "zq0_")
                    .replace("TMP", "zq1_")
            }),
            "the #…# on line 3 moved past another #…#",
        ),
        (
            Edit(|t| t.replace("zq1_", "/* zq1_ */ 1")),
            "the #…# on line 3 moved from code into a comment",
        ),
        (
            Edit(|t| t.replace("zq1_)", "zq1_.x)")),
            "what follows the #…# on line 3 changed from `)` to `.`",
        ),
    ] {
        assert_eq!(
            refused(two, Some(edit)),
            format!("<stdin>:2: islands.js: {message}")
        );
    }
    assert_eq!(
        refused(
            "<cfoutput><style>\na { width: #w#px }\n</style></cfoutput>\n",
            Some(Edit(|t| t.replace("zq0_px", "zq0_ px")))
        ),
        "<stdin>:2: islands.css: the #…# on line 2 is no longer joined to the `p` after it"
    );
}

/// A recovered region prints as written wherever it sits: its first
/// line where the printer puts it, every other line as in the source, and
/// everything around it formatted.
#[test]
fn a_recovered_region_prints_as_written() {
    // At the top level.
    assert_eq!(fmt("x=1;\nb = @;\ny=2;\n"), "x = 1;\nb = @;\ny = 2;\n");
    // Inside a function body: the statement's first line takes the body's
    // indent, its second line keeps the source's.
    assert_eq!(
        fmt("function f() {\nx=1;\n  foo(1,\n      @);\ny=2;\n}\n"),
        "function f() {\n    x = 1;\n    foo(1,\n      @);\n    y = 2;\n}\n"
    );
    // Inside a tag body: the `<cfif>` body holding an invalid attribute.
    assert_eq!(
        tag("<div>\n<cfif x>\n<cfparam @ name=\"y\">\n</cfif>\n<p>a</p>\n</div>\n"),
        "<div>\n    <cfif x>\n<cfparam @ name=\"y\">\n</cfif>\n    <p>a</p>\n</div>\n"
    );
    // A region that ends before the source does keeps its trailing
    // whitespace: the group `<cfif (a >` left open holds the space. The
    // body then starts ` b)>`, whose leading space a one-line CF body keeps.
    assert_eq!(tag("<cfif (a > b)>x</cfif>\n"), "<cfif (a > b)>x</cfif>\n");
}

/// An unclosed `#` ends at its tag boundary: the broken statement (or
/// string) prints as written with one warning, and everything after the
/// `</cfscript>` or the `<cfset>`'s `>` is formatted. It used to leave the
/// whole file as written (`<cfscript>`), or — for the `<cfset>` — print the
/// next tag unchanged inside the string, with no warning at all.
#[test]
fn an_unclosed_hash_leaves_the_rest_formatted() {
    let warned = |src: &str| {
        let (opts, _) = Options::from_json(r#"{"newline": "\n"}"#).unwrap();
        let out = format_with(src, Mode::Auto, &opts, &FormatCtx::default());
        let warnings: Vec<_> = out
            .warnings
            .iter()
            .map(|w| format!("{}:{}: {}", w.line, w.label(), w.message))
            .collect();
        assert_eq!(tag(&out.text), out.text, "not idempotent for {src:?}");
        (out.text, warnings)
    };
    for (src, text, warning) in [
        (
            "<cfscript> x = \"price #\"; y=1; </cfscript><p>ok</p>\n",
            "<cfscript>\n    x = \"price #\"; y=1;\n</cfscript>\n<p>ok</p>\n",
            "1:not formatted: an unclosed block",
        ),
        (
            "<cfset x = \"price #\"><cfset y=1>\n",
            "<cfset x = \"price #\"><cfset y = 1>\n",
            "1:not formatted: an unclosed block",
        ),
        (
            "<cfscript> x = #; y=1; </cfscript><p>ok</p>\n",
            "<cfscript>\n    x = #; y=1;\n</cfscript>\n<p>ok</p>\n",
            "1:not formatted: an unmatched run",
        ),
        (
            "<cfscript> x = 1 # 2; y=1; </cfscript><p>ok</p>\n",
            "<cfscript>\n    x = 1 # 2; y=1;\n</cfscript>\n<p>ok</p>\n",
            "1:not formatted: an unmatched run",
        ),
        // The statements before the broken one are formatted too.
        (
            "<cfscript>\nz=2;\nx = \"price #\";\n</cfscript>\n<p class=\"a\">#y#</p>\n",
            "<cfscript>\n    z = 2;\n    x = \"price #\";\n</cfscript>\n<p class=\"a\">#y#</p>\n",
            "3:not formatted: an unclosed block",
        ),
    ] {
        assert_eq!(
            warned(src),
            (text.to_string(), vec![warning.to_string()]),
            "{src:?}"
        );
    }
    // The closed forms format as they did, with no warning.
    for (src, text) in [
        (
            "<cfscript> x = \"price\"; y=1; </cfscript><p>ok</p>\n",
            "<cfscript>\n    x = 'price';\n    y = 1;\n</cfscript>\n<p>ok</p>\n",
        ),
        (
            "<cfscript> x = \"price ##\"; y = \"hi #name#\"; z = \"a#f(\">\")#b\"; // #\n/* # */ w=1; </cfscript><p>ok</p>\n",
            "<cfscript>\n    x = 'price ##';\n    y = 'hi #name#';\n    z = 'a#f('>')#b'; // #\n    /* # */ w = 1;\n</cfscript>\n<p>ok</p>\n",
        ),
        (
            "<cfset x = \"#f(\">\")#\"><cfset y=1>\n",
            "<cfset x = '#f('>')#'><cfset y = 1>\n",
        ),
    ] {
        assert_eq!(warned(src), (text.to_string(), vec![]), "{src:?}");
    }
}

#[test]
fn a_sole_pattern_parameter_with_a_default_hugs_as_prettier_does() {
    // No engine runs a default after a whole pattern, or an array pattern as
    // a parameter, so no golden holds them; both parse and print.
    let narrow = r#"{"newline": "\n", "max_columns": 20}"#;
    // A default that is a name, `{}` or `[]`: the parentheses hug.
    assert_eq!(
        fmt_with("function f({alpha, beta} = {}) {}", narrow),
        "function f({\n    alpha,\n    beta\n} = {}) {\n}\n"
    );
    assert_eq!(
        fmt_with("function f({alpha, beta} = defaults) {}", narrow),
        "function f({\n    alpha,\n    beta\n} = defaults) {\n}\n"
    );
    // Any other default: an ordinary parameter list.
    assert_eq!(
        fmt_with("function f({alpha} = {beta: 1}) {}", narrow),
        "function f(\n    {alpha} = {\n        beta: 1\n    }\n) {\n}\n"
    );
    // An array pattern hugs like a struct pattern.
    assert_eq!(
        fmt_with("function f([alpha, beta]) {}", narrow),
        "function f([\n    alpha,\n    beta\n]) {\n}\n"
    );
    // A comment on the parameter: no hug.
    assert_eq!(
        fmt("function f(/* c */ {a}) {}"),
        "function f(/* c */ {a}) {\n}\n"
    );
    // A comma after it is the list's trailing comma, dropped: the hug does
    // not depend on it, or the first run (no hug, comma dropped) and the
    // second (hug) would differ.
    assert_eq!(
        fmt_with("function f({alpha, beta},) {}", narrow),
        "function f({\n    alpha,\n    beta\n}) {\n}\n"
    );
    assert_eq!(
        fmt_with("k = ({alpha, beta},) => alpha;", narrow),
        "k = ({\n    alpha,\n    beta\n}) => alpha;\n"
    );
}

#[test]
fn a_skipped_element_counts_toward_the_array_threshold() {
    // `[, , c]` is three elements: with `array.multiline.element_count` 2
    // it breaks as `[a, , c]` does.
    let two = r#"{"newline": "\n", "array.multiline.element_count": 2, "array.multiline.min_item_length": 0}"#;
    assert_eq!(
        fmt_with("[, , c] = x;", two),
        "[\n    ,\n    ,\n    c\n] = x;\n"
    );
}
