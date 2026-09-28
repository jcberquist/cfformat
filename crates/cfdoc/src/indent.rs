//! Indentation state (`prettier/src/document/printer/indent.js`).

use std::rc::Rc;

use crate::doc::Align;

/// Indentation unit. Prettier's `{useTabs, tabWidth}`: under `Tabs(n)` a tab
/// counts as `n` columns when measuring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndentStyle {
    Tabs(usize),
    Spaces(usize),
}

impl IndentStyle {
    fn use_tabs(self) -> bool {
        matches!(self, IndentStyle::Tabs(_))
    }

    fn tab_width(self) -> usize {
        match self {
            IndentStyle::Tabs(n) | IndentStyle::Spaces(n) => n,
        }
    }
}

/// One entry of an indent queue (`INDENT_COMMAND_*`); a dedent is not
/// stored, it pops the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentCmd<'a> {
    Indent,
    Width(usize),
    Str(&'a str),
}

/// The indentation of a printer command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indent<'a> {
    /// What a hard line writes after the newline.
    pub value: String,
    /// Width of `value` in columns (Prettier's `length`).
    pub width: usize,
    pub queue: Vec<IndentCmd<'a>>,
    /// The indent a literal line and `dedentToRoot` return to. `None` is
    /// `ROOT_INDENT` itself (empty), whose `root` getter returns itself.
    pub root: Option<Rc<Indent<'a>>>,
}

/// `ROOT_INDENT`.
pub fn root_indent<'a>() -> Rc<Indent<'a>> {
    Rc::new(Indent {
        value: String::new(),
        width: 0,
        queue: Vec::new(),
        root: None,
    })
}

/// `generateIndent(indent, command, options)` (`indent.js:57`); `None` is
/// `INDENT_COMMAND_DEDENT`.
pub fn generate_indent<'a>(
    indent: &Indent<'a>,
    command: Option<IndentCmd<'a>>,
    style: IndentStyle,
) -> Indent<'a> {
    let queue = match command {
        None => indent.queue[..indent.queue.len().saturating_sub(1)].to_vec(),
        Some(command) => {
            let mut queue = Vec::with_capacity(indent.queue.len() + 1);
            queue.extend_from_slice(&indent.queue);
            queue.push(command);
            queue
        }
    };

    let mut out = Gen {
        style,
        value: String::new(),
        width: 0,
        last_tabs: 0,
        last_spaces: 0,
    };
    for command in &queue {
        match *command {
            IndentCmd::Indent => {
                out.flush();
                // indent.js:72
                if style.use_tabs() {
                    out.add_tabs(1);
                } else {
                    out.add_spaces(style.tab_width());
                }
            }
            IndentCmd::Str(string) => {
                out.flush();
                out.value.push_str(string);
                // `string.length` (UTF-16 units), not the display width.
                out.width += string.encode_utf16().count();
            }
            IndentCmd::Width(width) => {
                out.last_tabs += 1;
                out.last_spaces += width;
            }
        }
    }
    out.flush_spaces();

    Indent {
        value: out.value,
        width: out.width,
        queue,
        root: indent.root.clone(),
    }
}

struct Gen {
    style: IndentStyle,
    value: String,
    width: usize,
    last_tabs: usize,
    last_spaces: usize,
}

impl Gen {
    fn add_tabs(&mut self, count: usize) {
        self.value.extend(std::iter::repeat_n('\t', count));
        self.width += self.style.tab_width() * count;
    }

    fn add_spaces(&mut self, count: usize) {
        self.value.extend(std::iter::repeat_n(' ', count));
        self.width += count;
    }

    /// indent.js:111 — under tabs, pending aligns become one tab each when
    /// followed by an indent or string; trailing aligns stay spaces.
    fn flush(&mut self) {
        if self.style.use_tabs() {
            self.flush_tabs();
        } else {
            self.flush_spaces();
        }
    }

    fn flush_tabs(&mut self) {
        if self.last_tabs > 0 {
            self.add_tabs(self.last_tabs);
        }
        self.reset_last();
    }

    fn flush_spaces(&mut self) {
        if self.last_spaces > 0 {
            self.add_spaces(self.last_spaces);
        }
        self.reset_last();
    }

    fn reset_last(&mut self) {
        self.last_tabs = 0;
        self.last_spaces = 0;
    }
}

/// `makeIndent(indent, options)`.
pub fn make_indent<'a>(indent: &Rc<Indent<'a>>, style: IndentStyle) -> Rc<Indent<'a>> {
    Rc::new(generate_indent(indent, Some(IndentCmd::Indent), style))
}

/// `makeAlign(indent, n, options)` (`indent.js:145`).
pub fn make_align<'a>(
    indent: &Rc<Indent<'a>>,
    align: &'a Align,
    style: IndentStyle,
) -> Rc<Indent<'a>> {
    let command = match align {
        // `!indentOptions`
        Align::Width(0) => return indent.clone(),
        Align::Str(s) if s.is_empty() => return indent.clone(),
        Align::MarkRoot => {
            return Rc::new(Indent {
                root: Some(indent.clone()),
                ..(**indent).clone()
            })
        }
        Align::DedentToRoot => return indent.root.clone().unwrap_or_else(root_indent),
        Align::Dedent => None,
        Align::Width(n) => Some(IndentCmd::Width(*n)),
        Align::Str(s) => Some(IndentCmd::Str(s)),
    };
    Rc::new(generate_indent(indent, command, style))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply<'a>(aligns: &'a [Option<Align>], style: IndentStyle) -> Rc<Indent<'a>> {
        let mut ind = root_indent();
        for a in aligns {
            ind = match a {
                None => make_indent(&ind, style),
                Some(a) => make_align(&ind, a, style),
            };
        }
        ind
    }

    #[test]
    fn value_and_width() {
        let w = |n| Some(Align::Width(n));
        let s = |v: &'static str| Some(Align::from(v));
        let cases: &[(&[Option<Align>], IndentStyle, &str, usize)] = &[
            (&[None, None], IndentStyle::Spaces(2), "    ", 4),
            (&[None, None], IndentStyle::Tabs(4), "\t\t", 8),
            // Trailing aligns are spaces even under tabs.
            (&[None, w(3)], IndentStyle::Tabs(4), "\t   ", 7),
            // An align followed by an indent becomes one tab per align.
            (&[w(3), w(1), None], IndentStyle::Tabs(4), "\t\t\t", 12),
            (&[w(3), None], IndentStyle::Spaces(2), "     ", 5),
            (&[s("> "), None], IndentStyle::Tabs(4), "> \t", 6),
            (&[None, Some(Align::Dedent)], IndentStyle::Spaces(2), "", 0),
            (
                &[None, w(2), Some(Align::Dedent)],
                IndentStyle::Spaces(2),
                "  ",
                2,
            ),
            (&[None, w(0), s("")], IndentStyle::Spaces(2), "  ", 2),
        ];
        for (aligns, style, value, width) in cases {
            let ind = apply(aligns, *style);
            assert_eq!(
                (ind.value.as_str(), ind.width),
                (*value, *width),
                "{aligns:?} {style:?}"
            );
        }
    }

    #[test]
    fn root_tracking() {
        let style = IndentStyle::Spaces(2);
        let aligns = [None, Some(Align::MarkRoot), None, Some(Align::DedentToRoot)];
        assert_eq!(apply(&aligns, style).value, "  ");
        let aligns = [None, None, Some(Align::DedentToRoot)];
        assert_eq!(apply(&aligns, style).value, "");
    }
}
