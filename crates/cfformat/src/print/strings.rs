//! Strings and template expressions.

use cfdoc::utils::remove_lines;
use cfdoc::Doc;
use cfparse::{Element, ElementKind, Node, Quote};

use super::Printer;
use crate::options::{NestedQuotes, QuoteStyle};

impl Printer<'_> {
    /// A quoted string. `strings.quote` (`strings.attributes.quote` for
    /// attribute values) picks the quote; `strings.convert_nested_quotes`
    /// decides for a string whose text holds either quote character: always
    /// that quote, the quote as written, or (the default) that quote unless
    /// the text holds more of it than of the other, Prettier's rule.
    /// Converting doubles the new quote inside the text and un-doubles the old
    /// one (so `"te""s''t"` becomes `'te"s''''t'`). `#expr#` parts print flat,
    /// and their quotes are code, not text.
    pub(crate) fn string(&self, e: &Element) -> Doc {
        self.string_as(e, false)
    }

    /// [`Printer::string`]; `attribute` picks `strings.attributes.quote` for a
    /// string the tree does not mark `in_tag`: a `cfhttp(url = "x")` value is
    /// the attribute of a tag written as a call.
    pub(crate) fn string_as(&self, e: &Element, attribute: bool) -> Doc {
        let in_tag = attribute || matches!(e.kind, ElementKind::String { in_tag: true, .. });
        let style = if in_tag {
            self.opts.strings_attributes_quote
        } else {
            self.opts.strings_quote
        };
        self.string_with(e, style)
    }

    /// An HTML attribute value: its quote as written. The quote options are
    /// for CFML strings; an HTML attribute keeps the quote it was written
    /// with (kept for compatibility with CommandBox cfformat's output).
    pub(crate) fn html_string(&self, e: &Element) -> Doc {
        self.string_with(e, QuoteStyle::Ignored)
    }

    /// [`Printer::string_as`] with the target quote already decided.
    fn string_with(&self, e: &Element, style: QuoteStyle) -> Doc {
        let (ElementKind::String { quote, .. }, Some(_), Some(_)) = (&e.kind, &e.open, &e.close)
        else {
            return self.as_written(e);
        };
        let current = match quote {
            Quote::Single => '\'',
            Quote::Double => '"',
        };
        let wanted = match style {
            QuoteStyle::Single => '\'',
            QuoteStyle::Double => '"',
            QuoteStyle::Ignored => current,
        };
        // The text's quote characters, the written quote counted once for
        // its doubled escape.
        let (mut singles, mut doubles) = (0, 0);
        for t in e.children.iter().filter_map(Node::as_token) {
            singles += self.tree.text(t).matches('\'').count();
            doubles += self.tree.text(t).matches('"').count();
        }
        match current {
            '\'' => singles /= 2,
            _ => doubles /= 2,
        }
        let count = |q: char| if q == '\'' { singles } else { doubles };
        let other = if wanted == '\'' { '"' } else { '\'' };
        let target = match self.opts.strings_convert_nested_quotes {
            _ if style == QuoteStyle::Ignored => current,
            _ if singles + doubles == 0 => wanted,
            NestedQuotes::Always => wanted,
            NestedQuotes::Never => current,
            NestedQuotes::FewerEscapes if count(wanted) > count(other) => other,
            NestedQuotes::FewerEscapes => wanted,
        };

        let quote_doc = Doc::from(target.to_string());
        let mut parts = vec![quote_doc.clone()];
        let mut text = String::new();
        for n in &e.children {
            match n {
                Node::Token(t) => text.push_str(self.tree.text(t)),
                Node::Element(_) => {
                    parts.push(self.string_text(std::mem::take(&mut text), current, target));
                    parts.push(self.node(n));
                }
            }
        }
        parts.push(self.string_text(text, current, target));
        parts.push(quote_doc);
        Doc::Concat(parts)
    }

    /// A run of string text, re-escaped for `target`; line breaks inside the
    /// string are kept verbatim.
    fn string_text(&self, text: String, current: char, target: char) -> Doc {
        if text.is_empty() {
            return Doc::empty();
        }
        let text = match (current, target) {
            ('"', '\'') => text.replace('\'', "''").replace("\"\"", "\""),
            ('\'', '"') => text.replace('"', "\"\"").replace("''", "'"),
            _ => text,
        };
        self.text(&text)
    }

    /// `#expr#`: the expression printed flat (no line inside may break). One
    /// holding a line comment cannot be flat and keeps its lines through the
    /// fallback.
    pub(crate) fn template_expression(&self, e: &Element) -> Doc {
        let (Some(open), Some(close)) = (&e.open, &e.close) else {
            return self.as_written(e);
        };
        if e.children
            .iter()
            .any(|n| matches!(n, Node::Element(c) if c.kind == ElementKind::LineComment))
        {
            return self.as_written(e);
        }
        let inner = remove_lines(Doc::Concat(self.sequence(&e.children)));
        Doc::Concat(vec![self.token(open), inner, self.token(close)])
    }
}
