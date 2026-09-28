//! Parse tree types: [`Tree`], [`Node`], [`Token`], [`Element`], [`Item`] and the
//! [`TokenKind`] / [`ElementKind`] classifications.

use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use crate::normalize::Newline;

/// Which top-level syntax a source is parsed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Decide from the source: script if the file starts with a comment,
    /// `import` or a `component`/`interface` declaration, tags otherwise — and
    /// tags when the leading comments are followed by a tag. A caller that
    /// has a file name resolves it first with [`Mode::or_for_path`].
    Auto,
    /// CFScript.
    Script,
    /// Tag mode (CFML tags inside HTML), even when the text looks like script.
    Tags,
}

impl Mode {
    /// The mode a file's name decides, where the engines make it certain: a
    /// `.cfs` is always CFScript and a `.cfm` always a tag template (both
    /// compared ASCII case-insensitively). Everything else is [`Mode::Auto`]:
    /// a `.cfc` holds either a script component or a `<cfcomponent>`, and
    /// another name says nothing.
    pub fn for_path(path: &Path) -> Mode {
        match path.extension().and_then(|e| e.to_str()) {
            Some(e) if e.eq_ignore_ascii_case("cfs") => Mode::Script,
            Some(e) if e.eq_ignore_ascii_case("cfm") => Mode::Tags,
            _ => Mode::Auto,
        }
    }

    /// The mode to parse the file at `path` with when `self` was requested:
    /// [`Mode::Script`] and [`Mode::Tags`] stand, and [`Mode::Auto`] takes
    /// [`Mode::for_path`] of the path (stdin without one stays `Auto`).
    pub fn or_for_path(self, path: Option<&Path>) -> Mode {
        match (self, path) {
            (Mode::Auto, Some(p)) => Mode::for_path(p),
            _ => self,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Script => "script",
            Mode::Tags => "tags",
        }
    }
}

/// A parsed source file.
#[derive(Debug, Clone)]
pub struct Tree {
    /// Normalised source: BOM stripped, `\r\n` and lone `\r` replaced by `\n`.
    /// Every span in the tree indexes into this string.
    pub source: String,
    pub root: Element,
    /// The input started with a byte order mark.
    pub bom: bool,
    /// Dominant line ending of the input.
    pub newline: Newline,
    /// Maps normalised offsets back to the input (only when normalisation
    /// changed the text).
    pub(crate) original: Option<Original>,
    /// The regions the parse did not understand, in source order: each is a
    /// [`Recovered`](ElementKind::Recovered) element of the tree.
    pub recoveries: Vec<Recovery>,
}

/// A region of the source the parse did not understand: a script statement,
/// a tag body, or — outside any — the run itself. The tree holds it as a
/// [`Recovered`](ElementKind::Recovered) element whose tokens still tile the
/// region, so a formatter can print it as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// Normalised byte range, on token boundaries.
    pub span: Range<u32>,
    pub reason: RecoveryReason,
}

/// Why a region is a [`Recovery`]. A recovery inside another is one
/// recovery, the outer, whose reason is [`TooDeep`](RecoveryReason::TooDeep)
/// when that is the inner one's (the cut is what left the outer unclosed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryReason {
    /// Text no rule matched (`Other` or `Invalid` tokens): `b = @;`, an
    /// invalid character in a tag's attributes.
    Unmatched,
    /// A closing `)` `}` `]` with nothing open, or a CF closing tag with no
    /// opening tag (`</cfif>`).
    StrayCloser,
    /// A block, parenthesis or bracket, or a CF tag that needs its closing
    /// tag (`<cfif>`, `<cfloop>`, …), still open where its list ends.
    Unclosed,
    /// Nesting past the front ends' bound: the rest of the source was cut
    /// into unmatched text, or an expression run was left flat.
    TooDeep,
}

#[derive(Debug, Clone)]
pub(crate) struct Original {
    pub(crate) text: String,
    /// Bytes removed before the normalised text starts (the BOM).
    pub(crate) prefix: u32,
    /// Normalised offsets of every `\n` that was a `\r\n` in the input.
    pub(crate) crlf_at: Vec<u32>,
}

impl Tree {
    /// Text of a token (in the normalised source).
    pub fn text(&self, t: &Token) -> &str {
        &self.source[t.span.start as usize..t.span.end as usize]
    }

    /// Text of a span in the normalised source.
    pub fn slice(&self, span: Range<u32>) -> &str {
        &self.source[span.start as usize..span.end as usize]
    }

    /// Text of a span exactly as it appeared in the input (line endings not
    /// normalised). Used for `cfformat-ignore` regions.
    pub fn verbatim(&self, span: Range<u32>) -> Cow<'_, str> {
        match &self.original {
            None => Cow::Borrowed(self.slice(span)),
            Some(o) => {
                let map = |p: u32| -> usize {
                    let removed = o.crlf_at.partition_point(|&at| at < p) as u32;
                    (p + o.prefix + removed) as usize
                };
                Cow::Borrowed(&o.text[map(span.start)..map(span.end)])
            }
        }
    }

    /// 1-based line of a byte offset in the normalised source. Offsets past
    /// the end count as the last line.
    pub fn line_of(&self, offset: u32) -> usize {
        line_of(&self.source, offset)
    }

    /// Resolved top-level mode (`Auto` is resolved to script or tags).
    pub fn mode(&self) -> Mode {
        match self.root.kind {
            ElementKind::Root(mode) => mode,
            _ => Mode::Auto,
        }
    }

    /// Name of a tag element (`cfif`, `cf_custom`, `prefix:name`, `div`): the
    /// source text of its leading tag-name tokens. For a `TagBody` it is the
    /// name of its opening tag. Compare it with `eq_ignore_ascii_case`: CFML
    /// and HTML tag names are case-insensitive, and the text is as written.
    pub fn tag_name(&self, el: &Element) -> Option<&str> {
        tag_name_in(&self.source, el)
    }
}

/// [`Tree::tag_name`] over a bare source string (used before the tree exists).
/// The name tokens (`prefix`, `:`, `name` for a `<prefix:name>` custom tag)
/// are consecutive children, and a tag's children tile its span, so the
/// name is one slice of the source from the first to the last of them.
pub(crate) fn tag_name_in<'s>(source: &'s str, el: &Element) -> Option<&'s str> {
    if let Some(open) = el.open_tag() {
        return tag_name_in(source, open);
    }
    let mut name: Option<Range<u32>> = None;
    for node in &el.children {
        match (node, &mut name) {
            (Node::Token(t), None) if matches!(t.kind, TokenKind::Ident(Ident::TagName)) => {
                name = Some(t.span.clone());
            }
            (Node::Token(t), Some(span))
                if matches!(
                    t.kind,
                    TokenKind::Ident(Ident::TagName) | TokenKind::Punct(Punct::Prefix)
                ) =>
            {
                debug_assert_eq!(span.end, t.span.start, "tag-name tokens are adjacent");
                span.end = t.span.end;
            }
            (_, None) => {}
            (_, Some(_)) => break,
        }
    }
    name.map(|span| &source[span.start as usize..span.end as usize])
}

/// 1-based line of `offset` in `source` (clamped to the end of the text).
pub(crate) fn line_of(source: &str, offset: u32) -> usize {
    let end = (offset as usize).min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// A child of an element. The element is boxed so that a node is 16 bytes
/// rather than an element's 112: tokens outnumber elements several times
/// over, and the post-passes move nodes within their lists (drain, insert,
/// splice).
#[derive(Debug, Clone)]
pub enum Node {
    Token(Token),
    Element(Box<Element>),
}

// Kept to the size of a token and its tag; see `Node`.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Node>() == 16);

impl Node {
    pub fn span(&self) -> Range<u32> {
        match self {
            Node::Token(t) => t.span.clone(),
            Node::Element(e) => e.span.clone(),
        }
    }

    pub fn as_token(&self) -> Option<&Token> {
        match self {
            Node::Token(t) => Some(t),
            Node::Element(_) => None,
        }
    }

    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            Node::Token(_) => None,
        }
    }

    /// Not a whitespace/newline token and not a line comment: what delimited
    /// item comment attachment treats as the item's content. Block and doc
    /// comments are significant (they print inline, in place).
    pub fn is_significant(&self) -> bool {
        match self {
            Node::Token(t) => !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline),
            Node::Element(e) => e.kind != ElementKind::LineComment,
        }
    }

    /// A whitespace/newline token or a comment element (line, block or doc):
    /// what the expression post-passes carry inside a node without treating
    /// it as an operand or operator.
    pub fn is_trivia(&self) -> bool {
        match self {
            Node::Token(t) => matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline),
            Node::Element(e) => e.kind.is_comment(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub span: Range<u32>,
    pub kind: TokenKind,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub kind: ElementKind,
    /// Retained opening delimiter, e.g. `(`, `{`, `<`, `'`, `#`.
    pub open: Option<Token>,
    /// Retained closing delimiter, e.g. `)`, `>`, `/>`. For statements this is
    /// the `;` terminator.
    pub close: Option<Token>,
    /// Children of non-delimited elements.
    pub children: Vec<Node>,
    /// Items of delimited elements (struct/array/call args/params/…).
    pub items: Vec<Item>,
    /// Byte span over the normalised source, delimiters included.
    pub span: Range<u32>,
}

impl Element {
    /// Every token of the element in source order: `open`, the children,
    /// each item's nodes and separator by span, `close`.
    pub fn tokens(&self) -> Vec<Token> {
        let mut out = Vec::new();
        self.collect_tokens(&mut out);
        out
    }

    fn collect_tokens(&self, out: &mut Vec<Token>) {
        let node = |n: &Node, out: &mut Vec<Token>| match n {
            Node::Token(t) => out.push(t.clone()),
            Node::Element(e) => e.collect_tokens(out),
        };
        out.extend(self.open.iter().cloned());
        for n in &self.children {
            node(n, out);
        }
        for item in &self.items {
            let mut parts: Vec<(u32, Option<&Node>, Option<&Token>)> = item
                .nodes()
                .map(|n| (n.span().start, Some(n), None))
                .collect();
            if let Some(t) = &item.separator {
                parts.push((t.span.start, None, Some(t)));
            }
            parts.sort_by_key(|p| p.0);
            for part in parts {
                match part {
                    (_, Some(n), _) => node(n, out),
                    (_, _, Some(t)) => out.push(t.clone()),
                    _ => unreachable!(),
                }
            }
        }
        out.extend(self.close.iter().cloned());
    }

    /// The last token of [`Element::tokens`] that `pred` accepts, found
    /// walking back from the end, with nothing collected or cloned.
    pub fn rfind_token(&self, mut pred: impl FnMut(&Token) -> bool) -> Option<&Token> {
        self.rfind_token_dyn(&mut pred)
    }

    fn rfind_token_dyn<'a>(&'a self, pred: &mut dyn FnMut(&Token) -> bool) -> Option<&'a Token> {
        fn node<'a>(n: &'a Node, pred: &mut dyn FnMut(&Token) -> bool) -> Option<&'a Token> {
            match n {
                Node::Token(t) => pred(t).then_some(t),
                Node::Element(e) => e.rfind_token_dyn(pred),
            }
        }
        if let Some(t) = self.close.as_ref().filter(|t| pred(t)) {
            return Some(t);
        }
        for item in self.items.iter().rev() {
            // In the order `tokens` gives an item's parts: by span, the
            // separator among the nodes.
            let mut parts: Vec<(u32, Option<&Node>, Option<&Token>)> = item
                .nodes()
                .map(|n| (n.span().start, Some(n), None))
                .collect();
            if let Some(t) = &item.separator {
                parts.push((t.span.start, None, Some(t)));
            }
            parts.sort_by_key(|p| p.0);
            for part in parts.into_iter().rev() {
                let found = match part {
                    (_, Some(n), _) => node(n, pred),
                    (_, _, Some(t)) => pred(t).then_some(t),
                    _ => unreachable!(),
                };
                if found.is_some() {
                    return found;
                }
            }
        }
        for n in self.children.iter().rev() {
            if let Some(t) = node(n, pred) {
                return Some(t);
            }
        }
        self.open.as_ref().filter(|t| pred(t))
    }

    /// True iff this is an island with exactly one child and that child is a
    /// `Text` token (no CFML tags, template expressions, `##` escapes or CFML
    /// comments inside).
    pub fn is_pure_island(&self) -> bool {
        matches!(self.kind, ElementKind::Island(_))
            && self.children.len() == 1
            && matches!(&self.children[0], Node::Token(t) if t.kind == TokenKind::Text)
    }

    /// Statement terminator (`;`), if present.
    pub fn terminator(&self) -> Option<&Token> {
        match self.kind {
            ElementKind::Statement(_) => self.close.as_ref(),
            _ => None,
        }
    }

    /// The [`CfKind`] of a CF tag, or of a CF `TagBody`'s opening tag.
    pub fn cf_kind(&self) -> Option<CfKind> {
        match self.kind {
            ElementKind::CfTag(_, kind) => Some(kind),
            ElementKind::TagBody { cf: true } => self.open_tag()?.cf_kind(),
            _ => None,
        }
    }

    /// Opening tag element of a `TagBody` (its first child).
    pub fn open_tag(&self) -> Option<&Element> {
        match self.kind {
            ElementKind::TagBody { .. } => self.children.first().and_then(Node::as_element),
            _ => None,
        }
    }

    /// Closing tag element of a `TagBody` (its last child).
    pub fn close_tag(&self) -> Option<&Element> {
        match self.kind {
            ElementKind::TagBody { .. } => self.children.last().and_then(Node::as_element),
            _ => None,
        }
    }

    /// Nodes between the opening and closing tag of a `TagBody`; empty for
    /// any other element.
    pub fn body(&self) -> &[Node] {
        match self.kind {
            ElementKind::TagBody { .. } if self.children.len() >= 2 => {
                &self.children[1..self.children.len() - 1]
            }
            _ => &[],
        }
    }

    /// Child nodes, flattening items for delimited elements ([`Item::nodes`];
    /// separators excluded).
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.children
            .iter()
            .chain(self.items.iter().flat_map(Item::nodes))
    }
}

/// One entry of a delimited element (argument, parameter, array element,
/// struct member, `for (;;)` header clause).
///
/// After [`attach_item_comments`](crate::postpass::comments::attach_item_comments)
/// the line comments around an item live in `leading` / `trailing`, each run
/// with the whitespace and newlines that surround its comments, so tree order
/// is not source order inside an item: `trailing` can hold nodes from before
/// *and* after `separator`. [`Item::nodes`] plus `separator`, sorted by span,
/// tile the item's source.
#[derive(Debug, Clone, Default)]
pub struct Item {
    /// The item's own nodes (block and doc comments stay here, in place).
    pub children: Vec<Node>,
    /// The `,` (or `;` in a `for (;;)` header) that ended this item.
    pub separator: Option<Token>,
    /// Own-line comments before the item: the run of whitespace, newlines and
    /// line comments before its first significant child, when that run holds
    /// a line comment.
    pub leading: Vec<Node>,
    /// Line comments after the item: the run after its last significant child
    /// (before the separator or closing delimiter), then the same-line run
    /// after the separator up to and including the first newline. Each part
    /// is moved only when it holds a line comment.
    pub trailing: Vec<Node>,
}

impl Item {
    /// Every node of the item in field order: `leading`, `children`,
    /// `trailing` (separator excluded).
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.leading
            .iter()
            .chain(self.children.iter())
            .chain(self.trailing.iter())
    }

    /// Mutable [`Item::nodes`].
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.leading
            .iter_mut()
            .chain(self.children.iter_mut())
            .chain(self.trailing.iter_mut())
    }

    /// Comment elements in `leading`.
    pub fn leading_comments(&self) -> impl Iterator<Item = &Element> {
        comments(&self.leading)
    }

    /// Comment elements in `trailing`.
    pub fn trailing_comments(&self) -> impl Iterator<Item = &Element> {
        comments(&self.trailing)
    }

    /// Children that are neither whitespace/newline tokens nor line comments.
    /// An item with no significant children and no leading/trailing comments
    /// is whitespace only (a trailing comma's leftover) and prints as
    /// nothing.
    pub fn significant(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|n| n.is_significant())
    }
}

fn comments(nodes: &[Node]) -> impl Iterator<Item = &Element> {
    nodes
        .iter()
        .filter_map(Node::as_element)
        .filter(|e| e.kind.is_comment())
}

// ---------------------------------------------------------------------------
// Token kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    Newline,
    /// Inside a `cfformat-ignore` region: verbatim.
    Ignore,
    Keyword(Keyword),
    Operator(Operator),
    Punct(Punct),
    Ident(Ident),
    Literal(Literal),
    Storage(Storage),
    /// Text inside a comment.
    CommentText,
    /// `@param` and friends inside doc comments.
    DocTag,
    /// Opaque host text (JS/CSS/SQL/Java/JSON island run, HTML text).
    Text,
    Invalid,
    /// Fallback; printers pass the text through.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    If,
    Else,
    Switch,
    Case,
    Default,
    For,
    While,
    DoWhile,
    Try,
    Catch,
    Finally,
    Return,
    Break,
    Continue,
    Throw,
    /// `include`, `abort`, `rethrow`: the flow keywords no reader tells
    /// apart (the printer lays out a `return` or `throw` value).
    Flow,
    Import,
    Static,
    Java,
    /// `var`, declaring a function-local variable.
    Var,
    New,
    Required,
    Component,
    Interface,
    /// `function`.
    Function,
    /// `=>`.
    Arrow,
}

/// An operator token. The kind carries what the expression pass needs to
/// place it (a binary operator's [`Prec`], prefix or postfix) and what the
/// printer reads (`in`, a word `not`); the spelling is the token's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// An infix operator at its precedence level: arithmetic, `&`, the
    /// comparisons (each word of a [`Phrase`](ElementKind::Phrase) too),
    /// `and` / `or` / `xor` / `eqv` / `imp` and their symbols, and `?:`
    /// ([`Prec::Elvis`]: a ternary `?` immediately followed by `:`, merged
    /// into one token).
    Binary(Prec),
    /// `in`: a comparison, with a kind of its own because `for (k in s)` is
    /// read by it.
    In,
    /// `=`.
    Assign,
    /// `+=`, `-=`, `*=`, `/=`, `%=`, `&=`: binds on its left at the level of
    /// its arithmetic operator.
    AugAssign(Prec),
    TernaryQ,
    TernaryColon,
    /// Unary `+`, `-`.
    Sign,
    /// Prefix `++`, `--`.
    Increment,
    /// `++`, `--` after an operand.
    Postfix,
    /// `!`, or the word `not` (`word`).
    Not {
        word: bool,
    },
    /// `...`: a spread in an expression, a rest parameter in a parameter
    /// list.
    Spread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Punct {
    Terminator,
    EmptyTerminator,
    Comma,
    KeyValue,
    /// `case x:`, `label:`.
    Colon,
    /// `;` inside a `for (;;)` header.
    ExprSeparator,
    Accessor,
    SafeAccessor,
    StaticAccessor,
    /// `:` in a `<prefix:tag>` custom tag name.
    Prefix,
    Open(Delim),
    Close(Delim),
}

/// What an `Open` / `Close` token delimits. The recovery check reads it: a
/// `Paren`, `Brace`, `Bracket`, `String` or `Template` left open is an
/// [`Unclosed`](RecoveryReason::Unclosed) region, a `Fence`, `Tag` or
/// comment is not (a fence runs to the end of the file by rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delim {
    /// `(` `)`, a parameter list's included.
    Paren,
    Brace,
    Bracket,
    /// `<`, `</`, `>`, `/>` of a tag.
    Tag,
    /// A string's quotes.
    String,
    /// The `#`s of a template expression.
    Template,
    /// A code fence's ```` ``` ````.
    Fence,
    /// `/*`, `/**`, `//` and an HTML `<!--`.
    Comment,
    /// `<!---` and `--->`: a CFML comment, which nests.
    TagComment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ident {
    Variable,
    ScopeVar,
    This,
    Super,
    FunctionName,
    /// The name of a called function: a user-defined function in a
    /// plain call (`f()`), a method (`a.f()`, `A::f()`).
    Call,
    /// The name of a built-in function in a plain call.
    Builtin,
    Parameter,
    /// Named argument in a call (`foo(a = 1)`).
    ArgName,
    Label,
    /// A class name: a `new` target, a class before `::`, a `catch` type, a
    /// component's `extends` value.
    ClassName,
    StructKey,
    AttributeName,
    PropertyName,
    TagName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Literal {
    Number,
    Bool,
    Null,
    Unquoted,
    StringText,
    EscapeHash,
    EscapeQuote,
    /// `*` in `import a.b.*`.
    Constant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Storage {
    Type,
    Modifier,
}

// ---------------------------------------------------------------------------
// Element kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementKind {
    Root(Mode),
    /// A statement; the kind is the script parser's, from the rule that
    /// read it, and the expression pass's for an expression statement it
    /// builds into an assignment.
    Statement(StatementKind),
    Struct {
        ordered: bool,
    },
    Array,
    /// An Adobe typed array literal, `['string']['a', 'b']`: the type's
    /// [`Brackets`](ElementKind::Brackets), then the [`Array`](ElementKind::Array).
    TypedArray,
    /// Arguments of a call; the callee tokens are siblings before it. What
    /// is called is the callee's: [`Ident::Builtin`] for a built-in
    /// function, [`Ident::Call`] for a user-defined one or a method.
    Call,
    /// Function declaration parameters.
    Parameters,
    Block(BlockKind),
    Group,
    Brackets,
    FunctionDecl,
    ArrowFunction,
    ClassDecl,
    InterfaceDecl,
    Property,
    Param,
    Import,
    If,
    ElseIf,
    Else,
    For,
    While,
    DoWhile,
    Switch,
    /// `case <expr>:` / `default:` and the statements that follow it.
    Case,
    Try,
    Catch,
    Finally,
    /// `cfhttp(...)` (`acf: true`) vs `cfhttp url=...;`.
    ScriptTag {
        acf: bool,
    },
    ScriptTagAttributes,
    LineComment,
    BlockComment,
    DocComment,
    String {
        quote: Quote,
        in_tag: bool,
    },
    TemplateExpression,
    /// A CFML tag: its shape and how the tag scanner read it ([`CfKind`]).
    CfTag(TagShape, CfKind),
    HtmlTag(TagShape),
    Doctype,
    /// Paired tags (tag pairing post-pass): `children` are
    /// `[open tag, …body…, close tag]`, `open`/`close` are `None`; see
    /// [`Element::open_tag`], [`Element::body`], [`Element::close_tag`].
    TagBody {
        /// CFML tags (`CfTag`) rather than HTML tags.
        cf: bool,
    },
    Island(Island),
    /// Script code fence (```` ``` ````); children are tag-mode nodes.
    TagIsland,
    /// `cfformat-ignore` region, verbatim.
    Ignore,
    /// A region the parse did not understand ([`Recovery`]): its children
    /// are the nodes as the parse left them, printed as written.
    Recovered(RecoveryReason),

    // --- Expression structure: key-values and declarations built by the
    // front ends, the rest by the post-passes.
    // None of these has `open`/`close`; children keep the trivia between
    // their first and last significant node. See the views in
    // [`crate::nodes`].
    /// `key sep value`: struct member, named argument, parameter default or
    /// attribute ([`KeyValue`](crate::nodes::KeyValue)).
    KeyValue,
    /// A function declaration or arrow function fused with its body
    /// ([`Decl`](crate::nodes::Decl)).
    Function {
        /// `(a) => …` rather than `function (a) {…}`.
        arrow: bool,
    },
    /// `component … {}` fused with its body ([`Decl`](crate::nodes::Decl)).
    Class,
    /// `interface … {}` fused with its body ([`Decl`](crate::nodes::Decl)).
    Interface,
    /// `static { … }` ([`Decl`](crate::nodes::Decl)).
    StaticBlock,
    /// `target op value`, `=` or augmented, right-associative
    /// ([`Assignment`](crate::nodes::Assignment)).
    Assignment,
    /// `cond ? then : otherwise` ([`Ternary`](crate::nodes::Ternary)).
    Ternary,
    /// One run of same-precedence binary operators: operands and operator
    /// tokens alternate ([`Binary`](crate::nodes::Binary)).
    Binary {
        prec: Prec,
    },
    /// Prefix (`-a`, `not a`, `++a`, `...a`) or postfix (`a++`) operator
    /// ([`Unary`](crate::nodes::Unary)).
    Unary {
        postfix: bool,
    },
    /// A callee that is not a member access followed by its arguments:
    /// `foo(1)`, `foo()()` ([`CallExpr`](crate::nodes::CallExpr)).
    CallExpr,
    /// `new` + class name or string (+ arguments) ([`New`](crate::nodes::New)).
    New,
    /// An operand followed by member accesses and index brackets
    /// ([`Chain`](crate::nodes::Chain)).
    Chain,
    /// One access of a [`Chain`](ElementKind::Chain)
    /// ([`Segment`](crate::nodes::Segment)).
    Segment(SegmentKind),
    /// A multi-word operator (`less than`, `is not`, `does not contain`),
    /// built by the parser: its words, each an operator token of the
    /// phrase's kind, and the whitespace between them, so that no token
    /// spans whitespace or a line end. The expression pass reads it as one
    /// operator; a printer writes the words one space apart.
    Phrase,
}

/// What a [`Statement`](ElementKind::Statement) holds, from its first
/// significant child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementKind {
    /// `x = 1`, `a.b += 2`.
    Assignment,
    /// Anything else: a call, a lone operand, an unparsed run.
    Expression,
    /// `var x = 1`.
    Declaration,
    /// `if`, `for`, `while`, `do`, `switch`, `try` (and orphan clauses), a
    /// labelled loop, a bare block.
    Keyword,
    /// `return`, `break`, `continue`, `throw`, `abort`, `include`, …
    Flow,
    Function,
    Class,
    Interface,
    /// `static { … }`.
    StaticBlock,
    Property,
    Param,
    /// `http url="x";`, `cfhttp(url="x");`.
    ScriptTag,
    Import,
    /// A lone `;`.
    Empty,
}

/// Binary operator precedence, loosest first (`Ord` follows binding
/// strength). The lexer gives each binary and augmented-assignment operator
/// its level ([`Operator::Binary`], [`Operator::AugAssign`]); prefix
/// operators, the ternary and assignments are not binary levels. See
/// `postpass::expressions` for the full table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Prec {
    /// `?:`
    Elvis,
    /// `imp`
    Imp,
    /// `eqv`
    Eqv,
    /// `xor`
    Xor,
    /// `or`, `||`
    Or,
    /// `and`, `&&`
    And,
    /// `eq`, `==`, `lt`, `contains`, `is not`, `in`, …
    Comparison,
    /// `&`
    Concat,
    /// `+`, `-`
    Additive,
    /// `%`, `mod`
    Modulus,
    /// `*`, `/`, `\`
    Multiplicative,
    /// `^`
    Exponent,
}

/// What a chain [`Segment`](ElementKind::Segment) accesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentKind {
    /// `.name`, `?.name`, `::name`.
    Property,
    /// `.name(args)`, `?.name(args)`, `::name(args)`.
    Method,
    /// `[expr]`.
    Index,
}

impl ElementKind {
    /// Any [`Statement`](ElementKind::Statement).
    pub fn is_statement(&self) -> bool {
        matches!(self, ElementKind::Statement(_))
    }

    pub fn is_comment(&self) -> bool {
        matches!(
            self,
            ElementKind::LineComment | ElementKind::BlockComment | ElementKind::DocComment
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Plain,
    Function,
    Class,
    Interface,
    Static,
    Java,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote {
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagShape {
    Open,
    Close,
    SelfClosed,
}

/// Which CFML tag a [`CfTag`](ElementKind::CfTag) is, as far as a reader
/// tells tags apart by what they are rather than by their name: the tags
/// the tag scanner reads its own way (their attributes, their body), and
/// the branch tags of a `<cfif>` body. An opening tag's kind is the one the
/// scanner read it as; a closing tag has its opening tag's (`</cfquery>` is
/// `Query`, and `</cfcomponent>` is `Class` after the head `<cfcomponent>`).
/// A name the scanner reads differently — `<cfset2>`, a custom
/// `<cfelse:x>` — is `Generic`, whatever its first letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfKind {
    /// `<cffunction>`: `name`, `access` and `returntype` read as a
    /// declaration's; an HTML body.
    Function,
    /// `<cfoutput>` and `<cfmail>`: a body where `#…#` is read.
    Output,
    /// `<cfquery>`: a SQL island body.
    Query,
    /// `<cfproperty>`: `name` read as a declaration's.
    Property,
    /// `<cfscript>`: a script body.
    Script,
    /// `<cfset>`, `<cfreturn>`, `<cfif>`: script after the name.
    Expression,
    /// `<cfelseif>`: script after the name, and a branch of its `<cfif>`.
    ElseIf,
    /// `<cfelse>`: a branch of its `<cfif>`.
    Else,
    /// `<cfjava>`: a Java island body.
    Java,
    /// `<cfcomponent>` / `<cfinterface>` at the head of a tag-mode source:
    /// `extends` read as a declaration's.
    Class,
    /// Every other tag, the custom (`<cf_name>`, `<prefix:name>`) and
    /// extension (`<cfx_name>`) ones included.
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Island {
    pub lang: Lang,
    pub site: IslandSite,
    /// `<script type="…">` value, lowercased, if any; for a
    /// [`Lang::Unknown`] body the `<script>` or `<style>` type that named no
    /// language, trimmed and lowercased, `None` when it holds CFML.
    pub script_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Js,
    Css,
    Sql,
    Java,
    Json,
    /// A `<script>` / `<style>` body whose `type` names no language the
    /// scanner knows, or that only the server knows (a `type` holding CFML,
    /// or CFML in the tag that can emit one): data, printed byte for byte.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IslandSite {
    ScriptTag,
    StyleTag,
    /// JS in `onclick="…"` and other event attributes.
    EventAttribute,
    /// CSS in a `style="…"` attribute.
    StyleAttribute,
    CfQuery,
    CfJava,
}

// ---------------------------------------------------------------------------
// Names (inspect JSON / tree dump)
// ---------------------------------------------------------------------------

impl TokenKind {
    /// Short dotted name used by the inspect JSON (`k`) and `--tree` dump.
    pub fn name(&self) -> String {
        match self {
            TokenKind::Whitespace => "ws".into(),
            TokenKind::Newline => "nl".into(),
            TokenKind::Ignore => "ignore".into(),
            TokenKind::Keyword(k) => format!("kw.{}", k.name()),
            TokenKind::Operator(o) => format!("op.{}", o.name()),
            TokenKind::Punct(Punct::Open(d)) => format!("punct.open.{}", d.name()),
            TokenKind::Punct(Punct::Close(d)) => format!("punct.close.{}", d.name()),
            TokenKind::Punct(p) => format!("punct.{}", p.name()),
            TokenKind::Ident(i) => format!("ident.{}", i.name()),
            TokenKind::Literal(l) => format!("lit.{}", l.name()),
            TokenKind::Storage(s) => format!("storage.{}", s.name()),
            TokenKind::CommentText => "comment".into(),
            TokenKind::DocTag => "doc-tag".into(),
            TokenKind::Text => "text".into(),
            TokenKind::Invalid => "invalid".into(),
            TokenKind::Other => "other".into(),
        }
    }
}

macro_rules! names {
    ($ty:ty { $($variant:ident => $name:literal),* $(,)? }) => {
        impl $ty {
            pub fn name(&self) -> &'static str {
                match self {
                    $(<$ty>::$variant => $name,)*
                }
            }
        }
    };
}

names!(Keyword {
    If => "if", Else => "else", Switch => "switch", Case => "case",
    Default => "default", For => "for", While => "while", DoWhile => "do", Try => "try",
    Catch => "catch", Finally => "finally", Return => "return", Break => "break",
    Continue => "continue", Throw => "throw", Flow => "flow", Import => "import",
    Static => "static", Java => "java", Var => "var", New => "new", Required => "required",
    Component => "component", Interface => "interface", Function => "function", Arrow => "arrow",
});

impl Operator {
    /// A binary operator is named by its level (`op.additive`, `op.and`).
    pub fn name(&self) -> &'static str {
        match self {
            Operator::Binary(prec) => prec.name(),
            Operator::In => "in",
            Operator::Assign => "assign",
            Operator::AugAssign(_) => "aug-assign",
            Operator::TernaryQ => "ternary-q",
            Operator::TernaryColon => "ternary-colon",
            Operator::Sign => "sign",
            Operator::Increment => "increment",
            Operator::Postfix => "postfix",
            Operator::Not { .. } => "not",
            Operator::Spread => "spread",
        }
    }
}

impl Punct {
    pub fn name(&self) -> &'static str {
        match self {
            Punct::Terminator => "terminator",
            Punct::EmptyTerminator => "empty-terminator",
            Punct::Comma => "comma",
            Punct::KeyValue => "key-value",
            Punct::Colon => "colon",
            Punct::ExprSeparator => "expr-separator",
            Punct::Accessor => "accessor",
            Punct::SafeAccessor => "safe-accessor",
            Punct::StaticAccessor => "static-accessor",
            Punct::Prefix => "prefix",
            Punct::Open(_) => "open",
            Punct::Close(_) => "close",
        }
    }
}

names!(Delim {
    Paren => "paren", Brace => "brace", Bracket => "bracket", Tag => "tag",
    String => "string", Template => "template", Fence => "fence", Comment => "comment",
    TagComment => "tag-comment",
});

impl Ident {
    pub fn name(&self) -> &'static str {
        match self {
            Ident::Variable => "variable",
            Ident::ScopeVar => "scope",
            Ident::This => "this",
            Ident::Super => "super",
            Ident::FunctionName => "function-name",
            Ident::Call => "call",
            Ident::Builtin => "builtin",
            Ident::Parameter => "parameter",
            Ident::ArgName => "arg-name",
            Ident::Label => "label",
            Ident::ClassName => "class-name",
            Ident::StructKey => "struct-key",
            Ident::AttributeName => "attribute-name",
            Ident::PropertyName => "property-name",
            Ident::TagName => "tag-name",
        }
    }
}

names!(Literal {
    Number => "number", Bool => "bool", Null => "null", Unquoted => "unquoted",
    StringText => "string", EscapeHash => "escape-hash", EscapeQuote => "escape-quote",
    Constant => "constant",
});

names!(Storage { Type => "type", Modifier => "modifier" });

impl ElementKind {
    pub fn name(&self) -> &'static str {
        match self {
            ElementKind::Root(_) => "root",
            ElementKind::Statement(_) => "statement",
            ElementKind::Struct { .. } => "struct",
            ElementKind::Array => "array",
            ElementKind::TypedArray => "typed-array",
            ElementKind::Call => "call",
            ElementKind::Parameters => "parameters",
            ElementKind::Block(_) => "block",
            ElementKind::Group => "group",
            ElementKind::Brackets => "brackets",
            ElementKind::FunctionDecl => "function-decl",
            ElementKind::ArrowFunction => "arrow-function",
            ElementKind::ClassDecl => "class-decl",
            ElementKind::InterfaceDecl => "interface-decl",
            ElementKind::Property => "property",
            ElementKind::Param => "param",
            ElementKind::Import => "import",
            ElementKind::If => "if",
            ElementKind::ElseIf => "else-if",
            ElementKind::Else => "else",
            ElementKind::For => "for",
            ElementKind::While => "while",
            ElementKind::DoWhile => "do-while",
            ElementKind::Switch => "switch",
            ElementKind::Case => "case",
            ElementKind::Try => "try",
            ElementKind::Catch => "catch",
            ElementKind::Finally => "finally",
            ElementKind::ScriptTag { .. } => "script-tag",
            ElementKind::ScriptTagAttributes => "script-tag-attributes",
            ElementKind::LineComment => "line-comment",
            ElementKind::BlockComment => "block-comment",
            ElementKind::DocComment => "doc-comment",
            ElementKind::String { .. } => "string",
            ElementKind::TemplateExpression => "template-expression",
            ElementKind::CfTag(..) => "cf-tag",
            ElementKind::HtmlTag(_) => "html-tag",
            ElementKind::Doctype => "doctype",
            ElementKind::TagBody { .. } => "tag-body",
            ElementKind::Island(_) => "island",
            ElementKind::TagIsland => "tag-island",
            ElementKind::Ignore => "ignore",
            ElementKind::Recovered(_) => "recovered",
            ElementKind::KeyValue => "key-value",
            ElementKind::Function { .. } => "function",
            ElementKind::Class => "class",
            ElementKind::Interface => "interface",
            ElementKind::StaticBlock => "static-block",
            ElementKind::Assignment => "assignment",
            ElementKind::Ternary => "ternary",
            ElementKind::Binary { .. } => "binary",
            ElementKind::Unary { .. } => "unary",
            ElementKind::CallExpr => "call-expr",
            ElementKind::New => "new",
            ElementKind::Chain => "chain",
            ElementKind::Segment(_) => "segment",
            ElementKind::Phrase => "phrase",
        }
    }
}

names!(BlockKind {
    Plain => "plain", Function => "function", Class => "class", Interface => "interface",
    Static => "static", Java => "java",
});
names!(Quote { Single => "single", Double => "double" });
names!(TagShape { Open => "open", Close => "close", SelfClosed => "self-closed" });
names!(CfKind {
    Function => "function", Output => "output", Query => "query", Property => "property",
    Script => "script", Expression => "expression", ElseIf => "else-if", Else => "else",
    Java => "java", Class => "class", Generic => "generic",
});
names!(Lang { Js => "js", Css => "css", Sql => "sql", Java => "java", Json => "json", Unknown => "unknown" });
names!(IslandSite {
    ScriptTag => "script-tag", StyleTag => "style-tag", EventAttribute => "event-attribute",
    StyleAttribute => "style-attribute", CfQuery => "cfquery", CfJava => "cfjava",
});
names!(StatementKind {
    Assignment => "assignment", Expression => "expression", Declaration => "declaration",
    Keyword => "keyword", Flow => "flow", Function => "function", Class => "class",
    Interface => "interface", StaticBlock => "static-block", Property => "property",
    Param => "param", ScriptTag => "script-tag", Import => "import", Empty => "empty",
});
names!(Prec {
    Elvis => "elvis", Imp => "imp", Eqv => "eqv", Xor => "xor", Or => "or", And => "and",
    Comparison => "comparison", Concat => "concat", Additive => "additive", Modulus => "modulus",
    Multiplicative => "multiplicative", Exponent => "exponent",
});
names!(SegmentKind { Property => "property", Method => "method", Index => "index" });
names!(RecoveryReason {
    Unmatched => "unmatched", StrayCloser => "stray-closer", Unclosed => "unclosed",
    TooDeep => "too-deep",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_name_decides_the_mode_where_it_is_certain() {
        for (name, mode) in [
            ("a.cfs", Mode::Script),
            ("src/A.CFS", Mode::Script),
            ("a.cfm", Mode::Tags),
            ("views/a.Cfm", Mode::Tags),
            // A `.cfc` is a script component or a `<cfcomponent>`.
            ("a.cfc", Mode::Auto),
            ("a.cfml", Mode::Auto),
            ("a.txt", Mode::Auto),
            ("Makefile", Mode::Auto),
            ("cfs", Mode::Auto),
            (".cfs", Mode::Auto),
            ("", Mode::Auto),
        ] {
            assert_eq!(Mode::for_path(Path::new(name)), mode, "{name:?}");
        }
        // A requested mode stands; `Auto` takes the name's, if any.
        let cfs = Some(Path::new("a.cfs"));
        assert_eq!(Mode::Tags.or_for_path(cfs), Mode::Tags);
        assert_eq!(
            Mode::Script.or_for_path(Some(Path::new("a.cfm"))),
            Mode::Script
        );
        assert_eq!(Mode::Auto.or_for_path(cfs), Mode::Script);
        assert_eq!(Mode::Auto.or_for_path(None), Mode::Auto);
    }
}
