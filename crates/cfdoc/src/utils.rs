//! Doc utilities (`prettier/src/document/utilities/index.js`).

use crate::builders::join;
use crate::doc::{Doc, LineKind};
use crate::width::str_width;

/// `findInDoc(doc, fn, defaultValue)`: the first `Some` returned by `f` in a
/// pre-order walk (groups through `contents` only).
pub fn find_in_doc<'a, T>(doc: &'a Doc, mut f: impl FnMut(&'a Doc) -> Option<T>) -> Option<T> {
    let mut result = None;
    doc.walk(&mut |d| {
        if result.is_some() {
            return false;
        }
        result = f(d);
        true
    });
    result
}

/// `willBreak`: contains a broken group, a hard (or literal) line or a
/// `BreakParent`.
pub fn will_break(doc: &Doc) -> bool {
    find_in_doc(doc, |d| match d {
        Doc::Group(g) if g.should_break => Some(()),
        Doc::Line(LineKind::Hard | LineKind::Literal) | Doc::BreakParent => Some(()),
        _ => None,
    })
    .is_some()
}

/// `canBreak`: contains any line.
pub fn can_break(doc: &Doc) -> bool {
    find_in_doc(doc, |d| matches!(d, Doc::Line(_)).then_some(())).is_some()
}

/// The width of a doc printed flat, from [`flat_width`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FlatWidth {
    Finite(usize),
    /// Contains a hard or literal line (or a text with a newline), so it can
    /// never be printed on one line. Greater than every `Finite` width.
    Infinite,
}

impl FlatWidth {
    fn add(self, other: FlatWidth) -> FlatWidth {
        match (self, other) {
            (FlatWidth::Finite(a), FlatWidth::Finite(b)) => FlatWidth::Finite(a + b),
            _ => FlatWidth::Infinite,
        }
    }
}

/// The width `doc` takes printed on one line, in columns. Not in Prettier: a
/// formatter measures pieces with it before deciding how to lay them out.
///
/// `Text` → [`str_width`] (`Infinite` if it contains `\n`); `Line(Soft)` →
/// 0; `Line(Normal)` → 1; `Line(Hard | Literal)` → `Infinite`; `IfBreak` →
/// its flat side; a group → its `contents`, whether or not it is broken (a
/// conditional group's expanded states are not looked at); `Indent`,
/// `Align` and `IndentIfBreak` → their contents; `LineSuffix`,
/// `LineSuffixBoundary` and `BreakParent` → 0. So only a line that is
/// always a newline makes a doc unmeasurable; a caller that also wants a
/// broken group or a break parent to count asks
/// [`will_break`] too.
pub fn flat_width(doc: &Doc) -> FlatWidth {
    match doc {
        Doc::Text(text) => {
            if text.contains('\n') {
                FlatWidth::Infinite
            } else {
                FlatWidth::Finite(str_width(text))
            }
        }
        Doc::Concat(parts) | Doc::Fill(parts) => {
            let mut width = FlatWidth::Finite(0);
            for part in parts {
                width = width.add(flat_width(part));
                if width == FlatWidth::Infinite {
                    break;
                }
            }
            width
        }
        Doc::Indent(contents) | Doc::Align(_, contents) | Doc::IndentIfBreak { contents, .. } => {
            flat_width(contents)
        }
        Doc::Group(group) => flat_width(&group.contents),
        Doc::IfBreak { flat_doc, .. } => flat_width(flat_doc),
        Doc::Line(LineKind::Soft) => FlatWidth::Finite(0),
        Doc::Line(LineKind::Normal) => FlatWidth::Finite(1),
        Doc::Line(LineKind::Hard | LineKind::Literal) => FlatWidth::Infinite,
        Doc::LineSuffix(_) | Doc::LineSuffixBoundary | Doc::BreakParent => FlatWidth::Finite(0),
    }
}

/// `propagateBreaks` (`index.js:145`): every group containing a
/// `BreakParent` or a broken group gets `should_break` (with
/// `break_propagated`). Returns whether `doc` itself would break an enclosing
/// group.
///
/// Prettier walks a conditional group's `expandedStates` but
/// `breakParentGroup` (131) refuses to mark a group that has them, so breaks
/// stop there unless the group was already broken. No visited set: an owned
/// tree has no shared subtrees; a conditional group's `contents` copy is
/// walked too, so it matches `expanded_states[0]`.
pub fn propagate_breaks(doc: &mut Doc) -> bool {
    match doc {
        Doc::BreakParent => true,
        Doc::Group(group) => {
            let mut child_breaks = propagate_breaks(&mut group.contents);
            if let Some(states) = &mut group.expanded_states {
                for state in states {
                    propagate_breaks(state);
                }
                // breakParentGroup: `!parentGroup.expandedStates`
                child_breaks = false;
            }
            if child_breaks && !group.should_break {
                group.should_break = true;
                group.break_propagated = true;
            }
            // propagateBreaksOnExitFn: `if (group.break) breakParentGroup(…)`
            group.should_break
        }
        Doc::Concat(parts) | Doc::Fill(parts) => {
            let mut breaks = false;
            for part in parts {
                breaks |= propagate_breaks(part);
            }
            breaks
        }
        Doc::IfBreak {
            break_doc,
            flat_doc,
            ..
        } => {
            let b = propagate_breaks(break_doc);
            let f = propagate_breaks(flat_doc);
            b || f
        }
        Doc::Indent(contents)
        | Doc::Align(_, contents)
        | Doc::IndentIfBreak { contents, .. }
        | Doc::LineSuffix(contents) => propagate_breaks(contents),
        Doc::Text(_) | Doc::LineSuffixBoundary | Doc::Line(_) => false,
    }
}

/// `removeLines`: lines become `" "` / `""` (hard lines stay), `IfBreak`
/// becomes its flat side.
pub fn remove_lines(doc: Doc) -> Doc {
    doc.map(&mut |d| match d {
        Doc::Line(LineKind::Soft) => Doc::empty(),
        Doc::Line(LineKind::Normal) => Doc::from(" "),
        Doc::IfBreak { flat_doc, .. } => *flat_doc,
        d => d,
    })
}

/// `cleanDoc` (`cleanDocFn`, `index.js:263`): flattens concats and merges
/// adjacent strings, drops empty strings, unwraps single-part concats and
/// fills, removes empty indents/aligns/line suffixes/if-breaks/groups and a
/// group whose only content is an identical group.
pub fn clean_doc(doc: Doc) -> Doc {
    doc.map(&mut clean_doc_fn)
}

fn clean_doc_fn(doc: Doc) -> Doc {
    match doc {
        Doc::Fill(mut parts) => {
            if parts.iter().all(Doc::is_empty_text) {
                return Doc::empty();
            }
            if parts.len() == 1 {
                return parts.pop().unwrap();
            }
            Doc::Fill(parts)
        }
        Doc::Group(group) => {
            if group.contents.is_empty_text()
                && group.id.is_none()
                && !group.should_break
                && group.expanded_states.is_none()
            {
                return Doc::empty();
            }
            // Remove nested only group. Prettier compares `expandedStates` by
            // reference, so only two plain groups qualify.
            if let Doc::Group(inner) = &group.contents {
                if inner.id == group.id
                    && inner.should_break == group.should_break
                    && inner.break_propagated == group.break_propagated
                    && inner.expanded_states.is_none()
                    && group.expanded_states.is_none()
                {
                    return group.contents;
                }
            }
            Doc::Group(group)
        }
        Doc::Align(_, ref contents)
        | Doc::Indent(ref contents)
        | Doc::IndentIfBreak { ref contents, .. }
        | Doc::LineSuffix(ref contents)
            if contents.is_empty_text() =>
        {
            Doc::empty()
        }
        Doc::IfBreak {
            ref break_doc,
            ref flat_doc,
            ..
        } if break_doc.is_empty_text() && flat_doc.is_empty_text() => Doc::empty(),
        Doc::Concat(doc) => {
            let mut parts: Vec<Doc> = Vec::with_capacity(doc.len());
            for part in doc {
                match part {
                    Doc::Text(t) if t.is_empty() => {}
                    // Children are already clean, so a nested concat is flat.
                    Doc::Concat(nested) => {
                        for p in nested {
                            push_merging_text(&mut parts, p);
                        }
                    }
                    part => push_merging_text(&mut parts, part),
                }
            }
            match parts.len() {
                0 => Doc::empty(),
                1 => parts.pop().unwrap(),
                _ => Doc::Concat(parts),
            }
        }
        doc => doc,
    }
}

fn push_merging_text(parts: &mut Vec<Doc>, part: Doc) {
    if let (Doc::Text(t), Some(Doc::Text(last))) = (&part, parts.last_mut()) {
        last.to_mut().push_str(t);
        return;
    }
    parts.push(part);
}

/// `stripTrailingHardline`: `clean_doc`, then remove trailing
/// `[line, BreakParent]` pairs and trailing newlines of the last string,
/// recursing into the last part.
pub fn strip_trailing_hardline(doc: Doc) -> Doc {
    strip_trailing_hardline_from_doc(clean_doc(doc))
}

fn strip_trailing_hardline_from_parts(mut parts: Vec<Doc>) -> Vec<Doc> {
    // index.js:199 — any line kind followed by a break parent.
    while parts.len() >= 2
        && matches!(parts[parts.len() - 2], Doc::Line(_))
        && matches!(parts[parts.len() - 1], Doc::BreakParent)
    {
        parts.truncate(parts.len() - 2);
    }
    if let Some(last) = parts.pop() {
        parts.push(strip_trailing_hardline_from_doc(last));
    }
    parts
}

fn strip_trailing_hardline_from_doc(doc: Doc) -> Doc {
    match doc {
        Doc::Indent(contents) => Doc::Indent(Box::new(strip_trailing_hardline_from_doc(*contents))),
        Doc::IndentIfBreak {
            contents,
            group_id,
            negate,
        } => Doc::IndentIfBreak {
            contents: Box::new(strip_trailing_hardline_from_doc(*contents)),
            group_id,
            negate,
        },
        Doc::Group(mut group) => {
            group.contents = strip_trailing_hardline_from_doc(std::mem::take(&mut group.contents));
            Doc::Group(group)
        }
        Doc::LineSuffix(contents) => {
            Doc::LineSuffix(Box::new(strip_trailing_hardline_from_doc(*contents)))
        }
        Doc::IfBreak {
            break_doc,
            flat_doc,
            group_id,
        } => Doc::IfBreak {
            break_doc: Box::new(strip_trailing_hardline_from_doc(*break_doc)),
            flat_doc: Box::new(strip_trailing_hardline_from_doc(*flat_doc)),
            group_id,
        },
        Doc::Fill(parts) => Doc::Fill(strip_trailing_hardline_from_parts(parts)),
        Doc::Concat(parts) => Doc::Concat(strip_trailing_hardline_from_parts(parts)),
        // trimNewlinesEnd
        Doc::Text(mut t) => {
            let len = t.trim_end_matches(['\r', '\n']).len();
            if len != t.len() {
                t.to_mut().truncate(len);
            }
            Doc::Text(t)
        }
        // index.js:242 — `align` is left alone.
        doc @ (Doc::Align(..) | Doc::LineSuffixBoundary | Doc::Line(_) | Doc::BreakParent) => doc,
    }
}

/// `replaceEndOfLine(doc, replacement)`: every string containing `\n` becomes
/// its lines joined with `replacement` (usually
/// [`literalline`](crate::builders::literalline)).
pub fn replace_end_of_line(doc: Doc, replacement: Doc) -> Doc {
    doc.map(&mut |d| match d {
        Doc::Text(t) if t.contains('\n') => join(
            replacement.clone(),
            t.split('\n').map(|line| Doc::from(line.to_owned())),
        ),
        d => d,
    })
}
