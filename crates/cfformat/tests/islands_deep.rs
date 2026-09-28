//! Deep `<script>` / `<style>` bodies: the island bounds.
//! A body nested past `NESTING_LIMIT` (500) or larger than `SIZE_LIMIT`
//! (256 KB) is refused with the island warning and prints as written; one
//! within them is formatted on the island thread, whatever the caller's
//! stack. A script's depth is its brackets' and then its parsed tree's,
//! every node counted; a stylesheet's or JSON's, its brackets'; brackets
//! are counted outside strings and comments (`nesting_depth`). Every case
//! runs on a 2 MB thread, the stack a test or a default thread has, so the
//! caller's stack is shown not to matter.
//!
//! In any build, every shape — brackets, brackets with a closer hidden in a
//! literal at every level, unbraced statement bodies and JSX, chains of
//! operators, calls and members — formats at the largest size within the
//! limit and is refused one step past it; each bracket shape is refused at
//! the size where, handed to oxc unbounded, it first overflows the CLI's
//! 8 MB worker and at ten times that, and the masked CSS and JSON shapes at
//! the size limit, where a byte count of their brackets would let them
//! through to format into gigabytes or abort.
//!
//! The size-limit group (`every_chain_at_the_size_limit_is_refused`)
//! takes each chain and statement shape to the largest size under the
//! size limit: its tree is refused, and the parse that built it — the one
//! recursion the depth does not bound — is timed and its peak memory
//! printed; so is the parse of each masked script shape at that size. It
//! runs by default, in any build (about 5 s in a debug build, at most
//! 134 MB for one parse), so every platform's test job puts the parser at
//! the size limit on the island thread; CI runs it again in release with
//! its table printed, the measurement:
//!
//! ```text
//! cargo test --release -p cfformat --test islands_deep -- every_chain_at_the_size_limit_is_refused --nocapture
//! ```

use std::time::Instant;

use cfformat::islands::{nesting_depth, tree_depth, NESTING_LIMIT, SIZE_LIMIT};
use cfformat::{FormatCtx, Formatted, Islands, Options};
use cfparse::{Lang, Mode};

/// One deep shape: its name, the island option it goes to, the body for a
/// size (the page is `open` + body + `close`), and the levels the body
/// nests beyond the size (`@media`'s innermost rule).
struct Shape {
    name: String,
    key: &'static str,
    body: Box<dyn Fn(usize) -> String>,
    inner: usize,
}

/// A shape with no inner level.
fn shape(name: &str, key: &'static str, body: impl Fn(usize) -> String + 'static) -> Shape {
    Shape {
        name: name.into(),
        key,
        body: Box::new(body),
        inner: 0,
    }
}

const SCRIPT: (&str, &str) = ("<script>", "</script>\n");
const JSON: (&str, &str) = ("<script type=\"application/json\">", "</script>\n");
const STYLE: (&str, &str) = ("<style>", "</style>\n");

impl Shape {
    /// The island's language, as its key dispatches it.
    fn lang(&self) -> Lang {
        match self.key {
            "islands.json" => Lang::Json,
            "islands.css" => Lang::Css,
            _ => Lang::Js,
        }
    }

    fn page(&self, n: usize) -> (String, String) {
        let (open, close) = match self.key {
            "islands.json" => JSON,
            "islands.css" => STYLE,
            _ => SCRIPT,
        };
        let body = (self.body)(n);
        (format!("{open}{body}{close}"), body)
    }
}

/// The depth `NESTING_LIMIT` applies to, as the island formatter counts
/// it: the brackets first, then for a script the depth of its tree.
fn depth(shape: &Shape, body: &str) -> usize {
    let brackets = nesting_depth(body, shape.lang());
    if brackets > NESTING_LIMIT || shape.key != "islands.js" {
        return brackets;
    }
    tree_depth(body).unwrap_or_else(|| panic!("{}: does not parse", shape.name))
}

/// The largest size of `shape` within `NESTING_LIMIT`, by bisection (the
/// depth grows with the size, at least a level a step).
fn boundary(shape: &Shape) -> usize {
    let depth = |n| depth(shape, &shape.page(n).1);
    let (mut within, mut past) = (0, 2 * NESTING_LIMIT);
    assert!(depth(within) <= NESTING_LIMIT && depth(past) > NESTING_LIMIT);
    while past - within > 1 {
        let n = (within + past) / 2;
        if depth(n) <= NESTING_LIMIT {
            within = n;
        } else {
            past = n;
        }
    }
    within
}

/// The bracket shapes, each with the size at which, handed to oxc
/// unbounded, it first overflows the CLI's 8 MB worker in release.
fn brackets() -> Vec<(Shape, usize)> {
    fn s(name: &str, key: &'static str, body: fn(usize) -> String) -> Shape {
        shape(name, key, body)
    }
    vec![
        (
            s("js (", "islands.js", |n| {
                format!("x = {}1{};", "(".repeat(n), ")".repeat(n))
            }),
            4_875,
        ),
        (
            s("js [", "islands.js", |n| {
                format!("x = {}1{};", "[".repeat(n), "]".repeat(n))
            }),
            4_875,
        ),
        (
            s("js {a:", "islands.js", |n| {
                format!("x = {}1{};", "{a:".repeat(n), "}".repeat(n))
            }),
            4_187,
        ),
        (
            s("json [", "islands.json", |n| {
                format!("{}0{}", "[".repeat(n), "]".repeat(n))
            }),
            4_875,
        ),
        (
            s("json {\"a\":", "islands.json", |n| {
                format!("{}0{}", "{\"a\":".repeat(n), "}".repeat(n))
            }),
            4_187,
        ),
        (
            s("css .a{", "islands.css", |n| {
                format!("{}color:red;{}", ".a{".repeat(n), "}".repeat(n))
            }),
            1_015,
        ),
        (
            Shape {
                inner: 1,
                ..s("css @media", "islands.css", |n| {
                    format!(
                        "{}.a{{color:red}}{}",
                        "@media screen{".repeat(n),
                        "}".repeat(n)
                    )
                })
            },
            577,
        ),
    ]
}

/// The bracket shapes with a closer hidden in a literal at every level: a
/// comment, a string, a regular expression, a template literal. A byte
/// count of the brackets cancels a real opener with each, so
/// `.a{/* } */`×600 would count 1 and format into 2 MB, and at the size
/// limit into 3.4 GB (CSS) or an abort (JSON). And a closer of another kind
/// inside a CSS function's arguments, a token to the parser: counted
/// without matching kinds, `.a{b:foo(});`×600 is about 2 deep and formats
/// into 3.6 MB, and at the size limit aborts; its innermost `foo(` is one
/// level over the size.
fn masked() -> Vec<Shape> {
    fn s(name: &str, key: &'static str, body: fn(usize) -> String) -> Shape {
        shape(name, key, body)
    }
    vec![
        s("css .a{/* } */", "islands.css", |n| {
            format!("{}color:red;{}", ".a{/* } */".repeat(n), "}".repeat(n))
        }),
        Shape {
            inner: 1,
            ..s("css .a{b:foo(});", "islands.css", |n| {
                format!("{}color:red;{}", ".a{b:foo(});".repeat(n), "}".repeat(n))
            })
        },
        s("json [\"]\",", "islands.json", |n| {
            format!("{}0{}", "[\"]\",".repeat(n), "]".repeat(n))
        }),
        s("js [\"]\",", "islands.js", |n| {
            format!("x = {}0{};", "[\"]\",".repeat(n), "]".repeat(n))
        }),
        s("js {a:\"}\",b:", "islands.js", |n| {
            format!("x = {}1{};", "{a:\"}\",b:".repeat(n), "}".repeat(n))
        }),
        s("js [/]/,", "islands.js", |n| {
            format!("x = {}0{};", "[/]/,".repeat(n), "]".repeat(n))
        }),
        s("js [`]`,", "islands.js", |n| {
            format!("x = {}0{};", "[`]`,".repeat(n), "]".repeat(n))
        }),
        s("js [//]", "islands.js", |n| {
            format!("x = {}0{};", "[//]\n".repeat(n), "]".repeat(n))
        }),
        s("js (/*)*/", "islands.js", |n| {
            format!("x = {}1{};", "(/*)*/".repeat(n), ")".repeat(n))
        }),
    ]
}

/// The nesting with no bracket to count: an unbraced statement body, a JSX
/// child, a label, an `else if`. Each indents a level, so the output grows
/// with the square of the depth: unless the parsed tree is bounded,
/// `if(a)`×52,000 (260 KB) formats at 25.7 GB of RSS.
fn statements() -> Vec<Shape> {
    fn s(name: &str, body: fn(usize) -> String) -> Shape {
        shape(name, "islands.js", body)
    }
    vec![
        s("if(a)", |n| format!("{}x;", "if(a)".repeat(n))),
        s("while(a)", |n| format!("{}x;", "while(a)".repeat(n))),
        s("for(;;)", |n| format!("{}x;", "for(;;)".repeat(n))),
        s("with(a)", |n| format!("{}x;", "with(a)".repeat(n))),
        s("do", |n| {
            format!("{}x;{}", "do ".repeat(n), "while(a)".repeat(n))
        }),
        s("jsx <a>", |n| {
            format!("x = {}x{};", "<a>".repeat(n), "</a>".repeat(n))
        }),
        s("label a:", |n| format!("{}x;", "a:".repeat(n))),
        s("else if", |n| {
            format!("if(a)x;{}", "else if(a)x;".repeat(n))
        }),
    ]
}

/// The JavaScript chains with no bracket to count: `x = `, the unit `n`
/// times, `a;`. A call chain's brackets close at once (bracket depth 1),
/// so it is one of them. Unless the parsed tree is bounded, each formats at
/// the size limit, the unary `- ` / `+ ` needing 225 MB of the island
/// thread's 256.
const CHAINS: &[&str] = &[
    "()", "!", "- ", "+ ", "~", "typeof ", "new ", "x=", "a=>", "a,", ".b", "a?b:", "a + ", "a**",
];

/// A chain `n` long: `x = a()()…;` for calls, `x = <unit>…a;` otherwise.
fn chain(unit: &str, n: usize) -> String {
    if unit == "()" || unit == ".b" {
        format!("x = a{};", unit.repeat(n))
    } else {
        format!("x = {}a;", unit.repeat(n))
    }
}

/// The one chain that does not nest: a sequence is one node, however long.
const FLAT: &str = "a,";

/// Every chain as a shape.
fn chains() -> Vec<Shape> {
    CHAINS
        .iter()
        .map(|&unit| {
            shape(&format!("chain {unit:?}"), "islands.js", move |n| {
                chain(unit, n)
            })
        })
        .collect()
}

/// The largest size of `shape` whose hand-off text (the body and a
/// newline) fits `SIZE_LIMIT`.
fn longest(shape: &Shape) -> usize {
    let size = |n| (shape.body)(n).len() + 1;
    let n = (SIZE_LIMIT - size(0)) / (size(1) - size(0));
    assert!(size(n) <= SIZE_LIMIT && size(n + 1) > SIZE_LIMIT);
    n
}

/// Formats `page` as a tag-mode file with the default options (except that
/// the newline is `"\n"` whatever the platform's, so a body printed as
/// written is byte for byte the body) and a cache of its own, on a 2 MB
/// thread.
fn format(page: String) -> Formatted {
    std::thread::Builder::new()
        .name("islands_deep".into())
        .stack_size(2 << 20)
        .spawn(move || {
            let islands = Islands::new();
            let ctx = FormatCtx {
                path: None,
                islands: Some(&islands),
            };
            let opts = Options {
                newline: cfformat::options::NewlineStyle::Lf,
                ..Options::default()
            };
            cfformat::format_with(&page, Mode::Tags, &opts, &ctx)
        })
        .unwrap()
        .join()
        .unwrap()
}

/// `page` refused with `message`: one warning, the body as written.
fn assert_refused(name: &str, key: &str, page: String, body: &str, message: &str) {
    assert_output_refused(name, key, format(page), body, message);
}

/// [`assert_refused`] of a page already formatted.
fn assert_output_refused(name: &str, key: &str, out: Formatted, body: &str, message: &str) {
    let warnings: Vec<String> = out.warnings.iter().map(|w| w.to_string()).collect();
    assert_eq!(warnings, [format!("<stdin>:1: {key}: {message}")], "{name}");
    assert!(
        out.text.contains(body),
        "{name}: the body is not as written"
    );
    assert_eq!(
        (out.islands.formatted, out.islands.warnings),
        (1, 1),
        "{name}"
    );
}

/// `shape` formats at its [`boundary`] and is refused one step past it,
/// with the depth that tripped.
fn formats_at_the_limit_and_is_refused_past_it(shape: &Shape) {
    let started = Instant::now();
    let n = boundary(shape);
    let (page, body) = shape.page(n);
    let within = depth(shape, &body);
    let out = format(page);
    assert!(
        out.warnings.is_empty(),
        "{}: {:?}",
        shape.name,
        out.warnings
    );
    assert_eq!(out.islands.formatted, 1, "{}", shape.name);
    assert!(!out.text.contains(&body), "{}: not formatted", shape.name);
    let (page, body) = shape.page(n + 1);
    let past = depth(shape, &body);
    assert_refused(
        &shape.name,
        shape.key,
        page,
        &body,
        &format!("nested {past} levels deep, over the limit of 500"),
    );
    println!(
        "{}: formats at {n} ({within} deep), refused at {} ({past} deep), in {:?}",
        shape.name,
        n + 1,
        started.elapsed()
    );
}

#[test]
fn every_bracket_shape_formats_at_the_limit_and_is_refused_past_it() {
    assert_eq!(NESTING_LIMIT, 500);
    for (shape, _) in brackets() {
        // CSS and JSON by their brackets, 500 deep.
        if shape.key != "islands.js" {
            let n = NESTING_LIMIT - shape.inner;
            assert_eq!(boundary(&shape), n, "{}", shape.name);
            assert_eq!(nesting_depth(&shape.page(n).1, shape.lang()), NESTING_LIMIT);
        }
        formats_at_the_limit_and_is_refused_past_it(&shape);
    }
}

#[test]
fn every_masked_shape_formats_at_the_limit_and_is_refused_past_it() {
    for shape in masked() {
        // CSS and JSON by their brackets, 500 deep: the hidden closers
        // hide nothing.
        if shape.key != "islands.js" {
            let n = NESTING_LIMIT - shape.inner;
            assert_eq!(boundary(&shape), n, "{}", shape.name);
            assert_eq!(nesting_depth(&shape.page(n).1, shape.lang()), NESTING_LIMIT);
        }
        formats_at_the_limit_and_is_refused_past_it(&shape);
    }
}

#[test]
fn every_statement_shape_formats_at_the_limit_and_is_refused_past_it() {
    for shape in statements() {
        formats_at_the_limit_and_is_refused_past_it(&shape);
    }
}

#[test]
fn every_chain_formats_at_the_limit_and_is_refused_past_it() {
    assert_eq!(tree_depth(&chain(FLAT, 2 * NESTING_LIMIT)), Some(5));
    for shape in chains() {
        if shape.name != format!("chain {FLAT:?}") {
            formats_at_the_limit_and_is_refused_past_it(&shape);
        }
    }
}

#[test]
fn every_bracket_shape_that_aborted_is_refused() {
    for (shape, abort) in brackets() {
        for n in [abort, abort * 10] {
            let (page, body) = shape.page(n);
            let depth = n + shape.inner;
            assert_eq!(nesting_depth(&body, shape.lang()), depth, "{}", shape.name);
            assert_refused(
                &shape.name,
                shape.key,
                page,
                &body,
                &format!("nested {depth} levels deep, over the limit of 500"),
            );
        }
    }
}

/// The masked CSS and JSON shapes at the largest size under the size
/// limit: counted by bytes, the first formats into 3.4 GB at 9.8 GB of RSS
/// and the second aborts the process (`memory allocation of 17179934720
/// bytes failed`), and the CSS function shape, counted without matching
/// kinds, aborts too (`memory allocation of 4294950912 bytes failed` under
/// a 4 GB address-space limit). Refused by their brackets, before oxc sees
/// them.
#[test]
fn masked_css_and_json_at_the_size_limit_are_refused() {
    for shape in masked().into_iter().filter(|s| s.key != "islands.js") {
        let n = longest(&shape);
        let (page, body) = shape.page(n);
        let depth = n + shape.inner;
        assert_eq!(nesting_depth(&body, shape.lang()), depth, "{}", shape.name);
        assert_refused(
            &shape.name,
            shape.key,
            page,
            &body,
            &format!("nested {depth} levels deep, over the limit of 500"),
        );
        println!("{} × {n}: {} bytes, refused", shape.name, body.len() + 1);
        let (page, body) = shape.page(n + 1);
        let size = body.len() + 1;
        assert!(size > SIZE_LIMIT);
        assert_refused(
            &shape.name,
            shape.key,
            page,
            &body,
            &format!("{size} bytes, over the limit of 262144"),
        );
    }
}

#[test]
fn a_chain_past_the_size_limit_is_refused() {
    // `a + `×75,000 overflowed cfformat's own walk on the 8 MB worker.
    let body = chain("a + ", 75_000);
    let (page, size) = (format!("<script>{body}</script>\n"), body.len() + 1);
    assert_eq!(size, 300_007);
    assert_refused(
        "a + ",
        "islands.js",
        page,
        &body,
        "300007 bytes, over the limit of 262144",
    );
    // One byte past the limit.
    let body = chain("!", SIZE_LIMIT - chain("!", 0).len());
    assert_eq!(body.len() + 1, SIZE_LIMIT + 1);
    assert_refused(
        "!",
        "islands.js",
        format!("<script>{body}</script>\n"),
        &body,
        "262145 bytes, over the limit of 262144",
    );
}

/// Every chain and statement shape at the largest size the size limit
/// lets through is parsed on the island thread and refused by its tree
/// (but the flat sequence, which formats): the parse is what the depth
/// cannot bound, so its time and its peak memory are printed. The masked
/// CSS and JSON shapes at that size are refused by their brackets before
/// oxc sees them. The masked script shapes are parsed alone
/// ([`tree_depth`]): the parser's need on brackets the pre-scan does not
/// see, should its reading of a script be fooled.
#[test]
fn every_chain_at_the_size_limit_is_refused() {
    for shape in chains().into_iter().chain(statements()) {
        let n = longest(&shape);
        let (page, body) = shape.page(n);
        let before = reset_peak_rss();
        let started = Instant::now();
        let out = format(page);
        let elapsed = started.elapsed();
        let peak = peak_rss();
        let depth = tree_depth(&body).unwrap();
        let flat = shape.name == format!("chain {FLAT:?}");
        assert_eq!(depth <= NESTING_LIMIT, flat, "{}", shape.name);
        if flat {
            assert!(out.warnings.is_empty(), "{}", shape.name);
            assert_eq!(out.islands.formatted, 1);
        } else {
            assert_output_refused(
                &shape.name,
                shape.key,
                out,
                &body,
                &format!("nested {depth} levels deep, over the limit of 500"),
            );
        }
        println!(
            "{} × {n}: {} bytes, {depth} deep, {} in {elapsed:.2?}, peak RSS +{}",
            shape.name,
            body.len() + 1,
            if flat { "formatted" } else { "refused" },
            rss(before, peak),
        );
    }
    for shape in masked() {
        let n = longest(&shape);
        let (page, body) = shape.page(n);
        let before = reset_peak_rss();
        let started = Instant::now();
        let (outcome, depth) = if shape.key == "islands.js" {
            let depth = tree_depth(&body);
            assert!(depth.is_some(), "{}: does not parse", shape.name);
            ("parsed", depth.unwrap_or(0))
        } else {
            let depth = nesting_depth(&body, shape.lang());
            assert_output_refused(
                &shape.name,
                shape.key,
                format(page),
                &body,
                &format!("nested {depth} levels deep, over the limit of 500"),
            );
            ("refused", depth)
        };
        let elapsed = started.elapsed();
        println!(
            "{} × {n}: {} bytes, {depth} deep, {outcome} in {elapsed:.2?}, peak RSS +{}",
            shape.name,
            body.len() + 1,
            rss(before, peak_rss()),
        );
    }
}

/// The peak's growth over `before`, in MB, or `?` where either is unknown.
fn rss(before: Option<u64>, peak: Option<u64>) -> String {
    match (before, peak) {
        (Some(before), Some(peak)) => format!("{} MB", peak.saturating_sub(before) >> 10),
        _ => "?".into(),
    }
}

/// Resets the process's peak resident set to the current one and returns
/// it, in KB (Linux: `/proc/self/clear_refs`). `getrusage`'s maximum is
/// the process's whole life, so it could not tell one case from another.
fn reset_peak_rss() -> Option<u64> {
    std::fs::write("/proc/self/clear_refs", "5").ok()?;
    status_kb("VmRSS:")
}

/// The peak resident set since [`reset_peak_rss`], in KB.
fn peak_rss() -> Option<u64> {
    status_kb("VmHWM:")
}

/// A `kB` field of `/proc/self/status`.
fn status_kb(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with(field))?;
    line[field.len()..].trim().strip_suffix(" kB")?.parse().ok()
}

/// What the island thread costs: one `Islands::format` of a one-line
/// island, the spawn included (a fresh cache each time, so every call
/// formats). Printed, not asserted.
#[test]
#[ignore = "a timing: release only"]
fn one_island_costs() {
    use cfformat::islands::IslandRequest;
    use cfformat::IndentStyle;
    use cfparse::Lang;
    let text = "var x = {a:1}\n";
    let req = IslandRequest {
        lang: Lang::Js,
        text,
        path: "stdin.cfm.js".into(),
        indent: IndentStyle::Spaces(4),
        width: 80,
        config: Default::default(),
    };
    let runs = 1_000;
    let started = Instant::now();
    for _ in 0..runs {
        let (out, hit) = Islands::new().format(&req);
        assert!(out.is_ok() && !hit.cached);
    }
    println!(
        "one Islands::format of a one-line island: {:.1?} each",
        started.elapsed() / runs
    );
}
