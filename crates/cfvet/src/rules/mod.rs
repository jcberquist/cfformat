//! The rules, and the tree readers they share.

pub(crate) mod missing_var;
pub(crate) mod result_attributes;
pub(crate) mod unevaluated_call;

use cfparse::nodes::plain_text;
use cfparse::{Element, ElementKind, Node, Token, Tree};

/// The `name=value` attributes of a CF tag, a script tag (`http url="x";`
/// or `cfhttp(url="x")`) or a `param`: each attribute's name token and its
/// value nodes.
pub(crate) fn attributes(el: &Element) -> impl Iterator<Item = (&Token, &[Node])> {
    let direct = el.children.iter();
    let wrapped = el
        .children
        .iter()
        .filter_map(Node::as_element)
        .filter(|e| e.kind == ElementKind::ScriptTagAttributes)
        .flat_map(|e| e.items.iter().flat_map(|item| item.children.iter()));
    direct.chain(wrapped).filter_map(|node| {
        let kv = node.as_element()?.as_key_value()?;
        Some((kv.key().as_token()?, kv.value()))
    })
}

/// A tag's attribute `name` (any case) when its value is plain text
/// ([`plain_text`]): the text and the offset where it starts.
pub(crate) fn attribute<'a>(tree: &'a Tree, el: &'a Element, name: &str) -> Option<(&'a str, u32)> {
    attributes(el)
        .find(|(key, _)| tree.text(key).eq_ignore_ascii_case(name))
        .and_then(|(_, value)| plain_text(tree, value))
}
