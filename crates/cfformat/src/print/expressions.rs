//! Expressions over the parse tree's expression nodes.

use cfdoc::builders::{
    align, break_parent, conditional_group, group, group_opts, hardline, if_break, indent,
    indent_if_break, line, softline, GroupOpts,
};
use cfdoc::utils::{find_in_doc, remove_lines, will_break};
use cfdoc::Doc;
use cfparse::{Element, ElementKind, Literal, Node, Operator, Prec, TokenKind};

use super::delimited::printable_items;
use super::Printer;

impl Printer<'_> {
    /// `( … )`: flat when it fits, else the contents indented on their own
    /// lines; `parentheses.padding` adds a space inside a flat group. An empty
    /// group is `()`. A line comment inside breaks the group through its
    /// `break_parent`.
    pub(crate) fn group(&self, e: &Element) -> Doc {
        let (Some(open), Some(close)) = (&e.open, &e.close) else {
            return self.as_written(e);
        };
        let mut sig = e.children.iter().filter(|n| !n.is_trivia());
        let only = match (sig.next(), sig.next()) {
            (Some(Node::Element(c)), None) => Some(c),
            _ => None,
        };
        let commented = e
            .children
            .iter()
            .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()));
        let content = match only.map(|c| &c.kind) {
            // A binary that is the group's only content is not indented
            // again, and its line breaks are the group's own: when the group
            // breaks, every operator of the chain breaks with it (Prettier's
            // `isInsideParenthesis`), so a broken `if (` never keeps a long
            // `a && b` on one line between its parentheses.
            Some(ElementKind::Binary { .. }) => self.sequence_by(&e.children, &|n| match n {
                Node::Element(b) if matches!(b.kind, ElementKind::Binary { .. }) => {
                    self.binary_in_group(b)
                }
                n => self.node(n),
            }),
            // A ternary, struct or array that is the group's only content
            // hugs the parentheses and breaks on its own (`( cond` ⏎ `? a` ⏎
            // `: b )`, `( {` ⏎ … ⏎ `} )`), as Prettier prints a parenthesised
            // conditional or an arrow's `({ … })`, rather than taking lines
            // for `(` and `)` as well.
            Some(ElementKind::Ternary | ElementKind::Struct { .. } | ElementKind::Array)
                if !commented =>
            {
                let pad = || {
                    Doc::from(if self.opts.parentheses_padding {
                        " "
                    } else {
                        ""
                    })
                };
                let hug = |inner: Doc| {
                    group(vec![
                        self.token(open),
                        pad(),
                        inner,
                        pad(),
                        self.token(close),
                    ])
                };
                let t = only.expect("the group's content");
                if t.kind != ElementKind::Ternary {
                    return hug(Doc::Concat(self.sequence(&e.children)));
                }
                let (cond, rest) = self.ternary_parts(t);
                let ternary = |cond: Doc, rest: Doc, should_break: bool| {
                    group_opts(
                        vec![cond, rest],
                        GroupOpts {
                            should_break,
                            ..GroupOpts::default()
                        },
                    )
                };
                // A binary condition that breaks would continue at the
                // indent of `?` and `:`; the ternary hugs only while the
                // condition fits on the `(` line, else `(` and `)` take lines
                // of their own. Past a nested hug the plain hug keeps the doc
                // linear.
                let cond_binary = t
                    .as_ternary()
                    .and_then(|v| v.cond().as_element())
                    .is_some_and(|c| matches!(c.kind, ElementKind::Binary { .. }));
                let nested = |d: &Doc| {
                    find_in_doc(d, |d| {
                        matches!(d, Doc::Group(g) if g.expanded_states.as_ref().is_some_and(|s| !s.is_empty()))
                            .then_some(())
                    })
                    .is_some()
                };
                if !cond_binary || nested(&cond) || nested(&rest) {
                    return hug(ternary(cond, rest, false));
                }
                let pad_if_flat = || {
                    if self.opts.parentheses_padding {
                        if_break("", " ")
                    } else {
                        Doc::empty()
                    }
                };
                let broken = group(vec![
                    self.token(open),
                    pad_if_flat(),
                    indent(vec![softline(), ternary(cond.clone(), rest.clone(), false)]),
                    softline(),
                    pad_if_flat(),
                    self.token(close),
                ]);
                let forced = will_break(&cond) || will_break(&rest);
                let hugged = hug(ternary(remove_lines(cond), rest, true));
                if forced {
                    // Nothing prints flat, so the states are the hug and the
                    // fallback; a conditional group passes no break up to its
                    // parents (`propagate_breaks`), so the forced break is
                    // announced beside it, as Prettier's call arguments do.
                    return Doc::Concat(vec![
                        break_parent(),
                        conditional_group(vec![hugged, broken]),
                    ]);
                }
                return conditional_group(vec![broken.clone(), hugged, broken]);
            }
            _ => self.sequence(&e.children),
        };
        if content.is_empty() {
            return Doc::Concat(vec![self.token(open), self.token(close)]);
        }
        let pad = || {
            if self.opts.parentheses_padding {
                if_break("", " ")
            } else {
                Doc::empty()
            }
        };
        group(vec![
            self.token(open),
            pad(),
            indent(vec![softline(), Doc::Concat(content)]),
            softline(),
            pad(),
            self.token(close),
        ])
    }
}

impl Printer<'_> {
    /// An expression node's parts (its non-trivia children) with the
    /// comments that follow each part, printed as a comment run
    /// ([`Printer::comment_run`]): a line comment inside an expression
    /// (`a && // c` newline `b`) becomes a line suffix after the part it
    /// follows and breaks the enclosing group. The parser keeps the comments
    /// between an expression's first and last part as its children.
    fn parts<'n>(&self, e: &'n Element) -> Vec<(&'n Node, Run)> {
        part_comments(e)
            .into_iter()
            .map(|(n, comments)| (n, self.comment_run(&comments)))
            .collect()
    }

    /// Comments that follow one part of an expression: each as a same-line
    /// comment until the first line comment, which becomes the line's suffix;
    /// every comment after it starts a line of its own, and then so does the
    /// next part ([`Run::own_line`]). Several line comments in a row would
    /// otherwise flush one another's suffix onto lines of their own in an order
    /// the next run reads differently, or put code after a `//`.
    fn comment_run(&self, comments: &[&Element]) -> Run {
        self.comment_run_as(comments, false)
    }

    /// [`Printer::comment_run`]; `deferred` after a part the layout puts no
    /// line break after (a ternary's `?` and `:`, a unary operator), where
    /// the first line comment is a [`Printer::deferred_comment`]: it ends
    /// whatever line the part ends up on and breaks no group.
    fn comment_run_as(&self, comments: &[&Element], deferred: bool) -> Run {
        let mut parts = Vec::new();
        let mut suffixed = false;
        let mut own_line = false;
        for c in comments {
            if suffixed {
                parts.push(hardline());
                parts.push(self.comment(c));
                own_line = true;
            } else {
                parts.push(if deferred {
                    self.deferred_comment(c)
                } else {
                    self.same_line_comment(c)
                });
                suffixed = c.kind == ElementKind::LineComment;
            }
        }
        if own_line {
            parts.push(hardline());
        }
        Run {
            doc: Doc::Concat(parts),
            own_line,
        }
    }

    /// An operator: a token's text, or a multi-word operator's words one
    /// space apart (`is  not` → `is not`).
    fn operator(&self, n: &Node) -> Doc {
        match n {
            Node::Token(t) => Doc::from(self.tree.text(t).to_owned()),
            Node::Element(phrase) => {
                let mut words = phrase
                    .children
                    .iter()
                    .filter(|w| !w.is_trivia())
                    .filter_map(Node::as_token)
                    .map(|w| self.tree.text(w));
                let mut text = words.next().unwrap_or_default().to_owned();
                for w in words {
                    text.push(' ');
                    text.push_str(w);
                }
                Doc::from(text)
            }
        }
    }

    /// `target op value`, breaking after the operator as Prettier's
    /// `printAssignment` does ([`Printer::assign_layout`]).
    pub(crate) fn assignment(&self, e: &Element) -> Doc {
        self.assignment_with(e, None)
    }

    /// [`Printer::assignment`] with alignment padding after the target.
    ///
    /// Without comments, an assignment lays out as Prettier's
    /// `printAssignment` ([`Printer::assign_layout`]); with one line comment
    /// before its value, as [`Printer::line_commented_assignment`]; with
    /// other comments, it stays flat, its comments where the runs put them.
    pub(crate) fn assignment_with(&self, e: &Element, pad: Option<Doc>) -> Doc {
        let parts = part_comments(e);
        if e.kind == ElementKind::Assignment && parts.len() == 3 {
            let (target, op, value) = (parts[0].0, parts[1].0, parts[2].0);
            let comments: Vec<&Element> =
                parts.iter().flat_map(|(_, c)| c.iter().copied()).collect();
            if comments.is_empty() {
                let mut left = vec![self.node(target)];
                left.extend(pad);
                let left = Doc::Concat(left);
                let layout = self.assign_layout(value, false, &left);
                let operator = Doc::Concat(vec![Doc::from(" "), self.operator(op)]);
                return self.assign_doc(left, operator, self.assigned_value(value), layout);
            }
            if let [c] = comments[..] {
                if c.kind == ElementKind::LineComment && parts[2].1.is_empty() {
                    return self.line_commented_assignment(target, op, value, c, pad);
                }
            }
        }
        let mut parts = Vec::new();
        let mut pad = pad;
        let mut own_line = false;
        // The commented layout puts no line break between its parts: a line
        // comment ends the line the assignment ends on.
        for (i, (n, comments)) in part_comments(e).into_iter().enumerate() {
            let comments = self.comment_run_as(&comments, true);
            if i == 1 {
                parts.extend(pad.take());
            }
            if i > 0 && !own_line {
                parts.push(Doc::from(" "));
            }
            parts.push(if i == 1 {
                self.operator(n)
            } else {
                self.node(n)
            });
            parts.push(comments.doc);
            own_line = comments.own_line;
        }
        self.assign_break.set(None);
        Doc::Concat(parts)
    }

    /// An assignment with one line comment between its target and its value
    /// (`x // c` newline `= 1`, `x = // c` newline `1`): on one line with
    /// the comment at its end while the value fits there (`x = 1; // c`),
    /// else the comment on a line of its own after the operator and the
    /// value on the next, both indented (Prettier's layout for a value with
    /// a leading own-line comment). A value that holds a forced break (a
    /// function body, a struct with a comment) never prints on the
    /// operator's line: the comment would end the value's first line,
    /// after its `{`, where the next run reads it as the value's own.
    fn line_commented_assignment(
        &self,
        target: &Node,
        op: &Node,
        value: &Node,
        comment: &Element,
        pad: Option<Doc>,
    ) -> Doc {
        let mut left = vec![self.node(target)];
        left.extend(pad);
        left.extend([Doc::from(" "), self.operator(op)]);
        let value = self.assigned_value(value);
        let own_line = Doc::Concat(vec![
            Doc::Concat(left.clone()),
            indent(vec![
                hardline(),
                self.comment(comment),
                hardline(),
                value.clone(),
            ]),
        ]);
        self.assign_break.set(None);
        if will_break(&value) {
            return own_line;
        }
        let one_line = Doc::Concat(vec![
            Doc::Concat(left),
            Doc::from(" "),
            self.deferred_comment(comment),
            value,
        ]);
        conditional_group(vec![one_line, own_line])
    }

    /// `left op value` in `layout` (Prettier's `printAssignment`); `operator`
    /// carries its leading space (` =`, `:`).
    pub(crate) fn assign_doc(
        &self,
        left: Doc,
        operator: Doc,
        value: Doc,
        layout: AssignLayout,
    ) -> Doc {
        self.assign_doc_spaced(left, operator, true, value, layout)
    }

    /// [`Printer::assign_doc`]; `spaced: false` for an operator printed with
    /// no space after it (a struct separator set to `=`), where the value
    /// follows it directly on its line.
    pub(crate) fn assign_doc_spaced(
        &self,
        left: Doc,
        operator: Doc,
        spaced: bool,
        value: Doc,
        layout: AssignLayout,
    ) -> Doc {
        let gap = || if spaced { line() } else { softline() };
        match layout {
            AssignLayout::BreakAfterOperator => {
                let id = self.ids.borrow_mut().next_id();
                self.assign_break.set(Some(id));
                group(vec![
                    group(left),
                    operator,
                    group_opts(
                        indent(vec![gap(), value]),
                        GroupOpts {
                            id: Some(id),
                            ..GroupOpts::default()
                        },
                    ),
                ])
            }
            AssignLayout::NeverBreakAfterOperator => {
                self.assign_break.set(None);
                group(vec![
                    group(left),
                    operator,
                    Doc::from(if spaced { " " } else { "" }),
                    value,
                ])
            }
            AssignLayout::Fluid => {
                let id = self.ids.borrow_mut().next_id();
                self.assign_break.set(Some(id));
                group(vec![
                    group(left),
                    operator,
                    group_opts(
                        indent(gap()),
                        GroupOpts {
                            id: Some(id),
                            ..GroupOpts::default()
                        },
                    ),
                    // Prettier puts a `lineSuffixBoundary` here, for a
                    // comment its attachment leaves pending after the
                    // operator. A comment of this assignment's own prints
                    // through the commented layout, never this one, so a
                    // suffix pending here is a comment from further left
                    // on the line that breaks nothing
                    // ([`Printer::deferred_comment`]): flushed here, it
                    // would move to after the operator, and pull the value
                    // onto the next line, on this run only.
                    indent_if_break(value, id, false),
                ])
            }
        }
    }

    /// An assignment's or struct member's value: a binary without its own
    /// indent ([`Printer::binary_value`]), anything else as it prints.
    pub(crate) fn assigned_value(&self, n: &Node) -> Doc {
        match n {
            Node::Element(b) if matches!(b.kind, ElementKind::Binary { .. }) => {
                self.binary_value(b)
            }
            n => self.node(n),
        }
    }

    /// Prettier's `chooseLayout` for `left op value`; `short_key` is a
    /// struct member whose key is narrower than `indent_size + 3`.
    ///
    /// Break after the operator for a binary (unless a logical one ending in
    /// a non-empty struct or array), a ternary whose condition is such a
    /// binary, a plain string, or a poorly breakable chain or call
    /// ([`Printer::poorly_breakable`]); never for a short key, an
    /// interpolated string, a boolean or a number (when the left side
    /// cannot break); otherwise fluid. A value in parentheses is fluid;
    /// parentheses inside it (a ternary's condition) are looked through, as
    /// Prettier has no node for them.
    pub(crate) fn assign_layout(&self, value: &Node, short_key: bool, left: &Doc) -> AssignLayout {
        // Parentheses around the whole value are Prettier's own around a
        // broken `return` value: `x = (` ⏎ … ⏎ `)` keeps them on the
        // operator's line, breaking after it only when `x = (` does not fit.
        if matches!(value, Node::Element(g) if g.kind == ElementKind::Group) {
            return AssignLayout::Fluid;
        }
        if let Node::Element(e) = value {
            match e.kind {
                ElementKind::Binary { .. } if !inline_logical(e) => {
                    return AssignLayout::BreakAfterOperator;
                }
                ElementKind::Ternary => {
                    let cond = e.as_ternary().map(|t| unparen(t.cond()));
                    if let Some(Node::Element(c)) = cond {
                        if matches!(c.kind, ElementKind::Binary { .. }) && !inline_logical(c) {
                            return AssignLayout::BreakAfterOperator;
                        }
                    }
                }
                _ => {}
            }
        }
        if !short_key {
            let stripped = strip_unary(value);
            let plain_string = matches!(stripped, Node::Element(s)
                if matches!(s.kind, ElementKind::String { .. }) && !self.is_template(s));
            if plain_string || self.poorly_breakable(stripped, false) {
                return AssignLayout::BreakAfterOperator;
            }
        }
        let left_breaks = find_in_doc(left, |d| matches!(d, Doc::Line(_)).then_some(())).is_some();
        let literal = match value {
            Node::Token(t) => matches!(t.kind, TokenKind::Literal(Literal::Number | Literal::Bool)),
            Node::Element(s) => matches!(s.kind, ElementKind::String { .. }) && self.is_template(s),
        };
        if !left_breaks && (short_key || literal) {
            return AssignLayout::NeverBreakAfterOperator;
        }
        AssignLayout::Fluid
    }

    /// A string Prettier would print as a template literal: one with a
    /// `#…#` interpolation or a line break in it.
    fn is_template(&self, s: &Element) -> bool {
        is_interpolated(s) || self.tree.slice(s.span.clone()).contains('\n')
    }

    /// Prettier's `isPoorlyBreakableMemberOrCallChain`: a chain or call whose
    /// calls all take no argument or one short one
    /// ([`Printer::lone_short_argument`]), over a name or `this`, and that
    /// is not a member chain that breaks by groups
    /// ([`Printer::is_member_chain`]); breaking after `=` beats breaking
    /// inside it.
    fn poorly_breakable(&self, n: &Node, deep: bool) -> bool {
        match n {
            Node::Token(t) => {
                deep && matches!(t.kind, TokenKind::Ident(_) | TokenKind::Keyword(_))
                    && self
                        .tree
                        .text(t)
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            }
            Node::Element(e) => match e.kind {
                ElementKind::CallExpr => e.as_call_expr().is_some_and(|c| {
                    self.poorly_breakable_args(c.args()) && self.poorly_breakable(c.callee(), true)
                }),
                ElementKind::Chain => {
                    !self.is_member_chain(e)
                        && e.as_chain().is_some_and(|c| {
                            c.segments().all(|s| {
                                s.as_segment()
                                    .and_then(|v| v.call())
                                    .is_none_or(|a| self.poorly_breakable_args(a))
                            }) && self.poorly_breakable(c.head(), true)
                        })
                }
                _ => false,
            },
        }
    }

    fn poorly_breakable_args(&self, args: &Element) -> bool {
        let items = printable_items(args);
        match items[..] {
            [] => true,
            [item] => {
                let mut sig = item.significant();
                matches!((sig.next(), sig.next()), (Some(n), None) if self.lone_short_argument(n))
            }
            _ => false,
        }
    }

    /// Prettier's `isLoneShortArgument`, its threshold a quarter of
    /// `max_columns`: `this`, a short name, a number, boolean or null, a
    /// short plain string, an empty struct or array, a unary over one of
    /// these, a short name called with no argument.
    fn lone_short_argument(&self, n: &Node) -> bool {
        let threshold = self.opts.max_columns / 4;
        match n {
            Node::Token(t) => match t.kind {
                TokenKind::Literal(_) => true,
                TokenKind::Ident(_) | TokenKind::Keyword(_) => {
                    self.tree.text(t).chars().count() <= threshold
                }
                _ => false,
            },
            Node::Element(e) => match e.kind {
                ElementKind::String { .. } => {
                    let text = self.tree.slice(e.span.clone());
                    !is_interpolated(e) && text.chars().count() <= threshold && !text.contains('\n')
                }
                ElementKind::Struct { .. } | ElementKind::Array => printable_items(e).is_empty(),
                ElementKind::Unary { postfix: false } => e
                    .as_unary()
                    .is_some_and(|u| self.lone_short_argument(u.operand())),
                ElementKind::CallExpr => e.as_call_expr().is_some_and(|c| {
                    printable_items(c.args()).is_empty()
                        && matches!(c.callee(), Node::Token(t)
                            if self.tree.text(t).chars().count() + 2 <= threshold)
                }),
                _ => false,
            },
        }
    }

    /// One precedence level: `group([a, indent([" ", op, line, b, " ", op,
    /// line, c])])`, breaking after the operators with the continuation lines
    /// indented once; the first operand stays outside the indent, so its own
    /// lines (a broken argument list, chain segments) are not indented twice
    /// (Prettier's `printBinaryishExpression`: "don't include the initial
    /// expression in the indentation level"); every operator is padded, word
    /// operators included. A nested level is a nested group. Where the source
    /// broke the line does not matter, and the operator spacing has no option.
    pub(crate) fn binary(&self, e: &Element) -> Doc {
        self.binary_doc(e, Place::Indented, None)
    }

    /// [`Printer::binary`] as a `( … )` group's only content: the
    /// continuation lines stay at the enclosing indent and the parts are not
    /// grouped, so their lines break with the enclosing group.
    pub(crate) fn binary_in_group(&self, e: &Element) -> Doc {
        self.binary_doc(e, Place::GroupContent, None)
    }

    /// [`Printer::binary`] as an assignment's or a struct member's value:
    /// grouped, its continuation lines not indented again, since the value
    /// sits under the indent after the operator (Prettier's
    /// `shouldIndentIfInlining`).
    pub(crate) fn binary_value(&self, e: &Element) -> Doc {
        self.binary_doc(e, Place::Value, None)
    }

    /// [`Printer::binary`] as a `return` or `throw` value: indented, and a
    /// parenthesised head keeps `(` and `)` on lines of their own
    /// (`return (` ⏎ `a &&` ⏎ `b` ⏎ `) || c`): hugged, its continuation
    /// lines would share the chain's indent, which Prettier avoids with
    /// parentheses of its own after `return`.
    pub(crate) fn binary_return(&self, e: &Element) -> Doc {
        self.binary_doc(e, Place::Return, None)
    }

    /// `parent` is the enclosing binary's [`is_logical`] when there is one.
    fn binary_doc(&self, e: &Element, place: Place, parent: Option<bool>) -> Doc {
        let indented = matches!(place, Place::Indented | Place::Return);
        let logical = e.as_binary().map(|b| is_logical(b.prec())).unwrap_or(false);
        let mut parts = part_comments(e);
        // A line comment between an operand and its operator prints after the
        // operator (`a && // c`), with the comments after it and the
        // operator's own: that is where the next run finds them. Block
        // comments before it stay in place (`a /* c */ + b`).
        for i in (0..parts.len().saturating_sub(1)).step_by(2) {
            if let Some(at) = parts[i]
                .1
                .iter()
                .position(|c| c.kind == ElementKind::LineComment)
            {
                let moved = parts[i].1.split_off(at);
                parts[i + 1].1.splice(0..0, moved);
            }
        }
        let last = parts.len() - 1;
        // Prettier's `shouldGroup` (`printBinaryishExpressions`): a single
        // binary — two operands, neither a binary of the other type, not
        // inside one — groups its operator and right operand, "to avoid
        // having a small right part like -1 be on its own line" when the
        // left operand holds a forced break (`}) > 0`). A logical binary
        // that is a keyword group's whole content does not (`if (a &&` ⏎
        // `b)`), and a same-type neighbour breaks with the chain.
        // Parentheses are no node to Prettier: an operand `( a + b )` that
        // hugs its parentheses (below) is a binary here as well. The head of
        // a `return` value keeps the group's layout and stays a block; a
        // chain that is a group's content keeps its right operand grouped,
        // since its own line breaks are the group's.
        let binary_type = |n: &Node, head: bool| {
            n.as_element()
                .map(|c| match parenthesised_binary(c) {
                    Some(b)
                        if !(head && place == Place::Return) && place != Place::GroupContent =>
                    {
                        b
                    }
                    _ => c,
                })
                .and_then(|c| c.as_binary())
                .map(|b| is_logical(b.prec()))
        };
        let group_right = parts.len() == 3
            && !(logical && place == Place::GroupContent)
            && parent != Some(logical)
            && binary_type(parts[0].0, true) != Some(logical)
            && binary_type(parts[2].0, false) != Some(logical);
        // An inline component stays on its operator's line, as a component
        // body does after `=`: its body's hard lines break the group.
        let hugged: Vec<bool> = parts.iter().map(|(n, _)| is_inline_component(n)).collect();
        let mut docs = Vec::new();
        let mut right_at = None;
        for (i, (n, comments)) in parts.into_iter().enumerate() {
            let operator = i % 2 == 1;
            if operator && group_right && !hugged[last] {
                right_at = Some(docs.len());
            }
            docs.push(if operator {
                self.operator(n)
            } else {
                match n {
                    Node::Element(c) if matches!(c.kind, ElementKind::Binary { .. }) => {
                        self.binary_doc(c, Place::Indented, Some(logical))
                    }
                    // Not the head of a `return` value: its continuation
                    // lines would share the chain's indent (`return ( a &&`
                    // ⏎ `b ) ||` ⏎ `c`), which Prettier avoids with
                    // parentheses of its own after `return`; the group's
                    // own layout keeps them apart. Elsewhere Prettier hugs
                    // the head too.
                    Node::Element(g) if i > 0 || place != Place::Return => self
                        .parenthesised_operand(g)
                        .unwrap_or_else(|| self.node(n)),
                    n => self.node(n),
                }
            });
            let run = self.comment_run(&comments);
            docs.push(run.doc);
            if i < last && !run.own_line {
                docs.push(if operator && !hugged[i + 1] {
                    line()
                } else {
                    Doc::from(" ")
                });
            }
        }
        if let Some(at) = right_at {
            let right = docs.split_off(at);
            docs.push(group(right));
        }
        // A hugged last operand (and its comments) goes after the group, at
        // the binary's own indent, so its body breaks nothing before it.
        let tail = if hugged[last] {
            docs.split_off(docs.len() - 2)
        } else {
            Vec::new()
        };
        if indented && docs.len() > 2 {
            // The first operand and its comments, then the rest indented.
            let rest = docs.split_off(2);
            docs.push(indent(rest));
        }
        if place == Place::GroupContent {
            docs.extend(tail);
            return Doc::Concat(docs);
        }
        if tail.is_empty() {
            return group(docs);
        }
        Doc::Concat(vec![group(docs), Doc::Concat(tail)])
    }

    /// A `( … )` operand of a binary whose only content is a binary: the
    /// parentheses hug it and it breaks as any binary does, its continuation
    /// lines indented once (`a &&` ⏎ `( b ||` ⏎ `    c )`), as Prettier
    /// prints the parentheses around an operand; `None` for any other
    /// group, which keeps [`Printer::group`]'s layout (`(` and `)` on lines
    /// of their own, as Prettier breaks a unary's or a member's operand).
    fn parenthesised_operand(&self, g: &Element) -> Option<Doc> {
        let b = parenthesised_binary(g)?;
        let (Some(open), Some(close)) = (&g.open, &g.close) else {
            return None;
        };
        let pad = || {
            Doc::from(if self.opts.parentheses_padding {
                " "
            } else {
                ""
            })
        };
        Some(group(vec![
            self.token(open),
            pad(),
            self.binary(b),
            pad(),
            self.token(close),
        ]))
    }

    /// `group([cond, indent([line, "? ", then, line, ": ", otherwise])])`.
    pub(crate) fn ternary(&self, e: &Element) -> Doc {
        let cond_indent = self.ternary_cond_indent.replace(false);
        let (cond, rest) = self.ternary_branches(e, cond_indent);
        group(vec![cond, indent(rest)])
    }

    /// [`Printer::ternary`]'s condition and the indented rest, ungrouped.
    fn ternary_parts(&self, e: &Element) -> (Doc, Doc) {
        let (cond, rest) = self.ternary_branches(e, false);
        (cond, indent(rest))
    }

    /// A ternary that is a branch of another prints in the outer one's group,
    /// so the whole chain breaks together (Prettier's `printTernaryOld`
    /// groups only the outermost): in the `:` branch its `?` and `:` line up
    /// under the branch (`: b` ⏎ `  ? c` ⏎ `  : d`); in the `?` branch they
    /// sit `indent_size - 2` further in, so that they are indented once from
    /// the outer `?` (under tabs, where the branch is a whole indent, no
    /// further).
    fn nested_ternary(&self, e: &Element, consequent: bool) -> Doc {
        let (cond, rest) = self.ternary_branches(e, false);
        if !consequent {
            // A condition that breaks continues under its first character,
            // past the `: ` (Prettier's `printTernaryTest`).
            return Doc::Concat(vec![align(2, cond), rest]);
        }
        let rest = match self.opts.indent_size.checked_sub(2) {
            Some(n) if n > 0 && !self.opts.tab_indent => align(n, rest),
            _ => rest,
        };
        Doc::Concat(vec![cond, rest])
    }

    /// The condition, and `line, "? ", then, line, ": ", otherwise` with each
    /// branch aligned to the column after `? ` / `: ` (Prettier's
    /// `align(2, …)`, one indent under tabs): a struct or call argument list
    /// that breaks inside a branch indents from that column
    /// (`: foo( {` ⏎ `      key: 1` ⏎ `  } )`). A binary part is its own
    /// group, its continuation lines indented only with `binary_indent`
    /// ([`Printer::ternary_cond_indent`]). A parenthesised ternary branch
    /// prints as a nested one inside its parentheses (`: ( c` ⏎ `  ? d` ⏎
    /// `  : e )`), the chain Prettier prints once it drops them.
    fn ternary_branches(&self, e: &Element, binary_indent: bool) -> (Doc, Doc) {
        let mut parts = part_comments(e).into_iter();
        let mut next = |part: Part| {
            let (n, comments) = parts.next().expect("ternary part");
            // No line break follows `?` or `:`: a line comment after one
            // ends the line its branch ends on.
            let comments = self.comment_run_as(&comments, part == Part::Operator);
            let doc = match (part, n) {
                (Part::Operator, n) => self.operator(n),
                (part, Node::Element(b))
                    if !binary_indent && matches!(b.kind, ElementKind::Binary { .. }) =>
                {
                    self.branch_if(part, self.guarded(b, || self.binary_value(b)))
                }
                (Part::Condition, n) => self.node(n),
                // The two printers reach their elements directly, so each
                // level is counted against the depth bound here.
                (branch, Node::Element(t)) if t.kind == ElementKind::Ternary => self
                    .branch(self.guarded(t, || self.nested_ternary(t, branch == Part::Consequent))),
                (branch, Node::Element(g)) if parenthesised_ternary(g).is_some() => {
                    let consequent = branch == Part::Consequent;
                    self.branch(self.guarded(g, || self.parenthesised_ternary(g, consequent)))
                }
                (_, n) => self.branch(self.node(n)),
            };
            (doc, comments)
        };
        // After a comment on its own line the next part already starts one.
        let sep = |own_line: bool, sep: Doc| if own_line { Doc::empty() } else { sep };
        let (cond, c) = next(Part::Condition);
        let (question, q) = next(Part::Operator);
        let (then, t) = next(Part::Consequent);
        let (colon, k) = next(Part::Operator);
        let (otherwise, o) = next(Part::Alternate);
        (
            cond,
            Doc::Concat(vec![
                // The condition's comments sit at the continuation indent.
                c.doc,
                sep(c.own_line, line()),
                question,
                q.doc,
                sep(q.own_line, Doc::from(" ")),
                then,
                t.doc,
                sep(t.own_line, line()),
                colon,
                k.doc,
                sep(k.own_line, Doc::from(" ")),
                otherwise,
                o.doc,
            ]),
        )
    }

    /// A ternary branch, aligned two columns in: `align(2)`, or one indent
    /// under tabs (Prettier's `printBranch`; an alignment under tabs would
    /// end a line's indentation in spaces).
    fn branch(&self, doc: Doc) -> Doc {
        if self.opts.tab_indent {
            indent(doc)
        } else {
            align(2, doc)
        }
    }

    /// [`Printer::branch`] for a branch; a condition as it is.
    fn branch_if(&self, part: Part, doc: Doc) -> Doc {
        if part == Part::Condition {
            doc
        } else {
            self.branch(doc)
        }
    }

    /// A branch `( c ? d : e )` ([`parenthesised_ternary`]):
    /// [`Printer::nested_ternary`] between the parentheses, padded as any
    /// group.
    fn parenthesised_ternary(&self, g: &Element, consequent: bool) -> Doc {
        let t = parenthesised_ternary(g).expect("a parenthesised ternary");
        let pad = || {
            Doc::from(if self.opts.parentheses_padding {
                " "
            } else {
                ""
            })
        };
        Doc::Concat(vec![
            self.token(g.open.as_ref().expect("open")),
            pad(),
            self.guarded(t, || self.nested_ternary(t, consequent)),
            pad(),
            self.token(g.close.as_ref().expect("close")),
        ])
    }

    /// `-a`, `++a`, `!a`, `not a`, `...a`, `a++`: the operator against its
    /// operand; a word operator (`not`) takes a space.
    ///
    /// A symbolic prefix operator whose operand starts with the same symbol
    /// (`- -y`, `- --y`, `+ +y`) keeps one space, also when a line comment
    /// between them moves to the end of the line: written together they
    /// would lex as `--` / `++`, a different operator. A postfix operator never
    /// needs it: the tree has no postfix unary around an operand that ends in
    /// `+` or `-`, and a binary operator always prints with spaces.
    pub(crate) fn unary(&self, e: &Element) -> Doc {
        let postfix = e.as_unary().is_some_and(|u| u.is_postfix());
        let pieces = part_comments(e);
        let fuses = !postfix && pieces.len() == 2 && self.would_fuse(pieces[0].0, pieces[1].0);
        let mut parts = Vec::new();
        // A unary never breaks: a line comment inside one ends the line the
        // expression ends on.
        for (i, (n, comments)) in pieces.into_iter().enumerate() {
            let comments = self.comment_run_as(&comments, true);
            let operator = (i == 0) != postfix;
            if operator {
                let word = n
                    .as_token()
                    .is_some_and(|t| t.kind == TokenKind::Operator(Operator::Not { word: true }));
                parts.push(self.operator(n));
                parts.push(comments.doc);
                if (word || fuses) && !comments.own_line {
                    parts.push(Doc::from(" "));
                }
            } else {
                parts.push(self.node(n));
                parts.push(comments.doc);
            }
        }
        Doc::Concat(parts)
    }

    /// Whether prefix operator `op` printed against `operand` would join
    /// with it into another token: both are `+` or both `-` where they meet.
    /// The operand prints its first token as written, so the source's first
    /// character is the printed one.
    fn would_fuse(&self, op: &Node, operand: &Node) -> bool {
        let Some(last) = op.as_token().and_then(|t| self.tree.text(t).chars().last()) else {
            return false;
        };
        let first = self.tree.slice(operand.span()).chars().next();
        matches!(last, '+' | '-') && first == Some(last)
    }

    /// `new Name(args)`: `new`, one space, the class name, the arguments.
    pub(crate) fn new_expr(&self, e: &Element) -> Doc {
        let mut parts = Vec::new();
        let mut own_line = false;
        for (i, (n, comments)) in self.parts(e).into_iter().enumerate() {
            if i == 1 && !own_line {
                parts.push(Doc::from(" "));
            }
            parts.push(self.node(n));
            parts.push(comments.doc);
            own_line = comments.own_line;
        }
        self.assign_break.set(None);
        Doc::Concat(parts)
    }
}

/// The ternary a `( … )` group holds with nothing else: no comments, both
/// parentheses.
fn parenthesised_ternary(g: &Element) -> Option<&Element> {
    if g.kind != ElementKind::Group || g.open.is_none() || g.close.is_none() {
        return None;
    }
    if g.children
        .iter()
        .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()))
    {
        return None;
    }
    let mut sig = g.children.iter().filter(|n| !n.is_trivia());
    match (sig.next(), sig.next()) {
        (Some(Node::Element(t)), None) if t.kind == ElementKind::Ternary => Some(t),
        _ => None,
    }
}

/// A ternary's parts, in order ([`Printer::ternary_branches`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Condition,
    Operator,
    Consequent,
    Alternate,
}

/// Where a binary prints ([`Printer::binary_doc`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Its own group, the operands after the first indented once.
    Indented,
    /// [`Place::Indented`] as a `return` or `throw` value.
    Return,
    /// A `( … )` group's whole content: not indented, and its line breaks
    /// are the group's (Prettier's `isInsideParenthesis`).
    GroupContent,
    /// An assignment's or struct member's value: grouped, not indented.
    Value,
}

/// How an assignment breaks around its operator (Prettier's
/// `chooseLayout`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AssignLayout {
    /// The value moves to the next line, indented, whenever it does not fit
    /// on the operator's line.
    BreakAfterOperator,
    /// The value always starts on the operator's line.
    NeverBreakAfterOperator,
    /// The value moves to the next line only when even its first line does
    /// not fit on the operator's.
    Fluid,
}

/// The comments after one part of an expression ([`Printer::comment_run`]).
struct Run {
    doc: Doc,
    /// A comment printed on a line of its own, so the run ends with a hard
    /// line and the next part needs no separator.
    own_line: bool,
}

/// A `New` holding a component body: Lucee's inline component.
/// Prettier's `LogicalExpression` against its `BinaryExpression`: the
/// boolean connectives (`&&`, `||`, `?:`, `xor`, `eqv`, `imp`).
fn is_logical(prec: Prec) -> bool {
    matches!(
        prec,
        Prec::Elvis | Prec::Imp | Prec::Eqv | Prec::Xor | Prec::Or | Prec::And
    )
}

fn is_inline_component(n: &Node) -> bool {
    n.as_element().is_some_and(|e| {
        e.kind == ElementKind::New
            && e.children
                .iter()
                .any(|c| matches!(c, Node::Element(k) if k.kind == ElementKind::Class))
    })
}

/// A `( … )` group whose only content is a struct or an array (an arrow's
/// `({ … })`), with no comment inside.
pub(crate) fn is_parenthesised_literal(e: &Element) -> bool {
    if e.kind != ElementKind::Group || !e.items.is_empty() {
        return false;
    }
    let mut sig = e.children.iter().filter(|n| !n.is_trivia());
    let only = matches!(
        (sig.next(), sig.next()),
        (Some(Node::Element(c)), None)
            if matches!(c.kind, ElementKind::Struct { .. } | ElementKind::Array)
    );
    only && !e
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()))
}

/// The binary a `( … )` group holds as its only content, with no comment
/// inside and both parentheses present.
fn parenthesised_binary(g: &Element) -> Option<&Element> {
    if g.kind != ElementKind::Group || !g.items.is_empty() || g.open.is_none() || g.close.is_none()
    {
        return None;
    }
    let mut sig = g.children.iter().filter(|n| !n.is_trivia());
    let (Some(Node::Element(b)), None) = (sig.next(), sig.next()) else {
        return None;
    };
    let commented = g
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()));
    (matches!(b.kind, ElementKind::Binary { .. }) && !commented).then_some(b)
}

/// A node with comment-free parentheses around it looked through.
fn unparen(n: &Node) -> &Node {
    match n {
        Node::Element(g) if g.kind == ElementKind::Group && g.items.is_empty() => {
            let mut sig = g.children.iter().filter(|c| !c.is_trivia());
            let commented = g
                .children
                .iter()
                .any(|c| matches!(c, Node::Element(k) if k.kind.is_comment()));
            match (sig.next(), sig.next()) {
                (Some(inner), None) if !commented && g.open.is_some() && g.close.is_some() => {
                    unparen(inner)
                }
                _ => n,
            }
        }
        _ => n,
    }
}

/// A prefix unary's operand, repeatedly (Prettier looks through
/// `UnaryExpression` before deciding a layout).
fn strip_unary(n: &Node) -> &Node {
    match n {
        Node::Element(u) if u.kind == (ElementKind::Unary { postfix: false }) => {
            u.as_unary().map_or(n, |v| strip_unary(v.operand()))
        }
        _ => n,
    }
}

/// A string with a `#…#` interpolation, Prettier's template literal.
fn is_interpolated(s: &Element) -> bool {
    s.children
        .iter()
        .any(|c| matches!(c, Node::Element(t) if t.kind == ElementKind::TemplateExpression))
}

/// Prettier's `shouldInlineLogicalExpression`: a logical binary whose last
/// operand is a non-empty struct or array (`a || {` ⏎ … ⏎ `}`), or an inline
/// component, whose body the binary already keeps on its operator's line.
fn inline_logical(b: &Element) -> bool {
    let Some(v) = b.as_binary() else {
        return false;
    };
    is_logical(v.prec())
        && v.operands().last().is_some_and(|n| {
            is_inline_component(n)
                || matches!(n, Node::Element(c)
                    if matches!(c.kind, ElementKind::Struct { .. } | ElementKind::Array)
                        && c.items.iter().any(super::delimited::is_printable))
        })
}

/// [`Printer::parts`] with the comments still elements.
fn part_comments<'n>(e: &'n Element) -> Vec<(&'n Node, Vec<&'n Element>)> {
    let mut out: Vec<(&'n Node, Vec<&'n Element>)> = Vec::new();
    for n in &e.children {
        match n {
            Node::Element(c) if c.kind.is_comment() => match out.last_mut() {
                Some((_, comments)) => comments.push(c),
                None => unreachable!("expression node starting with a comment"),
            },
            n if n.is_trivia() => {}
            n => out.push((n, Vec::new())),
        }
    }
    out
}
