//! Shared helpers for the integration tests.
#![allow(dead_code)]

pub mod watchdog;

use std::path::{Path, PathBuf};

use cfparse::{Element, Mode, Node, Token, Tree};

/// The stack of the threads the depth tests parse on. The bound the parser
/// and printer keep (`MAX_DEPTH`) is chosen so that no input overflows the
/// 2 MB a default thread has, and release builds check exactly that. Debug
/// frames are larger, and on Windows large enough that 2 MB overflows at the
/// bound, so debug builds check with headroom instead.
pub const SMALL_STACK: usize = if cfg!(debug_assertions) {
    8 << 20
} else {
    2 << 20
};

/// `commandbox-cfformat` checkout next to the workspace, if present. Only
/// the models corpus (coverage test, bench) is read from there; the fixture
/// sources are vendored under `tests/fixtures`.
pub fn commandbox_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../commandbox-cfformat");
    dir.join("models").is_dir().then_some(dir)
}

/// Vendored copies of `commandbox-cfformat/tests/data/*/source.cfc`
/// (`<name>.cfc`) and `tests/data/exprTests/*.cfc` (`exprTests/<name>.cfc`),
/// and `recoveredOperators.cfc`, written here: operators after a recovered
/// region.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub struct Fixture {
    pub name: String,
    pub source: String,
    pub mode: Mode,
}

/// Every fixture under `tests/fixtures`. Script fixtures (first line `//`)
/// drop that line and parse as `Mode::Script`; tag fixtures parse as
/// `Mode::Auto`.
pub fn fixtures() -> Vec<Fixture> {
    let data = fixtures_dir();
    let mut out = Vec::new();
    for p in cfc_files(&data) {
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        out.push(fixture(name, &p));
    }
    for p in cfc_files(&data.join("exprTests")) {
        let name = format!("exprTests.{}", p.file_stem().unwrap().to_string_lossy());
        out.push(fixture(name, &p));
    }
    out
}

/// Sorted `*.cfc` files directly inside `dir`.
fn cfc_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "cfc"))
        .collect();
    files.sort();
    files
}

fn fixture(name: String, path: &Path) -> Fixture {
    let raw = std::fs::read_to_string(path).unwrap();
    if raw.starts_with("//") {
        let body = raw
            .split_once('\n')
            .map_or("", |(_, rest)| rest)
            .to_string();
        Fixture {
            name,
            source: body,
            mode: Mode::Script,
        }
    } else {
        Fixture {
            name,
            source: raw,
            mode: Mode::Auto,
        }
    }
}

/// All tokens in source order, delimiters and separators included. Inside an
/// item, `leading`, `children`, `trailing` and the separator are ordered by
/// span: `trailing` can hold nodes from both sides of the separator.
pub fn flatten(el: &Element) -> Vec<&Token> {
    let mut out = Vec::new();
    walk(el, &mut out);
    out
}

fn walk<'a>(el: &'a Element, out: &mut Vec<&'a Token>) {
    if let Some(t) = &el.open {
        out.push(t);
    }
    for n in &el.children {
        node(n, out);
    }
    for item in &el.items {
        let mut parts: Vec<(u32, Option<&'a Node>, Option<&'a Token>)> = item
            .nodes()
            .map(|n| (n.span().start, Some(n), None))
            .collect();
        if let Some(t) = &item.separator {
            parts.push((t.span.start, None, Some(t)));
        }
        parts.sort_by_key(|p| p.0);
        for part in parts {
            match part {
                (_, Some(n), _) => node(n, out),
                (_, _, Some(t)) => out.push(t),
                _ => unreachable!(),
            }
        }
    }
    if let Some(t) = &el.close {
        out.push(t);
    }
}

fn node<'a>(n: &'a Node, out: &mut Vec<&'a Token>) {
    match n {
        Node::Token(t) => out.push(t),
        Node::Element(e) => walk(e, out),
    }
}

/// Panics unless the tokens tile the normalised source exactly, and every
/// recovered region lies on token boundaries, in source order, none
/// overlapping the next.
pub fn assert_covers_source(name: &str, tree: &Tree) {
    let mut pos = 0u32;
    let mut boundaries = std::collections::HashSet::new();
    for t in flatten(&tree.root) {
        boundaries.insert(t.span.start);
        boundaries.insert(t.span.end);
        assert_eq!(
            t.span.start,
            pos,
            "{name}: gap or overlap before {:?} (token {:?} {:?})",
            &tree.source[pos as usize..(t.span.start as usize).max(pos as usize)],
            t.kind,
            tree.text(t)
        );
        assert!(t.span.end > t.span.start, "{name}: empty token at {pos}");
        for at in [t.span.start, t.span.end] {
            assert!(
                tree.source.is_char_boundary(at as usize),
                "{name}: token {:?} boundary {at} is inside a character",
                t.kind
            );
        }
        pos = t.span.end;
    }
    assert_eq!(pos as usize, tree.source.len(), "{name}: tokens stop early");
    let mut after = 0;
    for r in &tree.recoveries {
        assert!(
            boundaries.contains(&r.span.start) && boundaries.contains(&r.span.end),
            "{name}: recovery {:?} {:?} is not on token boundaries",
            r.reason,
            r.span
        );
        assert!(
            r.span.start >= after && r.span.start < r.span.end,
            "{name}: recovery {:?} {:?} overlaps the one before or is empty",
            r.reason,
            r.span
        );
        after = r.span.end;
    }
}

/// One pathological input shape: a name, its mode, the source for a
/// size, and the sizes to run.
pub struct Generator {
    pub name: &'static str,
    pub mode: Mode,
    pub source: fn(usize) -> String,
    /// The sizes to run: where each shape first aborted on a 2 MB thread or
    /// in the CLI's 8 MB workers while the parsers had no bound, or the size
    /// that was already bounded; for a bracket island shape, where it first
    /// aborted the CLI's 8 MB worker in release while islands had no limit;
    /// for a statement island shape, 501 levels, one past the limit; for a
    /// masked bracket island shape, 600 levels; for a shape that was
    /// quadratic, a size that took seconds.
    pub sizes: &'static [usize],
}

/// Every input shape that once aborted with a stack overflow, the shapes
/// that were already bounded (kept so they stay so), and the ones that were
/// once quadratic. Then the deep
/// `<script>` / `<style>` bodies: JS, JSON and CSS brackets, each past the
/// island nesting limit (500), so refused before oxc sees it; and the
/// nesting with no bracket to count (unbraced `if` / `while` / `for` /
/// `with` / `do` bodies, JSX, labels, `else if`), whose output would grow
/// with the square of the depth: past the limit too, so each is parsed and
/// refused by its tree. And the CSS and JSON brackets with a closer hidden
/// in a comment or a string at every level, which a byte count would
/// cancel: refused by the lexical count before oxc sees them, where
/// unrefused they format into 2 MB (600) and 216 MB (6,000). The operator,
/// call and member chains are not here: `cfformat`'s `tests/islands_deep.rs`
/// takes them to the size limit.
pub fn generators() -> Vec<Generator> {
    fn g(
        name: &'static str,
        mode: Mode,
        source: fn(usize) -> String,
        sizes: &'static [usize],
    ) -> Generator {
        Generator {
            name,
            mode,
            source,
            sizes,
        }
    }
    use Mode::{Script, Tags};
    vec![
        // Deep CFML: prefix and assignment chains, unbraced bodies, brackets
        // in a `#…#`, nested tags and blocks, labels, fences.
        g(
            "bang",
            Script,
            |n| format!("x={}x;", "!".repeat(n)),
            &[2_000, 20_000],
        ),
        g(
            "assign",
            Script,
            |n| format!("{}0;", "x=".repeat(n)),
            &[2_000, 20_000],
        ),
        g(
            "ifs",
            Script,
            |n| format!("{}x();", "if(x) ".repeat(n)),
            &[5_000, 20_000],
        ),
        g(
            "parens",
            Tags,
            |n| format!("<cfoutput>#{}x{}#</cfoutput>", "(".repeat(n), ")".repeat(n)),
            &[100_000, 400_000],
        ),
        g("cfquery", Tags, |n| "<cfquery>".repeat(n), &[5_000, 20_000]),
        g("cfoutput", Tags, |n| "<cfoutput>".repeat(n), &[20_000]),
        g("cfif", Tags, |n| "<cfif x>".repeat(n), &[200_000]),
        g("braces", Script, |n| "{".repeat(n), &[100_000]),
        g(
            "labels",
            Script,
            |n| format!("{}x();", "a: ".repeat(n)),
            &[200_000],
        ),
        g("fences", Script, |n| "```\n".repeat(n), &[30_000]),
        // More deep CFML: other operator, call and statement nestings, the
        // `#…#` scanners, fragments handed between the front ends.
        g(
            "minus",
            Script,
            |n| format!("x={}c;", "-".repeat(n)),
            &[20_000],
        ),
        g(
            "not",
            Script,
            |n| format!("x={}c;", "not ".repeat(n)),
            &[20_000],
        ),
        g(
            "increment",
            Script,
            |n| format!("x={}c;", "++".repeat(n)),
            &[20_000],
        ),
        g(
            "augassign",
            Script,
            |n| format!("x={}c;", "a+=".repeat(n)),
            &[20_000],
        ),
        g(
            "calls",
            Script,
            |n| format!("x=a{};", "()".repeat(n)),
            &[200_000],
        ),
        g(
            "ternary",
            Script,
            |n| format!("x={}c;", "a?b:".repeat(n)),
            &[20_000],
        ),
        g(
            "grouped bangs",
            Script,
            |n| format!("x={}x;", format!("{}(", "!".repeat(99)).repeat(n)),
            &[100],
        ),
        g("switch", Script, |n| "switch(x){".repeat(n), &[20_000]),
        g(
            "else",
            Script,
            |n| format!("{}x();", "if(x) x(); else ".repeat(n)),
            &[20_000],
        ),
        g(
            "do",
            Script,
            |n| format!("{}x();", "do ".repeat(n)),
            &[20_000],
        ),
        g(
            "while",
            Script,
            |n| format!("{}x();", "while(x) ".repeat(n)),
            &[20_000],
        ),
        g(
            "hash string",
            Tags,
            |n| format!("<cfoutput>#{}x", "f(\"#".repeat(n)),
            &[20_000],
        ),
        g(
            "attribute hash",
            Tags,
            |n| format!("<cfoutput><a onclick=\"#{}x\">", "f('#".repeat(n)),
            &[20_000],
        ),
        g(
            "script hash",
            Tags,
            |n| format!("<cfscript>x=\"#{}x", "f(\"#".repeat(n)),
            &[20_000],
        ),
        g(
            "style cfquery",
            Tags,
            |n| format!("<style>{}", "<cfquery>".repeat(n)),
            &[20_000],
        ),
        g(
            "fragments",
            Tags,
            |n| {
                let script = format!("<cfscript>{}x();</cfscript>", "if(x){".repeat(n));
                format!(
                    "<cfscript>{}```{script}```</cfscript>\n",
                    "if(x){".repeat(n)
                )
            },
            &[99],
        ),
        // Read as script (both fuzz passes parse every case as script too):
        // one line of unterminated statements, then a stray `)` — the
        // case the mutation fuzz once timed out on.
        g(
            "cfif stray paren",
            Tags,
            |n| {
                let mut s = "<cfif x>".repeat(n);
                s.insert(s.len() - 20, ')');
                s
            },
            &[20_000],
        ),
        g(
            "paired cfif",
            Tags,
            |n| format!("{}x{}", "<cfif x>".repeat(n), "</cfif>".repeat(n)),
            &[5_000],
        ),
        // Forms after which `statement` keeps reading into the same
        // statement: each continuation was a recursive call that `depth`
        // did not count (4,000 overflowed a 2 MB thread in release).
        g(
            "abort",
            Tags,
            |n| format!("<cfscript>{}</cfscript>", "abort ".repeat(n)),
            &[4_000],
        ),
        g(
            "cffile",
            Tags,
            |n| {
                format!(
                    "<cfscript>{}</cfscript>",
                    "cffile(action=\"read\") ".repeat(n)
                )
            },
            &[4_000],
        ),
        // Unclosed `#`s that the bounded body and tag-expression scanners
        // (`scan::Bounded`) retry from the next character: quadratic
        // before the memo and the depth bound (four seconds at 40,000),
        // and a comment in each `#…#` that only the budget stops.
        g(
            "bounded retry",
            Tags,
            |n| format!("<cfscript>x={}</cfscript>\n", "#\"#(".repeat(n)),
            &[40_000],
        ),
        g(
            "bounded retry tag",
            Tags,
            |n| format!("<cfset x={}><p>ok</p>\n", "\"#f(\"#".repeat(n)),
            &[40_000],
        ),
        g(
            "bounded budget",
            Tags,
            |n| format!("<cfscript>x=\"{}</cfscript>\n", "#\"//".repeat(n)),
            &[40_000],
        ),
        // Opening tags, then as many closing tags of another name: each
        // closer left unpaired had the body it collected walked again one
        // level out, where the next one collected it, quadratic in the
        // pairing and in the recoveries' file-wide walk (29 s at 20,000).
        // Both layers, and the HTML one inside a CF body.
        g(
            "mismatched closers",
            Tags,
            |n| format!("{}{}", "<cfif a>".repeat(n), "</cfoutput>".repeat(n)),
            &[20_000],
        ),
        g(
            "mismatched html closers",
            Tags,
            |n| format!("{}{}", "<div>".repeat(n), "</span>".repeat(n)),
            &[20_000],
        ),
        g(
            "mismatched html closers in cfoutput",
            Tags,
            |n| {
                format!(
                    "<cfoutput>{}{}</cfoutput>",
                    "<div>".repeat(n),
                    "</span>".repeat(n)
                )
            },
            &[20_000],
        ),
        // Island bodies: JS, JSON and CSS brackets past the nesting limit.
        g(
            "island js (",
            Tags,
            |n| {
                format!(
                    "<script>x = {}1{};</script>\n",
                    "(".repeat(n),
                    ")".repeat(n)
                )
            },
            &[4_875],
        ),
        g(
            "island js [",
            Tags,
            |n| {
                format!(
                    "<script>x = {}1{};</script>\n",
                    "[".repeat(n),
                    "]".repeat(n)
                )
            },
            &[4_875],
        ),
        g(
            "island js {a:",
            Tags,
            |n| {
                format!(
                    "<script>x = {}1{};</script>\n",
                    "{a:".repeat(n),
                    "}".repeat(n)
                )
            },
            &[4_187],
        ),
        g(
            "island json [",
            Tags,
            |n| {
                format!(
                    "<script type=\"application/json\">{}0{}</script>\n",
                    "[".repeat(n),
                    "]".repeat(n)
                )
            },
            &[4_875],
        ),
        g(
            "island json {\"a\":",
            Tags,
            |n| {
                format!(
                    "<script type=\"application/json\">{}0{}</script>\n",
                    "{\"a\":".repeat(n),
                    "}".repeat(n)
                )
            },
            &[4_187],
        ),
        g(
            "island css .a{",
            Tags,
            |n| {
                format!(
                    "<style>{}color:red;{}</style>\n",
                    ".a{".repeat(n),
                    "}".repeat(n)
                )
            },
            &[1_015],
        ),
        g(
            "island css @media",
            Tags,
            |n| {
                format!(
                    "<style>{}.a{{color:red}}{}</style>\n",
                    "@media screen{".repeat(n),
                    "}".repeat(n)
                )
            },
            &[577],
        ),
        // Island bodies with no bracket past 1: 501 levels of tree.
        g(
            "island js if(a)",
            Tags,
            |n| format!("<script>{}x;</script>\n", "if(a)".repeat(n)),
            &[501],
        ),
        g(
            "island js while(a)",
            Tags,
            |n| format!("<script>{}x;</script>\n", "while(a)".repeat(n)),
            &[501],
        ),
        g(
            "island js for(;;)",
            Tags,
            |n| format!("<script>{}x;</script>\n", "for(;;)".repeat(n)),
            &[501],
        ),
        g(
            "island js with(a)",
            Tags,
            |n| format!("<script>{}x;</script>\n", "with(a)".repeat(n)),
            &[501],
        ),
        g(
            "island js do",
            Tags,
            |n| {
                format!(
                    "<script>{}x;{}</script>\n",
                    "do ".repeat(n),
                    "while(a)".repeat(n)
                )
            },
            &[501],
        ),
        g(
            "island js <a>",
            Tags,
            |n| {
                format!(
                    "<script>x = {}x{};</script>\n",
                    "<a>".repeat(n),
                    "</a>".repeat(n)
                )
            },
            &[501],
        ),
        g(
            "island js a:",
            Tags,
            |n| format!("<script>{}x;</script>\n", "a:".repeat(n)),
            &[501],
        ),
        g(
            "island js else if",
            Tags,
            |n| format!("<script>if(a)x;{}</script>\n", "else if(a)x;".repeat(n)),
            &[501],
        ),
        // Island bodies with a closer hidden in a literal at every level.
        g(
            "island css masked",
            Tags,
            |n| {
                format!(
                    "<style>{}color:red;{}</style>\n",
                    ".a{/* } */".repeat(n),
                    "}".repeat(n)
                )
            },
            &[600],
        ),
        g(
            "island css function",
            Tags,
            |n| {
                format!(
                    "<style>{}color:red;{}</style>\n",
                    ".a{b:foo(});".repeat(n),
                    "}".repeat(n)
                )
            },
            &[600],
        ),
        g(
            "island json masked",
            Tags,
            |n| {
                format!(
                    "<script type=\"application/json\">{}0{}</script>\n",
                    "[\"]\",".repeat(n),
                    "]".repeat(n)
                )
            },
            &[600],
        ),
    ]
}

/// The fixed cases of `tests/fuzz.rs`: every [`generators`] shape at its
/// sizes and at ten times each, as `(name, source, mode)`.
pub fn fixed_cases() -> Vec<(String, String, Mode)> {
    let mut out = Vec::new();
    for g in generators() {
        for &size in g.sizes {
            for n in [size, size * 10] {
                out.push((format!("{} × {n}", g.name), (g.source)(n), g.mode));
            }
        }
    }
    out
}
