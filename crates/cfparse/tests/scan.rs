//! Corpus scan for tokens and shapes the parse should not produce: `Other` /
//! `Invalid` tokens, terminator-only empty statements, comments left at the
//! tail of a statement, chain or case (the parse hands them back out),
//! closing tags pairing left bare (unbalanced partials, listed per file), operator
//! tokens left outside an expression node and expression statements that
//! start with an identifier followed by more content (both: a run the
//! expression pass did not parse). Every tree must also tile its source
//! (`common::assert_covers_source`). Also prints histograms of statement
//! kinds, chain lengths and binary operand counts.
//!
//! Ignored by default (it needs the sibling `commandbox-cfformat` checkout and
//! prints a report rather than asserting a fixed number):
//!
//! ```text
//! cargo test -p cfparse --test scan -- --ignored --nocapture
//! ```

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cfparse::{Element, ElementKind, Mode, Node, StatementKind, TagShape, Token, TokenKind, Tree};

/// Corpus roots: the vendored fixtures plus `models`, `tests/data` and
/// `testbox` from the sibling checkout when it is present.
fn roots() -> Vec<(String, PathBuf)> {
    let mut out = vec![("fixtures".to_string(), common::fixtures_dir())];
    if let Some(dir) = common::commandbox_dir() {
        for name in ["models", "tests/data", "testbox"] {
            let path = dir.join(name);
            if path.is_dir() {
                out.push((name.to_string(), path));
            }
        }
    }
    out
}

fn cfml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            cfml_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "cfc" || e == "cfm" || e == "cfml")
        {
            out.push(path);
        }
    }
}

fn line_of(source: &str, offset: u32) -> (usize, &str) {
    let offset = offset as usize;
    let start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = source[offset..]
        .find('\n')
        .map_or(source.len(), |i| offset + i);
    (
        source[..offset].matches('\n').count() + 1,
        &source[start..end],
    )
}

fn walk<'a>(el: &'a Element, f: &mut dyn FnMut(&'a Element, &'a Token)) {
    for t in [el.open.as_ref(), el.close.as_ref()].into_iter().flatten() {
        f(el, t);
    }
    let nodes = el
        .children
        .iter()
        .chain(el.items.iter().flat_map(|i| i.nodes()));
    for n in nodes {
        match n {
            Node::Token(t) => f(el, t),
            // A multi-word operator's words sit where the phrase does.
            Node::Element(e) if e.kind == ElementKind::Phrase => {
                for w in e.children.iter().filter_map(Node::as_token) {
                    f(el, w);
                }
            }
            Node::Element(e) => walk(e, f),
        }
    }
    for sep in el.items.iter().filter_map(|i| i.separator.as_ref()) {
        f(el, sep);
    }
}

fn elements<'a>(el: &'a Element, out: &mut Vec<&'a Element>) {
    out.push(el);
    for n in el.nodes() {
        if let Node::Element(e) = n {
            elements(e, out);
        }
    }
}

/// A statement whose only content is its `;` terminator.
fn is_empty_statement(el: &Element) -> bool {
    el.kind.is_statement()
        && el.close.is_some()
        && el.items.is_empty()
        && el
            .children
            .iter()
            .all(|n| matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)))
}

/// An element with no closing delimiter whose last non-whitespace child is a
/// comment: the handback in `build.rs` must have moved it to the parent.
fn ends_with_comment(el: &Element) -> bool {
    matches!(
        el.kind,
        ElementKind::Statement(_)
            | ElementKind::If
            | ElementKind::ElseIf
            | ElementKind::Else
            | ElementKind::For
            | ElementKind::While
            | ElementKind::DoWhile
            | ElementKind::Switch
            | ElementKind::Case
            | ElementKind::Try
            | ElementKind::Catch
            | ElementKind::Finally
    ) && el
        .children
        .iter()
        .rev()
        .find(|n| !matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)))
        .is_some_and(|n| matches!(n, Node::Element(c) if c.kind.is_comment()))
}

/// An element that may hold operator tokens directly.
fn holds_operators(el: &Element) -> bool {
    matches!(
        el.kind,
        ElementKind::Assignment
            | ElementKind::Binary { .. }
            | ElementKind::Ternary
            | ElementKind::Unary { .. }
    )
}

/// An expression statement whose first significant child is a bare
/// identifier token with more significant content after it: the shape of a
/// run the expression pass left flat.
fn is_unrecognised_run(el: &Element) -> bool {
    if el.kind != ElementKind::Statement(StatementKind::Expression) {
        return false;
    }
    let mut parts = el.children.iter().filter(|n| !n.is_trivia());
    matches!(parts.next(), Some(Node::Token(t)) if matches!(t.kind, TokenKind::Ident(_)))
        && parts.next().is_some()
}

fn histogram<K: std::fmt::Display>(label: &str, counts: &BTreeMap<K, usize>) {
    println!("\n== {label} ==");
    for (k, v) in counts {
        println!("  {k:>14}: {v}");
    }
}

/// Offsets of the closing tags pairing left bare: every `CfTag(Close)` /
/// `HtmlTag(Close)` that is not the last child of a `TagBody`.
fn bare_closing_tags(el: &Element) -> Vec<u32> {
    let mut out = Vec::new();
    let last = el.children.len().saturating_sub(1);
    for (i, n) in el.nodes().enumerate() {
        let Node::Element(e) = n else { continue };
        let paired =
            matches!(el.kind, ElementKind::TagBody { .. }) && i == last && i < el.children.len();
        if matches!(
            e.kind,
            ElementKind::CfTag(TagShape::Close, _) | ElementKind::HtmlTag(TagShape::Close)
        ) && !paired
        {
            out.push(e.span.start);
        }
        out.extend(bare_closing_tags(e));
    }
    out
}

fn report(label: &str, hits: &[String]) {
    println!("\n== {label}: {} ==", hits.len());
    for h in hits {
        println!("  {h}");
    }
}

#[test]
#[ignore = "corpus scan; run with --ignored --nocapture"]
fn corpus_has_no_other_or_invalid_tokens() {
    let mut odd = Vec::new();
    let mut invalid = Vec::new();
    let mut empty = Vec::new();
    let mut tails = Vec::new();
    let mut unbalanced = Vec::new();
    let mut loose_operators = Vec::new();
    let mut unrecognised = Vec::new();
    let mut loose_postfix = Vec::new();
    let mut stmt_kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut chains: BTreeMap<usize, usize> = BTreeMap::new();
    let mut binaries: BTreeMap<usize, usize> = BTreeMap::new();
    let mut files = 0usize;

    for (label, root) in roots() {
        let mut paths = Vec::new();
        cfml_files(&root, &mut paths);
        paths.sort();
        for path in paths {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            // Fixture sources encode `Mode::Script` as a leading `//` line.
            let (src, mode) = if label == "fixtures" && raw.starts_with("//") {
                (
                    raw.split_once('\n').map_or("", |(_, r)| r).to_string(),
                    Mode::Script,
                )
            } else {
                (raw, Mode::Auto)
            };
            let name = path
                .strip_prefix(root.parent().unwrap_or(&root))
                .unwrap_or(&path)
                .display()
                .to_string();
            let tree = cfparse::parse_source(&src, mode);
            files += 1;
            common::assert_covers_source(&name, &tree);
            let bare = bare_closing_tags(&tree.root);
            if !bare.is_empty() {
                unbalanced.push(format!(
                    "{name}: {}",
                    bare.iter()
                        .map(|&at| format!("line {}", tree.line_of(at)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            note(&tree, &name, &mut odd, &mut invalid, &mut empty);
            let mut els = Vec::new();
            elements(&tree.root, &mut els);
            walk(&tree.root, &mut |parent, t| {
                if matches!(t.kind, TokenKind::Operator(_)) && !holds_operators(parent) {
                    let (line, text) = line_of(&tree.source, t.span.start);
                    loose_operators.push(format!(
                        "{name}:{line}: {:?} in {} `{}`",
                        tree.text(t),
                        parent.kind.name(),
                        text.trim()
                    ));
                }
            });
            for el in &els {
                match el.kind {
                    ElementKind::Statement(k) => *stmt_kinds.entry(k.name()).or_default() += 1,
                    ElementKind::Chain => {
                        *chains
                            .entry(el.as_chain().unwrap().segments().count())
                            .or_default() += 1
                    }
                    ElementKind::Binary { .. } => {
                        *binaries
                            .entry(el.as_binary().unwrap().operands().count())
                            .or_default() += 1
                    }
                    _ => {}
                }
                for n in el
                    .children
                    .iter()
                    .chain(el.items.iter().flat_map(|i| i.children.iter()))
                {
                    let loose = match n {
                        Node::Token(t) => {
                            matches!(t.kind, TokenKind::Punct(p) if matches!(p, cfparse::Punct::Accessor | cfparse::Punct::SafeAccessor | cfparse::Punct::StaticAccessor))
                                && !matches!(el.kind, ElementKind::Segment(_))
                        }
                        Node::Element(e) => match e.kind {
                            ElementKind::Call => !matches!(
                                el.kind,
                                ElementKind::Segment(_) | ElementKind::CallExpr | ElementKind::New
                            ),
                            ElementKind::Brackets => !matches!(el.kind, ElementKind::Segment(_)),
                            _ => false,
                        },
                    };
                    if loose {
                        let (line, text) = line_of(&tree.source, n.span().start);
                        loose_postfix.push(format!(
                            "{name}:{line}: in {} `{}`",
                            el.kind.name(),
                            text.trim()
                        ));
                    }
                }
                if is_unrecognised_run(el) {
                    let (line, text) = line_of(&tree.source, el.span.start);
                    unrecognised.push(format!("{name}:{line}: `{}`", text.trim()));
                }
            }
            for el in els.into_iter().filter(|e| ends_with_comment(e)) {
                let (line, text) = line_of(&tree.source, el.span.end.saturating_sub(1));
                tails.push(format!(
                    "{name}:{line}: {} ending `{}`",
                    el.kind.name(),
                    text.trim()
                ));
            }
        }
    }

    println!("\nscanned {files} files");
    report("other tokens", &odd);
    report("invalid tokens", &invalid);
    report("terminator-only empty statements", &empty);
    report("statements/chains/cases ending in a comment", &tails);
    report("closing tags left bare (unbalanced partials)", &unbalanced);
    report("operators outside an expression node", &loose_operators);
    report(
        "expression statements starting with an identifier and more",
        &unrecognised,
    );
    report(
        "calls, brackets and accessors outside a chain or call (informational)",
        &loose_postfix,
    );
    histogram("statement kinds", &stmt_kinds);
    histogram("chains by segment count", &chains);
    histogram("binaries by operand count", &binaries);
    assert!(odd.is_empty(), "{} `other` token(s)", odd.len());
    assert!(tails.is_empty(), "{} comment tail(s)", tails.len());
    assert!(
        loose_operators.is_empty(),
        "{} operator(s) outside an expression node",
        loose_operators.len()
    );
    assert!(
        unrecognised.is_empty(),
        "{} unrecognised expression run(s)",
        unrecognised.len()
    );
}

fn note(
    tree: &Tree,
    name: &str,
    odd: &mut Vec<String>,
    invalid: &mut Vec<String>,
    empty: &mut Vec<String>,
) {
    walk(&tree.root, &mut |_, t| {
        let bucket = match t.kind {
            TokenKind::Other => &mut *odd,
            TokenKind::Invalid => &mut *invalid,
            _ => return,
        };
        let (line, text) = line_of(&tree.source, t.span.start);
        bucket.push(format!(
            "{name}:{line}: {:?} in `{}`",
            tree.text(t),
            text.trim()
        ));
    });
    let mut els = Vec::new();
    elements(&tree.root, &mut els);
    for el in els.into_iter().filter(|e| is_empty_statement(e)) {
        let (line, text) = line_of(&tree.source, el.span.start);
        empty.push(format!("{name}:{line}: `{}`", text.trim()));
    }
}
