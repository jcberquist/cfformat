//! The output invariants over every passing golden case: idempotence, token
//! preservation (whitespace in host text, separators and comment text
//! included: `common::token_stream`), reparse (no `invalid` or new `other`
//! tokens, same top-level statement count), comment preservation, literal
//! preservation (JavaScript
//! islands handed off; islands of no language, `<cfquery>` SQL and Java
//! bodies exactly: `common::check_preserved`), no trailing whitespace, and
//! line width: a line longer than `max_columns` must be on [`WIDE`].

mod common;

/// Golden lines allowed over `max_columns`: `(fixture[case], 1-based line,
/// why)`. A line may only be here when it holds a string, a comment or a
/// token wider than the limit, or has no break point at all. The test fails
/// when a listed line is no longer too wide, so the list cannot go stale.
const WIDE: &[(&str, usize, &str)] = &[
    (
        "alignDocComments[0]",
        13,
        "comment: an aligned `@token` doc line",
    ),
    (
        "alignDocComments[0]",
        15,
        "comment: an aligned `@authenticate` doc line",
    ),
    ("arrayMultilineMaxCol[1]", 3, "string at 20 columns"),
    ("commentTag[1]", 2, "comment: a tag comment's own line"),
    ("commentTag[1]", 5, "comment: the dashed banner, one token"),
    ("commentTag[1]", 7, "comment: a dashed comment's first line"),
    ("commentTag[1]", 8, "comment: a dashed comment's text line"),
    ("commentTag[1]", 9, "comment: a dashed comment's last line"),
    (
        "cfqueryParamWide[0]",
        6,
        "island line: a tag inside an island never breaks",
    ),
    (
        "cfqueryParamWide[0]",
        9,
        "island line: a tag inside an island never breaks",
    ),
    (
        "cfqueryParamWide[1]",
        6,
        "island line: a tag inside an island never breaks",
    ),
    (
        "cfqueryParamWide[1]",
        9,
        "island line: a tag inside an island never breaks",
    ),
    (
        "cfqueryTagComment[0]",
        12,
        "island line: a tag inside an island never breaks",
    ),
    (
        "cfqueryTagComment[0]",
        13,
        "island line: a tag inside an island never breaks",
    ),
    ("componentAttrs[3]", 3, "string at 30 columns"),
    (
        "keywordStatement[1]",
        2,
        "no break point: `return true;` at 12 columns",
    ),
    (
        "keywordStatement[1]",
        6,
        "no break point: `continue;` at 12 columns",
    ),
    ("structMultilineMaxCol[1]", 2, "string at 20 columns"),
    (
        "tagCommentBodyWide[1]",
        7,
        "comment: a tag comment's text line",
    ),
    (
        "tagCommentBodyWide[1]",
        15,
        "comment: a tag comment's text line",
    ),
    (
        "tagHTML[1]",
        4,
        "no break point: a `<cfoutput>` attribute entry at 51 columns",
    ),
    (
        "tagScriptStyleIndent[4]",
        23,
        "island line: a verbatim island is raised as a whole, never re-wrapped",
    ),
    ("structMultilineMaxCol[1]", 3, "string at 20 columns"),
];

#[test]
fn invariants() {
    let mut failures = Vec::new();
    let mut wide_seen = Vec::new();
    let counts = check_all(&mut failures, &mut wide_seen);
    for (case, line, why) in WIDE {
        if !wide_seen.iter().any(|(c, l)| c == case && l == line) {
            failures.push(format!(
                "{case} line {line} is on WIDE ({why}) but is not over max_columns"
            ));
        }
    }
    eprintln!(
        "invariants: {} cases checked; literals preserved in {} JavaScript islands; text preserved in {} islands of no language; literals preserved in {} SQL and {} Java bodies; {} lines of trailing whitespace inside literal islands exempted",
        counts.cases,
        counts.literal_islands,
        counts.preserved.opaque,
        counts.preserved.sql,
        counts.preserved.java,
        counts.exempt
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// What [`check_all`] counted.
#[derive(Default)]
struct Counts {
    /// Golden cases checked.
    cases: usize,
    /// JavaScript islands whose literals were compared.
    literal_islands: usize,
    /// Islands whose text or literals were compared exactly.
    preserved: common::Preserved,
    /// Lines with trailing whitespace inside an island that keeps its
    /// literal text.
    exempt: usize,
}

fn check_all(failures: &mut Vec<String>, wide_seen: &mut Vec<(String, usize)>) -> Counts {
    let mut counts = Counts::default();
    for fixture in common::fixtures() {
        for (i, case) in fixture.cases.iter().enumerate() {
            counts.cases += 1;
            let name = format!("{}[{i}]", fixture.name);
            let opts = &case.options;
            let once = common::format_case(&fixture, &fixture.source, opts);
            let twice = common::format_case(&fixture, &once, opts);
            if twice != once {
                failures.push(format!("{name}: not idempotent\n{once}---\n{twice}"));
            }
            for p in common::check_output(&fixture.source, &once, fixture.mode, opts) {
                failures.push(format!("{name}: {p}"));
            }
            let (islands, problems) =
                common::check_literals(&fixture.source, &once, fixture.mode, opts);
            counts.literal_islands += islands;
            failures.extend(problems.into_iter().map(|p| format!("{name}: {p}")));
            let (preserved, problems) =
                common::check_preserved(&fixture.source, &once, fixture.mode);
            counts.preserved += preserved;
            failures.extend(problems.into_iter().map(|p| format!("{name}: {p}")));
            let exempt = common::literal_island_lines(&once, fixture.mode);
            for (n, line) in once.lines().enumerate() {
                if line.ends_with([' ', '\t']) {
                    if exempt.contains(&(n + 1)) {
                        counts.exempt += 1;
                    } else {
                        failures.push(format!("{name}: trailing whitespace on line {}", n + 1));
                    }
                }
                let width: usize = line
                    .chars()
                    .map(|c| if c == '\t' { opts.indent_size } else { 1 })
                    .sum();
                if width > opts.max_columns {
                    if WIDE.iter().any(|(c, l, _)| *c == name && *l == n + 1) {
                        wide_seen.push((name.clone(), n + 1));
                    } else {
                        failures.push(format!(
                            "{name} line {}: {width} > {}: {line:?}",
                            n + 1,
                            opts.max_columns
                        ));
                    }
                }
            }
        }
    }
    counts
}

/// The token-preservation invariant itself: outputs that change what the
/// page shows, a comment's text or a separator are caught; the changes the
/// printer may make are not.
#[test]
fn the_token_stream_tells_significant_changes_apart() {
    use cfparse::Mode;
    let opts = cfformat::Options::default();
    let caught =
        |src: &str, out: &str, mode: Mode| !common::check_output(src, out, mode, &opts).is_empty();
    for (src, out) in [
        // Edge whitespace of an inline element.
        ("<span>hello </span>world\n", "<span>hello</span>world\n"),
        ("<span> hello</span>\n", "<span>hello</span>\n"),
        // A `<pre>` / `<textarea>` body, byte for byte.
        (
            "<textarea>  hello  </textarea>\n",
            "<textarea>hello</textarea>\n",
        ),
        ("<pre>\n  a\n</pre>\n", "<pre>\n    a\n</pre>\n"),
        ("<pre>a\n\n\nb</pre>\n", "<pre>a\n\nb</pre>\n"),
        // Whitespace inserted between glued pieces.
        (
            "<p>text <cfif x>yes</cfif>more</p>\n",
            "<p>\n    text <cfif x>yes</cfif>\n    more\n</p>\n",
        ),
        ("<b>a</b><i>b</i>\n", "<b>a</b>\n<i>b</i>\n"),
        (
            "<a href=\"x\"><img src=\"y\"></a>\n",
            "<a href=\"x\">\n    <img src=\"y\">\n</a>\n",
        ),
        // Whitespace added or removed before a CF tag among an HTML tag's
        // attributes.
        (
            "<td<cfif x> class=\"a\"</cfif>>\n",
            "<td <cfif x> class=\"a\"</cfif>>\n",
        ),
        (
            "<a class=b<cfif x>c</cfif>>\n",
            "<a class=b <cfif x>c</cfif>>\n",
        ),
        (
            "<a id=\"b\" <cfif x>class=\"c\"</cfif>>\n",
            "<a id=\"b\"<cfif x>class=\"c\"</cfif>>\n",
        ),
        // A no-break space is text, not whitespace.
        ("<span>\u{a0}a</span>\n", "<span>a</span>\n"),
        ("<div>\n\u{a0}\n</div>\n", "<div>\n</div>\n"),
        // Whitespace removed between two words.
        ("a <cfset x = 1> b\n", "a <cfset x = 1>b\n"),
        // A comment's text.
        ("<!--- a b --->\n", "<!--- a c --->\n"),
    ] {
        assert!(
            caught(src, out, Mode::Tags),
            "not caught: {src:?} -> {out:?}"
        );
    }
    for (src, out) in [
        ("x = [1, 2];\n", "x = [1 2];\n"),
        ("f(a, b);\n", "f(a b);\n"),
        ("/* a\n   b */\nx = 1;\n", "/* a b */\nx = 1;\n"),
    ] {
        assert!(
            caught(src, out, Mode::Script),
            "not caught: {src:?} -> {out:?}"
        );
    }
    for (src, out) in [
        // A block tag's edges and surroundings, a CF body's edges, the
        // document's edges, re-indented lines.
        ("<div> a </div>\n", "<div>a</div>\n"),
        ("<p>a</p><p>b</p>\n", "<p>a</p>\n<p>b</p>\n"),
        ("<tr><td>a</td></tr>\n", "<tr>\n    <td>a</td>\n</tr>\n"),
        ("<cfif x> a </cfif>\n", "<cfif x>\n    a\n</cfif>\n"),
        (
            "<cfif x>a<cfelse>b</cfif>\n",
            "<cfif x>\n    a\n<cfelse>\n    b\n</cfif>\n",
        ),
        ("\n\n  <b>a</b>  \n\n", "<b>a</b>\n"),
        (
            "<div>\na\n      b\n</div>\n",
            "<div>\n    a\n    b\n</div>\n",
        ),
        // A tag comment's spacers and its own-line form.
        ("<!---a--->\n", "<!---\n    a\n--->\n"),
        // A CF tag among attributes moved to a line of its own.
        (
            "<td <cfif x>class=\"a\"</cfif> id=\"b\">\n",
            "<td\n    <cfif x>class=\"a\"</cfif>\n    id=\"b\"\n>\n",
        ),
    ] {
        assert!(!caught(src, out, Mode::Tags), "caught: {src:?} -> {out:?}");
    }
    for (src, out) in [
        // A trailing comma, and a comment's re-indented lines.
        ("x = [1, 2,];\n", "x = [1, 2];\n"),
        ("x = [1, 2];\n", "x = [\n    1,\n    2,\n];\n"),
        (
            "/**\n     * a\n     */\nx = 1;\n",
            "/**\n * a\n */\nx = 1;\n",
        ),
    ] {
        assert!(
            !caught(src, out, Mode::Script),
            "caught: {src:?} -> {out:?}"
        );
    }
}
