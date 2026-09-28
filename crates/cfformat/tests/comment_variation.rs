//! Comments at every boundary: each snippet below is formatted with a
//! comment inserted after each of its significant tokens in turn (a string,
//! a `#…#` or a comment counting as one token, so nothing lands inside one)
//! — a block comment (` /* c */ `) and a line comment (` // c` and a
//! newline), and in tag mode a tag comment (` <!--- c ---> `) too. Every
//! variant must format, its output must parse with no recovered region and
//! hold the same number of comments, and formatting the output again must
//! reproduce it byte for byte. A variant where the inserted text is not one
//! more comment (a `//` inside tag markup is text) or where the parse
//! recovers a region is skipped: neither says anything about where a
//! comment goes.
//!
//! Tokens are not compared, as `tests/invariants.rs` does for the goldens:
//! a comment the formatter moves may change how a token around it reads
//! (in `foo // c` newline `(x)` the callee reads as a variable, and as a
//! call once the comment has moved to the end of the line), and the next
//! run keeps what the first printed.
//!
//! The variants format with the default settings, where every snippet fits
//! on its line. At a narrow width they do not all pass: a line comment that
//! prints at the end of its line meets a bracket that breaks first
//! (`foo // c` newline `(a, b)` at 20 columns prints `foo( // c`), and the
//! next run reads it as the first comment inside the brackets and puts it
//! on a line of its own.

mod common;

use cfformat::{format_with, FormatCtx, Options};
use cfparse::{Element, ElementKind, Mode, Node, TokenKind, Tree};

const SCRIPT: &[&str] = &[
    "x = 1;",
    "var y = a + b * c;",
    "x = a ? b : c;",
    "x = (a + b) * c;",
    "x = -a++ + !b;",
    "x = a.b?.c[1];",
    "if (a && b) { x = 1; } else if (c) { x = 2; } else { x = 3; }",
    "for (var i = 1; i <= 10; i++) { total += i; }",
    "for (item in items) { writeOutput(item); }",
    "while (x < 10) { x++; }",
    "do { x--; } while (x > 0);",
    "switch (x) { case 1: y = 2; break; default: y = 3; }",
    "try { foo(); } catch (any e) { bar(e); } finally { baz(); }",
    "foo(1, \"two\", three);",
    "obj.method(a).other(b = 1);",
    "s = { a: 1, \"b\": [1, 2], c: { d: true } };",
    "arr = [1, 2, 3];",
    "f = function(a, b = 2) { return a + b; };",
    "g = (x) => x * 2;",
    "h = (x) => { return x; };",
    "function foo(required string a, numeric b = 1) { return a; }",
    "component extends=\"base\" accessors=\"true\" { property name=\"a\" type=\"string\"; function init() { return this; } }",
    "component { static { x = 1; } public string function bar() output=false { return \"\"; } }",
    "interface { function foo(); }",
    "import foo.bar.*;",
    "x = new Foo(1);",
    "throw(message = \"x\");",
    "param name=\"x\" default=\"1\";",
    "cfhttp(url = \"x\", method = \"get\");",
    "lock name=\"a\" timeout=\"5\" { x = 1; }",
    "return a ?: b;",
    "x = \"a#b#c\" & d;",
    "local.f = arguments.cb(a, b)?.c;",
    "items.each((item, i) => total += item);",
    "x = { a: { b: [1, { c: 2 }] } };",
    "result = foo(bar(1), baz(a = 2, b = 3));",
    "if (a) x = 1; else x = 2;",
    "x = a.b().c().d();",
    "public static function foo() { return; }",
    "x = new foo.Bar(argumentCollection = args);",
    "arr.map(function(x) { return x * 2; }).filter((x) => x > 2);",
    "savecontent variable=\"s\" { writeOutput(\"x\"); }",
    "x = y ?: z ?: w;",
    "x++; --y;",
    "q = queryExecute(\"select 1\", {}, { datasource: \"x\" });",
    "switch (a) { case \"x\": case \"y\": break; }",
    "while (true) break;",
    "a = b = c;",
    "x = !(a && b) || c;",
    "foo(a, function() { return 1; });",
    "try { } catch (e) { rethrow; }",
    "include \"foo.cfm\";",
    "x = a[b][c];",
    "x = a ? b ? c : d : e;",
    "s = { \"a\" = 1, b = function() {} };",
    "return (a + b) / 2;",
];

const TAGS: &[&str] = &[
    "<cfset x = 1>",
    "<cfset s = { a: 1, b: [1, 2] }>",
    "<cfif a eq 1><p>one</p><cfelseif a eq 2>two<cfelse>other</cfif>",
    "<cfoutput query=\"q\">#q.name#</cfoutput>",
    "<cfloop from=\"1\" to=\"10\" index=\"i\"><cfset total += i></cfloop>",
    "<cffunction name=\"foo\" access=\"public\"><cfargument name=\"a\" type=\"string\"><cfreturn a></cffunction>",
    "<cfscript>x = 1; foo(x);</cfscript>",
    "<div class=\"a\"><cfinclude template=\"b.cfm\"></div>",
    "<cfcomponent><cfproperty name=\"a\"><cffunction name=\"b\"></cffunction></cfcomponent>",
    "<cfparam name=\"x\" default=\"#now()#\">",
    "<cfreturn foo(a, b)>",
    "<cfif x gt 1 and y><cfset z = x ? y : 1></cfif>",
    "<cfloop array=\"#arr#\" item=\"x\"><cfoutput>#x#</cfoutput></cfloop>",
    "<cfset x = foo(a = 1, b = [1, 2])>",
    "<cfif a><cfreturn b></cfif>",
    "<cfswitch expression=\"#x#\"><cfcase value=\"1\">one</cfcase><cfdefaultcase>d</cfdefaultcase></cfswitch>",
    "<cftry><cfset a = 1><cfcatch type=\"any\"><cfrethrow></cfcatch></cftry>",
    "<cfscript>function f() { return 1; }</cfscript>",
    "<cfset x = a.b(c).d>",
    "<cfif a and (b or c)>x</cfif>",
];

/// Where a comment may go: after each significant token, a string, a
/// template expression or a comment counting as one token, so nothing is
/// inserted inside a string or a comment.
fn boundaries(tree: &Tree) -> Vec<u32> {
    fn walk(el: &Element, out: &mut Vec<u32>) {
        let atom = el.kind.is_comment()
            || matches!(
                el.kind,
                ElementKind::String { .. } | ElementKind::TemplateExpression
            );
        if atom {
            out.push(el.span.end);
            return;
        }
        for t in el.open.iter().chain(&el.close) {
            out.push(t.span.end);
        }
        for item in &el.items {
            if let Some(t) = &item.separator {
                out.push(t.span.end);
            }
        }
        for n in el.nodes() {
            match n {
                Node::Token(t) => {
                    if !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline) {
                        out.push(t.span.end);
                    }
                }
                Node::Element(e) => walk(e, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(&tree.root, &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

fn options() -> Options {
    Options::from_json(r#"{"newline": "\n"}"#).unwrap().0
}

fn format(src: &str, mode: Mode, opts: &Options) -> String {
    format_with(src, mode, opts, &FormatCtx::default()).text
}

/// Every variant of `snippets` in `mode`: the failures, and how many
/// variants ran.
fn run(snippets: &[&str], mode: Mode, comments: &[&str]) -> (Vec<String>, usize) {
    let opts = options();
    let mut failures = Vec::new();
    let mut ran = 0;
    for snippet in snippets {
        let tree = cfparse::parse_source(snippet, mode);
        assert!(tree.recoveries.is_empty(), "{snippet:?} recovers");
        let base = common::comment_count(&tree.root);
        let source = &tree.source;
        for at in boundaries(&tree) {
            for comment in comments {
                let at = at as usize;
                let src = format!("{}{comment}{}", &source[..at], &source[at..]);
                let parsed = cfparse::parse_source(&src, mode);
                if !parsed.recoveries.is_empty() || common::comment_count(&parsed.root) != base + 1
                {
                    continue;
                }
                ran += 1;
                let out = format(&src, mode, &opts);
                let mut problems = Vec::new();
                let reparsed = cfparse::parse_source(&out, mode);
                if !reparsed.recoveries.is_empty() {
                    problems.push("the output recovers".to_string());
                }
                let count = common::comment_count(&reparsed.root);
                if count != base + 1 {
                    problems.push(format!("{} comments in, {count} out", base + 1));
                }
                let again = format(&out, mode, &opts);
                if again != out {
                    problems.push(format!("not idempotent: second pass {again:?}"));
                }
                if !problems.is_empty() {
                    failures.push(format!(
                        "{src:?}\n  first pass {out:?}\n  {}",
                        problems.join("\n  ")
                    ));
                }
            }
        }
    }
    (failures, ran)
}

fn check(snippets: &[&str], mode: Mode, comments: &[&str]) {
    let (failures, ran) = run(snippets, mode, comments);
    assert!(ran > 0);
    assert!(
        failures.is_empty(),
        "{} of {ran} variants fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn script_comments_at_every_boundary() {
    check(SCRIPT, Mode::Script, &[" /* c */ ", " // c\n"]);
}

#[test]
fn tag_comments_at_every_boundary() {
    check(
        TAGS,
        Mode::Tags,
        &[" /* c */ ", " // c\n", " <!--- c ---> "],
    );
}
