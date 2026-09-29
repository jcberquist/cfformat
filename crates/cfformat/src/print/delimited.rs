//! Delimited elements: structs, arrays, call arguments, parameters and
//! `cfhttp(…)` attributes.
//!
//! One printer, [`Printer::print_delimited`], lays every kind out as
//! `group([open, pad, indent([softline, items]), softline, pad, close])`: flat
//! when it fits and the construct's threshold ([`threshold_breaks`]: at least
//! `element_count` items averaging more than `min_item_length` columns) does
//! not force a break, else one item per line. The threshold measures the
//! items' average width, not the whole flat text, so a long list of tiny items
//! does not break for its length alone. Comments come already attached to
//! their items by `cfparse`; `line_suffix` and `break_parent` place them.

use cfdoc::builders::{
    break_parent, group_opts, hardline, if_break, if_break_group, indent, indent_if_break, line,
    softline, GroupOpts,
};
use cfdoc::utils::flat_width;
use cfdoc::{Doc, FlatWidth, GroupId};
use cfparse::{Element, ElementKind, Ident, Item, Literal, Node, Punct, TokenKind};

use super::alignment::{self, RunPart};
use super::Printer;
use crate::options::{CommaStyle, QuoteStyle};
use crate::Options;

/// How a `KeyValue` item or attribute prints its separator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyValueStyle {
    /// Struct member: `struct.separator` verbatim, bare keys quoted when
    /// `struct.quote_keys`.
    Struct,
    /// Named argument: `key: value` or `key = value`, by the source token.
    Argument,
    /// `cfhttp(…)` attribute: always `key = value`.
    Padded,
    /// Parameter default `a = 1` (always padded); a parameter attribute
    /// (`string a key=true`) prints as an [`KeyValueStyle::Attribute`].
    Parameter,
    /// Declaration, `property`, `param` or `http …;` attribute: `key=value`,
    /// or `key = value` with `attributes.key_value.padding`.
    Attribute,
    /// HTML tag attribute: `key=value`, the value's quotes as written.
    /// Neither `strings.attributes.quote` nor `attributes.key_value.padding`
    /// reaches it: those options are for the attributes of CF tags and script
    /// declarations (kept for compatibility with CommandBox cfformat's
    /// output).
    HtmlAttribute,
}

impl KeyValueStyle {
    /// The style a `KeyValue` gets from its key alone, outside a delimited
    /// element that knows better: attribute, named argument, parameter
    /// default or struct member.
    pub(crate) fn by_key(e: &Element) -> Self {
        match e.as_key_value().map(|kv| kv.key()) {
            Some(Node::Token(t)) => match t.kind {
                TokenKind::Ident(Ident::AttributeName) => KeyValueStyle::Attribute,
                TokenKind::Ident(Ident::ArgName) => KeyValueStyle::Argument,
                TokenKind::Ident(Ident::Parameter) => KeyValueStyle::Parameter,
                _ => KeyValueStyle::Struct,
            },
            _ => KeyValueStyle::Struct,
        }
    }
}

/// Layout options of one delimited kind, read from its keys
/// (`struct.padding`, `struct.multiline.*`, `multiline.comma`, …).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DelimitedStyle {
    /// A space inside the delimiters when flat.
    pub padding: bool,
    /// A space inside the delimiters when empty (`{ }`, `[ ]`, `[ : ]`);
    /// parentheses never pad when empty.
    pub empty_padding: bool,
    /// Break when at least this many items (0: never, the list breaks only
    /// when it does not fit)…
    pub element_count: u32,
    /// …average more than this many columns printed flat.
    pub min_item_length: u32,
    /// Where the commas go when the list breaks: `multiline.comma` as this
    /// kind of list applies it ([`CommaStyle::literal`] for structs and
    /// arrays, [`CommaStyle::list`] for arguments and parameters), so never
    /// [`CommaStyle::DanglingAll`].
    pub comma: CommaStyle,
    /// How `KeyValue` items print.
    pub key_value: KeyValueStyle,
}

impl DelimitedStyle {
    /// [`threshold_breaks`] for items `widths` wide.
    pub fn breaks(&self, widths: &[FlatWidth]) -> bool {
        threshold_breaks(self.element_count, self.min_item_length, widths)
    }

    /// Whether [`DelimitedStyle::breaks`] needs item widths for `count` items.
    pub fn measures(&self, count: usize) -> bool {
        self.element_count > 0 && count as u32 >= self.element_count
    }

    /// `{ … }` and `[a: 1]`.
    pub fn structs(o: &Options) -> Self {
        DelimitedStyle {
            padding: o.struct_padding,
            empty_padding: o.struct_empty_padding,
            element_count: o.struct_multiline_element_count,
            min_item_length: o.struct_multiline_min_item_length,
            comma: o.multiline_comma.literal(),
            key_value: KeyValueStyle::Struct,
        }
    }

    /// `[ … ]`.
    pub fn array(o: &Options) -> Self {
        DelimitedStyle {
            padding: o.array_padding,
            empty_padding: o.array_empty_padding,
            element_count: o.array_multiline_element_count,
            min_item_length: o.array_multiline_min_item_length,
            comma: o.multiline_comma.literal(),
            key_value: KeyValueStyle::Struct,
        }
    }

    /// Call and constructor arguments.
    pub fn function_call(o: &Options) -> Self {
        DelimitedStyle {
            padding: o.parentheses_padding,
            empty_padding: false,
            element_count: o.function_call_multiline_element_count,
            min_item_length: o.function_call_multiline_min_item_length,
            comma: o.multiline_comma.list(),
            key_value: KeyValueStyle::Argument,
        }
    }

    /// `cfhttp(url = "x")`: the call thresholds, attributes always padded.
    pub fn script_tag(o: &Options) -> Self {
        DelimitedStyle {
            key_value: KeyValueStyle::Padded,
            ..Self::function_call(o)
        }
    }

    /// Parameters of a named function.
    pub fn function_declaration(o: &Options) -> Self {
        DelimitedStyle {
            padding: o.parentheses_padding,
            empty_padding: false,
            element_count: o.function_declaration_multiline_element_count,
            min_item_length: o.function_declaration_multiline_min_item_length,
            comma: o.multiline_comma.list(),
            key_value: KeyValueStyle::Parameter,
        }
    }

    /// Parameters of an anonymous function or arrow.
    pub fn function_anonymous(o: &Options) -> Self {
        DelimitedStyle {
            padding: o.parentheses_padding,
            empty_padding: false,
            element_count: o.function_anonymous_multiline_element_count,
            min_item_length: o.function_anonymous_multiline_min_item_length,
            comma: o.multiline_comma.list(),
            key_value: KeyValueStyle::Parameter,
        }
    }
}

/// A list's threshold (`*.multiline.element_count` /
/// `*.multiline.min_item_length`): it breaks when it has at least
/// `element_count` items (0 turns the threshold off) and their one-line
/// widths, `widths`, average more than `min_item_length` columns. An item
/// that cannot print on one line counts as wider than any.
pub(crate) fn threshold_breaks(
    element_count: u32,
    min_item_length: u32,
    widths: &[FlatWidth],
) -> bool {
    if element_count == 0 || (widths.len() as u32) < element_count {
        return false;
    }
    let mut total = 0usize;
    for w in widths {
        match w {
            FlatWidth::Finite(w) => total += w,
            FlatWidth::Infinite => return true,
        }
    }
    total > min_item_length as usize * widths.len()
}

/// An item prints when it has content or comments; the empty item of `[1,,2]`
/// or of a trailing comma does not.
pub(crate) fn is_printable(item: &Item) -> bool {
    item.significant().next().is_some() || item.leading_comments().next().is_some() || {
        item.trailing_comments().next().is_some()
    }
}

/// Whether an item carries line comments in its `leading` / `trailing` runs.
pub(crate) fn has_comments(item: &Item) -> bool {
    item.leading_comments().next().is_some() || item.trailing_comments().next().is_some()
}

impl Printer<'_> {
    /// `{ … }`, `[ … ]`, `( … )` with the threshold of `style` (see the module
    /// docs). Empty: the delimiters alone (`{}`, `[]`, `()`, and `[:]` for an
    /// empty ordered struct), a space apart with `style.empty_padding` (`{ }`,
    /// `[ ]`, `[ : ]`).
    pub(crate) fn print_delimited(&self, e: &Element, style: &DelimitedStyle) -> Doc {
        self.delimited(e, style, false)
    }

    fn delimited(&self, e: &Element, style: &DelimitedStyle, should_break: bool) -> Doc {
        let (Some(open), Some(close)) = (&e.open, &e.close) else {
            return self.as_written(e);
        };
        let items = printable_items(e);
        if items.is_empty() || is_empty_ordered_struct(e, &items) {
            let pad = || Doc::from(if style.empty_padding { " " } else { "" });
            let mut parts = vec![self.token(open), pad()];
            if !items.is_empty() {
                parts.push(Doc::from(":"));
                parts.push(pad());
            }
            parts.push(self.token(close));
            return Doc::Concat(parts);
        }
        let last = items.len() - 1;
        // A comment-only item takes no comma: the first item with content
        // takes the leading spacer.
        let first = items
            .iter()
            .position(|i| i.significant().next().is_some())
            .unwrap_or(0);
        let pads = self.item_pads(&items, style);
        let id = self.alignment_id(&pads);
        let measure = style.measures(items.len());
        let mut widths = Vec::new();
        let docs = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let at = (i == first, i == last);
                let pad = Self::item_pad(pads[i], id);
                self.item_doc(item, at, style, pad, measure.then_some(&mut widths))
            })
            .collect();
        group_opts(
            self.delimited_body(e, style, docs, None),
            GroupOpts {
                id,
                should_break: should_break || style.breaks(&widths),
            },
        )
    }

    /// `[open, pad, indent([softline, item, line, item …]), softline, pad,
    /// close]` over printed items (each with its separator), without the
    /// group, so a caller can put it in a conditional group. With
    /// `indent_id` the indent is `indent_if_break` on that group, so the
    /// body printed flat (a hugged argument's own lines) is not indented.
    pub(crate) fn delimited_body(
        &self,
        e: &Element,
        style: &DelimitedStyle,
        items: Vec<Doc>,
        indent_id: Option<GroupId>,
    ) -> Doc {
        self.delimited_body_with(e, style, items, indent_id, None)
    }

    /// [`Printer::delimited_body`] with the padding before the closing
    /// delimiter read from `close_pad_id`'s group rather than the list's:
    /// none after a hugged arrow whose body broke, since its `softline`
    /// already put the delimiter on a line of its own.
    pub(crate) fn delimited_body_with(
        &self,
        e: &Element,
        style: &DelimitedStyle,
        items: Vec<Doc>,
        indent_id: Option<GroupId>,
        close_pad_id: Option<GroupId>,
    ) -> Doc {
        let pad = || {
            if style.padding {
                if_break("", " ")
            } else {
                Doc::empty()
            }
        };
        let close_pad = match close_pad_id {
            Some(id) if style.padding => if_break_group("", " ", id),
            _ => pad(),
        };
        let mut body = vec![softline()];
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                body.push(line());
            }
            body.push(item);
        }
        Doc::Concat(vec![
            self.token(e.open.as_ref().expect("delimiter")),
            pad(),
            match indent_id {
                Some(id) => indent_if_break(body, id, false),
                None => indent(body),
            },
            softline(),
            close_pad,
            self.token(e.close.as_ref().expect("delimiter")),
        ])
    }

    /// One item with its separator and comments: the leading comments each
    /// on their own line; in a leading style the item's comma
    /// ([`comma_before`]); the content; a same-line trailing comment as a line
    /// suffix, so it lands after the `,` (`a, // t`) whichever side of the
    /// separator it was on; then the `,` ([`comma_after`]); then the own-line
    /// trailing comments. An item holding only comments prints them on their
    /// own lines and no separator: the previous item's `,` already precedes
    /// it, or in a leading style the next item's follows it.
    fn item_doc(
        &self,
        item: &Item,
        (first, last): (bool, bool),
        style: &DelimitedStyle,
        pad: Option<Doc>,
        widths: Option<&mut Vec<FlatWidth>>,
    ) -> Doc {
        let mut parts = Vec::new();
        for c in item.leading_comments() {
            parts.push(self.comment(c));
            parts.push(hardline());
        }
        if item.significant().next().is_none() {
            for (i, c) in item.trailing_comments().enumerate() {
                if i > 0 {
                    parts.push(hardline());
                }
                parts.push(self.comment(c));
            }
            parts.push(break_parent());
            return Doc::Concat(parts);
        }
        // A leading comma follows the item's own-line comments (`// c` newline
        // `, b`): printed before them it would end the line of a comment the
        // next parse gives to the previous item (a line comment right after a
        // comma belongs to the item before the comma).
        parts.extend(comma_before(style.comma, first));
        let content = self.item_content(item, style.key_value, pad.as_ref());
        if let Some(widths) = widths {
            widths.push(flat_width(&content));
        }
        parts.push(content);

        // The trailing run and the separator in source order: a comment with
        // no newline before it (since the content) shares the content's line.
        let mut trailing: Vec<&Node> = item.trailing.iter().collect();
        trailing.sort_by_key(|n| n.span().start);
        let mut newline = false;
        let mut own_line = Vec::new();
        for n in trailing {
            match n {
                Node::Token(t) if t.kind == TokenKind::Newline => newline = true,
                Node::Element(c) if c.kind.is_comment() => {
                    if newline {
                        own_line.push(c);
                    } else {
                        parts.push(self.same_line_comment(c));
                    }
                }
                _ => {}
            }
        }
        parts.extend(comma_after(style.comma, last));
        for c in own_line {
            parts.push(hardline());
            parts.push(self.comment(c));
        }
        Doc::Concat(parts)
    }

    /// The alignment padding of each item of a delimited list
    /// (`alignment.consecutive.assignments`): a run is a maximal sequence of
    /// items whose content is a `KeyValue` ([`Printer::item_left`]); an item
    /// holding only comments does not end it, any other item does (a
    /// positional argument, a parameter without a default).
    pub(crate) fn item_pads(&self, items: &[&Item], style: &DelimitedStyle) -> Vec<Option<usize>> {
        if !self.opts.alignment_consecutive_assignments {
            return vec![None; items.len()];
        }
        alignment::runs(items, |item| {
            if item.children.iter().all(|n| n.is_trivia()) {
                RunPart::Neutral
            } else {
                self.item_left(item, style.key_value)
                    .map_or(RunPart::Break, alignment::member)
            }
        })
    }

    /// A group id for a list whose items pad, so the padding prints only when
    /// the list breaks; `None` when nothing pads.
    pub(crate) fn alignment_id(&self, pads: &[Option<usize>]) -> Option<GroupId> {
        pads.iter()
            .any(Option::is_some)
            .then(|| self.ids.borrow_mut().next_id())
    }

    /// An item's padding on the list's group `id`.
    pub(crate) fn item_pad(pad: Option<usize>, id: Option<GroupId>) -> Option<Doc> {
        Some(alignment::pad_if_break(pad?, id?))
    }

    /// The printed left side of a `KeyValue` item: the key as printed (quotes
    /// included), after a parameter's modifiers and type (`required string
    /// carouselId`). `None` for any other item, or one with a comment inside.
    fn item_left(&self, item: &Item, style: KeyValueStyle) -> Option<Doc> {
        if item
            .children
            .iter()
            .any(|n| n.as_element().is_some_and(|c| c.kind.is_comment()))
        {
            return None;
        }
        let sig: Vec<&Node> = item.children.iter().filter(|n| !n.is_trivia()).collect();
        let (Node::Element(kv), words) = sig.split_last()? else {
            return None;
        };
        let words_ok = words.iter().all(|w| {
            matches!(w, Node::Token(t) if matches!(t.kind, TokenKind::Keyword(_) | TokenKind::Storage(_)))
        });
        if kv.kind != ElementKind::KeyValue || !words_ok {
            return None;
        }
        let view = kv.as_key_value()?;
        if kv
            .children
            .iter()
            .any(|n| n.as_element().is_some_and(|c| c.kind.is_comment()))
        {
            return None;
        }
        let mut left = self.sequence(words.iter().copied());
        if !left.is_empty() {
            left.push(Doc::from(" "));
        }
        left.push(self.key(view.key(), style));
        Some(Doc::Concat(left))
    }

    /// An item's content: its significant children through
    /// [`Printer::sequence`] (`required string a = 1` keeps its modifiers,
    /// block comments print in place), a `KeyValue` in `key_value` style with
    /// the alignment padding `pad` after its key.
    pub(crate) fn item_content(
        &self,
        item: &Item,
        key_value: KeyValueStyle,
        pad: Option<&Doc>,
    ) -> Doc {
        // A ternary that is a whole call argument indents the continuation
        // lines of a binary condition ([`Printer::ternary_cond_indent`]).
        let mut sig = item.children.iter().filter(|n| !n.is_trivia());
        let ternary_arg = key_value == KeyValueStyle::Argument
            && matches!(
                (sig.next(), sig.next()),
                (Some(Node::Element(t)), None) if t.kind == ElementKind::Ternary
            );
        self.ternary_cond_indent.set(ternary_arg);
        let parts = self.sequence_by(&item.children, &|n| match n {
            Node::Element(kv) if kv.kind == ElementKind::KeyValue => {
                self.key_value_with(kv, key_value, pad.cloned())
            }
            n => self.node(n),
        });
        Doc::Concat(parts)
    }

    /// A `Punct(KeyValue)` separator with its spacing in `style`.
    fn separator(&self, sep: &cfparse::Token, style: KeyValueStyle) -> Doc {
        Doc::from(match style {
            KeyValueStyle::Parameter => " = ".to_string(),
            KeyValueStyle::Struct => self.opts.struct_separator.clone(),
            KeyValueStyle::Argument if self.tree.text(sep) == ":" => ": ".to_string(),
            KeyValueStyle::Argument | KeyValueStyle::Padded => " = ".to_string(),
            KeyValueStyle::Attribute if self.opts.attributes_key_value_padding => {
                format!(" {} ", self.tree.text(sep))
            }
            KeyValueStyle::Attribute | KeyValueStyle::HtmlAttribute => {
                self.tree.text(sep).to_string()
            }
        })
    }

    /// `key sep value`. The value goes through [`Printer::sequence`], so an
    /// unparsed value keeps its parts; a comment between key and separator
    /// follows the key.
    pub(crate) fn key_value(&self, e: &Element, style: KeyValueStyle) -> Doc {
        self.key_value_with(e, style, None)
    }

    /// [`Printer::key_value`] with alignment padding after the key.
    pub(crate) fn key_value_with(
        &self,
        e: &Element,
        style: KeyValueStyle,
        pad: Option<Doc>,
    ) -> Doc {
        let Some(kv) = e.as_key_value() else {
            return self.as_written(e);
        };
        let key = kv.key();
        let sep = kv.separator();
        let style = match key {
            Node::Token(t)
                if style == KeyValueStyle::Parameter
                    && t.kind == TokenKind::Ident(Ident::AttributeName) =>
            {
                KeyValueStyle::Attribute
            }
            _ => style,
        };
        // A struct member without comments lays out as Prettier's object
        // property (`printAssignment`, with its short-key rule).
        let commented = e
            .children
            .iter()
            .any(|n| matches!(n, Node::Element(c) if c.kind.is_comment()));
        let mut value = kv.value().iter().filter(|n| !n.is_trivia());
        if let (KeyValueStyle::Struct, false, Some(v), None) =
            (style, commented, value.next(), value.next())
        {
            let key_doc = self.key(key, style);
            let short_key = matches!(flat_width(&key_doc),
                FlatWidth::Finite(w) if w < self.opts.indent_size + 3);
            let mut left = vec![key_doc];
            left.extend(pad);
            let left = Doc::Concat(left);
            let layout = self.assign_layout(v, short_key, &left);
            let separator = self.opts.struct_separator.trim_end();
            let spaced = separator.len() < self.opts.struct_separator.len();
            return self.assign_doc_spaced(
                left,
                Doc::from(separator.to_string()),
                spaced,
                self.assigned_value(v),
                layout,
            );
        }
        let mut parts = vec![self.key(key, style)];
        parts.extend(pad);
        // An attribute group breaks only between attributes, never inside
        // one: a line comment before an attribute's separator ends the line
        // the attribute ends on, and breaks nothing.
        let in_attributes = matches!(
            style,
            KeyValueStyle::Attribute | KeyValueStyle::HtmlAttribute
        );
        for n in &e.children {
            match n {
                Node::Element(c) if c.kind.is_comment() && c.span.start < sep.span.start => {
                    parts.push(if in_attributes {
                        self.deferred_comment(c)
                    } else {
                        self.same_line_comment(c)
                    });
                }
                _ => {}
            }
        }
        parts.push(self.separator(sep, style));
        // An unquoted tag value in pieces (`url=www.#host#/x`): whitespace
        // would end it, so the pieces print back to back.
        let pieces: Vec<&Node> = kv.value().iter().filter(|n| !n.is_trivia()).collect();
        if pieces.len() > 1 && pieces.iter().all(|n| is_unquoted_part(n)) {
            parts.extend(pieces.into_iter().map(|n| self.node(n)));
            return Doc::Concat(parts);
        }
        let attribute = matches!(style, KeyValueStyle::Padded | KeyValueStyle::Attribute);
        let html = style == KeyValueStyle::HtmlAttribute;
        parts.extend(self.sequence_by(kv.value(), &|n| match n {
            Node::Element(s) if attribute && matches!(s.kind, ElementKind::String { .. }) => {
                self.string_as(s, true)
            }
            // An unquoted value ends at whitespace: its operators
            // stay as tight as written, or the value would end at the first.
            Node::Element(b)
                if style == KeyValueStyle::Attribute
                    && matches!(b.kind, ElementKind::Binary { .. }) =>
            {
                self.as_written(b)
            }
            Node::Element(s) if html && matches!(s.kind, ElementKind::String { .. }) => {
                self.html_string(s)
            }
            n => self.node(n),
        }));
        Doc::Concat(parts)
    }

    /// A key as printed: a bare struct key is quoted with `struct.quote_keys`.
    fn key(&self, key: &Node, style: KeyValueStyle) -> Doc {
        match key {
            Node::Token(t)
                if style == KeyValueStyle::Struct
                    && t.kind == TokenKind::Ident(Ident::StructKey)
                    && self.opts.struct_quote_keys =>
            {
                let quote = if self.opts.strings_quote == QuoteStyle::Double {
                    '"'
                } else {
                    '\''
                };
                Doc::from(format!("{quote}{}{quote}", self.tree.text(t)))
            }
            key => self.node(key),
        }
    }

    /// `['string']['a', 'b']`: the type's brackets against the array (only
    /// spaces can sit between them).
    pub(crate) fn typed_array(&self, e: &Element) -> Doc {
        Doc::Concat(
            e.children
                .iter()
                .filter(|n| !n.is_trivia())
                .map(|n| self.node(n))
                .collect(),
        )
    }

    /// `a[expr]`'s brackets: padded with `brackets.padding` while flat
    /// (`a[ 1 ]`), the expression indented on its own line when it does not
    /// fit. A number literal alone never breaks (`a[ 1 ]`, Prettier's
    /// `printMemberLookup`). Every index reaches this, chain segments
    /// included; an array literal and `[:]` are delimited elements.
    pub(crate) fn brackets(&self, e: &Element) -> Doc {
        let (Some(open), Some(close)) = (&e.open, &e.close) else {
            return self.as_written(e);
        };
        let content = self.sequence(&e.children);
        if content.is_empty() {
            return Doc::Concat(vec![self.token(open), self.token(close)]);
        }
        let mut children = e
            .children
            .iter()
            .filter(|n| !matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)));
        if let (Some(Node::Token(t)), None) = (children.next(), children.next()) {
            if t.kind == TokenKind::Literal(Literal::Number) {
                let pad = if self.opts.brackets_padding { " " } else { "" };
                return Doc::Concat(vec![
                    self.token(open),
                    Doc::from(pad),
                    Doc::Concat(content),
                    Doc::from(pad),
                    self.token(close),
                ]);
            }
        }
        let pad = || {
            if self.opts.brackets_padding {
                if_break("", " ")
            } else {
                Doc::empty()
            }
        };
        cfdoc::builders::group(vec![
            self.token(open),
            pad(),
            indent(vec![softline(), Doc::Concat(content)]),
            softline(),
            pad(),
            self.token(close),
        ])
    }
}

/// What precedes an item of a broken list in a leading style: `, ` / `,`
/// before every item but the first, and before the first a spacer of the same
/// width, so the items align. Nothing when flat, and nothing in a trailing
/// style.
fn comma_before(comma: CommaStyle, first: bool) -> Option<Doc> {
    let (spacer, separator) = match comma {
        CommaStyle::Leading => ("  ", ", "),
        CommaStyle::LeadingTight => (" ", ","),
        // Trailing and dangling (`DelimitedStyle::comma` is never
        // `DanglingAll`: `CommaStyle::literal` / `list` map it).
        _ => return None,
    };
    Some(if_break(if first { spacer } else { separator }, ""))
}

/// What follows an item's content: `,` after every item but the last (in a
/// leading style only while flat: broken, the next item's comma precedes
/// it), and after the last the dangling comma when broken
/// ([`CommaStyle::Dangling`], which `CommaStyle::literal` / `list` map both
/// dangling values to where they apply; a leading style never dangles).
fn comma_after(comma: CommaStyle, last: bool) -> Option<Doc> {
    match comma {
        _ if last => (comma == CommaStyle::Dangling).then(|| if_break(",", "")),
        CommaStyle::Leading | CommaStyle::LeadingTight => Some(if_break("", ",")),
        _ => Some(Doc::from(",")),
    }
}

/// `[:]`: an ordered struct whose only item is the `:` token.
fn is_empty_ordered_struct(e: &Element, items: &[&Item]) -> bool {
    e.kind == (ElementKind::Struct { ordered: true }) && items.iter().all(|i| {
        let mut sig = i.significant();
        matches!(sig.next(), Some(Node::Token(t)) if t.kind == TokenKind::Punct(Punct::KeyValue))
            && sig.next().is_none()
    })
}

/// A piece of an unquoted tag attribute value: its text, or a `#…#`.
fn is_unquoted_part(n: &Node) -> bool {
    match n {
        Node::Token(t) => t.kind == TokenKind::Literal(Literal::Unquoted),
        Node::Element(e) => e.kind == ElementKind::TemplateExpression,
    }
}

/// Items that print (see [`is_printable`]).
pub(crate) fn printable_items(e: &Element) -> Vec<&Item> {
    e.items.iter().filter(|i| is_printable(i)).collect()
}

/// Items without comments, already printed, with their commas in
/// `style.comma` ([`comma_before`], [`comma_after`]).
pub(crate) fn plain_items(contents: Vec<Doc>, style: &DelimitedStyle) -> Vec<Doc> {
    let last = contents.len().saturating_sub(1);
    contents
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut parts: Vec<Doc> = comma_before(style.comma, i == 0).into_iter().collect();
            parts.push(c);
            parts.extend(comma_after(style.comma, i == last));
            Doc::Concat(parts)
        })
        .collect()
}
