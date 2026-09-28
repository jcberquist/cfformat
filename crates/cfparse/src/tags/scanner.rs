//! The scanner's state, its content loop and the shared emit helpers.
//!
//! One position walks the whole normalised source. At a content position the
//! scanner tries the CFML tag rules first and the HTML rules after them, so a
//! CF tag wins wherever both could start, and text that matches no rule is
//! emitted one token per line, the newline included.

use std::ops::Range;

use crate::scan::closes_tag;
use crate::tree::{Element, ElementKind, Node, RecoveryReason, Token, TokenKind};

/// Which rules are live in a run of content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ctx {
    /// HTML with CF tags: the top level, a `<cffunction>` body, a tag island.
    Html,
    /// A `<cfoutput>` / `<cfmail>` body: the same plus `#…#` expressions.
    Output,
    /// A `<cfcomponent>` body: CF tags alone, so no HTML tag, entity or bare
    /// `<` rule fires and text is `Other`.
    Class,
    /// A `<cfinterface>` body: `<!--- --->` comments and `<cffunction>`
    /// alone.
    Interface,
    /// The rest of a file that starts with a component: after a
    /// `<cfcomponent>` only another `<cfcomponent>` / `<cfinterface>` starts
    /// anything and the rest of the file is `Other`.
    Source,
}

impl Ctx {
    /// `#…#` and the `&##…;` entities are live.
    pub(super) fn hash(self) -> bool {
        self == Ctx::Output
    }

    /// The HTML rules (HTML tags, entities, a bare `>`) are live.
    pub(super) fn html(self) -> bool {
        matches!(self, Ctx::Html | Ctx::Output)
    }

    /// Unmatched text that is not whitespace.
    fn text_kind(self) -> TokenKind {
        match self {
            Ctx::Html | Ctx::Output => TokenKind::Text,
            // Not HTML: a component or interface body, or the file after one.
            Ctx::Class | Ctx::Interface | Ctx::Source => TokenKind::Other,
        }
    }
}

/// What ends a run of content, without consuming it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ContentStop {
    /// End of the source (or of the fragment).
    Eof,
    /// `</cffunction>`.
    CfFunction,
    /// `</cfoutput>` or `</cfmail>` — either ends either body.
    Output,
    /// `</cfcomponent>`.
    Class,
    /// `</cfinterface>`.
    Interface,
}

pub(super) struct Scanner<'a> {
    pub(super) src: &'a str,
    pub(super) pos: usize,
    /// One past the last byte the scanner may read (a fragment's end).
    pub(super) end: usize,
    /// One past the last byte of the whole run: [`Scanner::end`] before any
    /// [`Scanner::scoped`] pulled it in, and what [`Scanner::finish`]
    /// restores.
    limit: usize,
    /// Where [`Scanner::too_deep`] cut the source, `usize::MAX` until it
    /// does. Unlike [`Scanner::end`] a cut is never given back.
    cut: usize,
    /// Nesting of the recursive routines, bounded by [`MAX_DEPTH`](crate::MAX_DEPTH).
    pub(super) depth: u32,
    /// Inside [`Scanner::cf_tag_flat`]: the nodes become the tokens of an
    /// attribute value, and an island there keeps its text whole
    /// ([`Scanner::push_island`]).
    pub(super) flat: bool,
}

impl<'a> Scanner<'a> {
    pub(super) fn new(src: &'a str) -> Self {
        Scanner {
            src,
            pos: 0,
            end: src.len(),
            limit: src.len(),
            cut: usize::MAX,
            depth: 0,
            flat: false,
        }
    }

    /// A scanner over `range` of `src`, spans absolute, starting `depth`
    /// deep (the script parser's, whose ```` ``` ```` island this is).
    pub(super) fn fragment(src: &'a str, range: Range<u32>, depth: u32) -> Self {
        let mut s = Scanner::new(src);
        s.pos = range.start as usize;
        s.end = range.end as usize;
        s.limit = s.end;
        s.depth = depth;
        s
    }

    // -----------------------------------------------------------------------
    // Emitting
    // -----------------------------------------------------------------------

    pub(super) fn token(&self, span: Range<usize>, kind: TokenKind) -> Token {
        Token {
            span: span.start as u32..span.end as u32,
            kind,
        }
    }

    pub(super) fn node(&self, span: Range<usize>, kind: TokenKind) -> Node {
        Node::Token(self.token(span, kind))
    }

    /// Emit `span` as one token unless it is empty.
    pub(super) fn push(&self, out: &mut Vec<Node>, span: Range<usize>, kind: TokenKind) {
        if span.start < span.end {
            out.push(self.node(span, kind));
        }
    }

    /// Consume `len` bytes from the current position as one token.
    pub(super) fn take(&mut self, out: &mut Vec<Node>, len: usize, kind: TokenKind) {
        let from = self.pos;
        self.pos = (from + len).min(self.end);
        self.push(out, from..self.pos, kind);
    }

    /// Push a `<script>` / `<style>` island. Its body starts at the end of
    /// the tag's line when the line holds nothing else, so a CFML tag at the
    /// start of the next line is inside the island; the whitespace-only
    /// lines the island then starts with go before it, into `out`, as
    /// `Newline` / `Whitespace` tokens, so the island's text and first line
    /// start at its first non-blank line. Under [`Scanner::flat`] the island
    /// is pushed as it is.
    pub(super) fn push_island(&self, out: &mut Vec<Node>, mut el: Element) {
        if let Some(Node::Token(t)) = el.children.first_mut().filter(|_| !self.flat) {
            let text = &self.src[t.span.start as usize..t.span.end as usize];
            if t.kind == TokenKind::Text && text.starts_with('\n') {
                // Up to the last newline before the first non-blank.
                let blank = text.len() - text.trim_start().len();
                let cut = text[..blank].rfind('\n').map_or(0, |n| n + 1);
                let mut at = t.span.start;
                for line in text[..cut].split_inclusive('\n') {
                    let spaces = line.len() - 1;
                    let mut token = |len: usize, kind: TokenKind| {
                        out.push(Node::Token(Token {
                            span: at..at + len as u32,
                            kind,
                        }));
                        at += len as u32;
                    };
                    if spaces > 0 {
                        token(spaces, TokenKind::Whitespace);
                    }
                    token(1, TokenKind::Newline);
                }
                t.span.start = at;
                if t.span.is_empty() {
                    el.children.remove(0);
                }
                el.span.start = at;
            }
        }
        out.push(Node::Element(Box::new(el)));
    }

    pub(super) fn element(&self, kind: ElementKind, span: Range<u32>) -> Element {
        Element {
            kind,
            open: None,
            close: None,
            children: Vec::new(),
            items: Vec::new(),
            span,
        }
    }

    pub(super) fn rest(&self) -> &'a str {
        &self.src[self.pos..self.end]
    }

    pub(super) fn at(&self, at: usize) -> &'a str {
        &self.src[at.min(self.end)..self.end]
    }

    /// Advance over one character.
    pub(super) fn bump(&mut self) -> char {
        let c = self.rest().chars().next().unwrap_or('\0');
        self.pos = (self.pos + c.len_utf8()).min(self.end);
        c
    }

    // -----------------------------------------------------------------------
    // Unmatched text
    // -----------------------------------------------------------------------

    /// Emit `span`, text no rule matched, as `kind`. No token spans a line
    /// end: each line is its own token, and whitespace is `Newline` when it
    /// ends its line and `Whitespace` otherwise.
    pub(super) fn region(&self, out: &mut Vec<Node>, span: Range<usize>, kind: TokenKind) {
        self.emit_region(out, span, kind, false);
    }

    /// [`Scanner::region`] for a region inside a construct (a tag, a doctype,
    /// an HTML comment) or a component body, which is split into its
    /// whitespace and non-whitespace runs as well.
    pub(super) fn meta_region(&self, out: &mut Vec<Node>, span: Range<usize>, kind: TokenKind) {
        self.emit_region(out, span, kind, true);
    }

    fn emit_region(&self, out: &mut Vec<Node>, span: Range<usize>, kind: TokenKind, meta: bool) {
        if span.start >= span.end {
            return;
        }
        let mut at = span.start;
        for line in self.src[span].split_inclusive('\n') {
            let part = at..at + line.len();
            at = part.end;
            if meta {
                self.split_whitespace(out, part, kind);
            } else {
                out.push(self.node(part.clone(), self.region_kind(part, kind)));
            }
        }
    }

    fn region_kind(&self, span: Range<usize>, kind: TokenKind) -> TokenKind {
        let text = &self.src[span];
        if !text.bytes().all(|b| b.is_ascii_whitespace()) {
            return kind;
        }
        if text.ends_with('\n') {
            TokenKind::Newline
        } else {
            TokenKind::Whitespace
        }
    }

    fn split_whitespace(&self, out: &mut Vec<Node>, span: Range<usize>, kind: TokenKind) {
        let bytes = self.src[span.clone()].as_bytes();
        let ws = |b: u8| b.is_ascii_whitespace();
        let mut start = 0;
        for i in 1..=bytes.len() {
            if i == bytes.len() || ws(bytes[i]) != ws(bytes[start]) {
                let part = span.start + start..span.start + i;
                out.push(self.node(part.clone(), self.region_kind(part, kind)));
                start = i;
            }
        }
    }

    /// Emit the pending run `from..self.pos` as content text.
    pub(super) fn flush(&self, out: &mut Vec<Node>, from: &mut usize, ctx: Ctx) {
        // A component body's text takes the whitespace split, as text inside
        // a construct does; every other content run is split by lines only.
        if ctx == Ctx::Class {
            self.meta_region(out, *from..self.pos, ctx.text_kind());
        } else {
            self.region(out, *from..self.pos, ctx.text_kind());
        }
        *from = self.pos;
    }

    /// The whitespace at the start of a body is a region of its own, split off
    /// from the text that follows.
    pub(super) fn split_body_start(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        let mut from = self.pos;
        while self.pos < self.end {
            let c = self.rest().chars().next().unwrap();
            if !c.is_whitespace() {
                break;
            }
            self.pos += c.len_utf8();
        }
        self.flush(out, &mut from, ctx);
    }

    /// Emit the pending run as a region inside a construct
    /// ([`Scanner::meta_region`]).
    pub(super) fn flush_meta(&self, out: &mut Vec<Node>, from: &mut usize, kind: TokenKind) {
        self.meta_region(out, *from..self.pos, kind);
        *from = self.pos;
    }

    // -----------------------------------------------------------------------
    // The nesting bound
    // -----------------------------------------------------------------------

    /// At [`MAX_DEPTH`](crate::MAX_DEPTH), as in the script parser: the rest
    /// of the source up to its trailing whitespace becomes unmatched text —
    /// one region per line, the kind `ctx` gives —
    /// and the source **ends** there, so every enclosing routine sees the end
    /// at once and every open element closes without a `close` token. The
    /// trailing whitespace is left for [`Scanner::finish`], so that a body
    /// nested past the bound does not keep its last newline inside the
    /// deepest element and grow a blank line on every run.
    ///
    /// The cut text is a [`Recovered`](ElementKind::Recovered)`(TooDeep)`
    /// region.
    pub(super) fn too_deep(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        let end = self.src[..self.limit].trim_end().len().max(self.pos);
        let mut from = self.pos;
        self.pos = end;
        let first = out.len();
        self.flush(out, &mut from, ctx);
        if first < out.len() {
            crate::postpass::wrap_recovered(out, first, out.len() - 1, RecoveryReason::TooDeep);
        }
        self.cut = end;
        self.end = end;
    }

    /// The trailing whitespace [`Scanner::too_deep`] left, at the entry
    /// point; nothing to do when the bound was never reached.
    pub(super) fn finish(&mut self, out: &mut Vec<Node>) {
        if self.cut >= self.limit {
            return;
        }
        let from = self.cut;
        self.cut = usize::MAX;
        self.end = self.limit;
        self.pos = self.limit;
        self.region(out, from..self.limit, TokenKind::Text);
    }

    // -----------------------------------------------------------------------
    // The content loop
    // -----------------------------------------------------------------------

    /// `f` one level deeper: at [`MAX_DEPTH`](crate::MAX_DEPTH) the source
    /// is cut instead ([`Scanner::too_deep`], its text the kind `ctx`
    /// gives) and `f` does not run. Every routine that nests counts through
    /// here: a content run, and a CF tag an island body opens (`<cfquery>`
    /// × 5,000 would otherwise recurse through `cf_query` → `sql_island` →
    /// `cf_tag` with no bound).
    pub(super) fn nested(
        &mut self,
        out: &mut Vec<Node>,
        ctx: Ctx,
        f: impl FnOnce(&mut Self, &mut Vec<Node>),
    ) {
        if self.depth >= crate::MAX_DEPTH {
            self.too_deep(out, ctx);
            return;
        }
        self.depth += 1;
        f(self, out);
        self.depth -= 1;
    }

    /// HTML content with CF tags, to `stop` or the end.
    pub(super) fn content(&mut self, out: &mut Vec<Node>, ctx: Ctx, stop: ContentStop) {
        self.nested(out, ctx, |s, out| {
            let mut text = s.pos;
            while s.pos < s.end {
                if s.at_stop(stop) {
                    break;
                }
                if s.construct(out, ctx, &mut text) {
                    continue;
                }
                if s.bump() == '\n' {
                    s.flush(out, &mut text, ctx);
                }
            }
            s.flush(out, &mut text, ctx);
        });
    }

    /// Merge adjacent `Text` tokens among an island's own children, so its
    /// text between two elements is one token whatever lines it spans.
    /// Tokens inside a child element are that element's and never merge.
    pub(super) fn coalesce_text(children: &mut Vec<Node>) {
        let mut i = 1;
        while i < children.len() {
            let merge = match (&children[i - 1], &children[i]) {
                (Node::Token(a), Node::Token(b)) => {
                    a.kind == TokenKind::Text
                        && b.kind == TokenKind::Text
                        && a.span.end == b.span.start
                }
                _ => false,
            };
            if merge {
                let end = children[i].span().end;
                if let Node::Token(a) = &mut children[i - 1] {
                    a.span.end = end;
                }
                children.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Run `f` with the scanner's end pulled in to `end`.
    ///
    /// A delimiter that ends a whole region — a `<script>` / `<style>`
    /// island's escape, `</cfquery>`, `</cf(output|mail)>`, `</cffunction>`
    /// — ends everything nested inside it too, so the region is scanned with
    /// its end there and nothing nested can run past it.
    pub(super) fn scoped<T>(&mut self, end: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        let save = self.end;
        self.end = end.clamp(self.pos, save);
        let out = f(self);
        // A `too_deep` cut inside the region ends the source for the
        // enclosing routines too, so it survives the restore.
        self.end = save.min(self.cut);
        out
    }

    pub(super) fn at_stop(&self, stop: ContentStop) -> bool {
        let rest = self.rest();
        match stop {
            ContentStop::Eof => false,
            ContentStop::CfFunction => closes_tag(rest, "cffunction").is_some(),
            ContentStop::Output => {
                closes_tag(rest, "cfoutput").is_some() || closes_tag(rest, "cfmail").is_some()
            }
            ContentStop::Class => closes_tag(rest, "cfcomponent").is_some(),
            ContentStop::Interface => closes_tag(rest, "cfinterface").is_some(),
        }
    }

    /// One rule at the current position, the CFML rules before the HTML ones.
    /// Flushes the pending text first when something matches.
    fn construct(&mut self, out: &mut Vec<Node>, ctx: Ctx, text: &mut usize) -> bool {
        let rest = self.rest();
        let Some(&first) = rest.as_bytes().first() else {
            return false;
        };
        if ctx == Ctx::Source {
            if first == b'<' && (word_ci(rest, "<cfcomponent") || word_ci(rest, "<cfinterface")) {
                self.flush(out, text, ctx);
                self.cf_class(out);
                *text = self.pos;
                return true;
            }
            return false;
        }
        match first {
            b'<' => {
                if rest.starts_with("<!---") {
                    self.flush(out, text, ctx);
                    self.cf_comment(out);
                    *text = self.pos;
                    return true;
                }
                if ctx == Ctx::Interface {
                    // A `<cfinterface>` body: comments and `<cffunction>`
                    // only.
                    if self.cf_function_ahead() {
                        self.flush(out, text, ctx);
                        self.cf_tag(out);
                        *text = self.pos;
                        return true;
                    }
                    return false;
                }
                if self.cf_tag_ahead() {
                    self.flush(out, text, ctx);
                    self.cf_tag(out);
                    *text = self.pos;
                    return true;
                }
                if ctx.html() && self.html_ahead() {
                    self.flush(out, text, ctx);
                    self.html_construct(out, ctx);
                    *text = self.pos;
                    return true;
                }
                false
            }
            b'#' if ctx.hash() => {
                self.flush(out, text, ctx);
                self.template_expression(out);
                *text = self.pos;
                true
            }
            b'&' => {
                let len = if ctx.hash() {
                    self.hash_entity_len().or_else(|| self.entity_len())
                } else if ctx.html() {
                    self.entity_len()
                } else {
                    None
                };
                match len {
                    Some(_) => {
                        self.flush(out, text, ctx);
                        self.entity(out, ctx);
                        *text = self.pos;
                        true
                    }
                    None => false,
                }
            }
            b'>' if ctx.html() => {
                // A bare `>` is a `Text` token of its own.
                self.flush(out, text, ctx);
                self.take(out, 1, TokenKind::Text);
                *text = self.pos;
                true
            }
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // Entry
    // -----------------------------------------------------------------------

    /// The document: leading whitespace and `<!--- --->` comments, then either
    /// the `<cfcomponent>` / `<cfinterface>` path or HTML content to the end.
    pub(super) fn document(&mut self, out: &mut Vec<Node>) {
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            if rest.starts_with("<!---") {
                self.flush(out, &mut text, Ctx::Html);
                self.cf_comment(out);
                text = self.pos;
                continue;
            }
            let c = rest.chars().next().unwrap();
            if c.is_whitespace() {
                self.pos += c.len_utf8();
                if c == '\n' {
                    self.flush(out, &mut text, Ctx::Html);
                }
                continue;
            }
            self.flush(out, &mut text, Ctx::Html);
            if word_ci(rest, "<cfcomponent") || word_ci(rest, "<cfinterface") {
                // A file that starts with a component: the rest of the file
                // is its tail ([`Ctx::Source`]).
                self.cf_class(out);
                self.content(out, Ctx::Source, ContentStop::Eof);
                text = self.pos;
                continue;
            }
            self.content(out, Ctx::Html, ContentStop::Eof);
            text = self.pos;
        }
        self.flush(out, &mut text, Ctx::Html);
    }
}

/// `text` starts with `prefix`, ASCII case insensitively.
pub(super) fn starts_ci(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// `text` starts with `prefix` and a `\b` word break follows it.
pub(super) fn word_ci(text: &str, prefix: &str) -> bool {
    starts_ci(text, prefix)
        && text
            .as_bytes()
            .get(prefix.len())
            .is_none_or(|&b| !(b.is_ascii_alphanumeric() || b == b'_'))
}

/// A node's tokens in source order, `#…#` and ```` ``` ```` elements kept
/// whole: what the builder puts in an element that allows no children. A
/// key-value's separator gets back the kind the script parser read it with
/// ([`key_value::read_separator`](crate::key_value::read_separator)).
pub(super) fn flatten_to_templates(src: &str, node: Node, out: &mut Vec<Node>) {
    match node {
        Node::Element(mut e)
            if !matches!(
                e.kind,
                ElementKind::TemplateExpression | ElementKind::TagIsland
            ) =>
        {
            if e.kind == ElementKind::KeyValue {
                crate::key_value::read_separator(src, &mut e);
            }
            let mut parts: Vec<Node> = Vec::new();
            parts.extend(e.open.map(Node::Token));
            parts.extend(e.children);
            for item in e.items {
                parts.extend(item.children);
                parts.extend(item.separator.map(Node::Token));
            }
            parts.extend(e.close.map(Node::Token));
            parts.sort_by_key(|n| n.span().start);
            for part in parts {
                flatten_to_templates(src, part, out);
            }
        }
        node => out.push(node),
    }
}
