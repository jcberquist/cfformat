//! The verbatim island path.
//!
//! `<cfquery>` SQL, `<script>` JS, `<style>` CSS and `<cfjava>` print as they
//! are: every line keeps its indentation relative to the others, blank lines
//! survive and nothing is re-wrapped. The one change is that an island whose
//! least-indented line sits left of the owning tag is shifted right as a whole
//! until that line reaches the tag's indent (the least leading indent of the
//! non-blank lines), measured in columns with a tab counting `indent_size` and
//! re-emitted by [`Printer::indent_to_column`]: whole indents as tabs under
//! `tab_indent`, the remainder as spaces. That shift, the per-line trim and
//! the re-rendering are safe only when no string spans a line; an island whose
//! text may hold one ([`keeps_literal_text`]: a JS backtick or `\`-continued
//! line, a CSS `\`-continued line, a SQL string, quoted identifier or dollar
//! quote spanning a line, a Java `"""`) prints byte for byte instead
//! ([`Verbatim::Raw`]): every line as written, only its CFML tags formatted. A
//! body of no language ([`Lang::Unknown`]: a `<script>` / `<style>` type the
//! formatter does not know) is data and prints exactly ([`Verbatim::Exact`]):
//! its edge lines too, and the closing tag right after its last byte.
//!
//! Each line is a `literalline` plus that indentation as text, because
//! `literalline` returns to the document root (or to the nearest
//! `mark_as_root`, which is what lets a code fence hold an island). CFML tags
//! inside an island are still formatted, and a tag body inside one keeps its
//! body verbatim through the same rule — no dedent and no forced break after
//! its closing tag, so the island's own lines are all that decide where its
//! text goes.

use std::path::PathBuf;

use cfdoc::builders::{hardline, literalline};
use cfdoc::Doc;
use cfparse::{Element, ElementKind, Lang, Node, TokenKind};

use super::tags::{TagCtx, Verbatim};
use super::Printer;
use crate::islands::{
    dispatch, hand_off_text, keeps_literal_text, FormattedIsland, IslandRequest, Refused,
};
use crate::{Warning, WarningKind};

/// One preserved line: its leading whitespace in columns and its content.
#[derive(Default)]
struct Line {
    /// Leading whitespace of the line, in columns.
    columns: usize,
    parts: Vec<Doc>,
    /// Text seen so far while the line is still empty (its indentation).
    indent: String,
    /// Whether anything has been added to the line.
    started: bool,
}

/// An island's content as lines.
#[derive(Default)]
struct Lines {
    out: Vec<Line>,
    cur: Line,
    /// Whether a tag element was printed anywhere in the island.
    tags: bool,
    tab_width: usize,
    /// Byte for byte ([`Verbatim::Raw`] or [`Verbatim::Exact`]): text keeps
    /// its leading and trailing whitespace.
    raw: bool,
}

impl Lines {
    fn text(&mut self, s: &str) {
        let mut first = true;
        for part in s.split('\n') {
            if !first {
                self.newline();
            }
            first = false;
            if part.is_empty() {
                continue;
            }
            if self.raw {
                self.cur.started = true;
                self.cur.parts.push(Doc::from(part.to_owned()));
                continue;
            }
            if !self.cur.started {
                let content = part.trim_start();
                let lead = &part[..part.len() - content.len()];
                self.cur.indent.push_str(lead);
                if content.is_empty() {
                    continue;
                }
                self.cur.started = true;
                self.cur.parts.push(Doc::from(content.to_owned()));
            } else {
                self.cur.parts.push(Doc::from(part.to_owned()));
            }
        }
    }

    fn doc(&mut self, d: Doc) {
        self.cur.started = true;
        self.cur.parts.push(d);
    }

    fn newline(&mut self) {
        let mut line = std::mem::take(&mut self.cur);
        line.columns = line
            .indent
            .chars()
            .map(|c| if c == '\t' { self.tab_width } else { 1 })
            .sum();
        if !self.raw {
            trim_end(&mut line.parts);
        }
        self.out.push(line);
    }

    fn finish(mut self) -> (Vec<Line>, bool) {
        self.newline();
        (self.out, self.tags)
    }
}

/// Trims the trailing whitespace of a line's last text part.
fn trim_end(parts: &mut Vec<Doc>) {
    while let Some(Doc::Text(t)) = parts.last_mut() {
        let trimmed = t.trim_end();
        if trimmed.is_empty() {
            parts.pop();
        } else {
            if trimmed.len() != t.len() {
                *t = trimmed.to_owned().into();
            }
            break;
        }
    }
}

impl Printer<'_> {
    /// Indentation to `columns`: whole indents as the indent character, the
    /// remainder always spaces.
    pub(crate) fn indent_to_column(&self, columns: usize) -> String {
        if self.opts.tab_indent {
            let tabs = columns / self.opts.indent_size;
            let spaces = columns % self.opts.indent_size;
            "\t".repeat(tabs) + &" ".repeat(spaces)
        } else {
            " ".repeat(columns)
        }
    }

    /// A tag body whose content is an island (`<cfquery>`, `<script>`,
    /// `<style>`, `<cfjava>`): the preserved lines between the two tags. The
    /// closing tag starts its own line at the tag's indent when the island
    /// holds a newline or a tag; otherwise everything stays on one line.
    ///
    /// A pure island whose `islands.*` option is not `"off"` is formatted
    /// first ([`Printer::formatted_island`]); its lines then sit at the tag's
    /// indent and the closing tag always starts its own line. An island
    /// printed here whose text may hold a string spanning lines
    /// ([`keeps_literal_text`]) prints byte for byte ([`Verbatim::Raw`]); one
    /// of no language prints exactly, the closing tag directly after it
    /// ([`Verbatim::Exact`]).
    pub(crate) fn island_body(&self, open: Doc, body: &[Node], close: Doc, ctx: TagCtx) -> Doc {
        if let Some(lines) = self.formatted_island(body, ctx) {
            return Doc::Concat(vec![open, lines, hardline(), close]);
        }
        let verbatim = match island_lang(body) {
            Some(Lang::Unknown) => Verbatim::Exact,
            _ if self.keeps_literal_text(body) => Verbatim::Raw,
            _ => Verbatim::Shift,
        };
        let ctx = TagCtx {
            island: true,
            verbatim,
            ..ctx
        };
        let (lines, tags) = self.island_lines(body, ctx);
        let multi = (lines.len() > 1 || tags) && verbatim != Verbatim::Exact;
        let mut parts = vec![open];
        parts.push(self.render_island(lines, ctx));
        if multi {
            parts.push(hardline());
        }
        parts.push(close);
        Doc::Concat(parts)
    }

    /// The island formatter's (oxc's, in process) lines for an island body,
    /// or `None` for the verbatim path. The body must be one pure island and
    /// trivia, in a rooted context (inside a code fence the width budget and
    /// the island's column are unknown), non-blank, dispatched to an
    /// `islands.*` option that is not `"off"`. A refusal is a warning and
    /// prints verbatim.
    ///
    /// The formatter is handed the island's text as written
    /// ([`hand_off_text`]) and its output is spliced line by line under the
    /// doc's own indentation, which inside a tag body is the island's
    /// (`least`): each line follows a `hardline` and loses its trailing
    /// whitespace, except a line that starts inside literal text (a
    /// template literal, a continued string, a raw block comment: the
    /// result carries them, [`FormattedIsland::literal_lines`], and a text
    /// whose lines cannot be found is a refusal), which follows a
    /// `literalline` and prints as written, so its columns are the
    /// source's (and the line before one keeps its trailing whitespace,
    /// which is inside the literal). That is also why a second run changes
    /// nothing: the raw text the next run hands off is what the formatter
    /// printed, and literal lines were never moved.
    fn formatted_island(&self, body: &[Node], ctx: TagCtx) -> Option<Doc> {
        let islands = self.islands?;
        if !ctx.rooted {
            return None;
        }
        let mut elements = body.iter().filter_map(Node::as_element);
        let e = elements.next()?;
        let trivia = body.iter().all(|n| match n {
            Node::Token(t) => matches!(t.kind, TokenKind::Newline | TokenKind::Whitespace),
            Node::Element(_) => true,
        });
        if elements.next().is_some() || !trivia || !e.is_pure_island() {
            return None;
        }
        let ElementKind::Island(island) = &e.kind else {
            return None;
        };
        let target = dispatch(island)?;
        if !target.enabled(self.opts) {
            return None;
        }
        // A CSS string continued over a line: oxc would re-indent its
        // continuation, so the body stays byte for byte.
        if target.lang == Lang::Css
            && keeps_literal_text(self.tree.slice(e.span.clone()), Lang::Css)
        {
            return None;
        }
        let Node::Token(t) = &e.children[0] else {
            return None;
        };
        let text = hand_off_text(self.tree.text(t));
        if text.is_empty() {
            return None;
        }
        let least = ctx.least_columns(self.opts.indent_size);
        let path = match self.path {
            Some(p) => {
                let p = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
                let mut synthetic = p.into_os_string();
                synthetic.push(format!(".{}", target.ext));
                PathBuf::from(synthetic)
            }
            None => PathBuf::from(format!("stdin.cfm.{}", target.ext)),
        };
        // The project's configuration, found from the file's directory
        // (stdin: the current directory's, as prettier does).
        let at = match self.path {
            Some(_) => Some(path.clone()),
            None => std::env::current_dir().ok().map(|cwd| cwd.join(&path)),
        };
        let config = match at {
            Some(at) => islands.project_config(&at, self.opts.islands_config),
            None => Default::default(),
        };
        let req = IslandRequest {
            lang: target.lang,
            text: &text,
            path,
            indent: self.opts.indent_style(),
            width: self.opts.max_columns.saturating_sub(least).max(40),
            config,
        };
        let (result, hit) = islands.format(&req);
        self.report.borrow_mut().stats.record(hit, result.is_err());
        match result {
            Ok(FormattedIsland {
                text: out,
                literal_lines: literal,
            }) => {
                let lines: Vec<&str> = out.split('\n').collect();
                let first = lines.iter().position(|l| !l.trim().is_empty())?;
                let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
                let is_literal = |i: usize| literal.binary_search(&i).is_ok();
                let mut parts = vec![hardline()];
                for (i, l) in lines.iter().enumerate().take(last + 1).skip(first) {
                    if i > first {
                        parts.push(if is_literal(i) {
                            literalline()
                        } else {
                            hardline()
                        });
                    }
                    // A line whose end is inside the literal (the next line
                    // starts in it) keeps its trailing whitespace too.
                    let text = if is_literal(i + 1) { l } else { l.trim_end() };
                    parts.push(Doc::from(text.to_owned()));
                }
                Some(Doc::Concat(parts))
            }
            Err(Refused(message)) => {
                self.report.borrow_mut().warnings.push(Warning {
                    path: self.path.map(PathBuf::from),
                    line: self.tree.line_of(e.span.start),
                    kind: WarningKind::Island { key: target.key },
                    message,
                });
                None
            }
        }
    }

    /// Whether the island a tag body owns keeps its text byte for byte:
    /// [`keeps_literal_text`] over the body's host text in the island's
    /// language. CF tags, `#…#` and tag comments are server-side, not the
    /// language's text (an apostrophe in a `<cfset>` comment is no SQL
    /// quote): each stands for its line breaks only, so the host text's
    /// lines stay the source's.
    fn keeps_literal_text(&self, body: &[Node]) -> bool {
        let Some(lang) = island_lang(body) else {
            return false;
        };
        let mut text = String::new();
        self.host_text(body, &mut text);
        keeps_literal_text(&text, lang)
    }

    /// The host text of an island's nodes ([`Printer::keeps_literal_text`]).
    /// A nested tag body (`<cfif>` … `</cfif>`) is walked as
    /// [`Printer::collect_island`] walks it: its text is the island's, its
    /// own tags stand for their newlines.
    fn host_text(&self, nodes: &[Node], out: &mut String) {
        for n in nodes {
            match n {
                Node::Token(t) => out.push_str(self.tree.text(t)),
                Node::Element(e)
                    if matches!(e.kind, ElementKind::Island(_) | ElementKind::TagBody { .. }) =>
                {
                    self.host_text(&e.children, out)
                }
                Node::Element(e) => out.extend(
                    self.tree
                        .slice(e.span.clone())
                        .chars()
                        .filter(|&c| c == '\n'),
                ),
            }
        }
    }

    /// A paired tag inside an island: its own tags formatted, its body
    /// verbatim, nothing forced around it.
    pub(crate) fn island_tag_body(&self, e: &Element, ctx: TagCtx) -> Doc {
        let mut lines = self.new_lines(ctx);
        match (e.open_tag(), e.close_tag()) {
            (Some(open), Some(close)) => {
                lines.doc(self.tag(open, ctx));
                self.collect_island(e.body(), ctx, &mut lines);
                lines.doc(self.tag(close, ctx));
            }
            _ => lines.text(self.tree.slice(e.span.clone())),
        }
        let (lines, _) = lines.finish();
        self.render_island(lines, ctx)
    }

    fn new_lines(&self, ctx: TagCtx) -> Lines {
        Lines {
            tab_width: self.opts.indent_size,
            raw: ctx.verbatim != Verbatim::Shift,
            ..Lines::default()
        }
    }

    /// An island's content as lines.
    fn island_lines(&self, nodes: &[Node], ctx: TagCtx) -> (Vec<Line>, bool) {
        let mut lines = self.new_lines(ctx);
        self.collect_island(nodes, ctx, &mut lines);
        lines.finish()
    }

    /// Walks an island's nodes into `out`: text carries its own line breaks,
    /// CFML tags print through the tag printer, and anything else (a tag
    /// comment, an ignore region, a string) keeps its source text.
    fn collect_island(&self, nodes: &[Node], ctx: TagCtx, out: &mut Lines) {
        for n in nodes {
            match n {
                Node::Token(t) if t.kind == TokenKind::Newline => out.newline(),
                Node::Token(t) => out.text(self.tree.text(t)),
                Node::Element(e) => match e.kind {
                    ElementKind::Island(_) => self.collect_island(&e.children, ctx, out),
                    ElementKind::TagBody { .. } => match (e.open_tag(), e.close_tag()) {
                        (Some(open), Some(close)) => {
                            out.tags = true;
                            out.doc(self.tag(open, ctx));
                            self.collect_island(e.body(), ctx, out);
                            out.doc(self.tag(close, ctx));
                        }
                        _ => out.text(self.tree.slice(e.span.clone())),
                    },
                    ElementKind::CfTag(..) | ElementKind::HtmlTag(_) | ElementKind::Doctype => {
                        out.tags = true;
                        out.doc(self.tag(e, ctx));
                    }
                    ElementKind::TemplateExpression if e.open.is_some() && e.close.is_some() => {
                        out.doc(self.template_expression(e));
                    }
                    _ => out.text(self.tree.slice(e.span.clone())),
                },
            }
        }
    }

    /// The lines of an island: the first continues the opening tag's line,
    /// every other one is a `literalline` plus its own indentation shifted by
    /// `least - min` (`min` being the least indentation of the non-blank lines
    /// after the first, `least` the tag's), so the island keeps its shape and
    /// no line sits left of the tag. Trailing blank lines are dropped.
    ///
    /// Under [`Verbatim::Raw`] every line is as written instead: its own
    /// leading and trailing whitespace, no shift, nothing re-rendered, a
    /// whitespace-only first or last line dropped (no literal is open
    /// there). Under [`Verbatim::Exact`] no line is dropped either: the
    /// lines joined are the island's text.
    fn render_island(&self, mut lines: Vec<Line>, ctx: TagCtx) -> Doc {
        let raw = ctx.verbatim != Verbatim::Shift;
        let exact = ctx.verbatim == Verbatim::Exact;
        let blank = |l: &Line| {
            l.parts.is_empty()
                || (raw
                    && l.parts
                        .iter()
                        .all(|p| matches!(p, Doc::Text(t) if t.trim().is_empty())))
        };
        while !exact && lines.last().is_some_and(blank) {
            lines.pop();
        }
        if raw {
            let mut parts = Vec::new();
            for (i, line) in lines.into_iter().enumerate() {
                if i > 0 {
                    parts.push(literalline());
                } else if !exact && blank(&line) {
                    continue;
                }
                parts.extend(line.parts);
            }
            return Doc::Concat(parts);
        }
        // Every line after the first moves right by the same amount, so the
        // least-indented one lands on the tag's indent and the rest keep their
        // indentation relative to it. Nothing is ever lowered.
        let least = ctx.least_columns(self.opts.indent_size);
        let min = lines
            .iter()
            .skip(1)
            .filter(|l| !l.parts.is_empty())
            .map(|l| l.columns)
            .min()
            .unwrap_or(least);
        let shift = least.saturating_sub(min);
        let mut parts = Vec::new();
        for (i, line) in lines.into_iter().enumerate() {
            if i > 0 {
                parts.push(literalline());
            }
            if line.parts.is_empty() {
                continue;
            }
            if i == 0 {
                // Nothing precedes it on the line, so its indentation is the
                // source's.
                parts.push(Doc::from(line.indent));
            } else {
                parts.push(Doc::from(self.indent_to_column(line.columns + shift)));
            }
            parts.extend(line.parts);
        }
        Doc::Concat(parts)
    }

    /// An island inside an attribute value (`onclick="…"`, `style="…"`): the
    /// string's own text, printed as written, and its CF tags as in any
    /// island ([`TagCtx::island`]): flat, a body's text kept, so
    /// `alert('<cfif x> b</cfif>')` keeps the space it outputs.
    pub(crate) fn attribute_island(&self, e: &Element) -> Doc {
        let ctx = TagCtx {
            island: true,
            ..self.tags.get()
        };
        self.with_tag_ctx(ctx, || {
            Doc::Concat(
                e.children
                    .iter()
                    .map(|n| match n {
                        Node::Token(t) => self.token(t),
                        Node::Element(c) => self.element(c),
                    })
                    .collect(),
            )
        })
    }
}

/// The language of the island a tag body owns: its first island's.
fn island_lang(body: &[Node]) -> Option<Lang> {
    body.iter().find_map(|n| match &n.as_element()?.kind {
        ElementKind::Island(island) => Some(island.lang),
        _ => None,
    })
}
