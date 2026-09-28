//! `unevaluated-call`: a `#…#` that calls a function, in text the engine
//! outputs as written.
//!
//! Outside `<cfoutput>` a `#…#` in a template is text: `<a
//! onclick="#showDetail(1)#">` reaches the browser as written and
//! `showDetail` never runs. The parser parses `#…#` where it is live (CF
//! tag attributes, script, the bodies of `<cfoutput>`, `<cfmail>` and
//! `<cfquery>`), so a `#` left in HTML text, an HTML attribute value or an
//! island's text is one the engine does not read, with two exceptions the
//! rule models itself. Those three bodies are paired by name in source
//! order ([`live_spans`]), as the engines pair them, since the tree nests
//! them in HTML and a `<cfoutput>` opened in a `<style>` ends there. And
//! `output`: a `<cffunction output="true">` body is read as if
//! it were in `<cfoutput>`, and so is a function without `output` in a
//! `<cfcomponent output="true">` (checked on Lucee 6 and Adobe 2021,
//! 2026-09-25). An included template does not inherit either, so each file
//! stands alone.

use std::ops::Range;

use cfparse::{CfKind, Element, ElementKind, Literal, Node, TagShape, TokenKind, Tree};

use crate::Report;

pub const RULE: &str = "unevaluated-call";

/// The longest `#…#` the rule reads, in bytes.
const MAX_LEN: usize = 500;

pub(crate) fn check(tree: &Tree) -> Vec<Report> {
    let mut check = Check {
        tree,
        reports: Vec::new(),
        after: 0,
        live: live_spans(tree),
    };
    check.nodes(&tree.root, Ctx::default());
    check.reports
}

/// The tags whose bodies read `#…#` (the parser parses it there).
const LIVE_TAGS: [&str; 3] = ["cfoutput", "cfmail", "cfquery"];

/// The bodies of `<cfoutput>`, `<cfmail>` and `<cfquery>`, paired by name
/// in source order as the engines pair them: whatever HTML they cross. The
/// tree nests them in HTML, so `<style><cfoutput>…</style>…</cfoutput>`
/// leaves the `<cfoutput>` in the `<style>` island and the text after
/// `</style>` outside it. An opener with no closer runs to the end.
fn live_spans(tree: &Tree) -> Vec<Range<usize>> {
    fn find(tree: &Tree, el: &Element, out: &mut Vec<(usize, &'static str, TagShape)>) {
        for node in el.nodes() {
            let Node::Element(e) = node else { continue };
            if let ElementKind::CfTag(shape, _) = e.kind {
                let name = tree.tag_name(e).unwrap_or_default();
                if let Some(live) = LIVE_TAGS.iter().find(|n| name.eq_ignore_ascii_case(n)) {
                    let at = match shape {
                        TagShape::Close => e.span.start,
                        _ => e.span.end,
                    };
                    out.push((at as usize, live, shape));
                }
            }
            find(tree, e, out);
        }
    }
    let mut tags = Vec::new();
    find(tree, &tree.root, &mut tags);
    tags.sort_by_key(|&(at, _, _)| at);
    let mut open: Vec<(usize, &str)> = Vec::new();
    let mut out = Vec::new();
    for (at, name, shape) in tags {
        match shape {
            TagShape::Open => open.push((at, name)),
            TagShape::Close => {
                if let Some(i) = open.iter().rposition(|&(_, n)| n == name) {
                    out.push(open[i].0..at);
                    open.truncate(i);
                }
            }
            TagShape::SelfClosed => {}
        }
    }
    out.extend(open.into_iter().map(|(at, _)| at..tree.source.len()));
    out
}

/// Where a node stands.
#[derive(Clone, Copy, Default)]
struct Ctx {
    /// The enclosing `<cfcomponent>` has `output="true"`, so a function
    /// without `output` reads `#…#`.
    component_output: bool,
    /// Inside an HTML tag, where an attribute value's text is output.
    html_tag: bool,
}

struct Check<'a> {
    tree: &'a Tree,
    reports: Vec<Report>,
    /// The end of the last `#…#` reported, so its closing `#` is not read as
    /// an opener.
    after: usize,
    /// Where `#…#` is read ([`live_spans`]).
    live: Vec<Range<usize>>,
}

impl Check<'_> {
    fn nodes(&mut self, el: &Element, ctx: Ctx) {
        for node in el.nodes() {
            match node {
                Node::Token(t) => {
                    let text = t.kind == TokenKind::Text
                        || (ctx.html_tag && t.kind == TokenKind::Literal(Literal::StringText));
                    if text {
                        let start = t.span.start as usize;
                        for (i, _) in self.tree.text(t).match_indices('#') {
                            self.hash(start + i);
                        }
                    }
                }
                Node::Element(e) => self.element(e, ctx),
            }
        }
    }

    fn element(&mut self, e: &Element, ctx: Ctx) {
        match e.kind {
            // Comments are not output; a recovered or ignored region holds
            // no tree to read; a CF tag's own attributes and script read
            // `#…#`, and a template expression is one.
            _ if e.kind.is_comment() => {}
            ElementKind::Recovered(_)
            | ElementKind::Ignore
            | ElementKind::CfTag(..)
            | ElementKind::TemplateExpression => {}
            ElementKind::HtmlTag(_) => self.nodes(
                e,
                Ctx {
                    html_tag: true,
                    ..ctx
                },
            ),
            ElementKind::TagBody { cf: true } => {
                // `<cfcomponent>` by name: its kind is `Class` only at the
                // head of the file.
                let component = self
                    .tree
                    .tag_name(e)
                    .is_some_and(|n| n.eq_ignore_ascii_case("cfcomponent"));
                let function = e.cf_kind() == Some(CfKind::Function);
                let output = e.open_tag().map(|open| self.output(open));
                match output {
                    Some(Output::Off | Output::Absent) if component => self.nodes(
                        e,
                        Ctx {
                            component_output: false,
                            ..ctx
                        },
                    ),
                    // Lucee reads the pseudo-constructor's `#…#` and Adobe
                    // refuses it; the functions inherit it.
                    _ if component => self.body_only_functions(
                        e,
                        Ctx {
                            component_output: true,
                            ..ctx
                        },
                    ),
                    Some(Output::Off) if function => self.nodes(e, ctx),
                    Some(Output::Absent) if function && !ctx.component_output => self.nodes(e, ctx),
                    _ if function => {}
                    _ => self.nodes(e, ctx),
                }
            }
            _ => self.nodes(e, ctx),
        }
    }

    /// An `output="true"` component: only its functions can hold text the
    /// engine outputs as written.
    fn body_only_functions(&mut self, e: &Element, ctx: Ctx) {
        for node in e.nodes() {
            if let Node::Element(c) = node {
                let function = matches!(c.kind, ElementKind::TagBody { cf: true })
                    && c.cf_kind() == Some(CfKind::Function);
                if function {
                    self.element(c, ctx);
                } else if !matches!(c.kind, ElementKind::CfTag(..)) && !c.kind.is_comment() {
                    self.body_only_functions(c, ctx);
                }
            }
        }
    }

    /// A tag's `output` attribute.
    fn output(&self, open: &Element) -> Output {
        let has = crate::rules::attributes(open)
            .any(|(key, _)| self.tree.text(key).eq_ignore_ascii_case("output"));
        if !has {
            return Output::Absent;
        }
        match crate::rules::attribute(self.tree, open, "output")
            .map(|(v, _)| v.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("false" | "no" | "0") => Output::Off,
            // `true`, `yes`, a number, or a computed value: the body may
            // read `#…#`, so nothing in it is reported.
            _ => Output::On,
        }
    }

    /// A `#` at `at` in text the engine outputs as written.
    fn hash(&mut self, at: usize) {
        if at < self.after || self.live.iter().any(|span| span.contains(&at)) {
            return;
        }
        let Some(end) = call_end(&self.tree.source, at) else {
            return;
        };
        self.after = end;
        let shown = shown(&self.tree.source[at..end]);
        let (line, column) = crate::position(self.tree, at as u32);
        self.reports.push(Report {
            line,
            column,
            rule: RULE,
            message: format!(
                "`{shown}` is not inside `<cfoutput>`: it is output as written and the call never runs"
            ),
            name: shown,
            var_not_run: false,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    On,
    Off,
    Absent,
}

/// Where a `#…#` that starts at `at` and calls a function ends (one past
/// its closing `#`): a name, then member names, index brackets and
/// argument lists, at least one argument list, then `#`. Brackets balance,
/// quotes in them hold anything but their own quote (doubled to escape
/// it).
fn call_end(src: &str, at: usize) -> Option<usize> {
    let b = src.as_bytes();
    let limit = b.len().min(at + MAX_LEN);
    let mut i = at + 1;
    let mut calls = false;
    i = name_end(b, i, limit)?;
    loop {
        match b.get(i).filter(|_| i < limit)? {
            b'#' if calls => return Some(i + 1),
            b'.' => i = name_end(b, i + 1, limit)?,
            b'(' => {
                i = group_end(b, i, limit)?;
                calls = true;
            }
            b'[' => i = group_end(b, i, limit)?,
            _ => return None,
        }
    }
}

/// The end of the name that starts at `i`.
fn name_end(b: &[u8], i: usize, limit: usize) -> Option<usize> {
    let first = *b.get(i).filter(|_| i < limit)?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return None;
    }
    let len = b[i..limit]
        .iter()
        .position(|&c| !(c.is_ascii_alphanumeric() || c == b'_' || c == b'$'))
        .unwrap_or(limit - i);
    Some(i + len)
}

/// The end of the bracket group that opens at `i` (one past its closer).
fn group_end(b: &[u8], i: usize, limit: usize) -> Option<usize> {
    let mut stack = vec![b[i]];
    let mut j = i + 1;
    while j < limit {
        match b[j] {
            q @ (b'"' | b'\'') => {
                j += 1;
                loop {
                    if j >= limit {
                        return None;
                    }
                    if b[j] == q {
                        if b.get(j + 1) == Some(&q) {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    j += 1;
                }
            }
            c @ (b'(' | b'[' | b'{') => stack.push(c),
            c @ (b')' | b']' | b'}') => {
                let open = stack.pop()?;
                if !matches!((open, c), (b'(', b')') | (b'[', b']') | (b'{', b'}')) {
                    return None;
                }
                if stack.is_empty() {
                    return Some(j + 1);
                }
            }
            b'#' | b'<' | b'>' => return None,
            _ => {}
        }
        j += 1;
    }
    None
}

/// The `#…#` as the message shows it: each argument list that holds
/// anything as `(…)`. `text` is one [`call_end`] accepted, so its brackets
/// balance and its quotes close.
fn shown(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'(' || b[i] == b'[' {
            let end = group_end(b, i, b.len()).unwrap_or(b.len());
            if b[i] == b'(' && end - i > 2 {
                out.push_str("(…)");
            } else {
                out.push_str(&text[i..end]);
            }
            i = end;
        } else {
            let next = text[i..].find(['(', '[']).map_or(b.len(), |n| i + n);
            out.push_str(&text[i..next]);
            i = next;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_read_to_their_closing_hash() {
        for (src, end) in [
            ("#f()#", Some(5)),
            ("#f(\"x\")# y", Some(8)),
            ("#a.b(1).c[2]()#", Some(15)),
            ("#f(')#')#", Some(9)),
            ("#f('it''s')#", Some(12)),
            ("#f(g(1), [2])#", Some(14)),
            ("#name#", None),
            ("#a.b#", None),
            ("#fff; color: #000", None),
            ("#f(#x#)#", None),
            ("#f(1)", None),
            ("#f(1)x#", None),
            ("#1(2)#", None),
            ("#f(]#", None),
        ] {
            assert_eq!(call_end(src, 0), end, "{src}");
        }
    }

    #[test]
    fn messages_elide_arguments() {
        assert_eq!(shown("#showDetail(\"x\")#"), "#showDetail(…)#");
        assert_eq!(shown("#now()#"), "#now()#");
        assert_eq!(shown("#a.b(1).c[x]()#"), "#a.b(…).c[x]()#");
    }
}
