//! The HTML side: tags and their attributes, entities, `<!-- -->` comments,
//! the doctype, CDATA and `<?xml …?>`.

use crate::tree::{Delim, ElementKind, Ident, Literal, Node, Punct, Quote, TagShape, TokenKind};

use super::scanner::{starts_ci, Ctx, Scanner};

/// ASCII whitespace (tab, newline, form feed, space) plus `/`, `<` and `>`:
/// what ends a tag name.
pub(super) fn is_tag_name_break(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0c' | b' ' | b'/' | b'<' | b'>')
}

/// ASCII whitespace (tab, newline, form feed, space) plus `=`, `/` and `>`:
/// what ends an attribute name.
fn is_attr_name_break(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0c' | b' ' | b'=' | b'/' | b'>')
}

/// `%` and two hex digits: a percent escape in an `href` / `src` value.
fn is_escape(rest: &[u8]) -> bool {
    rest.len() >= 3 && rest[1].is_ascii_hexdigit() && rest[2].is_ascii_hexdigit()
}

fn is_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0c' | b' ')
}

/// Tags whose opening tag closes on `>` only, so a `/` before the `>` is not
/// part of the closing delimiter.
fn closes_without_slash(name: &str) -> bool {
    ["html", "head", "body", "form", "fieldset"]
        .iter()
        .any(|tag| name.eq_ignore_ascii_case(tag))
}

/// How an attribute's value is read, chosen by the attribute's name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AttrRead {
    /// Any other attribute: entities only.
    Generic,
    /// `class` / `id`: entities, and whitespace is its own region.
    Split,
    /// `href` / `src`: entities, `%hh` and `[/&?#]` / `://`.
    Path,
    /// `style`: a CSS island inside the string.
    Style,
    /// An event attribute: a JS island inside the string.
    Event,
}

/// Sorted, for [`is_event_attribute`]'s binary search.
const EVENT_ATTRIBUTES: &[&str] = &[
    "onabort",
    "onautocomplete",
    "onautocompleteerror",
    "onauxclick",
    "onblur",
    "oncancel",
    "oncanplay",
    "oncanplaythrough",
    "onchange",
    "onclick",
    "onclose",
    "oncontextmenu",
    "oncopy",
    "oncuechange",
    "ondblclick",
    "ondrag",
    "ondragend",
    "ondragenter",
    "ondragexit",
    "ondragleave",
    "ondragover",
    "ondragstart",
    "ondrop",
    "ondurationchange",
    "onemptied",
    "onended",
    "onerror",
    "onfocus",
    "onfocusin",
    "onfocusout",
    "oninput",
    "oninvalid",
    "onkeydown",
    "onkeypress",
    "onkeyup",
    "onload",
    "onloadeddata",
    "onloadedmetadata",
    "onloadstart",
    "onmousedown",
    "onmouseenter",
    "onmouseleave",
    "onmousemove",
    "onmouseout",
    "onmouseover",
    "onmouseup",
    "onmousewheel",
    "onpaste",
    "onpause",
    "onplay",
    "onplaying",
    "onprogress",
    "onratechange",
    "onreset",
    "onresize",
    "onscroll",
    "onseeked",
    "onseeking",
    "onselect",
    "onshow",
    "onsort",
    "onstalled",
    "onsubmit",
    "onsuspend",
    "ontimeupdate",
    "ontoggle",
    "onvolumechange",
    "onwaiting",
];

/// Whether `name` is in [`EVENT_ATTRIBUTES`], ASCII case-insensitively.
fn is_event_attribute(name: &str) -> bool {
    starts_ci(name, "on")
        && EVENT_ATTRIBUTES
            .binary_search_by(|event| {
                event
                    .bytes()
                    .cmp(name.bytes().map(|b| b.to_ascii_lowercase()))
            })
            .is_ok()
}

fn attr_value_kind(name: &str) -> AttrRead {
    let is = |attribute: &str| name.eq_ignore_ascii_case(attribute);
    if is("class") || is("id") {
        AttrRead::Split
    } else if is("href") || is("src") {
        AttrRead::Path
    } else if is("style") {
        AttrRead::Style
    } else if is_event_attribute(name) {
        AttrRead::Event
    } else {
        AttrRead::Generic
    }
}

impl Scanner<'_> {
    /// In HTML content a `<` always starts a construct: when nothing else
    /// matches, a lone `<` is one `Text` token ([`Scanner::html_construct`]).
    pub(super) fn html_ahead(&self) -> bool {
        self.rest().starts_with('<')
    }

    /// The HTML construct at a `<`: `<?xml`, a doctype, a comment, CDATA or a
    /// tag, else a nameless tag or a lone `<`.
    pub(super) fn html_construct(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        let rest = self.rest();
        if self.preprocessor_ahead() {
            return self.preprocessor(out);
        }
        if starts_ci(rest, "<!") && self.doctype_ahead() {
            return self.doctype(out, ctx);
        }
        if rest.starts_with("<!--") {
            return self.html_comment(out, ctx);
        }
        if rest.starts_with("<![CDATA[") {
            self.cdata(out);
            return;
        }
        if let Some(name_len) = self.tag_name_ahead() {
            return self.html_tag(out, ctx, name_len);
        }
        // A `<` with only whitespace before `>` or `/>` is a nameless tag,
        // its whitespace `invalid`; else a lone `<` is text.
        let after = rest[1..].trim_start_matches(|c: char| is_space(c as u8) && c.is_ascii());
        let gap = rest.len() - 1 - after.len();
        if after.starts_with("/>") || after.starts_with('>') {
            let close = if after.starts_with("/>") { 2 } else { 1 };
            self.take(out, 1, TokenKind::Punct(Punct::Open(Delim::Tag)));
            let mut from = self.pos;
            self.pos += gap;
            self.flush_meta(out, &mut from, TokenKind::Invalid);
            self.take(out, close, TokenKind::Punct(Punct::Close(Delim::Tag)));
            return;
        }
        self.take(out, 1, TokenKind::Text);
    }

    /// `<?xml`, followed by `-`, `?`, a tag-name break or the end: an XML
    /// processing instruction ahead.
    fn preprocessor_ahead(&self) -> bool {
        let rest = self.rest();
        starts_ci(rest, "<?xml")
            && rest
                .as_bytes()
                .get(5)
                .is_none_or(|&b| b == b'-' || is_tag_name_break(b) || b == b'?')
    }

    fn doctype_ahead(&self) -> bool {
        let rest = self.rest();
        starts_ci(rest, "<!doctype") && rest.as_bytes().get(9).is_none_or(|&b| is_tag_name_break(b))
    }

    /// `</?` followed by a letter: the tag name's length, if any.
    fn tag_name_ahead(&self) -> Option<usize> {
        let rest = self.rest().as_bytes();
        let open = if rest.starts_with(b"</") { 2 } else { 1 };
        if !rest.get(open)?.is_ascii_alphabetic() {
            return None;
        }
        let len = rest[open..]
            .iter()
            .position(|&b| is_tag_name_break(b))
            .unwrap_or(rest.len() - open);
        Some(len)
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    fn html_tag(&mut self, out: &mut Vec<Node>, ctx: Ctx, name_len: usize) {
        let start = self.pos;
        let closing = self.rest().starts_with("</");
        let open_len = if closing { 2 } else { 1 };
        let src = self.src;
        let name = &src[start + open_len..start + open_len + name_len];

        if !closing && name.eq_ignore_ascii_case("script") {
            return self.script_tag(out, ctx);
        }
        if !closing && name.eq_ignore_ascii_case("style") {
            return self.style_tag(out, ctx);
        }

        let mut el = self.element(ElementKind::HtmlTag(TagShape::Open), 0..0);
        el.open = Some(self.token(
            start..start + open_len,
            TokenKind::Punct(Punct::Open(Delim::Tag)),
        ));
        self.pos = start + open_len;
        self.take(&mut el.children, name_len, TokenKind::Ident(Ident::TagName));
        self.tag_body(&mut el, ctx, closes_without_slash(name));
        self.finish_tag(out, el, start);
    }

    /// Attributes to the closing delimiter. `strict` tags close on `>` alone.
    pub(super) fn tag_body(&mut self, el: &mut crate::tree::Element, ctx: Ctx, strict: bool) {
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b == b'>' || (!strict && rest.starts_with("/>")) {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                let len = if b == b'>' { 1 } else { 2 };
                let from = self.pos;
                self.pos += len;
                el.close =
                    Some(self.token(from..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
                return;
            }
            // CF comments, CF tags and `#…#` are read inside a tag too.
            if b == b'<' && self.rest().starts_with("<!---") {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                self.cf_comment(&mut el.children);
                text = self.pos;
                continue;
            }
            if b == b'<' && self.cf_tag_ahead() {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                self.cf_tag(&mut el.children);
                text = self.pos;
                continue;
            }
            if b == b'#' && ctx.hash() {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                self.template_expression(&mut el.children);
                text = self.pos;
                continue;
            }
            if is_space(b) {
                self.pos += 1;
                continue;
            }
            // A stray `=` or `/` starts no attribute: skip it.
            if b == b'=' || b == b'/' {
                self.pos += 1;
                continue;
            }
            self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
            self.attribute(&mut el.children, ctx);
            text = self.pos;
        }
        self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
    }

    /// Set the shape from the delimiters and give the element its span.
    pub(super) fn finish_tag(
        &self,
        out: &mut Vec<Node>,
        mut el: crate::tree::Element,
        start: usize,
    ) {
        let shape = if el
            .open
            .as_ref()
            .is_some_and(|t| &self.src[t.span.start as usize..t.span.end as usize] == "</")
        {
            TagShape::Close
        } else if el
            .close
            .as_ref()
            .is_some_and(|t| &self.src[t.span.start as usize..t.span.end as usize] == "/>")
        {
            TagShape::SelfClosed
        } else {
            TagShape::Open
        };
        el.kind = match el.kind {
            ElementKind::CfTag(_, kind) => ElementKind::CfTag(shape, kind),
            _ => ElementKind::HtmlTag(shape),
        };
        crate::key_value::attributes(&mut el.children, false);
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }

    // -----------------------------------------------------------------------
    // Attributes
    // -----------------------------------------------------------------------

    fn attribute(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        let start = self.pos;
        // The name runs to a name break; a quote, backtick or `<` inside it
        // is an `invalid` token of its own.
        let mut invalid: Vec<usize> = Vec::new();
        while self.pos < self.end {
            let b = self.rest().as_bytes()[0];
            if is_attr_name_break(b) {
                break;
            }
            // A CF comment or CF tag ends the name rather than making its `<`
            // invalid.
            if b == b'<' && (self.rest().starts_with("<!---") || self.cf_tag_ahead()) {
                break;
            }
            if b == b'#' && ctx.hash() {
                break;
            }
            if matches!(b, b'"' | b'\'' | b'`' | b'<') {
                invalid.push(self.pos);
            }
            self.bump();
        }
        let src = self.src;
        let name = &src[start..self.pos];
        let mut at = start;
        for &bad in &invalid {
            self.push(out, at..bad, TokenKind::Ident(Ident::AttributeName));
            self.push(out, bad..bad + 1, TokenKind::Invalid);
            at = bad + 1;
        }
        self.push(out, at..self.pos, TokenKind::Ident(Ident::AttributeName));

        // `=`, whitespace allowed before it: a value follows.
        let save = self.pos;
        let mut eq = self.pos;
        while eq < self.end && is_space(self.src.as_bytes()[eq]) {
            eq += 1;
        }
        if self.src.as_bytes().get(eq) != Some(&b'=') {
            self.pos = save;
            return;
        }
        let mut from = self.pos;
        self.pos = eq;
        self.flush_meta(out, &mut from, TokenKind::Text);
        self.take(out, 1, TokenKind::Punct(Punct::KeyValue));
        self.attribute_value(out, ctx, attr_value_kind(name))
    }

    fn attribute_value(&mut self, out: &mut Vec<Node>, ctx: Ctx, kind: AttrRead) {
        let mut from = self.pos;
        while self.pos < self.end && is_space(self.rest().as_bytes()[0]) {
            self.pos += 1;
        }
        self.flush_meta(out, &mut from, TokenKind::Text);
        let Some(&b) = self.rest().as_bytes().first() else {
            return;
        };
        match b {
            b'"' | b'\'' => self.quoted_attribute_value(out, ctx, kind, b),
            // An unquoted value cannot start with `=` or `>`: the attribute
            // has no value.
            b'=' | b'>' => (),
            _ => self.unquoted_attribute_value(out, ctx, kind),
        }
    }

    fn quoted_attribute_value(&mut self, out: &mut Vec<Node>, ctx: Ctx, kind: AttrRead, quote: u8) {
        let start = self.pos;
        let mut el = self.element(
            ElementKind::String {
                quote: if quote == b'"' {
                    Quote::Double
                } else {
                    Quote::Single
                },
                in_tag: true,
            },
            0..0,
        );
        el.open = Some(self.token(
            start..start + 1,
            TokenKind::Punct(Punct::Open(Delim::String)),
        ));
        self.pos = start + 1;
        match kind {
            AttrRead::Style => self.attribute_island(&mut el, ctx, quote, false),
            AttrRead::Event => self.attribute_island(&mut el, ctx, quote, true),
            _ => self.attribute_text(&mut el.children, ctx, kind, quote),
        }
        // A `punct.close.string` among the children — the script front end's,
        // from a CFML tag inside the value — is taken as the string's closing
        // delimiter and handed back when content follows it, and the real
        // quote then lands in the tag, after the string (`settle_string`).
        let stolen = settle_string(&mut el);
        // A CF tag read flat into the value brings its attributes along.
        crate::key_value::attributes(&mut el.children, false);
        if self.rest().as_bytes().first() == Some(&quote) {
            let from = self.pos;
            self.pos += 1;
            let close = self.token(
                from..self.pos,
                TokenKind::Punct(Punct::Close(Delim::String)),
            );
            if stolen {
                el.span = start as u32..from as u32;
                out.push(Node::Element(Box::new(el)));
                out.push(Node::Token(close));
                return;
            }
            el.close = Some(close);
        }
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }

    /// The content of a quoted HTML attribute value, to its closing quote.
    fn attribute_text(&mut self, out: &mut Vec<Node>, ctx: Ctx, kind: AttrRead, quote: u8) {
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b == quote {
                break;
            }
            if b == b'<' && rest.starts_with("<!---") {
                self.push_value_text(out, &mut text);
                self.cf_comment(out);
                text = self.pos;
                continue;
            }
            if b == b'<' && self.cf_tag_ahead() {
                self.push_value_text(out, &mut text);
                self.cf_tag_flat(out);
                text = self.pos;
                continue;
            }
            if b == b'#' && ctx.hash() {
                self.push_value_text(out, &mut text);
                self.template_expression(out);
                text = self.pos;
                continue;
            }
            if b == b'&' && self.entity_len().is_some() {
                self.push_value_text(out, &mut text);
                self.entity(out, ctx);
                text = self.pos;
                continue;
            }
            if kind == AttrRead::Split && is_space(b) {
                // `tag-attribute-value-separator-*-quoted`.
                self.push_value_text(out, &mut text);
                while self.pos < self.end && is_space(self.rest().as_bytes()[0]) {
                    self.pos += 1;
                }
                self.flush_meta(out, &mut text, TokenKind::Literal(Literal::StringText));
                continue;
            }
            if kind == AttrRead::Path {
                if b == b'%' && is_escape(rest.as_bytes()) {
                    self.push_value_text(out, &mut text);
                    self.take(out, 1, TokenKind::Text);
                    self.take(out, 2, TokenKind::Text);
                    text = self.pos;
                    continue;
                }
                if rest.starts_with("://") {
                    self.push_value_text(out, &mut text);
                    self.take(out, 3, TokenKind::Literal(Literal::StringText));
                    text = self.pos;
                    continue;
                }
                if matches!(b, b'/' | b'&' | b'?' | b'#') {
                    self.push_value_text(out, &mut text);
                    self.take(out, 1, TokenKind::Literal(Literal::StringText));
                    text = self.pos;
                    continue;
                }
            }
            self.bump();
        }
        self.push_value_text(out, &mut text);
    }

    fn push_value_text(&self, out: &mut Vec<Node>, from: &mut usize) {
        self.region(
            out,
            *from..self.pos,
            TokenKind::Literal(Literal::StringText),
        );
        *from = self.pos;
    }

    /// An unquoted attribute value: bare tokens in the tag, not a `String`
    /// element.
    fn unquoted_attribute_value(&mut self, out: &mut Vec<Node>, ctx: Ctx, kind: AttrRead) {
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            // The value ends at whitespace, `>` or `/>`.
            if is_space(b) || b == b'>' || rest.starts_with("/>") {
                break;
            }
            // CF comments, CF tags and `#…#` are read inside an unquoted
            // value too; a CF tag goes in flat, as its tokens with no element
            // around them (`cf_tag_flat`).
            if b == b'<' && rest.starts_with("<!---") {
                self.push_unquoted(out, &mut text);
                self.cf_comment(out);
                text = self.pos;
                continue;
            }
            if b == b'<' && self.cf_tag_ahead() {
                self.push_unquoted(out, &mut text);
                self.cf_tag_flat(out);
                text = self.pos;
                continue;
            }
            if b == b'#' && ctx.hash() {
                self.push_unquoted(out, &mut text);
                self.template_expression(out);
                text = self.pos;
                continue;
            }
            if b == b'&' && self.entity_len().is_some() {
                self.push_unquoted(out, &mut text);
                self.entity(out, ctx);
                text = self.pos;
                continue;
            }
            if matches!(b, b'"' | b'\'' | b'`' | b'<') {
                self.push_unquoted(out, &mut text);
                self.take(out, 1, TokenKind::Invalid);
                text = self.pos;
                continue;
            }
            if kind == AttrRead::Path {
                if b == b'%' && is_escape(rest.as_bytes()) {
                    self.push_unquoted(out, &mut text);
                    self.take(out, 1, TokenKind::Text);
                    self.take(out, 2, TokenKind::Text);
                    text = self.pos;
                    continue;
                }
                if rest.starts_with("://") {
                    self.push_unquoted(out, &mut text);
                    self.take(out, 3, TokenKind::Literal(Literal::Unquoted));
                    text = self.pos;
                    continue;
                }
                if matches!(b, b'/' | b'&' | b'?' | b'#') {
                    self.push_unquoted(out, &mut text);
                    self.take(out, 1, TokenKind::Literal(Literal::Unquoted));
                    text = self.pos;
                    continue;
                }
            }
            self.bump();
        }
        self.push_unquoted(out, &mut text);
    }

    fn push_unquoted(&self, out: &mut Vec<Node>, from: &mut usize) {
        self.region(out, *from..self.pos, TokenKind::Literal(Literal::Unquoted));
        *from = self.pos;
    }

    // -----------------------------------------------------------------------
    // Entities
    // -----------------------------------------------------------------------

    /// Length of the HTML entity at the current position, if there is one.
    pub(super) fn entity_len(&self) -> Option<usize> {
        let rest = self.rest().as_bytes();
        if rest.first() != Some(&b'&') {
            return None;
        }
        let (from, hex) = match rest.get(1) {
            Some(b'#') => match rest.get(2) {
                Some(b'x') | Some(b'X') => (3, true),
                _ => (2, false),
            },
            _ => (1, false),
        };
        let digits = |b: u8| {
            if from == 1 {
                b.is_ascii_alphanumeric()
            } else if hex {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        };
        let mut n = from;
        while n < rest.len() && digits(rest[n]) {
            n += 1;
        }
        let count = n - from;
        let ok = match (from, hex) {
            // `(&#[xX])[01]?\h{1,5}(;)`
            (3, true) => (1..=6).contains(&count),
            // `(&#)[0-9]{1,7}(;)`
            (2, false) => (1..=7).contains(&count),
            // `(&)[a-zA-Z0-9]+(;)`
            _ => count >= 1,
        };
        (ok && rest.get(n) == Some(&b';')).then_some(n + 1)
    }

    /// The length of an entity with a doubled `#` (`&##x41;`, `&##65;`), the
    /// way one is written where `#…#` is read, if one is ahead.
    pub(super) fn hash_entity_len(&self) -> Option<usize> {
        let rest = self.rest().as_bytes();
        if !rest.starts_with(b"&##") {
            return None;
        }
        let (from, hex) = match rest.get(3) {
            Some(b'x') | Some(b'X') => (4, true),
            _ => (3, false),
        };
        let mut n = from;
        while n < rest.len()
            && (if hex {
                rest[n].is_ascii_hexdigit()
            } else {
                rest[n].is_ascii_digit()
            })
        {
            n += 1;
        }
        (n > from && rest.get(n) == Some(&b';')).then_some(n + 1)
    }

    /// Emit an entity as `Text` tokens: the opener (`&`, `&#`, `&#x`), the
    /// name or digits, and `;`. In a doubled-hash entity the `##` is an
    /// `EscapeHash` of its own.
    pub(super) fn entity(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        if ctx.hash() {
            if let Some(len) = self.hash_entity_len() {
                let start = self.pos;
                let hex = matches!(self.src.as_bytes().get(start + 3), Some(b'x') | Some(b'X'));
                // `&`, the `##`, then the `x` of a hex entity.
                self.take(out, 1, TokenKind::Text);
                self.take(out, 2, TokenKind::Literal(Literal::EscapeHash));
                if hex {
                    self.take(out, 1, TokenKind::Text);
                }
                let digits = start + len - 1 - self.pos;
                self.take(out, digits, TokenKind::Text);
                self.take(out, 1, TokenKind::Text);
                return;
            }
        }
        let Some(len) = self.entity_len() else { return };
        let rest = self.rest().as_bytes();
        let open = match rest.get(1) {
            Some(b'#') => match rest.get(2) {
                Some(b'x') | Some(b'X') => 3,
                _ => 2,
            },
            _ => 1,
        };
        self.take(out, open, TokenKind::Text);
        self.take(out, len - open - 1, TokenKind::Text);
        self.take(out, 1, TokenKind::Text);
    }

    // -----------------------------------------------------------------------
    // Comment, doctype, CDATA, `<?xml`
    // -----------------------------------------------------------------------

    /// `<!-- … -->`: bare tokens, not an element, so the CF tags and `#…#`
    /// inside it are real children of whatever holds the comment.
    fn html_comment(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        // `<!-->` / `<!--->`: the `>` or `->` right after the opener is
        // `invalid`.
        let after = &self.rest()[4..];
        let bad = if after.starts_with("->") {
            2
        } else if after.starts_with('>') {
            1
        } else {
            0
        };
        self.take(out, 4, TokenKind::Punct(Punct::Open(Delim::Comment)));
        if bad > 0 {
            self.take(out, bad, TokenKind::Invalid);
        }
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            if rest.starts_with("-->") {
                break;
            }
            if rest.starts_with("<!-") && self.at(self.pos + 3).starts_with("-->") {
                break;
            }
            if rest.starts_with("<!---") {
                self.region(out, text..self.pos, TokenKind::CommentText);
                self.cf_comment(out);
                text = self.pos;
                continue;
            }
            if rest.starts_with('<') && self.cf_tag_ahead() {
                self.region(out, text..self.pos, TokenKind::CommentText);
                self.cf_tag(out);
                text = self.pos;
                continue;
            }
            if rest.starts_with('#') && ctx.hash() {
                self.region(out, text..self.pos, TokenKind::CommentText);
                self.template_expression(out);
                text = self.pos;
                continue;
            }
            if self.bump() == '\n' {
                self.region(out, text..self.pos, TokenKind::CommentText);
                text = self.pos;
            }
        }
        self.region(out, text..self.pos, TokenKind::CommentText);
        if self.rest().starts_with("<!-") && self.at(self.pos + 3).starts_with("-->") {
            self.take(out, 3, TokenKind::Invalid);
        }
        if self.rest().starts_with("-->") {
            self.take(out, 3, TokenKind::Punct(Punct::Close(Delim::Comment)));
        }
    }

    /// `<!DOCTYPE …>`: a `Doctype` element with `<!` / `>`.
    fn doctype(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        let start = self.pos;
        let mut el = self.element(ElementKind::Doctype, 0..0);
        el.open = Some(self.token(start..start + 2, TokenKind::Punct(Punct::Open(Delim::Tag))));
        self.pos = start + 2;
        self.take(&mut el.children, 7, TokenKind::Ident(Ident::TagName));
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b == b'>' {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                let from = self.pos;
                self.pos += 1;
                el.close =
                    Some(self.token(from..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
                break;
            }
            if b == b'"' || b == b'\'' {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                self.doctype_string(&mut el.children, b);
                text = self.pos;
                continue;
            }
            if b == b'[' {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                self.doctype_subset(&mut el.children, ctx);
                text = self.pos;
                continue;
            }
            self.bump();
        }
        if el.close.is_none() {
            self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
        }
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }

    /// The doctype's internal subset, `[ … ]`, with HTML comments, CF
    /// comments, CF tags and `#…#` read inside.
    fn doctype_subset(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        self.take(out, 1, TokenKind::Text);
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b == b']' {
                break;
            }
            if b == b'<' && rest.starts_with("<!--") && !rest.starts_with("<!---") {
                self.flush_meta(out, &mut text, TokenKind::Text);
                self.html_comment(out, ctx);
                text = self.pos;
                continue;
            }
            if b == b'<' && rest.starts_with("<!---") {
                self.flush_meta(out, &mut text, TokenKind::Text);
                self.cf_comment(out);
                text = self.pos;
                continue;
            }
            if b == b'<' && self.cf_tag_ahead() {
                self.flush_meta(out, &mut text, TokenKind::Text);
                self.cf_tag(out);
                text = self.pos;
                continue;
            }
            if b == b'#' && ctx.hash() {
                self.flush_meta(out, &mut text, TokenKind::Text);
                self.template_expression(out);
                text = self.pos;
                continue;
            }
            self.bump();
        }
        self.flush_meta(out, &mut text, TokenKind::Text);
        if self.rest().starts_with(']') {
            self.take(out, 1, TokenKind::Text);
        }
    }

    fn doctype_string(&mut self, out: &mut Vec<Node>, quote: u8) {
        let start = self.pos;
        let mut el = self.element(
            ElementKind::String {
                quote: if quote == b'"' {
                    Quote::Double
                } else {
                    Quote::Single
                },
                in_tag: false,
            },
            0..0,
        );
        el.open = Some(self.token(
            start..start + 1,
            TokenKind::Punct(Punct::Open(Delim::String)),
        ));
        self.pos = start + 1;
        let text = self.pos;
        while self.pos < self.end && self.rest().as_bytes()[0] != quote {
            self.bump();
        }
        self.push(
            &mut el.children,
            text..self.pos,
            TokenKind::Literal(Literal::StringText),
        );
        if self.rest().as_bytes().first() == Some(&quote) {
            let from = self.pos;
            self.pos += 1;
            el.close = Some(self.token(
                from..self.pos,
                TokenKind::Punct(Punct::Close(Delim::String)),
            ));
        }
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }

    /// `<![CDATA[ … ]]>`: bare tokens.
    fn cdata(&mut self, out: &mut Vec<Node>) {
        self.take(out, 3, TokenKind::Punct(Punct::Open(Delim::Tag)));
        self.take(out, 5, TokenKind::Text);
        self.take(out, 1, TokenKind::Punct(Punct::Open(Delim::Tag)));
        let text = self.pos;
        while self.pos < self.end && !self.rest().starts_with("]]>") {
            self.bump();
        }
        self.push(out, text..self.pos, TokenKind::Literal(Literal::Unquoted));
        if self.rest().starts_with("]]>") {
            self.take(out, 3, TokenKind::Punct(Punct::Close(Delim::Tag)));
        }
    }

    /// `<?xml …?>`: an `HtmlTag(Open)` with `<?` / `?>`.
    fn preprocessor(&mut self, out: &mut Vec<Node>) {
        let start = self.pos;
        let mut el = self.element(ElementKind::HtmlTag(TagShape::Open), 0..0);
        el.open = Some(self.token(start..start + 2, TokenKind::Punct(Punct::Open(Delim::Tag))));
        self.pos = start + 2;
        let bytes = self.src.as_bytes();
        let mut n = self.pos;
        while n < self.end && !is_tag_name_break(bytes[n]) && bytes[n] != b'?' {
            n += 1;
        }
        self.take(
            &mut el.children,
            n - self.pos,
            TokenKind::Ident(Ident::TagName),
        );
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if rest.starts_with("?>") || b == b'>' {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
                let len = if b == b'>' { 1 } else { 2 };
                let from = self.pos;
                self.pos += len;
                el.close =
                    Some(self.token(from..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
                break;
            }
            if is_space(b) {
                self.pos += 1;
                continue;
            }
            if b == b'=' || b == b'/' {
                self.pos += 1;
                continue;
            }
            self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
            self.attribute(&mut el.children, Ctx::Html);
            text = self.pos;
        }
        if el.close.is_none() {
            self.flush_meta(&mut el.children, &mut text, TokenKind::Text);
        }
        crate::key_value::attributes(&mut el.children, false);
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }
}

/// Move the first `Close(String)` child into `close`, or, when content
/// follows it, leave it among the children in source order. Returns
/// whether one was found at all.
fn settle_string(el: &mut crate::tree::Element) -> bool {
    let Some(i) = el.children.iter().position(|n| {
        matches!(n, Node::Token(t)
            if t.kind == TokenKind::Punct(Punct::Close(Delim::String)))
    }) else {
        return false;
    };
    let Node::Token(close) = el.children.remove(i) else {
        unreachable!("position found a token")
    };
    let content_end = el.children.iter().map(|n| n.span().end).max().unwrap_or(0);
    if content_end > close.span.start {
        let at = el
            .children
            .partition_point(|n| n.span().start < close.span.start);
        el.children.insert(at, Node::Token(close));
    } else {
        el.close = Some(close);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{attr_value_kind, AttrRead, EVENT_ATTRIBUTES};

    #[test]
    fn event_attributes_are_sorted_lower_case_for_the_binary_search() {
        assert!(EVENT_ATTRIBUTES.windows(2).all(|w| w[0] < w[1]));
        assert!(EVENT_ATTRIBUTES
            .iter()
            .all(|e| e.starts_with("on") && !e.bytes().any(|b| b.is_ascii_uppercase())));
    }

    #[test]
    fn attribute_names_match_in_any_case() {
        for (name, kind) in [
            ("onclick", AttrRead::Event),
            ("onClick", AttrRead::Event),
            ("ONWAITING", AttrRead::Event),
            ("onabort", AttrRead::Event),
            ("on", AttrRead::Generic),
            ("onclickx", AttrRead::Generic),
            ("one", AttrRead::Generic),
            ("Class", AttrRead::Split),
            ("ID", AttrRead::Split),
            ("HREF", AttrRead::Path),
            ("Src", AttrRead::Path),
            ("STYLE", AttrRead::Style),
            ("data-x", AttrRead::Generic),
        ] {
            assert!(attr_value_kind(name) == kind, "{name}");
        }
    }
}
