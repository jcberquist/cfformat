//! Tag pairing: `CfTag(Open) … CfTag(Close)` and `HtmlTag(Open) …
//! HtmlTag(Close)` with equal names become a
//! [`TagBody`](ElementKind::TagBody) holding `[open tag, …body…, close tag]`.
//!
//! CF tags pair first and apart from HTML tags: the engines process
//! CF tags and HTML is text to them. Each node list is walked twice — the CF
//! layer, where only CF tags pair and every HTML tag is content, then the
//! HTML layer over the result and over the body of every CF body it made,
//! where CF bodies are content. A CF body therefore survives any HTML tag
//! that crosses it (`<html>…<cfoutput>…</html></cfoutput>`: `<html>` and
//! `</html>` stay bare), and a list with no crossing pairs as one walk did.
//!
//! The node list is walked **backwards**. A closing tag opens a pending body;
//! an opening tag whose name equals the innermost pending body's name (ASCII
//! case-insensitive, as CFML compares tag names) closes it; every other node,
//! including unmatched opening tags (`<br>`, `<cfelse>`, `<cfabort>`), is
//! content of the innermost pending body. No tag list is needed.
//!
//! A closing tag still pending when the walk reaches the start of the list — a
//! view partial closing a tag opened in another file, or a CF / HTML
//! mis-nesting the CFML engines render anyway — is content, like an unmatched
//! opening tag, never an error: the innermost such tag is left bare where it
//! is and the nodes of its body are walked again one level out, so an opening
//! tag they hold can still pair with an enclosing closing tag
//! (`<div></cfoutput></div>` is a `<div>` body holding a bare `</cfoutput>`).
//! When no opening tag among them can pair there, they move out unwalked
//! (`released_inertly`): walking them again made a run of mismatched closing
//! tags quadratic.
//!
//! An opening tag whose attribute makes it bodyless ([`BODYLESS_WHEN`]:
//! `<cftransaction action="commit">` inside a transaction body) is content,
//! never a pairing candidate, so it cannot take the enclosing tag's closer.
//! Written with a closer of its own (`<x action="commit"></x>`) that closer
//! pairs with an enclosing opener if one is pending, and is otherwise a
//! stray closer, content like any other: no engine agrees on that form, so
//! the printer prints what it got.
//!
//! Pairing nests at most [`MAX_TAG_DEPTH`] bodies deep in one list, both
//! layers together: the post-passes and the printer recurse per element, and
//! 2,000 nested `<cfif>`s overflowed a 2 MB stack in `attach_item_comments`.

use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};

use crate::tree::{tag_name_in, Element, ElementKind, Ident, Literal, Node, TagShape, TokenKind};

/// `(tag, attribute, values)`: an opening tag with one of these literal
/// values is bodyless. Checked against the Lucee reference (`transaction`,
/// `thread`) and cfdocs.org's ACF pages: `cftransaction` `commit` /
/// `rollback` / `setsavepoint` sit inside a `begin` block; `cfthread`
/// `join` / `sleep` / `terminate` have no body in either engine, and Lucee's
/// `interrupt` neither. Names and values match ASCII case-insensitively,
/// quoted or bare; a value holding `#…#` is not in the table.
pub const BODYLESS_WHEN: &[(&str, &str, &[&str])] = &[
    (
        "cftransaction",
        "action",
        &["commit", "rollback", "setsavepoint"],
    ),
    (
        "cfthread",
        "action",
        &["join", "sleep", "terminate", "interrupt"],
    ),
];

/// Whether `open` (named `name`) is a bodyless form in [`BODYLESS_WHEN`].
/// The attributes are read flat: a key-value's nodes in place of it.
pub(crate) fn is_bodyless(source: &str, open: &Element, name: &str) -> bool {
    let Some((_, attribute, values)) = BODYLESS_WHEN
        .iter()
        .find(|(tag, _, _)| tag.eq_ignore_ascii_case(name))
    else {
        return false;
    };
    let text = |span: &std::ops::Range<u32>| &source[span.start as usize..span.end as usize];
    let mut sig = open
        .children
        .iter()
        .flat_map(|n| match n {
            Node::Element(e) if e.kind == ElementKind::KeyValue => e.children.iter(),
            n => std::slice::from_ref(n).iter(),
        })
        .filter(|n| !n.is_trivia());
    while let Some(node) = sig.next() {
        let Node::Token(t) = node else { continue };
        if t.kind != TokenKind::Ident(Ident::AttributeName)
            || !text(&t.span).eq_ignore_ascii_case(attribute)
        {
            continue;
        }
        let _equals = sig.next();
        let value = match sig.next() {
            Some(Node::Token(v)) if v.kind == TokenKind::Literal(Literal::Unquoted) => {
                text(&v.span)
            }
            Some(Node::Element(e)) if matches!(e.kind, ElementKind::String { .. }) => {
                match e.children.as_slice() {
                    [Node::Token(v)] if v.kind == TokenKind::Literal(Literal::StringText) => {
                        text(&v.span)
                    }
                    _ => return false,
                }
            }
            _ => return false,
        };
        return values.iter().any(|v| v.eq_ignore_ascii_case(value));
    }
    false
}

/// Pair tags in every node list of `el` and below: the root, tag islands,
/// islands (`<cfif>` inside `<cfquery>` SQL), `<cfscript>` statements, and the
/// bodies created here. Pairs never cross an element boundary.
pub fn pair_tags(el: &mut Element, source: &str) {
    // The CFML walks one flat token list; here every `children` vec is its own
    // list, visited bottom-up so a body is built from finished nodes.
    for node in el.children.iter_mut() {
        if let Node::Element(child) = node {
            pair_tags(child, source);
        }
    }
    for item in el.items.iter_mut() {
        for node in item.nodes_mut() {
            if let Node::Element(child) = node {
                pair_tags(child, source);
            }
        }
        if item.children.iter().any(|n| is_close_tag(n, Layer::Any)) {
            item.children = pair_list(std::mem::take(&mut item.children), source);
        }
    }
    if el.children.iter().any(|n| is_close_tag(n, Layer::Any)) {
        el.children = pair_list(std::mem::take(&mut el.children), source);
    }
}

/// Which tags a walk pairs: the other kind is content.
#[derive(Clone, Copy, PartialEq)]
enum Layer {
    Cf,
    Html,
    /// Either (only asked of [`is_close_tag`]).
    Any,
}

/// The shape of `node` when it is a tag of `layer`.
fn tag_shape(node: &Node, layer: Layer) -> Option<TagShape> {
    let Node::Element(e) = node else {
        return None;
    };
    match (&e.kind, layer) {
        (ElementKind::CfTag(shape, _), Layer::Cf | Layer::Any)
        | (ElementKind::HtmlTag(shape), Layer::Html | Layer::Any) => Some(*shape),
        _ => None,
    }
}

fn is_close_tag(node: &Node, layer: Layer) -> bool {
    tag_shape(node, layer) == Some(TagShape::Close)
}

/// Both layers over one node list: CF tags, then HTML tags.
fn pair_list(nodes: Vec<Node>, source: &str) -> Vec<Node> {
    let nodes = if nodes.iter().any(|n| is_close_tag(n, Layer::Cf)) {
        pair_nodes(nodes, source, Layer::Cf, 0)
    } else {
        nodes
    };
    html_layer(nodes, source, 0)
}

/// The HTML layer over `nodes`, a list `depth` bodies deep in its own list,
/// then over the body of every CF body in the result, at that body's depth
/// (HTML bodies around it counted): the CF layer built them all, nested, and
/// an HTML body it holds cannot close outside it. A CF body at
/// [`MAX_TAG_DEPTH`] or deeper — only an HTML body around it can put it
/// there — is bare tags, as a pair past the bound is in either layer.
fn html_layer(nodes: Vec<Node>, source: &str, depth: usize) -> Vec<Node> {
    let mut nodes = if depth < MAX_TAG_DEPTH && nodes.iter().any(|n| is_close_tag(n, Layer::Html)) {
        pair_nodes(nodes, source, Layer::Html, depth)
    } else {
        nodes
    };
    cf_bodies_within(&mut nodes, source, depth);
    nodes
}

/// [`html_layer`] on the body of every CF body in `nodes` (a list `depth`
/// bodies deep) and inside the HTML bodies there.
fn cf_bodies_within(nodes: &mut Vec<Node>, source: &str, depth: usize) {
    let mut i = 0;
    while i < nodes.len() {
        let Node::Element(e) = &mut nodes[i] else {
            i += 1;
            continue;
        };
        match e.kind {
            ElementKind::TagBody { cf: false } => {
                cf_bodies_within(&mut e.children, source, depth + 1);
            }
            ElementKind::TagBody { cf: true } if depth >= MAX_TAG_DEPTH => {
                // Its tags go bare in place; a CF body among them is met next.
                let children = std::mem::take(&mut e.children);
                nodes.splice(i..=i, children);
                continue;
            }
            ElementKind::TagBody { cf: true } => {
                e.children = html_layer(std::mem::take(&mut e.children), source, depth + 1);
            }
            _ => {}
        }
        i += 1;
    }
}

/// How many pending bodies one list may nest, CF and HTML together. A
/// closing tag met deeper is content, and so is the opening tag that would
/// have closed it: the outer bodies pair as before, everything below is bare
/// tags in the innermost one.
/// 5,000 nested tags overflowed a 2 MB stack in the post-passes; this leaves
/// room for a script fragment's own `MAX_DEPTH` below.
pub const MAX_TAG_DEPTH: usize = 1000;

/// A tag name borrowed from the source, compared and hashed ASCII
/// case-insensitively, as CFML compares tag names.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Caseless<'s>(pub(crate) &'s str);

impl PartialEq for Caseless<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(other.0)
    }
}

impl Eq for Caseless<'_> {}

impl Hash for Caseless<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for b in self.0.bytes() {
            state.write_u8(b.to_ascii_lowercase());
        }
        // The end, as `str` hashes it, so `ab` + `c` never collides with
        // `a` + `bc` in a composite key.
        state.write_u8(0xff);
    }
}

/// What a pending closing tag has collected since it was met: `items` in
/// walk order (source order reversed), and the names of the opening tags
/// among them that a walk of them again one level out could pair. Shared
/// with the file-wide walk of [`recover_tags`](super::recover_tags), which
/// collects tag indices.
pub(crate) struct Walked<'s, T> {
    pub(crate) items: VecDeque<T>,
    pub(crate) openers: HashSet<Caseless<'s>>,
}

impl<'s, T> Walked<'s, T> {
    pub(crate) fn new() -> Self {
        Walked {
            items: VecDeque::new(),
            openers: HashSet::new(),
        }
    }

    /// `item`, walked last; `opener` is its name when it is an opening tag
    /// that could pair.
    pub(crate) fn push(&mut self, item: T, opener: Option<Caseless<'s>>) {
        self.openers.extend(opener);
        self.items.push_back(item);
    }

    /// Append `other` (walked after these): what a released body's
    /// collection becomes when a walk of it again would pair nothing. The
    /// smaller side moves, so an item moves O(log n) times however many
    /// bodies release it in turn; appending to a parent that already held
    /// the released closing tag moved every item once per release.
    pub(crate) fn absorb(&mut self, mut other: Walked<'s, T>) {
        if other.items.len() > self.items.len() {
            while let Some(item) = self.items.pop_back() {
                other.items.push_front(item);
            }
            self.items = other.items;
        } else {
            self.items.extend(other.items);
        }
        if other.openers.len() > self.openers.len() {
            std::mem::swap(&mut self.openers, &mut other.openers);
        }
        self.openers.extend(other.openers);
    }
}

/// A closing tag waiting for its opening tag.
struct Pending<'s> {
    /// Its name: compared as CFML `==` compares.
    name: Caseless<'s>,
    close: Box<Element>,
    /// The nodes walked since, with the name of every opening tag of the
    /// layer among them that was content but a bodyless form (one that went
    /// past the bound is named even so: a name too many costs only a walk)…
    walked: Walked<'s, Node>,
    /// …and the closing tags among them left as content past
    /// [`MAX_TAG_DEPTH`], by span start and name: the only ones not bare.
    /// Together what a walk of them again could act on (see
    /// [`released_inertly`]).
    deep: Vec<(u32, Caseless<'s>)>,
}

/// What a node pushed as content adds to its pending body's summary.
enum Note<'s> {
    Plain,
    /// An opening tag of the layer, by name.
    Opener(Caseless<'s>),
    /// A closing tag left as content past the depth bound.
    Deep(u32, Caseless<'s>),
}

/// `node` as content of the innermost pending body, or of `out`.
fn push<'s>(stack: &mut [Pending<'s>], out: &mut Vec<Node>, node: Node, note: Note<'s>) {
    let Some(pending) = stack.last_mut() else {
        out.push(node);
        return;
    };
    let opener = match note {
        Note::Plain => None,
        Note::Opener(name) => Some(name),
        Note::Deep(start, name) => {
            pending.deep.push((start, name));
            None
        }
    };
    pending.walked.push(node, opener);
}

/// Whether walking the body of the released `pending` again, one level
/// out, would only move its nodes into `parent` (the next pending body, or
/// `out` when there is none) as content, in order, and leave bare each
/// closing tag among them that went past the bound; then the release does
/// just that. The walk made `'<cfif a>' × n + '</cfoutput>' × n` quadratic:
/// each released body walked every node released into it again, and each
/// closing tag past the bound, met afresh, took the level freed and was
/// released in turn.
///
/// Walked again, a node acts only as an opening tag that pairs or as a
/// closing tag past the bound; the rest are content wherever they go (bare
/// closing tags, bodyless forms, nodes of the other layer). Up to the first
/// such closing tag the innermost body is `parent`. That closing tag takes
/// the level `pending` held, the bound's (only there does a closing tag go
/// past it), so the ones after it go past the bound again, and once it is
/// released the rest is walked under `parent` again. So when no opening tag
/// among the nodes has the name of `parent` or of one of those closing
/// tags, nothing pairs. A name deeper in the stack needs no test: under
/// `parent` its opening tag is content too, and is tested when `parent` is
/// released. `depth` is the level `parent` holds, one less than `pending`'s.
fn released_inertly(pending: &Pending, parent: Option<&Pending>, depth: usize) -> bool {
    debug_assert!(pending.deep.is_empty() || depth + 1 == MAX_TAG_DEPTH);
    let openers = &pending.walked.openers;
    !parent.is_some_and(|p| openers.contains(&p.name))
        && !pending.deep.iter().any(|(_, name)| openers.contains(name))
}

/// One layer's backwards walk over `nodes`, a list whose bodies sit `start`
/// bodies deep: only tags of `layer` open or close a body.
fn pair_nodes(nodes: Vec<Node>, source: &str, layer: Layer, start: usize) -> Vec<Node> {
    // The bottom frame is `out`: what no pending body holds goes there.
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut stack: Vec<Pending> = Vec::new();
    // Nodes still to walk, the next one last: the list itself, then the
    // bodies of closing tags that never paired.
    let mut input = nodes;
    // Closing tags left bare (by span start): walked again, they are content.
    let mut bare: HashSet<u32> = HashSet::new();
    // Names of the closing tags met past `MAX_TAG_DEPTH`, innermost last:
    // content, as is the opening tag that matches the innermost one.
    let mut deep: Vec<Caseless> = Vec::new();

    loop {
        let Some(node) = input.pop() else {
            // A closing tag still pending at the start of the list is content
            // (the CFML threw `Unbalanced closing tag found`). The innermost
            // one stays bare where it is, and its body — the nodes walked
            // since, in walk order — is walked again one level out, where an
            // opening tag in it may pair with an enclosing closing tag.
            let Some(pending) = stack.pop() else {
                break;
            };
            bare.insert(pending.close.span.start);
            // Closing tags left as content for depth are in the nodes walked
            // again below, and are met afresh.
            deep.clear();
            let inert = released_inertly(&pending, stack.last(), stack.len() + start);
            let Pending {
                close,
                walked,
                deep: past_bound,
                ..
            } = pending;
            push(&mut stack, &mut out, Node::Element(close), Note::Plain);
            if inert {
                bare.extend(past_bound.iter().map(|&(at, _)| at));
                match stack.last_mut() {
                    Some(parent) => parent.walked.absorb(walked),
                    None => out.extend(walked.items),
                }
            } else {
                input.extend(walked.items.into_iter().rev());
            }
            continue;
        };
        let Some(shape) = tag_shape(&node, layer) else {
            push(&mut stack, &mut out, node, Note::Plain);
            continue;
        };
        let Node::Element(tag) = node else {
            unreachable!("a tag is an element");
        };
        let name = tag_name_in(source, &tag).map(Caseless);
        match (shape, name) {
            // A closing tag: open a pending body.
            (TagShape::Close, name) if !bare.contains(&tag.span.start) => {
                let name = name.unwrap_or_default();
                if stack.len() + start >= MAX_TAG_DEPTH {
                    deep.push(name);
                    let note = Note::Deep(tag.span.start, name);
                    push(&mut stack, &mut out, Node::Element(tag), note);
                    continue;
                }
                stack.push(Pending {
                    name,
                    close: tag,
                    walked: Walked::new(),
                    deep: Vec::new(),
                });
            }
            // Too deep: the opening tag of a closing tag left as content.
            (TagShape::Open, Some(name)) if deep.last() == Some(&name) => {
                deep.pop();
                push(&mut stack, &mut out, Node::Element(tag), Note::Opener(name));
            }
            // A bodyless form: content, never a pairing candidate.
            (TagShape::Open, Some(name)) if is_bodyless(source, &tag, name.0) => {
                push(&mut stack, &mut out, Node::Element(tag), Note::Plain);
            }
            // `cftag` / `htmltag` matching the innermost pending body: close it.
            (TagShape::Open, Some(name)) if stack.last().is_some_and(|p| p.name == name) => {
                let pending = stack.pop().unwrap();
                push(&mut stack, &mut out, tag_body(tag, pending), Note::Plain);
            }
            // An unmatched opening tag.
            (TagShape::Open, Some(name)) => {
                push(&mut stack, &mut out, Node::Element(tag), Note::Opener(name));
            }
            // Anything else: a bare closing tag, a nameless or self-closed tag.
            _ => push(&mut stack, &mut out, Node::Element(tag), Note::Plain),
        }
    }

    out.reverse();
    out
}

fn tag_body(open: Box<Element>, pending: Pending) -> Node {
    let Pending { close, walked, .. } = pending;
    let cf = matches!(close.kind, ElementKind::CfTag(..));
    let span = open.span.start..close.span.end;
    let mut children = Vec::with_capacity(walked.items.len() + 2);
    children.push(Node::Element(open));
    children.extend(walked.items.into_iter().rev());
    children.push(Node::Element(close));
    Node::Element(Box::new(Element {
        kind: ElementKind::TagBody { cf },
        open: None,
        close: None,
        children,
        items: Vec::new(),
        span,
    }))
}

#[cfg(test)]
mod tests {
    use super::{pair_list, pair_nodes, Layer, MAX_TAG_DEPTH};
    use crate::tree::{ElementKind, Node};
    use crate::{parse_source, Mode};

    /// `nodes` as their source text, a tag body in brackets.
    fn outline(nodes: &[Node], src: &str) -> String {
        let text = |n: &Node| src[n.span().start as usize..n.span().end as usize].to_string();
        nodes
            .iter()
            .map(|n| match n {
                Node::Element(e) if matches!(e.kind, ElementKind::TagBody { .. }) => {
                    format!("[{}]", outline(&e.children, src))
                }
                n => text(n),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Both layers over `src`'s top-level list, as [`super::pair_tags`]
    /// pairs it.
    fn paired(src: &str) -> String {
        outline(&pair_list(crate::tags::parse(src).children, src), src)
    }

    /// One layer over `src`'s top-level list as if it sat `start` bodies
    /// deep: small inputs reach [`MAX_TAG_DEPTH`].
    fn paired_at(src: &str, layer: Layer, start: usize) -> String {
        outline(
            &pair_nodes(crate::tags::parse(src).children, src, layer, start),
            src,
        )
    }

    #[test]
    fn a_run_of_mismatched_closers_is_bare_tags() {
        // Each released body holds the openers and is released in turn:
        // nothing pairs, and nothing is walked again (the walk that made
        // `'<cfif a>' × n + '</cfoutput>' × n` quadratic).
        let all_bare = |src: &str| {
            let tags: Vec<&str> = src.split_inclusive('>').collect();
            assert_eq!(paired(src), tags.join(" "), "{src}");
        };
        all_bare("<cfif a><cfif a><cfif a></cfoutput></cfoutput></cfoutput>");
        all_bare("<div><div><div></span></span></span>");
        assert_eq!(
            paired("<cfoutput><div><div></span></span></cfoutput>"),
            "[<cfoutput> <div> <div> </span> </span> </cfoutput>]"
        );
        // At the bound: the first closer takes the last level, the others
        // are content past it, each met afresh when the one before is
        // released, and still nothing pairs.
        let src = "<cfif a><cfif a><cfif a></cfoutput></cfoutput></cfoutput>";
        for start in [MAX_TAG_DEPTH - 1, MAX_TAG_DEPTH - 2] {
            let tags: Vec<&str> = src.split_inclusive('>').collect();
            assert_eq!(paired_at(src, Layer::Cf, start), tags.join(" "));
        }
    }

    #[test]
    fn a_released_body_is_walked_again_when_it_can_pair() {
        // An opener in the released body has the name of the closer below.
        assert_eq!(paired("<div></span></div>"), "[<div> </span> </div>]");
        assert_eq!(
            paired("<div></cfoutput></div>"),
            "[<div> </cfoutput> </div>]"
        );
        assert_eq!(
            paired("<cfif a><cfelse></cfoutput></cfif>"),
            "[<cfif a> <cfelse> </cfoutput> </cfif>]"
        );
        // Two levels out: released twice, then it pairs.
        assert_eq!(paired("<b></i></u></b>"), "[<b> </i> </u> </b>]");
        // At the bound: the second `</cfoutput>` is content past it and so
        // is the `<cfoutput>` that would close it; once the first is
        // released, the second takes its level and the opener pairs with it.
        assert_eq!(
            paired_at(
                "<cfoutput><cfif a></cfoutput></cfoutput>",
                Layer::Cf,
                MAX_TAG_DEPTH - 1
            ),
            "[<cfoutput> <cfif a> </cfoutput>] </cfoutput>"
        );
    }

    /// How deep the `TagBody` elements of a tree nest.
    fn body_depth(nodes: &[Node]) -> usize {
        nodes
            .iter()
            .filter_map(Node::as_element)
            .map(|e| {
                usize::from(matches!(e.kind, ElementKind::TagBody { .. })) + body_depth(&e.children)
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn pairing_past_the_limit_leaves_bare_tags() {
        // A 2 MB stack, as a test or rayon thread has.
        let deep = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(|| {
                let n = 20_000;
                let src = format!("{}x{}", "<cfif a>".repeat(n), "</cfif>".repeat(n));
                let tree = parse_source(&src, Mode::Tags);
                assert_eq!(tree.root.span.end as usize, src.len());
                // `body_depth` recurses too: past the limit it would not.
                let shallow = format!("{}x{}", "<p>".repeat(1200), "</p>".repeat(1200));
                let tree = parse_source(&shallow, Mode::Tags);
                assert_eq!(body_depth(&tree.root.children), super::MAX_TAG_DEPTH);
                // CF and HTML bodies share the bound: each `<div>` body adds a
                // level the CF layer never counted.
                let n = 2500;
                let mixed = format!(
                    "{}x{}",
                    "<cfif a><div>".repeat(n),
                    "</div></cfif>".repeat(n)
                );
                let tree = parse_source(&mixed, Mode::Tags);
                assert_eq!(body_depth(&tree.root.children), super::MAX_TAG_DEPTH);
            })
            .unwrap();
        deep.join().unwrap();
        // Inside the limit, a mis-nesting still pairs the outer tags.
        let tree = parse_source("<b><b><i></b></b>", Mode::Tags);
        assert_eq!(body_depth(&tree.root.children), 2);
    }

    #[test]
    fn bodyless_forms_are_content() {
        let bodies = |src: &str| {
            let tree = parse_source(src, Mode::Tags);
            let mut names = Vec::new();
            fn walk(nodes: &[Node], src: &str, out: &mut Vec<String>) {
                for e in nodes.iter().filter_map(Node::as_element) {
                    if matches!(e.kind, ElementKind::TagBody { .. }) {
                        let open = e.open_tag().unwrap();
                        out.push(src[open.span.start as usize..open.span.end as usize].to_string());
                    }
                    walk(&e.children, src, out);
                }
            }
            walk(&tree.root.children, src, &mut names);
            names
        };
        // The commit does not take the outer tag's closer.
        assert_eq!(
            bodies("<cftransaction><cftransaction action=\"commit\"></cftransaction>"),
            ["<cftransaction>"]
        );
        // Case-insensitive, quoted or bare; a dynamic value pairs as before.
        assert_eq!(
            bodies("<CFTHREAD action='run'><cfthread ACTION=Sleep duration=1></cfthread>"),
            ["<CFTHREAD action='run'>"]
        );
        assert_eq!(
            bodies("<cftransaction action=\"#a#\"></cftransaction>"),
            ["<cftransaction action=\"#a#\">"]
        );
        // With a closer of its own and nothing around it, both stay bare.
        assert!(bodies("<cftransaction action=\"rollback\"></cftransaction>").is_empty());
    }
}
