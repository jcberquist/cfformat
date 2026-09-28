//! Member chains: `Chain` and `Segment` (Prettier's `printMemberChain`).
//!
//! A chain without comments is partitioned into groups as Prettier does: the
//! first group is the head with the number indexes after it, then (unless
//! the head is a call) every property or index but the last before the
//! first method (`this.items` of `this.items.map()`); each later group is a
//! run of properties and indexes ending in a method (`.a.b()`), and the
//! properties and indexes after the last method close the last group. An
//! index never starts a group (Prettier's only for a number, here for every
//! index: a line starting with `[` is not an index to every CFML engine),
//! and a static access right after the head stays with it (`Class` newline
//! `::create()` reads `Class` as a variable).
//!
//! The first group merges with the second (Prettier's `shouldNotWrap`) when
//! the head alone is `this`, a factory-like name (a capital first letter, or
//! only `$` and `_`), or a name no longer than `indent_size` at the start of
//! an expression statement; or when the first group ends in a factory-like
//! property. A chain of at most two groups (three merged) is one line whose
//! arguments break. A longer one is its groups on one line, or, broken, the
//! first (merged) group and every later group on a line of its own, indented
//! once: always broken when it has more than two calls and one of them has
//! an argument that is not simple ([`Printer::is_simple_arg`]), when a group
//! before the last will break, or when the last group will break and an earlier
//! call has a function argument; otherwise the group is a conditional group
//! over one copy of the doc, flat while its first line fits (so a last
//! callback or struct may break under a one-line chain, `a.b().c(function()
//! {`), broken otherwise. `method_call.chain.multiline`, when above 0, breaks
//! every chain with at least that many methods.
//!
//! A chain with a comment has a simpler layout, kept for compatibility with
//! CommandBox cfformat's output: line 0 is the head with the properties and
//! indexes before the first method (and that method when the chain starts its
//! statement and line 0 is no wider than `indent_size`); every later method,
//! and every segment after a line comment, starts a line.

use cfdoc::builders::{
    break_parent, conditional_group_contents, group, group_opts, hardline, indent, indent_if_break,
    softline, GroupOpts,
};
use cfdoc::utils::{flat_width, will_break};
use cfdoc::{Doc, FlatWidth};
use cfparse::{Element, ElementKind, Ident, Item, Literal, Node, Operator, SegmentKind, TokenKind};

use super::alignment::one_line_width;
use super::delimited::printable_items;
use super::Printer;

impl Printer<'_> {
    /// A member chain inside an expression (see the module docs): its first
    /// method never joins line 0.
    pub(crate) fn chain(&self, e: &Element) -> Doc {
        self.chain_with(e, false)
    }

    /// A member chain; `statement_start` when it is the whole content of an
    /// expression statement, where a short line 0 keeps the first method.
    pub(crate) fn chain_with(&self, e: &Element, statement_start: bool) -> Doc {
        let methods = e
            .children
            .iter()
            .filter(|n| {
                matches!(n, Node::Element(s) if s.kind == ElementKind::Segment(SegmentKind::Method))
            })
            .count();
        let line_comments = e
            .children
            .iter()
            .any(|n| matches!(n, Node::Element(c) if c.kind == ElementKind::LineComment));
        if methods <= 1 && !line_comments {
            return Doc::Concat(self.flat_chain(e));
        }
        let commented = e
            .children
            .iter()
            .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()));
        if !commented {
            return self.member_chain(e, statement_start, methods);
        }

        // Line 0 is printed straight into `first`; every later line goes into
        // `rest` after a `softline` (own-line comments after a `hardline`).
        let mut first: Vec<Doc> = Vec::new();
        let mut rest: Vec<Doc> = Vec::new();
        // The flat width of line 0 so far; `None` once it cannot be flat.
        let mut width = Some(0usize);
        let mut on_first_line = true;
        let (mut method_seen, mut after_comment, mut newline, mut space) =
            (false, false, false, false);
        let mut head = true;
        let mut segment_seen = false;
        for n in &e.children {
            match n {
                Node::Token(t) if t.kind == TokenKind::Newline => {
                    newline = true;
                    continue;
                }
                Node::Token(t) if t.kind == TokenKind::Whitespace => continue,
                Node::Element(c) if c.kind == ElementKind::LineComment => {
                    if newline {
                        // An own-line comment: its own line inside the indent.
                        rest.extend([hardline(), self.comment(c)]);
                        on_first_line = false;
                    } else {
                        let doc = self.same_line_comment(c);
                        if on_first_line { &mut first } else { &mut rest }.push(doc);
                    }
                    (after_comment, newline, space) = (true, false, false);
                    continue;
                }
                // Block and doc comments stay in place, one space each side.
                Node::Element(c) if c.kind.is_comment() => {
                    let doc = Doc::Concat(vec![Doc::from(" "), self.comment(c)]);
                    if on_first_line {
                        width = width.zip(one_line_width(&doc)).map(|(a, b)| a + b + 1);
                        first.push(doc);
                    } else {
                        rest.push(doc);
                    }
                    (newline, space) = (false, true);
                    continue;
                }
                _ => {}
            }
            newline = false;
            if head {
                head = false;
                let doc = self.node(n);
                width = one_line_width(&doc);
                first.push(doc);
                continue;
            }
            let Node::Element(s) = n else {
                debug_assert!(false, "a chain part that is not a segment");
                first.push(self.node(n));
                continue;
            };
            let is_method = s.kind == ElementKind::Segment(SegmentKind::Method);
            let is_index = s.kind == ElementKind::Segment(SegmentKind::Index);
            // A segment starts a line after a method or a line comment; so
            // does the first method when its line already holds more than one
            // indent (a later line starts after a comment and holds a segment
            // by now) or when the chain does not start its statement (what
            // precedes it, `x = ` and the like, counts toward that width).
            // An index stays on the line of what it indexes (`a.b()[1]`): a
            // line starting with `[` is not an index to every CFML engine.
            // A static access on the head (`Class::create()`) stays on its
            // line too: `Class` newline `::create()` parses the head as a
            // variable rather than a class name.
            let static_head = !segment_seen && s.as_segment().is_some_and(|v| v.is_static());
            let starts_line = !static_head
                && ((method_seen && !is_index)
                    || after_comment
                    || (is_method
                        && (!statement_start
                            || !on_first_line
                            || width.is_none_or(|w| w > self.opts.indent_size))));
            let doc = self.segment(s);
            if starts_line {
                rest.push(softline());
                on_first_line = false;
                space = false;
            } else if on_first_line {
                width = width.zip(one_line_width(&doc)).map(|(a, b)| a + b);
            }
            if space {
                if on_first_line { &mut first } else { &mut rest }.push(Doc::from(" "));
                space = false;
            }
            if on_first_line { &mut first } else { &mut rest }.push(doc);
            method_seen |= is_method;
            segment_seen = true;
            after_comment = false;
        }
        first.push(indent(rest));
        match self.opts.method_call_chain_multiline {
            0 => group(first),
            n => {
                // `method_call.chain.multiline`: a chain with at least `n`
                // methods always breaks (unless it prints nothing at all).
                let first = Doc::Concat(first);
                let should_break = methods >= n as usize && prints_flat(&first);
                group_opts(
                    first,
                    GroupOpts {
                        should_break,
                        ..GroupOpts::default()
                    },
                )
            }
        }
    }

    /// A chain without comments, as Prettier's `printMemberChain` (see the
    /// module docs).
    fn member_chain(&self, e: &Element, statement_start: bool, methods: usize) -> Doc {
        let Partition {
            head,
            segments,
            first: i,
            groups,
            merge,
        } = self.partition(e, statement_start);
        let is_method = |s: &Element| s.as_segment().map(|v| v.kind()) == Some(SegmentKind::Method);

        let print_group = |g: &[&Element]| Doc::Concat(g.iter().map(|s| self.segment(s)).collect());
        let first = Doc::Concat(
            std::iter::once(self.node(head))
                .chain(segments[..i].iter().map(|s| self.segment(s)))
                .collect(),
        );
        let printed: Vec<Doc> = groups.iter().map(|g| print_group(g)).collect();

        let cutoff = if merge { 3 } else { 2 };
        let forced = self.opts.method_call_chain_multiline > 0
            && methods as u32 >= self.opts.method_call_chain_multiline;
        // The first group and the rest: at most `cutoff` groups in all.
        if !forced && groups.len() < cutoff {
            return group(Doc::Concat(std::iter::once(first).chain(printed).collect()));
        }

        let calls: Vec<&Element> = std::iter::once(head)
            .filter_map(|n| n.as_element())
            .filter(|c| c.kind == ElementKind::CallExpr)
            .filter_map(|c| c.as_call_expr().map(|v| v.args()))
            .chain(
                segments
                    .iter()
                    .filter_map(|s| s.as_segment().and_then(|v| v.call())),
            )
            .collect();
        let not_simple = calls.len() > 2
            && calls
                .iter()
                .any(|a| printable_items(a).iter().any(|i| !self.is_simple_item(i)));
        let early_break = will_break(&first) || printed[..printed.len() - 1].iter().any(will_break);
        let last_group_breaks = groups
            .last()
            .and_then(|g| g.last())
            .is_some_and(|s| is_method(s))
            && printed.last().is_some_and(will_break)
            && calls[..calls.len() - 1]
                .iter()
                .any(|a| printable_items(a).iter().any(|i| is_function_item(i)));
        let any_breaks = printed.iter().any(will_break) || will_break(&first);

        let mut printed = printed.into_iter();
        let mut lead = vec![first];
        if merge {
            lead.extend(printed.next());
        }
        let rest: Vec<Doc> = printed.flat_map(|g| [softline(), g]).collect();
        if forced || not_simple || early_break || last_group_breaks {
            lead.push(indent(rest));
            return group_opts(
                Doc::Concat(lead),
                GroupOpts {
                    should_break: true,
                    ..GroupOpts::default()
                },
            );
        }
        // One copy serves both of Prettier's states: flat it is the chain on
        // one line (a broken last group still breaks inside, not indented:
        // the indent is the group's `indent_if_break`), broken it is a group
        // per line. The conditional group passes no break up, so a forced
        // break inside is announced beside it.
        let id = self.ids.borrow_mut().next_id();
        lead.push(indent_if_break(rest, id, false));
        let mut chain = conditional_group_contents(Doc::Concat(lead));
        if let Doc::Group(g) = &mut chain {
            g.id = Some(id);
        }
        if any_breaks {
            Doc::Concat(vec![break_parent(), chain])
        } else {
            chain
        }
    }

    /// Whether a chain prints as a member chain that may break a group per
    /// line (Prettier labels those `memberChain`): one with a comment, or
    /// with more groups than its cutoff (or with the forced break); a
    /// shorter one is its parts on one line.
    pub(crate) fn is_member_chain(&self, e: &Element) -> bool {
        if e.children
            .iter()
            .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()))
        {
            return true;
        }
        let methods = e
            .children
            .iter()
            .filter(|n| {
                matches!(n, Node::Element(s) if s.kind == ElementKind::Segment(SegmentKind::Method))
            })
            .count();
        if methods <= 1 {
            return false;
        }
        let forced = self.opts.method_call_chain_multiline > 0
            && methods as u32 >= self.opts.method_call_chain_multiline;
        let p = self.partition(e, false);
        forced || p.groups.len() >= if p.merge { 3 } else { 2 }
    }

    /// A comment-free chain's groups (see the module docs).
    fn partition<'e>(&self, e: &'e Element, statement_start: bool) -> Partition<'e> {
        let mut sig = e.children.iter().filter(|n| !n.is_trivia());
        let head = sig.next().expect("chain head");
        let segments: Vec<&Element> = sig.filter_map(Node::as_element).collect();
        let kind = |s: &Element| s.as_segment().map(|v| v.kind());
        let is_method = |s: &Element| kind(s) == Some(SegmentKind::Method);
        let is_index = |s: &Element| kind(s) == Some(SegmentKind::Index);
        let head_call = matches!(head, Node::Element(c) if c.kind == ElementKind::CallExpr);

        // The first group: the head, a static access on it, the number
        // indexes after it, then every property or index but the last
        // before the first method.
        let mut i = 0;
        if segments
            .first()
            .is_some_and(|s| s.as_segment().is_some_and(|v| v.is_static()))
        {
            i = 1;
        }
        while i < segments.len() && is_number_index(segments[i]) {
            i += 1;
        }
        if !head_call {
            while i + 1 < segments.len() && !is_method(segments[i]) {
                i += 1;
            }
        }
        let last_method = segments.iter().rposition(|s| is_method(s));
        let mut groups: Vec<Vec<&Element>> = Vec::new();
        let mut current: Vec<&Element> = Vec::new();
        let mut seen_call = false;
        for (at, s) in segments.iter().enumerate().skip(i) {
            if seen_call && !is_index(s) && last_method.is_some_and(|m| at <= m) {
                groups.push(std::mem::take(&mut current));
                seen_call = false;
            }
            seen_call |= is_method(s);
            current.push(s);
        }
        if !current.is_empty() {
            groups.push(current);
        }

        let has_computed = groups
            .first()
            .and_then(|g| g.first())
            .is_some_and(|s| is_index(s));
        let merge = !groups.is_empty()
            && if i == 0 {
                match head {
                    Node::Token(t) => {
                        let name = self.tree.text(t);
                        t.kind == TokenKind::Ident(Ident::This)
                            || is_factory(name)
                            || (statement_start && name.chars().count() <= self.opts.indent_size)
                            || has_computed
                    }
                    Node::Element(_) => false,
                }
            } else {
                let last = segments[i - 1];
                kind(last) == Some(SegmentKind::Property)
                    && last
                        .as_segment()
                        .and_then(|v| v.name())
                        .is_some_and(|t| is_factory(self.tree.text(t)) || has_computed)
            };
        Partition {
            head,
            segments,
            first: i,
            groups,
            merge,
        }
    }

    /// Prettier's `isSimpleCallArgument` for a call argument (a named
    /// argument's value).
    fn is_simple_item(&self, item: &Item) -> bool {
        let mut sig = item.significant();
        match (sig.next(), sig.next()) {
            (Some(Node::Element(kv)), None) if kv.kind == ElementKind::KeyValue => {
                kv.as_key_value().map(|v| v.value()).is_some_and(|value| {
                    let mut sig = value.iter().filter(|n| !n.is_trivia());
                    matches!((sig.next(), sig.next()), (Some(n), None) if self.is_simple_arg(n, 2))
                })
            }
            (Some(n), None) => self.is_simple_arg(n, 2),
            _ => false,
        }
    }

    /// Prettier's `isSimpleCallArgument`: a name, a literal, a string whose
    /// interpolations are simple, a struct or array of simple values, a
    /// simple unary operand, a member access over simple parts, or a call
    /// with a simple callee and at most `depth` simple arguments, `depth`
    /// dropping by one per level of nesting.
    fn is_simple_arg(&self, n: &Node, depth: usize) -> bool {
        if depth == 0 {
            return false;
        }
        let Node::Element(e) = n else {
            return true;
        };
        let simple_items = |a: &Element, depth: usize| {
            let items = printable_items(a);
            items.len() <= depth && items.iter().all(|i| {
                let mut s = i.significant();
                matches!((s.next(), s.next()), (Some(n), None) if self.is_simple_arg(n, depth - 1))
            })
        };
        match e.kind {
            ElementKind::String { .. } => e.children.iter().all(|c| match c {
                Node::Token(t) => !self.tree.text(t).contains('\n'),
                // The `#`s are the template's delimiters, not children.
                Node::Element(t) if t.kind == ElementKind::TemplateExpression => sig(t)
                    .into_iter()
                    .all(|n| self.is_simple_arg(n, depth - 1)),
                Node::Element(_) => true,
            }),
            ElementKind::Struct { .. } | ElementKind::Array => {
                printable_items(e).iter().all(|i| {
                    let mut s = i.significant();
                    match (s.next(), s.next()) {
                        (Some(Node::Element(kv)), None) if kv.kind == ElementKind::KeyValue => {
                            kv.as_key_value().is_some_and(|v| {
                                let mut s = v.value().iter().filter(|n| !n.is_trivia());
                                matches!((s.next(), s.next()), (Some(n), None) if self.is_simple_arg(n, depth - 1))
                            })
                        }
                        (Some(n), None) => self.is_simple_arg(n, depth - 1),
                        _ => false,
                    }
                })
            }
            ElementKind::Group => {
                matches!(sig(e)[..], [n] if self.is_simple_arg(n, depth))
            }
            ElementKind::Unary { postfix } => e.as_unary().is_some_and(|u| {
                (postfix
                    || matches!(
                        u.op().kind,
                        TokenKind::Operator(Operator::Not { .. } | Operator::Sign)
                    ))
                    && self.is_simple_arg(u.operand(), depth)
            }),
            ElementKind::CallExpr => e.as_call_expr().is_some_and(|c| {
                self.is_simple_arg(c.callee(), depth) && simple_items(c.args(), depth)
            }),
            ElementKind::New => e.as_new().is_some_and(|v| {
                self.is_simple_arg(v.class(), depth) && v.args().is_none_or(|a| simple_items(a, depth))
            }),
            ElementKind::Chain => e.as_chain().is_some_and(|c| {
                self.is_simple_arg(c.head(), depth)
                    && c.segments().all(|s| match s.as_segment() {
                        Some(v) if v.kind() == SegmentKind::Method => {
                            v.call().is_some_and(|a| simple_items(a, depth))
                        }
                        Some(v) if v.kind() == SegmentKind::Index => {
                            v.brackets().is_some_and(|b| matches!(sig(b)[..], [n] if self.is_simple_arg(n, depth)))
                        }
                        _ => true,
                    })
            }),
            _ => false,
        }
    }

    /// A chain that never breaks: its parts in order, block comments in
    /// place with a space on each side.
    fn flat_chain(&self, e: &Element) -> Vec<Doc> {
        let mut parts = Vec::new();
        let mut space = false;
        for n in &e.children {
            match n {
                Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline) => {}
                Node::Element(c) if c.kind.is_comment() => {
                    parts.extend([Doc::from(" "), self.comment(c)]);
                    space = true;
                }
                n => {
                    if std::mem::take(&mut space) {
                        parts.push(Doc::from(" "));
                    }
                    parts.push(match n {
                        Node::Element(s) if matches!(s.kind, ElementKind::Segment(_)) => {
                            self.segment(s)
                        }
                        n => self.node(n),
                    });
                }
            }
        }
        parts
    }

    /// One access: `.name`, `?.name`, `::name` as written, a method's
    /// arguments through [`Printer::call_args`] (the member name is never
    /// cased), an index through [`Printer::brackets`]. A comment inside the
    /// access (`.` newline `// c` `name`) has no line break after it: a line
    /// comment ends the line the chain ends on ([`Printer::deferred_comment`]).
    pub(crate) fn segment(&self, e: &Element) -> Doc {
        let mut parts = Vec::new();
        for n in &e.children {
            match n {
                Node::Element(c) if c.kind.is_comment() => parts.push(self.deferred_comment(c)),
                n if n.is_trivia() => {}
                Node::Element(a) if matches!(a.kind, ElementKind::Call) => {
                    parts.push(self.call_args(a));
                }
                n => parts.push(self.node(n)),
            }
        }
        Doc::Concat(parts)
    }
}

/// Prettier's `isFactory`: a capital first letter, or only `$` and `_`.
fn is_factory(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        || (!name.is_empty() && name.chars().all(|c| c == '$' || c == '_'))
}

/// An index whose key is a number literal (`[0]`).
fn is_number_index(s: &Element) -> bool {
    s.as_segment().and_then(|v| v.brackets()).is_some_and(|b| {
        let mut sig = b.children.iter().filter(|n| !n.is_trivia());
        matches!(
            (sig.next(), sig.next()),
            (Some(Node::Token(t)), None) if t.kind == TokenKind::Literal(Literal::Number)
        )
    })
}

/// A call argument that is a function or arrow (a named argument's value
/// included).
fn is_function_item(item: &Item) -> bool {
    let mut sig = item.significant();
    match (sig.next(), sig.next()) {
        (Some(Node::Element(kv)), None) if kv.kind == ElementKind::KeyValue => {
            kv.as_key_value().is_some_and(|v| {
                v.value()
                    .iter()
                    .any(|n| matches!(n, Node::Element(f) if matches!(f.kind, ElementKind::Function { .. })))
            })
        }
        (Some(Node::Element(f)), None) => matches!(f.kind, ElementKind::Function { .. }),
        _ => false,
    }
}

/// An element's children without trivia.
fn sig(e: &Element) -> Vec<&Node> {
    e.children.iter().filter(|n| !n.is_trivia()).collect()
}

/// A chain's parts in Prettier's groups ([`Printer::partition`]).
struct Partition<'e> {
    head: &'e Node,
    segments: Vec<&'e Element>,
    /// How many segments the first group holds after the head.
    first: usize,
    /// The groups after the first.
    groups: Vec<Vec<&'e Element>>,
    /// The first two groups share a line (Prettier's `shouldNotWrap`).
    merge: bool,
}

/// Whether `doc` printed flat is at least a column wide (or cannot print
/// flat): `flat_width(doc) > 0`, stopping at the first part that is.
fn prints_flat(doc: &Doc) -> bool {
    match doc {
        Doc::Concat(parts) | Doc::Fill(parts) => parts.iter().any(prints_flat),
        Doc::Indent(contents) | Doc::Align(_, contents) | Doc::IndentIfBreak { contents, .. } => {
            prints_flat(contents)
        }
        Doc::Group(group) => prints_flat(&group.contents),
        Doc::IfBreak { flat_doc, .. } => prints_flat(flat_doc),
        leaf => flat_width(leaf) > FlatWidth::Finite(0),
    }
}
