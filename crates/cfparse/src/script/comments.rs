//! Trivia: whitespace, the four comment forms and `cfformat-ignore` regions.
//!
//! Token boundaries are line-based, and part of the tree's contract: a
//! whitespace run ends at the first `\n` (included), and comment content is
//! one token per line unless something inside the comment splits it: a
//! nested `<!---`, or a doc comment's `@tag` at the start of a line.

use crate::tree::{Delim, Element, ElementKind, Node, Punct, Token, TokenKind};

use crate::scan::{script_marker, Marker};

use super::lexer;
use super::parser::Parser;

impl Parser<'_> {
    /// Whitespace, newlines, comments and ignore regions, as siblings.
    pub(super) fn trivia(&mut self, out: &mut Vec<Node>) {
        loop {
            if let Some(end) = lexer::whitespace(self.src, self.at()) {
                self.emit_whitespace(out, end);
                continue;
            }
            if !self.comment(out) {
                return;
            }
        }
    }

    /// [`Parser::trivia`] but stopping before a newline (an `import` path
    /// ends at the line's end).
    pub(super) fn trivia_no_newline(&mut self, out: &mut Vec<Node>) {
        loop {
            let at = self.at();
            let mut end = at;
            while self
                .src
                .as_bytes()
                .get(end)
                .is_some_and(|c| c.is_ascii_whitespace() && *c != b'\n')
            {
                end += 1;
            }
            if end > at {
                self.emit(out, end, TokenKind::Whitespace);
                continue;
            }
            if self.peek() == Some('\n') || !self.comment(out) {
                return;
            }
        }
    }

    /// Whitespace tokens only; a comment after them is left to the caller.
    pub(super) fn whitespace_only(&mut self, out: &mut Vec<Node>) {
        while let Some(end) = lexer::whitespace(self.src, self.at()) {
            self.emit_whitespace(out, end);
        }
    }

    pub(super) fn emit_whitespace(&mut self, out: &mut Vec<Node>, end: usize) {
        let kind = if self.src[..end].ends_with('\n') {
            TokenKind::Newline
        } else {
            TokenKind::Whitespace
        };
        self.emit(out, end, kind);
    }

    /// One comment or ignore region; `false` when the next text is neither.
    fn comment(&mut self, out: &mut Vec<Node>) -> bool {
        if let Some(len) = script_marker(self.rest(), Marker::Start) {
            self.ignore_region(out, self.at() + len);
            return true;
        }
        if self.at_str("/**") && !self.rest()[3..].starts_with(['/', '*']) {
            let el = self.doc_comment();
            self.finish_into(out, el);
            return true;
        }
        if self.at_str("/*") {
            let el = self.block_comment();
            self.finish_into(out, el);
            return true;
        }
        if self.at_str("//") {
            self.line_comment(out);
            return true;
        }
        if self.at_str("<!---") {
            let el = self.tag_comment();
            self.finish_into(out, el);
            return true;
        }
        false
    }

    /// A `cfformat-ignore` region: one verbatim token from the start marker
    /// to the end marker (or the end of the source).
    fn ignore_region(&mut self, out: &mut Vec<Node>, marker_end: usize) {
        let start = self.at();
        let mut at = marker_end;
        let end = loop {
            if at >= self.src.len() {
                break self.src.len();
            }
            if let Some(len) = script_marker(&self.src[at..], Marker::End) {
                break at + len;
            }
            at += 1;
            while at < self.src.len() && !self.src.is_char_boundary(at) {
                at += 1;
            }
        };
        let mut el = self.element(ElementKind::Ignore);
        let token = self.take(end, TokenKind::Ignore);
        el.span = start as u32..end as u32;
        el.children.push(Node::Token(token));
        out.push(Node::Element(Box::new(el)));
    }

    /// `//` plus the rest of the line; the trailing newline becomes a
    /// sibling.
    fn line_comment(&mut self, out: &mut Vec<Node>) {
        let mut el = self.element(ElementKind::LineComment);
        el.open = Some(self.take(self.at() + 2, TokenKind::Punct(Punct::Open(Delim::Comment))));
        let end = lexer::line_end(self.src, self.at());
        let mut newline = None;
        if end > self.at() {
            let text = &self.src[self.at()..end];
            let kind = whitespace_kind(text).unwrap_or(TokenKind::CommentText);
            let mut token = self.take(end, kind);
            if self.src[..end].ends_with('\n') {
                newline = Some(Token {
                    span: token.span.end - 1..token.span.end,
                    kind: TokenKind::Newline,
                });
                token.span.end -= 1;
            }
            if !token.span.is_empty() {
                el.children.push(Node::Token(token));
            }
        }
        self.finish_into(out, el);
        if let Some(t) = newline {
            out.push(Node::Token(t));
        }
    }

    /// `/* … */`: content is one token per line.
    fn block_comment(&mut self) -> Element {
        let mut el = self.element(ElementKind::BlockComment);
        el.open = Some(self.take(self.at() + 2, TokenKind::Punct(Punct::Open(Delim::Comment))));
        loop {
            if self.eof() {
                return el;
            }
            if self.at_str("*/") {
                el.close = Some(self.take(
                    self.at() + 2,
                    TokenKind::Punct(Punct::Close(Delim::Comment)),
                ));
                return el;
            }
            let line = lexer::line_end(self.src, self.at());
            let end = match self.src[self.at()..line].find("*/") {
                Some(i) => self.at() + i,
                None => line,
            };
            let text = &self.src[self.at()..end];
            let kind = whitespace_kind(text).unwrap_or(TokenKind::CommentText);
            self.emit(&mut el.children, end, kind);
        }
    }

    /// `<!--- --->` in script, which nests: an inner `<!---` opens a comment
    /// element of its own inside this one.
    fn tag_comment(&mut self) -> Element {
        let mut el = self.element(ElementKind::BlockComment);
        el.open = Some(self.take(
            self.at() + 5,
            TokenKind::Punct(Punct::Open(Delim::TagComment)),
        ));
        loop {
            if self.eof() {
                return el;
            }
            if self.at_str("--->") {
                el.close = Some(self.take(
                    self.at() + 4,
                    TokenKind::Punct(Punct::Close(Delim::TagComment)),
                ));
                return el;
            }
            if self.at_str("<!---") && self.depth < crate::MAX_DEPTH {
                self.depth += 1;
                let inner = self.tag_comment();
                self.depth -= 1;
                self.finish_into(&mut el.children, inner);
                continue;
            }
            let line = lexer::line_end(self.src, self.at());
            // Past `MAX_DEPTH` an inner `<!---` is text.
            let rest = &self.src[self.at()..line];
            // The nested opener is searched from the next character, not the
            // next byte: `rest.get(1..)` is `None` inside a multibyte char.
            let next = rest.chars().next().map_or(0, char::len_utf8);
            let end = [
                rest.find("--->"),
                rest[next..].find("<!---").map(|i| i + next),
            ]
            .into_iter()
            .flatten()
            .min()
            .map_or(line, |i| self.at() + i);
            let text = &self.src[self.at()..end];
            let kind = whitespace_kind(text).unwrap_or(TokenKind::CommentText);
            self.emit(&mut el.children, end, kind);
        }
    }

    /// `/** … */`: content is one token per line, as in a block comment,
    /// except that a line starting with a `@tag` (after its `*`) gives the
    /// tag its own `DocTag` token.
    fn doc_comment(&mut self) -> Element {
        let mut el = self.element(ElementKind::DocComment);
        el.open = Some(self.take(self.at() + 3, TokenKind::Punct(Punct::Open(Delim::Comment))));
        loop {
            if self.eof() {
                return el;
            }
            if self.at_str("*/") {
                el.close = Some(self.take(
                    self.at() + 2,
                    TokenKind::Punct(Punct::Close(Delim::Comment)),
                ));
                return el;
            }
            if self.at_line_start() {
                if let Some(tag) = doc_tag(self.src, self.at()) {
                    self.emit_text(&mut el.children, tag.start);
                    self.emit(&mut el.children, tag.end, TokenKind::DocTag);
                }
            }
            // The rest of the line, up to the next `*/`.
            let line = lexer::line_end(self.src, self.at());
            let end = match self.src[self.at()..line].find("*/") {
                Some(i) => self.at() + i,
                None => line,
            };
            self.emit_text(&mut el.children, end);
        }
    }

    /// One token of comment text (whitespace-only runs keep their own kind).
    fn emit_text(&mut self, out: &mut Vec<Node>, end: usize) {
        if end <= self.at() {
            return;
        }
        let text = &self.src[self.at()..end];
        let kind = whitespace_kind(text).unwrap_or(TokenKind::CommentText);
        self.emit(out, end, kind);
    }

    fn at_line_start(&self) -> bool {
        self.at() == 0 || self.src.as_bytes()[self.at() - 1] == b'\n'
    }
}

/// `Whitespace` / `Newline` for an all-whitespace run, `None` otherwise.
fn whitespace_kind(text: &str) -> Option<TokenKind> {
    text.bytes().all(|b| b.is_ascii_whitespace()).then(|| {
        if text.ends_with('\n') {
            TokenKind::Newline
        } else {
            TokenKind::Whitespace
        }
    })
}

/// `^\s*\*?\s*(@\S*)\s`: the span of the `@tag` a doc comment line starts
/// with, which must be followed by whitespace.
fn doc_tag(src: &str, at: usize) -> Option<std::ops::Range<usize>> {
    let line = lexer::line_end(src, at);
    let b = src.as_bytes();
    let mut i = at;
    while i < line && b[i].is_ascii_whitespace() && b[i] != b'\n' {
        i += 1;
    }
    if b.get(i) == Some(&b'*') {
        i += 1;
        while i < line && b[i].is_ascii_whitespace() && b[i] != b'\n' {
            i += 1;
        }
    }
    if b.get(i) != Some(&b'@') {
        return None;
    }
    let start = i;
    while i < line && !b[i].is_ascii_whitespace() {
        i += 1;
    }
    // A trailing `\s` is mandatory.
    if i >= src.len() || !b[i].is_ascii_whitespace() {
        return None;
    }
    Some(start..i)
}
