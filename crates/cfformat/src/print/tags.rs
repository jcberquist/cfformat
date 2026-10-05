//! Tag mode: tags, tag bodies, tag comments and code fences.
//!
//! A node list becomes *lines*: a `Newline` token ends a line, a `Text` token
//! contributes its own lines, an element contributes one doc. Each line loses
//! its leading and trailing whitespace (HTML's, ASCII: a no-break space is
//! text), at most one blank line survives between two content lines and
//! blank lines at the start and end are dropped. A body breaks when it holds
//! a tag or a newline, and then its lines are indented once; otherwise it
//! stays on the open tag's line. No column is tracked here: whether a group
//! fits is the doc printer's `fits`, and indentation is its `indent`.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::OnceLock;

use cfdoc::builders::{
    group, hardline, hardline_without_break_parent, if_break_group, indent, line,
    line_suffix_boundary, softline,
};
use cfdoc::utils::{find_in_doc, remove_lines};
use cfdoc::Doc;
use cfparse::{
    CfKind, Delim, Element, ElementKind, Ident, IslandSite, Node, Punct, SegmentKind, TagShape,
    Token, TokenKind,
};

use super::delimited::KeyValueStyle;
use super::Printer;
use crate::options::TagBodyIndent;

/// How the verbatim island path keeps an island's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verbatim {
    /// Every line keeps its indentation relative to the others and loses
    /// its trailing whitespace; the island is shifted right as a whole when
    /// it sits left of its floor ([`Printer::body_floor`]: one level inside
    /// the tag, or the tag's indent), and never left.
    Shift,
    /// Byte for byte: the island's text may hold a string or literal
    /// spanning lines ([`crate::islands::keeps_literal_text`]), so no line
    /// is shifted, trimmed or re-indented. A whitespace-only first or last
    /// line is dropped (no literal is open there) and the closing tag
    /// starts its own line.
    Raw,
    /// Exactly the source's bytes: a body of no language
    /// ([`cfparse::Lang::Unknown`]) keeps every line, a whitespace-only
    /// first or last one included, and the closing tag follows its last
    /// line directly.
    Exact,
}

/// Where a tag prints: how many indent levels deep it sits, and whether it is
/// inside a verbatim island.
///
/// `depth` is structural — a body that holds a tag always breaks, so the
/// count and the printed indentation agree — and exists only so the island
/// printer can turn it into columns (`depth * indent_size`) at doc-build
/// time: a `literalline` resets the indentation, so a preserved line has to
/// carry its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TagCtx {
    /// Indent levels between the document root and this node.
    pub depth: usize,
    /// Whether `depth` counts from column 0. A tag-mode document does; a code
    /// fence sits at an indentation only the doc printer knows, so an island
    /// inside one keeps its source columns exactly instead of being raised.
    pub rooted: bool,
    /// Inside a verbatim island (a `<cfquery>`, `<script>`, `<style>` or
    /// `<cfjava>` body, or an attribute value holding one), where a tag
    /// never breaks and a body keeps its own lines.
    pub island: bool,
    /// How an island's lines are kept ([`Verbatim`]); `Shift` outside one.
    pub verbatim: Verbatim,
    /// Whether the tag printer is the enclosing one, which is what makes a
    /// `<!--- --->` a tag comment rather than verbatim text (inside a
    /// `<cfscript>` body it is verbatim, as it is in a script file).
    pub tags: bool,
    /// Inside an HTML tag's attribute list, where a CF tag body's attributes
    /// (`<div <cfif x>id="y"</cfif>>`) are HTML attributes
    /// ([`KeyValueStyle::HtmlAttribute`]), not a CF tag's.
    pub html_attributes: bool,
    /// Whether whitespace put just before what prints in this context — a
    /// node list's start, or one of its elements — cannot show on the page:
    /// whitespace or a tag next to which the browser drops it is already
    /// there. A CF tag body reads it to know whether a line break at its
    /// start would add a space to the page ([`Printer::tag_body`]);
    /// [`Printer::tag_nodes`] sets it for each element it prints.
    pub space_before: bool,
    /// The same just after it.
    pub space_after: bool,
}

impl TagCtx {
    /// A tag at `depth` columns from the start of a line, outside any island.
    pub(crate) fn base(depth: usize) -> Self {
        TagCtx {
            depth,
            rooted: true,
            island: false,
            verbatim: Verbatim::Shift,
            tags: true,
            html_attributes: false,
            space_before: true,
            space_after: true,
        }
    }

    /// A script document, where no tag printer is in play.
    pub(crate) fn script() -> Self {
        TagCtx {
            tags: false,
            ..TagCtx::base(0)
        }
    }

    /// The content of a code fence, whose own indentation is the script
    /// printer's.
    pub(crate) fn fence() -> Self {
        TagCtx {
            rooted: false,
            ..TagCtx::base(0)
        }
    }

    /// The least indentation an island line may have, in columns: this
    /// context's depth when it counts from column 0, else none (inside a
    /// code fence an island keeps its source columns). For an island the
    /// context is its body's floor ([`Printer::body_floor`]).
    pub(crate) fn least_columns(self, indent_size: usize) -> usize {
        if self.rooted {
            self.depth * indent_size
        } else {
            0
        }
    }

    /// The same context one level deeper.
    pub(crate) fn deeper(self) -> Self {
        TagCtx {
            depth: self.depth + 1,
            ..self
        }
    }
}

/// The HTML tag lists of `data/tags.json`, lower-case.
struct HtmlTags {
    /// HTML tags after which a line break is forced (`blockTags.html`,
    /// vendored from CommandBox).
    block: HashSet<String>,
    /// HTML tags that are not block tags but whose surrounding and edge
    /// whitespace a browser does not render either (`spacelessTags.html`,
    /// this project's own list): the document structure, the parts of a
    /// table, the entries of a list box.
    spaceless: HashSet<String>,
}

fn html_tags() -> &'static HtmlTags {
    static TAGS: OnceLock<HtmlTags> = OnceLock::new();
    TAGS.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Data {
            #[serde(rename = "blockTags")]
            block_tags: List,
            #[serde(rename = "spacelessTags")]
            spaceless_tags: List,
        }
        #[derive(serde::Deserialize)]
        struct List {
            html: Vec<String>,
        }
        let data: Data =
            serde_json::from_str(include_str!("../../data/tags.json")).expect("data/tags.json");
        HtmlTags {
            block: data.block_tags.html.into_iter().collect(),
            spaceless: data.spaceless_tags.html.into_iter().collect(),
        }
    })
}

/// `name` as the lower-case key of [`HtmlTags`]; borrowed when it is written
/// in lower case already, as HTML tags usually are.
fn lower(name: &str) -> Cow<'_, str> {
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(name.to_ascii_lowercase())
    } else {
        Cow::Borrowed(name)
    }
}

/// Whether whitespace at the edges of an HTML tag named `name`'s body, and
/// next to the tag, never shows on the page: a block tag or a spaceless one.
fn ignores_space(name: &str) -> bool {
    let name = lower(name);
    let tags = html_tags();
    tags.block.contains(name.as_ref()) || tags.spaceless.contains(name.as_ref())
}

/// A bare `<cfelse>` / `<cfelseif …>` splits its enclosing body into
/// segments, the else tag printing at the enclosing tag's indent between
/// them; a stray `</cfelse>` is only a bare closing tag.
fn is_else_tag(n: &Node) -> bool {
    n.as_element().is_some_and(|e| {
        matches!(e.kind, ElementKind::CfTag(shape, CfKind::Else | CfKind::ElseIf)
            if shape != TagShape::Close)
    })
}

/// One piece of a tag line: text (which can still be trimmed) or a printed
/// node.
enum Piece {
    Text(String),
    Doc(Doc),
}

/// A node list laid out as lines.
#[derive(Default)]
struct TagLines {
    out: Vec<Vec<Piece>>,
    cur: Vec<Piece>,
    space: bool,
}

impl TagLines {
    /// A `Whitespace` token: one space, but only between two things on one
    /// line.
    fn space(&mut self) {
        if !self.cur.is_empty() {
            self.space = true;
        }
    }

    fn flush_space(&mut self) {
        if self.space && !self.cur.is_empty() {
            self.cur.push(Piece::Text(" ".to_owned()));
        }
        self.space = false;
    }

    /// Text on the current line; leading whitespace goes at a line start.
    /// Whitespace here is HTML's, ASCII only: a no-break space is text the
    /// page shows.
    fn text(&mut self, s: &str) {
        let s = if self.cur.is_empty() {
            s.trim_start_matches(|c: char| c.is_ascii_whitespace())
        } else {
            s
        };
        if s.is_empty() {
            return;
        }
        self.flush_space();
        self.cur.push(Piece::Text(s.to_owned()));
    }

    /// Text that carries its own line breaks.
    fn text_lines(&mut self, s: &str) {
        let mut first = true;
        for part in s.split('\n') {
            if !first {
                self.newline();
            }
            first = false;
            self.text(part);
        }
    }

    fn doc(&mut self, d: Doc) {
        self.flush_space();
        self.cur.push(Piece::Doc(d));
    }

    /// Ends the current line, trimming its trailing whitespace.
    fn newline(&mut self) {
        while let Some(Piece::Text(t)) = self.cur.last_mut() {
            let trimmed = t.trim_end_matches(|c: char| c.is_ascii_whitespace());
            if trimmed.is_empty() {
                self.cur.pop();
            } else {
                if trimmed.len() != t.len() {
                    t.truncate(trimmed.len());
                }
                break;
            }
        }
        self.space = false;
        let line = std::mem::take(&mut self.cur);
        self.out.push(line);
    }

    /// The lines joined by hard lines: blank lines capped at one in a row,
    /// none at the start or the end.
    fn finish(mut self) -> Doc {
        self.newline();
        let mut parts = Vec::new();
        let mut blank = false;
        for line in self.out {
            if line.is_empty() {
                blank = !parts.is_empty();
                continue;
            }
            if !parts.is_empty() {
                parts.push(hardline());
                if blank {
                    parts.push(hardline());
                }
            }
            blank = false;
            parts.extend(line.into_iter().map(|p| match p {
                Piece::Text(t) => Doc::from(t),
                Piece::Doc(d) => d,
            }));
        }
        Doc::Concat(parts)
    }
}

impl Printer<'_> {
    /// The current tag context. The tag kinds are reached through
    /// [`Printer::element`], which takes no context, so the tag printers park
    /// it here while they recurse.
    pub(crate) fn tag_ctx(&self) -> TagCtx {
        self.tags.get()
    }

    /// Runs `f` with `ctx` as the current tag context.
    pub(crate) fn with_tag_ctx<R>(&self, ctx: TagCtx, f: impl FnOnce() -> R) -> R {
        let saved = self.tags.replace(ctx);
        let out = f();
        self.tags.set(saved);
        out
    }

    /// A tag-mode node list (the document root, a tag body, a code fence):
    /// the lines of [`TagLines`], with a break forced after an HTML tag body
    /// whose name is a block tag and after a CF tag body that whitespace
    /// follows, so the content after either starts its own line.
    pub(crate) fn tag_nodes(&self, nodes: &[Node], ctx: TagCtx) -> Doc {
        let mut lines = TagLines::default();
        for (i, n) in nodes.iter().enumerate() {
            match n {
                Node::Token(t) => match t.kind {
                    TokenKind::Newline => lines.newline(),
                    TokenKind::Whitespace => lines.space(),
                    // Text the parser could not classify (what follows
                    // `</cfcomponent>`) prints exactly as written: its
                    // indentation and line breaks are part of the token.
                    TokenKind::Other | TokenKind::Invalid => {
                        let text = self.tree.text(t);
                        let (text, newline) = match text.strip_suffix('\n') {
                            Some(text) => (text, true),
                            None => (text, false),
                        };
                        lines.doc(self.text(text));
                        if newline {
                            lines.newline();
                        }
                    }
                    // Host text, and the loose tokens of an HTML comment
                    // (`<!--`, its `comment` lines, `-->`, which the tree
                    // leaves bare): text, with its own line breaks.
                    _ => lines.text_lines(self.tree.text(t)),
                },
                // An island its tag body does not own (a misparse: the
                // whitespace a `<style>` embed left inside a `<cffunction>`)
                // is text.
                Node::Element(e) if matches!(e.kind, ElementKind::Island(_)) => {
                    lines.text_lines(self.tree.slice(e.span.clone()));
                }
                Node::Element(e) => {
                    // Whether whitespace next to this element would show:
                    // not where there is some already, nor next to a tag the
                    // browser drops it around, nor at an end of the list
                    // where the list's own context says so.
                    let quiet = |n: Option<&Node>, edge: bool, space: bool| match n {
                        None => edge,
                        Some(n) => {
                            space || n.as_element().is_some_and(|n| self.is_spaceless_html(n))
                        }
                    };
                    let before = i.checked_sub(1).map(|at| &nodes[at]);
                    let after = nodes.get(i + 1);
                    let at = TagCtx {
                        space_before: quiet(
                            before,
                            ctx.space_before,
                            before.is_some_and(|n| self.ends_with_space(n)),
                        ),
                        space_after: quiet(
                            after,
                            ctx.space_after,
                            after.is_some_and(|n| self.starts_with_space(n)),
                        ),
                        ..ctx
                    };
                    lines.doc(self.with_tag_ctx(at, || self.element(e)));
                    // What follows a CF tag body with no whitespace between
                    // them (`</cfif>more`) is glued to it on the page, so it
                    // stays glued here: a break would add a space. Next to
                    // a block tag the browser drops that whitespace anyway
                    // (`</cfscript><p>`, and after a block tag's body).
                    let glued = e.kind == (ElementKind::TagBody { cf: true })
                        && nodes.get(i + 1).is_some_and(|n| {
                            !self.starts_with_space(n)
                                && !n.as_element().is_some_and(|n| self.is_spaceless_html(n))
                        });
                    if !glued && self.forces_break(e) {
                        let next = nodes[i + 1..].iter().find(
                            |n| !matches!(n, Node::Token(t) if t.kind == TokenKind::Whitespace),
                        );
                        match next {
                            None => {}
                            Some(Node::Token(t)) if t.kind == TokenKind::Newline => {}
                            Some(_) => lines.newline(),
                        }
                    }
                }
            }
        }
        lines.finish()
    }

    /// Whether `n`, a node of a tag-mode node list, starts with whitespace
    /// (a whitespace or newline token, text such as ` more`).
    fn starts_with_space(&self, n: &Node) -> bool {
        self.tree
            .slice(n.span())
            .starts_with(|c: char| c.is_ascii_whitespace())
    }

    /// Whether `n`, a node of a tag-mode node list, ends with whitespace.
    fn ends_with_space(&self, n: &Node) -> bool {
        self.tree
            .slice(n.span())
            .ends_with(|c: char| c.is_ascii_whitespace())
    }

    /// Whether a line break follows this node even when the source has none.
    fn forces_break(&self, e: &Element) -> bool {
        match e.kind {
            ElementKind::TagBody { cf: true } => true,
            ElementKind::TagBody { cf: false } => self.is_block_html(e),
            _ => false,
        }
    }

    /// Whether `e` is an HTML block tag body.
    fn is_block_html(&self, e: &Element) -> bool {
        self.tree
            .tag_name(e)
            .is_some_and(|n| html_tags().block.contains(lower(n).as_ref()))
    }

    /// Whether `e` is an HTML tag, paired or bare, next to which whitespace
    /// does not show ([`ignores_space`]).
    fn is_spaceless_html(&self, e: &Element) -> bool {
        matches!(
            e.kind,
            ElementKind::TagBody { cf: false } | ElementKind::HtmlTag(_)
        ) && self.tree.tag_name(e).is_some_and(ignores_space)
    }

    /// A paired tag: `[open tag, body, close tag]`. The body breaks — its
    /// lines indented once between two hard lines — when it holds a tag or a
    /// newline, and stays on the open tag's line otherwise. A hard line at
    /// an end of the body reaches the page as whitespace, so there is one
    /// only where that cannot add a space to it ([`Edge`]): always for a
    /// block tag, where the source has whitespace for an inline HTML tag,
    /// and for a CF tag, which the page does not see, where
    /// [`Printer::cf_edges`] says so. Under
    /// `tags.body.indent: "cfml"` a broken paired CF tag body that starts with
    /// HTML (its first non-trivia node is not a CF tag) sits at the tag's own
    /// indent instead. A `<cfscript>` body is a script statement list and a
    /// body holding an island is printed by [`Printer::island_body`], each at
    /// the floor [`Printer::body_floor`] gives it; a body inside an island
    /// keeps its own lines.
    pub(crate) fn tag_body(&self, e: &Element, ctx: TagCtx) -> Doc {
        let (Some(open), Some(close)) = (e.open_tag(), e.close_tag()) else {
            return self.as_written(e);
        };
        let body = e.body();
        if ctx.island {
            return self.island_tag_body(e, ctx);
        }
        let open_doc = self.tag(open, ctx);
        let close_doc = self.tag(close, ctx);
        let name = self.tree.tag_name(e).unwrap_or_default();
        if body
            .iter()
            .any(|n| n.as_element().is_some_and(|c| owns_island(e, name, c)))
        {
            let floor = self.body_floor(e, ctx);
            return self.island_body(open_doc, body, close_doc, ctx, floor);
        }
        if is_preformatted(e, name) {
            // The browser shows a `<pre>` body's whitespace and submits a
            // `<textarea>` body as the control's value, so the text between
            // the two tags is printed exactly as written: no line trimmed,
            // re-indented or dropped, and the CF tags in it (still parsed:
            // they run on the server) left as they are. Only the tags
            // themselves are formatted.
            let text = self.tree.slice(open.span.end..close.span.start);
            return Doc::Concat(vec![open_doc, self.text(text), close_doc]);
        }
        if (name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style"))
            && body
                .iter()
                .find(|n| !n.is_trivia())
                .and_then(Node::as_element)
                .is_some_and(|c| {
                    matches!(
                        c.kind,
                        ElementKind::CfTag(..)
                            | ElementKind::TagBody { cf: true }
                            | ElementKind::TemplateExpression
                    ) || is_tag_comment(c)
                })
        {
            // A `<script>` / `<style>` whose body starts with CFML and holds
            // no island: the parser read the CF tag first and never opened
            // an island for the body, and a re-indented body can parse
            // otherwise (the tag no longer at column 0), so it prints as
            // written.
            return self.as_written(e);
        }
        if e.cf_kind() == Some(CfKind::Script) {
            // The statements sit at the body's floor: one level in by
            // default, the tag's own indent under
            // `tags.islands.indent: false`. The statement printers'
            // own count of the indentation (`at_indent`) is the floor's too,
            // so whatever reads it (the attribute alignment) agrees with what
            // prints. A `<!--- --->` in there is verbatim, as it is in a
            // script file.
            let floor = self.body_floor(e, ctx);
            let stmts = self.at_indent(floor.depth, || {
                self.with_tag_ctx(
                    TagCtx {
                        tags: false,
                        ..floor
                    },
                    || self.statements(body),
                )
            });
            let mut parts = vec![open_doc];
            if !stmts.is_empty() {
                parts.push(indent_to(ctx, floor, vec![hardline(), stmts]));
            }
            parts.push(hardline());
            parts.push(close_doc);
            return Doc::Concat(parts);
        }
        let broken = body
            .iter()
            .any(|n| n.as_element().is_some_and(is_tag_element))
            || self.tree.slice(body_span(body)).contains('\n');
        let mut parts = vec![open_doc];
        // Whitespace at either end of an inline HTML body
        // (`<span>hello </span>world`) or a CF body (transparent to the
        // browser: `<cfoutput>#a# </cfoutput>b`) is part of the page's text.
        // A block tag's edges are not: the browser drops that whitespace.
        // So each end of a body is one of three things ([`Edge`]), and a
        // body split by `<cfelse>` has that many more ends.
        let cf = e.kind == (ElementKind::TagBody { cf: true });
        let segments = self.else_segments(body);
        let edges: Vec<(Edge, Edge)> = if cf {
            let looped = self.is_loop(e, name);
            segments
                .iter()
                .map(|segment| self.cf_edges(segment.nodes, ctx, looped))
                .collect()
        } else if keeps_edge_space(e, name) {
            // An inline HTML tag: whitespace inside it shows wherever the
            // source has none (`<a href="x"><img></a>`: a break would put a
            // space in the link). A stray `<cfelse>` in it is on its own
            // line whatever is around it, as in a block tag's body.
            let text = self.tree.slice(body_span(body));
            let space = |there: bool| if there { Edge::Space } else { Edge::Glued };
            let last = segments.len() - 1;
            (0..=last)
                .map(|i| {
                    (
                        if i > 0 {
                            Edge::Space
                        } else {
                            space(text.starts_with(|c: char| c.is_ascii_whitespace()))
                        },
                        if i < last {
                            Edge::Space
                        } else {
                            space(text.ends_with(|c: char| c.is_ascii_whitespace()))
                        },
                    )
                })
                .collect()
        } else {
            vec![(Edge::Free, Edge::Free); segments.len()]
        };
        if broken {
            // A broken body has a line break at an end the source has
            // whitespace at, and at one where it would not show — unless
            // some end of the body is glued to what is next to it
            // (`<cfif x>O<cfelse>Uno</cfif>fficial`): then the author kept
            // the body tight on purpose, and only the ends with whitespace
            // break. An inline HTML body's ends are glued or have
            // whitespace; a block tag's never show.
            let glued = edges
                .iter()
                .any(|&(lead, trail)| lead == Edge::Glued || trail == Edge::Glued);
            let breaks = |edge: Edge| edge == Edge::Space || (edge == Edge::Free && !glued);
            // `tags.body.indent: "cfml"`: a CF tag body that starts with HTML
            // stays at the tag's indent, `depth` included, so an island in it
            // is raised to the flush column.
            let flush =
                self.opts.tags_body_indent == TagBodyIndent::Cfml && cf && starts_with_html(body);
            let inner = if flush { ctx } else { ctx.deeper() };
            for (segment, &(lead, trail)) in segments.iter().zip(&edges) {
                if let Some(tag) = segment.tag {
                    // No dedent: the segment's `indent` simply does not cover
                    // the `<cfelse>`, which prints at the enclosing tag's
                    // indent, on its own line when the ends around it break.
                    parts.push(self.with_tag_ctx(ctx, || self.element(tag)));
                }
                let within = TagCtx {
                    space_before: lead != Edge::Glued,
                    space_after: trail != Edge::Glued,
                    ..inner
                };
                let doc = self.tag_nodes(segment.nodes, within);
                if !doc.is_empty() {
                    let lines = if breaks(lead) {
                        vec![hardline(), doc]
                    } else {
                        vec![doc]
                    };
                    parts.push(if flush {
                        Doc::Concat(lines)
                    } else {
                        indent(lines)
                    });
                }
                if breaks(trail) {
                    parts.push(hardline());
                }
            }
        } else {
            let (lead, trail) = edges[0];
            let within = TagCtx {
                space_before: lead != Edge::Glued,
                space_after: trail != Edge::Glued,
                ..ctx
            };
            if body
                .iter()
                .any(|n| n.as_element().is_some_and(is_tag_comment))
            {
                // A one-line body holding a `<!--- --->` breaks like a body
                // with a newline when it does not fit: the comment would
                // otherwise break inside the line (`><!---` ⏎ text ⏎
                // `---></cffunction>`), and the next run reads that newline
                // and breaks the body. On one line an end's whitespace
                // collapses to one space instead of disappearing; a glued
                // end gets neither a space nor a break.
                let edge = |edge: Edge| match edge {
                    Edge::Space => line(),
                    Edge::Free => softline(),
                    Edge::Glued => Doc::Concat(Vec::new()),
                };
                let doc = self.tag_nodes(body, within.deeper());
                parts.push(group(vec![indent(vec![edge(lead), doc]), edge(trail)]));
            } else {
                let doc = self.tag_nodes(body, within);
                // On one line, that whitespace collapses to one space
                // instead of disappearing; a block tag's is dropped.
                let (lead, trail) = if keeps_edge_space(e, name) {
                    (lead == Edge::Space, trail == Edge::Space)
                } else {
                    (false, false)
                };
                if doc.is_empty() {
                    if lead || trail {
                        parts.push(Doc::from(" "));
                    }
                } else {
                    if lead {
                        parts.push(Doc::from(" "));
                    }
                    parts.push(doc);
                    if trail {
                        parts.push(Doc::from(" "));
                    }
                }
            }
        }
        parts.push(close_doc);
        Doc::Concat(parts)
    }

    /// The context the body of `e`, a paired tag printed in `ctx`, sits at
    /// when that body is a `<cfscript>` statement list or an island
    /// ([`Printer::island_body`]: `<script>`, `<style>`, `<cfquery>`,
    /// `<cfjava>`): the depth its lines are indented to, the floor of a
    /// verbatim island's shift and what the width handed to the island
    /// formatter is measured from. It is one level inside the tag under
    /// `tags.islands.indent` (the default) and the tag's own context
    /// otherwise. Inside a code fence (`rooted` false) an island keeps the
    /// tag's context, since it keeps its source columns there; a
    /// `<cfscript>` body, laid out by the doc printer, is one level in there
    /// too.
    pub(crate) fn body_floor(&self, e: &Element, ctx: TagCtx) -> TagCtx {
        let script = e.cf_kind() == Some(CfKind::Script);
        if self.opts.tags_islands_indent && (script || ctx.rooted) {
            ctx.deeper()
        } else {
            ctx
        }
    }

    /// The two ends of `nodes`, a CF tag body or one `<cfelse>` segment of
    /// it, printed in `ctx`. The tag does not reach the page, so what the
    /// body starts with follows what precedes the tag there, and what it
    /// ends with precedes what follows the tag: an end with no whitespace is
    /// [`Edge::Free`] only when whitespace would not show on that side —
    /// the context says so ([`TagCtx::space_before`]), or the body's own
    /// node at that end is a tag the browser drops whitespace around. Every
    /// segment of a `<cfif>` starts and ends in the same place on the page,
    /// so each is judged against the same context. A loop's body (`looped`)
    /// also follows itself: whitespace at its start shows after its own end
    /// unless that end has whitespace or a such a tag too, and the other way
    /// round. A body of nothing but whitespace, or empty, is one place: both
    /// its ends are judged against both sides.
    fn cf_edges(&self, nodes: &[Node], ctx: TagCtx, looped: bool) -> (Edge, Edge) {
        let text = self.tree.slice(body_span(nodes));
        let space = |c: char| c.is_ascii_whitespace();
        if text.trim_matches(space).is_empty() {
            let edge = match (text.is_empty(), ctx.space_before || ctx.space_after) {
                (false, _) => Edge::Space,
                (true, true) => Edge::Free,
                (true, false) => Edge::Glued,
            };
            return (edge, edge);
        }
        let spaceless = |n: Option<&Node>| {
            n.and_then(Node::as_element)
                .is_some_and(|n| self.is_spaceless_html(n))
        };
        // Per end: the source has whitespace there; the body's own node
        // there hides whitespace.
        let (lead_space, lead_tag) = (text.starts_with(space), spaceless(nodes.first()));
        let (trail_space, trail_tag) = (text.ends_with(space), spaceless(nodes.last()));
        let edge = |has_space: bool, tag: bool, outside: bool, other_end: bool| {
            if has_space {
                Edge::Space
            } else if tag || (outside && (!looped || other_end)) {
                Edge::Free
            } else {
                Edge::Glued
            }
        };
        (
            edge(
                lead_space,
                lead_tag,
                ctx.space_before,
                trail_space || trail_tag,
            ),
            edge(
                trail_space,
                trail_tag,
                ctx.space_after,
                lead_space || lead_tag,
            ),
        )
    }

    /// Whether `e`, a paired CF tag named `name`, may emit its body more
    /// than once: `<cfloop>`, and a `<cfoutput>` with a `query` or `group`
    /// attribute (judged by the word appearing in its opening tag, which
    /// errs towards a loop).
    fn is_loop(&self, e: &Element, name: &str) -> bool {
        if name.eq_ignore_ascii_case("cfloop") {
            return true;
        }
        name.eq_ignore_ascii_case("cfoutput")
            && e.open_tag().is_some_and(|open| {
                let tag = self.tree.slice(open.span.clone()).to_ascii_lowercase();
                tag.contains("query") || tag.contains("group")
            })
    }

    /// A body split at its bare `<cfelse>` / `<cfelseif>` tags; the first
    /// segment has no tag.
    fn else_segments<'n>(&self, body: &'n [Node]) -> Vec<ElseSegment<'n>> {
        let mut out = vec![ElseSegment {
            tag: None,
            nodes: body,
        }];
        let mut start = 0;
        for (i, n) in body.iter().enumerate() {
            if is_else_tag(n) {
                out.last_mut().expect("a segment").nodes = &body[start..i];
                out.push(ElseSegment {
                    tag: n.as_element(),
                    nodes: &body[i + 1..],
                });
                start = i + 1;
            }
        }
        out
    }

    /// One tag: `<` / `</` / `<!`, the name (lower-cased with
    /// `tags.lowercase`, never `DOCTYPE`), its content and `>` / `/>`.
    /// `<cfset>`, `<cfreturn>`, `<cfif>` and `<cfelseif>` print script after
    /// one space, with `>` on a line of its own when the script breaks, unless
    /// it ends on a bracket that stays on the tag's first line
    /// ([`closes_on_bracket`]); a doctype prints its text as written; every
    /// other tag prints an attribute group with no threshold (width only) and
    /// puts its closing delimiter on its own line when that group breaks.
    /// Inside an island a tag never breaks, unless it holds a forced break (a
    /// `//` comment, a function body).
    pub(crate) fn tag(&self, e: &Element, ctx: TagCtx) -> Doc {
        let mut parts = Vec::new();
        if let Some(t) = &e.open {
            parts.push(self.token(t));
        }
        let name = self.tree.tag_name(e);
        if let Some(name) = name {
            let lower = self.opts.tags_lowercase && e.kind != ElementKind::Doctype;
            parts.push(Doc::from(if lower {
                name.to_ascii_lowercase()
            } else {
                name.to_owned()
            }));
        }
        let names = name_tokens(e);
        let rest: Vec<&Node> = e.children[names..].iter().collect();
        let mut group_id = None;
        let mut close_breaks = false;
        let mut script = false;
        if e.kind == ElementKind::Doctype {
            if let (Some(first), Some(last)) = (rest.first(), rest.last()) {
                let text = self.tree.slice(first.span().start..last.span().end);
                let text = text.trim();
                if !text.is_empty() {
                    parts.push(Doc::from(" "));
                    parts.push(self.text(text));
                }
            }
        } else if matches!(
            e.kind,
            ElementKind::CfTag(_, CfKind::Expression | CfKind::ElseIf)
        ) {
            // `<cfset>`, `<cfreturn>`, `<cfif>`, `<cfelseif>`: script after
            // the name, not attributes.
            script = true;
            let content = self.with_tag_ctx(ctx, || self.sequence(rest.iter().copied()));
            let has_content = !content.is_empty();
            if has_content {
                parts.push(Doc::from(" "));
                parts.extend(content);
            }
            // A `//` comment ends the expression at the end of its line and
            // the tag's `>` is on a later one: the comment stays
            // in the tag as a line suffix, and `>` goes on the next line.
            if ends_with_line_comment(rest.iter().copied()) {
                parts.push(hardline());
            } else if self.ends_with_equals(&rest) {
                // `<cfset x = >` (a syntax error the engines report): glued,
                // `=>` would read as an arrow.
                parts.push(Doc::from(" "));
            } else if has_content && !closes_on_bracket(&rest) {
                close_breaks = true;
            } else if let (true, Some(id)) = (ends_with_assignment(&rest), self.assign_break.get())
            {
                // A value that ends on a bracket but moved to the line after
                // the operator is not a bracket closing on `>`'s line.
                group_id = Some(id);
            }
        } else {
            let html = matches!(e.kind, ElementKind::HtmlTag(_));
            let style = if html {
                KeyValueStyle::HtmlAttribute
            } else {
                KeyValueStyle::TagAttribute
            };
            // Whitespace between attributes never shows.
            let attrs_ctx = TagCtx {
                html_attributes: html,
                space_before: true,
                space_after: true,
                ..ctx.deeper()
            };
            if let Some((attrs, id)) = self.with_tag_ctx(attrs_ctx, || {
                let after = e.children[..names].last().map(|n| n.span().end);
                self.attribute_group(&rest, None, style, false, None, after)
            }) {
                parts.push(attrs);
                group_id = Some(id);
            }
        }
        if let Some(id) = group_id {
            parts.push(if_break_group(hardline_without_break_parent(), "", id));
        } else if close_breaks {
            parts.push(softline());
        }
        if script && rest.iter().any(|n| holds_line_comment(n)) {
            // A line comment in the script that no line break has printed
            // yet (`<cfset s // c` newline `= { a: 1 }>`) must print before
            // `>`: after it, it would be text. This group breaks, putting
            // `>` on the next line, only when such a comment is pending (its
            // boundary does not fit then). Being a group of its own, it ends
            // the measure of a group before it at its `softline` (the tag
            // sits in a broken body), so the pending comment does not break
            // `{ a: 1 }`, which prints on one line once the comment is at
            // the end. That also hides `>` from that measure, so the group
            // is only here when the script holds a line comment.
            parts.push(group(vec![softline(), line_suffix_boundary()]));
        }
        if let Some(t) = &e.close {
            parts.push(self.token(t));
        }
        if ctx.island {
            // Inside an island the tag sits on a line whose column is the
            // island's business (a `<cfqueryparam>` in the middle of a SQL
            // line): its attributes and script never break, whatever the
            // width, and the width invariant does not count island lines.
            // A tag holding a forced break cannot be flat — a `//` comment
            // in its script would leave the tag with its line suffix and
            // become island text — so it breaks as it does outside an island.
            let doc = Doc::Concat(parts);
            let forced = find_in_doc(&doc, |d| {
                matches!(d, Doc::BreakParent | Doc::LineSuffix(_)).then_some(())
            });
            return if forced.is_some() {
                doc
            } else {
                remove_lines(doc)
            };
        }
        if close_breaks {
            group(parts)
        } else {
            Doc::Concat(parts)
        }
    }

    /// Whether the last token of `nodes` that is not whitespace ends with `=`.
    fn ends_with_equals(&self, nodes: &[&Node]) -> bool {
        let space = |t: &Token| matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline);
        let last = nodes.iter().rev().find_map(|n| match n {
            Node::Token(t) if space(t) => None,
            Node::Token(t) => Some(t),
            Node::Element(e) => e.rfind_token(|t| !space(t)),
        });
        last.is_some_and(|t| self.tree.text(t).ends_with('='))
    }

    /// `<!--- … --->`. One line prints as `<!--- text --->` when it fits and
    /// as the three-line form otherwise; several lines always break. A comment
    /// whose first line starts with `-` (a dashed banner) has no spacers,
    /// keeps its lines at its own indent and glues `--->` after the last one;
    /// every other multi-line comment indents its lines once and puts `--->`
    /// on its own line.
    pub(crate) fn tag_comment(&self, e: &Element, ctx: TagCtx) -> Doc {
        let lines = self.comment_lines(e, ctx);
        let open = Doc::from("<!---");
        let close = Doc::from("--->");
        if lines.is_empty() {
            return Doc::Concat(vec![open, Doc::from(" "), close]);
        }
        let dashed = lines[0].starts_with('-');
        if lines.len() == 1 {
            let text = Doc::from(lines[0].clone());
            // A dashed banner has no spacers and never breaks, however long.
            if dashed {
                return Doc::Concat(vec![open, text, close]);
            }
            // On one line when `<!--- text --->` fits, else three.
            return group(vec![open, indent(vec![line(), text]), line(), close]);
        }
        let body = Doc::Concat(
            lines
                .iter()
                .enumerate()
                .flat_map(|(i, l)| {
                    let mut parts = Vec::new();
                    if i > 0 {
                        parts.push(hardline());
                    }
                    parts.push(Doc::from(l.clone()));
                    parts
                })
                .collect(),
        );
        if dashed {
            Doc::Concat(vec![open, body, close])
        } else {
            Doc::Concat(vec![
                open,
                indent(vec![hardline(), body]),
                hardline(),
                close,
            ])
        }
    }

    /// The trimmed lines of a tag comment's text. A nested `<!--- --->` is
    /// rendered first and its lines join the outer comment's, which is what
    /// re-indents them to one level.
    fn comment_lines(&self, e: &Element, ctx: TagCtx) -> Vec<String> {
        let mut text = String::new();
        for n in &e.children {
            match n {
                Node::Token(t) => text.push_str(self.tree.text(t)),
                Node::Element(c) if is_tag_comment(c) => {
                    text.push_str(&self.comment_text(c, ctx));
                }
                Node::Element(c) => text.push_str(self.tree.slice(c.span.clone())),
            }
        }
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        text.split('\n').map(|l| l.trim().to_owned()).collect()
    }

    /// A nested tag comment as text. Whether it fits on one line is measured
    /// at the indent of its depth (`depth * indent_size`), because a nested
    /// comment is laid out before the enclosing one and cannot use `fits`.
    fn comment_text(&self, e: &Element, ctx: TagCtx) -> String {
        let lines = self.comment_lines(e, ctx);
        if lines.is_empty() {
            return "<!--- --->".to_owned();
        }
        let dashed = lines[0].starts_with('-');
        let columns = ctx.depth * self.opts.indent_size;
        if lines.len() == 1
            && (dashed || columns + lines[0].chars().count() + 11 <= self.opts.max_columns)
        {
            let spacer = if dashed { "" } else { " " };
            return format!("<!---{spacer}{}{spacer}--->", lines[0]);
        }
        if dashed {
            return format!("<!---{}--->", lines.join("\n"));
        }
        let pad = " ".repeat(self.opts.indent_size);
        let body: Vec<String> = lines.iter().map(|l| format!("{pad}{l}")).collect();
        format!("<!---\n{}\n--->", body.join("\n"))
    }

    /// A tag-island code fence in script mode:
    /// ```` ``` ```` , the tags at the fence's own indent, ```` ``` ````.
    /// The tags inside count their depth from the fence, and an island inside
    /// one keeps its source columns: `literalline` returns to column 0, which
    /// is where those columns were measured.
    pub(crate) fn tag_island(&self, e: &Element) -> Doc {
        let body = self.tag_nodes(&e.children, TagCtx::fence());
        let fence = Doc::from("```");
        let mut parts = vec![fence.clone()];
        if !body.is_empty() {
            parts.push(Doc::Concat(vec![hardline(), body]));
        }
        parts.push(hardline());
        parts.push(fence);
        Doc::Concat(parts)
    }
}

/// `parts` under one `indent` when `floor`, a body's context, is deeper than
/// `ctx`, its tag's ([`Printer::body_floor`]); as they are otherwise.
pub(crate) fn indent_to(ctx: TagCtx, floor: TagCtx, parts: Vec<Doc>) -> Doc {
    if floor.depth > ctx.depth {
        indent(parts)
    } else {
        Doc::Concat(parts)
    }
}

/// An end of a tag body, or of one `<cfelse>` segment of it: what a line
/// break there would do to the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    /// The source has whitespace there: a line break keeps it.
    Space,
    /// It has none, and whitespace there would not show.
    Free,
    /// It has none, and whitespace there would show: no line break.
    Glued,
}

/// A `<cfelse>` / `<cfelseif>` clause of a tag body.
struct ElseSegment<'n> {
    /// The tag that opened this segment (`None` for the first).
    tag: Option<&'n Element>,
    nodes: &'n [Node],
}

/// Whether `island` is the content of `body`, a tag body named `name`: a
/// `<script>`, `<style>`, `<cfquery>` or `<cfjava>` island. The HTML tags
/// have no kind, so those two go by name.
fn owns_island(body: &Element, name: &str, island: &Element) -> bool {
    let ElementKind::Island(i) = &island.kind else {
        return false;
    };
    match i.site {
        IslandSite::ScriptTag => name.eq_ignore_ascii_case("script"),
        IslandSite::StyleTag => name.eq_ignore_ascii_case("style"),
        IslandSite::CfQuery => body.cf_kind() == Some(CfKind::Query),
        IslandSite::CfJava => body.cf_kind() == Some(CfKind::Java),
        _ => false,
    }
}

/// Whether whitespace at the edges of `e`'s body (a paired tag named
/// `name`) can show on the page: a CF tag body, or an HTML one whose tag
/// does not [ignore it](ignores_space).
fn keeps_edge_space(e: &Element, name: &str) -> bool {
    match e.kind {
        ElementKind::TagBody { cf: true } => true,
        ElementKind::TagBody { cf: false } => !ignores_space(name),
        _ => false,
    }
}

/// Whether `e`, a paired tag named `name`, is an HTML `<pre>` or `<textarea>`,
/// whose body is content whitespace and all.
pub(crate) fn is_preformatted(e: &Element, name: &str) -> bool {
    e.kind == (ElementKind::TagBody { cf: false })
        && (name.eq_ignore_ascii_case("pre") || name.eq_ignore_ascii_case("textarea"))
}

/// Whether a tag body's first significant node — whitespace, newlines and
/// comments (`<!--- --->` included) skipped — is not CFML, i.e. not a CF tag,
/// bare or paired (`tags.body.indent: "cfml"`). An empty body is not.
fn starts_with_html(body: &[Node]) -> bool {
    body.iter().find(|n| !n.is_trivia()).is_some_and(|n| {
        !n.as_element().is_some_and(|c| {
            matches!(
                c.kind,
                ElementKind::CfTag(..) | ElementKind::TagBody { cf: true }
            )
        })
    })
}

/// Whether `e` is a `<!--- --->` comment rather than a script one.
pub(crate) fn is_tag_comment(e: &Element) -> bool {
    e.kind == ElementKind::BlockComment
        && e.open
            .as_ref()
            .is_some_and(|t| t.kind == TokenKind::Punct(Punct::Open(Delim::TagComment)))
}

/// How many leading children of a tag element make up its name
/// (`Tree::tag_name`'s tokens).
fn name_tokens(e: &Element) -> usize {
    let mut count = 0;
    for n in &e.children {
        match n {
            Node::Token(t) if matches!(t.kind, TokenKind::Ident(Ident::TagName)) => count += 1,
            Node::Token(t) if t.kind == TokenKind::Punct(Punct::Prefix) && count > 0 => count += 1,
            _ if count == 0 => return 0,
            _ => break,
        }
    }
    count
}

/// The source span covered by a node list.
fn body_span(nodes: &[Node]) -> std::ops::Range<u32> {
    match (nodes.first(), nodes.last()) {
        (Some(first), Some(last)) => first.span().start..last.span().end,
        _ => 0..0,
    }
}

/// Whether a script tag's expression ends on a bracket that keeps the tag's
/// `>` on its line when the expression breaks, as Prettier keeps a JSX
/// expression container's `}` after a call, struct, array or function
/// (`printJsxExpressionContainer`'s `shouldInline`). `new Foo(…)` and an
/// index (`a[ … ]`) count as calls, and so does a condition in parentheses
/// as a whole, which breaks as `(` ⏎ … ⏎ `)`. An assignment is judged by its
/// value, a prefix operator (`not`, `!`) by its operand. Anything else that
/// breaks puts `>` on a line of its own, as a broken Vue attribute or
/// interpolation puts its closing delimiter.
fn closes_on_bracket(nodes: &[&Node]) -> bool {
    matches!(last_node(nodes), Some(Node::Element(e)) if ends_on_bracket(e))
}

/// Whether the last node of `nodes` that is not whitespace is an assignment.
fn ends_with_assignment(nodes: &[&Node]) -> bool {
    matches!(last_node(nodes), Some(Node::Element(e)) if e.kind == ElementKind::Assignment)
}

/// The last node of `nodes` that is not whitespace.
fn last_node<'n>(nodes: &[&'n Node]) -> Option<&'n Node> {
    nodes
        .iter()
        .rev()
        .find(|n| !matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)))
        .copied()
}

fn ends_on_bracket(e: &Element) -> bool {
    match e.kind {
        ElementKind::Unary { postfix: false } => {
            matches!(e.as_unary().map(|u| u.operand()), Some(Node::Element(o)) if ends_on_bracket(o))
        }
        ElementKind::Assignment => {
            matches!(e.as_assignment().map(|a| a.value()), Some(Node::Element(v)) if ends_on_bracket(v))
        }
        ElementKind::CallExpr
        | ElementKind::Struct { .. }
        | ElementKind::Array
        | ElementKind::Group
        | ElementKind::Function { .. }
        | ElementKind::ArrowFunction => true,
        ElementKind::New => matches!(
            e.children.last(),
            Some(Node::Element(c)) if matches!(c.kind, ElementKind::Call)
        ),
        ElementKind::Chain => matches!(
            e.children.last(),
            Some(Node::Element(s))
                if matches!(s.kind, ElementKind::Segment(SegmentKind::Method | SegmentKind::Index))
        ),
        _ => false,
    }
}

/// Whether `n` is a line comment or holds one.
fn holds_line_comment(n: &Node) -> bool {
    n.as_element()
        .is_some_and(|e| e.kind == ElementKind::LineComment || e.nodes().any(holds_line_comment))
}

/// Whether the last node of `nodes` that is not whitespace is a line comment,
/// or an element without a closing delimiter that ends with one.
fn ends_with_line_comment<'n>(nodes: impl DoubleEndedIterator<Item = &'n Node>) -> bool {
    let last = nodes
        .rev()
        .find(|n| !matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)));
    match last {
        Some(Node::Element(e)) if e.kind == ElementKind::LineComment => true,
        Some(Node::Element(e)) if e.close.is_none() && e.items.is_empty() => {
            ends_with_line_comment(e.children.iter())
        }
        _ => false,
    }
}

/// Whether `e` makes a body break: a tag, paired or bare.
fn is_tag_element(e: &Element) -> bool {
    matches!(
        e.kind,
        ElementKind::CfTag(..) | ElementKind::HtmlTag(_) | ElementKind::TagBody { .. }
    )
}
