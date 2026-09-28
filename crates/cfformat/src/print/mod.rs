//! Tree → [`Doc`]: the printers.
//!
//! One [`Printer`] walks the tree and returns a document per node;
//! `Printer::node` dispatches on [`ElementKind`]. Script mode starts at
//! `statements.rs`'s statement list, tag mode at `tags.rs`'s node list; the
//! two meet at a `<cfscript>` body and at a tag-island code fence.
//!
//! Source whitespace is trivia: no printer emits a `Whitespace` token, and
//! `Newline` tokens are only read where a statement list keeps its blank
//! lines, where a comment decides whether it shares a line with what precedes
//! it, and in tag mode, where a body breaks when the source has a newline in
//! it and an island line keeps its columns relative to the island's
//! least-indented line. No printer tracks a column: whether a group fits on
//! its line is the doc printer's `fits`.

use std::cell::{Cell, RefCell};
use std::path::Path;

use cfdoc::builders::{hardline, literalline};
use cfdoc::utils::replace_end_of_line;
use cfdoc::{Doc, GroupId, GroupIdGen};
use cfparse::{Element, ElementKind, Mode, Node, Token, TokenKind, Tree};

use crate::islands::{IslandStats, Islands};
use crate::Warning;
use crate::{FormatCtx, Options};

mod accessors;
mod alignment;
mod attributes;
mod calls;
mod casing;
mod comments;
mod components;
mod delimited;
mod expressions;
mod functions;
mod islands;
mod statements;
mod strings;
mod tags;

use tags::TagCtx;

/// How deep the printer's element dispatch (`Printer::element`) nests
/// before an element prints as written (its source text, newlines kept,
/// nothing below it visited), like the parser's own bound. Half the element depth at which a 2 MB thread first
/// overflowed in a debug build — nested tags 532, parentheses 578, structs
/// 559, blocks 667 (the script kinds 95 deep under nested tags: the parser
/// flattens past 100) — rounded down to a multiple of ten.
pub const MAX_DEPTH: u32 = 260;

/// Builds the document for one tree.
pub struct Printer<'a> {
    /// The tree being printed.
    pub(crate) tree: &'a Tree,
    /// Formatting options.
    pub(crate) opts: &'a Options,
    /// Group ids for `if_break` / `indent_if_break`.
    pub(crate) ids: RefCell<GroupIdGen>,
    /// Where the tag printers are (see [`TagCtx`]); the tag kinds are reached
    /// through [`Printer::element`], which takes no context.
    tags: Cell<TagCtx>,
    /// The file being printed, for an island's synthetic path.
    path: Option<&'a Path>,
    /// Island formatting; `None` keeps every island verbatim.
    islands: Option<&'a Islands>,
    /// What the island formatter said while the document was built.
    report: RefCell<PrintReport>,
    /// How many [`Printer::element`] calls are open ([`MAX_DEPTH`]).
    depth: Cell<u32>,
    /// The indent levels a statement list being printed sits at: one per
    /// enclosing block or case body, or a `<cfscript>` tag's depth. The doc
    /// printer alone knows the real column, so this is the printers' own
    /// count of the indents they wrap statement lists in; an indent that
    /// depends on a group breaking (a closure body in an argument list) is
    /// not counted. Read by the attribute alignment to tell whether a
    /// statement can print on one line before it is padded.
    indent_level: Cell<usize>,
    /// Set by the argument hugging for the one brace-less arrow it prints
    /// as the hugged last argument: the id for that arrow's body group, which
    /// the call's closing padding reads. [`Printer::function`] takes it, so
    /// it reaches that arrow alone.
    pub(crate) hug_arrow: Cell<Option<GroupId>>,
    /// Set by every assignment printed: the id for the group that breaks
    /// after its operator, `None` when it cannot break there. An assignment
    /// builds its value before itself, so after one is printed this holds
    /// its own id. Its one reader is [`Printer::tag`], for the `>` of a
    /// `<cfset>` whose script ends in an assignment; a side channel rather
    /// than a return value because the script between is printed by the
    /// statement printers, which return a `Doc` alone.
    pub(crate) assign_break: Cell<Option<GroupId>>,
    /// Set just before printing a ternary that is a whole call argument or
    /// a `return` / `throw` value, where a binary condition that breaks
    /// indents its continuation lines; anywhere else they stay at the
    /// condition's indent (Prettier's `shouldNotIndent`).
    /// [`Printer::ternary`] takes it, so it reaches that ternary alone; a
    /// side channel because its two setters (a call's argument list, a
    /// `return` / `throw` value) reach the ternary through the generic
    /// dispatch ([`Printer::element`]), which takes no context.
    pub(crate) ternary_cond_indent: Cell<bool>,
}

/// What printing one document reported beside the [`Doc`]: the warnings of
/// island formatting, collected while the printers run, and of the tree's
/// recovered regions, added by [`Printer::finish`].
#[derive(Debug, Default)]
pub struct PrintReport {
    /// What printed as written: islands the formatter refused, and the
    /// tree's recovered regions ([`Printer::finish`] adds those), in source
    /// order.
    pub warnings: Vec<Warning>,
    /// This document's formatter runs, cache hits, refusals and time in the
    /// formatter.
    pub stats: IslandStats,
}

impl<'a> Printer<'a> {
    /// A printer over `tree` that prints every island verbatim.
    pub fn new(tree: &'a Tree, opts: &'a Options) -> Self {
        Printer::with_ctx(tree, opts, &FormatCtx::default())
    }

    /// A printer over `tree` with a file path and island formatting.
    pub fn with_ctx(tree: &'a Tree, opts: &'a Options, ctx: &FormatCtx<'a>) -> Self {
        Printer {
            tree,
            opts,
            ids: RefCell::new(GroupIdGen::new()),
            tags: Cell::new(TagCtx::script()),
            path: ctx.path,
            islands: ctx.islands,
            report: RefCell::new(PrintReport::default()),
            depth: Cell::new(0),
            indent_level: Cell::new(0),
            hug_arrow: Cell::new(None),
            assign_break: Cell::new(None),
            ternary_cond_indent: Cell::new(false),
        }
    }

    /// Runs `f` with the statement-list indent one level deeper.
    pub(crate) fn indented<T>(&self, f: impl FnOnce() -> T) -> T {
        self.at_indent(self.indent_level.get() + 1, f)
    }

    /// Runs `f` with the statement-list indent at `level`.
    pub(crate) fn at_indent<T>(&self, level: usize, f: impl FnOnce() -> T) -> T {
        let saved = self.indent_level.replace(level);
        let out = f();
        self.indent_level.set(saved);
        out
    }

    /// The columns a statement list's lines start at, by the printers'
    /// count of the indents around it (`indent_level`).
    pub(crate) fn indent_columns(&self) -> usize {
        self.indent_level.get() * self.opts.indent_size
    }

    /// What the island formatter said while printing, and one warning per
    /// region of the tree the parse recovered in: each printed as
    /// written ([`ElementKind::Recovered`]).
    pub fn finish(self) -> PrintReport {
        let mut report = self.report.into_inner();
        let path = self.path.map(Path::to_path_buf);
        report.warnings.extend(
            self.tree.recoveries.iter().map(|r| {
                Warning::recovered(path.clone(), self.tree.line_of(r.span.start), r.reason)
            }),
        );
        report.warnings.sort_by_key(|w| w.line);
        report
    }

    /// The whole document: the root's contents, ending with one newline
    /// unless the source is empty. A tag-mode root is one broken body without
    /// its tags; a script root is a statement list.
    pub fn document(&self) -> Doc {
        let body = match self.tree.mode() {
            Mode::Tags => self.tag_nodes(&self.tree.root.children, TagCtx::base(0)),
            _ => self.statements(&self.tree.root.children),
        };
        if body.is_empty() {
            return body;
        }
        Doc::Concat(vec![body, hardline()])
    }

    /// Document for any node.
    pub(crate) fn node(&self, n: &Node) -> Doc {
        match n {
            Node::Token(t) => self.token(t),
            Node::Element(e) => self.element(e),
        }
    }

    /// Document for an element, by kind; at [`MAX_DEPTH`], its source text.
    pub(crate) fn element(&self, e: &Element) -> Doc {
        self.guarded(e, || self.element_by_kind(e))
    }

    /// `print` for `e` counted as one [`Printer::element`] level, for a
    /// printer that reaches a child element directly rather than through
    /// [`Printer::node`]; at [`MAX_DEPTH`], `e`'s source text.
    pub(crate) fn guarded(&self, e: &Element, print: impl FnOnce() -> Doc) -> Doc {
        let depth = self.depth.get();
        if depth >= MAX_DEPTH {
            return self.too_deep(e);
        }
        self.depth.set(depth + 1);
        let doc = print();
        self.depth.set(depth);
        doc
    }

    fn element_by_kind(&self, e: &Element) -> Doc {
        match e.kind {
            ElementKind::Ignore => self.verbatim(e),
            ElementKind::Recovered(_) => self.recovered(e),
            ElementKind::LineComment | ElementKind::BlockComment | ElementKind::DocComment => {
                self.comment(e)
            }
            ElementKind::Statement(_) => self.simple_statement(e),
            ElementKind::Block(_) => self.any_block(e),
            ElementKind::If
            | ElementKind::ElseIf
            | ElementKind::Else
            | ElementKind::For
            | ElementKind::While
            | ElementKind::DoWhile
            | ElementKind::Switch
            | ElementKind::Try
            | ElementKind::Catch
            | ElementKind::Finally => self.keyword(e),
            ElementKind::Case => self.case(e),
            ElementKind::Group if e.items.is_empty() => self.group(e),
            ElementKind::Struct { .. } => {
                self.print_delimited(e, &delimited::DelimitedStyle::structs(self.opts))
            }
            ElementKind::Array => {
                self.print_delimited(e, &delimited::DelimitedStyle::array(self.opts))
            }
            ElementKind::Brackets => self.brackets(e),
            ElementKind::TypedArray => self.typed_array(e),
            ElementKind::Call => self.call_args(e),
            // Reached through a declaration header's printer, which picks the
            // declaration or anonymous style; alone, the declaration style.
            ElementKind::Parameters => self.print_delimited(
                e,
                &delimited::DelimitedStyle::function_declaration(self.opts),
            ),
            ElementKind::CallExpr => self.call_expr(e),
            ElementKind::Chain => self.chain(e),
            ElementKind::Segment(_) => {
                debug_assert!(false, "a segment outside its chain");
                self.segment(e)
            }
            ElementKind::Function { .. } => self.function(e),
            ElementKind::FunctionDecl | ElementKind::ArrowFunction => self.function_header(e).0,
            ElementKind::KeyValue => self.key_value(e, delimited::KeyValueStyle::by_key(e)),
            ElementKind::Class | ElementKind::Interface => self.class(e),
            ElementKind::ClassDecl | ElementKind::InterfaceDecl => self.class_header(e).0,
            ElementKind::StaticBlock => self.static_block(e),
            ElementKind::Import => self.import(e),
            ElementKind::Property | ElementKind::Param | ElementKind::ScriptTag { acf: false } => {
                self.tag_statement(e)
            }
            ElementKind::ScriptTag { acf: true } => self.acf_script_tag(e),
            ElementKind::ScriptTagAttributes => self.script_tag_attributes(e),
            ElementKind::Assignment => self.assignment(e),
            ElementKind::Binary { .. } => self.binary(e),
            ElementKind::Ternary => self.ternary(e),
            ElementKind::Unary { .. } => self.unary(e),
            ElementKind::New => self.new_expr(e),
            ElementKind::String { .. } if e.open.is_some() && e.close.is_some() => self.string(e),
            ElementKind::TemplateExpression if e.open.is_some() && e.close.is_some() => {
                self.template_expression(e)
            }
            ElementKind::CfTag(..) | ElementKind::HtmlTag(_) | ElementKind::Doctype => {
                self.tag(e, self.tag_ctx())
            }
            ElementKind::TagBody { .. } => self.tag_body(e, self.tag_ctx()),
            ElementKind::TagIsland => self.tag_island(e),
            // A content island is printed by its owning tag body; one inside
            // an attribute value is the string's own text.
            ElementKind::Island(_) => self.attribute_island(e),
            ElementKind::Group => {
                debug_assert!(e.items.is_empty(), "a `for` header outside its keyword");
                self.as_written(e)
            }
            // A `String` or `TemplateExpression` the parser left without both
            // delimiters, and `Root` (never dispatched).
            _ => self.as_written(e),
        }
    }

    /// A token's text (copied: a `Doc`'s text is owned). Text containing a
    /// newline keeps its lines through `literalline`.
    pub(crate) fn token(&self, t: &Token) -> Doc {
        self.text(self.tree.text(t))
    }

    /// `s` as a doc; multi-line text keeps its lines through `literalline`.
    pub(crate) fn text(&self, s: &str) -> Doc {
        let doc = Doc::from(s.to_owned());
        if s.contains('\n') {
            replace_end_of_line(doc, literalline())
        } else {
            doc
        }
    }

    /// `cfformat-ignore` region or Java body: the source text as written,
    /// its line breaks the output's (`newline`) like every other line, so a
    /// file never comes out with mixed line endings.
    pub(crate) fn verbatim(&self, e: &Element) -> Doc {
        let text = self.tree.slice(e.span.clone());
        // The newline that ends the region is the statement list's.
        let text = text.strip_suffix('\n').unwrap_or(text).to_owned();
        replace_end_of_line(Doc::from(text), literalline())
    }

    /// An element at [`MAX_DEPTH`]: its source text up to its last token
    /// that is not whitespace. Whitespace an element ends with is trivia its
    /// parent would have printed (the parser's own bound leaves a
    /// `<cfscript>` body's last newline in its deepest group); printed here,
    /// it would add a line before the parent's own break on every run.
    fn too_deep(&self, e: &Element) -> Doc {
        let end = e
            .tokens()
            .iter()
            .rev()
            .find(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline))
            .map_or(e.span.start, |t| t.span.end);
        self.text(self.tree.slice(e.span.start..end))
    }

    /// A region the parse did not understand: its source text, as
    /// written. A region that runs to the end of the source ends at its last
    /// token that is not whitespace, as [`Printer::too_deep`] does: the
    /// document's own last newline stands for the rest, which would
    /// otherwise add a blank line on every run. Anywhere else its trailing
    /// whitespace is its own (`<cfif (a >`: the group left open holds the
    /// space before the tag's `>`) unless it holds a line break: then the
    /// region ends a line, and the break is the enclosing list's or tag's
    /// to print (a string left open at the end of a `<cfscript>` body takes
    /// the body's last newline, and printing it here added a blank line
    /// before `</cfscript>` on every run). The warning is
    /// [`Printer::finish`]'s.
    fn recovered(&self, e: &Element) -> Doc {
        let rest = &self.tree.source[e.span.end as usize..];
        if rest.trim().is_empty() {
            return self.too_deep(e);
        }
        let text = self.tree.slice(e.span.clone());
        let trimmed = text.trim_end();
        if text[trimmed.len()..].contains('\n') {
            return self.text(trimmed);
        }
        self.as_written(e)
    }

    /// An element the printers cannot lay out (a misparse: a string without
    /// both quotes, a `for` header outside its keyword): its source text,
    /// verbatim.
    pub(crate) fn as_written(&self, e: &Element) -> Doc {
        self.text(self.tree.slice(e.span.clone()))
    }
}
