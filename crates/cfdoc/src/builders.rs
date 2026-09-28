//! Doc builders (`prettier/src/document/builders/*.js`), snake_cased.
//!
//! Every builder takes `impl Into<Doc>`, so `&'static str`, `String` and
//! `Vec<Doc>` (a concat) can be passed directly.

use std::borrow::Cow;

use crate::doc::{Align, Doc, Group, GroupId, LineKind};

/// A string doc.
pub fn text(s: impl Into<Cow<'static, str>>) -> Doc {
    Doc::Text(s.into())
}

/// A concat (Prettier's array doc).
pub fn concat(parts: Vec<Doc>) -> Doc {
    Doc::Concat(parts)
}

/// Options for [`group_opts`] and [`conditional_group_opts`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupOpts {
    pub id: Option<GroupId>,
    pub should_break: bool,
}

/// `group(doc)`.
pub fn group(contents: impl Into<Doc>) -> Doc {
    group_opts(contents, GroupOpts::default())
}

/// `group(doc, {id, shouldBreak})`.
pub fn group_opts(contents: impl Into<Doc>, opts: GroupOpts) -> Doc {
    Doc::Group(Box::new(Group {
        id: opts.id,
        contents: contents.into(),
        should_break: opts.should_break,
        break_propagated: false,
        expanded_states: None,
    }))
}

/// `conditionalGroup(states)`: prints the first state that fits flat, else
/// the last state in break mode. `contents` is a copy of `states[0]`.
///
/// # Panics
///
/// If `states` is empty.
pub fn conditional_group(states: Vec<Doc>) -> Doc {
    conditional_group_opts(states, GroupOpts::default())
}

/// `conditionalGroup(states, {id, shouldBreak})`.
///
/// # Panics
///
/// If `states` is empty.
pub fn conditional_group_opts(states: Vec<Doc>, opts: GroupOpts) -> Doc {
    assert!(
        !states.is_empty(),
        "conditional_group needs at least one state"
    );
    Doc::Group(Box::new(Group {
        id: opts.id,
        contents: states[0].clone(),
        should_break: opts.should_break,
        break_propagated: false,
        expanded_states: Some(states),
    }))
}

/// cfformat extension: `conditionalGroup([contents])` without the copy a
/// one-state [`conditional_group`] makes — printed flat when it fits, else
/// broken, and never broken by its children (`propagate_breaks`). Stored as
/// a group with empty `expanded_states`, which the printer reads as
/// `[contents]`. A formatter nesting such groups (call arguments hugging a
/// callback that holds more calls) stays linear in the document size.
pub fn conditional_group_contents(contents: impl Into<Doc>) -> Doc {
    Doc::Group(Box::new(Group {
        id: None,
        contents: contents.into(),
        should_break: false,
        break_propagated: false,
        expanded_states: Some(Vec::new()),
    }))
}

/// `indent(doc)`.
pub fn indent(contents: impl Into<Doc>) -> Doc {
    Doc::Indent(Box::new(contents.into()))
}

/// `align(n, doc)`; `n` is a width (`usize`), a string, or an [`Align`].
pub fn align(n: impl Into<Align>, contents: impl Into<Doc>) -> Doc {
    Doc::Align(n.into(), Box::new(contents.into()))
}

/// `dedent(doc)` = `align(-1, doc)`.
pub fn dedent(contents: impl Into<Doc>) -> Doc {
    align(Align::Dedent, contents)
}

/// `dedentToRoot(doc)` = `align(-Infinity, doc)`.
pub fn dedent_to_root(contents: impl Into<Doc>) -> Doc {
    align(Align::DedentToRoot, contents)
}

/// `markAsRoot(doc)` = `align({type: "root"}, doc)`.
pub fn mark_as_root(contents: impl Into<Doc>) -> Doc {
    align(Align::MarkRoot, contents)
}

/// `addAlignmentToDoc(doc, size, tabWidth)`: indent `size` columns from
/// column 0, as whole indents plus an align for the remainder.
pub fn add_alignment_to_doc(doc: impl Into<Doc>, size: usize, tab_width: usize) -> Doc {
    let mut aligned = doc.into();
    if size > 0 {
        for _ in 0..size / tab_width {
            aligned = indent(aligned);
        }
        aligned = align(size % tab_width, aligned);
        aligned = dedent_to_root(aligned);
    }
    aligned
}

/// `line`: a space when flat, a newline when broken.
pub fn line() -> Doc {
    Doc::Line(LineKind::Normal)
}

/// `softline`: nothing when flat, a newline when broken.
pub fn softline() -> Doc {
    Doc::Line(LineKind::Soft)
}

/// `hardline` = `[hardlineWithoutBreakParent, breakParent]`.
pub fn hardline() -> Doc {
    Doc::Concat(vec![Doc::Line(LineKind::Hard), Doc::BreakParent])
}

/// `literalline` = `[literallineWithoutBreakParent, breakParent]`.
pub fn literalline() -> Doc {
    Doc::Concat(vec![Doc::Line(LineKind::Literal), Doc::BreakParent])
}

/// `hardlineWithoutBreakParent`.
pub fn hardline_without_break_parent() -> Doc {
    Doc::Line(LineKind::Hard)
}

/// `literallineWithoutBreakParent`.
pub fn literalline_without_break_parent() -> Doc {
    Doc::Line(LineKind::Literal)
}

/// `ifBreak(breakContents, flatContents)`, keyed on the enclosing group.
pub fn if_break(break_doc: impl Into<Doc>, flat_doc: impl Into<Doc>) -> Doc {
    Doc::IfBreak {
        break_doc: Box::new(break_doc.into()),
        flat_doc: Box::new(flat_doc.into()),
        group_id: None,
    }
}

/// `ifBreak(breakContents, flatContents, {groupId})`, keyed on the group
/// with that id (which must already have been printed, otherwise neither
/// side prints).
pub fn if_break_group(break_doc: impl Into<Doc>, flat_doc: impl Into<Doc>, id: GroupId) -> Doc {
    Doc::IfBreak {
        break_doc: Box::new(break_doc.into()),
        flat_doc: Box::new(flat_doc.into()),
        group_id: Some(id),
    }
}

/// `indentIfBreak(doc, {groupId, negate})`.
pub fn indent_if_break(contents: impl Into<Doc>, id: GroupId, negate: bool) -> Doc {
    Doc::IndentIfBreak {
        contents: Box::new(contents.into()),
        group_id: id,
        negate,
    }
}

/// `fill(parts)`: `parts` alternate content and separator.
pub fn fill(parts: Vec<Doc>) -> Doc {
    Doc::Fill(parts)
}

/// `lineSuffix(doc)`: deferred until the next newline.
pub fn line_suffix(contents: impl Into<Doc>) -> Doc {
    Doc::LineSuffix(Box::new(contents.into()))
}

/// `lineSuffixBoundary`: flushes pending line suffixes with a hard line.
pub fn line_suffix_boundary() -> Doc {
    Doc::LineSuffixBoundary
}

/// `breakParent`: breaks every enclosing (non-conditional) group.
pub fn break_parent() -> Doc {
    Doc::BreakParent
}

/// `join(separator, docs)`; the separator is cloned between items.
pub fn join(separator: impl Into<Doc>, docs: impl IntoIterator<Item = Doc>) -> Doc {
    let separator = separator.into();
    let mut parts = Vec::new();
    for (i, doc) in docs.into_iter().enumerate() {
        if i != 0 {
            parts.push(separator.clone());
        }
        parts.push(doc);
    }
    Doc::Concat(parts)
}
