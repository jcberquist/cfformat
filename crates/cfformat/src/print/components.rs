//! Components, interfaces, static blocks, `import`, `property`, `param` and
//! tags in script.
//!
//! Each is `[words, attributes]`: the tokens before the first attribute
//! joined by one space (`abstract component`, `property string test`,
//! `param numeric rc.test = 23`), then an attribute group
//! ([`Printer::attributes`]) with the construct's thresholds. A component's
//! `{` follows after one space, or on its own line when the attributes break;
//! the statement's `;` follows the last attribute (`setter="false";`), and a
//! block after a script tag is the statement's next child (`lock name="x" {`).
//! The attributes come already structured from the tree, as `KeyValue` and
//! `ScriptTagAttributes` elements, so nothing here regroups tokens.

use cfdoc::builders::{hardline_without_break_parent, if_break_group};
use cfdoc::{Doc, GroupId};
use cfparse::{Element, ElementKind, Node, TokenKind};

use super::attributes::AttributeThreshold;
use super::delimited::DelimitedStyle;
use super::Printer;

impl Printer<'_> {
    /// Nodes up to the first attribute through [`Printer::sequence`], the
    /// rest as an attribute group (with the alignment padding `columns` of an
    /// attribute run), and that group's id.
    pub(crate) fn words_and_attributes(
        &self,
        nodes: &[Node],
        threshold: AttributeThreshold,
        columns: Option<&[usize]>,
    ) -> (Doc, Option<GroupId>) {
        let Some(first) = nodes.iter().position(Self::is_attribute) else {
            return (Doc::Concat(self.sequence(nodes)), None);
        };
        let mut parts = self.sequence(&nodes[..first]);
        let rest: Vec<&Node> = nodes[first..].iter().collect();
        let (attrs, id) = self
            .attributes(&rest, threshold, columns)
            .expect("an attribute");
        parts.push(attrs);
        (Doc::Concat(parts), Some(id))
    }

    /// `component … { }` / `interface … { }`: the header with the metadata
    /// thresholds, then the class block (a blank line inside each brace), on
    /// the header's line or, when the attributes break, on the next.
    pub(crate) fn class(&self, e: &Element) -> Doc {
        let Some(decl) = e.as_decl() else {
            return self.as_written(e);
        };
        let body = decl.body();
        let mut parts = Vec::new();
        let mut attributes = None;
        for n in &e.children {
            match n {
                Node::Element(c) if c.kind.is_comment() => parts.push(self.same_line_comment(c)),
                n if n.is_trivia() => {}
                Node::Element(h)
                    if matches!(h.kind, ElementKind::ClassDecl | ElementKind::InterfaceDecl) =>
                {
                    let (doc, id) = self.class_header(h);
                    parts.push(doc);
                    attributes = id;
                }
                Node::Element(b) if std::ptr::eq(&**b, body) => {
                    parts.push(match attributes {
                        Some(id) => if_break_group(hardline_without_break_parent(), " ", id),
                        None => Doc::from(" "),
                    });
                    parts.push(self.element(b));
                }
                n => parts.push(self.node(n)),
            }
        }
        Doc::Concat(parts)
    }

    /// A `ClassDecl` / `InterfaceDecl`: modifiers, keyword, metadata.
    pub(crate) fn class_header(&self, h: &Element) -> (Doc, Option<GroupId>) {
        self.words_and_attributes(
            &h.children,
            Some((
                self.opts.metadata_multiline_element_count,
                self.opts.metadata_multiline_min_item_length,
            )),
            None,
        )
    }

    /// `static { … }`.
    pub(crate) fn static_block(&self, e: &Element) -> Doc {
        Doc::Concat(self.sequence(&e.children))
    }

    /// `import a.b.*`: the keyword, one space, the path as written. The path
    /// is one token (or a string) except for a trailing star, which is a
    /// token of its own (`a.b.` and `*`); printing the slice from the path's
    /// first token to its last keeps it glued. Comments between the keyword
    /// and the path print as [`Printer::sequence`] prints them anywhere else:
    /// a block comment in place, a line comment at the end of the keyword's
    /// line with the path starting the next. After the path, a block comment
    /// stays before the `;` (`import a.b /* c */;`); from the first line
    /// comment on, the comments are the statement's to print after its `;`
    /// ([`import_trailing`]), as any statement's trailing comments are
    /// (`import a.b; // c`): printed here, the line comment would end the
    /// line and leave the `;` alone on the next.
    pub(crate) fn import(&self, e: &Element) -> Doc {
        let mut sig = e
            .children
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.is_trivia());
        if sig.next().is_none() {
            return self.as_written(e);
        }
        let Some((path, first)) = sig.next() else {
            return Doc::Concat(self.sequence(&e.children));
        };
        let last = import_path_end(e).unwrap_or(path);
        let end = e.children[last].span().end;
        let mut parts = self.sequence_by(&e.children[..=path], &|n| {
            if std::ptr::eq(n, first) {
                self.text(self.tree.slice(first.span().start..end))
            } else {
                self.node(n)
            }
        });
        parts.extend(
            e.children[last + 1..import_trailing(e)]
                .iter()
                .filter_map(Node::as_element)
                .filter(|c| c.kind.is_comment())
                .map(|c| self.same_line_comment(c)),
        );
        Doc::Concat(parts)
    }

    /// `property …` (property thresholds), `param …` (param thresholds) and
    /// `http url="x" …` (no thresholds: it breaks on width only).
    pub(crate) fn tag_statement(&self, e: &Element) -> Doc {
        self.tag_statement_with(e, None)
    }

    /// [`Printer::tag_statement`] with the attribute alignment padding of
    /// its run (see [`Printer::attribute_group`]).
    pub(crate) fn tag_statement_with(&self, e: &Element, columns: Option<&[usize]>) -> Doc {
        let threshold = match e.kind {
            ElementKind::Property => Some((
                self.opts.property_multiline_element_count,
                self.opts.property_multiline_min_item_length,
            )),
            ElementKind::Param => Some((
                self.opts.param_multiline_element_count,
                self.opts.param_multiline_min_item_length,
            )),
            _ => None,
        };
        // Comments after the last attribute are followed by `;` or ` {`,
        // not by a line break: a line comment there ends the statement's
        // line, outside the attribute group, which it would otherwise break
        // for nothing.
        let end = e
            .children
            .iter()
            .rposition(|n| !n.is_trivia())
            .map_or(0, |i| i + 1);
        let (nodes, trailing) = e.children.split_at(end);
        let mut parts = vec![self.words_and_attributes(nodes, threshold, columns).0];
        parts.extend(
            trailing
                .iter()
                .filter_map(Node::as_element)
                .filter(|c| c.kind.is_comment())
                .map(|c| self.deferred_comment(c)),
        );
        Doc::Concat(parts)
    }

    /// `cfhttp(url = "x")`: the tag name and its attributes through the
    /// delimited printer with the call thresholds, every attribute
    /// `key = value`. A comment before the attributes is followed by them, not
    /// by a line break: a block comment keeps a space on both sides
    /// (`cfhttp /* c */ (…)`), a line comment ends the line the statement ends
    /// on ([`Printer::deferred_comment`]).
    pub(crate) fn acf_script_tag(&self, e: &Element) -> Doc {
        let mut parts = Vec::new();
        let mut after_block_comment = false;
        for n in &e.children {
            match n {
                Node::Element(c) if c.kind.is_comment() => {
                    parts.push(self.deferred_comment(c));
                    after_block_comment = c.kind != ElementKind::LineComment;
                }
                Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline) => {}
                n => {
                    if std::mem::take(&mut after_block_comment) {
                        parts.push(Doc::from(" "));
                    }
                    parts.push(self.node(n));
                }
            }
        }
        Doc::Concat(parts)
    }

    /// `ScriptTagAttributes`: `( … )` with the call thresholds, no hugging.
    pub(crate) fn script_tag_attributes(&self, e: &Element) -> Doc {
        self.print_delimited(e, &DelimitedStyle::script_tag(self.opts))
    }
}

/// The index of an `Import`'s last child that is neither trivia nor a
/// comment, the end of its path; `None` when it has only its keyword.
fn import_path_end(e: &Element) -> Option<usize> {
    let mut sig =
        e.children.iter().enumerate().filter(|(_, n)| {
            !n.is_trivia() && !n.as_element().is_some_and(|c| c.kind.is_comment())
        });
    sig.next()?;
    sig.next_back().map(|(i, _)| i)
}

/// Where the comments an `Import` leaves to its statement start: its first
/// line comment after the path, with the whitespace and line breaks before
/// it (they tell a comment on a line of its own from one on the path's
/// line); the end of its children when there is none, or no path. A line
/// comment ends its line, so it and whatever follows it print after the
/// statement's `;` ([`Printer::import`]).
pub(crate) fn import_trailing(e: &Element) -> usize {
    let Some(path) = import_path_end(e) else {
        return e.children.len();
    };
    let Some(comment) = e.children[path + 1..]
        .iter()
        .position(|n| {
            n.as_element()
                .is_some_and(|c| c.kind == ElementKind::LineComment)
        })
        .map(|i| path + 1 + i)
    else {
        return e.children.len();
    };
    let spaces = e.children[path + 1..comment]
        .iter()
        .rev()
        .take_while(|n| {
            matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline))
        })
        .count();
    comment - spaces
}
