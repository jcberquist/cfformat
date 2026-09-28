//! `printDocToString` (`prettier/src/document/printer/printer.js`).
//!
//! The control flow follows the JS line for line so the two can be diffed;
//! comments name the Prettier line where the Rust has to differ. Not ported:
//! `cursor`, `trim` and `label` docs, and cursor-position bookkeeping.

use std::rc::Rc;

use crate::doc::{Doc, GroupId, LineKind};
use crate::indent::{make_align, make_indent, root_indent, Indent, IndentStyle};
use crate::utils::propagate_breaks;
use crate::width::str_width;

/// Printer options (Prettier's `printWidth`, `useTabs`/`tabWidth`,
/// `endOfLine`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrintOptions {
    pub width: usize,
    pub indent: IndentStyle,
    /// `"\n"` or `"\r\n"`, written at every line break and in place of every
    /// `\n` inside text.
    pub newline: &'static str,
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            width: 80,
            indent: IndentStyle::Spaces(2),
            newline: "\n",
        }
    }
}

/// `MODE_BREAK` / `MODE_FLAT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Break,
    Flat,
}

/// What a command prints.
#[derive(Clone, Copy, Debug)]
enum CmdDoc<'a> {
    Doc(&'a Doc),
    /// A fill with its first `offset` parts already printed; replaces the
    /// `DOC_FILL_PRINTED_LENGTH` copy of the fill doc (printer.js:405).
    Fill {
        parts: &'a [Doc],
        offset: usize,
    },
    /// A slice printed as a concat; replaces the temporary
    /// `[content, whitespace, secondContent]` array (printer.js:412). Only
    /// ever measured by `fits`.
    Parts(&'a [Doc]),
}

#[derive(Clone, Debug)]
struct Cmd<'a> {
    indent: Rc<Indent<'a>>,
    mode: Mode,
    doc: CmdDoc<'a>,
}

/// Group modes by id (`groupModeMap`); `None` = not printed yet.
#[derive(Default)]
struct GroupModes(Vec<Option<Mode>>);

impl GroupModes {
    fn get(&self, id: GroupId) -> Option<Mode> {
        self.0.get(id as usize).copied().flatten()
    }

    fn set(&mut self, id: GroupId, mode: Mode) {
        let i = id as usize;
        if i >= self.0.len() {
            self.0.resize(i + 1, None);
        }
        self.0[i] = Some(mode);
    }
}

/// `hardlineWithoutBreakParent`, pushed by a line-suffix boundary.
static HARDLINE_WITHOUT_BREAK_PARENT: Doc = Doc::Line(LineKind::Hard);

/// Formats `doc`: runs [`propagate_breaks`], then prints it
/// (`printDocToString(doc, options).formatted`).
pub fn print_doc(doc: &mut Doc, opts: &PrintOptions) -> String {
    // printer.js:197 — propagateBreaks runs once, before printing.
    propagate_breaks(doc);
    print_resolved(doc, opts)
}

/// `fits` (printer.js:53). `rest` is the printer's command stack, read from
/// the top.
fn fits<'a>(
    next: Cmd<'a>,
    rest: &[Cmd<'a>],
    mut remaining_width: isize,
    mut has_line_suffix: bool,
    group_modes: &GroupModes,
    must_be_flat: bool,
    stack: &mut Vec<(Mode, CmdDoc<'a>)>,
) -> bool {
    let mut rest_index = rest.len();
    let mut has_pending_space = false;
    stack.clear();
    stack.push((next.mode, next.doc));
    // `output` (printer.js:71) only serves the unported `trim` doc.
    while remaining_width >= 0 {
        let Some((mode, doc)) = stack.pop() else {
            if rest_index == 0 {
                return true;
            }
            rest_index -= 1;
            stack.push((rest[rest_index].mode, rest[rest_index].doc));
            continue;
        };

        let doc = match doc {
            CmdDoc::Doc(doc) => doc,
            CmdDoc::Fill { parts, offset } => {
                push_parts_rev(stack, mode, &parts[offset..]);
                continue;
            }
            CmdDoc::Parts(parts) => {
                push_parts_rev(stack, mode, parts);
                continue;
            }
        };
        match doc {
            Doc::Text(text) => {
                if !text.is_empty() {
                    if has_pending_space {
                        remaining_width -= 1;
                        has_pending_space = false;
                    }
                    remaining_width -= str_width(text) as isize;
                }
            }
            Doc::Concat(parts) | Doc::Fill(parts) => push_parts_rev(stack, mode, parts),
            Doc::Indent(contents)
            | Doc::Align(_, contents)
            | Doc::IndentIfBreak { contents, .. } => stack.push((mode, CmdDoc::Doc(contents))),
            Doc::Group(group) => {
                if must_be_flat && group.should_break {
                    return false;
                }
                let group_mode = if group.should_break {
                    Mode::Break
                } else {
                    mode
                };
                // The most expanded state takes up the least space on the current line.
                let contents = match &group.expanded_states {
                    // Empty: `conditional_group_contents`, whose one state is `contents`.
                    Some(states) if group_mode == Mode::Break => {
                        states.last().unwrap_or(&group.contents)
                    }
                    _ => &group.contents,
                };
                stack.push((group_mode, CmdDoc::Doc(contents)));
            }
            Doc::IfBreak {
                break_doc,
                flat_doc,
                group_id,
            } => {
                let group_mode = match group_id {
                    Some(id) => group_modes.get(*id).unwrap_or(Mode::Flat),
                    None => mode,
                };
                let contents = if group_mode == Mode::Break {
                    break_doc
                } else {
                    flat_doc
                };
                if !contents.is_empty_text() {
                    stack.push((mode, CmdDoc::Doc(contents)));
                }
            }
            Doc::Line(kind) => {
                if mode == Mode::Break || matches!(kind, LineKind::Hard | LineKind::Literal) {
                    return true;
                }
                if *kind != LineKind::Soft {
                    has_pending_space = true;
                }
            }
            Doc::LineSuffix(_) => has_line_suffix = true,
            Doc::LineSuffixBoundary => {
                if has_line_suffix {
                    return false;
                }
            }
            Doc::BreakParent => {}
        }
    }
    false
}

fn push_parts_rev<'a>(stack: &mut Vec<(Mode, CmdDoc<'a>)>, mode: Mode, parts: &'a [Doc]) {
    for part in parts.iter().rev() {
        stack.push((mode, CmdDoc::Doc(part)));
    }
}

/// `printDocToString` (printer.js:179) without the pre-pass: `doc` must
/// already have gone through `propagate_breaks`.
fn print_resolved(doc: &Doc, opts: &PrintOptions) -> String {
    let mut group_modes = GroupModes::default();
    let width = opts.width as isize;
    let new_line = opts.newline;
    let style = opts.indent;
    let mut position: isize = 0;
    let mut commands: Vec<Cmd> = vec![Cmd {
        indent: root_indent(),
        mode: Mode::Break,
        doc: CmdDoc::Doc(doc),
    }];
    let mut should_remeasure = false;
    let mut line_suffix: Vec<Cmd> = Vec::new();
    let mut fits_stack = Vec::new();

    let mut result = String::new();

    while let Some(Cmd { indent, mode, doc }) = commands.pop() {
        let doc = match doc {
            CmdDoc::Doc(doc) => doc,
            CmdDoc::Fill { parts, offset } => {
                print_fill(
                    parts,
                    offset,
                    indent,
                    mode,
                    width - position,
                    &mut commands,
                    &line_suffix,
                    &group_modes,
                    &mut fits_stack,
                );
                flush_line_suffix_at_end(&mut commands, &mut line_suffix);
                continue;
            }
            CmdDoc::Parts(parts) => {
                for part in parts.iter().rev() {
                    commands.push(Cmd {
                        indent: indent.clone(),
                        mode,
                        doc: CmdDoc::Doc(part),
                    });
                }
                flush_line_suffix_at_end(&mut commands, &mut line_suffix);
                continue;
            }
        };

        match doc {
            Doc::Text(text) => {
                let formatted = if new_line != "\n" && text.contains('\n') {
                    std::borrow::Cow::Owned(text.replace('\n', new_line))
                } else {
                    std::borrow::Cow::Borrowed(&**text)
                };
                // Plugins may print single string, should skip measure the width
                if !formatted.is_empty() {
                    result.push_str(&formatted);
                    if !commands.is_empty() {
                        position += str_width(&formatted) as isize;
                    }
                }
            }

            Doc::Concat(parts) => {
                for part in parts.iter().rev() {
                    commands.push(Cmd {
                        indent: indent.clone(),
                        mode,
                        doc: CmdDoc::Doc(part),
                    });
                }
            }

            Doc::Indent(contents) => commands.push(Cmd {
                indent: make_indent(&indent, style),
                mode,
                doc: CmdDoc::Doc(contents),
            }),

            Doc::Align(align, contents) => commands.push(Cmd {
                indent: make_align(&indent, align, style),
                mode,
                doc: CmdDoc::Doc(contents),
            }),

            Doc::Group(group) => {
                let command = 'print_group: {
                    if mode == Mode::Flat && !should_remeasure {
                        break 'print_group Cmd {
                            indent,
                            mode: if group.should_break {
                                Mode::Break
                            } else {
                                Mode::Flat
                            },
                            doc: CmdDoc::Doc(&group.contents),
                        };
                    }

                    should_remeasure = false;
                    let remaining_width = width - position;
                    let has_line_suffix = !line_suffix.is_empty();

                    let flat_command = Cmd {
                        indent: indent.clone(),
                        mode: Mode::Flat,
                        doc: CmdDoc::Doc(&group.contents),
                    };
                    if !group.should_break
                        && fits(
                            flat_command.clone(),
                            &commands,
                            remaining_width,
                            has_line_suffix,
                            &group_modes,
                            false,
                            &mut fits_stack,
                        )
                    {
                        break 'print_group flat_command;
                    }

                    let Some(states) = &group.expanded_states else {
                        break 'print_group Cmd {
                            indent,
                            mode: Mode::Break,
                            doc: CmdDoc::Doc(&group.contents),
                        };
                    };

                    if !group.should_break {
                        // Expanded states are a rare case where a document
                        // can manually provide multiple representations of
                        // itself. It provides an array of documents
                        // going from the least expanded (most flattened)
                        // representation first to the most expanded. If a
                        // group has these, we need to manually go through
                        // these states and find the first one that fits.
                        for state in states.iter().take(states.len().saturating_sub(1)).skip(1) {
                            let flat_command = Cmd {
                                indent: indent.clone(),
                                mode: Mode::Flat,
                                doc: CmdDoc::Doc(state),
                            };
                            if fits(
                                flat_command.clone(),
                                &commands,
                                remaining_width,
                                has_line_suffix,
                                &group_modes,
                                false,
                                &mut fits_stack,
                            ) {
                                break 'print_group flat_command;
                            }
                        }
                    }

                    Cmd {
                        indent,
                        mode: Mode::Break,
                        doc: CmdDoc::Doc(states.last().unwrap_or(&group.contents)),
                    }
                };

                let command_mode = command.mode;
                commands.push(command);

                if let Some(id) = group.id {
                    group_modes.set(id, command_mode);
                }
            }

            Doc::Fill(parts) => print_fill(
                parts,
                0,
                indent,
                mode,
                width - position,
                &mut commands,
                &line_suffix,
                &group_modes,
                &mut fits_stack,
            ),

            Doc::IfBreak {
                break_doc,
                flat_doc,
                group_id,
            } => {
                let group_mode = match group_id {
                    Some(id) => group_modes.get(*id),
                    None => Some(mode),
                };
                let contents = match group_mode {
                    Some(Mode::Break) => Some(break_doc),
                    Some(Mode::Flat) => Some(flat_doc),
                    None => None,
                };
                if let Some(contents) = contents.filter(|c| !c.is_empty_text()) {
                    commands.push(Cmd {
                        indent,
                        mode,
                        doc: CmdDoc::Doc(contents),
                    });
                }
            }

            Doc::IndentIfBreak {
                contents,
                group_id,
                negate,
            } => {
                // `indentDoc(doc.contents)` (printer.js:443/453) is pushed as
                // `contents` under `makeIndent`, which is what printing the
                // indent doc would do next.
                let indented = match group_modes.get(*group_id) {
                    Some(Mode::Break) => Some(!negate),
                    Some(Mode::Flat) => Some(*negate),
                    None => None,
                };
                if let Some(indented) = indented {
                    if !contents.is_empty_text() {
                        commands.push(Cmd {
                            indent: if indented {
                                make_indent(&indent, style)
                            } else {
                                indent
                            },
                            mode,
                            doc: CmdDoc::Doc(contents),
                        });
                    }
                }
            }

            Doc::LineSuffix(contents) => line_suffix.push(Cmd {
                indent,
                mode,
                doc: CmdDoc::Doc(contents),
            }),

            Doc::LineSuffixBoundary => {
                if !line_suffix.is_empty() {
                    commands.push(Cmd {
                        indent,
                        mode,
                        doc: CmdDoc::Doc(&HARDLINE_WITHOUT_BREAK_PARENT),
                    });
                }
            }

            Doc::Line(kind) => {
                let hard = matches!(kind, LineKind::Hard | LineKind::Literal);
                if mode == Mode::Flat && !hard {
                    if *kind != LineKind::Soft {
                        result.push(' ');
                        position += 1;
                    }
                } else {
                    if mode == Mode::Flat {
                        // This line was forced into the output even if we
                        // were in flattened mode, so we need to tell the next
                        // group that no matter what, it needs to remeasure
                        // because the previous measurement didn't accurately
                        // capture the entire expression (this is necessary
                        // for nested groups)
                        should_remeasure = true;
                    }

                    if !line_suffix.is_empty() {
                        commands.push(Cmd {
                            indent,
                            mode,
                            doc: CmdDoc::Doc(doc),
                        });
                        commands.extend(line_suffix.drain(..).rev());
                    } else if *kind == LineKind::Literal {
                        result.push_str(new_line);
                        // `indent.root` is always set in Prettier; `None`
                        // stands for ROOT_INDENT (empty).
                        position = 0;
                        if let Some(root) = &indent.root {
                            result.push_str(&root.value);
                            position = root.width as isize;
                        }
                    } else {
                        trim_trailing_ws(&mut result);
                        result.push_str(new_line);
                        result.push_str(&indent.value);
                        position = indent.width as isize;
                    }
                }
            }

            Doc::BreakParent => {}
        }

        flush_line_suffix_at_end(&mut commands, &mut line_suffix);
    }

    result
}

/// Flush remaining line-suffix contents at the end of the document, in case
/// there is no new line after the line-suffix (printer.js:533).
fn flush_line_suffix_at_end<'a>(commands: &mut Vec<Cmd<'a>>, line_suffix: &mut Vec<Cmd<'a>>) {
    if commands.is_empty() && !line_suffix.is_empty() {
        commands.extend(line_suffix.drain(..).rev());
    }
}

/// The `DOC_TYPE_FILL` case (printer.js:343).
///
/// Fills each line with as much code as possible before moving to a new
/// line with the same indentation. `parts` alternate content and
/// whitespace; three layout cases:
/// * the first two content items fit on the same line without breaking
///   -> output the first content item and the whitespace "flat";
/// * only the first content item fits on the line without breaking
///   -> output the first content item "flat" and the whitespace with "break";
/// * neither content item fits on the line without breaking
///   -> output the first content item and the whitespace with "break".
#[allow(clippy::too_many_arguments)]
fn print_fill<'a>(
    parts: &'a [Doc],
    offset: usize,
    indent: Rc<Indent<'a>>,
    mode: Mode,
    remaining_width: isize,
    commands: &mut Vec<Cmd<'a>>,
    line_suffix: &[Cmd<'a>],
    group_modes: &GroupModes,
    fits_stack: &mut Vec<(Mode, CmdDoc<'a>)>,
) {
    let length = parts.len() - offset;
    if length == 0 {
        return;
    }

    let content = &parts[offset];
    let content_flat_command = Cmd {
        indent: indent.clone(),
        mode: Mode::Flat,
        doc: CmdDoc::Doc(content),
    };
    let content_break_command = Cmd {
        indent: indent.clone(),
        mode: Mode::Break,
        doc: CmdDoc::Doc(content),
    };
    let content_fits = fits(
        content_flat_command.clone(),
        &[],
        remaining_width,
        !line_suffix.is_empty(),
        group_modes,
        true,
        fits_stack,
    );

    if length == 1 {
        if content_fits {
            commands.push(content_flat_command);
        } else {
            commands.push(content_break_command);
        }
        return;
    }

    let whitespace = &parts[offset + 1];
    let whitespace_flat_command = Cmd {
        indent: indent.clone(),
        mode: Mode::Flat,
        doc: CmdDoc::Doc(whitespace),
    };
    let whitespace_break_command = Cmd {
        indent: indent.clone(),
        mode: Mode::Break,
        doc: CmdDoc::Doc(whitespace),
    };

    if length == 2 {
        if content_fits {
            commands.push(whitespace_flat_command);
            commands.push(content_flat_command);
        } else {
            commands.push(whitespace_break_command);
            commands.push(content_break_command);
        }
        return;
    }

    let remaining_command = Cmd {
        indent: indent.clone(),
        mode,
        doc: CmdDoc::Fill {
            parts,
            offset: offset + 2,
        },
    };

    let first_and_second_content_flat_command = Cmd {
        indent,
        mode: Mode::Flat,
        doc: CmdDoc::Parts(&parts[offset..offset + 3]),
    };
    let first_and_second_content_fits = fits(
        first_and_second_content_flat_command,
        &[],
        remaining_width,
        !line_suffix.is_empty(),
        group_modes,
        true,
        fits_stack,
    );

    commands.push(remaining_command);

    if first_and_second_content_fits {
        commands.push(whitespace_flat_command);
        commands.push(content_flat_command);
    } else if content_fits {
        commands.push(whitespace_break_command);
        commands.push(content_flat_command);
    } else {
        commands.push(whitespace_break_command);
        commands.push(content_break_command);
    }
}

/// `printResult.trim()` / `trimIndentation` (print-result.js:45): removes
/// trailing spaces and tabs (Prettier also returns how many, for a caller
/// this port does not have). Prettier trims
/// only the text written since the previous trim, but that text can never be
/// preceded by trailing whitespace (the previous trim removed it), so trimming
/// the whole buffer is equivalent.
fn trim_trailing_ws(out: &mut String) {
    let trimmed = out.trim_end_matches([' ', '\t']).len();
    out.truncate(trimmed);
}
