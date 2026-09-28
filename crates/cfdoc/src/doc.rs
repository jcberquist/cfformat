//! The document IR (`prettier/src/document/builders/*.js`).
//!
//! Prettier's docs are plain JS values (strings, arrays, `{type, …}` objects)
//! that may share subtrees. Here a [`Doc`] is an owned tree: builders move
//! their arguments in, and the pre-pass of [`crate::print_doc`]
//! (`propagate_breaks`) mutates it in place.

use std::borrow::Cow;

/// Identifies a group so `if_break` / `indent_if_break` can ask how it was
/// printed. Ids index a vector in the printer, so allocate them densely from
/// a per-document [`GroupIdGen`].
pub type GroupId = u32;

/// Allocates [`GroupId`]s for one document, starting at 0.
#[derive(Clone, Debug, Default)]
pub struct GroupIdGen {
    next: GroupId,
}

impl GroupIdGen {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a fresh id.
    pub fn next_id(&mut self) -> GroupId {
        let id = self.next;
        self.next += 1;
        id
    }
}

/// A document node.
#[derive(Clone, Debug, PartialEq)]
pub enum Doc {
    /// A string; `""` is the empty doc.
    Text(Cow<'static, str>),
    /// A JS array.
    Concat(Vec<Doc>),
    Indent(Box<Doc>),
    Align(Align, Box<Doc>),
    Group(Box<Group>),
    /// Alternating content / separator parts.
    Fill(Vec<Doc>),
    IfBreak {
        break_doc: Box<Doc>,
        flat_doc: Box<Doc>,
        group_id: Option<GroupId>,
    },
    IndentIfBreak {
        contents: Box<Doc>,
        group_id: GroupId,
        negate: bool,
    },
    LineSuffix(Box<Doc>),
    LineSuffixBoundary,
    /// A line break. `Hard` and `Literal` do not carry a `BreakParent`
    /// themselves; [`crate::builders::hardline`] adds one.
    Line(LineKind),
    BreakParent,
}

/// The `n` of Prettier's `align(n, doc)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Align {
    /// `align(n)` with `n >= 0`: `n` extra spaces (a tab each under tabs, see
    /// `indent.rs`). `Width(0)` is a no-op.
    Width(usize),
    /// `align("…")`: a literal prefix. `Str("")` is a no-op.
    Str(Cow<'static, str>),
    /// `dedent` (`align(-1)`): drop the innermost indent or align.
    Dedent,
    /// `dedentToRoot` (`align(-Infinity)`): back to the nearest `MarkRoot`.
    DedentToRoot,
    /// `markAsRoot` (`align({type: "root"})`).
    MarkRoot,
}

impl From<usize> for Align {
    fn from(n: usize) -> Self {
        Align::Width(n)
    }
}

impl From<&'static str> for Align {
    fn from(s: &'static str) -> Self {
        Align::Str(Cow::Borrowed(s))
    }
}

impl From<String> for Align {
    fn from(s: String) -> Self {
        Align::Str(Cow::Owned(s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// `line`: a space when flat.
    Normal,
    /// `softline`: nothing when flat.
    Soft,
    /// `hardlineWithoutBreakParent`: always a newline.
    Hard,
    /// `literallineWithoutBreakParent`: a newline without indentation (only
    /// the root indent, see `markAsRoot`).
    Literal,
}

/// Prettier's group object.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: Option<GroupId>,
    /// For a conditional group this is a copy of `expanded_states[0]`
    /// (Prettier shares the object; see `conditional_group`).
    pub contents: Doc,
    /// Set by the builder or by `propagate_breaks`.
    pub should_break: bool,
    /// `true` when `should_break` was set by `propagate_breaks` (Prettier's
    /// `break: "propagated"`); only `debug::format_doc` looks at it.
    pub break_propagated: bool,
    /// `conditionalGroup` states, least to most expanded.
    pub expanded_states: Option<Vec<Doc>>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc::Text(Cow::Borrowed(""))
    }
}

impl From<&'static str> for Doc {
    fn from(s: &'static str) -> Self {
        Doc::Text(Cow::Borrowed(s))
    }
}

impl From<String> for Doc {
    fn from(s: String) -> Self {
        Doc::Text(Cow::Owned(s))
    }
}

impl From<Cow<'static, str>> for Doc {
    fn from(s: Cow<'static, str>) -> Self {
        Doc::Text(s)
    }
}

impl From<Vec<Doc>> for Doc {
    fn from(parts: Vec<Doc>) -> Self {
        Doc::Concat(parts)
    }
}

impl From<Group> for Doc {
    fn from(group: Group) -> Self {
        Doc::Group(Box::new(group))
    }
}

impl Doc {
    /// The empty doc, `""`.
    pub const fn empty() -> Doc {
        Doc::Text(Cow::Borrowed(""))
    }

    /// `true` when this is `Text("")` (Prettier's falsy doc, as opposed to
    /// [`Doc::is_empty`]).
    pub fn is_empty_text(&self) -> bool {
        matches!(self, Doc::Text(t) if t.is_empty())
    }

    /// `isEmptyDoc` (`utilities/index.js:392`): `true` iff the doc prints
    /// nothing — only empty strings, with no lines, boundaries or break
    /// parents anywhere (groups are looked through, conditional groups via
    /// their `contents`).
    pub fn is_empty(&self) -> bool {
        let mut empty = true;
        self.walk(&mut |d| match d {
            Doc::Text(t) if t.is_empty() => true,
            Doc::Text(_) | Doc::LineSuffixBoundary | Doc::Line(_) | Doc::BreakParent => {
                empty = false;
                false
            }
            _ => empty,
        });
        empty
    }

    /// `traverseDoc(doc, onEnter)` (`utilities/traverse-doc.js`): pre-order,
    /// children in document order; returning `false` skips the node's
    /// children. Groups are entered through `contents` only, like Prettier
    /// without `shouldTraverseConditionalGroups`.
    pub fn walk<'a>(&'a self, enter: &mut impl FnMut(&'a Doc) -> bool) {
        if !enter(self) {
            return;
        }
        match self {
            Doc::Concat(parts) | Doc::Fill(parts) => {
                for part in parts {
                    part.walk(enter);
                }
            }
            Doc::IfBreak {
                break_doc,
                flat_doc,
                ..
            } => {
                break_doc.walk(enter);
                flat_doc.walk(enter);
            }
            Doc::Group(group) => group.contents.walk(enter),
            Doc::Indent(contents)
            | Doc::Align(_, contents)
            | Doc::IndentIfBreak { contents, .. }
            | Doc::LineSuffix(contents) => contents.walk(enter),
            Doc::Text(_) | Doc::LineSuffixBoundary | Doc::Line(_) | Doc::BreakParent => {}
        }
    }

    /// Mutable pre-order traversal; returning `false` skips the node's
    /// children. Unlike [`Doc::walk`], a conditional group's `contents` *and*
    /// every expanded state are visited, so edits keep the `contents` copy in
    /// step with `expanded_states[0]` (Prettier shares the object).
    pub fn walk_mut(&mut self, enter: &mut impl FnMut(&mut Doc) -> bool) {
        if !enter(self) {
            return;
        }
        match self {
            Doc::Concat(parts) | Doc::Fill(parts) => {
                for part in parts {
                    part.walk_mut(enter);
                }
            }
            Doc::IfBreak {
                break_doc,
                flat_doc,
                ..
            } => {
                break_doc.walk_mut(enter);
                flat_doc.walk_mut(enter);
            }
            Doc::Group(group) => {
                group.contents.walk_mut(enter);
                if let Some(states) = &mut group.expanded_states {
                    for state in states {
                        state.walk_mut(enter);
                    }
                }
            }
            Doc::Indent(contents)
            | Doc::Align(_, contents)
            | Doc::IndentIfBreak { contents, .. }
            | Doc::LineSuffix(contents) => contents.walk_mut(enter),
            Doc::Text(_) | Doc::LineSuffixBoundary | Doc::Line(_) | Doc::BreakParent => {}
        }
    }

    /// `mapDoc(doc, cb)` (`utilities/index.js:30`): rebuilds the tree
    /// bottom-up, calling `f` on every node after its children were mapped.
    /// For a conditional group the states are mapped and `contents` becomes a
    /// copy of the mapped first state, as in Prettier — except for a
    /// [`crate::builders::conditional_group_contents`] group, whose states are
    /// empty because `contents` *is* its one state.
    pub fn map(self, f: &mut impl FnMut(Doc) -> Doc) -> Doc {
        let mapped = match self {
            Doc::Concat(parts) => Doc::Concat(parts.into_iter().map(|p| p.map(f)).collect()),
            Doc::Fill(parts) => Doc::Fill(parts.into_iter().map(|p| p.map(f)).collect()),
            Doc::IfBreak {
                break_doc,
                flat_doc,
                group_id,
            } => Doc::IfBreak {
                break_doc: Box::new(break_doc.map(f)),
                flat_doc: Box::new(flat_doc.map(f)),
                group_id,
            },
            Doc::Group(mut group) => {
                match group.expanded_states.take() {
                    Some(states) if !states.is_empty() => {
                        let states: Vec<Doc> = states.into_iter().map(|s| s.map(f)).collect();
                        group.contents = states[0].clone();
                        group.expanded_states = Some(states);
                    }
                    expanded => {
                        group.contents = std::mem::take(&mut group.contents).map(f);
                        group.expanded_states = expanded;
                    }
                }
                Doc::Group(group)
            }
            Doc::Indent(contents) => Doc::Indent(Box::new(contents.map(f))),
            Doc::Align(align, contents) => Doc::Align(align, Box::new(contents.map(f))),
            Doc::IndentIfBreak {
                contents,
                group_id,
                negate,
            } => Doc::IndentIfBreak {
                contents: Box::new(contents.map(f)),
                group_id,
                negate,
            },
            Doc::LineSuffix(contents) => Doc::LineSuffix(Box::new(contents.map(f))),
            leaf @ (Doc::Text(_) | Doc::LineSuffixBoundary | Doc::Line(_) | Doc::BreakParent) => {
                leaf
            }
        };
        f(mapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Doc>();
    }

    #[test]
    fn group_ids_are_dense() {
        let mut ids = GroupIdGen::new();
        assert_eq!([ids.next_id(), ids.next_id(), ids.next_id()], [0, 1, 2]);
    }
}
