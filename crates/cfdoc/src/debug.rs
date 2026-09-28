//! Doc dumps: `printDocToDebug` (`prettier/src/document/debug.js`) and the
//! JSON shape of Prettier's doc objects (used by the parity test).

use std::fmt::Write;

use crate::doc::{Align, Doc, GroupId, LineKind};

/// `printDocToDebug(doc)`: the builder-call notation Prettier's
/// `--debug-print-doc` prints, e.g. `group(["a", indent([line, "b"])])`.
///
/// Group ids are numbers, so they print the way Prettier prints a non-symbol
/// id: `{ id: "3" }`.
pub fn format_doc(doc: &Doc) -> String {
    let mut out = String::new();
    print_doc(doc, None, &mut out);
    out
}

/// Parts of a concat after `flattenDoc` (debug.js:17): nested concats are
/// spliced in and empty strings dropped.
fn flatten<'a>(parts: &'a [Doc], out: &mut Vec<&'a Doc>) {
    for part in parts {
        match part {
            Doc::Concat(nested) => flatten(nested, out),
            Doc::Text(t) if t.is_empty() => {}
            part => out.push(part),
        }
    }
}

/// `printDoc(doc, index, parentParts)` (debug.js:72); returns `false` for the
/// `undefined` result (a `breakParent` right after a hard line).
fn print_doc(doc: &Doc, parent: Option<(&[&Doc], usize)>, out: &mut String) -> bool {
    match doc {
        Doc::Text(t) => json_string(t, out),
        Doc::Concat(parts) => {
            let mut flat = Vec::new();
            flatten(parts, &mut flat);
            let mut printed = Vec::with_capacity(flat.len());
            for i in 0..flat.len() {
                let mut s = String::new();
                if print_doc(flat[i], Some((&flat, i)), &mut s) {
                    printed.push(s);
                }
            }
            if printed.len() == 1 {
                out.push_str(&printed[0]);
            } else {
                out.push('[');
                out.push_str(&printed.join(", "));
                out.push(']');
            }
        }
        Doc::Line(kind) => {
            let with_break_parent = parent
                .and_then(|(parts, i)| parts.get(i + 1))
                .is_some_and(|d| matches!(d, Doc::BreakParent));
            out.push_str(match (kind, with_break_parent) {
                (LineKind::Literal, true) => "literalline",
                (LineKind::Literal, false) => "literallineWithoutBreakParent",
                (LineKind::Hard, true) => "hardline",
                (LineKind::Hard, false) => "hardlineWithoutBreakParent",
                (LineKind::Soft, _) => "softline",
                (LineKind::Normal, _) => "line",
            });
        }
        Doc::BreakParent => {
            let after_hardline = parent
                .and_then(|(parts, i)| i.checked_sub(1).map(|j| parts[j]))
                .is_some_and(|d| matches!(d, Doc::Line(LineKind::Hard | LineKind::Literal)));
            if after_hardline {
                return false;
            }
            out.push_str("breakParent");
        }
        Doc::Indent(contents) => wrap("indent(", contents, ")", out),
        Doc::Align(align, contents) => match align {
            Align::DedentToRoot => wrap("dedentToRoot(", contents, ")", out),
            Align::Dedent => wrap("dedent(", contents, ")", out),
            Align::MarkRoot => wrap("markAsRoot(", contents, ")", out),
            Align::Width(n) => wrap(&format!("align({n}, "), contents, ")", out),
            Align::Str(s) => {
                let mut open = String::from("align(");
                json_string(s, &mut open);
                open.push_str(", ");
                wrap(&open, contents, ")", out);
            }
        },
        Doc::IfBreak {
            break_doc,
            flat_doc,
            group_id,
        } => {
            out.push_str("ifBreak(");
            print_doc(break_doc, None, out);
            // `doc.flatContents` is falsy only for "" (an empty array is truthy).
            let has_flat = !flat_doc.is_empty_text();
            if has_flat {
                out.push_str(", ");
                print_doc(flat_doc, None, out);
            }
            if let Some(id) = group_id {
                if !has_flat {
                    out.push_str(", \"\"");
                }
                let _ = write!(out, ", {{ groupId: {} }}", group_id_str(*id));
            }
            out.push(')');
        }
        Doc::IndentIfBreak {
            contents,
            group_id,
            negate,
        } => {
            out.push_str("indentIfBreak(");
            print_doc(contents, None, out);
            let mut options = Vec::new();
            if *negate {
                options.push("negate: true".to_owned());
            }
            options.push(format!("groupId: {}", group_id_str(*group_id)));
            let _ = write!(out, ", {{ {} }})", options.join(", "));
        }
        Doc::Group(group) => {
            let mut options = Vec::new();
            if group.should_break && !group.break_propagated {
                options.push("shouldBreak: true".to_owned());
            }
            if let Some(id) = group.id {
                options.push(format!("id: {}", group_id_str(id)));
            }
            let options = if options.is_empty() {
                String::new()
            } else {
                format!(", {{ {} }}", options.join(", "))
            };

            if let Some(states) = &group.expanded_states {
                out.push_str("conditionalGroup([");
                let states = if states.is_empty() {
                    std::slice::from_ref(&group.contents)
                } else {
                    states
                };
                for (i, state) in states.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    print_doc(state, None, out);
                }
                out.push(']');
            } else {
                out.push_str("group(");
                print_doc(&group.contents, None, out);
            }
            out.push_str(&options);
            out.push(')');
        }
        Doc::Fill(parts) => {
            out.push_str("fill([");
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_doc(part, None, out);
            }
            out.push_str("])");
        }
        Doc::LineSuffix(contents) => wrap("lineSuffix(", contents, ")", out),
        Doc::LineSuffixBoundary => out.push_str("lineSuffixBoundary"),
    }
    true
}

fn wrap(open: &str, contents: &Doc, close: &str, out: &mut String) {
    out.push_str(open);
    print_doc(contents, None, out);
    out.push_str(close);
}

/// `JSON.stringify(String(id))`.
fn group_id_str(id: GroupId) -> String {
    format!("\"{id}\"")
}

/// `JSON.stringify(string)`.
fn json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Serialises `doc` as the JSON of Prettier's doc objects, for feeding
/// `printDocToString`: strings, arrays for concats, and
/// `{"type": "group", "contents", "break", "id", "expandedStates"}`,
/// `{"type": "line", "hard", "soft", "literal"}`,
/// `{"type": "if-break", "breakContents", "flatContents", "groupId"}`, …
///
/// Two values have no JSON form and need reviving on the JS side:
/// `dedentToRoot`'s `n` (`-Infinity`) is written as `null`, and a conditional
/// group's `contents` should be made the same object as `expandedStates[0]`.
/// Group ids are strings (`"g3"`).
pub fn to_prettier_json(doc: &Doc) -> String {
    let mut out = String::new();
    json_doc(doc, &mut out);
    out
}

fn json_doc(doc: &Doc, out: &mut String) {
    match doc {
        Doc::Text(t) => json_string(t, out),
        Doc::Concat(parts) => json_array(parts, out),
        Doc::Indent(contents) => json_node("indent", &[("contents", contents)], "", out),
        Doc::Align(align, contents) => {
            let n = match align {
                Align::Width(n) => n.to_string(),
                Align::Str(s) => {
                    let mut n = String::new();
                    json_string(s, &mut n);
                    n
                }
                Align::Dedent => "-1".to_owned(),
                Align::DedentToRoot => "null".to_owned(),
                Align::MarkRoot => r#"{"type":"root"}"#.to_owned(),
            };
            json_node(
                "align",
                &[("contents", contents)],
                &format!(r#","n":{n}"#),
                out,
            );
        }
        Doc::Group(group) => {
            let mut extra = format!(r#","break":{}"#, group.should_break);
            if let Some(id) = group.id {
                let _ = write!(extra, r#","id":"g{id}""#);
            }
            if let Some(states) = &group.expanded_states {
                extra.push_str(r#","expandedStates":"#);
                let states = if states.is_empty() {
                    std::slice::from_ref(&group.contents)
                } else {
                    states
                };
                json_array(states, &mut extra);
            }
            json_node("group", &[("contents", &group.contents)], &extra, out);
        }
        Doc::Fill(parts) => {
            out.push_str(r#"{"type":"fill","parts":"#);
            json_array(parts, out);
            out.push('}');
        }
        Doc::IfBreak {
            break_doc,
            flat_doc,
            group_id,
        } => {
            let extra = group_id
                .map(|id| format!(r#","groupId":"g{id}""#))
                .unwrap_or_default();
            json_node(
                "if-break",
                &[("breakContents", break_doc), ("flatContents", flat_doc)],
                &extra,
                out,
            );
        }
        Doc::IndentIfBreak {
            contents,
            group_id,
            negate,
        } => json_node(
            "indent-if-break",
            &[("contents", contents)],
            &format!(r#","groupId":"g{group_id}","negate":{negate}"#),
            out,
        ),
        Doc::LineSuffix(contents) => json_node("line-suffix", &[("contents", contents)], "", out),
        Doc::LineSuffixBoundary => out.push_str(r#"{"type":"line-suffix-boundary"}"#),
        Doc::Line(LineKind::Normal) => out.push_str(r#"{"type":"line"}"#),
        Doc::Line(LineKind::Soft) => out.push_str(r#"{"type":"line","soft":true}"#),
        Doc::Line(LineKind::Hard) => out.push_str(r#"{"type":"line","hard":true}"#),
        Doc::Line(LineKind::Literal) => {
            out.push_str(r#"{"type":"line","hard":true,"literal":true}"#)
        }
        Doc::BreakParent => out.push_str(r#"{"type":"break-parent"}"#),
    }
}

fn json_array(parts: &[Doc], out: &mut String) {
    out.push('[');
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_doc(part, out);
    }
    out.push(']');
}

fn json_node(ty: &str, children: &[(&str, &Doc)], extra: &str, out: &mut String) {
    let _ = write!(out, r#"{{"type":"{ty}""#);
    for (key, child) in children {
        let _ = write!(out, r#","{key}":"#);
        json_doc(child, out);
    }
    out.push_str(extra);
    out.push('}');
}
