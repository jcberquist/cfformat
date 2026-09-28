//! Comment attachment for delimited items: line comments around
//! an [`Item`] move into its `leading` / `trailing` so a printer can emit them
//! as line suffixes and break the group.
//!
//! Two runs move: a line comment right after a separator belongs to the item
//! before it, and line comments after an item's content belong to that item,
//! including `3 // c\n, 4`, where printing the comment in place would emit
//! `3 // c,`. The nodes move once, after the tree is built, so no printer has
//! to pick the comments out while printing.
//!
//! "Significant" is [`Node::is_significant`]: anything but whitespace,
//! newlines and line comments. Block and doc comments stay in `children`
//! (`2 /* b */` prints inline), and only runs that contain a line comment
//! move, so pure whitespace runs stay where they are.

use crate::tree::{Element, ElementKind, Item, Node, TokenKind};

/// Attach line comments to the items of every delimited element in `el` and
/// below (structs, arrays, call arguments, parameters, script tag attributes,
/// `for (;;)` headers). Run by [`parse_source`](crate::parse_source) after tag
/// pairing.
pub fn attach_item_comments(el: &mut Element) {
    for node in el.children.iter_mut() {
        if let Node::Element(child) = node {
            attach_item_comments(child);
        }
    }
    for item in el.items.iter_mut() {
        for node in item.nodes_mut() {
            if let Node::Element(child) = node {
                attach_item_comments(child);
            }
        }
    }
    if !el.items.is_empty() {
        attach(&mut el.items);
    }
}

fn attach(items: &mut Vec<Item>) {
    for i in 0..items.len() {
        // `if (i > 1 && peekLineComment(element))`: a line comment on the
        // separator's line belongs to the previous item.
        if i > 0 {
            let n = after_separator_run(&items[i].children);
            if n > 0 {
                let run: Vec<Node> = items[i].children.drain(..n).collect();
                items[i - 1].trailing.extend(run);
            }
        }

        let item = &mut items[i];
        // An item that ends with a line comment: the run after its last
        // significant child moves to `trailing`.
        let start = item
            .children
            .iter()
            .rposition(Node::is_significant)
            .map_or(0, |p| p + 1);
        if has_line_comment(&item.children[start..]) {
            let run = item.children.split_off(start);
            item.trailing.extend(run);
        }

        // Own-line comments before the first significant child.
        let end = item
            .children
            .iter()
            .position(Node::is_significant)
            .unwrap_or(item.children.len());
        if has_line_comment(&item.children[..end]) {
            item.leading = item.children.drain(..end).collect();
        }
    }

    // A dangling last item whose only content moved to the previous item's
    // `trailing` has nothing left, not even a separator, and is removed.
    if items.last().is_some_and(|i| {
        i.children.is_empty()
            && i.leading.is_empty()
            && i.trailing.is_empty()
            && i.separator.is_none()
    }) {
        items.pop();
    }
}

/// Length of the head run `[whitespace] line-comment … newline` at the start
/// of an item, or 0 when the item does not start with a same-line comment.
fn after_separator_run(nodes: &[Node]) -> usize {
    let mut seen = false;
    for (i, node) in nodes.iter().enumerate() {
        match node {
            Node::Token(t) if t.kind == TokenKind::Newline => return if seen { i + 1 } else { 0 },
            Node::Token(t) if t.kind == TokenKind::Whitespace => {}
            Node::Element(e) if e.kind == ElementKind::LineComment => seen = true,
            _ => return 0,
        }
    }
    if seen {
        nodes.len()
    } else {
        0
    }
}

fn has_line_comment(nodes: &[Node]) -> bool {
    nodes
        .iter()
        .any(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::LineComment))
}
