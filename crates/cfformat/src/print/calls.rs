//! Calls: `CallExpr` callees with their casing and the argument layout,
//! including Prettier's argument hugging (its `printCallArguments`).
//!
//! The inline, hugged and broken argument lists are states of one conditional
//! group, and `fits` picks among them:
//!
//! - last argument a function, struct or array (any argument count), unless
//!   the argument before it is of the same kind (Prettier's
//!   `shouldExpandLastArg`: `f({a: 1}, {b: 2})` breaks every argument or
//!   none): `[all flat, hug last, all broken]`, or `[hug last, all broken]`
//!   behind a `break_parent` when the last argument contains a forced break;
//! - a brace-less arrow counts as a last function only when its body is a
//!   call, a chain ending in one, a ternary, a struct or an array (Prettier's
//!   `couldExpandArg`), and only when its parameters need not break; hugged,
//!   it is printed again with flat parameters and its body after `=>`
//!   (`expandLastArg`: `foo(a, (k) =>` ⏎ `body` ⏎ `)`);
//! - first argument a function followed by exactly one more: the same with
//!   the first argument hugged;
//! - every argument a function: the hugged layout only;
//! - a single string argument that spans lines: `(string)`, no group
//!   (Prettier's `isTemplateOnItsOwnLine`); a single-line one is an ordinary
//!   argument;
//! - anything else, any argument with comments, a head argument that will
//!   break, or a list the `function_call` threshold forces to break: the
//!   delimited layout.

use cfdoc::builders::{
    break_parent, conditional_group, conditional_group_contents, group_opts, GroupOpts,
};
use cfdoc::utils::{find_in_doc, flat_width, remove_lines, will_break};
use cfdoc::{Doc, FlatWidth, GroupId};
use cfparse::{Element, ElementKind, Ident, Item, Node, SegmentKind, TokenKind};

use super::casing;
use super::delimited::{has_comments, is_printable, plain_items, printable_items, DelimitedStyle};
use super::expressions::is_parenthesised_literal;
use super::functions::{arrow_breaks_after, arrow_expression};
use super::Printer;

/// How many hugging argument lists a hugged arrow may hold and still get
/// the three layouts ([`Printer::call_args`]).
const NESTED_ARROW_HUGS: usize = 2;

/// What an argument is, for hugging.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arg {
    /// An anonymous function, or an arrow with a braced body.
    Function,
    /// A brace-less arrow whose body can hug ([`arrow_hugs`]).
    Arrow,
    Struct,
    Array,
    String,
    Other,
}

impl Printer<'_> {
    /// `callee(args)`: the callee through [`Printer::node`] with the
    /// function-name casing applied to a plain callee token.
    pub(crate) fn call_expr(&self, e: &Element) -> Doc {
        let Some(call) = e.as_call_expr() else {
            return self.as_written(e);
        };
        let args = call.args();
        let mut parts = Vec::new();
        for n in &e.children {
            match n {
                Node::Element(c) if c.kind.is_comment() => parts.push(self.same_line_comment(c)),
                n if n.is_trivia() => {}
                Node::Token(t) if std::ptr::eq(n, call.callee()) => {
                    parts.push(self.callee(t));
                }
                Node::Element(a) if std::ptr::eq(&**a, args) => parts.push(self.call_args(a)),
                n => parts.push(self.node(n)),
            }
        }
        Doc::Concat(parts)
    }

    /// A plain callee token with `function_call.casing.*` applied.
    fn callee(&self, t: &cfparse::Token) -> Doc {
        let name = self.tree.text(t);
        Doc::from(match t.kind {
            TokenKind::Ident(Ident::Builtin) => {
                casing::builtin(name, self.opts.function_call_casing_builtin)
            }
            // A call expression's callee is a plain call's name (a method's
            // is a segment's).
            TokenKind::Ident(Ident::Call) => {
                casing::user_defined(name, self.opts.function_call_casing_userdefined)
            }
            _ => name.to_owned(),
        })
    }

    /// The arguments of a call, constructor or method (see the module docs).
    pub(crate) fn call_args(&self, e: &Element) -> Doc {
        self.hugging_args(e, &DelimitedStyle::function_call(self.opts))
    }

    fn hugging_args(&self, e: &Element, style: &DelimitedStyle) -> Doc {
        let items = printable_items(e);
        // Without both parentheses (a call the source never closed),
        // `print_delimited` prints it as written.
        if items.is_empty()
            || items.iter().any(|i| has_comments(i))
            || e.open.is_none()
            || e.close.is_none()
        {
            return self.print_delimited(e, style);
        }
        let kinds: Vec<Arg> = items.iter().map(|i| arg_kind(i)).collect();
        let pad = || Doc::from(if style.padding { " " } else { "" });
        let open = || self.token(e.open.as_ref().expect("call arguments"));
        let close = || self.token(e.close.as_ref().expect("call arguments"));
        let content = |i: usize| self.item_content(items[i], style.key_value, None);

        if let [Arg::String] = kinds[..] {
            // Prettier's `isTemplateOnItsOwnLine`: a lone string that spans
            // lines hugs the parentheses; any other lone string is an
            // ordinary argument and breaks onto its own line when the call
            // does not fit.
            let only = content(0);
            if will_break(&only) {
                return Doc::Concat(vec![open(), pad(), only, pad(), close()]);
            }
            return self.print_delimited(e, style);
        }
        if kinds.iter().all(|k| *k == Arg::Function) {
            let mut parts = vec![open(), pad()];
            for i in 0..items.len() {
                if i > 0 {
                    parts.push(Doc::from(", "));
                }
                parts.push(content(i));
            }
            parts.extend([pad(), close()]);
            return Doc::Concat(parts);
        }

        let last = items.len() - 1;
        // Prettier: "if the last two arguments are of the same type, disable
        // last element expansion" — `f(name, {a: 1}, {b: 2})` never hugs its
        // last struct, so the arguments print all flat or one per line.
        let same_kind = last > 0 && kinds[last - 1] == kinds[last];
        // Prettier's `couldExpandArg`: an empty struct or array (no item, no
        // comment) cannot expand, so it neither hugs nor stops a first
        // function from hugging (`f(fn, {})`); it still counts as a struct
        // for the same-kind rule (`f(sql, {}, {a: 1})` does not hug).
        let expandable = |i: usize| match kinds[i] {
            Arg::Function | Arg::Arrow => true,
            Arg::Struct | Arg::Array => arg_node(items[i]).is_some_and(|(n, _)| {
                n.as_element().is_some_and(|e| {
                    e.items.iter().any(is_printable)
                        || e.children
                            .iter()
                            .any(|c| c.as_element().is_some_and(|c| c.kind.is_comment()))
                })
            }),
            Arg::String | Arg::Other => false,
        };
        let hug = if expandable(last) && !same_kind {
            last
        } else if items.len() == 2 && kinds[0] == Arg::Function && !expandable(1) {
            0
        } else {
            return self.print_delimited(e, style);
        };
        // Named arguments align when the list breaks, on the id of whichever
        // group below holds the list (`alignment.consecutive.assignments`).
        let pads = self.item_pads(&items, style);
        let align_id = self.alignment_id(&pads);
        // Each argument is printed once; the layouts below share these docs.
        let mut args: Vec<Doc> = (0..items.len())
            .map(|i| {
                self.item_content(
                    items[i],
                    style.key_value,
                    Self::item_pad(pads[i], align_id).as_ref(),
                )
            })
            .collect();
        // Measured only for a threshold that can fire: off (the default),
        // `style.breaks` reads nothing.
        let widths: Vec<FlatWidth> = if style.measures(items.len()) {
            args.iter().map(flat_width).collect()
        } else {
            Vec::new()
        };
        let broken = |body: Doc| {
            group_opts(
                body,
                GroupOpts {
                    id: align_id,
                    should_break: true,
                },
            )
        };
        if args
            .iter()
            .enumerate()
            .any(|(i, a)| i != hug && will_break(a))
        {
            let body = self.delimited_body(e, style, plain_items(args, style), None);
            return broken(body);
        }
        // Prettier's `ArgExpansionBailout`: a function whose parameters must
        // break (their `function_anonymous` threshold, a forced break) is not
        // hugged, since the hugged state keeps them on one line.
        if matches!(kinds[hug], Arg::Arrow | Arg::Function) && self.params_break(items[hug]) {
            let body = self.delimited_body(e, style, plain_items(args, style), None);
            return broken(body);
        }
        // The threshold of the delimited layout still applies: a list it
        // forces to break is never hugged.
        if style.breaks(&widths) {
            let body = self.delimited_body(e, style, plain_items(args, style), None);
            return broken(body);
        }
        // Every layout below is the delimited body in one conditional group:
        // printed flat it is `(a, b)` (the indent is `indent_if_break` on the
        // group), broken it is one argument per line.
        let id = align_id.unwrap_or_else(|| self.ids.borrow_mut().next_id());
        let with_id = |mut doc: Doc| {
            if let Doc::Group(g) = &mut doc {
                g.id = Some(id);
            }
            doc
        };
        let body =
            |args: Vec<Doc>| self.delimited_body(e, style, plain_items(args, style), Some(id));
        // A hugged brace-less arrow is printed again for the hugged state
        // (Prettier's `expandLastArg`): its parameters never break and its
        // body breaks after `=>`, with the `)` on a line of its own; the
        // returned id is its body group's, which the closing padding reads.
        let hugged_arg = |args: &mut Vec<Doc>| {
            if kinds[hug] == Arg::Function {
                // A braced function's parameters never break either
                // (`expandFirstArg` / `expandLastArg`); its doc's first group
                // is its parameter list when it has one, flattened in place.
                if has_params(items[hug]) {
                    flatten_first_group(&mut args[hug]);
                }
                return None;
            }
            if kinds[hug] != Arg::Arrow {
                return None;
            }
            // A struct or array body stays on the arrow's line: no body
            // group, and the closing padding is the list's.
            let breaks = arrow_decl(items[hug])
                .and_then(|f| f.as_decl())
                .is_some_and(|d| arrow_breaks_after(d.body()));
            let body_id = self.ids.borrow_mut().next_id();
            self.hug_arrow.set(Some(body_id));
            args[hug] = self.item_content(
                items[hug],
                style.key_value,
                Self::item_pad(pads[hug], align_id).as_ref(),
            );
            self.hug_arrow.set(None);
            breaks.then_some(body_id)
        };
        let hugged_body = |args: Vec<Doc>, close_pad_id: Option<GroupId>| {
            self.delimited_body_with(e, style, plain_items(args, style), Some(id), close_pad_id)
        };
        let broken_hug = |doc: Doc| {
            group_opts(
                doc,
                GroupOpts {
                    should_break: true,
                    ..GroupOpts::default()
                },
            )
        };
        if will_break(&args[hug]) {
            // Prettier: `[breakParent, conditionalGroup([hugged, allBroken])]`;
            // both are the body with the hugged argument broken, so one copy
            // serves.
            let close_pad_id = hugged_arg(&mut args);
            args[hug] = broken_hug(std::mem::take(&mut args[hug]));
            return Doc::Concat(vec![
                break_parent(),
                with_id(conditional_group_contents(hugged_body(args, close_pad_id))),
            ]);
        }
        // A hugged arrow whose body hugs arguments of its own (`expect(() =>
        // run(x, { … }))`) keeps the three states while few such groups nest
        // in it: each level stores its argument three times, so the count
        // bounds the growth; past it the shortcut below applies.
        let mut nested = 0;
        let hugs_inside = find_in_doc(&args[hug], |d| {
            if matches!(d, Doc::Group(g) if g.expanded_states.as_ref().is_some_and(|s| !s.is_empty())) {
                nested += 1;
                let limit = if kinds[hug] == Arg::Arrow {
                    NESTED_ARROW_HUGS
                } else {
                    0
                };
                return (nested > limit).then_some(());
            }
            None
        })
        .is_some();
        if hugs_inside {
            // The hugged argument hugs arguments of its own: all flat or all
            // broken, stored once, so nesting stays linear.
            return with_id(conditional_group_contents(body(args)));
        }
        // Prettier's three states: all flat, the hugged argument broken, all
        // broken — the first and the last are the same body, which
        // `indent_if_break` on the group's id lays out flat or broken. The
        // first state is stored rather than left empty so that it and the
        // group's `contents` stay equal, which is what `Doc::map`
        // (`remove_lines` over a `#expr#`) assumes.
        let all = body(args.clone());
        let close_pad_id = hugged_arg(&mut args);
        args[hug] = broken_hug(std::mem::take(&mut args[hug]));
        with_id(conditional_group(vec![
            all.clone(),
            hugged_body(args, close_pad_id),
            all,
        ]))
    }
}

impl Printer<'_> {
    /// Whether a function argument's header (its parameters) holds a forced
    /// break.
    fn params_break(&self, item: &Item) -> bool {
        let Some(Node::Element(h)) = arrow_decl(item)
            .and_then(|f| f.as_decl())
            .map(|d| d.header())
        else {
            return false;
        };
        will_break(&self.function_header(h).0)
    }
}

/// Whether a function argument has parameters (so its header prints a
/// parameter group).
fn has_params(item: &Item) -> bool {
    let Some(Node::Element(h)) = arrow_decl(item)
        .and_then(|f| f.as_decl())
        .map(|d| d.header())
    else {
        return false;
    };
    h.children.iter().any(|n| {
        matches!(n, Node::Element(p) if p.kind == ElementKind::Parameters
            && p.open.is_some()
            && p.items.iter().any(is_printable))
    })
}

/// Replaces the first group met in document order with its flat form
/// (`remove_lines`); `true` once done.
fn flatten_first_group(doc: &mut Doc) -> bool {
    match doc {
        Doc::Group(_) => {
            *doc = remove_lines(std::mem::take(doc));
            true
        }
        Doc::Concat(parts) | Doc::Fill(parts) => parts.iter_mut().any(flatten_first_group),
        Doc::Indent(c) | Doc::Align(_, c) | Doc::IndentIfBreak { contents: c, .. } => {
            flatten_first_group(c)
        }
        _ => false,
    }
}

/// The function element of an argument that is one (a named argument's
/// value included).
fn arrow_decl(item: &Item) -> Option<&Element> {
    match arg_node(item) {
        Some((Node::Element(f), _)) if matches!(f.kind, ElementKind::Function { .. }) => Some(f),
        _ => None,
    }
}

/// The argument's one node, looking through a named argument to its value
/// (`true` when it did), or `None` for anything else.
fn arg_node(item: &Item) -> Option<(&Node, bool)> {
    let mut sig = item.significant();
    let (Some(node), None) = (sig.next(), sig.next()) else {
        return None;
    };
    match node {
        Node::Element(kv) if kv.kind == ElementKind::KeyValue => {
            let value = kv.as_key_value().expect("key-value").value();
            let mut sig = value.iter().filter(|n| !n.is_trivia());
            match (sig.next(), sig.next()) {
                (Some(v), None) => Some((v, true)),
                _ => None,
            }
        }
        n => Some((n, false)),
    }
}

/// An argument's kind, looking through a named argument's value for a
/// function, struct or array (a named string argument does not hug).
fn arg_kind(item: &Item) -> Arg {
    let Some((node, named)) = arg_node(item) else {
        return Arg::Other;
    };
    match node {
        Node::Element(e) => match e.kind {
            ElementKind::Function { .. } => match e.as_decl() {
                Some(d) if d.arrow().is_some() && d.body().open.is_none() => {
                    if arrow_hugs(d.body()) {
                        Arg::Arrow
                    } else {
                        Arg::Other
                    }
                }
                _ => Arg::Function,
            },
            ElementKind::Struct { .. } => Arg::Struct,
            ElementKind::Array => Arg::Array,
            ElementKind::String { .. } if !named => Arg::String,
            _ => Arg::Other,
        },
        Node::Token(_) => Arg::Other,
    }
}

/// Prettier's `couldExpandArg` for an arrow with an expression body: it hugs
/// when the body is a call (a method chain ending in one included), a
/// ternary, a struct or an array (parenthesised or not). Any other body (`(k) => a && b`) leaves the
/// arguments all flat or one per line.
fn arrow_hugs(body: &Element) -> bool {
    let Some(e) = arrow_expression(body) else {
        return false;
    };
    match e.kind {
        ElementKind::CallExpr
        | ElementKind::Ternary
        | ElementKind::Struct { .. }
        | ElementKind::Array => true,
        ElementKind::Group => is_parenthesised_literal(e),
        ElementKind::Chain => e
            .as_chain()
            .and_then(|c| c.segments().last())
            .and_then(|s| s.as_segment())
            .is_some_and(|s| s.kind() == SegmentKind::Method),
        _ => false,
    }
}
