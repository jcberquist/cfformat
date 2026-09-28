//! Statement lists, blocks and statements: how the statements of a list are
//! separated and aligned, how a block indents and pads its body, and the
//! keyword statements (`if`, `for`, `switch`, `try`, …).

use cfdoc::builders::{
    break_parent, group, hardline, indent, line, line_suffix, line_suffix_boundary, softline,
};
use cfdoc::Doc;
use cfparse::{
    BlockKind, Element, ElementKind, Ident, Keyword, Node, Operator, Punct, SegmentKind,
    StatementKind, TokenKind,
};

use super::alignment::{self, ColumnPart, RunPart};
use super::delimited::KeyValueStyle;
use super::Printer;

/// The line breaks to print for `newlines` source newlines between two
/// nodes: as many, but never more than one blank line in a row.
pub(super) fn line_breaks(newlines: usize) -> usize {
    newlines.min(2)
}

/// The newlines a node's text ends with, past any spaces and tabs after
/// them: a brace-less arrow body holds the newlines that end its statement
/// and, inside a block, the indentation of the next line.
fn trailing_newlines(text: &str) -> usize {
    let kept = text.trim_end_matches([' ', '\t', '\n']).len();
    text[kept..].matches('\n').count()
}

/// What the last printed node of a statement list was.
#[derive(Clone, Copy, PartialEq)]
enum Prev {
    Code,
    BlockComment,
    LineComment,
}

impl Printer<'_> {
    /// The nodes of a script root, block or case body: statements, comments
    /// and blank lines, joined by hard lines. A run of *n* `Newline` tokens
    /// between two nodes is *n* hard lines, at most two (a blank line is
    /// kept, a run of them becomes one); newlines before the first and after
    /// the last node are dropped. Two
    /// statements on one line are split (`{var i = 0; i++;}`), except an
    /// empty statement right after its predecessor (`if () {};`). A comment
    /// with no newline before it shares the previous node's line.
    pub(crate) fn statements(&self, nodes: &[Node]) -> Doc {
        let pads = self.statement_pads(nodes);
        let columns = self.statement_columns(nodes);
        let mut parts = Vec::new();
        let mut prev: Option<Prev> = None;
        let mut last_code: Option<&Element> = None;
        let mut newlines = 0usize;
        for (i, n) in nodes.iter().enumerate() {
            let comment = n.as_element().filter(|e| e.kind.is_comment());
            match n {
                Node::Token(t) if t.kind == TokenKind::Whitespace => continue,
                Node::Token(t) if t.kind == TokenKind::Newline => {
                    newlines += 1;
                    continue;
                }
                _ => {}
            }
            match (prev, comment) {
                (None, _) => {}
                // `a = 1; // c`, `doThis() /* c */`.
                (Some(_), Some(c)) if newlines == 0 => {
                    parts.push(self.same_line_comment(c));
                    prev = Some(comment_prev(c));
                    continue;
                }
                (Some(p), None) if newlines == 0 => {
                    if p == Prev::BlockComment || last_code.is_some_and(is_bare_word_statement) {
                        // After a bare word with no terminator the statement
                        // continues on its line: `pageencoding "utf-8";`
                        // (Lucee) is a word and a string to the parser, and
                        // split over two lines it parses as a tag.
                        parts.push(Doc::from(" "));
                    } else if !is_empty_statement(n) {
                        parts.push(hardline());
                    }
                }
                (Some(_), _) => parts.extend((0..line_breaks(newlines)).map(|_| hardline())),
            }
            parts.push(match (pads[i], &columns[i], n) {
                (Some(spaces), _, Node::Element(e)) => {
                    self.simple_statement_with(e, Some(&alignment::pad(spaces)), None)
                }
                (None, Some(columns), Node::Element(e)) => {
                    self.simple_statement_with(e, None, Some(columns))
                }
                _ => self.node(n),
            });
            prev = Some(comment.map_or(Prev::Code, comment_prev));
            if comment.is_none() {
                last_code = n.as_element();
            }
            // A node whose text ends with newlines (an ignore region ends
            // with the newline after `cfformat-ignore-end`, a brace-less arrow
            // body keeps its newlines, also as the last part of a chain):
            // they separate it from the next node, blank lines included.
            newlines = trailing_newlines(self.tree.slice(n.span()));
        }
        Doc::Concat(parts)
    }

    /// The alignment padding of each node of a statement list
    /// (`alignment.consecutive.assignments`): a run is a maximal sequence of
    /// assignment statements ([`Printer::assignment_left`]); line comments
    /// between them do not end it, a blank line or any other node does. A run
    /// never leaves its statement list, so its members share one indent. Blank
    /// lines are counted as [`Printer::statements`] counts them.
    fn statement_pads(&self, nodes: &[Node]) -> Vec<Option<usize>> {
        if !self.opts.alignment_consecutive_assignments {
            return vec![None; nodes.len()];
        }
        let entries = self.run_entries(nodes, RunPart::Break, |n| match n {
            Node::Element(c) if c.kind == ElementKind::LineComment => RunPart::Neutral,
            Node::Element(s) => self
                .assignment_left(s)
                .map_or(RunPart::Break, alignment::member),
            Node::Token(_) => RunPart::Break,
        });
        let runs = alignment::runs(&entries, |e| e.1);
        spread(nodes.len(), &entries, runs)
    }

    /// The attribute padding of each node of a statement list
    /// (`alignment.consecutive.properties` and `.params`): a run is a
    /// maximal sequence of `property` or attribute-form `param` statements
    /// with the same attribute names ([`Printer::attribute_columns`]); a line
    /// comment between them does not end it, a blank line, any other node or
    /// a statement with other names does. A run never leaves its statement
    /// list, so its members share one indent.
    fn statement_columns(&self, nodes: &[Node]) -> Vec<Option<Vec<usize>>> {
        if !self.opts.alignment_consecutive_properties && !self.opts.alignment_consecutive_params {
            return vec![None; nodes.len()];
        }
        let entries = self.run_entries(nodes, ColumnPart::Break, |n| match n {
            Node::Element(c) if c.kind == ElementKind::LineComment => ColumnPart::Neutral,
            Node::Element(s) => self.attribute_columns(s),
            Node::Token(_) => ColumnPart::Break,
        });
        let runs = alignment::column_runs(&entries, |e| e.1.clone());
        spread(nodes.len(), &entries, runs)
    }

    /// The entries of a statement list an alignment run walks: each node but
    /// whitespace and newlines, with its index and its part, and `blank`
    /// (with no index) before a node that follows a blank line. Blank lines
    /// are counted as [`Printer::statements`] counts them.
    fn run_entries<P: Clone>(
        &self,
        nodes: &[Node],
        blank: P,
        part_of: impl Fn(&Node) -> P,
    ) -> Vec<(Option<usize>, P)> {
        let mut entries = Vec::new();
        let mut newlines = 0usize;
        for (i, n) in nodes.iter().enumerate() {
            match n {
                Node::Token(t) if t.kind == TokenKind::Whitespace => continue,
                Node::Token(t) if t.kind == TokenKind::Newline => {
                    newlines += 1;
                    continue;
                }
                _ => {}
            }
            if newlines > 1 && !entries.is_empty() {
                entries.push((None, blank.clone()));
            }
            entries.push((Some(i), part_of(n)));
            newlines = trailing_newlines(self.tree.slice(n.span()));
        }
        entries
    }

    /// A statement as a member of an attribute run: `property …;` (with
    /// `alignment.consecutive.properties`) or `param …;` written with
    /// attributes only (with `.params`), keyed by its kind and attribute names
    /// (a bare attribute such as `required` is a name), with the flat width of
    /// each attribute as printed, and whether the statement fits on one line
    /// unpadded at this indent (`Printer::indent_columns`): a statement that
    /// breaks anyway, by width or by its thresholds, is never padded, so its
    /// widths must not set the columns the others align to — otherwise one
    /// long statement would push a shorter one that fits into breaking for a
    /// column no flat line occupies. Only a statement that is its keyword
    /// followed by attributes and nothing else is a member, so a statement
    /// with other words (`property string name;`, `param string x = 1;`, which
    /// is an assignment), a comment anywhere inside it, or an attribute that
    /// cannot print on one line ends a run.
    fn attribute_columns(&self, stmt: &Element) -> ColumnPart {
        let (enabled, kind) = match stmt.kind {
            ElementKind::Statement(StatementKind::Property) => {
                (self.opts.alignment_consecutive_properties, "property")
            }
            ElementKind::Statement(StatementKind::Param) => {
                (self.opts.alignment_consecutive_params, "param")
            }
            _ => return ColumnPart::Break,
        };
        if !enabled || contains_comment(stmt) {
            return ColumnPart::Break;
        }
        fn significant(e: &Element) -> Vec<&Node> {
            e.children.iter().filter(|n| !n.is_trivia()).collect()
        }
        let tag = match significant(stmt)[..] {
            [Node::Element(t)] if matches!(t.kind, ElementKind::Property | ElementKind::Param) => t,
            _ => return ColumnPart::Break,
        };
        let parts = significant(tag);
        let Some((Node::Token(name), attributes)) = parts.split_first().map(|(f, r)| (*f, r))
        else {
            return ColumnPart::Break;
        };
        if !matches!(name.kind, TokenKind::Ident(Ident::TagName)) || attributes.is_empty() {
            return ColumnPart::Break;
        }
        let mut names = vec![kind.to_string()];
        let mut widths = Vec::new();
        for &n in attributes {
            let (key, doc) = match n {
                Node::Element(kv) if kv.kind == ElementKind::KeyValue => {
                    let Some(view) = kv.as_key_value() else {
                        return ColumnPart::Break;
                    };
                    (
                        self.tree.slice(view.key().span()),
                        self.key_value(kv, KeyValueStyle::Attribute),
                    )
                }
                Node::Token(t) if Self::is_attribute(n) => (self.tree.text(t), self.token(t)),
                _ => return ColumnPart::Break,
            };
            let Some(width) = alignment::one_line_width(&doc) else {
                return ColumnPart::Break;
            };
            names.push(key.to_string());
            widths.push(width);
        }
        let fits = alignment::one_line_width(&self.simple_statement_with(stmt, None, None))
            .is_some_and(|width| self.indent_columns() + width <= self.opts.max_columns);
        ColumnPart::Member {
            names,
            widths,
            fits,
        }
    }

    /// The left side of an assignment statement, printed, when the statement
    /// can join an alignment run: `x = …`, `var x = …` or `param [type] x =
    /// …` with a plain `=` (an augmented `+=` ends a run) whose target is a
    /// variable, a string or a chain of property and index segments with no
    /// call anywhere in it, and no comment inside. The left side is
    /// everything before the operator: `var a`, `param string a`,
    /// `slide['imageURL']`.
    pub(crate) fn assignment_left(&self, stmt: &Element) -> Option<Doc> {
        let has_comment = |e: &Element| {
            e.children
                .iter()
                .any(|n| n.as_element().is_some_and(|c| c.kind.is_comment()))
        };
        if has_comment(stmt) {
            return None;
        }
        fn sig(e: &Element) -> Vec<&Node> {
            e.children.iter().filter(|n| !n.is_trivia()).collect()
        }
        let stmt_parts = sig(stmt);
        let (prefix, assignment): (Vec<&Node>, &Element) = match (&stmt.kind, &stmt_parts[..]) {
            (ElementKind::Statement(StatementKind::Assignment), [Node::Element(a)]) => {
                (Vec::new(), a)
            }
            (
                ElementKind::Statement(StatementKind::Declaration),
                [words @ .., Node::Element(a)],
            ) if !words.is_empty()
                && words.iter().all(
                    |w| matches!(w, Node::Token(t) if t.kind == TokenKind::Keyword(Keyword::Var)),
                ) =>
            {
                (words.to_vec(), a)
            }
            (ElementKind::Statement(StatementKind::Param), [Node::Element(p)])
                if p.kind == ElementKind::Param && !has_comment(p) =>
            {
                match &sig(p)[..] {
                    [words @ .., Node::Element(a)]
                        if !words.is_empty() && words.iter().all(|w| w.as_token().is_some()) =>
                    {
                        (words.to_vec(), a)
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        let view = assignment.as_assignment()?;
        if assignment.kind != ElementKind::Assignment
            || has_comment(assignment)
            || view.op().kind != TokenKind::Operator(Operator::Assign)
            || !is_plain_target(view.target())
        {
            return None;
        }
        let mut left = self.sequence(prefix);
        if !left.is_empty() {
            left.push(Doc::from(" "));
        }
        left.push(self.node(view.target()));
        Some(Doc::Concat(left))
    }

    /// A comment on the same line as the code before it: a line comment is
    /// a line suffix that breaks the enclosing group, a block comment prints
    /// in place after one space. The boundary first moves a line comment
    /// that is still pending to its own line, so two line comments never
    /// share one suffix (`// a // b` would turn the second into text of the
    /// first).
    pub(crate) fn same_line_comment(&self, c: &Element) -> Doc {
        if c.kind == ElementKind::LineComment {
            Doc::Concat(vec![
                line_suffix_boundary(),
                line_suffix(vec![Doc::from(" "), self.comment(c)]),
                break_parent(),
            ])
        } else {
            Doc::Concat(vec![Doc::from(" "), self.comment(c)])
        }
    }

    /// A comment on the same line as the code before it, where the layout
    /// puts no line break after it: a line comment is a line suffix that
    /// breaks nothing, so it prints at the end of whatever line the code
    /// around it ends up on (`a ? b : // c` newline `d` prints
    /// `a ? b : d; // c`); a block comment prints as in
    /// [`Printer::same_line_comment`]. A `break_parent` here would break the
    /// enclosing group without putting the comment on a line of its own:
    /// the comment would still move to the end of the line, and the next
    /// run, reading it there, would not break the group.
    pub(crate) fn deferred_comment(&self, c: &Element) -> Doc {
        if c.kind == ElementKind::LineComment {
            Doc::Concat(vec![
                line_suffix_boundary(),
                line_suffix(vec![Doc::from(" "), self.comment(c)]),
            ])
        } else {
            self.same_line_comment(c)
        }
    }

    /// Comments at the start of `nodes` that sit on the line before them
    /// (after `{` or `case x:`), printed as same-line comments, and the rest
    /// of the nodes.
    pub(crate) fn leading_same_line_comments<'n>(&self, nodes: &'n [Node]) -> (Doc, &'n [Node]) {
        let mut parts = Vec::new();
        let mut rest = nodes.len();
        for (i, n) in nodes.iter().enumerate() {
            match n {
                Node::Token(t) if t.kind == TokenKind::Whitespace => {}
                Node::Element(c) if c.kind.is_comment() => parts.push(self.same_line_comment(c)),
                _ => {
                    rest = i;
                    break;
                }
            }
        }
        (Doc::Concat(parts), &nodes[rest..])
    }

    /// `{ … }`: the body indented on its own lines; an empty block is `{`
    /// hard line `}` (never `{}`). A component or interface body gets a blank
    /// line after `{` and before `}`; an empty one gets one blank line, not
    /// two: never more than one in a row.
    pub(crate) fn block(&self, e: &Element) -> Doc {
        let padded = matches!(
            e.kind,
            ElementKind::Block(BlockKind::Class | BlockKind::Interface)
        );
        let (suffix, rest) = self.leading_same_line_comments(&e.children);
        let body = self.indented(|| self.statements(rest));
        let mut parts = vec![self.token(e.open.as_ref().expect("braced block")), suffix];
        if body.is_empty() {
            parts.push(hardline());
            if padded {
                parts.push(hardline());
            }
        } else {
            let mut inner = vec![hardline()];
            if padded {
                inner.push(hardline());
            }
            inner.push(body);
            parts.push(indent(inner));
            parts.push(hardline());
            if padded {
                parts.push(hardline());
            }
        }
        if let Some(close) = &e.close {
            parts.push(self.token(close));
        }
        Doc::Concat(parts)
    }

    /// Children printed in order, then the terminator iff the source has one
    /// (semicolons are preserved, never inserted). A keyword
    /// statement is its keyword element (and a label, or a bare block); only
    /// `do … while (x);` and the like carry a terminator. The terminator
    /// follows the last part, before any comment that trails it (`foo();`
    /// newline `// c`, not `// c;`). A chain that is the statement's whole
    /// content may keep its first method on line 0 (`Printer::chain_with`).
    pub(crate) fn simple_statement(&self, e: &Element) -> Doc {
        self.simple_statement_with(e, None, None)
    }

    /// [`Printer::simple_statement`] with alignment padding after the left
    /// side of the statement's assignment (see [`Printer::assignment_left`]),
    /// or after each attribute of its `property` / `param` (see
    /// [`Printer::attribute_columns`]).
    fn simple_statement_with(
        &self,
        e: &Element,
        pad: Option<&Doc>,
        columns: Option<&[usize]>,
    ) -> Doc {
        let significant = || e.children.iter().filter(|n| !n.is_trivia());
        let last = significant().rfind(|n| !n.as_element().is_some_and(|c| c.kind.is_comment()));
        // Comments after the last part (before the terminator, or at the
        // end of a statement without one) have nothing after them but the
        // statement list's line break: a line comment on the last part's
        // line ends that line and breaks nothing, so an unbraced body
        // (`if (a) x = 1 // c` newline `;`) stays on its keyword's line.
        let end = last.map_or(0, |l| {
            e.children
                .iter()
                .position(|n| std::ptr::eq(n, l))
                .map_or(e.children.len(), |i| i + 1)
        });
        let (children, trailing) = e.children.split_at(end);
        let terminator = e.terminator().map(|t| self.token(t));
        let chain = match (&e.kind, significant().next(), significant().nth(1)) {
            (ElementKind::Statement(StatementKind::Expression), Some(first), None)
                if first
                    .as_element()
                    .is_some_and(|c| c.kind == ElementKind::Chain) =>
            {
                Some(first)
            }
            _ => None,
        };
        // `return` / `throw` with a binary value ([`Printer::binary_return`]).
        let returns = e.kind == ElementKind::Statement(StatementKind::Flow)
            && matches!(significant().next(), Some(Node::Token(t))
                if matches!(t.kind, TokenKind::Keyword(Keyword::Return | Keyword::Throw)));
        let mut parts = self.sequence_by(children, &|n| {
            let padded = |a: &Element| self.assignment_with(a, pad.cloned());
            let mut doc = match n.as_element() {
                Some(c) if chain.is_some_and(|f| std::ptr::eq(f, n)) => self.chain_with(c, true),
                Some(b) if returns && matches!(b.kind, ElementKind::Binary { .. }) => {
                    self.binary_return(b)
                }
                Some(t) if returns && t.kind == ElementKind::Ternary => {
                    self.ternary_cond_indent.set(true);
                    self.element(t)
                }
                Some(t)
                    if columns.is_some()
                        && matches!(t.kind, ElementKind::Property | ElementKind::Param) =>
                {
                    self.tag_statement_with(t, columns)
                }
                Some(a) if pad.is_some() && a.kind == ElementKind::Assignment => padded(a),
                // `param [type] name = value`: the words and the assignment.
                Some(p) if pad.is_some() && p.kind == ElementKind::Param => {
                    Doc::Concat(self.sequence_by(&p.children, &|m| match m {
                        Node::Element(a) if a.kind == ElementKind::Assignment => padded(a),
                        m => self.node(m),
                    }))
                }
                _ => self.node(n),
            };
            if last.is_some_and(|l| std::ptr::eq(l, n)) {
                if let Some(t) = &terminator {
                    doc = Doc::Concat(vec![doc, t.clone()]);
                }
            }
            doc
        });
        if last.is_none() {
            parts.extend(terminator);
        }
        // An import's comments from its first line comment on are printed
        // here, after the `;` ([`Printer::import`]).
        let hoisted = match last.and_then(Node::as_element) {
            Some(i) if i.kind == ElementKind::Import => {
                &i.children[super::components::import_trailing(i)..]
            }
            _ => &[],
        };
        let mut newlines = 0;
        let mut after_line_comment = false;
        for n in hoisted.iter().chain(trailing) {
            match n {
                Node::Token(t) if t.kind == TokenKind::Newline => newlines += 1,
                Node::Element(c) if c.kind.is_comment() => {
                    // A line comment on a line of its own in the source,
                    // and anything after a line comment: its own line, as
                    // `sequence` prints them.
                    let own_line = c.kind == ElementKind::LineComment && newlines > 0;
                    if own_line || after_line_comment {
                        parts.extend([hardline(), self.comment(c)]);
                    } else {
                        parts.push(self.deferred_comment(c));
                    }
                    after_line_comment = c.kind == ElementKind::LineComment;
                    newlines = 0;
                }
                _ => {}
            }
        }
        Doc::Concat(parts)
    }

    /// Significant nodes of a run the tree leaves unstructured (a simple
    /// statement's children, modifiers before a parameter, the tokens between
    /// a tag name and its attributes, a label and its colon, a `for` clause),
    /// each through [`Printer::node`], joined by exactly one space — except
    /// before a colon, comma, terminator or closing delimiter, after an
    /// opening delimiter, between an operand and a postfix part (`Call`,
    /// `Brackets`) and around a member accessor, which keep a run the
    /// expression pass could not parse glued (`isNull(o) ? new () : o`).
    /// Source whitespace is only read next to an `other` or `invalid` token:
    /// text the parser could not classify keeps the spacing it was written
    /// with. A line comment becomes a line suffix, or its own line when
    /// nothing precedes it.
    pub(crate) fn sequence<'n>(&self, nodes: impl IntoIterator<Item = &'n Node>) -> Vec<Doc> {
        self.sequence_by(nodes, &|n| self.node(n))
    }

    /// [`Printer::sequence`] with each significant node printed by `print`.
    pub(crate) fn sequence_by<'n>(
        &self,
        nodes: impl IntoIterator<Item = &'n Node>,
        print: &dyn Fn(&Node) -> Doc,
    ) -> Vec<Doc> {
        let mut parts = Vec::new();
        let mut prev: Option<&Node> = None;
        let (mut newlines, mut after_line_comment) = (0usize, false);
        let mut after_block_comment = false;
        for n in nodes {
            match n {
                Node::Token(t) if t.kind == TokenKind::Newline => {
                    newlines += 1;
                    continue;
                }
                Node::Token(t) if t.kind == TokenKind::Whitespace => continue,
                // Before anything is printed (`(` newline `// c` newline
                // `a)`): its own line, so it cannot trail the next part.
                Node::Element(c) if c.kind == ElementKind::LineComment && prev.is_none() => {
                    parts.push(self.comment(c));
                    parts.push(hardline());
                    newlines = 0;
                    continue;
                }
                // After a part: on that part's line when it was there, else
                // on its own line; either way the next part starts a new line
                // (a line suffix alone would carry the comment past it).
                Node::Element(c) if c.kind == ElementKind::LineComment => {
                    if newlines > 0 {
                        parts.push(hardline());
                        parts.push(self.comment(c));
                    } else {
                        parts.push(self.same_line_comment(c));
                    }
                    (newlines, after_line_comment) = (0, true);
                    continue;
                }
                _ => {}
            }
            // A block comment keeps the line breaks that followed it, as a
            // statement list does: `<!--- c --->` before a struct
            // member, and a blank line between `*/` and the next part.
            let breaks = if after_block_comment {
                line_breaks(newlines)
            } else {
                0
            };
            newlines = 0;
            if after_line_comment {
                parts.push(hardline());
                after_line_comment = false;
            } else if breaks > 0 {
                parts.extend((0..breaks).map(|_| hardline()));
            } else if let Some(p) = prev.filter(|p| is_unclassified(p) && is_unclassified(n)) {
                // Inside a run of unclassified tokens the whitespace is kept
                // as written, newlines included: the run may hold
                // a string's delimiters, and its text is the string's.
                parts.push(self.text(self.tree.slice(p.span().end..n.span().start)));
            } else if let Some(p) = prev {
                let space = if is_unclassified(p) || is_unclassified(n) {
                    p.span().end < n.span().start
                } else {
                    !is_opening(p)
                        && !is_accessor(p)
                        && !is_tight_before(n)
                        && !(is_postfix_part(n) && is_operand(p))
                };
                if space {
                    parts.push(Doc::from(" "));
                }
            }
            parts.push(print(n));
            after_block_comment = n.as_element().is_some_and(|c| c.kind.is_comment());
            prev = Some(n);
        }
        parts
    }

    /// `if`, `else if`, `else`, `for`, `while`, `do … while`, `switch`, `try`,
    /// `catch`, `finally`: the keyword, one space, the group, one space, the
    /// body; a chained clause (`else`, `catch`, `finally`, the `while` of a
    /// `do`) follows `}` after one space. An unbraced body goes on the same
    /// line when it fits, else indented on the next
    /// (`group([header, indent([line, body])])`). A clause after an unbraced
    /// body starts a new line. A comment between a body and the next clause
    /// stays where it was: on the `}` line when it was there in the source,
    /// else on its own line, and the clause then starts a new line.
    pub(crate) fn keyword(&self, e: &Element) -> Doc {
        let mut head: Vec<Doc> = Vec::new();
        let mut unbraced_body = false;
        let mut comment_before_clause = false;
        let mut newlines = 0usize;
        for n in &e.children {
            match n {
                Node::Token(t) if t.kind == TokenKind::Whitespace => continue,
                Node::Token(t) if t.kind == TokenKind::Newline => {
                    newlines += 1;
                    continue;
                }
                // The keyword, or the `while` of a `do`.
                Node::Token(t) => {
                    if unbraced_body {
                        head.push(hardline());
                    } else if !head.is_empty() {
                        head.push(Doc::from(" "));
                    }
                    head.push(self.token(t));
                }
                Node::Element(c) if c.kind.is_comment() => {
                    if newlines == 0 && !head.is_empty() {
                        head.push(self.same_line_comment(c));
                    } else {
                        head.push(hardline());
                        head.push(self.comment(c));
                    }
                    comment_before_clause = true;
                }
                // An ignore region between a body and the next clause (or
                // after the last body) always starts its own line.
                Node::Element(i) if i.kind == ElementKind::Ignore => {
                    head.push(hardline());
                    head.push(self.node(n));
                    comment_before_clause = true;
                }
                Node::Element(g) if g.kind == ElementKind::Group => {
                    head.push(Doc::from(" "));
                    head.push(if e.kind == ElementKind::For {
                        self.for_header(g)
                    } else {
                        self.group(g)
                    });
                }
                Node::Element(b) if matches!(b.kind, ElementKind::Block(_)) => {
                    head.push(Doc::from(" "));
                    head.push(self.node(n));
                    unbraced_body = false;
                    comment_before_clause = false;
                }
                Node::Element(s) if s.kind.is_statement() => {
                    let header = Doc::Concat(std::mem::take(&mut head));
                    head.push(group(vec![header, indent(vec![line(), self.node(n)])]));
                    unbraced_body = true;
                    comment_before_clause = false;
                }
                // The next clause: `else`, `else if`, `catch`, `finally`.
                Node::Element(clause) => {
                    if unbraced_body || comment_before_clause {
                        head.push(hardline());
                    } else if !head.is_empty() {
                        head.push(Doc::from(" "));
                    }
                    head.push(self.node(n));
                    unbraced_body = clause
                        .children
                        .iter()
                        .rfind(|c| !c.is_trivia())
                        .and_then(Node::as_element)
                        .is_some_and(|b| b.kind.is_statement());
                    comment_before_clause = false;
                }
            }
            newlines = 0;
        }
        Doc::Concat(head)
    }

    /// `for (init; condition; step)`: the clauses on one line, `; ` between
    /// them (`for (;;)` stays tight; `for_loop_semicolons.padding` is a
    /// removed key). A header too wide for its line breaks as a group, each
    /// clause on its own line inside the parentheses (Prettier's `ForStatement`:
    /// `for (` ⏎ `init;` ⏎ `condition;` ⏎ `step` ⏎ `)`), so no clause is split
    /// at a column that reads as the body's. A `for (k in s)` header is
    /// one clause holding `k in s`, printed flat like an assignment: the
    /// collection breaks on its own (`for (k in [` ⏎ … ⏎ `])`), never after
    /// the `in` (Prettier's `ForInStatement`). With `parentheses.padding` a
    /// header with any content is padded like every keyword group
    /// (`for ( ; i < 3; )`); `for (;;)` is empty and
    /// stays `()`-tight.
    fn for_header(&self, g: &Element) -> Doc {
        let mut parts = Vec::new();
        let mut after_separator = false;
        let mut has_content = false;
        let for_in = g.items.len() == 1;
        for item in &g.items {
            let mut nodes: Vec<&Node> = item.nodes().collect();
            nodes.sort_by_key(|n| n.span().start);
            let content = self.sequence_by(nodes, &|n| match n.as_element() {
                Some(b) if for_in && self.is_in_binary(b) => self.assignment(b),
                _ => self.node(n),
            });
            if after_separator && !content.is_empty() {
                parts.push(if for_in { Doc::from(" ") } else { line() });
            }
            has_content |= !content.is_empty();
            parts.extend(content);
            after_separator = false;
            if let Some(sep) = &item.separator {
                parts.push(self.token(sep));
                after_separator = true;
            }
        }
        let pad = self.opts.parentheses_padding && has_content;
        let mut header = vec![self.token(g.open.as_ref().expect("for group"))];
        if for_in || !has_content {
            if pad {
                header.push(Doc::from(" "));
            }
            header.extend(parts);
            if pad {
                header.push(Doc::from(" "));
            }
        } else {
            let edge = || if pad { line() } else { softline() };
            header.push(group(vec![
                indent(vec![edge(), Doc::Concat(parts)]),
                edge(),
            ]));
        }
        if let Some(close) = &g.close {
            header.push(self.token(close));
        }
        Doc::Concat(header)
    }

    /// `k in s`: a binary whose only operator is `in`.
    fn is_in_binary(&self, e: &Element) -> bool {
        e.as_binary().is_some_and(|b| {
            let mut ops = b.operators();
            matches!(ops.next(), Some(Node::Token(t)) if t.kind == TokenKind::Operator(Operator::In))
                && ops.next().is_none()
        })
    }

    /// `case x:` / `default:` and the statements up to the next case: the
    /// statements indented on the following lines, or a block on the case line
    /// (`case '2': {`).
    pub(crate) fn case(&self, e: &Element) -> Doc {
        let colon = e
            .children
            .iter()
            .position(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Punct(Punct::Colon)));
        let Some(colon) = colon else {
            return self.as_written(e);
        };
        let mut parts = Vec::new();
        let (keyword, value) = e.children[..colon].split_first().expect("case keyword");
        parts.push(self.node(keyword));
        let value = self.sequence(value);
        if !value.is_empty() {
            parts.push(Doc::from(" "));
            parts.extend(value);
        }
        parts.push(self.node(&e.children[colon]));
        let (suffix, rest) = self.leading_same_line_comments(&e.children[colon + 1..]);
        parts.push(suffix);
        let block_on_case_line = rest.first().and_then(Node::as_element).is_some_and(|s| {
            s.kind == ElementKind::Statement(StatementKind::Keyword)
                && matches!(
                    s.children.iter().find(|c| !c.is_trivia()),
                    Some(Node::Element(b)) if matches!(b.kind, ElementKind::Block(_))
                )
        });
        if block_on_case_line {
            parts.push(Doc::from(" "));
            parts.push(self.statements(rest));
        } else {
            let body = self.indented(|| self.statements(rest));
            if !body.is_empty() {
                parts.push(indent(vec![hardline(), body]));
            }
        }
        Doc::Concat(parts)
    }
}

/// An assignment target that can be aligned: an identifier or unquoted
/// token, a string, or a chain of property and index segments with no call
/// anywhere in it.
fn is_plain_target(target: &Node) -> bool {
    fn has_call(e: &Element) -> bool {
        matches!(
            e.kind,
            ElementKind::Call | ElementKind::CallExpr | ElementKind::New
        ) || e.nodes().filter_map(Node::as_element).any(has_call)
    }
    match target {
        Node::Token(t) => matches!(
            t.kind,
            TokenKind::Ident(_) | TokenKind::Literal(cfparse::Literal::Unquoted)
        ),
        Node::Element(e) => match e.kind {
            ElementKind::String { .. } => !has_call(e),
            ElementKind::Chain => {
                !has_call(e)
                    && e.as_chain().is_some_and(|c| {
                        c.segments().all(|s| {
                            matches!(
                                s.as_segment().map(|s| s.kind()),
                                Some(SegmentKind::Property | SegmentKind::Index)
                            )
                        })
                    })
            }
            _ => false,
        },
    }
}

fn comment_prev(c: &Element) -> Prev {
    if c.kind == ElementKind::LineComment {
        Prev::LineComment
    } else {
        Prev::BlockComment
    }
}

impl Printer<'_> {}

/// Per node of a statement list, the run result of its entry (blank-line
/// entries have no node).
fn spread<R>(
    len: usize,
    entries: &[(Option<usize>, impl Sized)],
    runs: Vec<Option<R>>,
) -> Vec<Option<R>> {
    let mut out: Vec<Option<R>> = (0..len).map(|_| None).collect();
    for ((i, _), run) in entries.iter().zip(runs) {
        if let Some(i) = i {
            out[*i] = run;
        }
    }
    out
}

/// Whether `e` holds a comment at any depth.
fn contains_comment(e: &Element) -> bool {
    e.children.iter().any(|n| {
        n.as_element()
            .is_some_and(|c| c.kind.is_comment() || contains_comment(c))
    })
}

fn is_empty_statement(n: &Node) -> bool {
    n.as_element()
        .is_some_and(|e| e.kind == ElementKind::Statement(StatementKind::Empty))
}

/// A node that takes no space before it in a [`Printer::sequence`].
fn is_tight_before(n: &Node) -> bool {
    match n {
        Node::Token(t) => {
            matches!(
                t.kind,
                TokenKind::Punct(
                    Punct::Colon
                        | Punct::Comma
                        | Punct::Terminator
                        | Punct::EmptyTerminator
                        | Punct::ExprSeparator
                        | Punct::Close(_)
                )
            ) || is_accessor(n)
        }
        Node::Element(_) => false,
    }
}

/// Arguments or index brackets, which glue to the operand before them.
fn is_postfix_part(n: &Node) -> bool {
    n.as_element()
        .is_some_and(|e| matches!(e.kind, ElementKind::Call | ElementKind::Brackets))
}

/// What a postfix part can follow: an identifier, literal or type token
/// (`User[] function`), `component` (an inline component's arguments,
/// `new component(1) {}`), a closing delimiter, or an element other than a
/// comment.
fn is_operand(n: &Node) -> bool {
    match n {
        Node::Token(t) => matches!(
            t.kind,
            TokenKind::Ident(_)
                | TokenKind::Literal(_)
                | TokenKind::Storage(cfparse::Storage::Type)
                | TokenKind::Keyword(cfparse::Keyword::Component)
                | TokenKind::Punct(Punct::Close(_))
        ),
        Node::Element(e) => !e.kind.is_comment(),
    }
}

/// `.`, `?.`, `::`.
fn is_accessor(n: &Node) -> bool {
    n.as_token().is_some_and(|t| {
        matches!(
            t.kind,
            TokenKind::Punct(Punct::Accessor | Punct::SafeAccessor | Punct::StaticAccessor)
        )
    })
}

/// A token the parser could not classify.
fn is_unclassified(n: &Node) -> bool {
    n.as_token()
        .is_some_and(|t| matches!(t.kind, TokenKind::Other | TokenKind::Invalid))
}

/// An opening delimiter token, which takes no space after it.
fn is_opening(n: &Node) -> bool {
    n.as_token()
        .is_some_and(|t| matches!(t.kind, TokenKind::Punct(Punct::Open(_))))
}

/// An expression statement with no terminator that is one identifier, or
/// one token the parser could not classify (`<` of a stray
/// `</cfscript>`).
fn is_bare_word_statement(e: &Element) -> bool {
    e.kind == ElementKind::Statement(StatementKind::Expression)
        && e.close.is_none()
        && matches!(
            e.children.iter().filter(|n| !n.is_trivia()).collect::<Vec<_>>()[..],
            [Node::Token(t)] if matches!(
                t.kind,
                TokenKind::Ident(Ident::Variable) | TokenKind::Other | TokenKind::Invalid
            )
        )
}
