//! Expression runs, literals, calls, declarations and the tag-in-script forms.
//!
//! The parser emits the **pre-post-pass** shape: flat operator runs inside
//! statements, `Call` argument lists as siblings of their callee tokens.
//! `postpass::expressions` and its neighbours build the structure; the
//! key-values of an item or an attribute list are built as the element that
//! holds them is finished (`parser::finish`, [`crate::key_value`]).

use crate::tree::{
    BlockKind, Delim, Element, ElementKind, Ident, Item, Keyword, Literal, Node, Operator, Prec,
    Punct, Quote, RecoveryReason, Storage, Token, TokenKind,
};

use super::lexer::{self, skip_trivia, Comments};
use super::parser::{fuse, match_op, skip_spaces, Parser, Stop};
use crate::scan;

/// Which rules a quoted run follows. They differ in what is special inside:
/// a script string emits a leading whitespace run as a token of its own, and
/// a component's `extends` or a property's `name` value does not interpolate,
/// so `#…#` there is plain text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StringStyle {
    /// A CFScript string.
    Script,
    /// An attribute value: `#…#` interpolates.
    Attribute,
    /// A component's `extends` or a property's `name` value: no
    /// interpolation.
    AttributeNamed,
}

/// What one attribute's value is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Value {
    /// Any attribute's value (an unquoted one is an `Unquoted` literal).
    Plain,
    /// A component's `extends`: the class it inherits from.
    InheritedClass,
    /// A property's `name`.
    PropertyName,
}

/// Where an attribute list sits, which decides where it ends and how its
/// values read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AttrStyle {
    /// A `property` statement: the `name` attribute is a property name.
    PropertyTag,
    /// A component header: values are strings or unquoted runs, up to `{`.
    Component,
    /// A function declaration's metadata, up to `;` or `{`.
    Declaration,
    /// A function parameter's attributes: they also end at `,` and `)`.
    Parameter,
    /// A tag in script (`param`, `lock`, `http`, …): the value is a script
    /// expression.
    Script,
}

impl Parser<'_> {
    // -----------------------------------------------------------------------
    // Expression runs
    // -----------------------------------------------------------------------

    /// An expression run: an operand, then operators and operands until
    /// `stop` ends it (also a `,` under `Stop::NO_COMMA`, and a line end the
    /// next text does not continue under `Stop::STATEMENT`).
    pub(super) fn expression(&mut self, out: &mut Vec<Node>, stop: Stop) {
        if self.too_deep(out) {
            return;
        }
        self.depth += 1;
        self.operand(out, stop);
        self.expression_tail(out, stop);
        self.depth -= 1;
    }

    /// The run after an operand: its operators and the operands after them.
    /// Also used on its own after a character no rule reads was skipped: the
    /// run goes on, so what follows is a postfix part (`q("a":["b"])`
    /// indexes; the brackets are no array literal).
    pub(super) fn expression_tail(&mut self, out: &mut Vec<Node>, stop: Stop) {
        // The operators are read even when no operand came first: `&= x` and
        // `[=]` are a binary operator with a missing left operand.
        while self.expression_end(out, stop) {
            self.operand(out, stop);
        }
    }

    /// Whether the run ends here: at the end of the source, at `;`, `}`, `)`
    /// or `]`, at `</cfscript>`, or at `,` under `no_comma`.
    fn at_break(&self, stop: Stop) -> bool {
        match self.peek() {
            None => true,
            Some(';' | '}' | ')' | ']') => true,
            Some(',') if stop.no_comma => true,
            _ => self.at_cfscript_end(),
        }
    }

    fn at_cfscript_end(&self) -> bool {
        scan::closes_tag(self.rest(), "cfscript").is_some()
    }

    /// One operand with its prefix operators: returns `false` when nothing was
    /// an operand.
    fn operand(&mut self, out: &mut Vec<Node>, stop: Stop) -> bool {
        loop {
            self.trivia(out);
            if self.at_break(stop) {
                return false;
            }
            let at = self.at();
            let c = self.peek().unwrap();

            if c == '\'' || c == '"' {
                let in_tag = self.in_tag;
                let s = self.string(in_tag, StringStyle::Script);
                self.finish_into(out, s);
                return true;
            }
            if self.at_word("new") {
                self.constructor(out);
                return true;
            }
            // A prefix operator does not end the operand: keep looking.
            if c == '!' && !self.rest().starts_with("!=") {
                self.emit_len(out, 1, TokenKind::Operator(Operator::Not { word: false }));
                continue;
            }
            if self.at_word("not") {
                self.emit_len(out, 3, TokenKind::Operator(Operator::Not { word: true }));
                continue;
            }
            // Not the start of a longer operator: `+=` is never a sign and
            // an `=`, which would print apart.
            if let Some((end, kind)) = match_op(self.src, at, lexer::PREFIX_OPERATORS)
                .filter(|&(end, _)| lexer::operator_len(self.src, at) == Some(end - at))
            {
                self.emit(out, end, kind);
                continue;
            }
            if let Some(end) = lexer::identifier(self.src, at) {
                let word = &self.src[at..end];
                if let Some(kind) = lexer::special_name(word) {
                    self.emit(out, end, kind);
                    return true;
                }
                if let Some(kind) = self.cfml_scope(word) {
                    self.emit(out, end, kind);
                    return true;
                }
            }
            if let Some(end) = lexer::number(self.src, at) {
                self.emit(out, end, TokenKind::Literal(Literal::Number));
                return true;
            }
            if self.at_function_lookahead(at) {
                self.function_declaration_and_body(out);
                return true;
            }
            // A pattern assigned to or iterated (`[a, b] = x`, `({a} = x)`,
            // `for ([k, v] in x)`), before the literals it looks like.
            if matches!(c, '[' | '{') && self.pattern_ahead(at) {
                self.binding_pattern(out);
                return true;
            }
            if c == '{' {
                let s = self.struct_literal(false);
                self.finish_into(out, s);
                return true;
            }
            if c == '[' {
                self.array_or_struct(out);
                return true;
            }
            // The word `java` before a `{` (whitespace between allowed)
            // starts a `{ … }` of Java text (none in the test corpora).
            if self.at_word("java") {
                let after = skip_spaces(self.src, at + 4);
                if matches!(self.src[after..].chars().next(), Some('\n' | '{')) {
                    self.emit_len(out, 4, TokenKind::Keyword(Keyword::Java));
                    self.whitespace_only(out);
                    if self.peek() == Some('{') {
                        let block = self.java_block();
                        self.finish_into(out, block);
                    }
                    return true;
                }
            }
            // A `#…#` is an operand: `#a# + 1` is a binary, not `#a#` then
            // `+1`. A `##` escape is not.
            if c == '#' {
                if self.at_str("##") {
                    self.emit_len(out, 2, TokenKind::Literal(Literal::EscapeHash));
                    continue;
                }
                let t = self.template_expression();
                self.finish_into(out, t);
                return true;
            }
            if let Some(end) = lexer::identifier(self.src, at) {
                let word = &self.src[at..end];
                if lexer::in_list(lexer::RESERVED_WORDS, word) && !self.reserved_as_variable(end) {
                    return false;
                }
            }
            if c == '(' || lexer::identifier(self.src, at).is_some() {
                if self.at_arrow_function(at) {
                    let f = self.arrow_function();
                    let head = out.len();
                    self.finish_into(out, f);
                    if self.arrow_tail(out) {
                        fuse(out, head, ElementKind::Function { arrow: true });
                    }
                    return true;
                }
                if c == '(' {
                    let g = self.parenthesized();
                    self.finish_into(out, g);
                    return true;
                }
                self.literal_variable(out);
                return true;
            }
            return false;
        }
    }

    /// A scope word as its own identifier kind: `super`, `this`, `thread`
    /// before `[` or `.`, and the scope variables (`variables`, `arguments`,
    /// …).
    fn cfml_scope(&self, word: &str) -> Option<TokenKind> {
        if word.eq_ignore_ascii_case("super") {
            return Some(TokenKind::Ident(Ident::Super));
        }
        if word.eq_ignore_ascii_case("this") {
            return Some(TokenKind::Ident(Ident::This));
        }
        if word.eq_ignore_ascii_case("thread") {
            let after = self.at() + word.len();
            return self.src[after..]
                .starts_with(['[', '.'])
                .then_some(TokenKind::Ident(Ident::ScopeVar));
        }
        lexer::in_list(lexer::SCOPE_VARIABLES, word).then_some(TokenKind::Ident(Ident::ScopeVar))
    }

    /// What follows an operand: postfix operators, call arguments and member
    /// accesses extend it, and a ternary ends the run. `true` when a binary
    /// operator, `=` or `,` was taken and another operand follows.
    fn expression_end(&mut self, out: &mut Vec<Node>, stop: Stop) -> bool {
        loop {
            if stop.statement && self.at_line_ending() && !self.statement_continues() {
                return false;
            }
            self.trivia(out);
            if self.at_break(stop) {
                return false;
            }
            let at = self.at();
            // Postfix `--` / `++`.
            if self.at_str("--") || self.at_str("++") {
                self.emit_len(out, 2, TokenKind::Operator(Operator::Postfix));
                continue;
            }
            // Binary operators, the word forms (`eq`, `contains`, …) too.
            if let Some(end) = lexer::phrase_operator(self.src, at) {
                self.phrase(out, end);
                return true;
            }
            if let Some((end, kind)) = match_op(self.src, at, lexer::BINARY_OPERATORS) {
                self.emit(out, end, kind);
                return true;
            }
            if let Some((end, kind)) = match_op(self.src, at, lexer::WORD_COMPARISONS) {
                self.emit(out, end, kind);
                return true;
            }
            // An assignment `=` (not `==`, not `=>`).
            if self.peek() == Some('=')
                && !self.rest().starts_with(["==", "=>"][0])
                && !self.rest().starts_with("=>")
                && !self.rest().starts_with("==")
            {
                self.emit_len(out, 1, TokenKind::Operator(Operator::Assign));
                return true;
            }
            if self.peek() == Some(',') {
                self.emit_len(out, 1, TokenKind::Punct(Punct::Comma));
                return true;
            }
            // A ternary `?` (`ternary_ahead`).
            if self.peek() == Some('?') && self.ternary_ahead() {
                self.ternary(out);
                return false;
            }
            // Templates juxtaposed (`#a##b#`) stay two operands in one run.
            // (Inside a `#…#` the last part is its content, and the `#` is
            // its own closing one.)
            let after_template = out.iter().rev().find(|n| !n.is_trivia()).is_some_and(
                |n| matches!(n, Node::Element(e) if e.kind == ElementKind::TemplateExpression),
            );
            if after_template && self.peek() == Some('#') && !self.at_str("##") {
                let t = self.template_expression();
                self.finish_into(out, t);
                continue;
            }
            // Call arguments, then member accesses.
            if self.peek() == Some('(') {
                let call = self.call_arguments();
                self.finish_into(out, call);
                continue;
            }
            if self.property_access(out) {
                continue;
            }
            return false;
        }
    }

    /// A multi-word operator up to `end`: a
    /// [`Phrase`](ElementKind::Phrase) holding each word as a comparison
    /// operator token and the whitespace between the words as whitespace
    /// and newline tokens.
    fn phrase(&mut self, out: &mut Vec<Node>, end: usize) {
        let mut el = self.element(ElementKind::Phrase);
        while self.at() < end {
            match lexer::whitespace(self.src, self.at()) {
                Some(space) => self.emit_whitespace(&mut el.children, space),
                None => {
                    let word = lexer::identifier(self.src, self.at()).expect("a phrase's word");
                    let kind = TokenKind::Operator(Operator::Binary(Prec::Comparison));
                    self.emit(&mut el.children, word, kind);
                }
            }
        }
        self.finish_into(out, el);
    }

    /// Whether the `?` here is a ternary: not when it starts a `?.` accessor,
    /// though `?.5` is a ternary over a number.
    fn ternary_ahead(&self) -> bool {
        match self.rest()[1..].chars().next() {
            Some('.') => self.rest()[2..].starts_with(|c: char| c.is_ascii_digit()),
            _ => true,
        }
    }

    /// `? a : b`; a `:` right after the `?` (`a ?: b`) merges with it into one
    /// `Operator::Binary(Prec::Elvis)` token.
    fn ternary(&mut self, out: &mut Vec<Node>) {
        let q = self.at();
        self.emit_len(out, 1, TokenKind::Operator(Operator::TernaryQ));
        let before = out.len();
        self.expression(out, Stop::NO_COMMA);
        self.trivia(out);
        if self.peek() != Some(':') {
            return;
        }
        if out.len() == before && self.at() == q + 1 {
            // `?:`: one `Operator::Binary(Prec::Elvis)` token.
            let last = out.last_mut().unwrap();
            let Node::Token(t) = last else { unreachable!() };
            t.span.end += 1;
            t.kind = TokenKind::Operator(Operator::Binary(Prec::Elvis));
            self.pos += 1;
        } else {
            self.emit_len(out, 1, TokenKind::Operator(Operator::TernaryColon));
        }
        self.expression(out, Stop::NO_COMMA)
    }

    /// Whether nothing but spaces and comments is left on the current line.
    pub(super) fn at_line_ending(&self) -> bool {
        // The line's end is found only for a block comment: every operator
        // of a long line asks, and scanning to the end each time would make
        // a one-line source quadratic.
        let bytes = self.src.as_bytes();
        let mut at = self.at();
        loop {
            while at < bytes.len() && bytes[at] != b'\n' && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            if at >= bytes.len() || bytes[at] == b'\n' {
                return true;
            }
            if self.src[at..].starts_with("//") {
                return true;
            }
            if self.src[at..].starts_with("/*") {
                let line_end = lexer::line_end(self.src, at);
                match self.src[at + 2..line_end.max(at + 2)].find("*/") {
                    Some(i) => {
                        at += 2 + i + 2;
                        continue;
                    }
                    None => return true,
                }
            }
            return false;
        }
    }

    /// Whether the next significant text continues the statement past a
    /// line end (`lexer::continues_statement`).
    pub(super) fn statement_continues(&self) -> bool {
        let at = skip_trivia(self.src, self.at(), Comments::All);
        at < self.src.len() && lexer::continues_statement(self.src, at)
    }

    // -----------------------------------------------------------------------
    // Operands
    // -----------------------------------------------------------------------

    /// A name in operand position: a variable, a call, or the name a
    /// function is assigned to.
    fn literal_variable(&mut self, out: &mut Vec<Node>) {
        let at = self.at();
        let end = lexer::identifier(self.src, at).expect("identifier");
        let name = &self.src[at..end];
        let after = skip_trivia(self.src, end, Comments::Block);
        if function_assignment_ahead(self.src, end, self.depth) {
            // The name a function is assigned to (`f = function() {}`,
            // `f = () => …`) stays a variable, not a function name.
            self.emit(out, end, TokenKind::Ident(Ident::Variable));
            return;
        }
        if self.src[after..].starts_with('(') {
            return self.function_call(out, end, name);
        }
        // `arrayNew[…](…)`, in any case: the built-in with type brackets
        // between the name and the arguments.
        if name.eq_ignore_ascii_case("arraynew") && self.src[end..].starts_with('[') {
            self.emit(out, end, TokenKind::Ident(Ident::Builtin));
            let types = self.array_types();
            self.finish_into(out, types);
            self.trivia(out);
            if self.peek() == Some('(') {
                let call = self.call_arguments();
                self.finish_into(out, call);
            }
            return;
        }
        self.literal_variable_base(out, at, end);
    }

    /// A plain name: a variable, or a class name before `::`.
    fn literal_variable_base(&mut self, out: &mut Vec<Node>, at: usize, end: usize) {
        // A dot path before `::` (`a.b.C::m()`) is a class name.
        if let Some(path) = lexer::dot_path(self.src, at) {
            if self.src[skip_spaces(self.src, path)..].starts_with("::") {
                self.emit(out, path, TokenKind::Ident(Ident::ClassName));
                return;
            }
        }
        self.emit(out, end, TokenKind::Ident(Ident::Variable));
    }

    /// A plain call `name(…)`: a built-in function when the name is one
    /// (`isNull (x)` as much as `isNull(x)`), a user-defined one otherwise.
    /// Whitespace and comments between the name and `(` are kept before the
    /// arguments.
    fn function_call(&mut self, out: &mut Vec<Node>, end: usize, name: &str) {
        let ident = if lexer::is_support_function(name) {
            Ident::Builtin
        } else {
            Ident::Call
        };
        self.emit(out, end, TokenKind::Ident(ident));
        self.trivia(out);
        let call = self.call_arguments();
        self.finish_into(out, call);
    }

    /// A call's `( … )`: comma-separated arguments, each an expression with
    /// an optional name (`name = v`, `name: v`).
    fn call_arguments(&mut self) -> Element {
        self.delimited(
            ElementKind::Call,
            Delim::Paren,
            &[')', '}', ']', ';'],
            Some(Stop::NO_COMMA),
            |p, nodes| {
                p.named_argument(nodes);
                p.expression(nodes, Stop::NO_COMMA);
            },
        )
    }

    /// A named argument's name and separator: an identifier, then `=` (not
    /// `==`, not `=>`) or `:` (not `::`).
    fn named_argument(&mut self, out: &mut Vec<Node>) {
        let at = self.at();
        let Some(end) = lexer::identifier(self.src, at) else {
            return;
        };
        // The name starts a word; `$` is a word character, so `$name` is a
        // name too (`f($a = 1)` has a named argument, not an assignment).
        if !lexer::word_boundary_before(self.src, at) {
            return;
        }
        let sep = skip_spaces(self.src, end);
        let rest = &self.src[sep..];
        let kind = if rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>") {
            TokenKind::Operator(Operator::Assign)
        } else if rest.starts_with(':') && !rest.starts_with("::") {
            TokenKind::Punct(Punct::KeyValue)
        } else {
            return;
        };
        self.emit(out, end, TokenKind::Ident(Ident::ArgName));
        self.whitespace_only(out);
        self.emit_len(out, 1, kind);
    }

    /// `new` and what it constructs: a string, `component`, `java` or a
    /// dot-path class name, then the arguments; Lucee's inline component is
    /// `inline_component`.
    fn constructor(&mut self, out: &mut Vec<Node>) {
        let start = out.len();
        self.emit_len(out, 3, TokenKind::Keyword(Keyword::New));
        self.trivia(out);
        let component = self.at_word("component");
        if component && self.inline_component(out, start) {
            return;
        }
        if matches!(self.peek(), Some('\'' | '"')) {
            let s = self.string(false, StringStyle::Script);
            self.finish_into(out, s);
        } else if self.at_word("component") {
            self.emit_len(out, 9, TokenKind::Keyword(Keyword::Component));
        } else if self.at_word("java") {
            self.emit_len(out, 4, TokenKind::Storage(Storage::Type));
        } else if let Some(end) = lexer::dot_path(self.src, self.at()) {
            self.emit(out, end, TokenKind::Ident(Ident::ClassName));
        }
        self.trivia(out);
        if self.peek() == Some('(') {
            let call = self.call_arguments();
            self.finish_into(out, call);
        }
    }

    /// Lucee's inline component, `new component(args) attr=value … { body }`.
    /// It is an anonymous component declaration: a `New` element holding
    /// `new` and a `Class` of the `ClassDecl` (`component`, the arguments, a
    /// run of component attributes) and the `Block(Class)` — so it prints
    /// like one. At
    /// `component`, after `new` (from `start` in `out`); `false`, with nothing
    /// consumed, when no `{` follows the attributes.
    fn inline_component(&mut self, out: &mut Vec<Node>, start: usize) -> bool {
        let mark = self.pos;
        let mut decl = self.element(ElementKind::ClassDecl);
        self.emit_len(
            &mut decl.children,
            9,
            TokenKind::Keyword(Keyword::Component),
        );
        self.trivia(&mut decl.children);
        if self.peek() == Some('(') {
            let call = self.call_arguments();
            self.finish_into(&mut decl.children, call);
        }
        if !self.inline_component_ahead() {
            self.pos = mark;
            return false;
        }
        let in_tag = std::mem::replace(&mut self.in_tag, true);
        self.tag_attributes(&mut decl.children, AttrStyle::Component);
        self.in_tag = in_tag;
        let mut new = self.element(ElementKind::New);
        new.children = out.drain(start..).collect();
        let head = new.children.len();
        self.finish_into(&mut new.children, decl);
        if self.peek() == Some('{') {
            let block = self.block(BlockKind::Class);
            self.finish_into(&mut new.children, block);
            fuse(&mut new.children, head, ElementKind::Class);
        }
        self.finish_into(out, new);
        true
    }

    /// After `new component` (and its arguments): a run of `name = value`
    /// attributes, then `{`.
    fn inline_component_ahead(&self) -> bool {
        let src = self.src;
        let mut at = skip_trivia(src, self.at(), Comments::All);
        loop {
            if src[at..].starts_with('{') {
                return true;
            }
            let Some(end) = lexer::attribute_name(src, at) else {
                return false;
            };
            at = skip_spaces(src, end);
            if !src[at..].starts_with('=') {
                return false;
            }
            at = skip_spaces(src, at + 1);
            match src[at..].chars().next() {
                // The value's string by the parser's rule (`''` / `""` and
                // `#…#`, `scan::string_end`), so the lookahead and the parser
                // agree on where `a="t#" {}`'s value ends.
                Some('\'' | '"') => {
                    let end = scan::string_end(src, at, self.depth);
                    let closed = end > at + 1 && src.as_bytes()[end - 1] == src.as_bytes()[at];
                    if end >= src.len() && !closed {
                        return false;
                    }
                    at = end;
                }
                Some(_) => {
                    let len = src[at..]
                        .find(|c: char| c.is_whitespace() || "<>/{;".contains(c))
                        .unwrap_or(src.len() - at);
                    if len == 0 {
                        return false;
                    }
                    at += len;
                }
                None => return false,
            }
            at = skip_trivia(src, at, Comments::All);
        }
    }

    /// A member access: `[…]`, or `.`, `?.` or `::` and a name (a method
    /// call when `(` follows).
    pub(super) fn property_access(&mut self, out: &mut Vec<Node>) -> bool {
        if self.peek() == Some('[') {
            let b = self.brackets();
            self.finish_into(out, b);
            return true;
        }
        let (len, kind) = if self.at_str("?.") {
            (2, TokenKind::Punct(Punct::SafeAccessor))
        } else if self.at_str("::") {
            (2, TokenKind::Punct(Punct::StaticAccessor))
        } else if self.peek() == Some('.') {
            (1, TokenKind::Punct(Punct::Accessor))
        } else {
            return false;
        };
        self.emit_len(out, len, kind);
        self.trivia(out);
        let at = self.at();
        let Some(end) = lexer::identifier(self.src, at) else {
            // A numeric member (`a.b.1234`) is a property name.
            let mut digits = at;
            while self
                .src
                .as_bytes()
                .get(digits)
                .is_some_and(u8::is_ascii_digit)
            {
                digits += 1;
            }
            if digits > at {
                self.emit(out, digits, TokenKind::Ident(Ident::PropertyName));
            }
            return true;
        };
        if function_assignment_ahead(self.src, end, self.depth) {
            self.emit(out, end, TokenKind::Ident(Ident::Variable));
            return true;
        }
        if self.src[skip_spaces(self.src, end)..].starts_with('(') {
            self.emit(out, end, TokenKind::Ident(Ident::Call));
            self.whitespace_only(out);
            let call = self.call_arguments();
            self.finish_into(out, call);
            return true;
        }
        self.emit(out, end, TokenKind::Ident(Ident::PropertyName));
        true
    }

    /// `[ … ]` after an operand: an index or a typed-array bracket.
    fn brackets(&mut self) -> Element {
        let mut el = self.element(ElementKind::Brackets);
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Bracket))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        loop {
            self.trivia(&mut el.children);
            if self.eof() || matches!(self.peek(), Some(']' | '}' | ')' | ';')) {
                break;
            }
            let before = self.pos;
            self.expression(&mut el.children, Stop::EXPR);
            if self.pos == before {
                self.skip_one(&mut el.children);
                self.expression_tail(&mut el.children, Stop::EXPR);
            }
        }
        self.in_tag = in_tag;
        self.trivia(&mut el.children);
        if self.peek() == Some(']') {
            el.close = Some(self.take(
                self.at() + 1,
                TokenKind::Punct(Punct::Close(Delim::Bracket)),
            ));
        }
        el
    }

    /// `( … )`, an operand or a statement's header: expressions up to `)`,
    /// or up to a `}`, `]` or `;` that leaves it unclosed.
    pub(super) fn parenthesized(&mut self) -> Element {
        let mut el = self.element(ElementKind::Group);
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Paren))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        loop {
            self.trivia(&mut el.children);
            if self.eof() || matches!(self.peek(), Some(')' | '}' | ']' | ';')) {
                break;
            }
            let before = self.pos;
            self.expression(&mut el.children, Stop::EXPR);
            if self.pos == before {
                self.skip_one(&mut el.children);
                self.expression_tail(&mut el.children, Stop::EXPR);
            }
        }
        self.in_tag = in_tag;
        self.trivia(&mut el.children);
        if self.peek() == Some(')') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::Paren))));
        }
        el
    }

    // -----------------------------------------------------------------------
    // Strings and template expressions
    // -----------------------------------------------------------------------

    /// A quoted string, read by `style` ([`StringStyle`]).
    pub(super) fn string(&mut self, in_tag: bool, style: StringStyle) -> Element {
        let quote = self.peek().expect("quote");
        let mut el = self.element(ElementKind::String {
            quote: if quote == '\'' {
                Quote::Single
            } else {
                Quote::Double
            },
            in_tag,
        });
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::String))));
        if style == StringStyle::Script {
            while let Some(end) = lexer::whitespace(self.src, self.at()) {
                self.emit(
                    &mut el.children,
                    end,
                    TokenKind::Literal(Literal::StringText),
                );
            }
        }
        let escape: String = [quote, quote].iter().collect();
        let interpolates = matches!(style, StringStyle::Script | StringStyle::Attribute);
        loop {
            if self.eof() {
                return el;
            }
            if self.at_str(&escape) {
                self.emit_len(
                    &mut el.children,
                    2,
                    TokenKind::Literal(Literal::EscapeQuote),
                );
                continue;
            }
            if self.peek() == Some(quote) {
                el.close =
                    Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::String))));
                return el;
            }
            if interpolates && self.at_str("##") {
                self.emit_len(&mut el.children, 2, TokenKind::Literal(Literal::EscapeHash));
                continue;
            }
            if interpolates && self.peek() == Some('#') {
                let t = self.template_expression();
                self.finish_into(&mut el.children, t);
                continue;
            }
            // One token per line, up to the next quote (or `#`, where it
            // interpolates).
            let line = lexer::line_end(self.src, self.at());
            let b = self.src.as_bytes();
            let mut i = self.at();
            while i < line {
                if b[i] == quote as u8 || (interpolates && b[i] == b'#') {
                    break;
                }
                i += 1;
            }
            let i = i.max(self.at() + 1);
            self.emit(&mut el.children, i, TokenKind::Literal(Literal::StringText));
        }
    }

    /// A `#…#` template expression.
    pub(super) fn template_expression(&mut self) -> Element {
        let mut el = self.element(ElementKind::TemplateExpression);
        el.open = Some(self.take(
            self.at() + 1,
            TokenKind::Punct(Punct::Open(Delim::Template)),
        ));
        self.template_contents(&mut el.children, true);
        if self.peek() == Some('#') {
            el.close = Some(self.take(
                self.at() + 1,
                TokenKind::Punct(Punct::Close(Delim::Template)),
            ));
        }
        el
    }

    /// The expressions of a `#…#`, up to a `#` (`hash_ends`) or the end. Text
    /// no rule reads at this level is one `Other` token, whitespace included,
    /// not split into runs as elsewhere. A tag's script (`<cfset>`, `<cfif>`,
    /// …, `hash_ends` false) loops the same way, a `#` there starting a
    /// template expression of its own.
    pub(super) fn template_contents(&mut self, out: &mut Vec<Node>, hash_ends: bool) {
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        loop {
            self.trivia(out);
            if self.eof() || (hash_ends && self.peek() == Some('#')) {
                break;
            }
            let before = self.pos;
            self.expression(out, Stop::EXPR);
            if self.pos == before {
                // Only at the contents' own level is unread text one run, and
                // there `;`, `}`, `)` and `]` are unread too (only `#` ends
                // the contents); inside a call's arguments, a group or a
                // struct it splits as anywhere else.
                let in_template = std::mem::replace(&mut self.in_template, true);
                self.skip_one(out);
                self.in_template = in_template;
                self.expression_tail(out, Stop::EXPR);
            }
        }
        self.in_tag = in_tag;
    }

    // -----------------------------------------------------------------------
    // Struct and array literals
    // -----------------------------------------------------------------------

    /// `{ … }`, or `[ … ]` for an ordered struct: comma-separated members.
    fn struct_literal(&mut self, ordered: bool) -> Element {
        let (open, close) = if ordered {
            (Delim::Bracket, ']')
        } else {
            (Delim::Brace, '}')
        };
        self.delimited(
            ElementKind::Struct { ordered },
            open,
            &[close],
            Some(Stop::NO_COMMA),
            Self::struct_member,
        )
    }

    /// One step of a struct member: a spread, a key, or a separator and its
    /// value; the literal's loop calls it again after a key.
    fn struct_member(&mut self, out: &mut Vec<Node>) {
        if self.at_str("...") {
            self.emit_len(out, 3, TokenKind::Operator(Operator::Spread));
            return self.expression(out, Stop::NO_COMMA);
        }
        // The key of a function-valued member (`f: function() {}`,
        // `f = () => …`) is a struct key like any other.
        if let Some((end, is_string)) = property_name(self.src, self.at()) {
            let sep = skip_spaces(self.src, end);
            if self.src[sep..].starts_with([':', '='])
                && either_function_ahead(self.src, skip_spaces(self.src, sep + 1), self.depth)
            {
                if is_string {
                    let s = self.string(false, StringStyle::Script);
                    self.finish_into(out, s);
                } else {
                    self.emit(out, end, TokenKind::Ident(Ident::StructKey));
                }
                return;
            }
            // The key alone: what follows it is the enclosing loop's business,
            // so `formstruct.name = 1` keeps the key and parses `.name = 1` as
            // an expression run after it.
            let rest_of_line = self.src[end..].trim_start_matches([' ', '\t', '\r', '\x0c']);
            let shorthand = lexer::identifier(self.src, self.at()) == Some(end)
                && (rest_of_line.is_empty()
                    || rest_of_line.starts_with(['}', ',', '\n'])
                    || rest_of_line.starts_with("//")
                    || rest_of_line.starts_with("/*"));
            if shorthand {
                self.emit(out, end, TokenKind::Ident(Ident::Variable));
                return;
            }
            if is_string {
                let s = self.string(false, StringStyle::Script);
                self.finish_into(out, s);
                return;
            }
            // An unquoted key is identifier characters only, not a dot path:
            // `a.b: 1` keys on `a` alone.
            let mut key = self.at();
            while self.src[key..]
                .chars()
                .next()
                .is_some_and(lexer::is_ident_part)
            {
                key += self.src[key..].chars().next().unwrap().len_utf8();
            }
            if key > self.at() {
                self.emit(out, key, TokenKind::Ident(Ident::StructKey));
                return;
            }
        }
        if self.peek() == Some(':') || self.peek() == Some('=') {
            self.emit_len(out, 1, TokenKind::Punct(Punct::KeyValue));
            return self.expression(out, Stop::NO_COMMA);
        }
        self.expression(out, Stop::NO_COMMA)
    }

    /// A `[` operand: a typed array, an ordered struct or an array literal.
    fn array_or_struct(&mut self, out: &mut Vec<Node>) {
        // A typed array (`["string"][1, 2]`): only a bracket holding one
        // quoted type name, then `[`, is typed. Every other literal keeps a
        // following `[` as its index, so `[1, 2][1]` is a literal indexed at
        // once.
        if self.typed_array_ahead() {
            let mut typed = self.element(ElementKind::TypedArray);
            let types = self.array_types();
            self.finish_into(&mut typed.children, types);
            self.trivia(&mut typed.children);
            if self.peek() == Some('[') {
                let a = self.array_literal();
                self.finish_into(&mut typed.children, a);
            }
            self.finish_into(out, typed);
            return;
        }
        if self.array_is_ordered_struct() {
            let s = self.struct_literal(true);
            self.finish_into(out, s);
            return;
        }
        let a = self.array_literal();
        self.finish_into(out, a);
    }

    /// A typed array's type bracket: `[`, a quoted type name, then
    /// expressions, then `]`.
    fn array_types(&mut self) -> Element {
        let mut el = self.element(ElementKind::Brackets);
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Bracket))));
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        self.trivia(&mut el.children);
        // Only a quoted type is read as one; anything else in the bracket
        // is read as expressions.
        if matches!(self.peek(), Some('\'' | '"')) {
            let quote = self.peek().unwrap();
            let mut s = self.element(ElementKind::String {
                quote: if quote == '\'' {
                    Quote::Single
                } else {
                    Quote::Double
                },
                in_tag: false,
            });
            s.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::String))));
            // The type name (a dot path) is one token, the rest of the
            // string another.
            if let Some(end) = lexer::dot_path(self.src, self.at()) {
                self.emit(
                    &mut s.children,
                    end,
                    TokenKind::Literal(Literal::StringText),
                );
            }
            let rest = self
                .rest()
                .find(quote)
                .map_or(self.src.len(), |i| self.at() + i);
            if rest > self.at() {
                self.emit(
                    &mut s.children,
                    rest,
                    TokenKind::Literal(Literal::StringText),
                );
            }
            if self.peek() == Some(quote) {
                s.close =
                    Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::String))));
            }
            self.finish_into(&mut el.children, s);
        }
        loop {
            self.trivia(&mut el.children);
            if self.eof() || matches!(self.peek(), Some(']' | '}' | ')' | ';')) {
                break;
            }
            let before = self.pos;
            self.expression(&mut el.children, Stop::EXPR);
            if self.pos == before {
                self.skip_one(&mut el.children);
                self.expression_tail(&mut el.children, Stop::EXPR);
            }
        }
        self.in_tag = in_tag;
        if self.peek() == Some(']') {
            el.close = Some(self.take(
                self.at() + 1,
                TokenKind::Punct(Punct::Close(Delim::Bracket)),
            ));
        }
        el
    }

    /// `[` `'type'` `]` `[`: the type bracket of a typed array literal.
    /// The name is a dot path, the string's whole text.
    fn typed_array_ahead(&self) -> bool {
        let at = skip_trivia(self.src, self.at() + 1, Comments::All);
        let Some(quote) = self.src[at..]
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"'))
        else {
            return false;
        };
        let name = at + 1;
        let Some(end) = lexer::dot_path(self.src, name) else {
            return false;
        };
        if !self.src[end..].starts_with(quote) {
            return false;
        }
        let close = skip_trivia(self.src, end + 1, Comments::All);
        self.src[close..].starts_with(']')
            && self.src[skip_spaces(self.src, close + 1)..].starts_with('[')
    }

    /// Whether a `[` opens an ordered struct: `[:`, or a first key followed by
    /// `=` (not `==`) or `:` (not `::`).
    fn array_is_ordered_struct(&self) -> bool {
        let at = skip_trivia(self.src, self.at() + 1, Comments::All);
        if self.src[at..].starts_with(':') {
            return true;
        }
        let Some((end, _)) = property_name(self.src, at) else {
            return false;
        };
        let sep = skip_spaces(self.src, end);
        let rest = &self.src[sep..];
        (rest.starts_with('=') && !rest.starts_with("=="))
            || (rest.starts_with(':') && !rest.starts_with("::"))
    }

    /// An array literal: comma-separated expressions.
    fn array_literal(&mut self) -> Element {
        self.delimited(
            ElementKind::Array,
            Delim::Bracket,
            &[']', '}', ')', ';'],
            Some(Stop::NO_COMMA),
            |p, nodes| p.expression(nodes, Stop::NO_COMMA),
        )
    }

    // -----------------------------------------------------------------------
    // Functions
    // -----------------------------------------------------------------------

    /// Whether a function declaration or a function expression starts here
    /// (`function_declaration_ahead`).
    pub(super) fn at_function_lookahead(&self, at: usize) -> bool {
        function_declaration_ahead(self.src, at)
    }

    /// A function declaration as a statement.
    pub(super) fn function_declaration_statement(&mut self, el: &mut Element) -> bool {
        if !self.at_function_lookahead(self.at()) {
            return false;
        }
        if !self.function_declaration_and_body(&mut el.children) {
            // A body-less `function f();` ends with a mandatory `;`.
            self.trivia(&mut el.children);
            self.expect_semicolon(el);
        }
        true
    }

    /// The header, then its body (or the Java pair), as one `Function`
    /// element; `false`, the header alone, when no body follows.
    fn function_declaration_and_body(&mut self, out: &mut Vec<Node>) -> bool {
        let decl = self.function_declaration();
        let java = std::mem::take(&mut self.java_body);
        let head = out.len();
        self.finish_into(out, decl);
        let body = if java {
            self.trivia(out);
            if self.peek() == Some('{') {
                let block = self.java_block();
                self.finish_into(out, block);
                true
            } else {
                false
            }
        } else {
            self.function_body(out)
        };
        if body {
            fuse(out, head, ElementKind::Function { arrow: false });
        }
        body
    }

    /// The header: modifiers, return type, `function`, the name, the
    /// parameters and the metadata attributes.
    fn function_declaration(&mut self) -> Element {
        let mut el = self.element(ElementKind::FunctionDecl);
        let out = &mut el.children;
        let in_tag = std::mem::replace(&mut self.in_tag, true);
        // The words before `function`: the access and storage modifiers,
        // each at most once and in any order, and at most one return type,
        // before or after them. That is Lucee's reading: `static private
        // function` is private and static, `string private function` is
        // private, and a repeated modifier (`private static static
        // function`) is the return type. No return type when the next word
        // is `function` and *another* `function` does not follow (`package
        // final function function private()`).
        let mut seen: Vec<&str> = Vec::new();
        let mut typed = false;
        while let Some(end) = lexer::identifier(self.src, self.at()) {
            let slot = modifier_slot(&self.src[self.at()..end]);
            if let Some(slot) = slot.filter(|s| !seen.contains(s)) {
                seen.push(slot);
                self.emit(out, end, TokenKind::Storage(Storage::Modifier));
            } else if !typed && !self.return_type_absent() {
                typed = true;
                self.storage_type(out);
            } else {
                break;
            }
            self.trivia(out);
        }
        if self.at_word("function") {
            self.emit_len(out, 8, TokenKind::Keyword(Keyword::Function));
            self.trivia(out);
        }
        if let Some(end) = lexer::identifier(self.src, self.at()) {
            self.emit(out, end, TokenKind::Ident(Ident::FunctionName));
        }
        self.trivia(out);
        if self.peek() == Some('(') {
            let params = self.function_parameters();
            self.finish_into(out, params);
        }
        // The metadata attributes, up to `;` or `{`.
        self.java_body = self.tag_attributes(out, AttrStyle::Declaration);
        self.in_tag = in_tag;
        el
    }

    /// Whether the next word is `function` and the one after it is not: then
    /// no return type is left to read.
    fn return_type_absent(&self) -> bool {
        let at = skip_spaces(self.src, self.at());
        if !lexer::keyword_at(self.src, at, "function") {
            return false;
        }
        let next = skip_spaces(self.src, at + "function".len());
        !lexer::keyword_at(self.src, next, "function")
    }

    /// A return type: a dot path, with `[]` for an array type.
    fn storage_type(&mut self, out: &mut Vec<Node>) {
        let at = self.at();
        let Some(end) = lexer::dot_path(self.src, at) else {
            return;
        };
        // A `function` return type is a `function` keyword token.
        if self.src[at..end].eq_ignore_ascii_case("function") {
            self.emit(out, end, TokenKind::Keyword(Keyword::Function));
            return;
        }
        // `type[]` keeps its brackets as an element.
        if self.src[end..].starts_with("[]") {
            self.emit(out, end, TokenKind::Storage(Storage::Type));
            let mut br = self.element(ElementKind::Brackets);
            br.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Bracket))));
            br.close = Some(self.take(
                self.at() + 1,
                TokenKind::Punct(Punct::Close(Delim::Bracket)),
            ));
            self.finish_into(out, br);
            return;
        }
        self.emit(out, end, TokenKind::Storage(Storage::Type));
    }

    /// A declaration's parameter list: `( … )` of comma-separated
    /// parameters.
    fn function_parameters(&mut self) -> Element {
        self.delimited(
            ElementKind::Parameters,
            Delim::Paren,
            &[')', '}', ';'],
            None,
            Self::function_parameter,
        )
    }

    /// One parameter: `required`? `type`? `name` `= default`? attributes; or
    /// a destructuring pattern.
    fn function_parameter(&mut self, out: &mut Vec<Node>) {
        if matches!(self.peek(), Some('[' | '{')) {
            self.binding_pattern(out);
            return self.parameter_tail(out);
        }
        if self.at_word("required") {
            self.emit_len(out, 8, TokenKind::Keyword(Keyword::Required));
            return;
        }
        if self.at_str("...") {
            self.emit_len(out, 3, TokenKind::Operator(Operator::Spread));
            if let Some(end) = lexer::identifier(self.src, self.at()) {
                self.emit(out, end, TokenKind::Ident(Ident::Parameter));
            }
            return;
        }
        let at = self.at();
        let Some(first) = lexer::dot_path(self.src, at) else {
            // No rule matches: skip to where one can, an operator at a time
            // (`?.` is not `?` then `.`, where the run would stop).
            let mut end = at;
            while end < self.src.len() {
                end += lexer::operator_len(self.src, end)
                    .unwrap_or_else(|| self.src[end..].chars().next().unwrap().len_utf8());
                if self.src[end..].starts_with([')', ',', '.'])
                    || lexer::dot_path(self.src, end).is_some()
                {
                    break;
                }
            }
            self.emit_unmatched(out, end);
            return;
        };
        let after = skip_spaces(self.src, first);
        if after > first {
            if let Some(name_end) = lexer::identifier(self.src, after) {
                self.emit(out, first, TokenKind::Storage(Storage::Type));
                self.whitespace_only(out);
                self.emit(out, name_end, TokenKind::Ident(Ident::Parameter));
                return self.parameter_tail(out);
            }
        }
        let Some(name_end) = lexer::identifier(self.src, at) else {
            self.emit(out, first, TokenKind::Storage(Storage::Type));
            return;
        };
        self.emit(out, name_end, TokenKind::Ident(Ident::Parameter));
        self.parameter_tail(out)
    }

    /// A parameter's `= default`, then its attributes.
    fn parameter_tail(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.peek() == Some('=') && !self.at_str("==") && !self.at_str("=>") {
            self.emit_len(out, 1, TokenKind::Operator(Operator::Assign));
            self.expression(out, Stop::NO_COMMA);
        }
        self.tag_attributes(out, AttrStyle::Parameter);
    }

    /// The body block after a header, when `{` follows; `true` when one was
    /// taken.
    fn function_body(&mut self, out: &mut Vec<Node>) -> bool {
        let mark = self.pos;
        let mut trivia = Vec::new();
        self.trivia(&mut trivia);
        if self.peek() == Some('{') {
            out.append(&mut trivia);
            let block = self.block(BlockKind::Function);
            self.finish_into(out, block);
            return true;
        }
        self.pos = mark;
        false
    }

    /// An arrow function's parameters: a parenthesised list or one name.
    fn arrow_function(&mut self) -> Element {
        let mut el = self.element(ElementKind::ArrowFunction);
        if self.peek() == Some('(') {
            let params = self.function_parameters();
            self.finish_into(&mut el.children, params);
        } else {
            let end = lexer::identifier(self.src, self.at()).expect("arrow parameter");
            let mut params = self.element(ElementKind::Parameters);
            let mut item = Item::default();
            self.emit(&mut item.children, end, TokenKind::Ident(Ident::Parameter));
            params.items.push(item);
            self.finish_into(&mut el.children, params);
        }
        el
    }

    /// `=>` and the arrow's body (a block or one expression), emitted after
    /// the parameters; `true` when the `=>` was there.
    fn arrow_tail(&mut self, out: &mut Vec<Node>) -> bool {
        self.trivia(out);
        if !self.at_str("=>") {
            return false;
        }
        self.emit_len(out, 2, TokenKind::Keyword(Keyword::Arrow));
        self.trivia(out);
        if self.peek() == Some('{') {
            let block = self.block(BlockKind::Function);
            self.finish_into(out, block);
            return true;
        }
        let mut body = self.element(ElementKind::Block(BlockKind::Function));
        self.expression(&mut body.children, Stop::NO_COMMA);
        self.finish_into(out, body);
        true
    }

    /// The arrow-function branch point, decided by lookahead.
    fn at_arrow_function(&self, at: usize) -> bool {
        let after = match self.src[at..].chars().next() {
            Some('(') => match scan::balanced_code(self.src, at, b'(', b')', self.depth) {
                Some(end) => end,
                None => return false,
            },
            _ => match lexer::identifier(self.src, at) {
                Some(end) => end,
                None => return false,
            },
        };
        self.src[skip_trivia(self.src, after, Comments::All)..].starts_with("=>")
    }

    // -----------------------------------------------------------------------
    // Declarations
    // -----------------------------------------------------------------------

    /// A `var` declaration, or a `static` / `final` one.
    pub(super) fn variable_declaration(&mut self, el: &mut Element) -> bool {
        if self.at_word("var") {
            self.emit_len(&mut el.children, 3, TokenKind::Keyword(Keyword::Var));
            loop {
                self.variable_binding(&mut el.children);
                self.trivia(&mut el.children);
                if self.peek() == Some(',') {
                    self.emit_len(&mut el.children, 1, TokenKind::Punct(Punct::Comma));
                    continue;
                }
                break;
            }
            self.expect_semicolon(el);
            return true;
        }
        // `static` / `final`: the modifiers, then a function declaration, or —
        // on the same line — a declaration or an assignment, all one
        // statement.
        let mut matched = false;
        loop {
            let mut found = None;
            for word in ["static", "final"] {
                if self.at_word(word) {
                    // Not before `.` or `?.`: `static.x` is a scope access.
                    let after = self.at() + word.len();
                    if !self.src[after..].starts_with('.') && !self.src[after..].starts_with("?.") {
                        found = Some(word.len());
                    }
                }
            }
            let Some(len) = found else { break };
            self.emit_len(&mut el.children, len, TokenKind::Storage(Storage::Modifier));
            matched = true;
            self.trivia(&mut el.children);
            if self.at_function_lookahead(self.at()) {
                if !self.function_declaration_and_body(&mut el.children) {
                    self.trivia(&mut el.children);
                    self.expect_semicolon(el);
                }
                return true;
            }
        }
        if matched {
            self.modifier_target(el);
            return true;
        }
        false
    }

    /// What follows `static` / `final` on their line when it is a `var`
    /// declaration or an assignment: parsed as a statement of its own and
    /// taken into `el`, children and terminator. Anything else (a newline or
    /// a comment first, `static foo();`) is left to the statement list.
    fn modifier_target(&mut self, el: &mut Element) {
        let spaces_only = el
            .children
            .iter()
            .rev()
            .take_while(|n| n.is_trivia())
            .all(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Whitespace));
        if !spaces_only || self.eof() {
            return;
        }
        let mark = self.pos;
        let mut next = self.statement();
        // A statement the parse gave up in: taken in, its recovery is the
        // enclosing statement's.
        let mut reason = None;
        if let ElementKind::Recovered(r) = next.kind {
            reason = Some(r);
            let Some(Node::Element(inner)) = next.children.pop() else {
                unreachable!("a recovered statement holds its statement");
            };
            next = *inner;
        }
        let first = next.children.iter().find(|n| !n.is_trivia());
        let declaration = matches!(first, Some(Node::Token(t))
            if t.kind == TokenKind::Keyword(Keyword::Var));
        let assignment = next.children.iter().any(
            |n| matches!(n, Node::Token(t) if t.kind == TokenKind::Operator(Operator::Assign)),
        );
        if declaration || assignment {
            el.children.extend(next.children);
            el.close = next.close;
            if let Some(r) = reason {
                self.recover(r);
            }
        } else {
            self.pos = mark;
        }
    }

    /// One `var` binding: a name (or a destructuring pattern), its member
    /// accesses and an initializer.
    pub(super) fn variable_binding(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if matches!(self.peek(), Some('[' | '{')) {
            self.binding_pattern(out);
            return self.initializer(out);
        }
        let at = self.at();
        let Some(end) = self.binding_name(at) else {
            return;
        };
        self.literal_variable_base(out, at, end);
        // Member accesses after the name, trivia and newlines allowed
        // between: `var a` ⏎ `.b = 1` is still one declaration.
        loop {
            let mark = self.pos;
            let kept = out.len();
            self.trivia(out);
            if !self.property_access(out) {
                self.pos = mark;
                out.truncate(kept);
                break;
            }
        }
        self.initializer(out)
    }

    /// `= value`, the value an expression run that stops at a `,`.
    fn initializer(&mut self, out: &mut Vec<Node>) {
        self.trivia(out);
        if self.peek() == Some('=') && !self.at_str("==") && !self.at_str("=>") {
            self.emit_len(out, 1, TokenKind::Operator(Operator::Assign));
            self.expression(out, Stop::NO_COMMA);
        }
    }

    /// A destructuring pattern at the `[` or `{` here (`[a, , c]`,
    /// `{a, b: {c}, d = 1, ...r}`), as a `Pattern` element finished into
    /// `out`. A nested pattern counts against the depth bound.
    pub(super) fn binding_pattern(&mut self, out: &mut Vec<Node>) {
        if self.too_deep(out) {
            return;
        }
        self.depth += 1;
        let array = self.peek() == Some('[');
        let (open, closers): (_, &[char]) = if array {
            (Delim::Bracket, &[']', '}', ')', ';'])
        } else {
            (Delim::Brace, &['}', ']', ')', ';'])
        };
        let pattern = self.delimited(
            ElementKind::Pattern { array },
            open,
            closers,
            None,
            |p, nodes| p.pattern_item(nodes, array),
        );
        self.finish_into(out, pattern);
        self.depth -= 1;
    }

    /// One step of a pattern's item: `...` and its target, or a struct
    /// pattern's `key :` and its target, or the target alone; then `=` and a
    /// default. A target is a nested pattern or a variable name. A skipped
    /// element (`[a, , c]`) is an item with nothing in it. Never a
    /// key-value: the rename's `:` is read here, and the default's `=` stays
    /// an assignment's.
    fn pattern_item(&mut self, out: &mut Vec<Node>, array: bool) {
        if self.at_str("...") {
            self.emit_len(out, 3, TokenKind::Operator(Operator::Spread));
            self.trivia(out);
        } else if !array {
            if let Some(end) = lexer::identifier(self.src, self.at()) {
                if self.src[skip_spaces(self.src, end)..].starts_with(':') {
                    self.emit(out, end, TokenKind::Ident(Ident::StructKey));
                    self.whitespace_only(out);
                    self.emit_len(out, 1, TokenKind::Punct(Punct::KeyValue));
                    self.trivia(out);
                }
            }
        }
        if matches!(self.peek(), Some('[' | '{')) {
            self.binding_pattern(out);
        } else if let Some(end) = self.binding_name(self.at()) {
            let at = self.at();
            self.literal_variable_base(out, at, end);
        } else {
            return;
        }
        self.trivia(out);
        if self.peek() == Some('=') && !self.at_str("==") && !self.at_str("=>") {
            self.emit_len(out, 1, TokenKind::Operator(Operator::Assign));
            self.expression(out, Stop::NO_COMMA);
        }
    }

    /// Whether the `[` or `{` at `at` opens a destructuring pattern rather
    /// than a literal or a block: after its balanced closer
    /// ([`scan::pattern_end`]) come `=` (not `==` or `=>`) or the word `in`.
    /// So `[a, b] = x`, `({a} = x)` and `for ([k, v] in x)` are patterns, and
    /// `[1, 2].each(f)`, `[a][1] = x` and `[1, 2] == x` are not.
    pub(super) fn pattern_ahead(&self, at: usize) -> bool {
        let Some(end) = scan::pattern_end(self.src, at, self.depth) else {
            return false;
        };
        let next = skip_trivia(self.src, end, Comments::All);
        let rest = &self.src[next..];
        (rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>"))
            || lexer::keyword_at(self.src, next, "in")
    }

    // -----------------------------------------------------------------------
    // Attributes
    // -----------------------------------------------------------------------

    /// An attribute list; returns `true` when it holds `type="java"` (a
    /// declaration's body is then opaque Java).
    pub(super) fn tag_attributes(&mut self, out: &mut Vec<Node>, style: AttrStyle) -> bool {
        let mut java = false;
        loop {
            let (mark, kept) = (self.pos, out.len());
            self.trivia(out);
            if self.eof() {
                return java;
            }
            // A `}` ends the list and is the enclosing block's; so does a
            // statement on a later line (a body-less declaration in an
            // interface). The trivia before either goes back to the
            // statement list.
            let newline = self.src[mark as usize..self.at()].contains('\n');
            if self.peek() == Some('}') || (newline && self.at_statement_start()) {
                out.truncate(kept);
                self.pos = mark;
                return java;
            }
            match self.peek().unwrap() {
                '{' => return java,
                ';' if style != AttrStyle::Component => return java,
                ',' | ')' if style == AttrStyle::Parameter => return java,
                _ => {}
            }
            // A component's `extends` and a property's `name` read their
            // values specially; every other attribute is generic.
            if style == AttrStyle::Component && self.at_word("extends") {
                self.emit_len(out, 7, TokenKind::Ident(Ident::AttributeName));
                self.attribute_value(out, style, Value::InheritedClass);
                continue;
            }
            let Some(end) = lexer::attribute_name(self.src, self.at()) else {
                // Any other character is invalid here, one at a time.
                let c = self.peek().unwrap();
                self.emit_invalid(out, c.len_utf8(), RecoveryReason::Unmatched);
                continue;
            };
            if !lexer::word_boundary_before(self.src, self.at()) {
                let c = self.peek().unwrap();
                self.emit_invalid(out, c.len_utf8(), RecoveryReason::Unmatched);
                continue;
            }
            let name = self.src[self.at()..end].to_ascii_lowercase();
            self.emit(out, end, TokenKind::Ident(Ident::AttributeName));
            let value = if style == AttrStyle::PropertyTag && name == "name" {
                Value::PropertyName
            } else {
                Value::Plain
            };
            // `type="java"`: one header, a Java body.
            let is_java = self.attribute_value(out, style, value);
            java |= style == AttrStyle::Declaration && name == "type" && is_java;
        }
    }

    /// A word that starts a statement rather than an attribute: any word
    /// followed by `(` (a call, `name(`), or `function`, an access or storage
    /// modifier or a reserved word followed by another word on its line
    /// (`public void function g()`; a bare `abstract` alone on its line is
    /// a valueless attribute).
    fn at_statement_start(&self) -> bool {
        let Some(end) = lexer::identifier(self.src, self.at()) else {
            return false;
        };
        let word = &self.src[self.at()..end];
        let next =
            end + self.src[end..].len() - self.src[end..].trim_start_matches([' ', '\t']).len();
        if self.src[next..].starts_with('(') {
            return true;
        }
        let lists = [
            lexer::ACCESS_MODIFIERS,
            lexer::STORAGE_MODIFIERS,
            lexer::RESERVED_WORDS,
        ];
        lists.iter().any(|l| lexer::in_list(l, word)) && lexer::identifier(self.src, next).is_some()
    }

    /// The opaque Java body: balanced braces, one coalesced `Text` token,
    /// CFScript comments still interrupting it. Braces inside a Java string,
    /// char or text-block literal do not count ([`scan::java_literal_end`]:
    /// `String s = "}";` does not close the body).
    fn java_block(&mut self) -> Element {
        let mut el = self.element(ElementKind::Block(BlockKind::Java));
        el.open = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Open(Delim::Brace))));
        let mut depth = 1usize;
        let mut run: Option<(usize, usize)> = None;
        loop {
            if self.eof() {
                break;
            }
            if self.at_str("/*") || self.at_str("//") {
                if let Some((start, end)) = run.take() {
                    self.pos = start as u32;
                    self.emit(&mut el.children, end, TokenKind::Text);
                }
                self.trivia(&mut el.children);
                continue;
            }
            let c = self.peek().unwrap();
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            let start = self.at();
            // A Java string, char or text block is text whatever it holds
            // (`"}"`, `'{'`, `"\""`); braces count only outside them.
            let end = if matches!(c, '"' | '\'') {
                scan::java_literal_end(self.src, start)
            } else {
                start + c.len_utf8()
            };
            run = Some((run.map_or(start, |(s, _)| s), end));
            self.pos = end as u32;
        }
        if let Some((start, end)) = run.take() {
            self.pos = start as u32;
            self.emit(&mut el.children, end, TokenKind::Text);
        }
        if self.peek() == Some('}') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::Close(Delim::Brace))));
        }
        el
    }

    /// `= value` after an attribute name; `true` when the value was `java`.
    fn attribute_value(&mut self, out: &mut Vec<Node>, style: AttrStyle, value: Value) -> bool {
        let mark = self.pos;
        let mut trivia = Vec::new();
        self.trivia(&mut trivia);
        if self.peek() != Some('=') {
            self.pos = mark;
            return false;
        }
        out.append(&mut trivia);
        self.emit_len(out, 1, TokenKind::Punct(Punct::KeyValue));
        if style == AttrStyle::Script {
            self.trivia(out);
            if self.peek() == Some('#') {
                let t = self.template_expression();
                self.finish_into(out, t);
                return false;
            }
            self.expression(out, Stop::EXPR);
            return false;
        }
        self.trivia(out);
        if matches!(self.peek(), Some('\'' | '"')) {
            // A quoted value's content is `StringText` (the `String` element
            // re-kinds every child); a property's `name` value is also split
            // at its whitespace runs.
            let in_tag = self.in_tag;
            let style = if value == Value::Plain {
                StringStyle::Attribute
            } else {
                StringStyle::AttributeNamed
            };
            let mut s = self.string(in_tag, style);
            if value == Value::PropertyName {
                split_at_whitespace(self.src, &mut s.children);
            }
            let java = s.children.iter().any(|n| {
                matches!(n, Node::Token(t) if self.src[t.span.start as usize..t.span.end as usize]
                    .eq_ignore_ascii_case("java"))
            });
            self.finish_into(out, s);
            return java;
        }
        // An unquoted value runs to the style's break set.
        let stops: &[char] = if style == AttrStyle::Parameter {
            &[
                ' ', '\t', '\n', '\r', '\x0c', '<', '/', '>', '{', ';', ')', ',',
            ]
        } else {
            &[' ', '\t', '\n', '\r', '\x0c', '<', '/', '>', '{', ';']
        };
        let start = self.at();
        let mut end = start;
        while let Some(c) = self.src[end..].chars().next() {
            if stops.contains(&c) || c == '#' {
                break;
            }
            end += c.len_utf8();
        }
        if end == start {
            if self.peek() == Some('#') {
                let t = self.template_expression();
                self.finish_into(out, t);
            }
            return false;
        }
        let kind = match value {
            Value::Plain => TokenKind::Literal(Literal::Unquoted),
            Value::InheritedClass => TokenKind::Ident(Ident::ClassName),
            Value::PropertyName => TokenKind::Ident(Ident::PropertyName),
        };
        let java = self.src[start..end].eq_ignore_ascii_case("java");
        self.emit(out, end, kind);
        java
    }
}

// ---------------------------------------------------------------------------
// Lookaheads
// ---------------------------------------------------------------------------

/// A struct key: an identifier, digits or a quoted string (a `\` escapes the
/// next character). The flag says whether it was quoted.
fn property_name(src: &str, at: usize) -> Option<(usize, bool)> {
    match src[at..].chars().next()? {
        q @ ('\'' | '"') => {
            let mut i = at + 1;
            while let Some(c) = src[i..].chars().next() {
                if c == '\\' {
                    i += 1 + src[i + 1..].chars().next().map_or(0, char::len_utf8);
                    continue;
                }
                i += c.len_utf8();
                if c == q {
                    return Some((i, true));
                }
            }
            None
        }
        c if c.is_ascii_digit() => {
            let mut i = at;
            while src.as_bytes().get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            Some((i, false))
        }
        _ => lexer::identifier(src, at).map(|e| (e, false)),
    }
}

/// Whether `=` (not `==`, not `=>`) and a function follow
/// (`either_function_ahead`): the name before is assigned a function.
fn function_assignment_ahead(src: &str, at: usize, depth: u32) -> bool {
    let eq = skip_spaces(src, at);
    if !src[eq..].starts_with('=') || src[eq..].starts_with("==") || src[eq..].starts_with("=>") {
        return false;
    }
    either_function_ahead(src, skip_spaces(src, eq + 1), depth)
}

/// Whether a function starts here: `function`, or an arrow's parameters
/// (`(…)` or one name) followed by `=>`.
fn either_function_ahead(src: &str, at: usize, depth: u32) -> bool {
    let at = skip_spaces(src, at);
    if lexer::keyword_at(src, at, "function") {
        return true;
    }
    let after = match src[at..].chars().next() {
        Some('(') => match scan::balanced_code(src, at, b'(', b')', depth) {
            Some(end) => end,
            None => return false,
        },
        _ => match lexer::identifier(src, at) {
            Some(end) => end,
            None => return false,
        },
    };
    src[skip_spaces(src, after)..].starts_with("=>")
}

/// Whether a function declaration starts here: modifiers and a return type,
/// each followed by whitespace, then `function`.
fn function_declaration_ahead(src: &str, at: usize) -> bool {
    // The modifiers (each at most once, in any order) and at most one
    // return type (a dot path, `[]` allowed), each followed by a space,
    // then `function`: the same reading as `function_declaration`.
    let mut at = at;
    let mut seen: Vec<&str> = Vec::new();
    let mut typed = false;
    loop {
        if lexer::keyword_at(src, at, "function") {
            return true;
        }
        let Some(end) = lexer::identifier(src, at) else {
            return false;
        };
        let slot = modifier_slot(&src[at..end]);
        let end = if let Some(slot) = slot.filter(|s| !seen.contains(s)) {
            seen.push(slot);
            end
        } else if !typed {
            typed = true;
            let Some(end) = lexer::dot_path(src, at) else {
                return false;
            };
            if src[end..].starts_with("[]") {
                end + 2
            } else {
                end
            }
        } else {
            return false;
        };
        let next = skip_spaces(src, end);
        if next == end {
            return false;
        }
        at = next;
    }
}

/// The slot a modifier word fills, in any case: the four access words
/// share one (`package remote function` makes `remote` the return type on
/// Lucee), and each storage word (`abstract`, `final`, `static`) has its
/// own. `None` for any other word.
fn modifier_slot(word: &str) -> Option<&'static str> {
    if lexer::in_list(lexer::ACCESS_MODIFIERS, word) {
        return Some("access");
    }
    lexer::STORAGE_MODIFIERS
        .iter()
        .copied()
        .find(|w| w.eq_ignore_ascii_case(word))
}

/// Splits each token into its whitespace and non-whitespace runs, as the tag
/// scanner's `Scanner::meta_region` does.
fn split_at_whitespace(src: &str, nodes: &mut Vec<Node>) {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for node in nodes.drain(..) {
        let Node::Token(t) = node else {
            out.push(node);
            continue;
        };
        let text = &src[t.span.start as usize..t.span.end as usize];
        let mixed = text.bytes().any(|b| b.is_ascii_whitespace())
            && !text.bytes().all(|b| b.is_ascii_whitespace());
        if !mixed {
            out.push(Node::Token(t));
            continue;
        }
        let bytes = text.as_bytes();
        let mut from = 0;
        for i in 1..=bytes.len() {
            if i == bytes.len()
                || bytes[i].is_ascii_whitespace() != bytes[from].is_ascii_whitespace()
            {
                out.push(Node::Token(Token {
                    span: t.span.start + from as u32..t.span.start + i as u32,
                    kind: t.kind,
                }));
                from = i;
            }
        }
    }
    *nodes = out;
}
