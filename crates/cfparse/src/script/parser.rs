//! Recursive-descent CFScript parser producing the pre-post-pass tree.
//!
//! [`Parser`] reads normalised source left to right, a statement at a time,
//! and emits tokens and the delimited elements around them (blocks, groups,
//! strings, `#…#`, calls, structs, arrays), the key-values of an item or an
//! attribute list as it finishes the element holding them ([`finish`]), and
//! a declaration fused with its body ([`fuse`]); the post-passes then give
//! expressions their structure. The tree is the
//! contract: its kinds, token boundaries, delimiters and where trivia sits,
//! as the snapshot tests pin them.
//!
//! It never fails. Text no rule reads becomes an `Other` token or a
//! recovered region, and past [`MAX_DEPTH`] nested levels the rest of the
//! source is one unmatched run. A ```` ``` ```` tag island goes to the tag
//! scanner, whose sub-parse cannot fail either.

use std::ops::Range;

use crate::key_value;
use crate::postpass::{recovered, wrap, wrap_recovered};
use crate::tree::{
    BlockKind, Delim, Element, ElementKind, Ident, Item, Keyword, Literal, Mode, Node, Operator,
    Punct, RecoveryReason, StatementKind, Storage, Token, TokenKind,
};

use super::expressions::{AttrStyle, StringStyle};
use super::lexer::{self, skip_trivia, Comments, Op};
use crate::scan;
use crate::MAX_DEPTH;

/// Where an expression run stops: at a `,` or not, and at a line end or not
/// (the statement-level continuation rule).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Stop {
    /// A `,` ends the run instead of continuing it (an item of a
    /// comma-separated list, an initializer).
    pub(super) no_comma: bool,
    /// The run is a statement's: end of line ends it unless the next text
    /// continues it.
    pub(super) statement: bool,
}

impl Stop {
    pub(super) const EXPR: Stop = Stop {
        no_comma: false,
        statement: false,
    };
    pub(super) const NO_COMMA: Stop = Stop {
        no_comma: true,
        statement: false,
    };
    pub(super) const STATEMENT: Stop = Stop {
        no_comma: false,
        statement: true,
    };
}

pub(super) struct Parser<'a> {
    pub(super) src: &'a str,
    /// The whole source: `src` is cut short at `MAX_DEPTH` (`too_deep`).
    whole: &'a str,
    pub(super) pos: u32,
    /// Whether a string parsed right now is a tag/declaration attribute value
    /// (`Element::String { in_tag }`).
    pub(super) in_tag: bool,
    /// Set by `tag_attributes` when a `type="java"` attribute makes the
    /// function declaration expect a Java body.
    pub(super) java_body: bool,
    /// Skipping at the top level of a `#…#` or a tag's expression, where
    /// unmatched text stays one `Other` token instead of splitting at its
    /// whitespace ([`Parser::skip_one`]).
    pub(super) in_template: bool,
    /// Nesting of expressions and statement lists (`MAX_DEPTH`).
    pub(super) depth: u32,
    /// Set by a form after which the same statement keeps reading (`abort`
    /// without a `;`, `cffile(…)` without one): `statement_body` loops
    /// instead of the form calling it again. The tree is the same either way — the
    /// continuation's nodes are appended to the one element — but a
    /// recursive call added stack frames without moving `depth`, so
    /// `abort abort abort …` overflowed the stack at a few thousand.
    pub(super) resume_statement: bool,
    /// One entry per statement being parsed, innermost last: the first
    /// reason the parse gave up inside it. A marked
    /// statement is wrapped in a `Recovered` element when it ends.
    frames: Vec<Option<RecoveryReason>>,
}

impl<'a> Parser<'a> {
    /// A parser over `src` at nesting `depth` (0 for a whole source, the
    /// caller's for a fragment: [`MAX_DEPTH`] is one budget).
    pub(super) fn new(src: &'a str, depth: u32) -> Self {
        Parser {
            src,
            whole: src,
            pos: 0,
            in_tag: false,
            java_body: false,
            in_template: false,
            depth,
            resume_statement: false,
            frames: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Primitives
    // -----------------------------------------------------------------------

    pub(super) fn at(&self) -> usize {
        self.pos as usize
    }

    pub(super) fn rest(&self) -> &'a str {
        &self.src[self.at()..]
    }

    pub(super) fn eof(&self) -> bool {
        self.at() >= self.src.len()
    }

    pub(super) fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    pub(super) fn at_str(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    /// A keyword with `\b` on both sides at the current offset, ASCII
    /// case-insensitively ([`lexer::keyword_at`]).
    pub(super) fn at_word(&self, word: &str) -> bool {
        lexer::keyword_at(self.src, self.at(), word)
    }

    /// Take the text up to `end` as one token and advance.
    pub(super) fn take(&mut self, end: usize, kind: TokenKind) -> Token {
        debug_assert!(end > self.at() && end <= self.src.len());
        let span = self.pos..end as u32;
        self.pos = end as u32;
        Token { span, kind }
    }

    pub(super) fn emit(&mut self, out: &mut Vec<Node>, end: usize, kind: TokenKind) {
        let t = self.take(end, kind);
        out.push(Node::Token(t));
    }

    /// Take `len` bytes as one token.
    pub(super) fn emit_len(&mut self, out: &mut Vec<Node>, len: usize, kind: TokenKind) {
        let end = self.at() + len;
        self.emit(out, end, kind);
    }

    /// Text no rule matched: one run per line (a token never spans a line
    /// end), split again into whitespace and non-whitespace runs. Whitespace
    /// keeps its own kind; the rest is `Other`, a recovery.
    pub(super) fn emit_unmatched(&mut self, out: &mut Vec<Node>, end: usize) {
        self.emit_unmatched_as(out, end, RecoveryReason::Unmatched);
    }

    /// [`Parser::emit_unmatched`], recovering for `reason` when the run
    /// holds anything but whitespace.
    fn emit_unmatched_as(&mut self, out: &mut Vec<Node>, end: usize, reason: RecoveryReason) {
        let first = out.len();
        while self.at() < end {
            let line = lexer::line_end(self.src, self.at()).min(end);
            while self.at() < line {
                let bytes = self.src.as_bytes();
                let space = bytes[self.at()].is_ascii_whitespace();
                let mut to = self.at();
                while to < line && bytes[to].is_ascii_whitespace() == space {
                    to += 1;
                }
                if space {
                    let kind = if self.src[..to].ends_with('\n') {
                        TokenKind::Newline
                    } else {
                        TokenKind::Whitespace
                    };
                    self.emit(out, to, kind);
                } else {
                    self.emit(out, to, TokenKind::Other);
                }
            }
        }
        let other = out[first..]
            .iter()
            .any(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Other));
        if other {
            self.recover_run(out, first, reason);
        }
    }

    // -----------------------------------------------------------------------
    // Recovery
    // -----------------------------------------------------------------------

    /// The parse gave up here for `reason`: the innermost statement being
    /// parsed is marked (its first reason stands). `false` when no statement
    /// is open — the caller's run is the region.
    pub(super) fn recover(&mut self, reason: RecoveryReason) -> bool {
        match self.frames.last_mut() {
            Some(frame) => {
                frame.get_or_insert(reason);
                true
            }
            None => false,
        }
    }

    /// [`Parser::recover`] for the nodes `out[first..]` just emitted, which
    /// are the region themselves when no statement takes it.
    pub(super) fn recover_run(
        &mut self,
        out: &mut Vec<Node>,
        first: usize,
        reason: RecoveryReason,
    ) {
        if !self.recover(reason) && first < out.len() {
            wrap_recovered(out, first, out.len() - 1, reason);
        }
    }

    /// An `Invalid` token of `len` bytes, a recovery for `reason`.
    pub(super) fn emit_invalid(&mut self, out: &mut Vec<Node>, len: usize, reason: RecoveryReason) {
        let first = out.len();
        self.emit_len(out, len, TokenKind::Invalid);
        self.recover_run(out, first, reason);
    }

    /// At `MAX_DEPTH`: the rest of the source up to its trailing whitespace
    /// becomes unmatched text, the source ends there for every enclosing
    /// rule, and the caller returns. The entry point takes the whitespace
    /// (`finish`): inside the deepest element, a `<cfscript>` body's last
    /// newline printed as part of it and the tag's own break after it added
    /// a blank line on every run.
    pub(super) fn too_deep(&mut self, out: &mut Vec<Node>) -> bool {
        if self.depth < MAX_DEPTH {
            return false;
        }
        let end = self.src.trim_end().len().max(self.at());
        if self.at() < end {
            self.emit_unmatched_as(out, end, RecoveryReason::TooDeep);
        } else {
            self.recover(RecoveryReason::TooDeep);
        }
        self.src = &self.src[..end];
        true
    }

    /// The whitespace `too_deep` left, at the top level.
    fn finish(&mut self, out: &mut Vec<Node>) {
        if self.src.len() < self.whole.len() {
            self.src = self.whole;
            self.whitespace_only(out);
        }
    }

    /// A binding name after `var` (or in `for (var … in`): an identifier,
    /// and a reserved word too when what follows can only follow a name —
    /// `=`, `;`, `,`, `.` or `in` (so `var case = 1;` declares `case`).
    pub(super) fn binding_name(&self, at: usize) -> Option<usize> {
        let end = lexer::identifier(self.src, at)?;
        if !lexer::in_list(lexer::RESERVED_WORDS, &self.src[at..end]) {
            return Some(end);
        }
        let next = skip_spaces(self.src, end);
        let rest = &self.src[next..];
        let named = (rest.starts_with(['=', ';', ',', '.']) && !rest.starts_with("=="))
            || lexer::keyword_at(self.src, next, "in");
        named.then_some(end)
    }

    /// Whether the reserved word ending at `end` is a variable: what follows
    /// it on its line is something no keyword use of it allows — `;`, `.`,
    /// `,`, a closing bracket, a binary operator, or `in` (`out &= case;`,
    /// `case.label`, `for (case in q)`).
    pub(super) fn reserved_as_variable(&self, end: usize) -> bool {
        let next =
            end + self.src[end..].len() - self.src[end..].trim_start_matches([' ', '\t']).len();
        let rest = &self.src[next..];
        rest.starts_with([
            ';', '.', ',', ')', ']', '}', '&', '+', '-', '*', '/', '%', '^', '=', '<', '>', '!',
            '?', '|',
        ]) || lexer::keyword_at(self.src, next, "in")
            // An accessor or an operator on a later line (`catch` ⏎ `.x`,
            // `catch` ⏎ `& type`); a closer does not cross a line: `return`
            // ⏎ `}` is the keyword.
            || self.src[skip_spaces(self.src, end)..].starts_with([
                '.', '&', '+', '-', '*', '/', '%', '^', '=', '<', '>', '!', '?', '|',
            ])
    }

    /// Text no rule can start on: the parser advances until some rule can
    /// start again, and everything it passed is one unmatched run, which is
    /// what keeps the parser total. At the top level of a `#…#` or a tag's
    /// expression the run is a single `Other` token; elsewhere it splits at
    /// its whitespace ([`Parser::emit_unmatched`]).
    pub(super) fn skip_one(&mut self, out: &mut Vec<Node>) {
        // At least the character here, then everything after it no rule can
        // start on. The run always ends past this offset, so the caller makes
        // progress.
        let from = self.at();
        let mut end = from + self.src[from..].chars().next().map_or(1, char::len_utf8);
        // To the end of the line at most, the newline included; the line's
        // end is not looked up first (that is quadratic on a long line).
        let first_newline = self.src.as_bytes()[from] == b'\n';
        while !first_newline && end < self.src.len() {
            let c = self.src[end..].chars().next().unwrap();
            if c == '\n' {
                end += 1;
                break;
            }
            if !c.is_whitespace() && !self.unmatchable(end) {
                break;
            }
            end += c.len_utf8();
        }
        // Whitespace just before it matched nothing either, so it belongs to
        // the same region. A newline ends the region, so it stays.
        let mut start = from;
        while let Some(Node::Token(t)) = out.last() {
            if t.kind != TokenKind::Whitespace || t.span.end != start as u32 {
                break;
            }
            start = t.span.start as usize;
            out.pop();
        }
        self.pos = start as u32;
        if self.in_template {
            // Top level of a `#…#`: one `Other` token, whitespace and all.
            let first = out.len();
            self.emit(out, end, TokenKind::Other);
            self.recover_run(out, first, RecoveryReason::Unmatched);
        } else {
            self.emit_unmatched(out, end);
        }
    }

    /// Whether no rule of an expression context can start at `at`. Inside a
    /// `#…#` the closing delimiters are unmatchable too: only `#` ends it.
    /// `|` and `:` only matter doubled (`||`, `::`).
    fn unmatchable(&self, at: usize) -> bool {
        let Some(c) = self.src[at..].chars().next() else {
            return true;
        };
        if self.in_template && matches!(c, ';' | '}' | ')' | ']') {
            return true;
        }
        let doubled = |c: char| self.src[at + 1..].starts_with(c);
        match c {
            ';' | '}' | ')' | ']' | ',' | '.' | '[' | '(' | '{' | '#' | '"' | '\'' | '!' | '?'
            | '+' | '-' | '*' | '/' | '\\' | '%' | '&' | '^' | '<' | '>' | '=' => false,
            '|' | ':' => !doubled(c),
            _ => !c.is_alphanumeric() && c != '_' && c != '$',
        }
    }

    pub(super) fn element(&self, kind: ElementKind) -> Element {
        Element {
            kind,
            open: None,
            close: None,
            children: Vec::new(),
            items: Vec::new(),
            span: 0..0,
        }
    }

    // -----------------------------------------------------------------------
    // Entry point
    // -----------------------------------------------------------------------

    /// A tag mode fragment (`script::parse_fragment`): the source ends where
    /// the fragment does, so every rule sees the fragment's end as the end.
    pub(super) fn fragment(mut self, start: u32, kind: super::Fragment) -> Vec<Node> {
        self.pos = start;
        let mut out = Vec::new();
        match kind {
            super::Fragment::Statements => self.statement_list(&mut out, Stops::ROOT),
            super::Fragment::Expression => loop {
                self.template_contents(&mut out, true);
                if self.eof() {
                    break;
                }
                // The tag scanner's `#…#` end (`scan::hash_end`) can lie past
                // a `#` the expression stops at — `#(}#x`: the `#` inside
                // `(` opens a nested one for the scanner, not for the parser —
                // and the range is the fragment's to tile: the `#` is one
                // unmatched run, and the contents go on.
                self.skip_one(&mut out);
            },
            super::Fragment::TagExpression => {
                // A tag's script (`<cfset>`, `<cfif>`, …): a leading `var`
                // word as a storage type, then expressions, which loop like a
                // `#…#` until the tag ends.
                self.whitespace_only(&mut out);
                if self.at_word("var") {
                    self.emit_len(&mut out, 3, TokenKind::Storage(Storage::Type));
                }
                self.template_contents(&mut out, false);
            }
        }
        self.finish(&mut out);
        out
    }

    pub(super) fn run(mut self, mode: Mode) -> Element {
        let mut root = self.element(ElementKind::Root(mode));
        self.statement_list(&mut root.children, Stops::ROOT);
        self.finish(&mut root.children);
        root.span = 0..self.src.len() as u32;
        root
    }
}

/// What ends a statement list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Stops {
    /// `}` ends the list (a block).
    brace: bool,
    /// `case` / `default:` / `}` end the list (a case body).
    case: bool,
}

impl Stops {
    pub(super) const ROOT: Stops = Stops {
        brace: false,
        case: false,
    };
    pub(super) const BLOCK: Stops = Stops {
        brace: true,
        case: false,
    };
    pub(super) const CASE: Stops = Stops {
        brace: true,
        case: true,
    };
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

impl Parser<'_> {
    /// A statement list: trivia and statements until the list's terminator.
    pub(super) fn statement_list(&mut self, out: &mut Vec<Node>, stops: Stops) {
        if self.too_deep(out) {
            return;
        }
        self.depth += 1;
        self.statements(out, stops);
        self.depth -= 1;
    }

    fn statements(&mut self, out: &mut Vec<Node>, stops: Stops) {
        loop {
            // A `<cfscript>` body's comments are siblings too, as in a
            // script file, not the next statement's leading children.
            self.trivia(out);
            if self.eof() {
                return;
            }
            let c = self.peek().unwrap();
            if stops.brace && c == '}' {
                return;
            }
            if stops.case && (self.at_word("case") || self.at_default_label()) {
                return;
            }
            if self.at_str("```") {
                let island = self.tag_island();
                out.push(Node::Element(Box::new(island)));
                continue;
            }
            if matches!(c, ')' | '}' | ']') {
                let at = out.len();
                self.emit_invalid(out, 1, RecoveryReason::StrayCloser);
                merge_recovered(out, at);
                if stops.brace || stops.case {
                    // A stray closer ends the enclosing block or case here,
                    // unclosed.
                    return;
                }
                continue;
            }
            let before = self.pos;
            let statement = self.statement();
            if self.pos == before {
                self.skip_one(out);
                continue;
            }
            let at = out.len();
            self.finish_into(out, statement);
            merge_recovered(out, at);
        }
    }

    /// A ```` ``` ```` tag island: tag-mode source up to the next
    /// ```` ``` ````. The closing fence wins wherever it is — inside a string
    /// or a tag — and without one the island runs to the end of the file.
    /// The body goes to the tag scanner alone, spans in place; its
    /// post-passes run once, from the root.
    fn tag_island(&mut self) -> Element {
        let mut el = self.element(ElementKind::TagIsland);
        el.open = Some(self.take(self.at() + 3, TokenKind::Punct(Punct::Open(Delim::Fence))));
        let end = self
            .rest()
            .find("```")
            .map_or(self.src.len(), |i| self.at() + i);
        if end > self.at() {
            el.children = crate::tags::parse_fragment(self.src, self.pos..end as u32, self.depth);
            self.pos = end as u32;
        }
        if self.at_str("```") {
            el.close = Some(self.take(self.at() + 3, TokenKind::Punct(Punct::Close(Delim::Fence))));
        }
        el.span = element_span(&el).unwrap_or(self.pos..self.pos);
        el
    }

    /// Whether `default`, whitespace and `:` start here: the `default:` label
    /// that ends a case body.
    fn at_default_label(&self) -> bool {
        if !self.at_word("default") {
            return false;
        }
        let after = self.at() + "default".len();
        let after = self.src[after..]
            .find(|c: char| !c.is_whitespace())
            .map_or(self.src.len(), |i| after + i);
        self.src[after..].starts_with(':')
    }

    /// One statement, as a `Statement` element. A statement the parse gave
    /// up inside comes back as a `Recovered` element holding it, unfinished
    /// ([`Parser::finish_into`] finishes both).
    pub(super) fn statement(&mut self) -> Element {
        self.frames.push(None);
        let mut el = self.element(ElementKind::Statement(StatementKind::Expression));
        let start = self.pos;
        self.statement_body(&mut el);
        if self.pos == start && !self.eof() {
            // Nothing matched: the skipped run is the whole statement.
            self.skip_one(&mut el.children);
        }
        if el.children.is_empty() && el.close.is_none() {
            el.span = start..self.pos;
        }
        match self.frames.pop().flatten() {
            Some(reason) => {
                let mut wrapper = self.element(ElementKind::Recovered(reason));
                wrapper.span = el.span.clone();
                wrapper.children.push(Node::Element(Box::new(el)));
                wrapper
            }
            None => el,
        }
    }

    /// The statement's nodes, and its kind: the kind of the rule that read
    /// it first.
    pub(super) fn statement_body(&mut self, el: &mut Element) {
        let kind = self.statement_body_once(el);
        el.kind = ElementKind::Statement(kind);
        // A form after which the statement keeps reading sets
        // `resume_statement` rather than recursing: see the field. What it
        // reads next is part of this statement, whose kind stands.
        while std::mem::take(&mut self.resume_statement) {
            self.statement_body_once(el);
        }
    }

    /// One pass of the statement rules over `el`, returning the kind of the
    /// rule that matched; `statement_body` repeats it while a form asks to
    /// resume.
    fn statement_body_once(&mut self, el: &mut Element) -> StatementKind {
        // A lone `;` is an empty statement.
        if self.peek() == Some(';') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::EmptyTerminator)));
            return StatementKind::Empty;
        }
        // A reserved word assigned to or accessed (`switch = 3;`,
        // `case.x = 1;`) is a variable, not its keyword.
        if let Some(end) = lexer::identifier(self.src, self.at()) {
            let rest = &self.src[skip_spaces(self.src, end)..];
            if lexer::in_list(lexer::RESERVED_WORDS, &self.src[self.at()..end])
                && ((rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>"))
                    || rest.starts_with('.'))
            {
                return self.expression_statement(el);
            }
        }
        if let Some(kind) = self.component_declaration(el) {
            return kind;
        }
        if self.import_statement(el) {
            return StatementKind::Import;
        }
        if self.function_declaration_statement(el) {
            return StatementKind::Function;
        }
        if let Some(kind) = self.static_block(el) {
            return kind;
        }
        if self.variable_declaration(el) {
            return StatementKind::Declaration;
        }
        if self.conditional(el) || self.bare_block(el) {
            return StatementKind::Keyword;
        }
        if self.label(&mut el.children) {
            // After a label the statement rules match from the top again,
            // into the same statement. `a: b: c: …` nests like a body and
            // counts against the bound: otherwise only a tail call would keep
            // it off the stack, and only in release builds.
            if !self.too_deep(&mut el.children) {
                self.depth += 1;
                self.statement_body(el);
                self.depth -= 1;
            }
            return StatementKind::Keyword;
        }
        if self.flow_control(el) {
            return StatementKind::Flow;
        }
        if let Some(kind) = self.tag_in_script(el) {
            return kind;
        }
        self.expression_statement(el)
    }

    /// An expression statement: an expression run, then an optional `;`.
    /// Its kind is `Function` when the run starts with a function (the
    /// expression pass makes it `Expression` again if it builds the function
    /// into something larger, and `Assignment` for an assignment).
    fn expression_statement(&mut self, el: &mut Element) -> StatementKind {
        self.expression(&mut el.children, Stop::STATEMENT);
        self.trivia(&mut el.children);
        self.expect_semicolon(el);
        let first = el.children.iter().find(|n| !n.is_trivia());
        match first.and_then(Node::as_element).map(|e| &e.kind) {
            Some(
                ElementKind::Function { .. }
                | ElementKind::FunctionDecl
                | ElementKind::ArrowFunction,
            ) => StatementKind::Function,
            _ => StatementKind::Expression,
        }
    }

    /// A `;` here is the statement's terminator.
    pub(super) fn expect_semicolon(&mut self, el: &mut Element) {
        if self.peek() == Some(';') && el.close.is_none() {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Terminator)));
        }
    }

    /// `component` / `interface`, with an `abstract` or `final` modifier
    /// before a component: the header's attributes, then the body block.
    fn component_declaration(&mut self, el: &mut Element) -> Option<StatementKind> {
        let out = &mut el.children;
        // `abstract` / `final`, trivia, `component`. The lookahead runs the
        // consumer's own trivia routine and keeps what it emitted, so the
        // two agree on comments (`abstract /* c */ component`, `final` ⏎
        // `// c` ⏎ `component`).
        let mut modifier = None;
        for word in ["abstract", "final"] {
            if self.at_word(word) {
                let mark = self.pos;
                let end = self.at() + word.len();
                self.pos = end as u32;
                let mut trivia = Vec::new();
                self.trivia(&mut trivia);
                let after = self.pos;
                let hit = self.at_word("component");
                self.pos = mark;
                if hit {
                    modifier = Some((end, trivia, after));
                    break;
                }
            }
        }
        let is_component = modifier.is_some() || self.at_word("component");
        if !is_component && !self.at_word("interface") {
            return None;
        }
        let (word, decl, body) = if is_component {
            ("component", ElementKind::ClassDecl, BlockKind::Class)
        } else {
            (
                "interface",
                ElementKind::InterfaceDecl,
                BlockKind::Interface,
            )
        };
        let keyword = if is_component {
            Keyword::Component
        } else {
            Keyword::Interface
        };
        let mut header = self.element(decl);
        if let Some((end, mut trivia, after)) = modifier {
            self.emit(
                &mut header.children,
                end,
                TokenKind::Storage(Storage::Modifier),
            );
            header.children.append(&mut trivia);
            self.pos = after;
        }
        self.emit(
            &mut header.children,
            self.at() + word.len(),
            TokenKind::Keyword(keyword),
        );
        // The attributes run up to the body's `{`.
        let in_tag = std::mem::replace(&mut self.in_tag, true);
        self.tag_attributes(&mut header.children, AttrStyle::Component);
        self.in_tag = in_tag;
        let head = out.len();
        self.finish_into(out, header);
        if self.peek() == Some('{') {
            let block = self.block(body);
            self.finish_into(out, block);
            let kind = if is_component {
                ElementKind::Class
            } else {
                ElementKind::Interface
            };
            fuse(out, head, kind);
        }
        Some(if is_component {
            StatementKind::Class
        } else {
            StatementKind::Interface
        })
    }

    /// `import path;`: the path is dotted names, `*` or a string, on the
    /// keyword's line.
    fn import_statement(&mut self, el: &mut Element) -> bool {
        if !self.at_word("import") {
            return false;
        }
        let out = &mut el.children;
        // The `Import` element starts at the keyword.
        let mut path = self.element(ElementKind::Import);
        self.emit_len(
            &mut path.children,
            "import".len(),
            TokenKind::Keyword(Keyword::Import),
        );
        // The path ends before a newline or a `;`.
        loop {
            self.trivia_no_newline(&mut path.children);
            if self.eof() || matches!(self.peek(), Some('\n' | ';')) {
                break;
            }
            let c = self.peek().unwrap();
            if c == '\'' || c == '"' {
                let s = self.string(false, StringStyle::Script);
                self.finish_into(&mut path.children, s);
            } else if let Some(end) = lexer::dot_path(self.src, self.at()) {
                self.emit(&mut path.children, end, TokenKind::Ident(Ident::Variable));
            } else if c == '*' {
                self.emit_len(&mut path.children, 1, TokenKind::Literal(Literal::Constant));
            } else {
                break; // anything else ends the path
            }
        }
        self.finish_into(out, path);
        self.trivia(out);
        self.expect_semicolon(el);
        true
    }

    /// `static` then, past any whitespace, a block: a `StaticBlock`
    /// statement, or a `Declaration` when no block follows after all.
    fn static_block(&mut self, el: &mut Element) -> Option<StatementKind> {
        if !self.at_word("static") {
            return None;
        }
        let after = self.at() + "static".len();
        let after = skip_spaces(self.src, after);
        if !matches!(self.src[after..].chars().next(), Some('\n' | '{')) {
            return None;
        }
        let out = &mut el.children;
        let head = out.len();
        self.emit_len(out, "static".len(), TokenKind::Keyword(Keyword::Static));
        self.trivia(out);
        if self.peek() == Some('{') {
            let block = self.block(BlockKind::Static);
            self.finish_into(out, block);
            if fuse(out, head, ElementKind::StaticBlock) {
                return Some(StatementKind::StaticBlock);
            }
        }
        Some(StatementKind::Declaration)
    }

    /// A block as a statement of its own.
    fn bare_block(&mut self, el: &mut Element) -> bool {
        if self.peek() != Some('{') {
            return false;
        }
        let block = self.block(BlockKind::Plain);
        self.finish_into(&mut el.children, block);
        true
    }

    /// A label: `name:` not followed by another `:`.
    fn label(&mut self, out: &mut Vec<Node>) -> bool {
        let Some(end) = lexer::identifier(self.src, self.at()) else {
            return false;
        };
        let colon = skip_spaces(self.src, end);
        if !self.src[colon..].starts_with(':') || self.src[colon + 1..].starts_with(':') {
            return false;
        }
        self.emit(out, end, TokenKind::Ident(Ident::Label));
        self.whitespace_only(out);
        self.emit_len(out, 1, TokenKind::Punct(Punct::Colon));
        self.trivia(out);
        true
    }

    /// `break` / `continue` with an optional label, `abort`, and `return`,
    /// `throw`, `rethrow`, `include` with an optional expression.
    fn flow_control(&mut self, el: &mut Element) -> bool {
        for (word, keyword) in [("break", Keyword::Break), ("continue", Keyword::Continue)] {
            if self.at_word(word) {
                self.emit_len(&mut el.children, word.len(), TokenKind::Keyword(keyword));
                self.expect_label(&mut el.children);
                self.trivia(&mut el.children);
                self.expect_semicolon(el);
                return true;
            }
        }
        if self.at_word("abort") {
            // Without a `;` the statement keeps reading after `abort`.
            self.emit_len(
                &mut el.children,
                "abort".len(),
                TokenKind::Keyword(Keyword::Flow),
            );
            self.trivia(&mut el.children);
            if self.peek() == Some(';') {
                el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::EmptyTerminator)));
                return true;
            }
            // `statement_body` reads the rest into `el`, in its loop.
            self.resume_statement = true;
            return true;
        }
        let restricted = [
            ("return", TokenKind::Keyword(Keyword::Return)),
            ("throw", TokenKind::Keyword(Keyword::Throw)),
            ("rethrow", TokenKind::Keyword(Keyword::Flow)),
            ("include", TokenKind::Keyword(Keyword::Flow)),
        ];
        for (word, kind) in restricted {
            if !self.at_word(word) {
                continue;
            }
            if word == "throw" || word == "rethrow" {
                // `throw(…)` / `throw (…)` is a function call.
                let after = skip_spaces(self.src, self.at() + word.len());
                if self.src[after..].starts_with('(') {
                    continue;
                }
            }
            if word == "include" {
                // `include template=…` / `include runeonce=…` is the tag in
                // script, not the keyword.
                let after = self.at() + word.len();
                let trimmed = skip_spaces(self.src, after);
                if trimmed > after {
                    let attr = &self.src[trimmed..];
                    if ["template", "runeonce"].iter().any(|a| {
                        attr.get(..a.len())
                            .is_some_and(|h| h.eq_ignore_ascii_case(a))
                            && attr[a.len()..].starts_with('=')
                    }) {
                        continue;
                    }
                }
            }
            self.emit_len(&mut el.children, word.len(), kind);
            // A bare keyword at the end of the line takes no expression.
            if self.at_line_ending() {
                return true;
            }
            self.expression_statement(el);
            return true;
        }
        false
    }

    /// The optional label after `break` / `continue`, on the keyword's own
    /// line: a name on the *next* line is not one, so `break` ⏎ `case 2:`
    /// ends the case.
    fn expect_label(&mut self, out: &mut Vec<Node>) {
        let line = lexer::line_end(self.src, self.at());
        let at = skip_trivia(self.src, self.at(), Comments::Block).min(line);
        if at >= line {
            return;
        }
        let Some(end) = lexer::identifier(self.src, at) else {
            return;
        };
        self.trivia(out);
        if self.at() != at {
            return;
        }
        // A reserved word is a label here too.
        self.emit(out, end, TokenKind::Ident(Ident::Label));
    }
}

// ---------------------------------------------------------------------------
// Blocks and keyword statements
// ---------------------------------------------------------------------------

impl Parser<'_> {
    /// `{ … }`: a statement list up to `}` (a plain, function, component or
    /// interface body).
    pub(super) fn block(&mut self, kind: BlockKind) -> Element {
        let mut el = self.element(ElementKind::Block(kind));
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Brace))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        self.statement_list(&mut el.children, Stops::BLOCK);
        self.in_tag = in_tag;
        if self.peek() == Some('}') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::Brace))));
        }
        el
    }

    /// A `switch` body: a block whose statements are `Case` elements.
    fn switch_block(&mut self) -> Element {
        let mut el = self.element(ElementKind::Block(BlockKind::Plain));
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Brace))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        loop {
            self.trivia(&mut el.children);
            if self.eof() || self.peek() == Some('}') {
                break;
            }
            if self.at_word("case") || self.at_default_label() {
                let case = self.switch_case();
                self.finish_into(&mut el.children, case);
                continue;
            }
            // A statement outside any case (`switch (x) { switch (y) { …`)
            // nests like a body.
            if self.too_deep(&mut el.children) {
                break;
            }
            let before = self.pos;
            self.depth += 1;
            let statement = self.statement();
            self.depth -= 1;
            if self.pos == before {
                self.skip_one(&mut el.children);
                continue;
            }
            let at = el.children.len();
            self.finish_into(&mut el.children, statement);
            merge_recovered(&mut el.children, at);
        }
        self.in_tag = in_tag;
        if self.peek() == Some('}') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::Brace))));
        }
        el
    }

    /// `case <expr>:` / `default:` and the statements after it.
    fn switch_case(&mut self) -> Element {
        let mut el = self.element(ElementKind::Case);
        let out = &mut el.children;
        if self.at_word("case") {
            self.emit_len(out, "case".len(), TokenKind::Keyword(Keyword::Case));
            self.expression(out, Stop::EXPR);
        } else {
            self.emit_len(out, "default".len(), TokenKind::Keyword(Keyword::Default));
        }
        self.trivia(out);
        if self.peek() == Some(':') {
            self.emit_len(out, 1, TokenKind::Punct(Punct::Colon));
        }
        self.statement_list(out, Stops::CASE);
        el
    }

    /// `switch`, `do … while`, `for`, `while`, `if` and `try`, plus a clause
    /// with no statement to attach to (`else`, `catch`, …).
    fn conditional(&mut self, el: &mut Element) -> bool {
        let out = &mut el.children;
        if self.at_word("switch") {
            let mut sw = self.element(ElementKind::Switch);
            self.emit_len(
                &mut sw.children,
                "switch".len(),
                TokenKind::Keyword(Keyword::Switch),
            );
            self.expect_parenthesized(&mut sw.children);
            self.trivia(&mut sw.children);
            if self.peek() == Some('{') {
                let block = self.switch_block();
                self.finish_into(&mut sw.children, block);
            }
            self.finish_into(out, sw);
            return true;
        }
        if self.at_word("do") {
            let mut dw = self.element(ElementKind::DoWhile);
            self.emit_len(
                &mut dw.children,
                "do".len(),
                TokenKind::Keyword(Keyword::DoWhile),
            );
            self.body_statement(&mut dw.children);
            self.trivia(&mut dw.children);
            if self.at_word("while") {
                self.emit_len(
                    &mut dw.children,
                    "while".len(),
                    TokenKind::Keyword(Keyword::While),
                );
                self.trivia(&mut dw.children);
                if self.peek() == Some('(') {
                    let group = self.parenthesized();
                    self.finish_into(&mut dw.children, group);
                }
            }
            self.finish_into(out, dw);
            // The `;` after `while (…)` belongs to the statement, not to an
            // empty statement of its own.
            self.trivia(out);
            self.expect_semicolon(el);
            return true;
        }
        if self.at_word("for") {
            let mut f = self.element(ElementKind::For);
            self.emit_len(
                &mut f.children,
                "for".len(),
                TokenKind::Keyword(Keyword::For),
            );
            self.for_condition(&mut f.children);
            self.body_statement(&mut f.children);
            self.finish_into(out, f);
            return true;
        }
        if self.at_word("while") {
            let mut w = self.element(ElementKind::While);
            self.emit_len(
                &mut w.children,
                "while".len(),
                TokenKind::Keyword(Keyword::While),
            );
            self.expect_parenthesized(&mut w.children);
            self.body_statement(&mut w.children);
            self.finish_into(out, w);
            return true;
        }
        if self.at_word("if") {
            let mut chain = self.element(ElementKind::If);
            self.emit_len(
                &mut chain.children,
                "if".len(),
                TokenKind::Keyword(Keyword::If),
            );
            self.expect_parenthesized(&mut chain.children);
            self.body_statement(&mut chain.children);
            self.if_chain(&mut chain.children);
            self.finish_into(out, chain);
            return true;
        }
        if self.at_word("try") {
            let mut t = self.element(ElementKind::Try);
            self.emit_len(
                &mut t.children,
                "try".len(),
                TokenKind::Keyword(Keyword::Try),
            );
            self.block_scope(&mut t.children);
            self.try_chain(&mut t.children);
            self.finish_into(out, t);
            return true;
        }
        if let Some(mut clause) = self.orphan_clause() {
            let kind = clause.kind.clone();
            match kind {
                ElementKind::ElseIf => {
                    self.expect_parenthesized(&mut clause.children);
                    self.body_statement(&mut clause.children);
                }
                ElementKind::Else => self.body_statement(&mut clause.children),
                ElementKind::Finally => self.block_scope(&mut clause.children),
                ElementKind::Catch => {
                    self.catch_binding(&mut clause.children);
                    self.block_scope(&mut clause.children);
                }
                _ => unreachable!(),
            }
            self.finish_into(out, clause);
            return true;
        }
        false
    }

    /// `else if` / `else` / `finally` / `catch` with their keyword tokens
    /// already taken.
    fn orphan_clause(&mut self) -> Option<Element> {
        if self.at_else_if() {
            let mut el = self.element(ElementKind::ElseIf);
            self.else_if_keywords(&mut el.children);
            return Some(el);
        }
        let (kind, keyword, word) = if self.at_word("else") {
            (ElementKind::Else, Keyword::Else, "else")
        } else if self.at_word("finally") {
            (ElementKind::Finally, Keyword::Finally, "finally")
        } else if self.at_word("catch") {
            (ElementKind::Catch, Keyword::Catch, "catch")
        } else {
            return None;
        };
        let mut el = self.element(kind);
        self.emit_len(&mut el.children, word.len(), TokenKind::Keyword(keyword));
        Some(el)
    }

    /// `else\s+if\b`.
    fn at_else_if(&self) -> bool {
        if !self.at_word("else") {
            return false;
        }
        let after = self.at() + "else".len();
        let at = skip_spaces(self.src, after);
        at > after && lexer::keyword_at(self.src, at, "if")
    }

    /// The `else` and `if` of an `else if` and the whitespace between them,
    /// each its own token: the two words may be lines apart.
    fn else_if_keywords(&mut self, out: &mut Vec<Node>) {
        self.emit_len(out, "else".len(), TokenKind::Keyword(Keyword::Else));
        self.whitespace_only(out);
        self.emit_len(out, "if".len(), TokenKind::Keyword(Keyword::If));
    }

    /// The `else if` / `else` clauses after an `if`.
    fn if_chain(&mut self, out: &mut Vec<Node>) {
        loop {
            self.trivia(out);
            if self.at_else_if() {
                let mut clause = self.element(ElementKind::ElseIf);
                self.else_if_keywords(&mut clause.children);
                self.expect_parenthesized(&mut clause.children);
                self.body_statement(&mut clause.children);
                self.finish_into(out, clause);
                continue;
            }
            if self.at_word("else") {
                let mut clause = self.element(ElementKind::Else);
                self.emit_len(
                    &mut clause.children,
                    "else".len(),
                    TokenKind::Keyword(Keyword::Else),
                );
                self.body_statement(&mut clause.children);
                self.finish_into(out, clause);
                return;
            }
            return;
        }
    }

    /// The `catch` / `finally` clauses after a `try`.
    fn try_chain(&mut self, out: &mut Vec<Node>) {
        loop {
            self.trivia(out);
            if self.at_word("catch") {
                let mut clause = self.element(ElementKind::Catch);
                self.emit_len(
                    &mut clause.children,
                    "catch".len(),
                    TokenKind::Keyword(Keyword::Catch),
                );
                self.catch_binding(&mut clause.children);
                self.block_scope(&mut clause.children);
                self.finish_into(out, clause);
                continue;
            }
            if self.at_word("finally") {
                let mut clause = self.element(ElementKind::Finally);
                self.emit_len(
                    &mut clause.children,
                    "finally".len(),
                    TokenKind::Keyword(Keyword::Finally),
                );
                self.block_scope(&mut clause.children);
                self.finish_into(out, clause);
                return;
            }
            return;
        }
    }

    /// A keyword statement's body: a `Block`, or a `Statement` of its own. An
    /// unbraced body nests like a block (`if (a) if (b) …`,
    /// `while (x) while (y) …`, `else if` chains): it counts against
    /// [`MAX_DEPTH`] the way `statement_list` does.
    fn body_statement(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.eof() {
            return;
        }
        if self.peek() == Some('{') {
            let block = self.block(BlockKind::Plain);
            self.finish_into(out, block);
            return;
        }
        if self.too_deep(out) {
            return;
        }
        let before = self.pos;
        self.depth += 1;
        let statement = self.statement();
        self.depth -= 1;
        if self.pos == before {
            self.skip_one(out);
            return;
        }
        self.finish_into(out, statement);
    }

    /// A `Block` or nothing (`try`, `catch`, `finally`).
    fn block_scope(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.peek() == Some('{') {
            let block = self.block(BlockKind::Plain);
            self.finish_into(out, block);
        }
    }

    /// `open` … its closer around comma-separated items read by
    /// [`Parser::delimited_items`], with `in_tag` off inside. The closer is
    /// taken when it is the one `open` needs; `closers` are the characters
    /// that end the list (that closer, and any that leave it unclosed).
    pub(super) fn delimited(
        &mut self,
        kind: ElementKind,
        open: Delim,
        closers: &[char],
        resume: Option<Stop>,
        read: impl FnMut(&mut Self, &mut Vec<Node>),
    ) -> Element {
        let close = match open {
            Delim::Paren => ')',
            Delim::Bracket => ']',
            _ => '}',
        };
        let mut el = self.element(kind);
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(open))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        el.items.push(Item::default());
        self.delimited_items(&mut el, (',', Punct::Comma), closers, resume, read);
        self.in_tag = in_tag;
        if self.peek() == Some(close) {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(open))));
        }
        el
    }

    /// A delimited list's items, from `el`'s last item on: each item's
    /// trivia, then `read` up to the `separator` (its character and kind),
    /// a character of `closers` or the end, where the list stops. A `read`
    /// that takes nothing is at a character no rule reads, taken as one
    /// `other` token; with `resume`, the item's expression goes on after it.
    /// An empty last item is dropped when the element is [`finish`]ed.
    pub(super) fn delimited_items(
        &mut self,
        el: &mut Element,
        separator: (char, Punct),
        closers: &[char],
        resume: Option<Stop>,
        mut read: impl FnMut(&mut Self, &mut Vec<Node>),
    ) {
        loop {
            let mut nodes = std::mem::take(&mut el.items.last_mut().unwrap().children);
            self.trivia(&mut nodes);
            if self.eof() || self.peek().is_some_and(|c| closers.contains(&c)) {
                el.items.last_mut().unwrap().children = nodes;
                return;
            }
            if self.peek() == Some(separator.0) {
                el.items.last_mut().unwrap().children = nodes;
                let sep = self.take(self.at() + 1, TokenKind::Punct(separator.1));
                el.items.last_mut().unwrap().separator = Some(sep);
                el.items.push(Item::default());
                continue;
            }
            let before = self.pos;
            read(self, &mut nodes);
            if self.pos == before {
                self.skip_one(&mut nodes);
                if let Some(stop) = resume {
                    self.expression_tail(&mut nodes, stop);
                }
            }
            el.items.last_mut().unwrap().children = nodes;
        }
    }

    /// A parenthesized group, when one follows.
    fn expect_parenthesized(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.peek() == Some('(') {
            let group = self.parenthesized();
            self.finish_into(out, group);
        }
    }

    /// A `catch` header: `(`, an optional type (dotted or quoted), an
    /// optional `var`, the variable name and its member accesses, `)`.
    fn catch_binding(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.peek() != Some('(') {
            return;
        }
        let mut el = self.element(ElementKind::Group);
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Paren))));
        let out2 = &mut el.children;
        self.trivia(out2);
        // `("java.lang.Exception" e)` — a quoted type name.
        if matches!(self.peek(), Some('\'' | '"')) {
            let quote = self.peek().unwrap();
            if let Some(i) = self.rest()[1..].find(quote) {
                let end = self.at() + 1 + i + 1;
                self.emit_len(out2, 1, TokenKind::Punct(Punct::Open(Delim::String)));
                if end - 1 > self.at() {
                    self.emit(out2, end - 1, TokenKind::Ident(Ident::ClassName));
                }
                self.emit_len(out2, 1, TokenKind::Punct(Punct::Close(Delim::String)));
                self.trivia(out2);
            }
        } else if let Some(end) = lexer::dot_path(self.src, self.at()) {
            // A dotted name is the type only when whitespace and another name
            // follow it.
            let after = skip_spaces(self.src, end);
            if after > end && lexer::identifier(self.src, after).is_some() {
                self.emit(out2, end, TokenKind::Ident(Ident::ClassName));
                self.whitespace_only(out2);
            }
        }
        if self.at_word("var") {
            let after = skip_spaces(self.src, self.at() + 3);
            if after > self.at() + 3 && lexer::identifier(self.src, after).is_some() {
                self.emit_len(out2, 3, TokenKind::Storage(Storage::Type));
                self.whitespace_only(out2);
            }
        }
        if let Some(end) = lexer::identifier(self.src, self.at()) {
            self.emit(out2, end, TokenKind::Ident(Ident::Variable));
            // `catch (local.exp)`: the binding's member accesses, an accessor
            // chain like any other.
            while self.property_access(out2) {}
        }
        // Anything else before `)` is unmatched text.
        let stop = self
            .rest()
            .find(')')
            .map_or(self.src.len(), |i| self.at() + i);
        self.emit_unmatched(out2, stop);
        self.trivia(out2);
        if self.peek() == Some(')') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::Paren))));
        }
        self.finish_into(out, el);
    }

    /// A `for` header, itemized: `for (;;)` / `for (x in y)`.
    fn for_condition(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.peek() != Some('(') {
            return;
        }
        let mut el = self.element(ElementKind::Group);
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Paren))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        el.items.push(Item::default());
        if self.for_is_in() {
            // `for (x in y)`: `var`? binding `in` expression, one item. After
            // `var` the binding is a variable name, so a scope name there
            // (`var local in …`) is an ordinary `Ident::Variable`.
            let item = el.items.last_mut().unwrap();
            let mut nodes = std::mem::take(&mut item.children);
            self.trivia(&mut nodes);
            if self.at_word("var") {
                self.emit_len(&mut nodes, 3, TokenKind::Keyword(Keyword::Var));
                self.trivia(&mut nodes);
                if matches!(self.peek(), Some('[' | '{')) {
                    self.binding_pattern(&mut nodes);
                    self.trivia(&mut nodes);
                    if self.at_word("in") {
                        self.emit_len(&mut nodes, 2, TokenKind::Operator(Operator::In));
                    }
                } else if self.binding_name(self.at()).is_some() {
                    // A variable name and its member accesses
                    // (`var local.x in …`).
                    self.variable_binding(&mut nodes);
                    self.trivia(&mut nodes);
                    if self.at_word("in") {
                        self.emit_len(&mut nodes, 2, TokenKind::Operator(Operator::In));
                    }
                }
            }
            el.items.last_mut().unwrap().children = nodes;
        } else {
            // `for (;;)`: declaration `;` expression `;` expressions.
            let mut nodes = std::mem::take(&mut el.items.last_mut().unwrap().children);
            self.trivia(&mut nodes);
            if self.at_word("var") {
                // Comma-separated bindings; a scope name bound after `var`
                // is a variable name.
                self.emit_len(&mut nodes, 3, TokenKind::Keyword(Keyword::Var));
                loop {
                    self.variable_binding(&mut nodes);
                    self.trivia(&mut nodes);
                    if self.peek() != Some(',') {
                        break;
                    }
                    self.emit_len(&mut nodes, 1, TokenKind::Punct(Punct::Comma));
                }
            }
            el.items.last_mut().unwrap().children = nodes;
        }
        self.delimited_items(
            &mut el,
            (';', Punct::ExprSeparator),
            &[')', '}', ']'],
            Some(Stop::EXPR),
            |p, nodes| p.expression(nodes, Stop::EXPR),
        );
        self.in_tag = in_tag;
        if self.peek() == Some(')') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::Paren))));
        }
        self.finish_into(out, el);
    }

    /// Whether a `for` header is `x in y` rather than `;;`, by lookahead:
    /// an optional `var`, a binding (a name, or a balanced `[…]` / `{…}`
    /// pattern), then `in`.
    fn for_is_in(&self) -> bool {
        let mut at = skip_trivia(self.src, self.at(), Comments::Block);
        let var = lexer::keyword_at(self.src, at, "var");
        if var {
            at = skip_trivia(self.src, at + 3, Comments::Block);
        }
        at = match self.src[at..].chars().next() {
            Some('[') => match scan::balanced_code(self.src, at, b'[', b']', self.depth) {
                Some(end) => end,
                None => return false,
            },
            Some('{') => match scan::balanced_code(self.src, at, b'{', b'}', self.depth) {
                Some(end) => end,
                None => return false,
            },
            _ => match lexer::identifier(self.src, at) {
                // After `var`, member accesses: `var local.x in …`.
                Some(mut end) if var => {
                    while self.src[end..].starts_with('.') {
                        match lexer::identifier(self.src, end + 1) {
                            Some(e) => end = e,
                            None => break,
                        }
                    }
                    end
                }
                Some(end) => end,
                None => return false,
            },
        };
        at = skip_trivia(self.src, at, Comments::Block);
        lexer::keyword_at(self.src, at, "in")
    }
}

// ---------------------------------------------------------------------------
// Finalisation
// ---------------------------------------------------------------------------

impl Parser<'_> {
    /// Finish `el` and append it — plus whatever it hands back — to `out`.
    ///
    /// A block, parenthesis, bracket, string or `#…#` left open is a
    /// recovery ([`Unclosed`](RecoveryReason::Unclosed)): its statement's,
    /// or the element's own region outside any statement. A string still
    /// open where its fragment ends (`x = "price #";` in a `<cfscript>`
    /// body) printed as written with no warning before.
    pub(super) fn finish_into(&mut self, out: &mut Vec<Node>, mut el: Element) {
        if let ElementKind::Recovered(reason) = el.kind {
            // A statement from `statement`: finished inside its region, its
            // trailing trivia handed back after it.
            let Some(Node::Element(statement)) = el.children.pop() else {
                unreachable!("a recovered statement holds its statement");
            };
            let (element, handback) = finish(*statement);
            if let Some(e) = element {
                out.push(Node::Element(Box::new(recovered(
                    vec![Node::Element(Box::new(e))],
                    reason,
                ))));
            }
            out.extend(handback);
            return;
        }
        let unclosed = el.close.is_none()
            && matches!(
                el.open,
                Some(Token {
                    kind: TokenKind::Punct(Punct::Open(
                        Delim::Paren
                            | Delim::Brace
                            | Delim::Bracket
                            | Delim::String
                            | Delim::Template
                    )),
                    ..
                })
            );
        let (element, handback) = finish(el);
        if let Some(e) = element {
            if unclosed && !self.recover(RecoveryReason::Unclosed) {
                out.push(Node::Element(Box::new(recovered(
                    vec![Node::Element(Box::new(e))],
                    RecoveryReason::Unclosed,
                ))));
            } else {
                out.push(Node::Element(Box::new(e)));
            }
        }
        out.extend(handback);
    }
}

/// A declaration read with its body: `out[head]` is its header (a
/// `FunctionDecl`, `ArrowFunction`, `ClassDecl` or `InterfaceDecl`, or the
/// `static` keyword) and the body the rule just read ends `out`. The two,
/// and whatever lies between them (trivia, an arrow's `=>`), become one
/// element of `kind`. A body left open outside any statement is a
/// recovered region rather than a block: the header then stays apart, as a
/// body-less `function f();` does. Returns whether they were fused.
pub(super) fn fuse(out: &mut Vec<Node>, head: usize, kind: ElementKind) -> bool {
    let body =
        matches!(out.last(), Some(Node::Element(e)) if matches!(e.kind, ElementKind::Block(_)));
    let fused = body && out.len() > head + 1;
    if fused {
        wrap(out, head, out.len() - 1, kind);
    }
    fused
}

/// A recovered statement's region takes in what the parse split off it on
/// its line: the unterminated statement just before it —
/// `b =` of `b = @;`, whose expression stopped at the `@` — and a lone `;`
/// just after it. `at` is where the statement list's newest element was
/// pushed.
fn merge_recovered(out: &mut Vec<Node>, at: usize) {
    let Some(Node::Element(e)) = out.get(at) else {
        return;
    };
    let lone_semicolon = e.kind.is_statement() && e.children.is_empty() && e.close.is_some();
    let is_recovered = matches!(e.kind, ElementKind::Recovered(_));
    if !is_recovered && !lone_semicolon {
        return;
    }
    // Back along the line, whitespace between: the unterminated statements
    // the region takes in (a recovered one only), up to an earlier region
    // that takes it all in. Found first and moved once, so a long line of
    // them (tag mode read as script) stays linear.
    let mut start = at;
    let mut region = None;
    let mut i = at;
    while i > 0 {
        i -= 1;
        match &out[i] {
            Node::Token(t) if t.kind == TokenKind::Whitespace => {}
            Node::Element(p) if matches!(p.kind, ElementKind::Recovered(_)) => {
                region = Some(i);
                break;
            }
            Node::Element(p) if is_recovered && p.kind.is_statement() && p.close.is_none() => {
                start = i;
            }
            _ => break,
        }
    }
    match region {
        // The earlier region takes the rest in; its reason stands.
        Some(r) => {
            let tail: Vec<Node> = out.drain(r + 1..=at).collect();
            let Node::Element(region) = &mut out[r] else {
                unreachable!("a region is an element");
            };
            for n in tail {
                match n {
                    Node::Element(e) if matches!(e.kind, ElementKind::Recovered(_)) => {
                        region.children.extend(e.children)
                    }
                    n => region.children.push(n),
                }
            }
            region.span.end = region
                .children
                .last()
                .map_or(region.span.end, |n| n.span().end);
        }
        None if start < at => {
            let head: Vec<Node> = out.drain(start..at).collect();
            let Node::Element(region) = &mut out[start] else {
                unreachable!("a region is an element");
            };
            region.span.start = head[0].span().start;
            region.children.splice(0..0, head);
        }
        None => {}
    }
}

/// Finish an element the parser has filled: drop an empty last item, build
/// its key-values ([`key_value`]), drop an element with nothing in it (an
/// island excepted), hand back the trailing trivia of a statement-like
/// element, and set its span.
pub(super) fn finish(mut el: Element) -> (Option<Element>, Vec<Node>) {
    if el
        .items
        .last()
        .is_some_and(|i| i.children.is_empty() && i.separator.is_none())
    {
        el.items.pop();
    }
    match el.kind {
        ElementKind::Call
        | ElementKind::Struct { .. }
        | ElementKind::Parameters
        | ElementKind::ScriptTagAttributes
        | ElementKind::Group => key_value::items(&mut el),
        ElementKind::FunctionDecl
        | ElementKind::ClassDecl
        | ElementKind::InterfaceDecl
        | ElementKind::Property
        | ElementKind::Param
        | ElementKind::ScriptTag { .. } => {
            let script_tag = el.kind == ElementKind::ScriptTag { acf: false };
            key_value::attributes(&mut el.children, script_tag);
        }
        _ => {}
    }
    let empty =
        el.open.is_none() && el.close.is_none() && el.children.is_empty() && el.items.is_empty();
    if empty && !matches!(el.kind, ElementKind::Island(_)) {
        return (None, Vec::new());
    }

    let mut handback: Vec<Node> = Vec::new();
    if ends_at_its_content(&el.kind) {
        let floor = el.close.as_ref().map_or(0, |t| t.span.end);
        // An ignore region is handed back with the trivia: a statement
        // without a `;` ends before it, as it does before a comment, or the
        // region would print as the statement's trailing comment.
        let is_tail = |n: &Node| {
            n.is_trivia() || matches!(n, Node::Element(e) if e.kind == ElementKind::Ignore)
        };
        while let Some(last) = el.children.last() {
            if !is_tail(last) || last.span().start < floor {
                break;
            }
            handback.extend(el.children.pop());
        }
        handback.reverse();
    }
    if el.kind.is_statement() && el.children.is_empty() && el.close.is_none() {
        return (None, handback);
    }
    el.span = element_span(&el).unwrap_or(el.span);
    (Some(el), handback)
}

/// Kinds whose span ends at their content: their trailing trivia is handed
/// back to the enclosing list.
fn ends_at_its_content(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Statement(_)
            | ElementKind::If
            | ElementKind::ElseIf
            | ElementKind::Else
            | ElementKind::For
            | ElementKind::While
            | ElementKind::DoWhile
            | ElementKind::Switch
            | ElementKind::Case
            | ElementKind::Try
            | ElementKind::Catch
            | ElementKind::Finally
    )
}

/// The span covering an element's delimiters, children and items; `None`
/// when it has none.
pub(super) fn element_span(el: &Element) -> Option<Range<u32>> {
    let mut start = u32::MAX;
    let mut end = 0;
    let mut see = |r: Range<u32>| {
        start = start.min(r.start);
        end = end.max(r.end);
    };
    if let Some(t) = &el.open {
        see(t.span.clone());
    }
    if let Some(t) = &el.close {
        see(t.span.clone());
    }
    for n in &el.children {
        see(n.span());
    }
    for i in &el.items {
        for n in &i.children {
            see(n.span());
        }
        if let Some(t) = &i.separator {
            see(t.span.clone());
        }
    }
    (start != u32::MAX).then_some(start..end)
}

// ---------------------------------------------------------------------------
// Shared scanning helpers
// ---------------------------------------------------------------------------

/// Skip ASCII whitespace, newlines included.
pub(super) fn skip_spaces(src: &str, mut at: usize) -> usize {
    while src.as_bytes().get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// The operator table lookup used in operand and operator position.
pub(super) fn match_op(src: &str, at: usize, table: &'static [Op]) -> Option<(usize, TokenKind)> {
    table
        .iter()
        .find_map(|op| lexer::op_at(src, at, op).map(|end| (end, op.kind)))
}
