//! Islands (`<script>`, `<style>`, `<cfquery>`, `style="…"`, `onclick="…"`)
//! and the script stubs: where a `<cfscript>` body, a tag's expression and a
//! `#…#` end, found before the script parser reads them.

use crate::tree::{
    Delim, Element, ElementKind, Ident, Island, IslandSite, Lang, Node, Punct, TagShape, TokenKind,
};

use super::html::is_tag_name_break;
use super::scanner::{starts_ci, Ctx, Scanner};
use crate::scan;

/// Where a `<script>` / `<style>` island's text ends, and what the escape
/// consumes after it.
struct Escape {
    /// Where the island's text ends (where the escape starts).
    end: usize,
    /// Bytes the escape consumes there, emitted as one region.
    consume: usize,
}

/// What the `type` attribute of a `<script>` or `<style>` selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScriptType {
    Js,
    Json,
    /// `text/html`: the body is ordinary HTML content.
    Html,
    /// Anything else, or a type only the server knows: the body is a
    /// [`Lang::Unknown`] island to the closing tag.
    Other,
}

/// An attribute's value as the scanner reads it
/// ([`Scanner::attribute_value_text`]).
#[derive(Clone, PartialEq, Eq, Debug)]
enum AttrValue {
    /// No such attribute.
    Absent,
    /// The value's text; empty for a bare attribute or `""`.
    Static(String),
    /// The value holds CFML (a `#…#`, a CF tag, a tag comment) or a token
    /// that is not text: only the server knows it.
    Dynamic,
}

fn mime_body(value: &str) -> &str {
    match value.find(';') {
        Some(n) => &value[..n],
        None => value,
    }
}

fn javascript_mime(value: &str) -> bool {
    let v = mime_body(value).to_ascii_lowercase();
    if let Some(rest) = v
        .strip_prefix("application/")
        .or_else(|| v.strip_prefix("text/"))
    {
        let rest = rest.strip_prefix("x-").unwrap_or(rest);
        if matches!(rest, "javascript" | "ecmascript") {
            return true;
        }
    }
    matches!(
        v.as_str(),
        "text/jscript" | "text/livescript" | "text/babel"
    ) || (v.starts_with("text/javascript1.")
        && v.len() == 17
        && matches!(&v[16..], "0" | "1" | "2" | "3" | "4" | "5"))
}

/// Whether `e`, standing directly in a tag, is CFML that can emit
/// attributes ([`Scanner::attribute_value_text`]).
fn emits_attributes(e: &Element) -> bool {
    matches!(
        e.kind,
        ElementKind::TemplateExpression
            | ElementKind::CfTag(..)
            | ElementKind::TagBody { cf: true }
    )
}

fn json_mime(value: &str) -> bool {
    let v = mime_body(value).to_ascii_lowercase();
    v == "importmap"
        || v == "speculationrules"
        || v == "application/json"
        || v == "text/json"
        || (v.ends_with("+json") && v.is_ascii())
}

impl Scanner<'_> {
    // -----------------------------------------------------------------------
    // `<script>` and `<style>`
    // -----------------------------------------------------------------------

    pub(super) fn script_tag(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        self.host_tag(out, ctx, true)
    }

    pub(super) fn style_tag(&mut self, out: &mut Vec<Node>, ctx: Ctx) {
        self.host_tag(out, ctx, false)
    }

    fn host_tag(&mut self, out: &mut Vec<Node>, ctx: Ctx, script: bool) {
        let start = self.pos;
        let name_len = if script { 6 } else { 5 };
        let mut el = self.element(ElementKind::HtmlTag(TagShape::Open), 0..0);
        el.open = Some(self.token(start..start + 1, TokenKind::Punct(Punct::Open(Delim::Tag))));
        self.pos = start + 1;
        self.take(&mut el.children, name_len, TokenKind::Ident(Ident::TagName));
        // The opening tag ends at `>` or at `/>`; a self-closed one has no
        // body.
        self.tag_body(&mut el, ctx, false);
        let self_closed = el
            .close
            .as_ref()
            .is_some_and(|t| &self.src[t.span.start as usize..t.span.end as usize] == "/>");
        let type_value = self.attribute_value_text(&el, "type");
        self.finish_tag(out, el, start);
        if self_closed || self.pos >= self.end {
            return;
        }

        // A dynamic `type` is a language only the server knows: raw text,
        // whatever the body.
        let kind = match (&type_value, script) {
            (AttrValue::Dynamic, _) => ScriptType::Other,
            (AttrValue::Absent, _) => ScriptType::Js,
            (AttrValue::Static(v), _) if v.is_empty() => ScriptType::Js,
            (AttrValue::Static(v), true)
                if javascript_mime(v) || v.eq_ignore_ascii_case("module") =>
            {
                ScriptType::Js
            }
            (AttrValue::Static(v), true) if json_mime(v) => ScriptType::Json,
            (AttrValue::Static(v), true) if mime_body(v).eq_ignore_ascii_case("text/html") => {
                ScriptType::Html
            }
            (AttrValue::Static(v), false) if mime_body(v).eq_ignore_ascii_case("text/css") => {
                ScriptType::Js
            }
            (AttrValue::Static(_), _) => ScriptType::Other,
        };
        let close = if script { "script" } else { "style" };

        let site = if script {
            IslandSite::ScriptTag
        } else {
            IslandSite::StyleTag
        };
        match kind {
            ScriptType::Html => (),
            ScriptType::Other => {
                // A type the scanner does not know, or one only the server
                // knows. The body is data, an island of no language, in which
                // CF tags, CF comments and `#…#` are still read.
                let script_type = match &type_value {
                    AttrValue::Static(v) => Some(v.trim().to_ascii_lowercase()),
                    _ => None,
                };
                self.opaque_island(out, ctx, site, script_type, close)
            }
            _ => {
                let lang = match (script, kind) {
                    (false, _) => Lang::Css,
                    (_, ScriptType::Json) => Lang::Json,
                    _ => Lang::Js,
                };
                let script_type = match site {
                    IslandSite::ScriptTag => match &type_value {
                        AttrValue::Static(v) => Some(v.as_str()),
                        _ => None,
                    }
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_ascii_lowercase),
                    _ => None,
                };
                self.host_island(out, ctx, lang, site, script_type, close)
            }
        }
    }

    /// The body of a `<script>` / `<style>` whose type names no language: a
    /// [`Lang::Unknown`] island of the text to the closing tag, built as
    /// the SQL island is — the text in [`TokenKind::Text`] runs, CF comments,
    /// CF tags and `#…#` (where the context reads them) as children. No
    /// `<!--` / CDATA wrapper and no escape: every byte between the opening
    /// tag's `>` and the closing tag's `</` is the island's.
    fn opaque_island(
        &mut self,
        out: &mut Vec<Node>,
        ctx: Ctx,
        site: IslandSite,
        script_type: Option<String>,
        close: &str,
    ) {
        let start = self.pos;
        let mut el = self.element(
            ElementKind::Island(Island {
                lang: Lang::Unknown,
                site,
                script_type,
            }),
            start as u32..start as u32,
        );
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            if self.close_tag_ahead(close) {
                break;
            }
            if rest.starts_with("<!---") {
                self.push(&mut el.children, text..self.pos, TokenKind::Text);
                self.cf_comment(&mut el.children);
                text = self.pos;
                continue;
            }
            if rest.starts_with('<') && self.cf_tag_ahead() {
                self.push(&mut el.children, text..self.pos, TokenKind::Text);
                self.nested(&mut el.children, ctx, |s, out| s.cf_tag(out));
                text = self.pos;
                continue;
            }
            if rest.starts_with('#') && ctx.hash() {
                self.push(&mut el.children, text..self.pos, TokenKind::Text);
                self.template_expression(&mut el.children);
                text = self.pos;
                continue;
            }
            self.bump();
        }
        self.push(&mut el.children, text..self.pos, TokenKind::Text);
        Scanner::coalesce_text(&mut el.children);
        el.span = start as u32..self.pos as u32;
        self.push_island(out, el);
    }

    /// Where a `<script>` / `<style>` island's text ends, and how many bytes
    /// the escape takes after it: a `-->` alone on its line, a `-->` right
    /// before the closing tag, or the whitespace that alone precedes the
    /// closing tag on its line.
    ///
    /// Every alternative is decided within a single line, and what the escape
    /// consumes becomes one region outside the island, not tokens of its own
    /// (the tree's token boundaries are part of its contract).
    fn host_escape(&self, from: usize, name: &str) -> Escape {
        let ws = |b: u8| matches!(b, b'\t' | b'\x0c' | b' ' | b'\r');
        let back = |mut at: usize, floor: usize| {
            while at > floor && ws(self.src.as_bytes()[at - 1]) {
                at -= 1;
            }
            at
        };
        let mut ls = from;
        while ls < self.end {
            let line = self.src[ls..self.end]
                .split_inclusive('\n')
                .next()
                .unwrap_or("");
            let le = ls + line.len();
            let eol = if line.ends_with('\n') { le - 1 } else { le };
            // A `-->` alone on its line (whitespace around it) ends it, and
            // the escape takes the whole line.
            if self.src[ls..eol].trim() == "-->" {
                return Escape {
                    end: ls,
                    consume: le - ls,
                };
            }
            if let Some(k) = self.close_tag_in(ls, le, name) {
                let j = back(k, ls);
                if j >= ls + 3 && &self.src.as_bytes()[j - 3..j] == b"-->" {
                    // `-->` before the closing tag, only whitespace between:
                    // the island ends before the whitespace in front of the
                    // `-->`, and the escape takes everything up to `</`.
                    let i = back(j - 3, ls);
                    return Escape {
                        end: i,
                        consume: k - i,
                    };
                }
                // Only whitespace before the closing tag on its line: the
                // escape takes that whitespace.
                if j == ls {
                    return Escape {
                        end: ls,
                        consume: k - ls,
                    };
                }
                return Escape { end: k, consume: 0 };
            }
            ls = le;
        }
        Escape {
            end: self.end,
            consume: 0,
        }
    }

    /// Offset of `</name` followed by a tag-name break inside `[ls, le)`.
    fn close_tag_in(&self, ls: usize, le: usize, name: &str) -> Option<usize> {
        let hay = &self.src[ls..le];
        let mut at = 0;
        while let Some(n) = hay[at..].find('<') {
            let p = at + n;
            let rest = &hay[p..];
            if starts_ci(rest, "</")
                && starts_ci(&rest[2.min(rest.len())..], name)
                && rest
                    .as_bytes()
                    .get(2 + name.len())
                    .is_some_and(|&b| is_tag_name_break(b))
            {
                return Some(ls + p);
            }
            at = p + 1;
        }
        None
    }

    fn close_tag_ahead(&self, name: &str) -> bool {
        let rest = self.rest();
        starts_ci(rest, "</")
            && starts_ci(&rest[2.min(rest.len())..], name)
            && rest
                .as_bytes()
                .get(2 + name.len())
                .is_some_and(|&b| is_tag_name_break(b))
    }

    /// The body of a `<script>` / `<style>` that is an island.
    fn host_island(
        &mut self,
        out: &mut Vec<Node>,
        ctx: Ctx,
        lang: Lang,
        site: IslandSite,
        script_type: Option<String>,
        close: &str,
    ) {
        self.body_start(out);
        let start = self.pos;
        let escape = self.host_escape(start, close);
        let mut el = self.element(
            ElementKind::Island(Island {
                lang,
                site,
                script_type,
            }),
            start as u32..start as u32,
        );
        let end = escape.end;
        self.scoped(end, |s| {
            s.island_body(&mut el.children, ctx, |s| s.pos >= end)
        });
        Scanner::coalesce_text(&mut el.children);
        el.span = start as u32..self.pos as u32;
        self.push_island(out, el);
        let from = self.pos;
        self.pos = (self.pos + escape.consume).min(self.end);
        self.region(out, from..self.pos, TokenKind::Text);
    }

    /// Where the body begins after the tag's `>`, and the `<!--` /
    /// `<![CDATA[` wrapper that can precede it on the tag's line.
    fn body_start(&mut self, out: &mut Vec<Node>) {
        // Anything but whitespace or a wrapper after the `>` on its line:
        // the body starts at the `>` itself, so there is nothing to emit.
        let rest = self.rest();
        let line = rest.split_inclusive('\n').next().unwrap_or("");
        let head = line.trim_start_matches([' ', '\t']);
        if let Some(c) = head.chars().next() {
            let opens_wrapper = head.starts_with("<!--") || head.starts_with("<![CDATA[");
            if (!c.is_whitespace() && c != '<') || (c == '<' && !opens_wrapper) {
                return;
            }
        }
        // Otherwise the line holds whitespace, then a CDATA wrapper, a
        // `<!--`, or nothing more.
        let ws = line.len() - line.trim_start().len();
        let after_ws = &line[ws..];
        if after_ws.starts_with("<![CDATA[") {
            let mut from = self.pos;
            self.pos += ws;
            self.flush_meta(out, &mut from, TokenKind::Text);
            self.take(out, 3, TokenKind::Punct(Punct::Open(Delim::Tag)));
            self.take(out, 5, TokenKind::Text);
            self.take(out, 1, TokenKind::Punct(Punct::Open(Delim::Tag)));
            return;
        }
        if after_ws.starts_with("<!--") {
            let mut from = self.pos;
            self.pos += ws;
            self.flush_meta(out, &mut from, TokenKind::Text);
            self.take(out, 4, TokenKind::Punct(Punct::Open(Delim::Comment)));
            // The rest of the `<!--` line too, when it is only whitespace.
            let tail = self.rest();
            let tail_line = tail.split_inclusive('\n').next().unwrap_or("");
            if tail_line.ends_with('\n') && tail_line[..tail_line.len() - 1].trim().is_empty() {
                let mut from = self.pos;
                self.pos += tail_line.len();
                self.flush_meta(out, &mut from, TokenKind::Text);
            }
            return;
        }
        // The rest of the line is whitespace: the body starts at its `\n`.
        if let Some(nl) = line.find('\n') {
            let mut from = self.pos;
            self.pos += nl;
            self.flush_meta(out, &mut from, TokenKind::Text);
            return;
        }
        // No newline left: nothing on this line opens the body. Its
        // whitespace is emitted like the line's tail above, so the tokens
        // tile `<script>   `.
        let mut from = self.pos;
        self.pos = self.end;
        self.flush_meta(out, &mut from, TokenKind::Text);
    }

    /// Island content: host text coalesced into one `Text` run between the
    /// CFML children that interrupt it.
    fn island_body(&mut self, out: &mut Vec<Node>, ctx: Ctx, stop: impl Fn(&Scanner<'_>) -> bool) {
        let mut text = self.pos;
        while self.pos < self.end {
            if stop(self) {
                break;
            }
            let rest = self.rest();
            let b = rest.as_bytes()[0];
            if b == b'<' && rest.starts_with("<!---") {
                self.push(out, text..self.pos, TokenKind::Text);
                self.cf_comment(out);
                text = self.pos;
                continue;
            }
            if b == b'<' && self.cf_tag_ahead() {
                self.push(out, text..self.pos, TokenKind::Text);
                self.nested(out, ctx, |s, out| s.cf_tag(out));
                text = self.pos;
                continue;
            }
            if b == b'#' && ctx.hash() {
                self.push(out, text..self.pos, TokenKind::Text);
                self.template_expression(out);
                text = self.pos;
                continue;
            }
            self.bump();
        }
        self.push(out, text..self.pos, TokenKind::Text);
    }

    // -----------------------------------------------------------------------
    // Attribute islands
    // -----------------------------------------------------------------------

    /// `style="…"` (CSS) and `onclick="…"` (JS): the island opens inside the
    /// string, after its opening quote.
    pub(super) fn attribute_island(&mut self, el: &mut Element, ctx: Ctx, quote: u8, event: bool) {
        let start = self.pos;
        let (lang, site) = if event {
            (Lang::Js, IslandSite::EventAttribute)
        } else {
            (Lang::Css, IslandSite::StyleAttribute)
        };
        let mut island = self.element(
            ElementKind::Island(Island {
                lang,
                site,
                script_type: None,
            }),
            start as u32..start as u32,
        );
        // The value ends where ordinary attribute parsing (`attribute_text`)
        // ends it: at the first quote outside a `#…#` when the context reads
        // them, a CF tag or a `<!--- --->` comment, each parsed whole. A
        // quote inside one of those does not end the island:
        // `onclick="#f("x")#"`, and `style="a;<cfif x eq "b">c</cfif>"`,
        // whose `<cfif>` the engines read before any HTML.
        self.island_body(&mut island.children, ctx, |s| {
            s.rest().as_bytes().first() == Some(&quote)
        });
        Scanner::coalesce_text(&mut island.children);
        island.span = start as u32..self.pos as u32;
        el.children.push(Node::Element(Box::new(island)));
    }

    // -----------------------------------------------------------------------
    // `<cfquery>`
    // -----------------------------------------------------------------------

    /// The SQL island of a `<cfquery>` body.
    pub(super) fn sql_island(&mut self, out: &mut Vec<Node>) {
        out.push(Node::Element(Box::new(self.sql_island_body())));
    }

    fn sql_island_body(&mut self) -> Element {
        let start = self.pos;
        let mut el = self.element(
            ElementKind::Island(Island {
                lang: Lang::Sql,
                site: IslandSite::CfQuery,
                script_type: None,
            }),
            start as u32..start as u32,
        );
        let mut text = self.pos;
        while self.pos < self.end {
            let rest = self.rest();
            if scan::closes_tag(rest, "cfquery").is_some() {
                break;
            }
            let b = rest.as_bytes()[0];
            if b == b'<' && rest.starts_with("<!---") {
                self.push(&mut el.children, text..self.pos, TokenKind::Text);
                self.cf_comment(&mut el.children);
                text = self.pos;
                continue;
            }
            if b == b'<' && self.cf_tag_ahead() {
                self.push(&mut el.children, text..self.pos, TokenKind::Text);
                self.nested(&mut el.children, Ctx::Html, |s, out| s.cf_tag(out));
                text = self.pos;
                continue;
            }
            if b == b'#' {
                if rest.starts_with("##") {
                    self.push(&mut el.children, text..self.pos, TokenKind::Text);
                    self.take(
                        &mut el.children,
                        2,
                        TokenKind::Literal(crate::tree::Literal::EscapeHash),
                    );
                    text = self.pos;
                    continue;
                }
                self.push(&mut el.children, text..self.pos, TokenKind::Text);
                self.template_expression(&mut el.children);
                text = self.pos;
                continue;
            }
            self.bump();
        }
        self.push(&mut el.children, text..self.pos, TokenKind::Text);
        Scanner::coalesce_text(&mut el.children);
        el.span = start as u32..self.pos as u32;
        el
    }

    // -----------------------------------------------------------------------
    // The script stubs
    // -----------------------------------------------------------------------

    /// Where a `<cfscript>` body ends — the first `</cfscript>` outside a
    /// string, a comment, a fence or a `#…#`. A string or `#…#` that does not
    /// close before it is a bare character ([`scan::Bounded`]), so
    /// `x = "price #";` or `x = #;` leaves the body's end where it is and the
    /// script parser reports the statement.
    pub(super) fn script_body_end(&self) -> usize {
        let depth = self.depth;
        let mut bounded = scan::Bounded::new(self.code(), self.pos, scan::Boundary::ScriptClose);
        let mut at = self.pos;
        while at < self.end {
            let rest = self.at(at);
            let b = rest.as_bytes()[0];
            match b {
                b'<' if scan::closes_tag(rest, "cfscript").is_some() => return at,
                b'<' if rest.starts_with("<!---") => at = scan::tag_comment_end(self.code(), at),
                b'\'' | b'"' => at = bounded.string_end(at, depth, true).unwrap_or(at + 1),
                b'#' => {
                    at = bounded
                        .hash_end(at + 1, depth)
                        .map_or(at + 1, |end| end + 1)
                }
                b'/' => at = scan::comment_end(self.code(), at),
                b'`' if rest.starts_with("```") => at = self.fence_end(at + 3),
                _ => at += rest.chars().next().map_or(1, char::len_utf8),
            }
        }
        self.end
    }

    /// Where a tag's expression ends — the first `>` or `/>` outside a string,
    /// a comment or a `#…#`, `=>` consumed first. Returns the end and the
    /// length of the closing delimiter. A string or `#…#` that does not close
    /// before it is a bare character ([`scan::Bounded`]):
    /// `<cfset x = "price #">` ends at its own `>`.
    pub(super) fn angle_end(&self) -> (usize, usize) {
        let depth = self.depth;
        let mut bounded = scan::Bounded::new(self.code(), self.pos, scan::Boundary::TagClose);
        let mut at = self.pos;
        while at < self.end {
            let rest = self.at(at);
            let b = rest.as_bytes()[0];
            match b {
                b'>' => return (at, 1),
                b'/' if rest.starts_with("/>") => return (at, 2),
                b'=' if rest.starts_with("=>") => at += 2,
                b'<' if rest.starts_with("<!---") => at = scan::tag_comment_end(self.code(), at),
                b'\'' | b'"' => at = bounded.string_end(at, depth, false).unwrap_or(at + 1),
                b'#' => {
                    at = bounded
                        .hash_end(at + 1, depth)
                        .map_or(at + 1, |end| end + 1)
                }
                b'/' => at = scan::comment_end(self.code(), at),
                _ => at += rest.chars().next().map_or(1, char::len_utf8),
            }
        }
        (self.end, 0)
    }

    /// Where a `#…#` ends ([`scan::hash_end`] within the scanner's range, at
    /// the scanner's depth).
    pub(super) fn hash_end(&self, from: usize) -> usize {
        scan::hash_end(self.code(), from, self.depth)
    }

    /// The source up to the scanner's `end`, what the [`scan`] routines read.
    fn code(&self) -> &str {
        &self.src[..self.end]
    }

    /// The value of an attribute in a tag the scanner just built: a quoted
    /// value's text tokens, an unquoted value's literal tokens,
    /// [`AttrValue::Dynamic`] when either holds CFML (a `#…#`, a CF tag, a tag
    /// comment) or a token that is not text. Whitespace and newlines around
    /// the `=` are trivia alike.
    ///
    /// The whole tag is [`AttrValue::Dynamic`] too, whatever `name`'s own
    /// value, when CFML stands directly in it — a `#…#`, a CF tag or a CF
    /// tag body among the tag's children, in attribute-name position
    /// (`<script #attrs#>`, `<script <cfif x>type="…"</cfif>>`) or as an
    /// unquoted value (`src=#url#`): the server can emit `type=` there. A
    /// child is always one or the other, so the one walk needs no more
    /// state than the attribute it is reading. CFML inside another
    /// attribute's quoted value (`src="#url#"`) is inside that value's
    /// string element and emits no attribute; a tag comment emits nothing.
    fn attribute_value_text(&self, el: &Element, name: &str) -> AttrValue {
        let text = |t: &crate::tree::Token| &self.src[t.span.start as usize..t.span.end as usize];
        let nodes = &el.children;
        let mut found = None;
        let mut i = 0;
        while i < nodes.len() {
            match &nodes[i] {
                Node::Element(e) if emits_attributes(e) => return AttrValue::Dynamic,
                // A CF tag in an unquoted value goes in flat
                // (`cf_tag_flat`): its `<` is the only tag delimiter a
                // tag's children hold.
                Node::Token(t) if t.kind == TokenKind::Punct(Punct::Open(Delim::Tag)) => {
                    return AttrValue::Dynamic
                }
                Node::Token(t)
                    if found.is_none()
                        && t.kind == TokenKind::Ident(Ident::AttributeName)
                        && text(t).eq_ignore_ascii_case(name) =>
                {
                    let (value, next) = self.attribute_value_at(nodes, i + 1);
                    if value == AttrValue::Dynamic {
                        return value;
                    }
                    found = Some(value);
                    i = next;
                    continue;
                }
                _ => (),
            }
            i += 1;
        }
        found.unwrap_or(AttrValue::Absent)
    }

    /// The value of the attribute whose name ends at `nodes[from]`, and the
    /// index of the first node the value does not take.
    fn attribute_value_at(&self, nodes: &[Node], from: usize) -> (AttrValue, usize) {
        let text = |t: &crate::tree::Token| &self.src[t.span.start as usize..t.span.end as usize];
        let mut value = String::new();
        // Whether the `=` was seen: before it, what follows is another
        // attribute and the value is empty (a bare `type`).
        let mut assigned = false;
        let mut i = from;
        while let Some(node) = nodes.get(i) {
            match node {
                Node::Element(_) if !assigned => break,
                Node::Element(e) if matches!(e.kind, ElementKind::String { .. }) => {
                    for n in &e.children {
                        match n {
                            Node::Token(t) if matches!(t.kind, TokenKind::Literal(_)) => {
                                value.push_str(text(t))
                            }
                            _ => return (AttrValue::Dynamic, i),
                        }
                    }
                    i += 1;
                    break;
                }
                // `type=#kind#`, `type=a#b#`.
                Node::Element(_) => return (AttrValue::Dynamic, i),
                Node::Token(t) => match t.kind {
                    TokenKind::Whitespace | TokenKind::Newline if value.is_empty() => {}
                    TokenKind::Punct(Punct::KeyValue) if value.is_empty() && !assigned => {
                        assigned = true
                    }
                    TokenKind::Literal(_) if assigned => value.push_str(text(t)),
                    _ => break,
                },
            }
            i += 1;
        }
        (AttrValue::Static(value), i)
    }

    /// The end of a tag-island code fence: just past the next ```` ``` ````,
    /// else the scanner's end.
    fn fence_end(&self, from: usize) -> usize {
        match self.at(from).find("```") {
            Some(n) => from + n + 3,
            None => self.end,
        }
    }
}
