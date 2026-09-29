//! Attribute lists after a declaration header or a tag name: function
//! metadata, component attributes, `property`, `param` and `http …;`.
//!
//! The attributes are the tree's `KeyValue` elements (and bare attribute
//! names); one group holds them, and it breaks on width or on the construct's
//! thresholds.

use cfdoc::builders::{group_opts, indent, line, GroupOpts};
use cfdoc::utils::flat_width;
use cfdoc::{Doc, GroupId};
use cfparse::{ElementKind, Node, TokenKind};

use super::alignment::{self, RunPart};
use super::delimited::{threshold_breaks, KeyValueStyle};
use super::Printer;

/// `(element_count, min_item_length)` of an attribute list
/// ([`threshold_breaks`]); `None` breaks on width only (`http url=… ;`).
pub(crate) type AttributeThreshold = Option<(u32, u32)>;

impl Printer<'_> {
    /// Whether `n` is an attribute: a `KeyValue` or a bare attribute name.
    pub(crate) fn is_attribute(n: &Node) -> bool {
        match n {
            Node::Element(e) => e.kind == ElementKind::KeyValue,
            Node::Token(t) => matches!(t.kind, TokenKind::Ident(cfparse::Ident::AttributeName)),
        }
    }

    /// `group(indent([line, attr, line, attr …]))` with a group id: one space
    /// before each attribute when flat, each on its own line indented once
    /// when broken. Returns `None` when `nodes` holds no attribute.
    pub(crate) fn attributes(
        &self,
        nodes: &[&Node],
        threshold: AttributeThreshold,
        columns: Option<&[usize]>,
    ) -> Option<(Doc, GroupId)> {
        self.attribute_group(
            nodes,
            threshold,
            KeyValueStyle::Attribute,
            true,
            columns,
            None,
        )
    }

    /// [`Printer::attributes`]. A `KeyValue` prints in `style`: `key=value`,
    /// or `key = value` for a script attribute with
    /// `attributes.key_value.padding` ([`KeyValueStyle`]); a bare attribute as its
    /// name, a comment in place. Any other node is an entry of its own on its
    /// own `line` (a CF tag or `#expr#` between two HTML attributes), unless
    /// it sits directly against what precedes it (`class=a<cfif x> b</cfif>`),
    /// which stays glued: whitespace would end the value before it. `after`
    /// is where the source ends before the first node (a tag's name), so a
    /// node written against the name stays glued to it too
    /// (`<td<cfif x> class="a"</cfif>>`): a space there would print on the
    /// page beside the one the CF body already holds. The threshold measures
    /// the attributes joined by one space, without the space before the first
    /// one. With `alignment.consecutive.assignments`,
    /// consecutive `KeyValue` attributes pad their keys to the widest when the
    /// group breaks (`name     ="name"`); a bare attribute or glued text ends a
    /// run, a comment does not. `any` returns a group even when there is no
    /// attribute at all, which is what an HTML tag needs.
    ///
    /// `columns` is a statement's share of an attribute run
    /// (`alignment.consecutive.properties` / `.params`): the spaces after
    /// each attribute, in order, that align it with the same attribute of the
    /// statements around it. They print only when the group is flat, so a
    /// statement printed one attribute per line is never padded, and they
    /// count against `max_columns`, so padding that would not fit breaks the
    /// statement instead of overflowing the line. The thresholds still
    /// measure the attributes as written, never the padding, so whether a
    /// statement breaks by threshold does not depend on its neighbours.
    pub(crate) fn attribute_group(
        &self,
        nodes: &[&Node],
        threshold: AttributeThreshold,
        style: KeyValueStyle,
        require_attribute: bool,
        columns: Option<&[usize]>,
        after: Option<u32>,
    ) -> Option<(Doc, GroupId)> {
        let count = nodes.iter().filter(|n| Self::is_attribute(n)).count();
        // A comment counts: `<p <!--- c --->>` keeps it.
        let printable = nodes
            .iter()
            .any(|n| !n.is_trivia() || n.as_element().is_some_and(|c| c.kind.is_comment()));
        if if require_attribute {
            count == 0
        } else {
            !printable
        } {
            return None;
        }
        let unclassified = |n: &Node| {
            n.as_token()
                .is_some_and(|t| matches!(t.kind, TokenKind::Other | TokenKind::Invalid))
        };
        let pads = if self.opts.alignment_consecutive_assignments {
            alignment::runs(nodes, |n| match n {
                n if n.is_trivia() => RunPart::Neutral,
                Node::Element(kv)
                    if kv.kind == ElementKind::KeyValue
                        && !kv
                            .children
                            .iter()
                            .any(|c| c.as_element().is_some_and(|c| c.kind.is_comment())) =>
                {
                    kv.as_key_value().map_or(RunPart::Break, |view| {
                        alignment::member(self.node(view.key()))
                    })
                }
                _ => RunPart::Break,
            })
        } else {
            vec![None; nodes.len()]
        };
        let id = self.ids.borrow_mut().next_id();
        let column_pad = |k: usize| {
            columns
                .and_then(|c| c.get(k))
                .filter(|&&p| p > 0)
                .map(|&p| alignment::pad_if_flat(p, id))
        };
        let mut attribute = 0;
        // The attributes' flat widths, for a threshold that can fire only:
        // none for HTML and CF tags, and none while its count is 0.
        let measure = threshold.is_some_and(|(element_count, _)| element_count > 0);
        let mut widths = Vec::new();
        let mut parts = Vec::new();
        let mut prev: Option<&Node> = None;
        // Where the source ends before `n`: the previous printed node, or
        // `after` for the first one.
        let mut prev_end = after;
        for (i, &n) in nodes.iter().enumerate() {
            match n {
                Node::Element(c) if c.kind.is_comment() => parts.push(self.same_line_comment(c)),
                n if n.is_trivia() => continue,
                // Text the parser could not classify prints as written.
                n if prev.is_some_and(|p| unclassified(p) || unclassified(n)) => {
                    if prev.is_some_and(|p| p.span().end < n.span().start) {
                        parts.push(Doc::from(" "));
                    }
                    parts.push(self.node(n));
                }
                Node::Element(kv) if kv.kind == ElementKind::KeyValue => {
                    parts.push(line());
                    let pad = pads[i].map(|p| alignment::pad_if_break(p, id));
                    let doc = self.key_value_with(kv, style, pad);
                    if measure {
                        widths.push(flat_width(&doc));
                    }
                    parts.push(doc);
                    parts.extend(column_pad(attribute));
                    attribute += 1;
                }
                Node::Token(t) if Self::is_attribute(n) => {
                    parts.push(line());
                    let doc = self.token(t);
                    if measure {
                        widths.push(flat_width(&doc));
                    }
                    parts.push(doc);
                    parts.extend(column_pad(attribute));
                    attribute += 1;
                }
                // A node written against the one before it stays glued;
                // anything else is an entry of its own.
                n => {
                    if prev_end != Some(n.span().start) {
                        parts.push(line());
                    }
                    parts.push(match n {
                        Node::Element(s)
                            if style == KeyValueStyle::HtmlAttribute
                                && matches!(s.kind, ElementKind::String { .. }) =>
                        {
                            self.html_string(s)
                        }
                        n => self.node(n),
                    });
                }
            }
            prev = Some(n);
            prev_end = Some(n.span().end);
        }
        let should_break = threshold.is_some_and(|(element_count, min_item_length)| {
            threshold_breaks(element_count, min_item_length, &widths)
        });
        let doc = group_opts(
            indent(parts),
            GroupOpts {
                id: Some(id),
                should_break,
            },
        );
        Some((doc, id))
    }
}
