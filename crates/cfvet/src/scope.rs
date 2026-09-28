//! Function units and the walk over one unit's own code, shared by every
//! rule that reasons about a function's scope.
//!
//! A unit is a script `function` (declaration, expression or arrow) or a
//! `<cffunction>` body. Code outside every unit is not a unit's: a
//! component's pseudo-constructor or a template writes to `variables` on
//! purpose. A `thread` body runs with its own scopes, so it and everything in
//! it is outside every unit; a region the parser recovered from, or one left
//! verbatim by `cfformat-ignore`, holds no tree to reason about.

use std::ops::Range;

use cfparse::{CfKind, Element, ElementKind, Node, Tree};

/// One function body.
pub(crate) struct Unit<'a> {
    /// The `Function` element or the `<cffunction>` `TagBody`.
    pub el: &'a Element,
    /// The index in [`units`]' list of the unit this one is nested in.
    pub parent: Option<usize>,
    /// The function's name, `(closure)` when it has none.
    pub name: String,
}

/// Every unit of the tree, each after the unit it is nested in.
pub(crate) fn units(tree: &Tree) -> Vec<Unit<'_>> {
    fn find<'a>(
        tree: &'a Tree,
        nodes: impl Iterator<Item = &'a Node>,
        parent: Option<usize>,
        out: &mut Vec<Unit<'a>>,
    ) {
        for node in nodes {
            let Node::Element(el) = node else { continue };
            if skipped(tree, el) {
                continue;
            }
            if is_unit(el) {
                out.push(Unit {
                    el,
                    parent,
                    name: unit_name(tree, el),
                });
                let index = out.len() - 1;
                find(tree, unit_nodes(el).into_iter(), Some(index), out);
            } else {
                find(tree, el.nodes(), parent, out);
            }
        }
    }
    let mut out = Vec::new();
    find(tree, tree.root.nodes(), None, &mut out);
    out
}

/// Calls `f` on every element of `unit`'s own code, outermost first, with
/// the span of the block the element's own tokens run in: not the units
/// nested in it (each is walked on its own) and not what [`units`] skips.
///
/// A block is a run of code that may be skipped while the code around it
/// runs: an `if` / `else if` / `else` branch, a loop body, a `case`, a
/// `catch` body, and in tags the same constructs (`<cfif>` split at its
/// `<cfelseif>` / `<cfelse>`, `<cfcatch>`, `<cfloop>`, a tag with a `query`
/// attribute that loops over the rows). The unit's body is the outermost
/// block. Code that runs whenever the code around it runs belongs to the
/// enclosing block: a `try` body (it always starts, so what it declares
/// before a throw has run by the time a `catch`, the `finally` or the code
/// after the `try` runs), a `do … while` body (it runs at least once), a
/// `finally` body, a `<cfscript>` body, a `lock` or `savecontent` body, the
/// header of a `for` (its initialiser always runs).
pub(crate) fn walk<'a>(
    tree: &'a Tree,
    unit: &Unit<'a>,
    f: &mut dyn FnMut(&'a Element, &Range<u32>),
) {
    let block = unit.el.span.clone();
    match unit.el.kind {
        ElementKind::TagBody { .. } => visit(tree, unit.el, unit.el.body().iter(), &block, f),
        _ => visit(tree, unit.el, unit.el.nodes(), &block, f),
    }
}

/// Walks `nodes`, children of `parent` that run in `block`.
fn visit<'a>(
    tree: &'a Tree,
    parent: &'a Element,
    nodes: impl Iterator<Item = &'a Node>,
    block: &Range<u32>,
    f: &mut dyn FnMut(&'a Element, &Range<u32>),
) {
    for node in nodes {
        let Node::Element(el) = node else { continue };
        if skipped(tree, el) || is_unit(el) {
            continue;
        }
        let own = opens_block(tree, parent, el).unwrap_or_else(|| block.clone());
        f(el, &own);
        if matches!(el.kind, ElementKind::TagBody { .. }) {
            tag_body(tree, el, &own, f);
        } else {
            visit(tree, el, el.nodes(), &own, f);
        }
    }
}

/// The block `el` opens as a child of `parent`, if it opens one: a script
/// branch, loop body, `case` or `catch` body.
fn opens_block(tree: &Tree, parent: &Element, el: &Element) -> Option<Range<u32>> {
    let opens = match parent.kind {
        _ if el.kind == ElementKind::Case => true,
        ElementKind::If
        | ElementKind::ElseIf
        | ElementKind::Else
        | ElementKind::For
        | ElementKind::While
        | ElementKind::Catch => is_body(el),
        // `loop … { }`, `cfloop(…) { }`: the body block is the script tag's
        // sibling in the statement.
        ElementKind::Statement(_) => {
            matches!(el.kind, ElementKind::Block(_))
                && parent.children.iter().any(|n| {
                    n.as_element().is_some_and(|e| {
                        matches!(e.kind, ElementKind::ScriptTag { .. }) && loops(tree, e)
                    })
                })
        }
        _ => false,
    };
    opens.then(|| el.span.clone())
}

/// A script branch or loop body: a block, or a brace-less body, which is
/// the statement alone since an `else` is the `if` element's child too.
fn is_body(el: &Element) -> bool {
    matches!(el.kind, ElementKind::Block(_) | ElementKind::Statement(_))
}

/// A tag body's open tag runs in `block`, the enclosing one; its body runs
/// in a block of its own when the tag branches or loops.
fn tag_body<'a>(
    tree: &'a Tree,
    el: &'a Element,
    block: &Range<u32>,
    f: &mut dyn FnMut(&'a Element, &Range<u32>),
) {
    let Some(inner) = inner(el) else { return };
    visit(tree, el, el.children.iter().take(1), block, f);
    let body = el.body();
    if !tag_is(tree, el, "cfif") {
        // A `<cftry>` body always starts, so it stays in the enclosing
        // block; its `<cfcatch>` children are blocks of their own, and a
        // `<cffinally>` body runs whenever the `<cftry>` does.
        let own_block = loops(tree, el)
            || ["cfcatch", "cfcase", "cfdefaultcase"]
                .iter()
                .any(|n| tag_is(tree, el, n));
        let own = if own_block { inner } else { block.clone() };
        visit(tree, el, body.iter(), &own, f);
        return;
    }
    // The branch tags themselves run in the enclosing block.
    for (nodes, branch) in if_branches(el, inner) {
        let split = body.get(nodes.end);
        visit(tree, el, body[nodes].iter(), &branch, f);
        visit(tree, el, split.into_iter(), block, f);
    }
}

/// The span between a tag body's open and close tags.
fn inner(el: &Element) -> Option<Range<u32>> {
    let open = el.open_tag()?;
    Some(open.span.end..el.close_tag().map_or(el.span.end, |c| c.span.start))
}

/// A `<cfif>` body's branches, split at its `<cfelseif>` / `<cfelse>`
/// children: the range of body nodes in each, and its span, which runs
/// from the end of its tag to the start of the next branch tag or of the
/// close tag. `inner` is the whole body's span.
fn if_branches(el: &Element, inner: Range<u32>) -> Vec<(Range<usize>, Range<u32>)> {
    let body = el.body();
    let mut out = Vec::new();
    let (mut start, mut lo) = (0, inner.start);
    loop {
        let end = body[start..]
            .iter()
            .position(is_split)
            .map_or(body.len(), |i| start + i);
        let hi = body.get(end).map_or(inner.end, |n| n.span().start);
        out.push((start..end, lo..hi));
        let Some(split) = body.get(end) else { break };
        lo = split.span().end;
        start = end + 1;
    }
    out
}

fn is_split(node: &Node) -> bool {
    node.as_element()
        .is_some_and(|e| matches!(e.cf_kind(), Some(CfKind::ElseIf | CfKind::Else)))
}

/// The branches of a conditional that runs exactly one of them, each as
/// the span of the block [`walk`] hands its code: an `if` with an `else`
/// (any `else if` between), a `switch` with a `default`, a `<cfif>` with a
/// `<cfelse>`, a `<cfswitch>` with a `<cfdefaultcase>`. `None` for
/// anything else: an `if` without an `else` or a `switch` without a
/// `default` may run no branch, and a loop may run its body any number of
/// times.
pub(crate) fn branches(tree: &Tree, el: &Element) -> Option<Vec<Range<u32>>> {
    let body_of = |e: &Element| {
        e.children
            .iter()
            .filter_map(Node::as_element)
            .find(|c| is_body(c))
            .map(|c| c.span.clone())
    };
    match el.kind {
        ElementKind::If => {
            let mut out = vec![body_of(el)?];
            let mut has_else = false;
            for child in el.children.iter().filter_map(Node::as_element) {
                match child.kind {
                    ElementKind::ElseIf => out.push(body_of(child)?),
                    ElementKind::Else => {
                        out.push(body_of(child)?);
                        has_else = true;
                    }
                    _ => {}
                }
            }
            has_else.then_some(out)
        }
        ElementKind::Switch => {
            let cases: Vec<&Element> = el
                .children
                .iter()
                .filter_map(Node::as_element)
                .filter(|c| matches!(c.kind, ElementKind::Block(_)))
                .flat_map(|b| b.children.iter().filter_map(Node::as_element))
                .filter(|c| c.kind == ElementKind::Case)
                .collect();
            let is_default = |c: &Element| {
                c.children
                    .iter()
                    .filter_map(Node::as_token)
                    .any(|t| t.kind == cfparse::TokenKind::Keyword(cfparse::Keyword::Default))
            };
            let holds_code = |c: &Element| {
                c.children.iter().any(|n| {
                    n.as_element()
                        .is_some_and(|e| matches!(e.kind, ElementKind::Statement(_)))
                })
            };
            // `case 1: case 2: …`: an empty case falls through to the next
            // one's code, so it is no branch of its own.
            let out = cases
                .iter()
                .enumerate()
                .filter(|&(i, c)| holds_code(c) || i + 1 == cases.len())
                .map(|(_, c)| c.span.clone())
                .collect();
            cases.iter().any(|c| is_default(c)).then_some(out)
        }
        ElementKind::TagBody { cf: true } if tag_is(tree, el, "cfif") => {
            let has_else = el.body().iter().any(|n| {
                n.as_element()
                    .is_some_and(|e| e.cf_kind() == Some(CfKind::Else))
            });
            let branches = if_branches(el, inner(el)?);
            has_else.then(|| branches.into_iter().map(|(_, span)| span).collect())
        }
        ElementKind::TagBody { cf: true } if tag_is(tree, el, "cfswitch") => {
            let cases: Vec<&Element> = el
                .body()
                .iter()
                .filter_map(Node::as_element)
                .filter(|e| tag_is(tree, e, "cfcase") || tag_is(tree, e, "cfdefaultcase"))
                .collect();
            let has_default = cases.iter().any(|e| tag_is(tree, e, "cfdefaultcase"));
            let out = cases.iter().map(|e| inner(e)).collect::<Option<_>>()?;
            has_default.then_some(out)
        }
        _ => None,
    }
}

/// A tag that runs its body once per iteration, possibly never: a loop, or
/// a tag with a `query` attribute (`<cfoutput query>`, `<cfmail query>`).
fn loops(tree: &Tree, el: &Element) -> bool {
    let tag = el.open_tag().unwrap_or(el);
    ["cfloop", "loop", "cfwhile", "while"]
        .iter()
        .any(|n| tag_is(tree, tag, n))
        || crate::rules::attributes(tag)
            .any(|(key, _)| tree.text(key).eq_ignore_ascii_case("query"))
}

/// A unit's own nodes: all of a script function (its parameters are
/// declarations), the body of a `<cffunction>`.
fn unit_nodes(el: &Element) -> Vec<&Node> {
    match el.kind {
        ElementKind::TagBody { .. } => el.body().iter().collect(),
        _ => el.nodes().collect(),
    }
}

fn is_unit(el: &Element) -> bool {
    match el.kind {
        ElementKind::Function { .. } => true,
        ElementKind::TagBody { cf: true } => el.cf_kind() == Some(CfKind::Function),
        _ => false,
    }
}

/// Recovered and verbatim regions, and thread bodies.
fn skipped(tree: &Tree, el: &Element) -> bool {
    match el.kind {
        ElementKind::Recovered(_) | ElementKind::Ignore => true,
        ElementKind::TagBody { cf: true } | ElementKind::CfTag(..) => tag_is(tree, el, "cfthread"),
        // `thread name="t" { … }`: the body block is the script tag's
        // sibling in the statement, so the statement goes whole.
        ElementKind::Statement(_) => el.children.iter().any(|n| {
            n.as_element().is_some_and(|e| {
                matches!(e.kind, ElementKind::ScriptTag { .. })
                    && (tag_is(tree, e, "thread") || tag_is(tree, e, "cfthread"))
            })
        }),
        _ => false,
    }
}

/// Whether `el` is the tag `name`, ASCII case-insensitively ([`Tree::tag_name`]).
pub(crate) fn tag_is(tree: &Tree, el: &Element, name: &str) -> bool {
    tree.tag_name(el)
        .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

fn unit_name(tree: &Tree, el: &Element) -> String {
    use cfparse::{Ident, TokenKind};
    let name = match el.kind {
        ElementKind::TagBody { .. } => el
            .open_tag()
            .and_then(|open| crate::rules::attribute(tree, open, "name"))
            .map(|(text, _)| text.trim().to_owned()),
        _ => el
            .children
            .iter()
            .filter_map(Node::as_element)
            .find(|e| e.kind == ElementKind::FunctionDecl)
            .and_then(|decl| {
                decl.children
                    .iter()
                    .filter_map(Node::as_token)
                    .find(|t| t.kind == TokenKind::Ident(Ident::FunctionName))
            })
            .map(|t| tree.text(t).to_owned()),
    };
    name.filter(|n| !n.is_empty())
        .unwrap_or_else(|| "(closure)".to_owned())
}
