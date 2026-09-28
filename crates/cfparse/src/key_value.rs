//! `KeyValue` elements — `key sep value` in struct members, named
//! arguments, parameter defaults and attributes — built by the front ends
//! as they finish the element that holds them: the script parser for the
//! items and the attribute lists it reads, the tag scanner for a tag's
//! attributes. Two shapes, by where the nodes live:
//!
//! * **Items** ([`items`]: a struct member, a call argument, a parameter, a
//!   `cffile(…)` attribute, a `for` header clause): the first separator of
//!   the item — `Punct(KeyValue)` (`:` or `=`) or `Operator(Assign)` —
//!   preceded by a key: a struct key, argument name, parameter name or
//!   attribute name token, or a `String` / `TemplateExpression` element
//!   (quoted and dynamic struct keys, only with a `Punct(KeyValue)`
//!   separator). The value is everything up to the item's last significant
//!   node, so a parameter's attributes after its default are part of the
//!   default's key-value. Nodes before the key (`required string` before a
//!   parameter) stay in the item. The separator becomes `Punct(KeyValue)`
//!   (text unchanged), so the expression pass treats it as a boundary and
//!   `foo(a = 1)` is not an assignment.
//! * **Attributes** ([`attributes`]: a tag, a declaration header,
//!   `property` / `param` / `http url=…`): `attribute-name = value`, the
//!   value being the one significant node after the separator, whichever
//!   rule read it. A bare attribute (`disabled`) stays a token. In a generic
//!   script tag (`application action="x" mappings=a().b`) a value is an
//!   expression: it also takes every node directly after it, up to
//!   whitespace or the next attribute name. Elsewhere an unquoted value the
//!   scanner split at `#` (`url=www.#host#/x`) takes the unquoted text and
//!   `#…#` directly after it; anything else directly after it (a CF tag
//!   between HTML attributes) stays a sibling. The list is read right to
//!   left, so an attribute written against the value before it
//!   (`name="x"timeout=5`) is already one node when that value takes it.
//!
//! "Significant" is anything but whitespace, newlines and comments
//! ([`Node::is_trivia`]); the trivia between a key-value's first and last
//! part are its children, the trivia around it stay in the list.

use crate::postpass::wrap;
use crate::tree::{Element, ElementKind, Ident, Literal, Node, Operator, Punct, TokenKind};

fn token_kind(node: &Node) -> Option<TokenKind> {
    node.as_token().map(|t| t.kind)
}

fn is_separator(node: &Node) -> bool {
    matches!(
        token_kind(node),
        Some(TokenKind::Punct(Punct::KeyValue) | TokenKind::Operator(Operator::Assign))
    )
}

/// A key that may precede `sep`.
fn is_key(key: &Node, sep: &Node) -> bool {
    match key {
        Node::Token(t) => matches!(
            t.kind,
            TokenKind::Ident(
                Ident::StructKey | Ident::ArgName | Ident::Parameter | Ident::AttributeName
            )
        ),
        Node::Element(e) => {
            matches!(
                e.kind,
                ElementKind::String { .. } | ElementKind::TemplateExpression
            ) && token_kind(sep) == Some(TokenKind::Punct(Punct::KeyValue))
        }
    }
}

/// The key-value of every item of `el` (a call's arguments, a struct's
/// members, a parameter list, a `cffile(…)` attribute list, a `for`
/// header), each item read whole.
pub(crate) fn items(el: &mut Element) {
    for item in el.items.iter_mut() {
        item_key_value(&mut item.children);
    }
}

/// One item's key-value: its first separator, when a key comes right before
/// it and a value after it, through the item's last significant node.
fn item_key_value(nodes: &mut Vec<Node>) {
    let mut key = None;
    let mut sep = None;
    for (i, node) in nodes.iter().enumerate() {
        if node.is_trivia() {
            continue;
        }
        if is_separator(node) {
            sep = Some(i);
            break;
        }
        key = Some(i);
    }
    let (Some(key), Some(sep)) = (key, sep) else {
        return;
    };
    let Some(last) = nodes.iter().rposition(|n| !n.is_trivia()) else {
        return;
    };
    if last == sep || !is_key(&nodes[key], &nodes[sep]) {
        return;
    }
    if let Node::Token(t) = &mut nodes[sep] {
        t.kind = TokenKind::Punct(Punct::KeyValue);
    }
    wrap(nodes, key, last, ElementKind::KeyValue);
}

/// Undo an item's re-kinding in a key-value about to be taken apart (the
/// tag scanner flattens a CF tag read inside an HTML attribute value into
/// its tokens): the `=` after an argument or parameter name, which the
/// script parser reads as `op.assign`, is that again, so the flat tokens
/// are what the parser emitted. Any other separator was read as
/// `punct.key-value` already.
pub(crate) fn read_separator(src: &str, kv: &mut Element) {
    let mut parts = kv.children.iter_mut().filter(|n| !n.is_trivia());
    let key = parts.next().and_then(|n| token_kind(n));
    let Some(Node::Token(sep)) = parts.next() else {
        return;
    };
    let assigned = matches!(
        key,
        Some(TokenKind::Ident(Ident::ArgName | Ident::Parameter))
    ) && &src[sep.span.start as usize..sep.span.end as usize] == "=";
    if assigned {
        sep.kind = TokenKind::Operator(Operator::Assign);
    }
}

/// The attributes among `nodes` (a tag's or a header's children), right to
/// left: every `attribute-name = value` becomes a key-value. `script_tag` is
/// the generic script tag's rule for what joins the value.
pub(crate) fn attributes(nodes: &mut Vec<Node>, script_tag: bool) {
    // The last three significant nodes seen, leftmost first: a candidate
    // name, separator and value.
    let mut window: [Option<usize>; 3] = [None; 3];
    let mut i = nodes.len();
    while i > 0 {
        i -= 1;
        if nodes[i].is_trivia() {
            continue;
        }
        window = [Some(i), window[0], window[1]];
        let [Some(name), Some(sep), Some(value)] = window else {
            continue;
        };
        let is_attr = token_kind(&nodes[name]) == Some(TokenKind::Ident(Ident::AttributeName))
            && token_kind(&nodes[sep]) == Some(TokenKind::Punct(Punct::KeyValue))
            && !matches!(
                token_kind(&nodes[value]),
                Some(TokenKind::Ident(Ident::AttributeName) | TokenKind::Punct(_))
            );
        if !is_attr {
            continue;
        }
        let joins = |n: &Node| {
            if script_tag {
                !n.is_trivia() && token_kind(n) != Some(TokenKind::Ident(Ident::AttributeName))
            } else {
                is_unquoted_part(n) && is_unquoted_part(&nodes[value])
            }
        };
        let mut end = value;
        while nodes
            .get(end + 1)
            .is_some_and(|n| n.span().start == nodes[end].span().end && joins(n))
        {
            end += 1;
        }
        wrap(nodes, name, end, ElementKind::KeyValue);
        // The next value candidate is the significant node before the name.
        window = [None; 3];
    }
}

/// A piece of an unquoted tag attribute value: its text, or a `#…#`.
fn is_unquoted_part(node: &Node) -> bool {
    match node {
        Node::Token(t) => t.kind == TokenKind::Literal(Literal::Unquoted),
        Node::Element(e) => e.kind == ElementKind::TemplateExpression,
    }
}
