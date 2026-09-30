//! Indented `--tree` dump of a [`Tree`], one node per line.
//!
//! ```text
//! root script
//!   statement expression terminator=";"
//!     call-expr
//!       ident.call "foo"
//!       call udf open="(" close=")"
//!         item sep=","
//!           lit.number "1"
//!           trailing:
//!             ws " "
//!             line-comment open="//"
//!               comment " one"
//!             nl "\n"
//! ```

use std::fmt::Write;

use crate::json::JsonOpts;
use crate::tree::{Element, ElementKind, Node, Token, TokenKind, Tree};

/// Render the tree; `opts` adds spans (`@start..end`).
pub fn format_tree(tree: &Tree, opts: JsonOpts) -> String {
    let mut out = String::new();
    Dumper { tree, opts }.element(&tree.root, 0, &mut out);
    out
}

struct Dumper<'a> {
    tree: &'a Tree,
    opts: JsonOpts,
}

impl Dumper<'_> {
    fn pad(depth: usize, out: &mut String) {
        out.extend(std::iter::repeat_n("  ", depth));
    }

    fn element(&self, e: &Element, depth: usize, out: &mut String) {
        Self::pad(depth, out);
        out.push_str(e.kind.name());
        match &e.kind {
            ElementKind::Root(mode) => {
                let _ = write!(out, " {}", mode.as_str());
            }
            ElementKind::Struct { ordered: true } => out.push_str(" ordered"),
            ElementKind::Pattern { array: true } => out.push_str(" array"),
            ElementKind::Block(k) => {
                let _ = write!(out, " {}", k.name());
            }
            ElementKind::ScriptTag { acf: true } => out.push_str(" acf"),
            ElementKind::String { quote, in_tag } => {
                let _ = write!(out, " {}", quote.name());
                if *in_tag {
                    out.push_str(" in_tag");
                }
            }
            ElementKind::CfTag(shape, _) | ElementKind::HtmlTag(shape) => {
                let _ = write!(out, " {}", shape.name());
                if let ElementKind::CfTag(_, kind) = e.kind {
                    let _ = write!(out, " {}", kind.name());
                }
                if let Some(name) = self.tree.tag_name(e) {
                    let _ = write!(out, " name={name}");
                }
            }
            ElementKind::TagBody { cf } => {
                out.push_str(if *cf { " cf" } else { " html" });
                if let Some(name) = self.tree.tag_name(e) {
                    let _ = write!(out, " name={name}");
                }
            }
            ElementKind::Island(i) => {
                let _ = write!(out, " lang={} site={}", i.lang.name(), i.site.name());
                if let Some(t) = &i.script_type {
                    let _ = write!(out, " type={t:?}");
                }
                if e.is_pure_island() {
                    out.push_str(" pure");
                }
            }
            ElementKind::Statement(kind) => {
                let _ = write!(out, " {}", kind.name());
            }
            ElementKind::Recovered(reason) => {
                let _ = write!(out, " {}", reason.name());
            }
            ElementKind::Function { arrow: true } => out.push_str(" arrow"),
            ElementKind::Binary { prec } => {
                let _ = write!(out, " {}", prec.name());
            }
            ElementKind::Unary { postfix: true } => out.push_str(" postfix"),
            ElementKind::Segment(kind) => {
                let _ = write!(out, " {}", kind.name());
                if let Some(segment) = e.as_segment() {
                    if segment.is_safe() {
                        out.push_str(" safe");
                    }
                    if segment.is_static() {
                        out.push_str(" static");
                    }
                }
            }
            _ => {}
        }
        if let Some(t) = &e.open {
            let _ = write!(out, " open={:?}", self.tree.text(t));
        }
        if let Some(t) = &e.close {
            let key = if e.kind.is_statement() {
                "terminator"
            } else {
                "close"
            };
            let _ = write!(out, " {key}={:?}", self.tree.text(t));
        }
        self.extras(&e.span, out);
        out.push('\n');
        for n in &e.children {
            self.node(n, depth + 1, out);
        }
        for item in &e.items {
            Self::pad(depth + 1, out);
            out.push_str("item");
            if let Some(sep) = &item.separator {
                let _ = write!(out, " sep={:?}", self.tree.text(sep));
            }
            out.push('\n');
            self.attached("leading", &item.leading, depth + 2, out);
            for n in &item.children {
                self.node(n, depth + 2, out);
            }
            self.attached("trailing", &item.trailing, depth + 2, out);
        }
    }

    /// An item's `leading` / `trailing` nodes under a `leading:` / `trailing:`
    /// line, one level deeper than the item's children.
    fn attached(&self, label: &str, nodes: &[Node], depth: usize, out: &mut String) {
        if nodes.is_empty() {
            return;
        }
        Self::pad(depth, out);
        let _ = writeln!(out, "{label}:");
        for n in nodes {
            self.node(n, depth + 1, out);
        }
    }

    fn node(&self, n: &Node, depth: usize, out: &mut String) {
        match n {
            Node::Token(t) => self.token(t, depth, out),
            Node::Element(e) => self.element(e, depth, out),
        }
    }

    fn token(&self, t: &Token, depth: usize, out: &mut String) {
        Self::pad(depth, out);
        let text = if t.kind == TokenKind::Ignore {
            self.tree.verbatim(t.span.clone())
        } else {
            self.tree.text(t).into()
        };
        let _ = write!(out, "{} {:?}", t.kind.name(), text);
        self.extras(&t.span, out);
        out.push('\n');
    }

    fn extras(&self, span: &std::ops::Range<u32>, out: &mut String) {
        if self.opts.spans {
            let _ = write!(out, " @{}..{}", span.start, span.end);
        }
    }
}
