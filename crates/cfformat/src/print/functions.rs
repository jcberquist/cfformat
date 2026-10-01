//! Function declarations, anonymous functions and arrows.
//!
//! `[modifiers type function name, params, attributes, sep, body]`: the words
//! before the parameters joined by one space, the parameters through the
//! delimited printer (`function_declaration` for a named function,
//! `function_anonymous` otherwise) unless a sole pattern parameter hugs the
//! parentheses ([`hugged_parameter`]), the metadata attributes as an attribute
//! group, then `{` after one space — or on its own line when the attributes
//! broke (`if_break_group` on the attribute group). An arrow prints ` => ` and
//! its body; a brace-less body may break onto the next line after `=>`
//! ([`Printer::arrow_tail`]). The spacing around the parameter list has no
//! option: always `function f(` and `) {`.

use cfdoc::builders::{
    break_parent, group, group_opts, hardline, hardline_without_break_parent, if_break_group,
    indent, line, line_suffix, softline, GroupOpts,
};
use cfdoc::utils::remove_lines;
use cfdoc::{Doc, GroupId};
use cfparse::{BlockKind, Element, ElementKind, Ident, Item, Keyword, Node, TokenKind};

use super::delimited::{
    has_comments, is_pattern_assignment, is_printable, printable_items, DelimitedStyle,
};
use super::expressions::is_parenthesised_literal;
use super::Printer;

impl Printer<'_> {
    /// A `Function` (declaration, anonymous function or arrow) with its body.
    pub(crate) fn function(&self, e: &Element) -> Doc {
        let Some(decl) = e.as_decl() else {
            return self.as_written(e);
        };
        let body = decl.body();
        // Taken first: nothing printed below is the hugged arrow.
        let hugged = self.hug_arrow.take();
        let breaks_after_arrow = arrow_breaks_after(body);
        let mut parts = Vec::new();
        let mut attributes: Option<GroupId> = None;
        let mut arrow = false;
        for n in &e.children {
            match n {
                Node::Element(c) if c.kind.is_comment() => parts.push(self.same_line_comment(c)),
                n if n.is_trivia() => {}
                Node::Element(h)
                    if matches!(
                        h.kind,
                        ElementKind::FunctionDecl | ElementKind::ArrowFunction
                    ) =>
                {
                    let (doc, id) = self.function_header(h);
                    // Prettier's `expandLastArg`: a hugged arrow's
                    // parameters never break.
                    parts.push(if hugged.is_some() {
                        remove_lines(doc)
                    } else {
                        doc
                    });
                    attributes = id;
                }
                Node::Token(t) if t.kind == TokenKind::Keyword(Keyword::Arrow) => {
                    // Always ` => `, or ` =>` when the body starts the next
                    // line.
                    parts.push(Doc::from(if breaks_after_arrow { " =>" } else { " => " }));
                    arrow = true;
                }
                Node::Element(b) if std::ptr::eq(&**b, body) => {
                    if !arrow {
                        parts.push(match attributes {
                            Some(id) => if_break_group(hardline_without_break_parent(), " ", id),
                            None => Doc::from(" "),
                        });
                    }
                    if arrow && breaks_after_arrow {
                        parts.push(self.arrow_tail(b, hugged));
                    } else {
                        parts.push(self.element(b));
                    }
                }
                n => parts.push(self.node(n)),
            }
        }
        Doc::Concat(parts)
    }

    /// A `FunctionDecl` or `ArrowFunction` header, and the id of its
    /// attribute group when it has attributes. A body-less declaration
    /// (interface, abstract component) is just this.
    pub(crate) fn function_header(&self, h: &Element) -> (Doc, Option<GroupId>) {
        let Some(params_at) = h
            .children
            .iter()
            .position(|n| matches!(n, Node::Element(p) if p.kind == ElementKind::Parameters))
        else {
            // No parameter list (a misparse): the parts in order.
            return (Doc::Concat(self.sequence(&h.children)), None);
        };
        let named = h.children[..params_at].iter().any(
            |n| matches!(n, Node::Token(t) if t.kind == TokenKind::Ident(Ident::FunctionName)),
        );
        let style = if named && h.kind == ElementKind::FunctionDecl {
            DelimitedStyle::function_declaration(self.opts)
        } else {
            DelimitedStyle::function_anonymous(self.opts)
        };

        // The words before the parameters. Comments after the last word are
        // followed by the parameters, not by a line break: a line comment
        // there ends the line the header ends on.
        let words = &h.children[..params_at];
        let end = words
            .iter()
            .rposition(|n| !n.is_trivia())
            .map_or(0, |i| i + 1);
        let mut parts = self.sequence(&words[..end]);
        parts.extend(
            words[end..]
                .iter()
                .filter_map(Node::as_element)
                .filter(|c| c.kind.is_comment())
                .map(|c| self.deferred_comment(c)),
        );
        // A block comment keeps a space on both sides; a line comment is a
        // line suffix, and `(` follows the name as it does without one.
        if h.children[..params_at]
            .iter()
            .rfind(|n| !matches!(n, Node::Token(t) if t.kind == TokenKind::Whitespace || t.kind == TokenKind::Newline))
            .is_some_and(|n| matches!(n, Node::Element(c) if c.kind.is_comment() && c.kind != ElementKind::LineComment))
        {
            parts.push(Doc::from(" "));
        }
        let params = h.children[params_at].as_element().expect("parameters");
        parts.push(if let Some(only) = hugged_parameter(params) {
            // Prettier's `shouldHugTheOnlyFunctionParameter`: the
            // parentheses hug a sole pattern, which breaks inside itself
            // (`function f({` ⏎ `a,` ⏎ `b` ⏎ `}) {`); the parameter
            // threshold has one item to count and does not apply.
            let pad = || Doc::from(if style.padding { " " } else { "" });
            Doc::Concat(vec![
                self.token(params.open.as_ref().expect("parentheses")),
                pad(),
                self.item_content(only, style.key_value, None),
                pad(),
                self.token(params.close.as_ref().expect("parentheses")),
            ])
        } else if params.open.is_some() {
            self.print_delimited(params, &style)
        } else {
            // `a => …`: the bare parameter.
            Doc::Concat(
                params
                    .items
                    .iter()
                    .flat_map(|i| self.sequence(&i.children))
                    .collect(),
            )
        });

        let rest: Vec<&Node> = h.children[params_at + 1..].iter().collect();
        let threshold = Some((
            self.opts.metadata_multiline_element_count,
            self.opts.metadata_multiline_min_item_length,
        ));
        match self.attributes(&rest, threshold, None) {
            Some((doc, id)) => {
                parts.push(doc);
                (Doc::Concat(parts), Some(id))
            }
            None => {
                // Comments, and whatever the parser left after the
                // parameters. The body follows after one space, so a line
                // comment ends the line the header ends on.
                for n in rest {
                    match n {
                        Node::Element(c) if c.kind.is_comment() => {
                            parts.push(self.deferred_comment(c));
                        }
                        n if n.is_trivia() => {}
                        n => {
                            parts.push(Doc::from(" "));
                            parts.push(self.node(n));
                        }
                    }
                }
                (Doc::Concat(parts), None)
            }
        }
    }

    /// A brace-less arrow body: the expression, then what the parser left
    /// in the body after it (the newline that ends the statement,
    /// comments and blank lines before the next one). Comments keep their
    /// lines; the newlines after the last of them are the statement list's,
    /// which counts those a node's text ends with (blank lines included),
    /// and nowhere else a line of their own: before a list's `)` or `,`
    /// they print nothing. An own-line comment is a line suffix
    /// that starts with a line break (Prettier's trailing own-line comment):
    /// whatever follows the arrow on its line — a list's `,`, a call's `)` —
    /// prints before it, never inside the comment. A blank line before it is
    /// not kept: in a list the comment becomes a comment-only item or an
    /// own-line trailing comment on the next parse, and those never keep one.
    pub(crate) fn arrow_body(&self, b: &Element) -> Doc {
        let (mut expr, trailing) = self.arrow_body_parts(b);
        expr.extend(trailing);
        Doc::Concat(expr)
    }

    /// A brace-less arrow body after ` =>` (Prettier's `printArrowFunction`):
    /// `group(indent([line, body]))`, so a body too long for the arrow's
    /// line starts the next one, indented. A hugged last argument
    /// (`hugged`, the group's id) adds `softline` inside the group, which
    /// puts the call's `)` on a line of its own when the body breaks. What the
    /// body's line ends with (comments, the statement's newlines) stays
    /// outside, at the arrow's indent.
    fn arrow_tail(&self, b: &Element, hugged: Option<GroupId>) -> Doc {
        let (expr, trailing) = self.arrow_body_parts(b);
        let mut body = vec![indent(vec![line(), Doc::Concat(expr)])];
        if hugged.is_some() {
            body.push(softline());
        }
        let body = group_opts(
            body,
            GroupOpts {
                id: hugged,
                ..GroupOpts::default()
            },
        );
        Doc::Concat(vec![body, Doc::Concat(trailing)])
    }

    /// [`Printer::arrow_body`] as the expression and what follows it.
    fn arrow_body_parts(&self, b: &Element) -> (Vec<Doc>, Vec<Doc>) {
        let end = b
            .children
            .iter()
            .rposition(|n| !n.is_trivia())
            .map_or(0, |i| i + 1);
        // Prettier's `shouldNotIndent`: a binary that is an arrow's body
        // keeps its continuation lines at the body's indent.
        let expr = self.sequence_by(&b.children[..end], &|n| match n {
            Node::Element(e) if matches!(e.kind, ElementKind::Binary { .. }) => {
                group(self.binary_in_group(e))
            }
            n => self.node(n),
        });
        let mut parts = Vec::new();
        let mut newlines = 0usize;
        for n in &b.children[end..] {
            match n {
                Node::Token(t) if t.kind == TokenKind::Newline => newlines += 1,
                Node::Element(c) if c.kind.is_comment() => {
                    if newlines == 0 {
                        parts.push(self.same_line_comment(c));
                    } else {
                        parts.push(line_suffix(vec![hardline(), self.comment(c)]));
                        parts.push(break_parent());
                    }
                    newlines = 0;
                }
                _ => {}
            }
        }
        (expr, parts)
    }

    /// A `Block`: braced through [`Printer::block`], a Java body verbatim,
    /// a brace-less arrow body through [`Printer::arrow_body`].
    pub(crate) fn any_block(&self, e: &Element) -> Doc {
        match e.kind {
            ElementKind::Block(BlockKind::Java) => self.verbatim(e),
            _ if e.open.is_some() => self.block(e),
            _ => self.arrow_body(e),
        }
    }
}

/// A brace-less arrow body that may start the line after `=>`: anything but
/// a struct, an array (parenthesised or not) or another function, which stay
/// on the arrow's line and break inside (Prettier keeps an object, array or
/// arrow body there).
pub(crate) fn arrow_breaks_after(body: &Element) -> bool {
    if body.open.is_some() || body.kind == ElementKind::Block(BlockKind::Java) {
        return false;
    }
    match body.children.iter().find(|n| !n.is_trivia()) {
        None => false,
        Some(Node::Token(_)) => true,
        Some(Node::Element(e)) => {
            !matches!(
                e.kind,
                ElementKind::Struct { .. } | ElementKind::Array | ElementKind::Function { .. }
            ) && !is_parenthesised_literal(e)
        }
    }
}

/// The one parameter of a parenthesised list that hugs its parentheses: a
/// pattern, or a pattern whose default is a name, `{}` or `[]`, with no
/// comment on it. A comma after it is the list's trailing comma, dropped as
/// a flat list drops it: refusing the hug for it would print the list
/// unhugged once and hugged on the next run.
fn hugged_parameter(params: &Element) -> Option<&Item> {
    if params.open.is_none() || params.close.is_none() {
        return None;
    }
    let [only] = printable_items(params)[..] else {
        return None;
    };
    if has_comments(only) {
        return None;
    }
    let mut sig = only.significant();
    let (Some(Node::Element(p)), None) = (sig.next(), sig.next()) else {
        return None;
    };
    let commented = |e: &Element| {
        e.children
            .iter()
            .any(|n| n.as_element().is_some_and(|c| c.kind.is_comment()))
    };
    let hugs = match p.kind {
        ElementKind::Pattern { .. } => true,
        ElementKind::Assignment if is_pattern_assignment(p) && !commented(p) => {
            let mut value = p.children.iter().filter(|n| !n.is_trivia()).skip(2);
            match (value.next(), value.next()) {
                (Some(Node::Token(t)), None) => matches!(t.kind, TokenKind::Ident(_)),
                (Some(Node::Element(v)), None) => {
                    matches!(
                        v.kind,
                        ElementKind::Struct { ordered: false } | ElementKind::Array
                    ) && !v.items.iter().any(is_printable)
                        && !commented(v)
                }
                _ => false,
            }
        }
        _ => false,
    };
    hugs.then_some(only)
}

/// The expression of a brace-less arrow body, when it is an element.
pub(crate) fn arrow_expression(body: &Element) -> Option<&Element> {
    let mut sig = body.children.iter().filter(|n| !n.is_trivia());
    sig.next().and_then(Node::as_element)
}
