//! The in-process island formatter (`islands.*: "oxc"`): the
//! oxc formatter crates format JavaScript, a module, JSON and CSS without a
//! process or anything on `PATH`. This is the only file that names an oxc
//! type; the crates' API is unstable upstream, so a bump touches this file
//! alone.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};

use cfparse::Lang;
use oxc_allocator::Allocator;
use oxc_ast::ast::{CommentKind, Program, StringLiteral, TemplateElement};
use oxc_ast::AstKind;
use oxc_ast_visit::Visit;
use oxc_formatter_core::{IndentWidth, LineEnding, LineWidth};
use oxc_span::SourceType;

use super::config::{
    ArrowParens, IslandConfig, ObjectWrap, OperatorPosition, QuoteProps, TrailingComma,
};
use super::holes::Site;
use super::{
    nesting_depth, FormattedIsland, IslandFormatter, IslandRequest, Refused, ISLAND_STACK,
    ISLAND_THREAD, NESTING_LIMIT, SIZE_LIMIT,
};
use crate::IndentStyle;

/// Formats an island with oxc, picking the language from the synthetic
/// path's extension: `js` (script or module, whichever the text is), `mjs`,
/// `json` or `css`. Stateless: one allocator per request.
///
/// A parse error is [`Refused`] with oxc's one-line message, and
/// so is a panic inside oxc (`internal error: …`). A text over
/// [`SIZE_LIMIT`] bytes or with brackets nested deeper than
/// [`NESTING_LIMIT`] outside its strings and comments ([`nesting_depth`])
/// is refused before oxc sees it (`300000 bytes, over the limit of
/// 262144`, `nested 501 levels deep, over the limit of 500`); so is a
/// JavaScript text whose parsed tree is deeper than [`NESTING_LIMIT`],
/// after the parse and before the formatter. oxc runs on a thread of its
/// own with an [`ISLAND_STACK`] stack.
#[derive(Debug, Clone, Copy, Default)]
pub struct Oxc;

impl IslandFormatter for Oxc {
    fn format(&self, req: &IslandRequest) -> Result<String, Refused> {
        on_island_thread(req, format_caught)
    }

    /// [`Oxc::format`] and the [`literal_lines`](super::literal_lines) of
    /// its text, both on the one island thread.
    fn format_island(&self, req: &IslandRequest) -> Result<FormattedIsland, Refused> {
        on_island_thread(req, |req| {
            let text = format_caught(req)?;
            let literal_lines = literal_lines_here(&text, req.lang)?;
            Ok(FormattedIsland {
                text,
                literal_lines,
            })
        })
    }
}

/// `f` on the island thread ([`on_island_stack`]) after the two linear
/// refusals on the caller's thread: a text over [`SIZE_LIMIT`] bytes and one
/// nested deeper than [`NESTING_LIMIT`] ([`nesting_depth`]).
fn on_island_thread<T: Send>(
    req: &IslandRequest,
    f: impl FnOnce(&IslandRequest) -> Result<T, Refused> + Send,
) -> Result<T, Refused> {
    let size = req.text.len();
    if size > SIZE_LIMIT {
        return Err(Refused(format!(
            "{size} bytes, over the limit of {SIZE_LIMIT}"
        )));
    }
    let depth = nesting_depth(req.text, req.lang);
    if depth > NESTING_LIMIT {
        return Err(too_deep(depth));
    }
    on_island_stack(|| f(req))
        .unwrap_or_else(|e| Err(Refused(format!("cannot start the island thread: {e}"))))
}

/// [`format()`], a panic inside oxc caught as a refusal (`internal error: …`).
fn format_caught(req: &IslandRequest) -> Result<String, Refused> {
    catch_unwind(AssertUnwindSafe(|| format(req)))
        .unwrap_or_else(|payload| Err(Refused(internal_error(payload.as_ref()))))
}

/// Runs `f` on a new thread named [`ISLAND_THREAD`] with an
/// [`ISLAND_STACK`] stack and waits for it: oxc and the walks over its
/// trees recurse without a bound, so they get the same room on whatever
/// thread the caller is (a test's 2 MB, the CLI's 8 MB worker). An error
/// is the thread's failure to start; a panic in `f` resumes on the caller.
fn on_island_stack<T: Send>(f: impl FnOnce() -> T + Send) -> std::io::Result<T> {
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name(ISLAND_THREAD.into())
            .stack_size(ISLAND_STACK)
            .spawn_scoped(scope, f)?;
        Ok(handle
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload)))
    })
}

/// The refusal of a text nested `depth` levels deep, by its brackets or
/// by its tree.
fn too_deep(depth: usize) -> Refused {
    Refused(format!(
        "nested {depth} levels deep, over the limit of {NESTING_LIMIT}"
    ))
}

/// `internal error: ` and the panic's message, when it is a string.
fn internal_error(payload: &(dyn Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("panic");
    format!("internal error: {message}")
}

/// The three layout settings every language takes, in oxc's types. oxc's
/// ranges are narrower than cfformat's: `indent_width` 0–24 and `width`
/// 1–320; a value outside is clamped to the nearest end.
struct Layout {
    style: oxc_formatter_core::IndentStyle,
    indent: IndentWidth,
    width: LineWidth,
}

impl Layout {
    fn new(req: &IslandRequest) -> Self {
        let (style, indent) = match req.indent {
            IndentStyle::Spaces(n) => (oxc_formatter_core::IndentStyle::Space, n),
            IndentStyle::Tabs(n) => (oxc_formatter_core::IndentStyle::Tab, n),
        };
        let indent = indent.clamp(IndentWidth::MIN.into(), IndentWidth::MAX.into());
        let width = req
            .width
            .clamp(LineWidth::MIN.into(), LineWidth::MAX.into());
        Layout {
            style,
            indent: IndentWidth::try_from(indent as u8).unwrap_or_default(),
            width: LineWidth::try_from(width as u16).unwrap_or_default(),
        }
    }
}

/// The project configuration onto oxc's option structs, as oxfmt maps its
/// own (`apps/oxfmt/src/core/options/to_oxc_formatter.rs`,
/// `to_oxc_formatter_css.rs`, `to_oxc_formatter_json.rs` at the pinned tag):
/// each option the configuration sets replaces the default, the layout is
/// left alone. CSS takes `singleQuote` and `trailingComma`; JSON takes
/// `trailingComma`, `bracketSpacing`, `objectWrap`, `singleQuote` and
/// `quoteProps`.
mod apply {
    use super::*;

    fn quote(single: bool) -> oxc_formatter::QuoteStyle {
        if single {
            oxc_formatter::QuoteStyle::Single
        } else {
            oxc_formatter::QuoteStyle::Double
        }
    }

    pub(super) fn js(c: &IslandConfig, o: &mut oxc_formatter::JsFormatOptions) {
        use oxc_formatter::{
            ArrowParentheses, AttributePosition, BracketSameLine, BracketSpacing, Expand,
            OperatorPosition as Position, QuoteProperties, Semicolons, TrailingCommas,
        };
        if let Some(v) = c.single_quote {
            o.quote_style = quote(v);
        }
        if let Some(v) = c.jsx_single_quote {
            o.jsx_quote_style = quote(v);
        }
        if let Some(v) = c.quote_props {
            o.quote_properties = match v {
                QuoteProps::AsNeeded => QuoteProperties::AsNeeded,
                QuoteProps::Consistent => QuoteProperties::Consistent,
                QuoteProps::Preserve => QuoteProperties::Preserve,
            };
        }
        if let Some(v) = c.trailing_comma {
            o.trailing_commas = match v {
                TrailingComma::All => TrailingCommas::All,
                TrailingComma::Es5 => TrailingCommas::Es5,
                TrailingComma::None => TrailingCommas::None,
            };
        }
        if let Some(v) = c.semi {
            o.semicolons = if v {
                Semicolons::Always
            } else {
                Semicolons::AsNeeded
            };
        }
        if let Some(v) = c.arrow_parens {
            o.arrow_parentheses = match v {
                ArrowParens::Avoid => ArrowParentheses::AsNeeded,
                ArrowParens::Always => ArrowParentheses::Always,
            };
        }
        if let Some(v) = c.bracket_spacing {
            o.bracket_spacing = BracketSpacing::from(v);
        }
        if let Some(v) = c.bracket_same_line {
            o.bracket_same_line = BracketSameLine::from(v);
        }
        if let Some(v) = c.single_attribute_per_line {
            o.attribute_position = if v {
                AttributePosition::Multiline
            } else {
                AttributePosition::Auto
            };
        }
        if let Some(v) = c.object_wrap {
            o.expand = match v {
                ObjectWrap::Preserve => Expand::Auto,
                ObjectWrap::Collapse => Expand::Never,
            };
        }
        if let Some(v) = c.operator_position {
            o.operator_position = match v {
                OperatorPosition::Start => Position::Start,
                OperatorPosition::End => Position::End,
            };
        }
    }

    pub(super) fn css(c: &IslandConfig, o: &mut oxc_formatter_css::CssFormatOptions) {
        use oxc_formatter_css::{SingleQuote, TrailingCommas};
        if let Some(v) = c.single_quote {
            o.single_quote = SingleQuote::from(v);
        }
        if let Some(v) = c.trailing_comma {
            // `all` and `es5` are one for CSS.
            o.trailing_commas = match v {
                TrailingComma::All | TrailingComma::Es5 => TrailingCommas::Always,
                TrailingComma::None => TrailingCommas::Never,
            };
        }
    }

    pub(super) fn json(c: &IslandConfig, o: &mut oxc_formatter_json::JsonFormatOptions) {
        use oxc_formatter_json::{BracketSpacing, Expand, QuoteProps as Props, TrailingCommas};
        if let Some(v) = c.trailing_comma {
            // `all` and `es5` are one for JSON.
            o.trailing_commas = match v {
                TrailingComma::All | TrailingComma::Es5 => TrailingCommas::Always,
                TrailingComma::None => TrailingCommas::Never,
            };
        }
        if let Some(v) = c.bracket_spacing {
            o.bracket_spacing = BracketSpacing::from(v);
        }
        if let Some(v) = c.object_wrap {
            o.expand = match v {
                ObjectWrap::Preserve => Expand::Auto,
                ObjectWrap::Collapse => Expand::Never,
            };
        }
        if let Some(v) = c.single_quote {
            o.single_quote = v.into();
        }
        if let Some(v) = c.quote_props {
            o.quote_props = match v {
                QuoteProps::AsNeeded => Props::AsNeeded,
                QuoteProps::Consistent => Props::Consistent,
                QuoteProps::Preserve => Props::Preserve,
            };
        }
    }
}

/// Prints oxc's formatted document; a print error is a refusal.
macro_rules! print {
    ($formatted:expr) => {
        $formatted
            .print()
            .map(|printed| printed.as_code().to_owned())
            .map_err(|e| Refused(e.to_string()))
    };
}

/// The refusal for an oxc diagnostic: its one-line message.
macro_rules! rejected {
    ($diagnostic:expr) => {
        Refused($diagnostic.message.to_string())
    };
}

fn format(req: &IslandRequest) -> Result<String, Refused> {
    #[cfg(test)]
    if req.text == tests::PANIC {
        panic!("{}", tests::PANIC.trim_end());
    }
    let layout = Layout::new(req);
    let allocator = Allocator::default();
    let ext = req.path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "js" | "mjs" => {
            let source_type = if ext == "mjs" {
                SourceType::mjs()
            } else {
                SourceType::unambiguous()
            };
            let mut options = oxc_formatter::JsFormatOptions {
                indent_style: layout.style,
                indent_width: layout.indent,
                line_width: layout.width,
                line_ending: LineEnding::Lf,
                ..Default::default()
            };
            apply::js(&req.config, &mut options);
            // `oxc_formatter::format` with the tree's depth measured
            // between its parse and its formatter: any diagnostic refuses,
            // as there.
            let ret = oxc_formatter::parse_for_format(&allocator, req.text, source_type);
            if let Some(diagnostic) = ret.diagnostics.into_iter().next() {
                return Err(rejected!(diagnostic));
            }
            let program = allocator.alloc(ret.program);
            let depth = program_depth(program);
            if depth > NESTING_LIMIT {
                return Err(too_deep(depth));
            }
            print!(oxc_formatter::format_program(&allocator, program, options))
        }
        "json" => {
            let mut options = oxc_formatter_json::JsonFormatOptions {
                indent_style: layout.style,
                indent_width: layout.indent,
                line_width: layout.width,
                line_ending: LineEnding::Lf,
                ..Default::default()
            };
            apply::json(&req.config, &mut options);
            let formatted = oxc_formatter_json::format(&allocator, req.text, options)
                .map_err(|d| rejected!(d))?;
            print!(formatted)
        }
        "css" => {
            let mut options = oxc_formatter_css::CssFormatOptions {
                indent_style: layout.style,
                indent_width: layout.indent,
                line_width: layout.width,
                line_ending: LineEnding::Lf,
                ..Default::default()
            };
            apply::css(&req.config, &mut options);
            let formatted = oxc_formatter_css::format(&allocator, req.text, options)
                .map_err(|d| rejected!(d))?;
            print!(formatted)
        }
        _ => Err(Refused(format!("no oxc formatter for .{ext}"))),
    }
}

/// [`super::tree_depth`]: the depth of `text`'s tree, parsed as the walks
/// parse it (script, then module), on the island's thread; `None` when it
/// parses as neither or the thread cannot start.
pub(super) fn tree_depth(text: &str) -> Option<usize> {
    on_island_stack(|| {
        let allocator = Allocator::default();
        let ret = [SourceType::unambiguous(), SourceType::mjs()]
            .into_iter()
            .map(|source_type| oxc_formatter::parse_for_format(&allocator, text, source_type))
            .find(|ret| ret.diagnostics.is_empty())?;
        Some(program_depth(&ret.program))
    })
    .ok()
    .flatten()
}

/// The depth of a program's tree: the most nodes open at once, the
/// program included. Every node the visitor enters counts, whatever it is.
fn program_depth(program: &Program) -> usize {
    let mut depth = Depth::default();
    depth.visit_program(program);
    depth.deepest
}

/// The depth [`program_depth`] measures.
#[derive(Default)]
struct Depth {
    depth: usize,
    deepest: usize,
}

impl<'a> Visit<'a> for Depth {
    fn enter_node(&mut self, _: AstKind<'a>) {
        self.depth += 1;
        self.deepest = self.deepest.max(self.depth);
    }

    fn leave_node(&mut self, _: AstKind<'a>) {
        self.depth -= 1;
    }
}

/// [`super::literal_lines`]: the lines of `text` (a formatter's output
/// for an island) that start inside text the formatter prints as written.
/// For a JS island: a template-literal quasi, a string literal that spans
/// lines (a `\`-newline continuation), or a block comment the formatter
/// does not re-indent; parsed as [`format()`] parses (the formatter's own
/// `parse_for_format`), as a script or module, whichever parses, the parse
/// and the walk on the island's thread ([`on_island_stack`]). A text that
/// parses as neither, or a thread that cannot start, is a refusal: its
/// lines are unknown. For a CSS or JSON island: a block comment
/// ([`comment_literal_lines`]), found by a scan that cannot fail. A text of
/// one line has none.
pub(super) fn literal_lines(text: &str, lang: Lang) -> Result<Vec<usize>, Refused> {
    if !text.contains('\n') {
        return Ok(Vec::new());
    }
    if lang != Lang::Js {
        return Ok(comment_literal_lines(text, lang));
    }
    on_island_stack(|| js_literal_lines(text))
        .unwrap_or_else(|e| Err(Refused(format!("cannot start the island thread: {e}"))))
}

/// [`literal_lines`] on the calling thread, which must be the island's.
fn literal_lines_here(text: &str, lang: Lang) -> Result<Vec<usize>, Refused> {
    if !text.contains('\n') {
        return Ok(Vec::new());
    }
    if lang != Lang::Js {
        return Ok(comment_literal_lines(text, lang));
    }
    js_literal_lines(text)
}

/// [`literal_lines`] of a CSS or JSON text: the lines that start inside a
/// block comment the formatter prints as written. The CSS formatter prints
/// every comment so, as prettier does; the JSON formatter re-aligns one
/// whose lines after the first all start with `*` ([`indentable`]), as the
/// JavaScript formatter does, and prints any other raw. Indenting such a
/// line would add to it at every run. The comments are found by the scan
/// [`nesting_depth`] makes, which knows a string from a comment, with no
/// parse; neither language has another text that spans lines (a CSS
/// `\`-newline body is not handed off, and a JSON string holds no line
/// break). An unquoted `url(…)` body the scan reads over a line is literal
/// too: the formatter prints a URL on one line, so the scan took a `/*` in
/// one for a comment's start, and where that comment ends is not known.
fn comment_literal_lines(text: &str, lang: Lang) -> Vec<usize> {
    let mut spans = Vec::new();
    super::scan(text.as_bytes(), lang, &mut |site, range| {
        let body = &text[range.clone()];
        let raw = match site {
            Site::Comment => body.starts_with("/*") && (lang == Lang::Css || !indentable(body)),
            Site::Url => true,
            _ => false,
        };
        if raw && body.contains('\n') {
            spans.push((range.start as u32, range.end as u32 - 1));
        }
    });
    lines_starting_in(text, spans)
}

/// The 0-based lines of `text` that start inside one of `spans`, byte
/// ranges `(start, end)` of literal text: a line starting at `o` is
/// literal when `start < o <= end` (a line may start exactly where a quasi
/// ends, with its closing backtick: indenting it would still add to the
/// quasi).
fn lines_starting_in(text: &str, mut spans: Vec<(u32, u32)>) -> Vec<usize> {
    spans.sort_unstable();
    let mut out = Vec::new();
    for (i, o) in text
        .match_indices('\n')
        .map(|(at, _)| at as u32 + 1)
        .enumerate()
    {
        let before = spans.partition_point(|&(start, _)| start < o);
        if before > 0 && o <= spans[before - 1].1 {
            out.push(i + 1);
        }
    }
    out
}

/// [`literal_lines`] of a JavaScript text holding a newline.
fn js_literal_lines(text: &str) -> Result<Vec<usize>, Refused> {
    let allocator = Allocator::default();
    let parsed = [SourceType::unambiguous(), SourceType::mjs()]
        .into_iter()
        .map(|source_type| oxc_formatter::parse_for_format(&allocator, text, source_type))
        .find(|ret| ret.diagnostics.is_empty());
    let Some(ret) = parsed else {
        // oxc's own output parsed once already; another formatter's may
        // not, and nothing says which lines are literal.
        return Err(Refused(
            "internal error: the formatted text does not parse".into(),
        ));
    };
    let mut spans = Spans(Vec::new());
    spans.visit_program(&ret.program);
    for c in ret.program.comments.iter() {
        let body = &text[c.span.start as usize..c.span.end as usize];
        if c.kind == CommentKind::MultiLineBlock && !indentable(body) {
            spans.0.push((c.span.start, c.span.end - 1));
        }
    }
    Ok(lines_starting_in(text, spans.0))
}

/// [`super::literal_texts`]: the program's literal texts, or `None` when
/// `text` parses neither as a script nor as a module, or when the island's
/// thread ([`on_island_stack`]), where the parse and the walk run, cannot
/// start.
pub(super) fn literal_texts(text: &str) -> Option<super::LiteralTexts> {
    on_island_stack(|| js_literal_texts(text)).ok().flatten()
}

/// [`literal_texts`], on the island's thread.
fn js_literal_texts(text: &str) -> Option<super::LiteralTexts> {
    let allocator = Allocator::default();
    let ret = [SourceType::unambiguous(), SourceType::mjs()]
        .into_iter()
        .map(|source_type| oxc_formatter::parse_for_format(&allocator, text, source_type))
        .find(|ret| ret.diagnostics.is_empty())?;
    let slice = |start: u32, end: u32| text[start as usize..end as usize].to_owned();
    let mut texts = Texts::default();
    texts.visit_program(&ret.program);
    let comments = ret
        .program
        .comments
        .iter()
        .filter(|c| c.kind != CommentKind::Line)
        .map(|c| slice(c.span.start, c.span.end))
        .collect();
    Some(super::LiteralTexts {
        quasis: texts.quasis.into_iter().map(|(a, b)| slice(a, b)).collect(),
        comments,
    })
}

/// The quasi spans [`literal_texts`] collects.
#[derive(Default)]
struct Texts {
    quasis: Vec<(u32, u32)>,
}

impl<'a> Visit<'a> for Texts {
    fn visit_template_element(&mut self, it: &TemplateElement<'a>) {
        self.quasis.push((it.span.start, it.span.end));
    }
}

/// Prettier's "indentable block comment" (oxc's `is_alignable_comment`):
/// every line after the first starts, after its whitespace, with `*`. Such
/// a comment is re-aligned by the formatter; any other is printed raw.
fn indentable(comment: &str) -> bool {
    comment
        .split('\n')
        .skip(1)
        .all(|line| line.trim_start().starts_with('*'))
}

/// The literal spans of a program ([`literal_lines`]).
struct Spans(Vec<(u32, u32)>);

impl<'a> Visit<'a> for Spans {
    fn visit_template_element(&mut self, it: &TemplateElement<'a>) {
        self.0.push((it.span.start, it.span.end));
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        // Only a continuation spans lines; the quotes are outside.
        self.0.push((it.span.start, it.span.end - 1));
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::path::PathBuf;

    use cfparse::Lang;

    use super::*;
    use crate::islands::Islands;

    /// A text whose request panics inside the backend (the test hook in
    /// [`format`]).
    pub(in crate::islands) const PANIC: &str = "cfformat test: panic inside oxc\n";

    fn req(ext: &str, text: &str) -> IslandRequest<'static> {
        IslandRequest {
            lang: match ext {
                "json" => Lang::Json,
                "css" => Lang::Css,
                _ => Lang::Js,
            },
            text: Box::leak(text.to_owned().into_boxed_str()),
            path: PathBuf::from(format!("stdin.cfm.{ext}")),
            indent: IndentStyle::Spaces(2),
            width: 80,
            config: Default::default(),
        }
    }

    fn oxc(req: &IslandRequest) -> Result<String, Refused> {
        Oxc.format(req)
    }

    // The expectations below are prettier 3.9.6's output for the same text,
    // but where a comment says otherwise: `printf '…' | prettier
    // --stdin-filepath x.<ext> --tab-width 2 --print-width 80` (and
    // `--tab-width 4 --print-width 40`, `--use-tabs`, for the layout test).

    #[test]
    fn javascript() {
        let text = "var a = {b:1,c:[1,2]}\nif(a){b()}\nconst f = (x)=>x*2\n";
        assert_eq!(
            oxc(&req("js", text)).unwrap(),
            "var a = { b: 1, c: [1, 2] };\nif (a) {\n  b();\n}\nconst f = (x) => x * 2;\n"
        );
        // A script: HTML comment wrappers and sloppy code parse. (Not
        // prettier's output: prettier 3.9.6 refuses this text with
        // `SyntaxError: Unexpected token (1:1)`.)
        let text = "<!--\nalert('hi')\n//-->\n";
        assert_eq!(
            oxc(&req("js", text)).unwrap(),
            "<!--\nalert(\"hi\");\n//-->\n"
        );
        // `import` makes `.js` a module.
        let text = "import x from './x.js'\nexport const y = x\n";
        assert_eq!(
            oxc(&req("js", text)).unwrap(),
            "import x from \"./x.js\";\nexport const y = x;\n"
        );
    }

    #[test]
    fn module() {
        let text = "import { a } from \"./a.js\";\na()\n";
        assert_eq!(
            oxc(&req("mjs", text)).unwrap(),
            "import { a } from \"./a.js\";\na();\n"
        );
        // A module refuses an HTML comment.
        assert!(matches!(oxc(&req("mjs", "<!--\na();\n")), Err(Refused(_))));
    }

    #[test]
    fn json() {
        let text = "{\"a\": 1,\n  \"b\": [1, 2], // c\n\"d\": {\"e\": null},}\n";
        assert_eq!(
            oxc(&req("json", text)).unwrap(),
            "{\n  \"a\": 1,\n  \"b\": [1, 2], // c\n  \"d\": { \"e\": null }\n}\n"
        );
    }

    #[test]
    fn css() {
        let text =
            "body { margin: 0 }\n  .a{color:red;background:BLUE}\n.b { .c { color: red } }\n";
        assert_eq!(
            oxc(&req("css", text)).unwrap(),
            "body {\n  margin: 0;\n}\n.a {\n  color: red;\n  background: BLUE;\n}\n.b {\n  .c {\n    color: red;\n  }\n}\n"
        );
    }

    #[test]
    fn syntax_errors_are_refusals() {
        assert_eq!(
            oxc(&req("js", "SYNTAX ERROR\n")),
            Err(Refused(
                "Expected a semicolon or an implicit semicolon after a statement, but found none"
                    .into()
            ))
        );
        assert_eq!(
            oxc(&req("js", "var = ;\n")),
            Err(Refused("Unexpected token".into()))
        );
        assert!(matches!(
            oxc(&req("css", ".a { color: ")),
            Err(Refused(m)) if m.contains("unclosed block")
        ));
        assert!(matches!(oxc(&req("json", "{\"a\": }\n")), Err(Refused(_))));
    }

    #[test]
    fn indent_and_width_reach_the_output() {
        let text = "call(argumentNumberOne, argumentNumberTwo, argumentNumberThree);\n";
        let mut r = req("js", text);
        assert_eq!(oxc(&r).unwrap(), text);
        r.width = 40;
        r.indent = IndentStyle::Spaces(4);
        assert_eq!(
            oxc(&r).unwrap(),
            "call(\n    argumentNumberOne,\n    argumentNumberTwo,\n    argumentNumberThree,\n);\n"
        );
        r.indent = IndentStyle::Tabs(4);
        assert_eq!(
            oxc(&r).unwrap(),
            "call(\n\targumentNumberOne,\n\targumentNumberTwo,\n\targumentNumberThree,\n);\n"
        );
        // Out of oxc's range: clamped, not refused.
        r.indent = IndentStyle::Spaces(100);
        r.width = 10_000;
        assert_eq!(oxc(&r).unwrap(), text);
        r.width = 0;
        assert!(oxc(&r).is_ok());
    }

    #[test]
    fn no_crlf() {
        for (ext, text) in [
            ("js", "a()\r\nb()\r\n"),
            ("json", "{\"a\":\r\n1}\r\n"),
            ("css", ".a{color:red}\r\n.b{}\r\n"),
        ] {
            let out = oxc(&req(ext, text)).unwrap();
            assert!(!out.contains('\r'), "{ext}: {out:?}");
        }
    }

    #[test]
    fn a_panic_is_a_refusal() {
        // The default hook prints the panic; the panic hook is global, so it
        // is left alone rather than swapped under other tests.
        assert_eq!(
            oxc(&req("js", PANIC)),
            Err(Refused(
                "internal error: cfformat test: panic inside oxc".into()
            ))
        );
        assert_eq!(
            internal_error(&42_u8),
            "internal error: panic",
            "a payload that is not a string"
        );
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        // A parenthesis is no node, so 500 of them are a tree of 4, and 501
        // are the brackets' refusal, before the parse.
        let parens = |n: usize| format!("x = {}1{};\n", "(".repeat(n), ")".repeat(n));
        assert_eq!(oxc(&req("js", &parens(500))).unwrap(), "x = 1;\n");
        assert_eq!(
            oxc(&req("js", &parens(501))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
        let rules = |n: usize| format!("{}color:red;{}", ".a{".repeat(n), "}".repeat(n));
        assert!(oxc(&req("css", &rules(500))).is_ok());
        assert_eq!(
            oxc(&req("css", &rules(501))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
        // A closer in a comment or a string hides no opener: 500 levels
        // format (into 2 MB) and 501 are refused, as above.
        let masked = |n: usize| format!("{}color:red;{}", ".a{/* } */".repeat(n), "}".repeat(n));
        assert!(oxc(&req("css", &masked(500))).is_ok());
        assert_eq!(
            oxc(&req("css", &masked(501))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
        // Nor does a closer of another kind inside a function's
        // arguments, a token to the parser: 499 rules under the open
        // `foo(` format, 500 are refused at 501.
        let function =
            |n: usize| format!("{}color:red;{}", ".a{b:foo(});".repeat(n), "}".repeat(n));
        assert!(oxc(&req("css", &function(499))).is_ok());
        assert_eq!(
            oxc(&req("css", &function(500))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
        let masked = |n: usize| format!("{}0{}", "[\"]\",".repeat(n), "]".repeat(n));
        assert!(oxc(&req("json", &masked(500))).is_ok());
        assert_eq!(
            oxc(&req("json", &masked(501))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
    }

    #[test]
    fn tree_depth_counts_every_node() {
        use crate::islands::tree_depth;
        // The program, the statement, the assignment, its operands.
        assert_eq!(tree_depth("x = 1;"), Some(4));
        assert_eq!(tree_depth("x;"), Some(3));
        // An unbraced body, a label and a JSX element are a level each; an
        // object literal two (the object, the property); a parenthesis none.
        assert_eq!(tree_depth("if(a)if(a)if(a)x;"), Some(3 + 3));
        assert_eq!(tree_depth("a:a:a:x;"), Some(3 + 3));
        assert_eq!(tree_depth("x = {a:{a:{a:1}}};"), Some(4 + 6));
        assert_eq!(tree_depth("x = (((1)));"), Some(4));
        // (The element, its opening tag, the tag's name.)
        assert_eq!(tree_depth("x = <a>x</a>;"), Some(4 + 2));
        assert_eq!(tree_depth("x = <a><a><a>x</a></a></a>;"), Some(4 + 2 + 2));
        // Chains: an operator, a call or a member is a level.
        assert_eq!(tree_depth("x = !!!a;"), Some(4 + 3));
        assert_eq!(tree_depth("x = a()()();"), Some(4 + 3));
        assert_eq!(tree_depth("x = a + a + a + a;"), Some(4 + 3));
        // A module parses when a script does not.
        assert_eq!(tree_depth("import x from 'x';"), Some(4));
        assert_eq!(tree_depth("SYNTAX ERROR"), None);
    }

    #[test]
    fn a_tree_past_the_limit_is_refused() {
        // No bracket deeper than 1: the tree alone sees it.
        let ifs = |n: usize| format!("{}x;\n", "if(a)".repeat(n));
        let n = NESTING_LIMIT - 3;
        assert_eq!(nesting_depth(&ifs(n + 1), Lang::Js), 1);
        assert_eq!(crate::islands::tree_depth(&ifs(n)), Some(NESTING_LIMIT));
        assert!(oxc(&req("js", &ifs(n))).is_ok());
        assert_eq!(
            oxc(&req("js", &ifs(n + 1))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
        // A module too.
        assert_eq!(
            oxc(&req("mjs", &ifs(n + 1))),
            Err(Refused(
                "nested 501 levels deep, over the limit of 500".into()
            ))
        );
    }

    #[test]
    fn a_text_past_the_size_limit_is_refused() {
        // A chain has no bracket to count; the size bounds it. Refused
        // before oxc, so the case is cheap.
        let bangs = format!("x = {}a;\n", "!".repeat(SIZE_LIMIT + 1 - "x = a;\n".len()));
        assert_eq!(bangs.len(), 262_145);
        assert_eq!(
            oxc(&req("js", &bangs)),
            Err(Refused("262145 bytes, over the limit of 262144".into()))
        );
        // At the limit it is formatted: a string, which is cheap to format
        // in a debug build (a chain at the limit is not: `tests/islands_deep.rs`).
        let string = format!(
            "x = \"{}\";\n",
            "a".repeat(SIZE_LIMIT - "x = \"\";\n".len())
        );
        assert_eq!(string.len(), SIZE_LIMIT);
        assert!(oxc(&req("js", &string)).is_ok());
    }

    #[test]
    fn oxc_and_the_walks_run_on_the_island_thread() {
        let name = || std::thread::current().name().map(String::from);
        assert_eq!(
            on_island_stack(name).unwrap().as_deref(),
            Some(ISLAND_THREAD)
        );
        // A panic there resumes on the caller (the walks have no refusal to
        // turn it into; `Oxc` catches its own first).
        let caught = std::panic::catch_unwind(|| on_island_stack(|| panic!("in a walk")));
        assert!(caught.is_err());
    }

    #[test]
    fn an_unknown_extension_is_refused() {
        assert_eq!(
            oxc(&req("ts", "let a: number = 1;\n")),
            Err(Refused("no oxc formatter for .ts".into()))
        );
    }

    /// A configuration from its JSON, as a `.prettierrc` would hold it.
    fn config(json: &str) -> std::sync::Arc<crate::islands::IslandConfig> {
        let dir = std::env::temp_dir()
            .join(format!("cfformat-oxc-{}", std::process::id()))
            .join(format!("{:x}", {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                json.hash(&mut h);
                h.finish()
            }));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".prettierrc"), json).unwrap();
        let islands = Islands::new();
        let config = islands.project_config(
            &dir.join("page.cfm.js"),
            crate::options::IslandConfigMode::Auto,
        );
        assert!(islands.config_warnings().is_empty());
        config
    }

    // As above: prettier 3.9.6's output with the same options on its
    // command line (`--single-quote --no-semi --trailing-comma none
    // --arrow-parens avoid --quote-props consistent --no-bracket-spacing
    // --object-wrap collapse --experimental-operator-position start`).
    #[test]
    fn the_configuration_reaches_the_output() {
        let text = "const f = (x) => {return {\"a-b\": \"s\", c: [1,\n 2]}}\n";
        let mut r = req("js", text);
        r.config = config(
            r#"{"singleQuote": true, "semi": false, "arrowParens": "avoid",
                "quoteProps": "consistent", "bracketSpacing": false,
                "objectWrap": "collapse"}"#,
        );
        assert_eq!(
            oxc(&r).unwrap(),
            "const f = x => {\n  return {'a-b': 's', 'c': [1, 2]}\n}\n"
        );
        let mut r = req(
            "js",
            "call(argumentNumberOne, argumentNumberTwo, argumentNumberThree);\n",
        );
        r.width = 40;
        r.config = config(r#"{"trailingComma": "none"}"#);
        assert_eq!(
            oxc(&r).unwrap(),
            "call(\n  argumentNumberOne,\n  argumentNumberTwo,\n  argumentNumberThree\n);\n"
        );
        let mut r = req(
            "js",
            "total = argumentNumberOne + argumentNumberTwo + three;\n",
        );
        r.width = 40;
        r.config = config(r#"{"experimentalOperatorPosition": "start"}"#);
        assert_eq!(
            oxc(&r).unwrap(),
            "total =\n  argumentNumberOne\n  + argumentNumberTwo\n  + three;\n"
        );
        // CSS: `singleQuote` and `trailingComma` only.
        let mut r = req("css", ".a { content: \"x\"; }\n");
        r.config = config(r#"{"singleQuote": true, "semi": false}"#);
        assert_eq!(oxc(&r).unwrap(), ".a {\n  content: 'x';\n}\n");
        // JSON: `bracketSpacing` (prettier: `--no-bracket-spacing
        // --single-quote`), and a JSON file keeps its quotes.
        let mut r = req("json", "{\"a\": {\"b\": 1}}\n");
        r.config = config(r#"{"bracketSpacing": false, "singleQuote": true}"#);
        assert_eq!(oxc(&r).unwrap(), "{\"a\": {\"b\": 1}}\n");
    }

    #[test]
    fn the_configuration_is_in_the_key() {
        let islands = Islands::new();
        let a = req("js", "a('x')\n");
        let mut b = a.clone();
        b.config = config(r#"{"singleQuote": true}"#);
        let (first, _) = islands.format(&a);
        let (second, hit) = islands.format(&b);
        assert!(!hit.cached);
        assert_eq!(first.unwrap().text, "a(\"x\");\n");
        assert_eq!(second.unwrap().text, "a('x');\n");
        // The same configuration read twice is the same entry.
        let mut c = a.clone();
        c.config = config(r#"{"singleQuote": true}"#);
        assert!(islands.format(&c).1.cached);
    }

    #[test]
    fn the_path_is_not_in_the_key() {
        // The same island from two directories with the same (or no)
        // configuration is one run: the key is the extension, the indent,
        // the width, the configuration and the text.
        let islands = Islands::new();
        let mut a = req("js", "a()\n");
        a.path = PathBuf::from("/one/page.cfm.js");
        let mut b = a.clone();
        b.path = PathBuf::from("/two/page.cfm.js");
        let (first, hit) = islands.format(&a);
        assert!(!hit.cached);
        let (second, hit) = islands.format(&b);
        assert!(hit.cached);
        assert_eq!(first, second);
        assert_eq!(first.unwrap().text, "a();\n");
        let stats = islands.stats();
        assert_eq!((stats.formatted, stats.cached), (1, 1));
    }
}
