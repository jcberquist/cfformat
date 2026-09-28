//! `alignment.consecutive.assignments` and `alignment.consecutive.params` /
//! `.properties`, done in the printers, over the tree, rather than over the
//! output text.
//!
//! A printer that owns a sequence — a statement list, a delimited list, an
//! attribute group — partitions its entries into runs with [`runs`],
//! measuring each member's **printed** left side flat, and adds the padding
//! after that left side. The padding is spaces whatever `tab_indent` says
//! (tabs would line up only at one tab width), and it is whitespace the
//! parser discards, so a second run measures the same widths.

use cfdoc::builders::if_break_group;
use cfdoc::utils::{flat_width, will_break};
use cfdoc::{Doc, FlatWidth, GroupId};

/// What an entry of a sequence is to a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunPart {
    /// A member whose left side prints this many columns flat.
    Member(usize),
    /// Neither joins nor ends a run (a comment).
    Neutral,
    /// Ends the run (a blank line, a node of another shape, a left side that
    /// cannot be measured).
    Break,
}

/// Partitions `entries` into runs: maximal sequences of members, ignoring
/// neutral entries, ended by a breaker. Returns, per entry, the spaces that
/// bring a member's left side to the widest of its run, for members of runs of
/// two or more; `None` otherwise (a run of one pads nothing).
pub(crate) fn runs<T>(entries: &[T], mut part_of: impl FnMut(&T) -> RunPart) -> Vec<Option<usize>> {
    let mut pads = vec![None; entries.len()];
    let mut run: Vec<(usize, usize)> = Vec::new();
    let flush = |run: &mut Vec<(usize, usize)>, pads: &mut Vec<Option<usize>>| {
        if run.len() > 1 {
            let widest = run.iter().map(|&(_, w)| w).max().unwrap_or(0);
            for &(i, w) in run.iter() {
                pads[i] = Some(widest - w);
            }
        }
        run.clear();
    };
    for (i, entry) in entries.iter().enumerate() {
        match part_of(entry) {
            RunPart::Member(width) => run.push((i, width)),
            RunPart::Neutral => {}
            RunPart::Break => flush(&mut run, &mut pads),
        }
    }
    flush(&mut run, &mut pads);
    pads
}

/// `RunPart::Member` of a left side's flat width, or `Break` when it cannot be
/// measured: it holds a hard line or a group that will break.
pub(crate) fn member(left: Doc) -> RunPart {
    one_line_width(&left).map_or(RunPart::Break, RunPart::Member)
}

/// The flat width of a printed doc, `None` when it cannot print on one line:
/// it holds a hard line, or a group that will break (a broken part is not
/// short however few columns its last line takes). Stricter than
/// [`flat_width`], which measures a broken group as if it were flat.
pub(crate) fn one_line_width(doc: &Doc) -> Option<usize> {
    match flat_width(doc) {
        FlatWidth::Finite(width) if !will_break(doc) => Some(width),
        _ => None,
    }
}

/// What a statement is to an attribute run (`alignment.consecutive.params`
/// and `.properties`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ColumnPart {
    /// A member: its run key (the statement's kind and its attribute names,
    /// in order), the flat width of each attribute, and whether the
    /// statement fits on one line unpadded. A member that does not is
    /// printed broken and unpadded whatever the run does, so it stays in
    /// the run without setting any column's width.
    Member {
        names: Vec<String>,
        widths: Vec<usize>,
        fits: bool,
    },
    /// Neither joins nor ends a run (a line comment).
    Neutral,
    /// Ends the run (a blank line, another statement, an attribute that
    /// cannot be measured, a comment inside the statement).
    Break,
}

/// Partitions `entries` into attribute runs: maximal sequences of members with
/// equal names, ignoring neutral entries, ended by a breaker or by a member
/// whose names differ, which starts the next run (names compare
/// case-insensitively, each member's with the last one's). Returns, per entry
/// of a run of two or more, the spaces after each attribute that bring it to
/// the widest of its column among the members that fit on one line; the last
/// attribute is never padded (nothing after it needs aligning), so its entry
/// is 0, and a member wider than every fitting one gets 0 too (it prints
/// broken, where the pads do not show).
pub(crate) fn column_runs<T>(
    entries: &[T],
    mut part_of: impl FnMut(&T) -> ColumnPart,
) -> Vec<Option<Vec<usize>>> {
    let mut pads = vec![None; entries.len()];
    let mut run: Vec<(usize, Vec<usize>, bool)> = Vec::new();
    let mut key: Vec<String> = Vec::new();
    let flush = |run: &mut Vec<(usize, Vec<usize>, bool)>, pads: &mut Vec<Option<Vec<usize>>>| {
        if run.len() > 1 {
            let columns = run[0].1.len();
            let widest: Vec<usize> = (0..columns)
                .map(|c| {
                    run.iter()
                        .filter(|(_, _, fits)| *fits)
                        .map(|(_, w, _)| w[c])
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            for (i, widths, _) in run.iter() {
                let mut pad: Vec<usize> = widths
                    .iter()
                    .zip(&widest)
                    .map(|(w, m)| m.saturating_sub(*w))
                    .collect();
                if let Some(last) = pad.last_mut() {
                    *last = 0;
                }
                pads[*i] = Some(pad);
            }
        }
        run.clear();
    };
    for (i, entry) in entries.iter().enumerate() {
        match part_of(entry) {
            ColumnPart::Member {
                names,
                widths,
                fits,
            } => {
                let same = names.len() == key.len()
                    && names
                        .iter()
                        .zip(&key)
                        .all(|(a, b)| a.to_lowercase() == b.to_lowercase());
                if run.is_empty() || !same {
                    flush(&mut run, &mut pads);
                    key = names;
                }
                run.push((i, widths, fits));
            }
            ColumnPart::Neutral => {}
            ColumnPart::Break => flush(&mut run, &mut pads),
        }
    }
    flush(&mut run, &mut pads);
    pads
}

/// `n` spaces.
pub(crate) fn pad(n: usize) -> Doc {
    if n == 0 {
        Doc::empty()
    } else {
        Doc::from(" ".repeat(n))
    }
}

/// `n` spaces when the group `id` breaks, nothing when it is flat: a list
/// printed on one line never pads.
pub(crate) fn pad_if_break(n: usize, id: GroupId) -> Doc {
    if_break_group(pad(n), "", id)
}

/// `n` spaces when the group `id` is flat, nothing when it breaks: an
/// attribute printed on a line of its own never pads.
pub(crate) fn pad_if_flat(n: usize, id: GroupId) -> Doc {
    if_break_group("", pad(n), id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_partition() {
        use RunPart::*;
        let parts = [
            Member(1),
            Neutral,
            Member(3),
            Break,
            Member(2),
            Break,
            Member(4),
            Member(4),
        ];
        assert_eq!(
            runs(&parts, |p| *p),
            vec![Some(2), None, Some(0), None, None, None, Some(0), Some(0)]
        );
    }

    #[test]
    fn column_runs_partition() {
        let member = |names: &[&str], widths: &[usize]| ColumnPart::Member {
            names: names.iter().map(|n| n.to_string()).collect(),
            widths: widths.to_vec(),
            fits: true,
        };
        let broken = |names: &[&str], widths: &[usize]| match member(names, widths) {
            ColumnPart::Member { names, widths, .. } => ColumnPart::Member {
                names,
                widths,
                fits: false,
            },
            _ => unreachable!(),
        };
        let parts = [
            member(&["name", "inject"], &[8, 12]),
            ColumnPart::Neutral,
            // Names are compared without regard to case.
            member(&["Name", "INJECT"], &[14, 3]),
            // Other names: a new run (of one, so nothing is padded).
            member(&["name", "type"], &[5, 3]),
            ColumnPart::Break,
            // A bare attribute is a name like the others.
            member(&["name", "required"], &[7, 8]),
            member(&["name", "required"], &[9, 8]),
            member(&["name", "type", "required"], &[3, 10, 8]),
            member(&["name", "type", "required"], &[5, 6, 8]),
            ColumnPart::Break,
            // A member that does not fit stays in the run but sets no
            // column: the widest fitting `name` is 7, and it is not padded.
            broken(&["name", "inject"], &[20, 4]),
            member(&["name", "inject"], &[7, 9]),
            member(&["name", "inject"], &[3, 30]),
            ColumnPart::Break,
            // Two members of which neither fits: a run with nothing to pad.
            broken(&["name", "inject"], &[20, 4]),
            broken(&["name", "inject"], &[8, 4]),
        ];
        assert_eq!(
            column_runs(&parts, |p| p.clone()),
            vec![
                Some(vec![6, 0]),
                None,
                Some(vec![0, 0]),
                None,
                None,
                Some(vec![2, 0]),
                Some(vec![0, 0]),
                Some(vec![2, 0, 0]),
                Some(vec![0, 4, 0]),
                None,
                Some(vec![0, 0]),
                Some(vec![0, 0]),
                Some(vec![4, 0]),
                None,
                Some(vec![0, 0]),
                Some(vec![0, 0]),
            ]
        );
    }
}
