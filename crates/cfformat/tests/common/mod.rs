//! Shared helpers for the golden and invariant tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use cfformat::Options;
use cfparse::{
    BlockKind, Element, ElementKind, Ident, IslandSite, Lang, Mode, Node, Punct, Token, TokenKind,
    Tree,
};
use serde_json::{Map, Value};

/// `commandbox-cfformat` checkout next to the workspace, if present.
pub fn commandbox_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../commandbox-cfformat");
    dir.join("models").is_dir().then_some(dir)
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// One settings case of a fixture.
pub struct Case {
    pub options: Options,
    pub expected: String,
}

pub struct Fixture {
    pub name: String,
    pub dir: PathBuf,
    /// The source file: `source.cfc`, or `source.cfm` for a template
    /// fixture formatted as a `.cfm` is.
    pub path: PathBuf,
    pub source: String,
    pub mode: Mode,
    pub cases: Vec<Case>,
}

/// Every fixture under `tests/fixtures` (the `exprTests` sources are not
/// fixtures: see [`expr_tests`]). Script fixtures (first line `//`) drop
/// that line and parse as `Mode::Script`; tag fixtures parse as `Mode::Auto`,
/// and a fixture whose source is `source.cfm` as `Mode::Tags`, as the CLI
/// reads a `.cfm`.
pub fn fixtures() -> Vec<Fixture> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("settings.json").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter().map(|dir| load(&dir)).collect()
}

fn load(dir: &Path) -> Fixture {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let cfm = dir.join("source.cfm");
    let path = if cfm.is_file() {
        cfm
    } else {
        dir.join("source.cfc")
    };
    let raw = std::fs::read_to_string(&path).unwrap();
    let (source, mode) = match split_source(&raw) {
        (source, Mode::Auto) if path.extension().is_some_and(|e| e == "cfm") => {
            (source, Mode::Tags)
        }
        split => split,
    };
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
            .unwrap_or_else(|e| panic!("{name}/settings.json: {e}"));
    let settings = match settings {
        Value::Array(list) => list,
        object @ Value::Object(_) => vec![object],
        _ => panic!("{name}/settings.json: expected an array or an object"),
    };
    let expected = split_expectations(&std::fs::read_to_string(dir.join("formatted.txt")).unwrap());
    assert_eq!(
        settings.len(),
        expected.len(),
        "{name}: settings cases vs expectations"
    );
    let cases: Vec<Case> = settings
        .into_iter()
        .zip(expected)
        .map(|(s, expected)| Case {
            options: options(&name, s),
            expected,
        })
        .collect();
    Fixture {
        name,
        dir: dir.to_path_buf(),
        path,
        source,
        mode,
        cases,
    }
}

/// Options for one settings object. The expectations use `\n`, so a case
/// that does not set `newline` gets `"\n"` rather than the platform's; and
/// a case that does not name `islands.config` gets `"off"`, so that no
/// `.prettierrc` above the checkout moves an expectation (only the cases
/// that turn it on read a configuration, their fixture directory's). Every
/// case is written in the current keys: a case the migration would change,
/// even silently (a spelling-only rename, agreeing comma keys), fails here.
fn options(name: &str, settings: Value) -> Options {
    let Value::Object(mut map) = settings else {
        panic!("{name}: a settings case is not an object");
    };
    map.entry("newline").or_insert_with(|| Value::from("\n"));
    map.entry("islands.config")
        .or_insert_with(|| Value::from("off"));
    let (migrated, warnings) = Options::migrate(map.clone());
    assert!(
        warnings.is_empty() && migrated == map,
        "{name}/settings.json: a case in old keys: {warnings:?}"
    );
    Options::validate(&map).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Formats `src` as the fixture's source file (so an island's
/// configuration is looked up from the fixture directory), with a cache of
/// its own.
pub fn format_case(fixture: &Fixture, src: &str, opts: &Options) -> String {
    let islands = cfformat::Islands::new();
    let ctx = cfformat::FormatCtx {
        path: Some(&fixture.path),
        islands: Some(&islands),
    };
    cfformat::format_with(src, fixture.mode, opts, &ctx).text
}

pub fn split_source(raw: &str) -> (String, Mode) {
    if raw.starts_with("//") {
        let body = raw.split_once('\n').map_or("", |(_, rest)| rest);
        (body.to_string(), Mode::Script)
    } else {
        (raw.to_string(), Mode::Auto)
    }
}

/// `formatted.txt` holds one expectation per settings case, separated by a
/// line holding only `~` (and its line ending, `\n` or `\r\n`).
pub fn split_expectations(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "~" {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push_str(line);
        }
    }
    out
}

/// Inverse of [`split_expectations`].
pub fn join_expectations(parts: &[String]) -> String {
    parts.join("~\n")
}

/// The `exprTests` sources (name, source without the leading `//` line).
pub fn expr_tests() -> Vec<(String, String)> {
    let dir = fixtures_dir().join("exprTests");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "cfc"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let raw = std::fs::read_to_string(&p).unwrap();
            let name = format!("exprTests.{}", p.file_stem().unwrap().to_string_lossy());
            (name, split_source(&raw).0)
        })
        .collect()
}

/// Comment elements in `el` and below.
pub fn comment_count(el: &Element) -> usize {
    usize::from(el.kind.is_comment())
        + el.nodes()
            .filter_map(Node::as_element)
            .map(comment_count)
            .sum::<usize>()
}

/// One entry of a normalised token stream: the token kind's name and its
/// normalised text.
pub type Tok = (String, String);

/// Token preservation: the significant tokens of a tree in source order,
/// normalised for what the formatter may change.
///
/// - Host text (the text, whitespace and newlines between the tags of a
///   tag-mode document, a tag body, a code fence or a verbatim island) is
///   one `text` entry per run between two other nodes, each whitespace run
///   collapsed to one space: the tag printer re-indents, trims line ends and
///   caps blank lines, but whether two pieces had whitespace between them
///   shows on the page. The run's leading or trailing space is dropped where
///   the printer may legitimately add or remove whitespace: at the edges of
///   the document root, a code fence, an island, a CF tag body (`<cfscript>`
///   is script, not host text) and a block tag's body, next to a block tag
///   (paired or bare) and next to a `<cfelse>` / `<cfelseif>`; a block tag
///   here is one of [`spaceless_tags`]. An inline HTML body keeps both. A
///   paired `<pre>` / `<textarea>` body is one `pre` entry, its text exact.
/// - Whitespace and newlines elsewhere are skipped, but in an HTML tag's
///   attribute list a CF tag, `#expr#` or other non-attribute written
///   directly against what precedes it (the tag's name, an attribute, a
///   value) gets a `glued` entry before it ([`glues_in_html_tag`]): whether
///   whitespace separates it shows on the page.
/// - A comment is a `comment` entry: its delimiters and its text with each
///   line trimmed (the printer re-indents continuation lines) and blank
///   first and last lines dropped (a `<!--- --->` gains or loses its spacers
///   and its own-line form); a nested comment is normalised the same way.
///   Under `alignment.doc_comments` a doc comment's text is left out: that
///   option rewrites it. [`check_output`] compares the comments apart from
///   the other entries.
/// - A separator comma is kept; a trailing one (only trivia between it and
///   the element's closing delimiter, or its end) is dropped, since
///   trailing commas come and go, and so is the comma after an empty item
///   (`[1,,2]`), which the printer drops with the item.
/// - A `String` element becomes one `string` entry per text run with its
///   quotes removed and its doubled-quote escapes decoded (quote style), a
///   bare struct key is a `string` entry too (`struct.quote_keys`); a
///   `Punct(KeyValue)` is `:` (`struct.separator`); builtin, UDF and member
///   call names are one kind, lower-cased (casing, and the space before
///   `(`); the inner whitespace of operator,
///   keyword and HTML comment tokens is collapsed (an HTML comment's lines
///   are re-indented as text is); and a tag name is lower-cased under
///   `tags.lowercase`.
///
/// With islands on, an island oxc takes — pure, non-blank, outside a code
/// fence, dispatched to an `islands.*` option that is not `"off"` — is one
/// `island` entry naming its synthetic extension: its text is oxc's
/// business. So is a JavaScript or CSS island holding only text, `##` and
/// `#…#` ([`interpolated`]), whose `island` entry is followed by its `##`
/// and `#…#` in order, each `#…#` streamed as any expression is: the
/// formatter must give back every CF token and string of them, in order.
pub fn token_stream(tree: &Tree, opts: &Options) -> Vec<Tok> {
    let mut out = Vec::new();
    stream_element(tree, opts, &tree.root, false, &mut out);
    out
}

/// HTML tags next to which and at the edges of whose body whitespace does
/// not show on the page, lower-cased: the block tags (`data/tags.json`
/// `blockTags.html`) and the tags a browser lays out without it though they
/// are not block tags (`spacelessTags.html`: `<tr>`, `<head>`, …).
fn spaceless_tags() -> &'static std::collections::HashSet<String> {
    static TAGS: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    TAGS.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!("../../data/tags.json")).unwrap();
        ["blockTags", "spacelessTags"]
            .iter()
            .flat_map(|key| data[key]["html"].as_array().unwrap())
            .map(|v| v.as_str().unwrap().to_ascii_lowercase())
            .collect()
    })
}

/// Whether `el` is an HTML tag, paired or bare, of [`spaceless_tags`].
fn is_spaceless_html(tree: &Tree, el: &Element) -> bool {
    matches!(
        el.kind,
        ElementKind::TagBody { cf: false } | ElementKind::HtmlTag(_)
    ) && tree
        .tag_name(el)
        .is_some_and(|n| spaceless_tags().contains(&n.to_ascii_lowercase()))
}

/// Whether `el` is a bare `<cfelse>` / `<cfelseif>`, which splits the CF
/// body around it into segments, each on lines of its own.
fn is_else_tag(tree: &Tree, el: &Element) -> bool {
    matches!(el.kind, ElementKind::CfTag(shape, _) if shape != cfparse::TagShape::Close)
        && tree
            .tag_name(el)
            .is_some_and(|n| n.eq_ignore_ascii_case("cfelse") || n.eq_ignore_ascii_case("cfelseif"))
}

/// Whether whitespace next to `el`, on either side, may be added or removed:
/// a block tag, a `<cfelse>` / `<cfelseif>` or an island.
fn strips_around(tree: &Tree, el: &Element) -> bool {
    is_spaceless_html(tree, el)
        || is_else_tag(tree, el)
        || matches!(el.kind, ElementKind::Island(_))
}

/// For an element whose children are host text: whether whitespace at its
/// edges may be added or removed. `None` for any other element.
fn host_edges(tree: &Tree, el: &Element) -> Option<bool> {
    match el.kind {
        ElementKind::Root(Mode::Tags) | ElementKind::TagIsland | ElementKind::Island(_) => {
            Some(true)
        }
        ElementKind::TagBody { cf: true } => tree
            .tag_name(el)
            .is_none_or(|n| !n.eq_ignore_ascii_case("cfscript"))
            .then_some(true),
        ElementKind::TagBody { cf: false } => Some(is_spaceless_html(tree, el)),
        _ => None,
    }
}

/// A comment's `comment` entry: its delimiters around its text, each line
/// of the text trimmed and blank first and last lines dropped, a nested
/// comment normalised the same way.
fn comment_text(tree: &Tree, el: &Element) -> String {
    let mut text = String::new();
    for n in &el.children {
        match n {
            Node::Element(c) if c.kind.is_comment() => text.push_str(&comment_text(tree, c)),
            n => text.push_str(tree.slice(n.span())),
        }
    }
    let lines: Vec<&str> = text.split('\n').map(str::trim).collect();
    let first = lines.iter().position(|l| !l.is_empty());
    let last = lines.iter().rposition(|l| !l.is_empty());
    let body = match (first, last) {
        (Some(a), Some(b)) => lines[a..=b].join("\n"),
        _ => String::new(),
    };
    let open = el.open.as_ref().map_or("", |t| tree.text(t));
    let close = el.close.as_ref().map_or("", |t| tree.text(t));
    format!("{open}{body}{close}")
}

/// The host text gathered since the last other node, flushed as one `text`
/// entry.
fn flush_text(buf: &mut String, strip_left: bool, strip_right: bool, out: &mut Vec<Tok>) {
    let mut text = String::new();
    let mut space = false;
    for c in buf.chars() {
        if c.is_ascii_whitespace() {
            space = true;
            continue;
        }
        if space && !(text.is_empty() && strip_left) {
            text.push(' ');
        }
        space = false;
        text.push(c);
    }
    if space && !strip_right && !(text.is_empty() && strip_left) {
        text.push(' ');
    }
    buf.clear();
    if !text.is_empty() {
        out.push(("text".into(), text));
    }
}

/// The children of a host-text element ([`host_edges`]).
fn stream_host(
    tree: &Tree,
    opts: &Options,
    el: &Element,
    edges: bool,
    fenced: bool,
    out: &mut Vec<Tok>,
) {
    let (open, nodes, close) = match (el.open_tag(), el.close_tag()) {
        (Some(open), Some(close)) => (Some(open), el.body(), Some(close)),
        _ => (None, &el.children[..], None),
    };
    if let Some(open) = open {
        stream_element(tree, opts, open, fenced, out);
    }
    let mut buf = String::new();
    let mut strip_left = edges;
    for n in nodes {
        match n {
            Node::Token(t)
                if matches!(
                    t.kind,
                    TokenKind::Text | TokenKind::Whitespace | TokenKind::Newline
                ) =>
            {
                buf.push_str(tree.text(t));
            }
            Node::Token(t) => {
                flush_text(&mut buf, strip_left, false, out);
                strip_left = false;
                stream_token(tree, opts, t, out);
            }
            Node::Element(e) => {
                let strips = strips_around(tree, e);
                flush_text(&mut buf, strip_left, strips, out);
                strip_left = strips;
                stream_element(tree, opts, e, fenced, out);
            }
        }
    }
    flush_text(&mut buf, strip_left, edges, out);
    if let Some(close) = close {
        stream_element(tree, opts, close, fenced, out);
    }
}

/// The `islands.*` extension of an island oxc would take: a pure one, or
/// one [`interpolated`].
fn formatted_island(
    tree: &Tree,
    opts: &Options,
    el: &Element,
    fenced: bool,
) -> Option<&'static str> {
    let ElementKind::Island(island) = &el.kind else {
        return None;
    };
    if fenced || !(el.is_pure_island() || interpolated(tree, el)) {
        return None;
    }
    let target = cfformat::islands::dispatch(island).filter(|t| t.enabled(opts))?;
    if target.lang == Lang::Json && !el.is_pure_island() {
        return None;
    }
    let text = tree.slice(el.span.clone());
    (!text.trim().is_empty()).then_some(target.ext)
}

/// Whether `el`, an island that is not pure, holds only what the printer
/// stands in for: text with no `#`, `##`, and `#…#` with both delimiters.
/// Whether the printer then hands it off (its `#…#` on one line, text
/// outside them) or prints it as written, the streams agree: either way
/// its `##` and `#…#` are there, in order.
fn interpolated(tree: &Tree, el: &Element) -> bool {
    !el.is_pure_island()
        && el.children.iter().all(|n| match n {
            Node::Token(t) => match t.kind {
                TokenKind::Literal(cfparse::Literal::EscapeHash) => true,
                TokenKind::Text | TokenKind::Whitespace | TokenKind::Newline => {
                    !tree.text(t).contains('#')
                }
                _ => false,
            },
            Node::Element(e) => {
                e.kind == ElementKind::TemplateExpression && e.open.is_some() && e.close.is_some()
            }
        })
}

/// The text a test hands a parser for an [`interpolated`] island: each
/// `##` as `#` and each `#…#` as `cfhole` and its number, the same for an
/// input and its output whatever each `#…#` prints as.
fn stand_in(tree: &Tree, el: &Element) -> String {
    let mut out = String::new();
    let mut holes = 0;
    for n in &el.children {
        match n {
            Node::Token(t) if t.kind == TokenKind::Literal(cfparse::Literal::EscapeHash) => {
                out.push('#')
            }
            Node::Token(t) => out.push_str(tree.text(t)),
            Node::Element(_) => {
                out.push_str(&format!("cfhole{holes}"));
                holes += 1;
            }
        }
    }
    out
}

fn stream_element(tree: &Tree, opts: &Options, el: &Element, fenced: bool, out: &mut Vec<Tok>) {
    if el.kind.is_comment() {
        // `alignment.doc_comments` rewrites a doc comment's tag lines (their
        // padding, `@returns` as `@return`, their order): only its presence
        // is compared.
        let text = if el.kind == ElementKind::DocComment && opts.alignment_doc_comments {
            String::new()
        } else {
            comment_text(tree, el)
        };
        out.push(("comment".into(), text));
        return;
    }
    if let Some(ext) = formatted_island(tree, opts, el, fenced) {
        out.push(("island".into(), ext.into()));
        if !el.is_pure_island() {
            for n in &el.children {
                match n {
                    Node::Token(t)
                        if t.kind == TokenKind::Literal(cfparse::Literal::EscapeHash) =>
                    {
                        stream_token(tree, opts, t, out)
                    }
                    Node::Token(_) => {}
                    Node::Element(e) => stream_element(tree, opts, e, fenced, out),
                }
            }
        }
        return;
    }
    let fenced = fenced || el.kind == ElementKind::TagIsland;
    if let ElementKind::String { quote, .. } = el.kind {
        let q = match quote {
            cfparse::Quote::Single => "'",
            cfparse::Quote::Double => "\"",
        };
        let mut text = String::new();
        let flush = |text: &mut String, out: &mut Vec<Tok>| {
            if !text.is_empty() {
                out.push(("string".into(), text.replace(&q.repeat(2), q)));
                text.clear();
            }
        };
        for n in &el.children {
            match n {
                Node::Token(t) => text.push_str(tree.text(t)),
                Node::Element(e) => {
                    flush(&mut text, out);
                    stream_element(tree, opts, e, fenced, out);
                }
            }
        }
        flush(&mut text, out);
        return;
    }
    if is_preformatted(tree, el) {
        let (open, close) = (el.open_tag().unwrap(), el.close_tag().unwrap());
        stream_element(tree, opts, open, fenced, out);
        let body = tree.slice(open.span.end..close.span.start);
        out.push(("pre".into(), body.to_string()));
        stream_element(tree, opts, close, fenced, out);
        return;
    }
    if let Some(edges) = host_edges(tree, el) {
        stream_host(tree, opts, el, edges, fenced, out);
        return;
    }
    let mut parts: Vec<(u32, Part<'_>)> = Vec::new();
    if let Some(t) = &el.open {
        parts.push((t.span.start, Part::Token(t)));
    }
    for n in &el.children {
        parts.push((n.span().start, Part::Node(n)));
    }
    for item in &el.items {
        let mut empty = true;
        for n in item.nodes() {
            empty &= n.is_trivia();
            parts.push((n.span().start, Part::Node(n)));
        }
        // The comma after an empty item (`[1,,2]`) goes with the item: the
        // printer drops both, as the golden fixtures expect.
        if let Some(t) = item.separator.as_ref().filter(|_| !empty) {
            parts.push((t.span.start, Part::Token(t)));
        }
    }
    if let Some(t) = &el.close {
        parts.push((t.span.start, Part::Close(t)));
    }
    parts.sort_by_key(|p| p.0);
    // Where the source ends before the part at hand, in an HTML tag.
    let mut html_end = matches!(el.kind, ElementKind::HtmlTag(_)).then_some(None);
    for (i, (_, part)) in parts.iter().enumerate() {
        if let (Some(end), Part::Node(n)) = (html_end.as_mut(), part) {
            if !n.is_trivia() {
                if glues_in_html_tag(n) && *end == Some(n.span().start) {
                    out.push(("glued".into(), String::new()));
                }
                *end = Some(n.span().end);
            }
        }
        match *part {
            // A trailing comma: nothing but trivia before the closing
            // delimiter or the element's end.
            Part::Token(t) | Part::Node(Node::Token(t))
                if t.kind == TokenKind::Punct(Punct::Comma)
                    && parts[i + 1..].iter().all(|(_, p)| match p {
                        Part::Close(_) => true,
                        Part::Node(n) => n.is_trivia(),
                        Part::Token(_) => false,
                    }) => {}
            Part::Token(t) | Part::Close(t) | Part::Node(Node::Token(t)) => {
                stream_token(tree, opts, t, out)
            }
            Part::Node(Node::Element(e)) => stream_element(tree, opts, e, fenced, out),
        }
    }
}

/// Whether `n`, in an HTML tag's attribute list, is a node the printer
/// keeps glued to what precedes it when the source has it so and separates
/// from it otherwise: anything but the tag's name, an attribute or a
/// comment (a CF tag among the attributes, `#expr#`). On the page a space
/// before `<cfif x> class="a"</cfif>` is a space more beside the one its
/// body holds, and none before `class=a<cfif x>b</cfif>` is part of the
/// value.
fn glues_in_html_tag(n: &Node) -> bool {
    match n {
        Node::Element(e) => !e.kind.is_comment() && e.kind != ElementKind::KeyValue,
        Node::Token(t) => !matches!(
            t.kind,
            TokenKind::Ident(Ident::TagName | Ident::AttributeName)
                | TokenKind::Punct(Punct::Prefix)
        ),
    }
}

enum Part<'a> {
    Token(&'a Token),
    Node(&'a Node),
    /// The element's closing delimiter.
    Close(&'a Token),
}

fn stream_token(tree: &Tree, opts: &Options, t: &Token, out: &mut Vec<Tok>) {
    let text = tree.text(t);
    let (kind, text) = match t.kind {
        TokenKind::Whitespace | TokenKind::Newline => return,
        TokenKind::Punct(Punct::KeyValue) => (t.kind.name(), ":".to_string()),
        TokenKind::Ident(Ident::StructKey) => ("string".to_string(), text.to_string()),
        // Built-in, user-defined and method call names compare as one kind
        // in lower case: the casing options change a name's letters.
        TokenKind::Ident(Ident::Builtin | Ident::Call) => ("call".to_string(), text.to_lowercase()),
        TokenKind::Ident(Ident::TagName) if opts.tags_lowercase => {
            (t.kind.name(), text.to_lowercase())
        }
        // An HTML comment (`<!-- … -->`) is loose tokens, not an element, and
        // the tag printer re-indents its lines exactly as it re-indents text;
        // text outside host text (an ignored region, a recovered one) is
        // compared the same way.
        TokenKind::Operator(_)
        | TokenKind::Keyword(_)
        | TokenKind::Text
        | TokenKind::CommentText => {
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return;
            }
            (t.kind.name(), text)
        }
        // An `Other` or `Invalid` token can end with the line's
        // newline, and the last one in a file gains the newline every output
        // ends with.
        TokenKind::Other | TokenKind::Invalid => {
            (t.kind.name(), text.trim_end_matches('\n').to_string())
        }
        _ => (t.kind.name(), text.to_string()),
    };
    out.push((kind, text));
}

/// Top-level statements of a tree.
pub fn statement_count(tree: &Tree) -> usize {
    tree.root
        .children
        .iter()
        .filter(|n| n.as_element().is_some_and(|e| e.kind.is_statement()))
        .count()
}

/// Texts of the tokens of `kind` in `el` and below.
pub fn tokens_of_kind(tree: &Tree, el: &Element, kind: TokenKind, out: &mut Vec<String>) {
    let tokens = el
        .open
        .iter()
        .chain(&el.close)
        .chain(el.items.iter().filter_map(|i| i.separator.as_ref()));
    for t in tokens {
        if t.kind == kind {
            out.push(tree.text(t).trim_end_matches('\n').to_string());
        }
    }
    for n in el.nodes() {
        match n {
            Node::Token(t) if t.kind == kind => {
                out.push(tree.text(t).trim_end_matches('\n').to_string())
            }
            Node::Token(_) => {}
            Node::Element(e) => tokens_of_kind(tree, e, kind, out),
        }
    }
}

/// Reparse and token preservation for one formatted source: the output
/// reparses with no `invalid` or `other` token the input did not have and the
/// same number of top-level statements; the normalised token streams of
/// input and output are equal; the comment count is unchanged. Returns the
/// problems found.
pub fn check_output(src: &str, out: &str, mode: Mode, opts: &Options) -> Vec<String> {
    let mut problems = Vec::new();
    let before = cfparse::parse_source(src, mode);
    let after = cfparse::parse_source(out, mode);
    // A few corpus inputs already hold `invalid` tokens (unparseable code the
    // formatter passes through); only tokens the input did not have count.
    for kind in [TokenKind::Invalid, TokenKind::Other] {
        let (mut t_in, mut t_out) = (Vec::new(), Vec::new());
        tokens_of_kind(&before, &before.root, kind, &mut t_in);
        tokens_of_kind(&after, &after.root, kind, &mut t_out);
        for t in &t_in {
            if let Some(i) = t_out.iter().position(|o| o == t) {
                t_out.swap_remove(i);
            }
        }
        if !t_out.is_empty() {
            problems.push(format!(
                "output has new `{}` tokens: {t_out:?}",
                kind.name()
            ));
        }
    }
    let (s_in, s_out) = (statement_count(&before), statement_count(&after));
    if s_in != s_out {
        problems.push(format!("{s_in} top-level statements in, {s_out} out"));
    }
    let (c_in, c_out) = (comment_count(&before.root), comment_count(&after.root));
    if c_in != c_out {
        problems.push(format!("{c_in} comments in, {c_out} out"));
    }
    // Comments are compared on their own, in order: the printer moves a
    // line comment across a comma or an operator (to the end of the line it
    // trails), so their places among the other tokens are not kept.
    let split =
        |t: Vec<Tok>| -> (Vec<Tok>, Vec<Tok>) { t.into_iter().partition(|t| t.0 != "comment") };
    let (t_in, comments_in) = split(token_stream(&before, opts));
    let (t_out, comments_out) = split(token_stream(&after, opts));
    if comments_in != comments_out {
        let at = comments_in
            .iter()
            .zip(&comments_out)
            .position(|(a, b)| a != b)
            .unwrap_or(comments_in.len().min(comments_out.len()));
        problems.push(format!(
            "comment {at} differs: in {:?} / out {:?}",
            comments_in.get(at),
            comments_out.get(at)
        ));
    }
    if t_in != t_out {
        let at = t_in
            .iter()
            .zip(&t_out)
            .position(|(a, b)| a != b)
            .unwrap_or(t_in.len().min(t_out.len()));
        let lo = at.saturating_sub(3);
        problems.push(format!(
            "token stream differs at {at}: in {:?} / out {:?}",
            &t_in[lo..(at + 3).min(t_in.len())],
            &t_out[lo..(at + 3).min(t_out.len())]
        ));
    }
    problems
}

/// The texts of the JavaScript islands of `tree` a formatter takes (pure,
/// non-blank, outside a code fence, dispatched to an enabled `islands.js`;
/// or holding `#…#`, as [`stand_in`] writes them), in source order.
pub fn handed_off_js(tree: &Tree, opts: &Options) -> Vec<String> {
    fn walk(tree: &Tree, opts: &Options, el: &Element, fenced: bool, out: &mut Vec<String>) {
        if formatted_island(tree, opts, el, fenced).is_some_and(|ext| ext == "js" || ext == "mjs") {
            out.push(match el.is_pure_island() {
                true => tree.slice(el.span.clone()).to_owned(),
                false => stand_in(tree, el),
            });
            return;
        }
        let fenced = fenced || el.kind == ElementKind::TagIsland;
        for n in el.nodes() {
            if let Node::Element(e) = n {
                walk(tree, opts, e, fenced, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, opts, &tree.root, false, &mut out);
    out
}

/// The literal-preservation invariant: for every JavaScript island
/// handed off, in input and output order, the multiset of template-literal
/// quasi texts is unchanged, and so is the multiset of block comments with
/// each line's leading whitespace trimmed (the formatter re-aligns a JSDoc
/// comment, not its text), and a JSDoc comment's trailing whitespace too
/// (the formatter drops it, as prettier does). Returns the islands checked
/// and the problems.
pub fn check_literals(src: &str, out: &str, mode: Mode, opts: &Options) -> (usize, Vec<String>) {
    let before = handed_off_js(&cfparse::parse_source(src, mode), opts);
    let after = handed_off_js(&cfparse::parse_source(out, mode), opts);
    if before.len() != after.len() {
        return (
            0,
            vec![format!(
                "{} JavaScript islands in, {} out",
                before.len(),
                after.len()
            )],
        );
    }
    let (mut checked, mut problems) = (0, Vec::new());
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        // An island the formatter refused is printed verbatim; one that
        // does not parse has no literals to compare.
        let (Some(a), Some(b)) = (
            cfformat::islands::literal_texts(a),
            cfformat::islands::literal_texts(b),
        ) else {
            continue;
        };
        checked += 1;
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        // A comment whose lines after the first all start with `*` is one
        // the formatter re-aligns, as prettier does, trailing whitespace
        // dropped with the leading; any other block comment prints raw, its
        // lines' ends as written.
        let comments = |v: Vec<String>| {
            sorted(
                v.iter()
                    .map(|c| {
                        let aligned = c.lines().skip(1).all(|l| l.trim_start().starts_with('*'));
                        c.lines()
                            .map(|l| if aligned { l.trim() } else { l.trim_start() })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .collect(),
            )
        };
        if sorted(a.quasis.clone()) != sorted(b.quasis.clone()) {
            problems.push(format!(
                "island {i}: template literals differ: in {:?} / out {:?}",
                a.quasis, b.quasis
            ));
        }
        if comments(a.comments.clone()) != comments(b.comments.clone()) {
            problems.push(format!(
                "island {i}: block comments differ: in {:?} / out {:?}",
                a.comments, b.comments
            ));
        }
    }
    (checked, problems)
}

/// What [`check_preserved`] compared.
#[derive(Debug, Default, Clone, Copy)]
pub struct Preserved {
    /// Islands of no language (`Lang::Unknown`) whose text was compared.
    pub opaque: usize,
    /// `<cfquery>` islands whose SQL literals were compared.
    pub sql: usize,
    /// Java blocks and `<cfjava>` islands whose Java literals were compared.
    pub java: usize,
}

impl std::ops::AddAssign for Preserved {
    fn add_assign(&mut self, o: Self) {
        self.opaque += o.opaque;
        self.sql += o.sql;
        self.java += o.java;
    }
}

/// The exact-preservation invariants, each over the input's and the
/// output's islands in source order:
///
/// - a body of no language (`Lang::Unknown`) keeps its text byte for byte:
///   the concatenation of its text tokens, each CF element one marker (a
///   paired CF tag body's own text included, its tags markers), is the same;
/// - a `<cfquery>` keeps its SQL literals: the multiset of strings, quoted
///   identifiers and dollar quotes `cfformat::islands::sql_literal_texts`
///   finds in that text, under each dialect reading apart (one is not
///   enough: MySQL's `'a\'` ⏎ `b'` is one literal, and PostgreSQL's
///   reading has that newline between two, where a shift would not show);
/// - a Java block (`type="java"`) or `<cfjava>` keeps its Java literals:
///   the multiset of strings, chars and text blocks [`java_literal_end`]
///   finds outside comments.
///
/// Returns the islands compared and the problems.
pub fn check_preserved(src: &str, out: &str, mode: Mode) -> (Preserved, Vec<String>) {
    let (before, after) = (
        preserved_texts(&cfparse::parse_source(src, mode)),
        preserved_texts(&cfparse::parse_source(out, mode)),
    );
    let mut counts = Preserved::default();
    let mut problems = Vec::new();
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    for (what, a, b, count) in [
        ("opaque", &before.0, &after.0, &mut counts.opaque),
        ("SQL", &before.1, &after.1, &mut counts.sql),
        ("Java", &before.2, &after.2, &mut counts.java),
    ] {
        if a.len() != b.len() {
            problems.push(format!("{} {what} islands in, {} out", a.len(), b.len()));
            continue;
        }
        for (i, (a, b)) in a.iter().zip(b).enumerate() {
            *count += 1;
            let (la, lb) = match what {
                "opaque" => (vec![a.clone()], vec![b.clone()]),
                "SQL" => (sql_literals(a), sql_literals(b)),
                _ => (sorted(java_literals(a)), sorted(java_literals(b))),
            };
            if la != lb {
                problems.push(format!(
                    "{what} island {i}: text differs: in {la:?} / out {lb:?}"
                ));
            }
        }
    }
    (counts, problems)
}

/// Every reading's SQL literals, each list sorted.
fn sql_literals(text: &str) -> Vec<String> {
    cfformat::islands::sql_literal_texts(text)
        .into_iter()
        .enumerate()
        .flat_map(|(reading, mut texts)| {
            texts.sort();
            texts.into_iter().map(move |t| format!("{reading}: {t}"))
        })
        .collect()
}

/// The text [`check_preserved`] compares for every island of no language,
/// every `<cfquery>` and every Java block or `<cfjava>`, in source order.
fn preserved_texts(tree: &Tree) -> (Vec<String>, Vec<String>, Vec<String>) {
    /// Stands for a CF element: not a quote of any language.
    const MARKER: char = '\u{1}';
    fn text(tree: &Tree, nodes: &[Node], out: &mut String) {
        for n in nodes {
            match n {
                Node::Token(t) => out.push_str(tree.text(t)),
                Node::Element(e) if matches!(e.kind, ElementKind::TagBody { .. }) => {
                    match (e.open_tag(), e.close_tag()) {
                        (Some(_), Some(_)) => {
                            out.push(MARKER);
                            text(tree, e.body(), out);
                            out.push(MARKER);
                        }
                        _ => out.push(MARKER),
                    }
                }
                Node::Element(_) => out.push(MARKER),
            }
        }
    }
    fn walk(tree: &Tree, el: &Element, out: &mut (Vec<String>, Vec<String>, Vec<String>)) {
        let slot = match &el.kind {
            ElementKind::Island(i) if i.lang == Lang::Unknown => Some(&mut out.0),
            ElementKind::Island(i) if i.site == IslandSite::CfQuery => Some(&mut out.1),
            ElementKind::Island(i) if i.site == IslandSite::CfJava => Some(&mut out.2),
            ElementKind::Block(BlockKind::Java) => Some(&mut out.2),
            _ => None,
        };
        if let Some(slot) = slot {
            let mut s = String::new();
            text(tree, &el.children, &mut s);
            slot.push(s);
            return;
        }
        for n in el.nodes() {
            if let Node::Element(e) = n {
                walk(tree, e, out);
            }
        }
    }
    let mut out = Default::default();
    walk(tree, &tree.root, &mut out);
    out
}

/// The Java literals of `text` — `"…"`, `'…'` and `"""…"""`, delimiters
/// included — outside `//` and `/* */` comments, as `java_block` skips them.
fn java_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        at = if rest.starts_with("//") {
            rest.find('\n').map_or(text.len(), |n| at + n)
        } else if let Some(body) = rest.strip_prefix("/*") {
            body.find("*/").map_or(text.len(), |n| at + 2 + n + 2)
        } else if rest.starts_with(['"', '\'']) {
            let end = java_literal_end(text, at);
            out.push(text[at..end].to_owned());
            end
        } else {
            at + rest.chars().next().map_or(1, char::len_utf8)
        };
    }
    out
}

/// `cfparse`' `scan::java_literal_end` (private there), restated: the end
/// of the Java string, char or text block opening at `from`, past its
/// closing delimiter; `\` escapes the next character; a `"…"` or `'…'`
/// ends at its line's newline, an open text block at the end.
fn java_literal_end(src: &str, from: usize) -> usize {
    let b = src.as_bytes();
    let triple = src[from..].starts_with("\"\"\"");
    let quote = b[from];
    let mut at = from + if triple { 3 } else { 1 };
    while at < b.len() {
        match b[at] {
            b'\\' => at += 2,
            b'"' if triple && src[at..].starts_with("\"\"\"") => return at + 3,
            b'\n' if !triple => return at,
            c if c == quote && !triple => return at + 1,
            _ => at += 1,
        }
    }
    src.len()
}

/// The 1-based lines of `out` that may keep trailing whitespace: every
/// line of an island whose text may hold a string spanning lines
/// (`keeps_literal_text`, or a backtick — a template literal the oxc path
/// splices as written) and every line of a `<pre>` / `<textarea>` body
/// (printed as written), found by reparsing `out`.
pub fn literal_island_lines(out: &str, mode: Mode) -> std::collections::HashSet<usize> {
    fn walk(tree: &Tree, el: &Element, out: &mut std::collections::HashSet<usize>) {
        if is_preformatted(tree, el) {
            out.extend(tree.line_of(el.span.start)..=tree.line_of(el.span.end));
            return;
        }
        if let ElementKind::Island(island) = &el.kind {
            let text = tree.slice(el.span.clone());
            if text.contains('`') || cfformat::islands::keeps_literal_text(text, island.lang) {
                out.extend(tree.line_of(el.span.start)..=tree.line_of(el.span.end));
                return;
            }
        }
        for n in el.nodes() {
            if let Node::Element(e) = n {
                walk(tree, e, out);
            }
        }
    }
    let tree = cfparse::parse_source(out, mode);
    let mut lines = std::collections::HashSet::new();
    walk(&tree, &tree.root, &mut lines);
    lines
}

/// Whether `el` is a paired HTML `<pre>` or `<textarea>`, whose body the
/// tag printer prints as written.
pub fn is_preformatted(tree: &Tree, el: &Element) -> bool {
    el.kind == (ElementKind::TagBody { cf: false })
        && tree
            .tag_name(el)
            .is_some_and(|n| n.eq_ignore_ascii_case("pre") || n.eq_ignore_ascii_case("textarea"))
}

/// A settings object as a map, for callers building options by hand.
pub fn settings(json: &str) -> Map<String, Value> {
    match serde_json::from_str(json).unwrap() {
        Value::Object(m) => m,
        _ => panic!("not an object"),
    }
}
