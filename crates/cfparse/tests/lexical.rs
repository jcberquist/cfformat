//! Lexical correctness. Each case is a small source and either the token
//! kinds it must produce (where the rule is lexical) or the text it must
//! format to with the default settings (where the rule is visible).

mod common;

use cfparse::{parse_source, Mode, Tree};

fn parse(src: &str, mode: Mode) -> Tree {
    let tree = parse_source(src, mode);
    common::assert_covers_source(src, &tree);
    tree
}

/// The significant tokens as `Kind text` lines (whitespace and newlines
/// dropped).
fn tokens(src: &str, mode: Mode) -> Vec<String> {
    let tree = parse(src, mode);
    common::flatten(&tree.root)
        .into_iter()
        .map(|t| (format!("{:?}", t.kind), tree.text(t).to_string()))
        .filter(|(k, _)| k != "Whitespace" && k != "Newline")
        .map(|(k, t)| format!("{k} {t}"))
        .collect()
}

/// Formats with the default settings, except that the newline is `"\n"`
/// whatever the platform's; also checks the source tiles.
fn fmt(src: &str, mode: Mode) -> String {
    parse(src, mode);
    let opts = cfformat::Options {
        newline: cfformat::options::NewlineStyle::Lf,
        ..Default::default()
    };
    cfformat::format_source(src, mode, &opts)
}

/// Asserts `src` formats to `want` (a trailing newline is added by the
/// printer and ignored here).
#[track_caller]
fn assert_fmt(src: &str, mode: Mode, want: &str) {
    let got = fmt(src, mode);
    assert_eq!(got.trim_end_matches('\n'), want, "source: {src:?}");
}

// ---------------------------------------------------------------------------
// UTF-8: a multibyte character formats exactly like an ASCII
// placeholder in its place
// ---------------------------------------------------------------------------

#[test]
fn utf8_twins_format_like_their_ascii_twin() {
    // (mode, source with the placeholder, placeholder, the character)
    let cases: &[(Mode, &str, &str, &str)] = &[
        (Mode::Tags, "<script>ZZ</script>", "Z", "é"),
        (Mode::Script, "include \"123456Z.cfm\";", "Z", "é"),
        (Mode::Script, "cffile(x);", "x", "💩"),
        (Mode::Script, "cffile(a, x);", "x", "💩"),
        (Mode::Tags, "<!---Z<!---x--->y--->", "Z", "é"),
        (Mode::Script, "<!---Z<!---x--->y--->", "Z", "é"),
        (Mode::Script, "a=1;\n<!---Z<!---x--->y--->\nb=1;", "Z", "é"),
        (Mode::Script, "a=1;\n<!---Z<!---x--->y--->\nb=1;", "Z", "💩"),
        (Mode::Script, "include \"aZ.cfm\";", "Z", "\u{301}"),
        (Mode::Tags, "<!---aZ<!---x--->y--->", "Z", "\u{301}"),
        (Mode::Tags, "<style>a{content:\"Z\"}</style>", "Z", "é"),
        (
            Mode::Tags,
            "<script>\n  x = 1;\n  Z -->\n</script>",
            "Z",
            "é",
        ),
    ];
    for &(mode, ascii, placeholder, ch) in cases {
        let wide = ascii.replace(placeholder, ch);
        let want = fmt(ascii, mode).replace(placeholder, ch);
        assert_eq!(fmt(&wide, mode), want, "source: {wide:?} (twin {ascii:?})");
    }
}

#[test]
fn utf8_edge_cases() {
    assert_fmt(
        "<script>éé</script>",
        Mode::Tags,
        "<script>\néé;\n</script>",
    );
    assert_fmt(
        "include \"123456é.cfm\";",
        Mode::Script,
        "include '123456é.cfm';",
    );
    assert_fmt("cffile(💩);", Mode::Script, "cffile(💩);");
    // `y--->` is comment text, not script.
    assert_fmt(
        "a=1;\n<!---é<!---x--->y--->\nb=1;",
        Mode::Script,
        "a = 1;\n<!---é<!---x--->y--->\nb = 1;",
    );
}

#[test]
fn recovery_advances_by_a_character() {
    let got = tokens("cffile(💩);", Mode::Script);
    assert!(got.contains(&"Invalid 💩".to_string()), "{got:#?}");
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[test]
fn exponents_are_part_of_the_number() {
    for (src, number) in [
        ("x=1e3;", "1e3"),
        ("x=1.2e-3;", "1.2e-3"),
        ("x=1E+5;", "1E+5"),
        ("x=.5e2;", ".5e2"),
        ("x=1.e3;", "1.e3"),
        ("x=0x1e3;", "0x1e3"),
        ("x=1e;", "1"),
        ("x=1ex;", "1"),
        ("x=1e+;", "1"),
    ] {
        let got = tokens(src, Mode::Script);
        assert_eq!(
            got[2],
            format!("Literal(Number) {number}"),
            "{src}: {got:?}"
        );
    }
    assert_fmt("x=1e3;", Mode::Script, "x = 1e3;");
    assert_fmt("x=1.2e-3;", Mode::Script, "x = 1.2e-3;");
    assert_fmt("<cfset x=1e3>", Mode::Tags, "<cfset x = 1e3>");
}

// ---------------------------------------------------------------------------
// One identifier-boundary rule: `$` is a word character
// ---------------------------------------------------------------------------

#[test]
fn dollar_is_a_word_character() {
    for (src, first) in [
        ("var$foo=1;", "Ident(Variable) var$foo"),
        ("$var = 1;", "Ident(Variable) $var"),
        ("x$ = 1;", "Ident(Variable) x$"),
        ("if$ = 1;", "Ident(Variable) if$"),
        ("return_x = 1;", "Ident(Variable) return_x"),
    ] {
        let got = tokens(src, Mode::Script);
        assert_eq!(got[0], first, "{src}: {got:?}");
        assert_eq!(got[1], "Operator(Assign) =", "{src}: {got:?}");
    }
    let got = tokens("x = a and$;", Mode::Script);
    assert_eq!(got[3], "Ident(Variable) and$", "{got:?}");
    assert_fmt("var$foo=1;", Mode::Script, "var$foo = 1;");
}

// ---------------------------------------------------------------------------
// Keywords are case-insensitive; text is printed as written
// ---------------------------------------------------------------------------

#[test]
fn uppercase_keywords_are_keywords() {
    let got = tokens("IF (x) {RETURN 1;}", Mode::Script);
    assert_eq!(got[0], "Keyword(If) IF", "{got:?}");
    assert!(
        got.contains(&"Keyword(Return) RETURN".to_string()),
        "{got:?}"
    );
    let got = tokens("Var a = TRUE; b = Null;", Mode::Script);
    assert_eq!(got[0], "Keyword(Var) Var", "{got:?}");
    assert!(got.contains(&"Literal(Bool) TRUE".to_string()), "{got:?}");
    assert!(got.contains(&"Literal(Null) Null".to_string()), "{got:?}");
    // A member name and a script-tag attribute name stay names.
    let got = tokens("x.Var = 1;", Mode::Script);
    assert_eq!(got[2], "Ident(PropertyName) Var", "{got:?}");
    let got = tokens("y = obj.Function();", Mode::Script);
    assert_eq!(got[4], "Ident(Call) Function", "{got:?}");
    let got = tokens("http Method=\"get\";", Mode::Script);
    assert_eq!(got[1], "Ident(AttributeName) Method", "{got:?}");

    assert_fmt(
        "IF (x) {RETURN 1;}",
        Mode::Script,
        "IF (x) {\n    RETURN 1;\n}",
    );
    assert_eq!(
        fmt("IF (x) {RETURN 1;}", Mode::Script),
        fmt("if (x) {return 1;}", Mode::Script)
            .replace("if", "IF")
            .replace("return", "RETURN")
    );
    assert_fmt(
        "FUNCTION f() { VAR a = TRUE; }",
        Mode::Script,
        "FUNCTION f() {\n    VAR a = TRUE;\n}",
    );
    assert_fmt("x.Var = 1;", Mode::Script, "x.Var = 1;");
    assert_fmt(
        "<cfscript>IF (x) {y();}</cfscript>",
        Mode::Tags,
        &fmt("<cfscript>if (x) {y();}</cfscript>", Mode::Tags)
            .trim_end()
            .replace("if", "IF"),
    );
}

// ---------------------------------------------------------------------------
// One operator table; the generated case is
// `every_binary_operator_continues_a_statement` in `script/lexer.rs`
// ---------------------------------------------------------------------------

#[test]
fn word_operators_continue_a_statement() {
    for op in [
        "lte",
        "GTE",
        "does not contain",
        "greater than",
        "less than or equal to",
        "equal",
        "not equal",
        "eqv",
        "imp",
        "contains",
    ] {
        assert_fmt(
            &format!("x = a\n{op} b;"),
            Mode::Script,
            &format!("x = a {op} b;"),
        );
    }
    assert_fmt("x = a\nnot b;", Mode::Script, "x = a\nnot b;");
}

/// A multi-word operator and `else if` are one token per word, with the
/// whitespace between the words as whitespace and newline tokens: no token
/// spans a line end. The printer writes the words one space apart.
#[test]
fn phrase_operators_and_else_if_are_word_tokens() {
    for (src, words) in [
        ("x = a less\n    than b;", &["less", "than"][..]),
        (
            "x = a GREATER\n than  OR\tequal to b;",
            &["GREATER", "than", "OR", "equal", "to"],
        ),
        ("x = a is\nnot b;", &["is", "not"]),
    ] {
        let comparisons: Vec<String> = tokens(src, Mode::Script)
            .into_iter()
            .filter_map(|t| {
                t.strip_prefix("Operator(Binary(Comparison)) ")
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(comparisons, words, "{src:?}");
    }
    let tokens_of = |src: &str| {
        let tree = parse(src, Mode::Script);
        common::flatten(&tree.root)
            .into_iter()
            .map(|t| tree.text(t).to_string())
            .collect::<Vec<_>>()
    };
    for src in [
        "x = a less\n    than b;",
        "x = a greater than\nor equal to b;",
        "if (a) {} else\n\n  if (b) {}",
    ] {
        for t in tokens_of(src) {
            assert!(
                !t.trim_end_matches('\n').contains('\n'),
                "{src:?}: {t:?} spans a line end"
            );
        }
    }
    assert_fmt(
        "x = a less\n    than b;",
        Mode::Script,
        "x = a less than b;",
    );
    assert_fmt(
        "<cfif a greater\n than  b>x</cfif>",
        Mode::Tags,
        "<cfif a greater than b>x</cfif>",
    );
    assert_fmt(
        "if (a) {} ELSE\n   IF (b) {}",
        Mode::Script,
        "if (a) {\n} ELSE IF (b) {\n}",
    );
}

// ---------------------------------------------------------------------------
// Custom-tag names take digits
// ---------------------------------------------------------------------------

#[test]
fn custom_tag_names_take_digits() {
    for (src, names) in [
        ("<cf_foo2>x</cf_foo2>", &["cf_foo2", "cf_foo2"][..]),
        ("<cf_a1-b2>x</cf_a1-b2>", &["cf_a1-b2", "cf_a1-b2"]),
        ("<cfx_img2>", &["cfx_img2"]),
        ("<ns:foo2 />", &["ns", "foo2"]),
        ("<ns:foo2>x</ns:foo2>", &["ns", "foo2", "ns:foo2"]),
    ] {
        let got: Vec<String> = tokens(src, Mode::Tags)
            .into_iter()
            .filter_map(|t| t.strip_prefix("Ident(TagName) ").map(str::to_string))
            .collect();
        assert_eq!(got, names, "{src}");
    }
    assert_fmt("<cf_foo2>x</cf_foo2>", Mode::Tags, "<cf_foo2>x</cf_foo2>");
    assert_fmt(
        "<cf_a1-b2>x</cf_a1-b2>",
        Mode::Tags,
        "<cf_a1-b2>x</cf_a1-b2>",
    );
}

// ---------------------------------------------------------------------------
// Delimiters inside strings and comments: the arrow lookahead
// ---------------------------------------------------------------------------

#[test]
fn arrow_lookahead_skips_strings_and_comments() {
    for (src, want) in [
        ("f=(x /* ) */)=>x;", "f = (x /* ) */) => x;"),
        ("f=(x=\")\")=>x;", "f = (x = ')') => x;"),
        ("f=(x=')')=>x;", "f = (x = ')') => x;"),
        ("f=(x=\"#a(\")\")#\")=>x;", "f = (x = '#a(')')#') => x;"),
    ] {
        assert_fmt(src, Mode::Script, want);
        let got = tokens(src, Mode::Script);
        assert!(
            got.contains(&"Keyword(Arrow) =>".to_string()),
            "{src}: {got:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Java bodies: braces inside Java literals do not count
// ---------------------------------------------------------------------------

#[test]
fn java_body_braces_skip_java_literals() {
    for body in [
        "String s = \"}\"; return s;",
        "char c = '{'; return c;",
        "char c = '\\''; String t = \"{\"; return c;",
        "String s = \"\\\"}\"; return s;",
        "String s = \"\"\"\n  }\n  \"\"\"; return s;",
        "/* } */ return 1;",
    ] {
        let src = format!("function f() type=\"java\" {{ {body} }}");
        assert_fmt(&src, Mode::Script, &src);
        // The body is one element: its close is the last `}`.
        let got = tokens(&src, Mode::Script);
        assert_eq!(got.last().unwrap(), "Punct(Close(Brace)) }", "{src}");
        assert_eq!(
            got.iter()
                .filter(|t| t.starts_with("Punct(Close(Brace))"))
                .count(),
            1,
            "{src}: {got:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Closing tags: `</name\s*>`, any case
// ---------------------------------------------------------------------------

#[test]
fn closing_tags_take_whitespace_and_any_case() {
    let plain = fmt("<cfscript>x=1;</cfscript><p>ok</p>", Mode::Tags);
    for src in [
        "<cfscript>x=1;</cfscript ><p>ok</p>",
        "<cfscript>x=1;</CFSCRIPT><p>ok</p>",
        "<cfscript>x=1;</cfscript\n><p>ok</p>",
    ] {
        // The printer drops a closing tag's whitespace (`print/tags.rs`
        // `tag`), so each prints like the plain spelling.
        assert_eq!(fmt(src, Mode::Tags), plain, "{src}");
        let got = tokens(src, Mode::Tags);
        assert!(got.contains(&"Text ok".to_string()), "{src}: {got:?}");
    }
    assert_eq!(
        fmt(
            "<cfquery name=\"q\">select 1</cfquery ><p>ok</p>",
            Mode::Tags
        ),
        fmt(
            "<cfquery name=\"q\">select 1</cfquery><p>ok</p>",
            Mode::Tags
        )
    );
    // A closing CF tag with whitespace is a CF tag, paired with its opener.
    let tree = parse("<cfif a>x</cfif >", Mode::Tags);
    let body = tree.root.children[0].as_element().unwrap();
    assert_eq!(body.kind.name(), "tag-body", "{:?}", body.kind);
    let close = body.children.last().unwrap().as_element().unwrap();
    assert_eq!(close.kind.name(), "cf-tag", "{:?}", close.kind);
}

// ---------------------------------------------------------------------------
// Event and style attribute islands end where an ordinary attribute value
// does
// ---------------------------------------------------------------------------

#[test]
fn attribute_islands_end_outside_hashes() {
    for (attr, value) in [
        ("onclick", "\"#f(\"x\")#\""),
        ("onclick", "'#f('x')#'"),
        ("style", "\"width:#w(\"px\")#\""),
        ("onclick", "\"a##b\""),
    ] {
        let island = format!("<cfoutput><b {attr}={value}></b></cfoutput>");
        let plain = format!("<cfoutput><b title={value}></b></cfoutput>");
        // The island's value is whole: it formats like an ordinary one.
        assert_eq!(
            fmt(&island, Mode::Tags),
            fmt(&plain, Mode::Tags).replace("title", attr),
            "{island}"
        );
    }
    assert_fmt(
        "<cfoutput><button onclick=\"#f(\"x\")#\"></button></cfoutput>",
        Mode::Tags,
        "<cfoutput>\n    <button onclick=\"#f('x')#\"></button>\n</cfoutput>",
    );
    // Outside `<cfoutput>` a `#` is text, for an island as for any value.
    assert_eq!(
        fmt("<b onclick=\"#f(\"x\")#\"></b>", Mode::Tags),
        fmt("<b title=\"#f(\"x\")#\"></b>", Mode::Tags).replace("title", "onclick")
    );
}

/// A CF tag or tag comment in a style or event value is parsed whole, as in
/// any other value: a quote inside it does not end the island (the engines
/// read the `<cfif>` before any HTML).
#[test]
fn attribute_islands_end_outside_cf_tags() {
    for (attr, value) in [
        ("style", "\"a;<cfif x eq \"b\"> c;</cfif>\""),
        ("onclick", "\"a(); <cfif x eq \"b\">c();</cfif>\""),
        ("style", "'a;<cfset y = 'b'>'"),
        ("style", "\"a;<!--- \"c\" --->b\""),
    ] {
        let src = format!("<td {attr}={value}>x</td>");
        let tree = parse(&src, Mode::Tags);
        let body = tree.root.children[0].as_element().unwrap();
        let open = body.children[0].as_element().unwrap();
        let text: Vec<_> = body.children[1..]
            .iter()
            .map(|n| tree.slice(n.span()))
            .collect();
        assert_eq!(text, ["x", "</td>"], "{src}");
        assert_eq!(
            tree.slice(open.span.clone()),
            src.strip_suffix("x</td>").unwrap()
        );
    }
}

// ---------------------------------------------------------------------------
// Component lookahead and consumption agree; an unterminated host
// tag tiles
// ---------------------------------------------------------------------------

#[test]
fn component_modifier_comments_are_trivia() {
    for (src, comment) in [
        ("abstract /* c */ component {}", "Punct(Open(Comment)) /*"),
        ("final\n// c\ncomponent {}", "Punct(Open(Comment)) //"),
    ] {
        for mode in [Mode::Script, Mode::Auto] {
            let got = tokens(src, mode);
            let modifier = src.split(|c: char| !c.is_alphabetic()).next().unwrap();
            assert_eq!(
                got[0],
                format!("Storage(Modifier) {modifier}"),
                "{src}: {got:?}"
            );
            assert_eq!(got[1], comment, "{src}: {got:?}");
            let kw = got
                .iter()
                .position(|t| t.starts_with("Keyword(Component)"))
                .unwrap();
            assert_eq!(got[kw], "Keyword(Component) component", "{src}: {got:?}");
        }
        assert_fmt(src, Mode::Script, fmt(src, Mode::Auto).trim_end());
    }
}

#[test]
fn an_unterminated_host_tag_tiles() {
    for src in [
        "<script>   ",
        "<style>  ",
        "<script>\t",
        "<script type=\"module\">  ",
    ] {
        parse(src, Mode::Tags);
        fmt(src, Mode::Tags);
    }
}

// ---------------------------------------------------------------------------
// Found by the mutation fuzz: a `#…#` whose scanned end lies past a `#` the
// expression stops at still tiles
// ---------------------------------------------------------------------------

#[test]
fn a_template_expression_tiles_its_range() {
    for src in [
        "<cfoutput>#(}#",
        "<cfoutput>#(}#x",
        "<cfoutput>#[}#abc</cfoutput>",
        "<cfoutput><p>#a(}#</p><p>#b#</p></cfoutput>",
    ] {
        parse(src, Mode::Tags);
        fmt(src, Mode::Tags);
    }
}

#[test]
fn an_inline_component_with_an_open_hash_does_not_panic() {
    for src in [
        "x = new component a=\"t#\" {}",
        "x = new component accessors=\"tr#ue\" output=false {\n}",
        "return new Component javaSettings='{#\"maven\":[\"a:b:1\"]}'{\n}",
    ] {
        parse(src, Mode::Script);
        fmt(src, Mode::Script);
    }
    let got = tokens("x = new component a=\"t\" {}", Mode::Script);
    assert!(
        got.contains(&"Keyword(Component) component".to_string()),
        "{got:?}"
    );
}
