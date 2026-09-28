//! The `cfvet-ignore` comment.

use std::collections::HashSet;

use cfparse::{Element, Node, Tree};

/// The token a comment holds to silence the reports on its own line and on
/// the line after it.
pub(crate) const IGNORE: &str = "cfvet-ignore";

/// The lines a `cfvet-ignore` comment silences: every line the comment
/// covers, and the one after it. Any comment form counts (`//`, `/* */`, a
/// doc comment, `<!--- --->`).
pub(crate) fn suppressed_lines(tree: &Tree) -> HashSet<usize> {
    fn find(tree: &Tree, el: &Element, out: &mut HashSet<usize>) {
        if el.kind.is_comment() {
            if tree.slice(el.span.clone()).contains(IGNORE) {
                let first = tree.line_of(el.span.start);
                let last = tree.line_of(el.span.end.saturating_sub(1).max(el.span.start));
                out.extend(first..=last + 1);
            }
            return;
        }
        for node in el.nodes() {
            if let Node::Element(e) = node {
                find(tree, e, out);
            }
        }
    }
    let mut out = HashSet::new();
    find(tree, &tree.root, &mut out);
    out
}
