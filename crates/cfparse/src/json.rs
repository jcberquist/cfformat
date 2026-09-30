//! Inspect JSON (`cfparse parse --json`, the snapshots).
//!
//! Tokens are `{k, t}` (kind + text); elements are
//! `{kind, …fields, open?, close?|terminator?, children?|items?}`; items are
//! `{leading?, children, trailing?, sep?}`. Byte spans are included only
//! when requested ([`JsonOpts::spans`]). A tree the parse recovered
//! in has `recoveries: [{reason, span}]` beside `root`.

use serde_json::{json, Map, Value};

use crate::tree::{Element, ElementKind, Item, Node, Token, Tree};

#[derive(Debug, Clone, Copy, Default)]
pub struct JsonOpts {
    /// Add `span` (`[start, end]` byte offsets into the normalised source).
    pub spans: bool,
}

/// The whole tree as a JSON value.
pub fn to_json(tree: &Tree, opts: JsonOpts) -> Value {
    let w = Writer { tree, opts };
    let mut v = json!({
        "mode": tree.mode().as_str(),
        "bom": tree.bom,
        "newline": tree.newline.name(),
        "root": w.element(&tree.root),
    });
    // Only when the parse recovered, so an understood tree's JSON has no
    // `recoveries` key.
    if !tree.recoveries.is_empty() {
        let list = tree
            .recoveries
            .iter()
            .map(|r| json!({"reason": r.reason.name(), "span": [r.span.start, r.span.end]}))
            .collect();
        v["recoveries"] = Value::Array(list);
    }
    v
}

/// Pretty JSON where objects and arrays holding only scalars stay on one line
/// (keeps token objects compact).
pub fn to_string_pretty(v: &Value) -> String {
    let mut out = String::new();
    write(v, 0, &mut out);
    out.push('\n');
    out
}

struct Writer<'a> {
    tree: &'a Tree,
    opts: JsonOpts,
}

impl Writer<'_> {
    fn node(&self, n: &Node) -> Value {
        match n {
            Node::Token(t) => self.token(t),
            Node::Element(e) => self.element(e),
        }
    }

    fn token(&self, t: &Token) -> Value {
        let text = if t.kind == crate::TokenKind::Ignore {
            self.tree.verbatim(t.span.clone()).into_owned()
        } else {
            self.tree.text(t).to_string()
        };
        token_value(&text, t, self.opts)
    }

    /// Delimiters are shown as plain text unless spans are requested.
    fn delim(&self, t: &Token) -> Value {
        if self.opts.spans {
            self.token(t)
        } else {
            Value::String(self.tree.text(t).to_string())
        }
    }

    fn element(&self, e: &Element) -> Value {
        let mut m = Map::new();
        m.insert("kind".into(), e.kind.name().into());
        match &e.kind {
            ElementKind::Struct { ordered: true } => {
                m.insert("ordered".into(), true.into());
            }
            ElementKind::Pattern { array: true } => {
                m.insert("array".into(), true.into());
            }
            ElementKind::Block(k) => {
                m.insert("block".into(), k.name().into());
            }
            ElementKind::ScriptTag { acf } => {
                m.insert("acf".into(), (*acf).into());
            }
            ElementKind::String { quote, in_tag } => {
                m.insert("quote".into(), quote.name().into());
                m.insert("in_tag".into(), (*in_tag).into());
            }
            ElementKind::CfTag(shape, _) | ElementKind::HtmlTag(shape) => {
                m.insert("shape".into(), shape.name().into());
                if let ElementKind::CfTag(_, kind) = e.kind {
                    m.insert("cf_kind".into(), kind.name().into());
                }
                if let Some(name) = self.tree.tag_name(e) {
                    m.insert("name".into(), name.into());
                }
            }
            ElementKind::TagBody { cf } => {
                m.insert("cf".into(), (*cf).into());
                if let Some(name) = self.tree.tag_name(e) {
                    m.insert("name".into(), name.into());
                }
            }
            ElementKind::Island(island) => {
                m.insert("lang".into(), island.lang.name().into());
                m.insert("site".into(), island.site.name().into());
                if let Some(t) = &island.script_type {
                    m.insert("script_type".into(), t.as_str().into());
                }
                m.insert("pure".into(), e.is_pure_island().into());
            }
            ElementKind::Statement(kind) => {
                m.insert("stmt".into(), kind.name().into());
            }
            ElementKind::Recovered(reason) => {
                m.insert("reason".into(), reason.name().into());
            }
            ElementKind::Function { arrow: true } => {
                m.insert("arrow".into(), true.into());
            }
            ElementKind::Binary { prec } => {
                m.insert("prec".into(), prec.name().into());
            }
            ElementKind::Unary { postfix: true } => {
                m.insert("postfix".into(), true.into());
            }
            ElementKind::Segment(kind) => {
                m.insert("segment".into(), kind.name().into());
                if let Some(segment) = e.as_segment() {
                    if segment.is_safe() {
                        m.insert("safe".into(), true.into());
                    }
                    if segment.is_static() {
                        m.insert("static".into(), true.into());
                    }
                }
            }
            _ => {}
        }
        if self.opts.spans {
            m.insert("span".into(), json!([e.span.start, e.span.end]));
        }
        if let Some(t) = &e.open {
            m.insert("open".into(), self.delim(t));
        }
        if let Some(t) = &e.close {
            let key = if e.kind.is_statement() {
                "terminator"
            } else {
                "close"
            };
            m.insert(key.into(), self.delim(t));
        }
        if !e.children.is_empty() {
            m.insert(
                "children".into(),
                Value::Array(e.children.iter().map(|n| self.node(n)).collect()),
            );
        }
        if !e.items.is_empty() {
            m.insert(
                "items".into(),
                Value::Array(e.items.iter().map(|i| self.item(i)).collect()),
            );
        }
        Value::Object(m)
    }

    fn item(&self, i: &Item) -> Value {
        let nodes = |nodes: &[Node]| Value::Array(nodes.iter().map(|n| self.node(n)).collect());
        let mut m = Map::new();
        if !i.leading.is_empty() {
            m.insert("leading".into(), nodes(&i.leading));
        }
        m.insert("children".into(), nodes(&i.children));
        if !i.trailing.is_empty() {
            m.insert("trailing".into(), nodes(&i.trailing));
        }
        if let Some(t) = &i.separator {
            m.insert("sep".into(), self.delim(t));
        }
        Value::Object(m)
    }
}

fn token_value(text: &str, t: &Token, opts: JsonOpts) -> Value {
    let mut m = Map::new();
    m.insert("k".into(), t.kind.name().into());
    m.insert("t".into(), text.into());
    if opts.spans {
        m.insert("span".into(), json!([t.span.start, t.span.end]));
    }
    Value::Object(m)
}

fn is_flat(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().all(|x| !x.is_array() && !x.is_object()),
        Value::Object(o) => o.values().all(|x| match x {
            Value::Array(a) => a.iter().all(|y| !y.is_array() && !y.is_object()),
            Value::Object(_) => false,
            _ => true,
        }),
        _ => true,
    }
}

fn write(v: &Value, indent: usize, out: &mut String) {
    if is_flat(v) {
        write_inline(v, out);
        return;
    }
    let pad = |n: usize, out: &mut String| out.extend(std::iter::repeat_n(' ', n));
    match v {
        Value::Array(a) => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                pad(indent + 2, out);
                write(x, indent + 2, out);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push(']');
        }
        Value::Object(o) => {
            out.push_str("{\n");
            for (i, (k, x)) in o.iter().enumerate() {
                pad(indent + 2, out);
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write(x, indent + 2, out);
                if i + 1 < o.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push('}');
        }
        _ => write_inline(v, out),
    }
}

fn write_inline(v: &Value, out: &mut String) {
    match v {
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_inline(x, out);
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push_str("{ ");
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_inline(x, out);
            }
            out.push_str(" }");
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}
