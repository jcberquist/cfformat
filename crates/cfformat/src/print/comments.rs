//! Line, block and doc comments.
//!
//! Where a comment goes is decided by its parent's printer (a comment
//! after a statement is its sibling, on the same line iff no
//! `Newline` sits between them); this module prints the comment itself.

use cfdoc::builders::{hardline, literalline};
use cfdoc::Doc;
use cfparse::{Element, ElementKind};

use super::Printer;
use crate::options::Asterisks;

impl Printer<'_> {
    /// A comment element. The newline that ends a line comment is the
    /// caller's. A `<!--- --->` is a tag comment only where the tag printer is
    /// the enclosing one (`TagCtx::tags`): in a script file, and inside a
    /// `<cfscript>` body, it is verbatim text like any other block comment.
    pub(crate) fn comment(&self, e: &Element) -> Doc {
        if e.kind == ElementKind::LineComment {
            self.line_comment(e)
        } else if super::tags::is_tag_comment(e) && self.tag_ctx().tags {
            self.tag_comment(e, self.tag_ctx())
        } else {
            self.block_comment(e)
        }
    }

    /// `//` + text, with one space added before text that has none:
    /// `//test` → `// test`, `//` stays.
    fn line_comment(&self, e: &Element) -> Doc {
        let open = e.open.as_ref().map_or(e.span.start, |t| t.span.end);
        let delimiter = self.tree.slice(e.span.start..open);
        let text = self.tree.slice(open..e.span.end).trim_end();
        let space = if !text.is_empty() && !text.starts_with(char::is_whitespace) {
            " "
        } else {
            ""
        };
        Doc::from(format!("{delimiter}{space}{text}"))
    }

    /// Block and doc comments print their lines verbatim (trailing whitespace
    /// trimmed), except when every line after the first starts with `*` and
    /// `comment.asterisks` is not `ignored`: those lines are re-emitted at the
    /// current indent (`indent`) or one column right of it (`align`). With
    /// `alignment.doc_comments` a doc comment's lines are first rewritten by
    /// [`align_doc_comment`].
    fn block_comment(&self, e: &Element) -> Doc {
        let text = self.tree.slice(e.span.clone());
        let aligned = (self.opts.alignment_doc_comments && e.kind == ElementKind::DocComment)
            .then(|| align_doc_comment(text))
            .flatten();
        let lines: Vec<&str> = match &aligned {
            Some(lines) => lines.iter().map(String::as_str).collect(),
            None => text.split('\n').collect(),
        };
        let starred = lines.len() > 1 && lines[1..].iter().all(|l| l.trim_start().starts_with('*'));
        let mut parts = vec![Doc::from(lines[0].trim_end().to_owned())];
        if starred && self.opts.comment_asterisks != Asterisks::Ignored {
            let lead = if self.opts.comment_asterisks == Asterisks::Align {
                " "
            } else {
                ""
            };
            for l in &lines[1..] {
                parts.push(hardline());
                parts.push(Doc::from(format!("{lead}{}", l.trim())));
            }
        } else {
            for l in &lines[1..] {
                parts.push(literalline());
                parts.push(Doc::from(l.trim_end().to_owned()));
            }
        }
        Doc::Concat(parts)
    }
}

/// Which block of a doc comment a tag line belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TagBlock {
    Param,
    Return,
    Throws,
}

/// A tag line and the untagged lines that continue it.
struct TagLine<'a> {
    block: TagBlock,
    indent: &'a str,
    tag: &'a str,
    rest: String,
    continuation: Vec<String>,
}

/// `alignment.doc_comments`: the lines of a doc comment with its tags aligned
/// and its blocks ordered, or `None` when the comment is left as it is — fewer
/// than three lines, a line after the first that does not start with `*`, or
/// no tag line at all (so an untagged comment never changes).
///
/// Each line after the first and before the last is `[ \t]*\*`, an optional
/// tag — `@throws Name` or `@[A-Za-z0-9$._:-]+` — and the rest. A tag line is
/// a `@return` / `@returns` line (printed `@return`), a `@throws…` line, or a
/// param line (any other tag); an untagged non-empty line right after a tag
/// line (or its continuation) continues that tag and stays after it, so a
/// tag's text wrapped over several lines stays together. Other untagged lines
/// are the description, consecutive empty lines collapsed to one, except that
/// an empty line after a tag line is a separator and is dropped: the output
/// places its own separators between blocks. Output: the first line, the
/// description, the params with their text padded to the widest param tag, the
/// return lines, the throws with their text padded to the widest
/// `@throws Name`, the last line; an empty ` *` line before each block unless
/// the line before is already empty.
pub(crate) fn align_doc_comment(text: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() < 3 || !lines[1..].iter().all(|l| l.trim_start().starts_with('*')) {
        return None;
    }
    let middle = &lines[1..lines.len() - 1];
    let mut description: Vec<String> = Vec::new();
    let mut tags: Vec<TagLine> = Vec::new();
    let mut empty_line = "";
    let mut continues = false;
    for line in middle {
        let blank = line.len() - line.trim_start_matches([' ', '\t']).len();
        let after = line[blank..].strip_prefix('*')?;
        let indent = &line[..=blank];
        empty_line = indent;
        match doc_tag(after) {
            Some((tag, rest)) => {
                let block = if tag == "@return" || tag == "@returns" {
                    TagBlock::Return
                } else if tag.starts_with("@throws") {
                    TagBlock::Throws
                } else {
                    TagBlock::Param
                };
                let rest = if rest.trim().is_empty() {
                    String::new()
                } else {
                    format!(" {}", rest.trim_start())
                };
                tags.push(TagLine {
                    block,
                    indent,
                    tag: if block == TagBlock::Return {
                        "@return"
                    } else {
                        tag
                    },
                    rest,
                    continuation: Vec::new(),
                });
                continues = true;
            }
            None if continues && !after.trim().is_empty() => {
                let tag = tags.last_mut().expect("a tag line before a continuation");
                tag.continuation.push(format!("{indent}{after}"));
            }
            None => {
                continues = false;
                let empty = after.trim().is_empty();
                // An empty line after a tag line separates blocks; the output
                // places its own separators. Before any tag it is description.
                if empty && !tags.is_empty() {
                    continue;
                }
                if !(empty && description.last().is_some_and(|l| is_empty_doc_line(l))) {
                    description.push(format!("{indent}{after}"));
                }
            }
        }
    }
    if tags.is_empty() {
        return None;
    }
    let mut out = vec![lines[0].to_string()];
    out.extend(description);
    for block in [TagBlock::Param, TagBlock::Return, TagBlock::Throws] {
        let members: Vec<&TagLine> = tags.iter().filter(|t| t.block == block).collect();
        if members.is_empty() {
            continue;
        }
        if out.len() > 1 && !out.last().is_some_and(|l| is_empty_doc_line(l)) {
            out.push(empty_line.to_string());
        }
        let widest = members.iter().map(|t| t.tag.len()).max().unwrap_or(0);
        for t in members {
            let pad = match block {
                TagBlock::Return => 0,
                _ => widest - t.tag.len(),
            };
            out.push(format!(
                "{} {}{}{}",
                t.indent,
                t.tag,
                " ".repeat(pad),
                t.rest
            ));
            out.extend(t.continuation.iter().cloned());
        }
    }
    out.push(lines[lines.len() - 1].to_string());
    Some(out)
}

/// The tag at the start of a doc line's text after its `*` (spaces and tabs
/// first), and the rest of the line: `@throws Name` (one space, a name of
/// `[A-Za-z0-9$._]`), else `@[A-Za-z0-9$._:-]+`.
fn doc_tag(after_star: &str) -> Option<(&str, &str)> {
    let text = after_star.trim_start_matches([' ', '\t']);
    let word = |s: &str, extra: &[char]| {
        s.find(|c: char| !(c.is_ascii_alphanumeric() || "$._".contains(c) || extra.contains(&c)))
            .unwrap_or(s.len())
    };
    if let Some(name) = text.strip_prefix("@throws ") {
        let n = word(name, &[]);
        if n > 0 {
            let end = "@throws ".len() + n;
            return Some((&text[..end], &text[end..]));
        }
    }
    let n = word(text.strip_prefix('@')?, &[':', '-']);
    (n > 0).then(|| (&text[..=n], &text[n + 1..]))
}

/// A doc line with nothing after its `*`.
fn is_empty_doc_line(line: &str) -> bool {
    line.trim() == "*"
}
