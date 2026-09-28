//! Typed views over the expression-structure elements: key-values and
//! declarations, built by the front ends, and the rest, built by the
//! post-passes ([`crate::postpass`]).
//!
//! The new elements have no `open`/`close`: their parts are children, with
//! the trivia (whitespace, newlines, comments) between the first and the last
//! part kept in span order. A view skips that trivia and names the parts. Get
//! one with the matching `Element::as_*` method, which returns `None` for any
//! other kind:
//!
//! ```
//! use cfparse::{parse_source, Mode, Node};
//!
//! let tree = parse_source("x = a + b * 2;", Mode::Script);
//! let stmt = tree.root.children[0].as_element().unwrap();
//! let assign = stmt.children[0].as_element().unwrap().as_assignment().unwrap();
//! assert_eq!(tree.text(assign.op()), "=");
//! let sum = assign.value().as_element().unwrap().as_binary().unwrap();
//! assert_eq!(sum.operands().count(), 2);
//! ```

use crate::tree::{Element, ElementKind, Node, Punct, SegmentKind, Token, TokenKind};

/// Children of `el` that are not trivia, in order.
fn parts(el: &Element) -> impl Iterator<Item = &Node> {
    el.children.iter().filter(|n| !n.is_trivia())
}

fn nth(el: &Element, n: usize) -> &Node {
    parts(el)
        .nth(n)
        .expect("expression node built with all its parts")
}

fn token(node: &Node) -> &Token {
    node.as_token()
        .expect("operator part of an expression node is a token")
}

impl Element {
    /// View of a [`KeyValue`](ElementKind::KeyValue).
    pub fn as_key_value(&self) -> Option<KeyValue<'_>> {
        (self.kind == ElementKind::KeyValue).then_some(KeyValue(self))
    }

    /// View of a [`Function`](ElementKind::Function),
    /// [`Class`](ElementKind::Class), [`Interface`](ElementKind::Interface)
    /// or [`StaticBlock`](ElementKind::StaticBlock).
    pub fn as_decl(&self) -> Option<Decl<'_>> {
        matches!(
            self.kind,
            ElementKind::Function { .. }
                | ElementKind::Class
                | ElementKind::Interface
                | ElementKind::StaticBlock
        )
        .then_some(Decl(self))
    }

    /// View of an [`Assignment`](ElementKind::Assignment).
    pub fn as_assignment(&self) -> Option<Assignment<'_>> {
        (self.kind == ElementKind::Assignment).then_some(Assignment(self))
    }

    /// View of a [`Ternary`](ElementKind::Ternary).
    pub fn as_ternary(&self) -> Option<Ternary<'_>> {
        (self.kind == ElementKind::Ternary).then_some(Ternary(self))
    }

    /// View of a [`Binary`](ElementKind::Binary).
    pub fn as_binary(&self) -> Option<Binary<'_>> {
        matches!(self.kind, ElementKind::Binary { .. }).then_some(Binary(self))
    }

    /// View of a [`Unary`](ElementKind::Unary).
    pub fn as_unary(&self) -> Option<Unary<'_>> {
        matches!(self.kind, ElementKind::Unary { .. }).then_some(Unary(self))
    }

    /// View of a [`CallExpr`](ElementKind::CallExpr).
    pub fn as_call_expr(&self) -> Option<CallExpr<'_>> {
        (self.kind == ElementKind::CallExpr).then_some(CallExpr(self))
    }

    /// View of a [`New`](ElementKind::New).
    pub fn as_new(&self) -> Option<New<'_>> {
        (self.kind == ElementKind::New).then_some(New(self))
    }

    /// View of a [`Chain`](ElementKind::Chain).
    pub fn as_chain(&self) -> Option<Chain<'_>> {
        (self.kind == ElementKind::Chain).then_some(Chain(self))
    }

    /// View of a [`Segment`](ElementKind::Segment).
    pub fn as_segment(&self) -> Option<Segment<'_>> {
        matches!(self.kind, ElementKind::Segment(_)).then_some(Segment(self))
    }
}

/// `key sep value`. The key is a struct key, argument name, parameter name or
/// attribute name token, or a `String` / `TemplateExpression` element; the
/// separator is always re-kinded to `Punct(KeyValue)` (`:` or `=`).
#[derive(Debug, Clone, Copy)]
pub struct KeyValue<'a>(pub &'a Element);

impl<'a> KeyValue<'a> {
    pub fn key(&self) -> &'a Node {
        nth(self.0, 0)
    }

    pub fn separator(&self) -> &'a Token {
        token(nth(self.0, 1))
    }

    /// Every child after the separator, trivia included. After the
    /// expression pass this holds exactly one significant node unless the
    /// value could not be parsed.
    pub fn value(&self) -> &'a [Node] {
        let sep = self
            .0
            .children
            .iter()
            .position(
                |n| matches!(n, Node::Token(t) if t.kind == TokenKind::Punct(Punct::KeyValue)),
            )
            .expect("key-value separator");
        &self.0.children[sep + 1..]
    }
}

/// A declaration fused with its body: `Function` (`FunctionDecl` or
/// `ArrowFunction` header; for an arrow the `=>` token sits between header and
/// body), `Class` (`ClassDecl`), `Interface` (`InterfaceDecl`), `StaticBlock`
/// (the `static` keyword token).
#[derive(Debug, Clone, Copy)]
pub struct Decl<'a>(pub &'a Element);

impl<'a> Decl<'a> {
    pub fn header(&self) -> &'a Node {
        nth(self.0, 0)
    }

    /// `=>` of an arrow function.
    pub fn arrow(&self) -> Option<&'a Token> {
        parts(self.0)
            .filter_map(Node::as_token)
            .find(|t| t.kind == TokenKind::Keyword(crate::tree::Keyword::Arrow))
    }

    /// The `Block` (braced or, for an arrow, brace-less).
    pub fn body(&self) -> &'a Element {
        parts(self.0)
            .last()
            .and_then(Node::as_element)
            .expect("declaration body block")
    }
}

/// `target op value`.
#[derive(Debug, Clone, Copy)]
pub struct Assignment<'a>(pub &'a Element);

impl<'a> Assignment<'a> {
    pub fn target(&self) -> &'a Node {
        nth(self.0, 0)
    }

    /// `=` (`Operator::Assign`) or `+=` etc. (`Operator::AugAssign`).
    pub fn op(&self) -> &'a Token {
        token(nth(self.0, 1))
    }

    pub fn value(&self) -> &'a Node {
        nth(self.0, 2)
    }
}

/// `cond ? then : otherwise`.
#[derive(Debug, Clone, Copy)]
pub struct Ternary<'a>(pub &'a Element);

impl<'a> Ternary<'a> {
    pub fn cond(&self) -> &'a Node {
        nth(self.0, 0)
    }

    pub fn question(&self) -> &'a Token {
        token(nth(self.0, 1))
    }

    pub fn then(&self) -> &'a Node {
        nth(self.0, 2)
    }

    pub fn colon(&self) -> &'a Token {
        token(nth(self.0, 3))
    }

    pub fn otherwise(&self) -> &'a Node {
        nth(self.0, 4)
    }
}

/// `a op b op c …` at one precedence level.
#[derive(Debug, Clone, Copy)]
pub struct Binary<'a>(pub &'a Element);

impl<'a> Binary<'a> {
    /// Operands (two or more), in order.
    pub fn operands(&self) -> impl Iterator<Item = &'a Node> {
        parts(self.0).step_by(2)
    }

    /// Operators (one fewer than the operands), in order: each a token, or
    /// a [`Phrase`](ElementKind::Phrase) element for a multi-word operator
    /// (`less than`).
    pub fn operators(&self) -> impl Iterator<Item = &'a Node> {
        parts(self.0).skip(1).step_by(2)
    }

    pub fn prec(&self) -> crate::tree::Prec {
        match self.0.kind {
            ElementKind::Binary { prec } => prec,
            _ => unreachable!("Binary view over a binary element"),
        }
    }
}

/// `op operand` or `operand op`.
#[derive(Debug, Clone, Copy)]
pub struct Unary<'a>(pub &'a Element);

impl<'a> Unary<'a> {
    pub fn is_postfix(&self) -> bool {
        matches!(self.0.kind, ElementKind::Unary { postfix: true })
    }

    pub fn op(&self) -> &'a Token {
        token(nth(self.0, usize::from(self.is_postfix())))
    }

    pub fn operand(&self) -> &'a Node {
        nth(self.0, usize::from(!self.is_postfix()))
    }
}

/// `callee(args)` where the callee is not a member access.
#[derive(Debug, Clone, Copy)]
pub struct CallExpr<'a>(pub &'a Element);

impl<'a> CallExpr<'a> {
    pub fn callee(&self) -> &'a Node {
        nth(self.0, 0)
    }

    /// The `Call` element.
    pub fn args(&self) -> &'a Element {
        parts(self.0)
            .last()
            .and_then(Node::as_element)
            .expect("call arguments")
    }
}

/// `new Name(args)`, `new "path"(args)`, `new java(args)`, `new Name`.
#[derive(Debug, Clone, Copy)]
pub struct New<'a>(pub &'a Element);

impl<'a> New<'a> {
    pub fn keyword(&self) -> &'a Token {
        token(nth(self.0, 0))
    }

    /// Class name token (`ident.class-name`, `kw.component`,
    /// `storage.type` for `java`) or `String` element.
    pub fn class(&self) -> &'a Node {
        nth(self.0, 1)
    }

    /// The `Call` element, when the constructor has arguments.
    pub fn args(&self) -> Option<&'a Element> {
        parts(self.0).nth(2).and_then(Node::as_element)
    }
}

/// `head segment segment …`.
#[derive(Debug, Clone, Copy)]
pub struct Chain<'a>(pub &'a Element);

impl<'a> Chain<'a> {
    pub fn head(&self) -> &'a Node {
        nth(self.0, 0)
    }

    /// The `Segment` elements (one or more).
    pub fn segments(&self) -> impl Iterator<Item = &'a Element> {
        parts(self.0).skip(1).filter_map(Node::as_element)
    }
}

/// One access of a chain: `.name`, `?.name`, `::name`, with `Call` arguments
/// for a method, or a `Brackets` element for an index.
#[derive(Debug, Clone, Copy)]
pub struct Segment<'a>(pub &'a Element);

impl<'a> Segment<'a> {
    pub fn kind(&self) -> SegmentKind {
        match self.0.kind {
            ElementKind::Segment(kind) => kind,
            _ => unreachable!("Segment view over a segment element"),
        }
    }

    /// `.`, `?.` or `::`; `None` for an index.
    pub fn accessor(&self) -> Option<&'a Token> {
        parts(self.0).next().and_then(Node::as_token).filter(|t| {
            matches!(
                t.kind,
                TokenKind::Punct(Punct::Accessor | Punct::SafeAccessor | Punct::StaticAccessor)
            )
        })
    }

    /// The member name; `None` for an index.
    pub fn name(&self) -> Option<&'a Token> {
        self.accessor()?;
        parts(self.0).nth(1).and_then(Node::as_token)
    }

    /// Arguments of a method.
    pub fn call(&self) -> Option<&'a Element> {
        parts(self.0)
            .filter_map(Node::as_element)
            .find(|e| matches!(e.kind, ElementKind::Call))
    }

    /// Brackets of an index.
    pub fn brackets(&self) -> Option<&'a Element> {
        parts(self.0)
            .filter_map(Node::as_element)
            .find(|e| e.kind == ElementKind::Brackets)
    }

    /// `?.`
    pub fn is_safe(&self) -> bool {
        self.accessor()
            .is_some_and(|t| t.kind == TokenKind::Punct(Punct::SafeAccessor))
    }

    /// `::`
    pub fn is_static(&self) -> bool {
        self.accessor()
            .is_some_and(|t| t.kind == TokenKind::Punct(Punct::StaticAccessor))
    }
}

// Plain attribute values: the one reading `cfformat arrange` and cfvet
// share.

impl<'a> KeyValue<'a> {
    /// The value's text when it is plain ([`plain_text`]), and the offset
    /// where that text starts.
    pub fn plain_text(&self, tree: &'a crate::tree::Tree) -> Option<(&'a str, u32)> {
        plain_text(tree, self.value())
    }
}

/// The text of a value written as text alone, known without evaluating or
/// unescaping anything, and the offset where the text starts; `None` for
/// anything else. `value` holds exactly one node besides trivia, which is:
///
/// - a string whose content is string text alone (possibly none): no `#…#`
///   (in a tag attribute `#x#` stays string text, and is refused here as
///   the expression it is) and no escaped quote or `##`;
/// - an unquoted word: `Literal::Unquoted`, a tag's `access=private`
///   (`Storage::Modifier`) or a script property's `name=foo`
///   (`Ident::PropertyName`). A script tag's `name=q` is a variable
///   (`Ident::Variable`), evaluated, so not plain; neither is a number or a
///   boolean.
///
/// This is the stricter of two readings it replaces: arrange also took an
/// escaped string (unescaped), a number, a boolean and any identifier but
/// no comment before the value, and cfvet also took `#` in tag string
/// text. The differences are in names and access words no engine accepts.
pub fn plain_text<'a>(tree: &'a crate::tree::Tree, value: &[Node]) -> Option<(&'a str, u32)> {
    use crate::tree::{Ident, Literal, Storage};
    let mut significant = value.iter().filter(|n| !n.is_trivia());
    let node = significant.next()?;
    if significant.next().is_some() {
        return None;
    }
    match node {
        Node::Token(t)
            if matches!(
                t.kind,
                TokenKind::Literal(Literal::Unquoted)
                    | TokenKind::Storage(Storage::Modifier)
                    | TokenKind::Ident(Ident::PropertyName)
            ) =>
        {
            Some((tree.text(t), t.span.start))
        }
        Node::Element(e) if matches!(e.kind, ElementKind::String { .. }) => {
            let text_only = e.children.iter().all(|n| {
                matches!(n, Node::Token(t)
                    if t.kind == TokenKind::Literal(Literal::StringText)
                        && !tree.text(t).contains('#'))
            });
            if !text_only {
                return None;
            }
            let start = match e.children.first() {
                Some(first) => first.span().start,
                None => e.open.as_ref().map_or(e.span.start, |q| q.span.end),
            };
            let end = e.children.last().map_or(start, |last| last.span().end);
            Some((tree.slice(start..end), start))
        }
        _ => None,
    }
}
