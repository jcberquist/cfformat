//! The CFML side: `<!--- --->` comments and `cfformat-ignore` regions, the
//! generic / custom / extension tags, the five bespoke tags, `#…#` template
//! expressions and every hand-off to
//! [`script::parse_fragment`](crate::script::parse_fragment).

use crate::script::{self, Fragment};
use crate::tree::{
    CfKind, Delim, Element, ElementKind, Ident, Island, IslandSite, Lang, Literal, Node, Punct,
    Quote, Storage, TagShape, TokenKind,
};

use super::scanner::{starts_ci, word_ci, ContentStop, Ctx, Scanner};
use crate::scan::{closes_tag, tag_marker, Marker};

/// The opening tags read by name before any other rule, with their kind. A
/// `\b` must follow the name, as after `<cfcomponent>` / `<cfinterface>`:
/// `<cfoutputs>` is the generic tag `cfoutputs`, as its closing tag
/// `</cfoutputs>` is, and `<cfset2>` a generic tag too, not a `<cfset>`.
const NAMED: [(&str, CfKind); 12] = [
    ("cffunction", CfKind::Function),
    ("cfoutput", CfKind::Output),
    ("cfmail", CfKind::Output),
    ("cfquery", CfKind::Query),
    ("cfproperty", CfKind::Property),
    // A generic tag, listed so that `<cfargument:x>` is not read as a
    // `prefix:name` custom tag.
    ("cfargument", CfKind::Generic),
    ("cfscript", CfKind::Script),
    ("cfset", CfKind::Expression),
    ("cfreturn", CfKind::Expression),
    ("cfif", CfKind::Expression),
    ("cfelseif", CfKind::ElseIf),
    ("cfjava", CfKind::Java),
];

/// The kind of a generic tag name (`cf` and letters): `<cfelse>`, which is
/// not in [`NAMED`] so that `<cfelse:x>` stays a custom tag, and a closing
/// tag, which takes the kind of an opening tag of its name. An opening tag
/// named like a [`NAMED`] one here failed its `\b` and is `Generic`.
fn generic_kind(name: &str, closing: bool) -> CfKind {
    if name.eq_ignore_ascii_case("cfelse") {
        return CfKind::Else;
    }
    if !closing {
        return CfKind::Generic;
    }
    NAMED
        .iter()
        .find(|(word, _)| name.eq_ignore_ascii_case(word))
        .map_or(CfKind::Generic, |&(_, kind)| kind)
}

/// An opening or closing CFML tag recognised at the current position.
#[derive(Clone, Copy, Debug)]
pub(super) struct CfTagAt {
    pub(super) kind: CfKind,
    pub(super) closing: bool,
    /// `<` or `</`.
    pub(super) open: usize,
    /// Bytes of the tag name after the opening delimiter.
    pub(super) name: usize,
    /// Offset of the `:` inside the name, for a `<prefix:tag>` custom tag.
    pub(super) prefix: Option<usize>,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `[A-Za-z0-9\-_]*`: the name after `cf_`, `cfx_` or a `prefix:`. Digits
/// are name characters, so `<cf_foo2>` is one name, not cut at the digit.
fn custom_len(text: &str, from: usize) -> usize {
    text.as_bytes()[from..]
        .iter()
        .position(|&b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .unwrap_or(text.len() - from)
}

/// A generic CF tag name: `cf` and one or more ASCII letters.
fn generic_len(text: &str) -> Option<usize> {
    if !starts_ci(text, "cf") {
        return None;
    }
    let n = text.as_bytes()[2..]
        .iter()
        .position(|&b| !b.is_ascii_alphabetic())
        .unwrap_or(text.len() - 2);
    (n > 0).then_some(2 + n)
}

/// A `prefix:tag` custom tag name: ASCII letters, `:`, then a name with
/// digits allowed ([`custom_len`]).
fn prefixed_len(text: &str) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let head = b.iter().position(|&c| !c.is_ascii_alphabetic())?;
    if head == 0 || b.get(head) != Some(&b':') {
        return None;
    }
    let tail = custom_len(text, head + 1);
    Some((head, head + 1 + tail))
}

impl Scanner<'_> {
    /// The CFML tag opening or closing at the current position, if any.
    pub(super) fn cf_tag_at(&self) -> Option<CfTagAt> {
        let rest = self.rest();
        if !rest.starts_with('<') {
            return None;
        }
        let closing = rest.as_bytes().get(1) == Some(&b'/');
        let open = if closing { 2 } else { 1 };
        let after = &rest[open.min(rest.len())..];
        let make = |kind, name, prefix| {
            Some(CfTagAt {
                kind,
                closing,
                open,
                name,
                prefix,
            })
        };
        // A closing tag is `</name\s*>`, whitespace allowed before the `>`
        // ([`closes_tag`]).
        let closed = |n: usize| {
            after[n.min(after.len())..]
                .trim_start_matches(|c: char| c.is_ascii_whitespace())
                .starts_with('>')
        };

        if !closing {
            for (word, kind) in NAMED {
                if word_ci(after, word) {
                    return make(kind, word.len(), None);
                }
            }
        }
        if starts_ci(after, "cf_") {
            let n = 3 + custom_len(after, 3);
            if !closing || closed(n) {
                return make(CfKind::Generic, n, None);
            }
        }
        if !closing {
            if let Some((colon, n)) = prefixed_len(after) {
                return make(CfKind::Generic, n, Some(colon));
            }
        } else if let Some((_, n)) = prefixed_len(after) {
            if closed(n) {
                return make(CfKind::Generic, n, None);
            }
        }
        if starts_ci(after, "cfx_") {
            let n = 4 + custom_len(after, 4);
            if !closing || closed(n) {
                return make(CfKind::Generic, n, None);
            }
        }
        if let Some(n) = generic_len(after) {
            if !closing || closed(n) {
                return make(generic_kind(&after[..n], closing), n, None);
            }
        }
        None
    }

    pub(super) fn cf_tag_ahead(&self) -> bool {
        self.cf_tag_at().is_some()
    }

    /// An opening `<cffunction …>` ahead: the only tag an interface body
    /// reads.
    pub(super) fn cf_function_ahead(&self) -> bool {
        self.cf_tag_at()
            .is_some_and(|t| t.kind == CfKind::Function && !t.closing)
    }

    // -----------------------------------------------------------------------
    // Comments and ignore regions
    // -----------------------------------------------------------------------

    /// `<!--- … --->`, nested, or a `cfformat-ignore` region.
    pub(super) fn cf_comment(&mut self, out: &mut Vec<Node>) {
        let start = self.pos;
        if let Some(len) = tag_marker(self.at(start), Marker::Start) {
            let mut at = start + len;
            while at < self.end {
                if let Some(len) = tag_marker(self.at(at), Marker::End) {
                    at += len;
                    break;
                }
                at += self.src[at..].chars().next().map_or(1, char::len_utf8);
            }
            self.pos = at.min(self.end);
            let mut el = self.element(ElementKind::Ignore, start as u32..self.pos as u32);
            el.children = vec![self.node(start..self.pos, TokenKind::Ignore)];
            out.push(Node::Element(Box::new(el)));
            return;
        }

        // At the bound a nested `<!--- --->` is comment *text* rather than
        // another element. `Scanner::too_deep`'s cut cannot be used here:
        // it would leave every enclosing comment without its `--->`, and the
        // printer writes one for a comment the source never closed
        // (`print::tag_comment`), so each run would add a level and a format
        // would never settle — measured at +100 lines per run on a 101-deep
        // file. Not nesting keeps the closers the source has, so the tree
        // stays bounded and the output is a fixed point.
        self.depth += 1;
        let nest = self.depth < crate::MAX_DEPTH;
        let mut el = self.element(ElementKind::BlockComment, 0..0);
        el.open = Some(self.token(
            start..start + 5,
            TokenKind::Punct(Punct::Open(Delim::TagComment)),
        ));
        self.pos = start + 5;
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            if rest.starts_with("--->") {
                self.flush_comment(&mut el.children, &mut text);
                let from = self.pos;
                self.pos += 4;
                el.close = Some(self.token(
                    from..self.pos,
                    TokenKind::Punct(Punct::Close(Delim::TagComment)),
                ));
                break;
            }
            if nest && rest.starts_with("<!---") {
                self.flush_comment(&mut el.children, &mut text);
                self.cf_comment(&mut el.children);
                text = self.pos;
                continue;
            }
            if self.bump() == '\n' {
                self.flush_comment(&mut el.children, &mut text);
            }
        }
        if el.close.is_none() {
            self.flush_comment(&mut el.children, &mut text);
        }
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
        self.depth -= 1;
    }

    /// Comment text: whitespace keeps its kind, everything else is
    /// `CommentText`.
    fn flush_comment(&self, out: &mut Vec<Node>, from: &mut usize) {
        self.region(out, *from..self.pos, TokenKind::CommentText);
        *from = self.pos;
    }

    // -----------------------------------------------------------------------
    // Template expressions
    // -----------------------------------------------------------------------

    /// `##` or a `#…#` whose body goes to the script front end.
    pub(super) fn template_expression(&mut self, out: &mut Vec<Node>) {
        if self.rest().starts_with("##") {
            self.take(out, 2, TokenKind::Literal(Literal::EscapeHash));
            return;
        }
        let start = self.pos;
        let mut el = self.element(ElementKind::TemplateExpression, 0..0);
        el.open = Some(self.token(
            start..start + 1,
            TokenKind::Punct(Punct::Open(Delim::Template)),
        ));
        self.pos = start + 1;
        let body_end = self.hash_end(self.pos);
        el.children = self.script_nodes(self.pos..body_end, Fragment::Expression);
        self.pos = body_end;
        if self.rest().starts_with('#') {
            let from = self.pos;
            self.pos += 1;
            el.close = Some(self.token(
                from..self.pos,
                TokenKind::Punct(Punct::Close(Delim::Template)),
            ));
        }
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }

    /// Hand a script span to the script front end.
    pub(super) fn script_nodes(
        &mut self,
        span: std::ops::Range<usize>,
        kind: Fragment,
    ) -> Vec<Node> {
        if span.start >= span.end {
            return Vec::new();
        }
        script::parse_fragment_at(
            self.src,
            span.start as u32..span.end as u32,
            kind,
            self.depth,
        )
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    /// The CFML tag at the current position, as an element in `out`.
    pub(super) fn cf_tag(&mut self, out: &mut Vec<Node>) {
        let Some(info) = self.cf_tag_at() else {
            return;
        };
        let start = self.pos;
        if info.closing {
            self.close_tag(out, info);
            return;
        }
        match info.kind {
            CfKind::Script => self.cf_script(out, info),
            CfKind::Expression | CfKind::ElseIf => self.cf_expression(out, info),
            CfKind::Output => self.cf_output(out, info),
            CfKind::Query => self.cf_query(out, info),
            CfKind::Java => self.cf_java(out, info),
            CfKind::Function => {
                let mut el = self.open_tag(info);
                self.cf_tag_body(&mut el, info.kind, true);
                let opened = el.close.is_some();
                self.finish_tag(out, el, start);
                if !opened {
                    return;
                }
                // A `<cffunction>` body is read as HTML content, so HTML tags
                // are tags there even inside a `<cfcomponent>`.
                self.split_body_start(out, Ctx::Html);
                self.content(out, Ctx::Html, ContentStop::CfFunction);
                if let Some(info) = self.cf_tag_at().filter(|t| t.closing) {
                    self.close_tag(out, info);
                }
            }
            CfKind::Property | CfKind::Else | CfKind::Class | CfKind::Generic => {
                let mut el = self.open_tag(info);
                self.cf_tag_body(&mut el, info.kind, false);
                self.finish_tag(out, el, start);
            }
        }
    }

    /// A CFML tag inside an element that allows no children (an HTML
    /// attribute value): its tokens, `#…#` kept whole.
    pub(super) fn cf_tag_flat(&mut self, out: &mut Vec<Node>) {
        let mut nodes = Vec::new();
        let flat = std::mem::replace(&mut self.flat, true);
        self.cf_tag(&mut nodes);
        self.flat = flat;
        for node in nodes {
            super::scanner::flatten_to_templates(self.src, node, out);
        }
    }

    /// `</cfname>`: the delimiters and the name.
    fn close_tag(&mut self, out: &mut Vec<Node>, info: CfTagAt) {
        let start = self.pos;
        let mut el = self.element(ElementKind::CfTag(TagShape::Close, info.kind), 0..0);
        el.open = Some(self.token(
            start..start + info.open,
            TokenKind::Punct(Punct::Open(Delim::Tag)),
        ));
        self.pos = start + info.open;
        self.take(
            &mut el.children,
            info.name,
            TokenKind::Ident(Ident::TagName),
        );
        // `</name >`: the whitespace before `>`.
        let gt = self.pos + self.rest().find('>').unwrap_or(0);
        self.region(&mut el.children, self.pos..gt, TokenKind::Whitespace);
        self.pos = gt + 1;
        el.close = Some(self.token(gt..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
        el.span = start as u32..self.pos as u32;
        out.push(Node::Element(Box::new(el)));
    }

    /// The opening delimiter and the tag name of an opening tag.
    fn open_tag(&mut self, info: CfTagAt) -> Element {
        let start = self.pos;
        let mut el = self.element(ElementKind::CfTag(TagShape::Open, info.kind), 0..0);
        el.open = Some(self.token(
            start..start + info.open,
            TokenKind::Punct(Punct::Open(Delim::Tag)),
        ));
        self.pos = start + info.open;
        let name = TokenKind::Ident(Ident::TagName);
        match info.prefix {
            Some(colon) => {
                self.take(&mut el.children, colon, name);
                self.take(&mut el.children, 1, TokenKind::Punct(Punct::Prefix));
                self.take(&mut el.children, info.name - colon - 1, name);
            }
            None => self.take(&mut el.children, info.name, name),
        }
        el
    }

    /// Attributes and CF comments until the closing delimiter. `strict` tags
    /// (`cffunction`, `cfoutput`, `cfquery`, `cfscript`, …) close on `>`
    /// only, the others on `/>` too.
    fn cf_tag_body(&mut self, el: &mut Element, kind: CfKind, strict: bool) {
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b == b'>' || (!strict && rest.starts_with("/>")) {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
                let len = if b == b'>' { 1 } else { 2 };
                let from = self.pos;
                self.pos += len;
                el.close =
                    Some(self.token(from..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
                return;
            }
            if rest.starts_with("<!---") {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
                self.cf_comment(&mut el.children);
                text = self.pos;
                continue;
            }
            if b.is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }
            if let Some(len) = self.cf_attribute_name_len() {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
                self.cf_attribute(&mut el.children, kind, len);
                text = self.pos;
                continue;
            }
            // Any other non-space character is one `invalid` token.
            self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
            self.bump();
            self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
        }
        self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
    }

    /// `\b[_[:alpha:]][[:alnum:]_\-:]*\b` at the current position.
    fn cf_attribute_name_len(&self) -> Option<usize> {
        let rest = self.rest().as_bytes();
        let first = *rest.first()?;
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        // The leading `\b` holds: the previous byte is whitespace, `>` or a
        // delimiter, never a word character, at every call site.
        if self.pos > 0 && is_word(self.src.as_bytes()[self.pos - 1]) {
            return None;
        }
        let greedy = rest
            .iter()
            .position(|&b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b':'))
            .unwrap_or(rest.len());
        // The trailing `\b`: the longest prefix with a word boundary after it.
        (1..=greedy)
            .rev()
            .find(|&n| is_word(rest[n - 1]) && rest.get(n).is_none_or(|&b| !is_word(b)))
    }

    fn cf_attribute(&mut self, out: &mut Vec<Node>, kind: CfKind, len: usize) {
        let start = self.pos;
        let src = self.src;
        let name = &src[start..start + len];
        self.take(out, len, TokenKind::Ident(Ident::AttributeName));
        let mut from = self.pos;
        while self.pos < self.end && self.rest().as_bytes()[0].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.rest().as_bytes().first() != Some(&b'=') {
            self.pos = from;
            return;
        }
        self.flush_meta(out, &mut from, TokenKind::Invalid);
        self.take(out, 1, TokenKind::Punct(Punct::KeyValue));
        let value = bespoke_value(kind, name);
        let mut from = self.pos;
        while self.pos < self.end && self.rest().as_bytes()[0].is_ascii_whitespace() {
            self.pos += 1;
        }
        self.flush_meta(out, &mut from, TokenKind::Invalid);
        let Some(&b) = self.rest().as_bytes().first() else {
            return;
        };
        match b {
            b'"' | b'\'' => self.cf_string(out, b, value.quoted()),
            // An unquoted value cannot start with `<`, `/`, `>`, `{` or `;`:
            // the attribute has no value.
            b'<' | b'/' | b'>' | b'{' | b';' => (),
            _ => self.cf_unquoted_value(out, value),
        }
    }

    /// A quoted CFML attribute value.
    fn cf_string(&mut self, out: &mut Vec<Node>, quote: u8, value: Bespoke) {
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
        if value == Bespoke::ReturnType {
            self.return_type(&mut el.children, true);
        }
        let mut text = self.pos;
        // The quoted form of a bespoke value ([`Bespoke::Named`]) escapes
        // `''` whatever the quote is and reads no `#…#`; any other value
        // escapes its own quote, doubled. The first lone quote ends the
        // string.
        let escape: &[u8] = match value {
            Bespoke::Named => b"''",
            _ if quote == b'"' => b"\"\"",
            _ => b"''",
        };
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if rest.as_bytes().starts_with(escape) {
                self.flush_string(&mut el.children, &mut text);
                self.take(
                    &mut el.children,
                    2,
                    TokenKind::Literal(Literal::EscapeQuote),
                );
                text = self.pos;
                continue;
            }
            if b == quote {
                break;
            }
            if b == b'#' && value != Bespoke::Named {
                self.flush_string(&mut el.children, &mut text);
                if rest.starts_with("##") {
                    self.take(&mut el.children, 2, TokenKind::Literal(Literal::EscapeHash));
                } else {
                    self.template_expression(&mut el.children);
                }
                text = self.pos;
                continue;
            }
            if self.bump() == '\n' {
                self.flush_string(&mut el.children, &mut text);
            }
        }
        self.flush_string(&mut el.children, &mut text);
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

    /// Every token inside a CFML string is `lit.string`, whitespace
    /// included.
    fn flush_string(&self, out: &mut Vec<Node>, from: &mut usize) {
        // Inside a CFML string every token is `lit.string`, whitespace
        // included, one per line.
        let mut at = *from;
        while at < self.pos {
            let line = self.src[at..self.pos]
                .split_inclusive('\n')
                .next()
                .unwrap_or("");
            out.push(self.node(at..at + line.len(), TokenKind::Literal(Literal::StringText)));
            at += line.len();
        }
        *from = self.pos;
    }

    /// An unquoted attribute value: up to whitespace or one of `<`, `/`, `>`,
    /// `{`, `;`, with `#…#` read inside.
    fn cf_unquoted_value(&mut self, out: &mut Vec<Node>, value: Bespoke) {
        let kind = match value {
            Bespoke::Name => TokenKind::Ident(Ident::FunctionName),
            Bespoke::Access => TokenKind::Storage(Storage::Modifier),
            Bespoke::Extends => TokenKind::Ident(Ident::ClassName),
            Bespoke::PropertyName => TokenKind::Ident(Ident::PropertyName),
            _ => TokenKind::Literal(Literal::Unquoted),
        };
        if value == Bespoke::ReturnType {
            self.return_type(out, false);
        }
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b.is_ascii_whitespace() || matches!(b, b'<' | b'/' | b'>' | b'{' | b';') {
                break;
            }
            if b == b'#' {
                self.region(out, text..self.pos, kind);
                if rest.starts_with("##") {
                    self.take(out, 2, TokenKind::Literal(Literal::EscapeHash));
                } else {
                    self.template_expression(out);
                }
                text = self.pos;
                continue;
            }
            self.bump();
        }
        self.region(out, text..self.pos, kind);
    }

    /// The type at the start of a `returntype` value, read once: a primitive
    /// type name, `function` or `void`, else a dotted name with an optional
    /// `[]`. The rest of the value is read as any other.
    fn return_type(&mut self, out: &mut Vec<Node>, quoted: bool) {
        const PRIMITIVES: &[&str] = &[
            "any",
            "array",
            "binary",
            "boolean",
            "component",
            "date",
            "guid",
            "numeric",
            "query",
            "string",
            "struct",
            "xml",
            "uuid",
        ];
        let kind = |k: TokenKind| {
            if quoted {
                TokenKind::Literal(Literal::StringText)
            } else {
                k
            }
        };
        let rest = self.rest();
        let word = rest
            .as_bytes()
            .iter()
            .position(|&b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'.'))
            .unwrap_or(rest.len());
        if word == 0 {
            return;
        }
        let text = &rest[..word];
        let boundary = |n: usize| {
            is_word(rest.as_bytes()[n - 1]) && rest.as_bytes().get(n).is_none_or(|&b| !is_word(b))
        };
        for name in PRIMITIVES.iter().chain(["function", "void"].iter()) {
            if word_ci(rest, name) && boundary(name.len()) {
                self.take(out, name.len(), kind(TokenKind::Storage(Storage::Type)));
                return;
            }
        }
        // Any other name (`[_$[:alnum:]][_$[:alnum:].]*`) up to its last
        // word boundary, then `[]` when it follows.
        if text.as_bytes()[0].is_ascii_alphanumeric()
            || text.as_bytes()[0] == b'_'
            || text.as_bytes()[0] == b'$'
        {
            if let Some(n) = (1..=word).rev().find(|&n| boundary(n)) {
                if rest[n..].starts_with("[]") {
                    self.take(out, n, kind(TokenKind::Storage(Storage::Type)));
                    self.take(out, 1, kind(TokenKind::Punct(Punct::Open(Delim::Bracket))));
                    self.take(out, 1, kind(TokenKind::Punct(Punct::Close(Delim::Bracket))));
                } else {
                    self.take(out, n, kind(TokenKind::Storage(Storage::Type)));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tags with bodies
    // -----------------------------------------------------------------------

    /// `<cfscript> … </cfscript>`: the body goes over as `Statements`.
    fn cf_script(&mut self, out: &mut Vec<Node>, info: CfTagAt) {
        let start = self.pos;
        let mut el = self.open_tag(info);
        // `<cfscript>` takes no attributes: every non-space character
        // before the `>` is one `invalid` token.
        let mut text = self.pos;
        while self.pos < self.end {
            let b = self.rest().as_bytes()[0];
            if b == b'>' {
                self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
                let from = self.pos;
                self.pos += 1;
                el.close =
                    Some(self.token(from..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
                text = self.pos;
                break;
            }
            if b.is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }
            self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
            self.bump();
            self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
        }
        if el.close.is_none() {
            self.flush_meta(&mut el.children, &mut text, TokenKind::Invalid);
        }
        self.finish_tag(out, el, start);
        if self.pos >= self.end {
            return;
        }
        let body_end = self.script_body_end();
        let nodes = self.script_nodes(self.pos..body_end, Fragment::Statements);
        out.extend(nodes);
        self.pos = body_end;
        if let Some(info) = self.cf_tag_at().filter(|t| t.closing) {
            self.close_tag(out, info);
        }
    }

    /// `<cfset>` / `<cfreturn>` / `<cfif>` / `<cfelseif>`.
    fn cf_expression(&mut self, out: &mut Vec<Node>, info: CfTagAt) {
        let start = self.pos;
        let mut el = self.open_tag(info);
        let (body_end, close) = self.angle_end();
        el.children
            .extend(self.script_nodes(self.pos..body_end, Fragment::TagExpression));
        self.pos = body_end;
        if close > 0 {
            let from = self.pos;
            self.pos += close;
            el.close = Some(self.token(from..self.pos, TokenKind::Punct(Punct::Close(Delim::Tag))));
        }
        self.finish_tag(out, el, start);
    }

    /// `<cfoutput>` / `<cfmail>`: the body reads `#…#` template expressions.
    fn cf_output(&mut self, out: &mut Vec<Node>, info: CfTagAt) {
        let start = self.pos;
        let mut el = self.open_tag(info);
        self.cf_tag_body(&mut el, info.kind, true);
        let opened = el.close.is_some();
        self.finish_tag(out, el, start);
        if !opened {
            return;
        }
        // Unlike a `<cffunction>` body's, the leading whitespace is not a
        // region of its own (no `split_body_start`): it joins the text after
        // it.
        self.content(out, Ctx::Output, ContentStop::Output);
        if let Some(info) = self.cf_tag_at().filter(|t| t.closing) {
            self.close_tag(out, info);
        }
    }

    /// `<cfjava>`: a Java island. The body is one opaque run to the first
    /// `</cfjava>`, with no CF tag, CF comment or `#…#` read inside it; that
    /// `</cfjava>` is an ordinary closing tag.
    fn cf_java(&mut self, out: &mut Vec<Node>, info: CfTagAt) {
        let start = self.pos;
        let mut el = self.open_tag(info);
        self.cf_tag_body(&mut el, info.kind, true);
        let opened = el.close.is_some();
        self.finish_tag(out, el, start);
        if !opened {
            return;
        }
        let body = self.pos;
        while self.pos < self.end && closes_tag(self.rest(), "cfjava").is_none() {
            self.bump();
        }
        let mut island = self.element(
            ElementKind::Island(Island {
                lang: Lang::Java,
                site: IslandSite::CfJava,
                script_type: None,
            }),
            body as u32..self.pos as u32,
        );
        self.push(&mut island.children, body..self.pos, TokenKind::Text);
        out.push(Node::Element(Box::new(island)));
        if let Some(info) = self.cf_tag_at().filter(|t| t.closing) {
            self.close_tag(out, info);
        }
    }

    /// `<cfquery>`: a SQL island with CFML inside.
    fn cf_query(&mut self, out: &mut Vec<Node>, info: CfTagAt) {
        let start = self.pos;
        let mut el = self.open_tag(info);
        self.cf_tag_body(&mut el, info.kind, true);
        let opened = el.close.is_some();
        self.finish_tag(out, el, start);
        if !opened {
            return;
        }
        self.sql_island(out);
        if let Some(info) = self.cf_tag_at().filter(|t| t.closing) {
            self.close_tag(out, info);
        }
    }

    /// `main`'s `<cfcomponent>` / `<cfinterface>` path.
    pub(super) fn cf_class(&mut self, out: &mut Vec<Node>) {
        let start = self.pos;
        let interface = word_ci(self.rest(), "<cfinterface");
        let name = if interface {
            "cfinterface"
        } else {
            "cfcomponent"
        };
        let info = CfTagAt {
            kind: CfKind::Class,
            closing: false,
            open: 1,
            name: name.len(),
            prefix: None,
        };
        let mut el = self.open_tag(info);
        self.cf_tag_body(&mut el, info.kind, true);
        let opened = el.close.is_some();
        self.finish_tag(out, el, start);
        if !opened {
            return;
        }
        let (ctx, stop) = if interface {
            (Ctx::Interface, ContentStop::Interface)
        } else {
            (Ctx::Class, ContentStop::Class)
        };
        self.content(out, ctx, stop);
        if let Some(info) = self.cf_tag_at().filter(|t| t.closing) {
            // The closing tag of the body read as a class is one too.
            let kind = if self.at_stop(stop) {
                CfKind::Class
            } else {
                info.kind
            };
            self.close_tag(out, CfTagAt { kind, ..info });
        }
    }
}

/// How the value of an attribute of a bespoke tag is read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Bespoke {
    None,
    /// `cffunction name`: an unquoted value is a function name.
    Name,
    /// `cffunction access`: an unquoted value is a modifier.
    Access,
    /// `cffunction returntype`: a type name first (see `return_type`).
    ReturnType,
    /// `cfcomponent`/`cfinterface` `extends`.
    Extends,
    /// `cfproperty name`.
    PropertyName,
    /// The quoted forms of all of the above but `returntype`: `''` escapes
    /// whatever the quote, and no `#…#`.
    Named,
}

impl Bespoke {
    fn quoted(self) -> Bespoke {
        match self {
            Bespoke::None | Bespoke::ReturnType => self,
            _ => Bespoke::Named,
        }
    }
}

/// How the value of the attribute `name` (any case) of a `kind` tag is read.
fn bespoke_value(kind: CfKind, name: &str) -> Bespoke {
    let is = |attribute: &str| name.eq_ignore_ascii_case(attribute);
    match kind {
        CfKind::Function if is("name") => Bespoke::Name,
        CfKind::Function if is("access") => Bespoke::Access,
        CfKind::Function if is("returntype") => Bespoke::ReturnType,
        CfKind::Property if is("name") => Bespoke::PropertyName,
        CfKind::Class if is("extends") => Bespoke::Extends,
        _ => Bespoke::None,
    }
}
