//! The tag-in-script forms: `property`, `param`, `http url="x";`,
//! `transaction { … }` and Lucee's `cffile(…)`.

use crate::tree::{
    BlockKind, Delim, Element, ElementKind, Ident, Literal, Node, Operator, Punct, RecoveryReason,
    StatementKind, Storage, TokenKind,
};

use super::expressions::AttrStyle;
use super::lexer::{self, skip_trivia, Comments};
use super::parser::{skip_spaces, Parser, Stop};

impl Parser<'_> {
    /// A tag written in script: `property`, `param`, a generic script tag
    /// (`lock name="x" {`, `http url="x";`) or a `cf`-prefixed tag called
    /// like a function (`cffile(…)`), and the statement's kind; `None` when
    /// the statement is none.
    pub(super) fn tag_in_script(&mut self, el: &mut Element) -> Option<StatementKind> {
        if self.at_property() {
            self.script_tag_property(el);
            return Some(StatementKind::Property);
        }
        if self.at_param() {
            self.script_tag_param(el);
            return Some(StatementKind::Param);
        }
        let name_end = self.tag_name_end()?;
        let name = &self.src[self.at()..name_end];
        // A `cf`-prefixed script tag followed by `(`: `cffile (…)`, a
        // comment before `(` too.
        if lexer::is_cf_tag_in_script(name)
            && self.src[skip_trivia(self.src, name_end, Comments::All)..].starts_with('(')
        {
            self.script_tag_cf(el, name_end);
            return Some(StatementKind::ScriptTag);
        }
        if !lexer::is_tag_in_script(name) {
            return None;
        }
        // The bare-name branch point: a tag name alone at the end of its line
        // is a variable when the next line starts with `.`.
        let after = skip_spaces(self.src, name_end);
        let bare = self.src[name_end..after].contains('\n') || after == self.src.len();
        if bare {
            if self.src[after..].starts_with('.') {
                return None;
            }
        } else if !self.generic_tag_ahead(name_end) {
            return None;
        }
        self.script_tag_generic(el, name_end);
        Some(StatementKind::ScriptTag)
    }

    /// Whether a generic script tag follows the tag name:
    /// `(?=\s+NAME\s*[=;{\n]|\s*[{\n])`, NAME an attribute name
    /// ([`lexer::attribute_name`]), with comments between the attribute name and what follows it read as
    /// whitespace (`lock name // c` newline `="a"`): the attributes that
    /// follow are read with their comments.
    fn generic_tag_ahead(&self, name_end: usize) -> bool {
        let after = skip_spaces(self.src, name_end);
        if self.src[after..].starts_with('{') {
            return true;
        }
        if after == name_end {
            return false;
        }
        let Some(attr) = lexer::attribute_name(self.src, after) else {
            return false;
        };
        let next = skip_trivia(self.src, attr, Comments::All);
        self.src[next..].starts_with(['=', ';', '{', '\n'])
    }

    /// The end of the identifier at the current offset, a candidate tag name
    /// (the caller checks it against the script tag list).
    fn tag_name_end(&self) -> Option<usize> {
        let end = lexer::identifier(self.src, self.at())?;
        lexer::word_boundary_before(self.src, self.at()).then_some(end)
    }

    fn at_property(&self) -> bool {
        if !self.at_word("property") {
            return false;
        }
        let end = self.at() + "property".len();
        let after = skip_spaces(self.src, end);
        if after > end && lexer::dot_path(self.src, after).is_some() {
            return true;
        }
        self.src[after..].starts_with(['\n', '{'])
    }

    fn at_param(&self) -> bool {
        if !self.at_word("param") {
            return false;
        }
        let end = self.at() + "param".len();
        let after = skip_spaces(self.src, end);
        if after > end && lexer::identifier(self.src, after).is_some() {
            return true;
        }
        self.src[after..].starts_with(['\n', '{'])
    }

    /// `property …;`
    fn script_tag_property(&mut self, el: &mut Element) {
        let mut tag = self.element(ElementKind::Property);
        self.emit_len(
            &mut tag.children,
            "property".len(),
            TokenKind::Ident(Ident::TagName),
        );
        let in_tag = std::mem::replace(&mut self.in_tag, true);
        // An inline type and name (`property string name …`) or a name alone
        // (`property name …`); a word followed by `=` is an attribute.
        self.whitespace_only(&mut tag.children);
        let at = self.at();
        if let Some(first) = lexer::dot_path(self.src, at) {
            let after = skip_spaces(self.src, first);
            let second = lexer::identifier(self.src, after).filter(|_| after > first);
            let assigned = |end: usize| self.src[skip_spaces(self.src, end)..].starts_with('=');
            if let Some(second) = second.filter(|e| !assigned(*e)) {
                self.emit(&mut tag.children, first, TokenKind::Storage(Storage::Type));
                self.whitespace_only(&mut tag.children);
                self.emit(
                    &mut tag.children,
                    second,
                    TokenKind::Ident(Ident::PropertyName),
                );
            } else if lexer::identifier(self.src, at) == Some(first) && !assigned(first) {
                self.emit(
                    &mut tag.children,
                    first,
                    TokenKind::Ident(Ident::PropertyName),
                );
            }
        }
        self.tag_attributes(&mut tag.children, AttrStyle::PropertyTag);
        self.in_tag = in_tag;
        let block = self.peek() == Some('{');
        self.finish_into(&mut el.children, tag);
        if block {
            let b = self.block(BlockKind::Plain);
            self.finish_into(&mut el.children, b);
            return;
        }
        self.trivia(&mut el.children);
        self.expect_semicolon(el);
    }

    /// `param …;`
    fn script_tag_param(&mut self, el: &mut Element) {
        let mut tag = self.element(ElementKind::Param);
        self.emit_len(
            &mut tag.children,
            "param".len(),
            TokenKind::Ident(Ident::TagName),
        );
        let in_tag = std::mem::replace(&mut self.in_tag, true);
        self.param_inline(&mut tag.children);
        self.tag_attributes(&mut tag.children, AttrStyle::Script);
        self.in_tag = in_tag;
        let block = self.peek() == Some('{');
        self.finish_into(&mut el.children, tag);
        if block {
            let b = self.block(BlockKind::Plain);
            self.finish_into(&mut el.children, b);
            return;
        }
        self.trivia(&mut el.children);
        self.expect_semicolon(el);
    }

    /// The inline part of a `param` before its attributes: `param name`,
    /// `param type name`, either followed by `= value`. Nothing when the
    /// first word is an attribute given a value (`param name="x"`).
    fn param_inline(&mut self, out: &mut Vec<Node>) {
        let mark = self.pos;
        let mut trivia = Vec::new();
        self.whitespace_only(&mut trivia);
        let at = self.at();
        // Nothing inline when the statement ends or a block opens here.
        if self.src[at..].starts_with(['{', ';', '\n']) || at >= self.src.len() {
            self.pos = mark;
            return;
        }
        // A comment before the `=` (`param name // c` newline `="x"`) leaves
        // it an attribute.
        if let Some(end) = lexer::identifier(self.src, at) {
            if lexer::in_list(lexer::PARAM_ATTRIBUTES, &self.src[at..end])
                && self.src[skip_trivia(self.src, end, Comments::All)..].starts_with('=')
            {
                self.pos = mark;
                return;
            }
        }
        let Some(first) = lexer::dot_path(self.src, at) else {
            self.pos = mark;
            return;
        };
        out.append(&mut trivia);
        // `param name max=…`: a name followed by an attribute name is the
        // name alone.
        let after = skip_spaces(self.src, first);
        if after > first {
            if let Some(attr_end) = lexer::identifier(self.src, after) {
                if lexer::in_list(lexer::PARAM_ATTRIBUTES, &self.src[after..attr_end]) {
                    self.emit(out, first, TokenKind::Literal(Literal::Unquoted));
                    return;
                }
            }
        }
        // `param type name`: an identifier, whitespace, then a dotted path.
        if lexer::identifier(self.src, at) == Some(first) && after > first {
            if let Some(second) = lexer::dot_path(self.src, after) {
                self.emit(out, first, TokenKind::Storage(Storage::Type));
                self.whitespace_only(out);
                self.emit(out, second, TokenKind::Literal(Literal::Unquoted));
                return self.param_assignment(out);
            }
        }
        self.emit(out, first, TokenKind::Literal(Literal::Unquoted));
        self.param_assignment(out)
    }

    fn param_assignment(&mut self, out: &mut Vec<Node>) {
        let mark = self.pos;
        let mut trivia = Vec::new();
        self.trivia(&mut trivia);
        if self.peek() != Some('=') || self.at_str("==") || self.at_str("=>") {
            self.pos = mark;
            return;
        }
        out.append(&mut trivia);
        self.emit_len(out, 1, TokenKind::Operator(Operator::Assign));
        // The value after `param name =` is a script expression, so a string
        // here is an expression's string, not an attribute value.
        let in_tag = std::mem::replace(&mut self.in_tag, false);
        self.expression(out, Stop::EXPR);
        self.in_tag = in_tag;
    }

    /// A generic script tag (`lock name="x" { … }`, `http url="x";`) or a
    /// bare one (the tag name alone on its line): the name, the attributes,
    /// then a `{ … }` body or an optional `;`.
    fn script_tag_generic(&mut self, el: &mut Element, name_end: usize) {
        let mut tag = self.element(ElementKind::ScriptTag { acf: false });
        self.emit(
            &mut tag.children,
            name_end,
            TokenKind::Ident(Ident::TagName),
        );
        let in_tag = std::mem::replace(&mut self.in_tag, true);
        self.tag_attributes(&mut tag.children, AttrStyle::Script);
        self.in_tag = in_tag;
        let block = self.peek() == Some('{');
        self.finish_into(&mut el.children, tag);
        if block {
            let b = self.block(BlockKind::Plain);
            self.finish_into(&mut el.children, b);
            return;
        }
        // An optional `;`, taken as the statement's empty terminator rather
        // than through `expect_semicolon`.
        self.trivia(&mut el.children);
        if self.peek() == Some(';') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::EmptyTerminator)));
        }
    }

    /// A `cf`-prefixed tag called like a function: `cffile(action="read", …);`
    fn script_tag_cf(&mut self, el: &mut Element, name_end: usize) {
        let mut tag = self.element(ElementKind::ScriptTag { acf: true });
        self.emit(
            &mut tag.children,
            name_end,
            TokenKind::Ident(Ident::TagName),
        );
        self.trivia(&mut tag.children);
        if self.peek() == Some('(') {
            let attrs = self.script_tag_attributes();
            self.finish_into(&mut tag.children, attrs);
        }
        self.finish_into(&mut el.children, tag);
        // The statement goes on after the call: a `{` body and the `;` both
        // belong to this statement.
        self.trivia(&mut el.children);
        if self.peek() == Some(';') {
            el.close = Some(self.take(self.at() + 1, TokenKind::Punct(Punct::EmptyTerminator)));
            return;
        }
        if self.eof() || matches!(self.peek(), Some('}' | ')' | ']')) {
            return;
        }
        // `statement_body` reads the rest into `el`, in its loop (not a
        // recursive call: `cffile(…) cffile(…) …` would grow the stack).
        self.resume_statement = true;
    }

    /// The `( … )` of a `cffile(…)` tag: comma-separated `name = value` (or
    /// `name: value`) items, each value a script expression.
    fn script_tag_attributes(&mut self) -> Element {
        self.delimited(
            ElementKind::ScriptTagAttributes,
            Delim::Paren,
            &[')', '}', ';'],
            None,
            Self::script_tag_attribute,
        )
    }

    fn script_tag_attribute(&mut self, out: &mut Vec<Node>) {
        let Some(end) = lexer::attribute_name(self.src, self.at()) else {
            // One character, not one byte: `cffile(💩)`.
            let len = self.peek().map_or(1, char::len_utf8);
            self.emit_invalid(out, len, RecoveryReason::Unmatched);
            return;
        };
        self.emit(out, end, TokenKind::Ident(Ident::AttributeName));
        let mark = self.pos;
        let mut trivia = Vec::new();
        self.trivia(&mut trivia);
        if !matches!(self.peek(), Some('=' | ':')) {
            self.pos = mark;
            return;
        }
        out.append(&mut trivia);
        self.emit_len(out, 1, TokenKind::Punct(Punct::KeyValue));
        self.expression(out, Stop::NO_COMMA)
    }
}
