//! Tree post-passes that run after a front end has built the tree. Each is a
//! function over `&mut Element` that recurses on its own; none keeps state
//! between elements. [`parse_source`](crate::parse_source) runs them in this
//! order: [`tags::pair_tags`], [`comments::attach_item_comments`],
//! [`expressions::build_expressions`] — and around them the two recovery
//! passes: [`recover_tags`] after tag pairing, and [`collect_recoveries`]
//! last. A pass exists only where it needs the whole tree (pairing, the
//! recovery walk, the item-comment move) or precedence (the expression
//! pass); what a front end knows at the node it is reading — a key-value,
//! a declaration fused with its body, a statement's kind — it builds there.

pub mod comments;
pub mod expressions;
pub mod tags;

use std::collections::HashSet;

use tags::Walked;

use crate::tree::{
    tag_name_in, Element, ElementKind, Node, Recovery, RecoveryReason, TagShape, Token, TokenKind,
};

/// Replace `nodes[lo..=hi]` with one element of `kind` holding them as
/// children (the trivia rule: whatever sits between the first and last part
/// moves with them). `lo` and `hi` are significant nodes.
pub(crate) fn wrap(nodes: &mut Vec<Node>, lo: usize, hi: usize, kind: ElementKind) {
    let children: Vec<Node> = nodes.drain(lo..=hi).collect();
    nodes.insert(lo, Node::Element(Box::new(element(kind, children))));
}

/// A new element over `children` (non-empty, in span order).
pub(crate) fn element(kind: ElementKind, children: Vec<Node>) -> Element {
    let first = children.first().expect("element parts");
    let span = first.span().start..children.last().unwrap().span().end;
    Element {
        kind,
        open: None,
        close: None,
        children,
        items: Vec::new(),
        span,
    }
}

/// A [`Recovered`](ElementKind::Recovered) element over `children`
/// (non-empty, in span order).
pub(crate) fn recovered(children: Vec<Node>, reason: RecoveryReason) -> Element {
    element(ElementKind::Recovered(reason), children)
}

/// Wrap `nodes[lo..=hi]` in a [`Recovered`](ElementKind::Recovered) element,
/// leading and trailing trivia left outside: the region is what the parse
/// did not understand, its surrounding whitespace is the list's.
pub(crate) fn wrap_recovered(nodes: &mut Vec<Node>, lo: usize, hi: usize, reason: RecoveryReason) {
    let Some(lo) = (lo..=hi).find(|&i| !nodes[i].is_trivia()) else {
        return;
    };
    let hi = (lo..=hi).rfind(|&i| !nodes[i].is_trivia()).unwrap_or(lo);
    if lo == hi {
        // In place: draining one node and inserting its region shifted the
        // rest of the list twice, quadratic over a list of stray closing
        // tags (`'</cfoutput>' × n`).
        let placeholder = Node::Token(Token {
            span: 0..0,
            kind: TokenKind::Invalid,
        });
        let node = std::mem::replace(&mut nodes[lo], placeholder);
        nodes[lo] = Node::Element(Box::new(recovered(vec![node], reason)));
        return;
    }
    let children: Vec<Node> = nodes.drain(lo..=hi).collect();
    nodes.insert(lo, Node::Element(Box::new(recovered(children, reason))));
}

/// CF tags that are never complete without their closing tag: an opening
/// one the file-wide CF walk of [`recover_tags`] leaves unpaired is
/// [`Unclosed`](RecoveryReason::Unclosed).
/// Conservative: a tag with a bodyless form (`<cftransaction
/// action="commit">`, `<cfthread action="join">`, `tags::BODYLESS_WHEN`)
/// or an optional closer is not listed.
pub const NEEDS_CLOSING_TAG: &[&str] = &[
    "cfcase",
    "cfcatch",
    "cfcomponent",
    "cfdefaultcase",
    "cffinally",
    "cffunction",
    "cfif",
    "cfinterface",
    "cflock",
    "cfloop",
    "cfmail",
    "cfoutput",
    "cfquery",
    "cfsavecontent",
    "cfscript",
    "cfsilent",
    "cfswitch",
    "cftry",
    "cfwhile",
];

/// Tag-mode recoveries, after [`tags::pair_tags`]: a
/// CF closing tag no opening tag took ([`StrayCloser`](RecoveryReason::StrayCloser)),
/// an opening tag of [`NEEDS_CLOSING_TAG`] left unpaired
/// ([`Unclosed`](RecoveryReason::Unclosed)), a CF tag holding an `Invalid`
/// token ([`Unmatched`](RecoveryReason::Unmatched)). "Unpaired" is the
/// file-wide CF walk's (every CF tag of the file in source order, HTML
/// tags and element boundaries ignored), not the tree's: a bare CF tag
/// the walk pairs across an element boundary (a `<cfoutput>` opened in a
/// `<style>` island and closed after it) is not a recovery. The region is
/// the innermost tag body around the problem; outside any, the stray closer
/// or the tag itself, or an unclosed tag and everything after it in its list.
pub fn recover_tags(root: &mut Element, source: &str) {
    let unpaired = unpaired_cf_tags(root, source);
    recover_in(root, false, source, &unpaired);
}

/// A CF tag as the file-wide walk sees it.
struct TagAt<'s> {
    start: u32,
    close: bool,
    name: tags::Caseless<'s>,
}

/// The span starts of the CF tags that pairing the file's CF tags alone
/// leaves unpaired: every `CfTag` element of the tree in source order,
/// whatever list holds it (tag bodies, islands, `<cfscript>` bodies, code
/// fences), HTML tags and element boundaries ignored — the engines' view.
/// The walk is [`tags::pair_tags`]'s: backwards, a closer pairs with the
/// nearest opener of its name, an unpaired closer is released and the tags
/// after it walked again one level out, [`tags::BODYLESS_WHEN`] openers and
/// self-closed tags take no part. As there, the walk again is skipped when
/// it could pair nothing: every one of those tags is an opener that did not
/// pair, so only one with the name of the closer below can, and without one
/// they just move to it (`'<cfif a>' × n + '</cfoutput>' × n` walked each
/// of the `n` openers once per closer).
fn unpaired_cf_tags(root: &Element, source: &str) -> HashSet<u32> {
    let mut tags = Vec::new();
    collect_cf_tags(root, source, &mut tags);
    tags.sort_by_key(|t| t.start);
    let mut paired = vec![false; tags.len()];
    let mut released = vec![false; tags.len()];
    // Indices still to walk, the next one last; pending closers, innermost
    // last, each with the openers walked since (in walk order).
    let mut input: Vec<usize> = (0..tags.len()).collect();
    let mut stack: Vec<(usize, Walked<usize>)> = Vec::new();
    loop {
        let Some(i) = input.pop() else {
            let Some((close, walked)) = stack.pop() else {
                break;
            };
            released[close] = true;
            match stack.last_mut() {
                Some((parent, collected)) if !walked.openers.contains(&tags[*parent].name) => {
                    collected.absorb(walked);
                }
                Some(_) => input.extend(walked.items.into_iter().rev()),
                // Walked again with nothing pending, each stays unpaired.
                None => {}
            }
            continue;
        };
        if tags[i].close {
            if !released[i] {
                stack.push((i, Walked::new()));
            }
        } else if stack
            .last()
            .is_some_and(|(close, _)| tags[*close].name == tags[i].name)
        {
            let (close, _) = stack.pop().unwrap();
            paired[close] = true;
            paired[i] = true;
        } else if let Some((_, walked)) = stack.last_mut() {
            walked.push(i, Some(tags[i].name));
        }
    }
    tags.iter()
        .zip(paired)
        .filter(|(_, paired)| !paired)
        .map(|(t, _)| t.start)
        .collect()
}

/// Every opening and closing `CfTag` at or below `el`, bodyless forms out.
fn collect_cf_tags<'s>(el: &Element, source: &'s str, out: &mut Vec<TagAt<'s>>) {
    let nodes = el.children.iter().chain(el.items.iter().flat_map(|i| {
        i.leading
            .iter()
            .chain(i.children.iter())
            .chain(i.trailing.iter())
    }));
    for node in nodes {
        let Node::Element(child) = node else { continue };
        if let ElementKind::CfTag(shape @ (TagShape::Open | TagShape::Close), _) = child.kind {
            let name = tag_name_in(source, child).unwrap_or_default();
            let close = shape == TagShape::Close;
            if close || !tags::is_bodyless(source, child, name) {
                out.push(TagAt {
                    start: child.span.start,
                    close,
                    name: tags::Caseless(name),
                });
            }
        }
        collect_cf_tags(child, source, out);
    }
}

/// [`recover_tags`] under `el`; `in_body` when a tag body encloses it.
/// Returns the reason a problem inside `el` gives the innermost enclosing
/// tag body, when there is one to take it.
fn recover_in(
    el: &mut Element,
    in_body: bool,
    source: &str,
    unpaired: &HashSet<u32>,
) -> Option<RecoveryReason> {
    if matches!(el.kind, ElementKind::Recovered(_)) {
        return None;
    }
    let body = matches!(el.kind, ElementKind::TagBody { .. });
    let inside = in_body || body;
    let mut reason = None;
    let last = el.children.len().saturating_sub(1);
    let mut i = 0;
    while i < el.children.len() {
        let paired = body && (i == 0 || i == last);
        let problem = match &mut el.children[i] {
            Node::Element(child) => {
                let inner = recover_in(child, inside, source, unpaired);
                match inner {
                    Some(r) if matches!(child.kind, ElementKind::TagBody { .. }) => {
                        wrap_recovered(&mut el.children, i, i, r);
                        None
                    }
                    Some(r) => Some((r, false)),
                    // A body's own tags: only an invalid attribute is wrong.
                    None if paired => tag_problem(child, source, unpaired)
                        .filter(|(r, _)| *r == RecoveryReason::Unmatched),
                    None => tag_problem(child, source, unpaired),
                }
            }
            Node::Token(_) => None,
        };
        if let Some((r, to_end)) = problem {
            if inside {
                reason.get_or_insert(r);
            } else {
                let hi = if to_end { el.children.len() - 1 } else { i };
                wrap_recovered(&mut el.children, i, hi, r);
            }
        }
        i += 1;
    }
    for item in el.items.iter_mut() {
        for node in item.nodes_mut() {
            if let Node::Element(child) = node {
                if let Some(r) = recover_in(child, inside, source, unpaired) {
                    reason.get_or_insert(r);
                }
            }
        }
    }
    reason
}

/// What is wrong with a CF tag seen as a list's own node (not the opening
/// or closing tag of a body), and whether the region runs to the list's end.
/// A bare tag is only wrong when the file-wide walk left it `unpaired`.
fn tag_problem(
    tag: &Element,
    source: &str,
    unpaired: &HashSet<u32>,
) -> Option<(RecoveryReason, bool)> {
    let ElementKind::CfTag(shape, _) = tag.kind else {
        return None;
    };
    let is_invalid = |n: &Node| matches!(n, Node::Token(t) if t.kind == TokenKind::Invalid);
    // An attribute left without a value takes the invalid token after it
    // (`a= /`) into its key-value.
    let invalid = tag.children.iter().any(|n| match n {
        Node::Element(e) if e.kind == ElementKind::KeyValue => e.children.iter().any(is_invalid),
        n => is_invalid(n),
    });
    if invalid {
        return Some((RecoveryReason::Unmatched, false));
    }
    if !unpaired.contains(&tag.span.start) {
        return None;
    }
    match shape {
        TagShape::Close => Some((RecoveryReason::StrayCloser, false)),
        TagShape::Open => {
            let name = tag_name_in(source, tag)?;
            NEEDS_CLOSING_TAG
                .iter()
                .any(|n| n.eq_ignore_ascii_case(name))
                .then_some((RecoveryReason::Unclosed, true))
        }
        TagShape::SelfClosed => None,
    }
}

/// The last pass: a [`Recovered`](ElementKind::Recovered)
/// region inside another is folded into the outer one (its nodes spliced in
/// place), the outer taking [`TooDeep`](RecoveryReason::TooDeep) as its
/// reason when the inner had it; the regions left are the tree's
/// [`Recovery`] list, in source order.
pub fn collect_recoveries(el: &mut Element, out: &mut Vec<Recovery>) {
    for node in el
        .children
        .iter_mut()
        .chain(el.items.iter_mut().flat_map(|i| {
            i.leading
                .iter_mut()
                .chain(i.children.iter_mut())
                .chain(i.trailing.iter_mut())
        }))
    {
        let Node::Element(child) = node else { continue };
        if let ElementKind::Recovered(reason) = child.kind {
            let too_deep = unwrap_recovered(child);
            let reason = if too_deep {
                RecoveryReason::TooDeep
            } else {
                reason
            };
            child.kind = ElementKind::Recovered(reason);
            out.push(Recovery {
                span: child.span.clone(),
                reason,
            });
        } else {
            collect_recoveries(child, out);
        }
    }
    out.sort_by_key(|r| r.span.start);
}

/// Splice every `Recovered` element below `el` into its parent list.
/// Returns whether one of them was [`TooDeep`](RecoveryReason::TooDeep).
fn unwrap_recovered(el: &mut Element) -> bool {
    let mut too_deep = false;
    let lists = std::iter::once(&mut el.children).chain(
        el.items
            .iter_mut()
            .flat_map(|i| [&mut i.leading, &mut i.children, &mut i.trailing]),
    );
    for list in lists {
        let mut i = 0;
        while i < list.len() {
            if let Node::Element(e) = &mut list[i] {
                too_deep |= unwrap_recovered(e);
                if let ElementKind::Recovered(reason) = e.kind {
                    too_deep |= reason == RecoveryReason::TooDeep;
                    let children = std::mem::take(&mut e.children);
                    let n = children.len();
                    list.splice(i..=i, children);
                    i += n;
                    continue;
                }
            }
            i += 1;
        }
    }
    too_deep
}
