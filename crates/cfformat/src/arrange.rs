//! Member ordering (`cfformat arrange`): a component's functions ordered by
//! access, then name, and, on request, its properties by name.
//!
//! This is a splice over the parse tree, not a printer transform: the
//! output is the input's own bytes with the members of each run permuted,
//! so nothing outside the moved members changes, and arranging never
//! formats. It reads the tree only (no [`Options`](crate::Options), no
//! islands).
//!
//! The pieces, per component body:
//! - A **unit** is a member (a function, or with
//!   [`ArrangeOptions::properties`] a property) with the comments attached
//!   to it: those directly above it, each starting its own line, with no
//!   blank line between them or before the member, and at most one comment
//!   starting on the member's last line. A lone `;` after the member on
//!   that line (`};`) belongs to it too. A unit must start and end its
//!   lines; a member that shares a line with anything else stays where it
//!   is. That is what makes arranging idempotent: a unit moved next to
//!   something else can neither lose nor gain a comment on the next run.
//! - Everything else is **fixed**: other statements, ignore regions,
//!   comments attached to no member (one followed by a blank line, one
//!   below a member), banners, text and other tags, and the members whose
//!   order cannot be read with confidence ([`Ineligible`]). A **banner** is
//!   a one-line comment whose words are framed by rule characters
//!   (`// ---- PRIVATE ----`): it titles the members below it, so it never
//!   attaches to the first of them, even with no blank line between.
//! - A **run** is two or more units of one kind with only whitespace
//!   between them, and, for properties, no blank line: property order is
//!   what authors group by hand (injections, then data), so a blank line
//!   between two properties ends the run, while functions cross blank
//!   lines freely. Each run is sorted on its own, so a banner or a
//!   statement divides a body into sections, and properties and functions
//!   never cross each other. The whitespace between two units belongs to
//!   the slot: the n-th separator stays the n-th, so blank lines and the
//!   first line's indentation stay where they were.

use std::cmp::Ordering;
use std::ops::Range;

use cfparse::{
    BlockKind, CfKind, Element, ElementKind, Ident, Keyword, Mode, Node, RecoveryReason,
    StatementKind, Storage, TagShape, TokenKind, Tree,
};

/// What [`arrange`] moves beyond functions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArrangeOptions {
    /// Sort properties by name too. Off by default: property order is
    /// visible at runtime (`getMetadata().properties`, ORM mappings,
    /// serialisation).
    pub properties: bool,
}

/// An arranged source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arranged {
    /// The output: the input with the units of each run in order.
    pub text: String,
    /// `text` differs from the input.
    pub changed: bool,
    /// The component bodies left as written because the parse recovered
    /// in them, in source order.
    pub skipped: Vec<Skipped>,
}

/// A component body [`arrange`] left as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skipped {
    /// The 1-based line where the body starts.
    pub line: usize,
    /// Why the parse recovered in it (the first recovery that overlaps it).
    pub reason: RecoveryReason,
}

/// A function's access, widest first (the order functions sort in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    Remote,
    Public,
    Package,
    Private,
}

impl Access {
    fn parse(word: &str) -> Option<Access> {
        [
            ("remote", Access::Remote),
            ("public", Access::Public),
            ("package", Access::Package),
            ("private", Access::Private),
        ]
        .into_iter()
        .find(|(w, _)| word.eq_ignore_ascii_case(w))
        .map(|(_, a)| a)
    }
}

/// The two kinds of member. A run holds one kind only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberKind {
    Function,
    Property,
}

/// A member and its attached comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// From the first leading comment (or the member) to the trailing
    /// comment (or the member), in the normalised source.
    pub span: Range<u32>,
    pub kind: MemberKind,
    /// The name as written.
    pub name: String,
    /// The function's access; [`Access::Public`] for a property.
    pub access: Access,
}

impl Unit {
    /// The order units sort in: for functions `init` first, then widest
    /// access, then name; for properties the name alone. Names compare
    /// ASCII case-insensitively.
    pub fn order(&self, other: &Unit) -> Ordering {
        let name = |u: &Unit| u.name.to_ascii_lowercase();
        match self.kind {
            MemberKind::Function => {
                let init = |u: &Unit| !u.name.eq_ignore_ascii_case("init");
                (init(self), self.access, name(self)).cmp(&(init(other), other.access, name(other)))
            }
            MemberKind::Property => name(self).cmp(&name(other)),
        }
    }
}

/// Two or more units of one kind with only whitespace between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub kind: MemberKind,
    /// In source order.
    pub units: Vec<Unit>,
    /// The whitespace between consecutive units (one fewer than the units).
    pub separators: Vec<Range<u32>>,
}

/// Why a child of a component body stays where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fixed {
    /// A comment attached to no member: one followed by a blank line, one
    /// below a member.
    Comment,
    /// A one-line comment whose words are framed by rule characters
    /// (`// ---- PRIVATE ----`, `/***** Helpers *****/`).
    Banner,
    /// A statement other than a member: pseudo-constructor code,
    /// `static { }`, an import.
    Statement,
    /// A property, without [`ArrangeOptions::properties`].
    Property,
    /// A `cfformat-ignore` or `@formatter:off` region.
    Ignore,
    /// Text, HTML, `<cfscript>` or any other tag.
    Other,
    /// A member whose order is not read with confidence.
    Ineligible(Ineligible),
    /// A member that shares a line with something else.
    SharedLine,
}

/// Why a member is not arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ineligible {
    /// A function whose name is not a plain identifier (a `cffunction`
    /// whose `name` is missing or not plain).
    Name,
    /// An `access` attribute that is not plain or not one of the four
    /// access words.
    Access,
    /// Access modifiers or attributes that disagree.
    Disagree,
    /// A doc comment with an `@access` tag: whether the engines honour it
    /// is not checked, so it is not guessed.
    DocAccess,
    /// A property with no name or a name that is not plain.
    PropertyName,
    /// A property with both a `name=` attribute and a shorthand name.
    TwoNames,
}

/// One component body: what [`arrange`] saw in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// The body's element (`{ … }`, or the `<cfcomponent>` tag body).
    pub span: Range<u32>,
    /// Set when the parse recovered in the body: then it holds no runs and
    /// no fixed elements.
    pub skipped: Option<RecoveryReason>,
    pub runs: Vec<Run>,
    /// Units that form no run (one alone between fixed elements).
    pub alone: usize,
    /// The fixed elements, in source order.
    pub fixed: Vec<(Range<u32>, Fixed)>,
}

/// Arranges every top-level component and interface body in `src`.
pub fn arrange(src: &str, mode: Mode, opts: &ArrangeOptions) -> Arranged {
    let tree = cfparse::parse_source(src, mode);
    // The fixed elements are for `bodies`' callers; arranging never reads
    // them.
    let bodies = read_bodies(&tree, opts, false);
    let skipped = bodies
        .iter()
        .filter_map(|b| {
            b.skipped.map(|reason| Skipped {
                line: tree.line_of(b.span.start),
                reason,
            })
        })
        .collect();
    // Each slot that receives another unit's text: (slot, unit).
    let mut moves = Vec::new();
    for run in bodies.iter().flat_map(|b| &b.runs) {
        let mut order: Vec<usize> = (0..run.units.len()).collect();
        order.sort_by(|&a, &b| run.units[a].order(&run.units[b]));
        for (slot, &unit) in order.iter().enumerate() {
            if slot != unit {
                moves.push((run.units[slot].span.clone(), run.units[unit].span.clone()));
            }
        }
    }
    if moves.is_empty() {
        return Arranged {
            text: src.to_owned(),
            changed: false,
            skipped,
        };
    }
    moves.sort_by_key(|(slot, _)| slot.start);
    // The input's own bytes throughout, so line endings (mixed ones too)
    // survive as they were.
    let mut text = String::with_capacity(src.len());
    if tree.bom {
        text.push('\u{FEFF}');
    }
    let mut at = 0;
    for (slot, unit) in moves {
        text.push_str(&tree.verbatim(at..slot.start));
        text.push_str(&tree.verbatim(unit));
        at = slot.end;
    }
    text.push_str(&tree.verbatim(at..tree.source.len() as u32));
    let changed = text != src;
    Arranged {
        text,
        changed,
        skipped,
    }
}

/// Every run of every top-level component body in `tree`, in source order.
pub fn runs(tree: &Tree, opts: &ArrangeOptions) -> Vec<Run> {
    read_bodies(tree, opts, false)
        .into_iter()
        .flat_map(|b| b.runs)
        .collect()
}

/// Every top-level component and interface body in `tree`, in source
/// order: a script `component { }` or `interface { }` block, or a
/// `<cfcomponent>` or `<cfinterface>` tag body. Nested bodies are never
/// arranged (a `<cfscript>` inside `<cfcomponent>`, a function inside a
/// function).
pub fn bodies(tree: &Tree, opts: &ArrangeOptions) -> Vec<Body> {
    read_bodies(tree, opts, true)
}

/// [`bodies`], each body's `fixed` left empty unless `fixed`.
fn read_bodies(tree: &Tree, opts: &ArrangeOptions, fixed: bool) -> Vec<Body> {
    let mut found = Vec::new();
    top_level(tree, &tree.root.children, &mut found);
    found
        .into_iter()
        .map(|(el, nodes)| {
            let overlap = tree
                .recoveries
                .iter()
                .find(|r| r.span.start < el.span.end && el.span.start < r.span.end);
            match overlap {
                Some(r) => Body {
                    span: el.span.clone(),
                    skipped: Some(r.reason),
                    runs: Vec::new(),
                    alone: 0,
                    fixed: Vec::new(),
                },
                None => body(tree, el, nodes, opts, fixed),
            }
        })
        .collect()
}

/// The component bodies among `nodes`, looking through a recovered region
/// (the parse may wrap a whole component in one).
fn top_level<'a>(tree: &Tree, nodes: &'a [Node], out: &mut Vec<(&'a Element, &'a [Node])>) {
    for el in nodes.iter().filter_map(Node::as_element) {
        match &el.kind {
            ElementKind::Recovered(_) => top_level(tree, &el.children, out),
            ElementKind::Statement(StatementKind::Class | StatementKind::Interface) => {
                if let Some(block) = script_body(el) {
                    out.push((block, &block.children));
                }
            }
            ElementKind::TagBody { cf: true } => {
                let name = tree.tag_name(el).unwrap_or_default();
                if name.eq_ignore_ascii_case("cfcomponent")
                    || name.eq_ignore_ascii_case("cfinterface")
                {
                    out.push((el, el.body()));
                }
            }
            _ => {}
        }
    }
}

/// The `{ … }` of a `component` or `interface` statement, fused into a
/// `Class` / `Interface` element or not.
fn script_body(stmt: &Element) -> Option<&Element> {
    let is_body = |e: &&Element| {
        matches!(
            e.kind,
            ElementKind::Block(BlockKind::Class | BlockKind::Interface)
        )
    };
    stmt.children
        .iter()
        .filter_map(Node::as_element)
        .find_map(|e| match e.kind {
            ElementKind::Class | ElementKind::Interface => {
                e.children.iter().filter_map(Node::as_element).find(is_body)
            }
            _ => Some(e).filter(is_body),
        })
}

/// A child of a body, classified.
enum Piece<'a> {
    /// Whitespace and newlines.
    Space,
    Comment(&'a Element),
    /// A lone `;` (an empty statement).
    Semicolon,
    Member(Member),
    Fixed(Fixed),
}

/// An eligible member, before its comments are attached.
struct Member {
    kind: MemberKind,
    name: String,
    access: Access,
}

fn body(
    tree: &Tree,
    el: &Element,
    nodes: &[Node],
    opts: &ArrangeOptions,
    want_fixed: bool,
) -> Body {
    let pieces: Vec<(Range<u32>, Piece)> = nodes
        .iter()
        .map(|n| (n.span(), classify(tree, n, opts)))
        .collect();
    let src = tree.source.as_bytes();
    let starts_line = |at: u32| {
        src[..at as usize]
            .iter()
            .rev()
            .find(|&&b| b != b' ' && b != b'\t')
            .is_none_or(|&b| b == b'\n')
    };
    let ends_line = |at: u32| {
        src[at as usize..]
            .iter()
            .find(|&&b| b != b' ' && b != b'\t')
            .is_none_or(|&b| b == b'\n')
    };
    let newlines = |span: Range<u32>| tree.slice(span).matches('\n').count();
    // The piece before / after `i` that is not whitespace.
    let prev = |i: usize| (0..i).rev().find(|&j| !matches!(pieces[j].1, Piece::Space));
    let next = |i: usize| (i + 1..pieces.len()).find(|&j| !matches!(pieces[j].1, Piece::Space));

    // Each piece's place: part of a unit (`Some(unit index)`) or not.
    let mut owner: Vec<Option<usize>> = vec![None; pieces.len()];
    let mut units: Vec<(Unit, usize)> = Vec::new();
    let mut fixed: Vec<Option<Fixed>> = pieces
        .iter()
        .map(|(_, p)| match p {
            Piece::Fixed(f) => Some(*f),
            _ => None,
        })
        .collect();
    for (i, (span, piece)) in pieces.iter().enumerate() {
        let Piece::Member(member) = piece else {
            continue;
        };
        let mut first = i;
        while let Some(j) = prev(first) {
            let comment_above = matches!(pieces[j].1, Piece::Comment(c) if !is_banner(tree, c))
                && owner[j].is_none()
                && starts_line(pieces[j].0.start)
                && newlines(pieces[j].0.end..pieces[first].0.start) == 1;
            if !comment_above {
                break;
            }
            first = j;
        }
        let mut last = i;
        // `};`: a stray terminator on the member's last line goes with it.
        if let Some(j) = next(last) {
            if matches!(pieces[j].1, Piece::Semicolon) && newlines(span.end..pieces[j].0.start) == 0
            {
                last = j;
            }
        }
        if let Some(j) = next(last) {
            if matches!(pieces[j].1, Piece::Comment(_))
                && newlines(pieces[last].0.end..pieces[j].0.start) == 0
            {
                last = j;
            }
        }
        let unit_span = pieces[first].0.start..pieces[last].0.end;
        if !starts_line(unit_span.start) || !ends_line(unit_span.end) {
            fixed[i] = Some(Fixed::SharedLine);
            continue;
        }
        let doc_access = (first..i).any(|j| match pieces[j].1 {
            Piece::Comment(c) => doc_access(tree, c),
            _ => false,
        });
        if doc_access {
            fixed[i] = Some(Fixed::Ineligible(Ineligible::DocAccess));
            continue;
        }
        for o in &mut owner[first..=last] {
            *o = Some(units.len());
        }
        units.push((
            Unit {
                span: unit_span,
                kind: member.kind,
                name: member.name.clone(),
                access: member.access,
            },
            first,
        ));
    }

    // Walk the body: a run grows while units of one kind follow each other
    // with only whitespace between; anything fixed, or a unit of the other
    // kind, ends it. A blank line ends a run of properties too: authors
    // group properties by hand (injections, then data), and the groups
    // keep to themselves.
    let mut runs = Vec::new();
    let mut alone = 0;
    let mut current: Vec<Unit> = Vec::new();
    let mut close = |current: &mut Vec<Unit>, runs: &mut Vec<Run>| match current.len() {
        0 => {}
        1 => {
            alone += 1;
            current.clear();
        }
        _ => {
            let units = std::mem::take(current);
            runs.push(Run {
                kind: units[0].kind,
                separators: units
                    .windows(2)
                    .map(|w| w[0].span.end..w[1].span.start)
                    .collect(),
                units,
            });
        }
    };
    let mut fixed_out = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        if let Some(u) = owner[i] {
            let unit = &units[u].0;
            let ends_run = |c: &Unit| {
                c.kind != unit.kind
                    || (unit.kind == MemberKind::Property
                        && newlines(c.span.end..unit.span.start) > 1)
            };
            if current.last().is_some_and(ends_run) {
                close(&mut current, &mut runs);
            }
            current.push(unit.clone());
            while i < pieces.len() && owner[i] == Some(u) {
                i += 1;
            }
            continue;
        }
        // Anything but whitespace is fixed and ends the run.
        if !matches!(pieces[i].1, Piece::Space) {
            close(&mut current, &mut runs);
            if want_fixed {
                let why = match (&pieces[i].1, fixed[i]) {
                    (_, Some(f)) => f,
                    (Piece::Comment(c), None) if is_banner(tree, c) => Fixed::Banner,
                    (Piece::Comment(_), None) => Fixed::Comment,
                    (Piece::Semicolon, None) => Fixed::Statement,
                    (Piece::Space | Piece::Member(_) | Piece::Fixed(_), None) => {
                        unreachable!("an unowned member is fixed")
                    }
                };
                fixed_out.push((pieces[i].0.clone(), why));
            }
        }
        i += 1;
    }
    close(&mut current, &mut runs);
    Body {
        span: el.span.clone(),
        skipped: None,
        runs,
        alone,
        fixed: fixed_out,
    }
}

/// What a child of a component body is.
fn classify<'a>(tree: &Tree, node: &'a Node, opts: &ArrangeOptions) -> Piece<'a> {
    let el = match node {
        Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline) => {
            return Piece::Space;
        }
        Node::Token(t) if tree.text(t).bytes().all(|b| b.is_ascii_whitespace()) => {
            return Piece::Space;
        }
        Node::Token(_) => return Piece::Fixed(Fixed::Other),
        Node::Element(e) => e,
    };
    let member = |m: Result<Member, Ineligible>| match m {
        Ok(m) => Piece::Member(m),
        Err(why) => Piece::Fixed(Fixed::Ineligible(why)),
    };
    match &el.kind {
        k if k.is_comment() => Piece::Comment(el),
        ElementKind::Ignore => Piece::Fixed(Fixed::Ignore),
        ElementKind::Statement(StatementKind::Function) => member(script_function(tree, el)),
        ElementKind::Statement(StatementKind::Property) if opts.properties => {
            member(script_property(tree, el))
        }
        ElementKind::Statement(StatementKind::Property) => Piece::Fixed(Fixed::Property),
        ElementKind::Statement(StatementKind::Empty) if tree.slice(el.span.clone()) == ";" => {
            Piece::Semicolon
        }
        ElementKind::Statement(_) => Piece::Fixed(Fixed::Statement),
        ElementKind::TagBody { cf: true } if el.cf_kind() == Some(CfKind::Function) => {
            member(tag_function(tree, el))
        }
        ElementKind::CfTag(TagShape::Open | TagShape::SelfClosed, CfKind::Property) => {
            if opts.properties {
                member(tag_property(tree, el))
            } else {
                Piece::Fixed(Fixed::Property)
            }
        }
        _ => Piece::Fixed(Fixed::Other),
    }
}

/// A script function's name and access.
fn script_function(tree: &Tree, stmt: &Element) -> Result<Member, Ineligible> {
    // A bodiless function (an interface's) is its header alone.
    let decl = stmt
        .children
        .iter()
        .filter_map(Node::as_element)
        .find_map(|e| match e.kind {
            ElementKind::FunctionDecl => Some(e),
            ElementKind::Function { arrow: false } => e.as_decl()?.header().as_element(),
            _ => None,
        })
        .filter(|e| e.kind == ElementKind::FunctionDecl)
        .ok_or(Ineligible::Name)?;
    let name = decl
        .children
        .iter()
        .filter_map(Node::as_token)
        .find(|t| t.kind == TokenKind::Ident(Ident::FunctionName))
        .map(|t| tree.text(t).to_owned())
        .ok_or(Ineligible::Name)?;
    // The modifiers come in any order (`static private function`); the
    // access word among them, if any, is the access.
    let modifiers = decl
        .children
        .iter()
        .filter_map(Node::as_token)
        .take_while(|t| t.kind != TokenKind::Keyword(Keyword::Function))
        .filter(|t| t.kind == TokenKind::Storage(Storage::Modifier))
        .filter_map(|t| Access::parse(tree.text(t)))
        .map(Ok);
    let access = resolve_access(
        modifiers.chain(
            attributes(tree, decl, "access")
                .map(|v| v.and_then(Access::parse).ok_or(Ineligible::Access)),
        ),
    )?;
    Ok(Member {
        kind: MemberKind::Function,
        name,
        access,
    })
}

/// A `cffunction`'s name and access, from its opening tag's attributes.
fn tag_function(tree: &Tree, body: &Element) -> Result<Member, Ineligible> {
    let open = body.open_tag().ok_or(Ineligible::Name)?;
    let name = single(attributes(tree, open, "name")).ok_or(Ineligible::Name)?;
    let access = resolve_access(
        attributes(tree, open, "access")
            .map(|v| v.and_then(Access::parse).ok_or(Ineligible::Access)),
    )?;
    Ok(Member {
        kind: MemberKind::Function,
        name,
        access,
    })
}

/// The one access every source agrees on; `public` when none gives one.
fn resolve_access(
    sources: impl Iterator<Item = Result<Access, Ineligible>>,
) -> Result<Access, Ineligible> {
    let mut found = None;
    for access in sources {
        let access = access?;
        match found {
            Some(a) if a != access => return Err(Ineligible::Disagree),
            _ => found = Some(access),
        }
    }
    Ok(found.unwrap_or(Access::Public))
}

/// A script property's name: its `name` attribute, or the shorthand's
/// (`property string foo;`).
fn script_property(tree: &Tree, stmt: &Element) -> Result<Member, Ineligible> {
    let prop = stmt
        .children
        .iter()
        .filter_map(Node::as_element)
        .find(|e| e.kind == ElementKind::Property)
        .unwrap_or(stmt);
    let shorthand: Vec<String> = prop
        .children
        .iter()
        .filter_map(Node::as_token)
        .filter(|t| t.kind == TokenKind::Ident(Ident::PropertyName))
        .map(|t| tree.text(t).to_owned())
        .collect();
    let attributes: Vec<Option<&str>> = attributes(tree, prop, "name").collect();
    let name = match (attributes.as_slice(), shorthand.as_slice()) {
        ([], [name]) => name.clone(),
        ([Some(name)], []) => (*name).to_owned(),
        ([_, ..], [_, ..]) => return Err(Ineligible::TwoNames),
        _ => return Err(Ineligible::PropertyName),
    };
    Ok(Member {
        kind: MemberKind::Property,
        name,
        access: Access::Public,
    })
}

/// A `cfproperty`'s name attribute.
fn tag_property(tree: &Tree, tag: &Element) -> Result<Member, Ineligible> {
    let name = single(attributes(tree, tag, "name")).ok_or(Ineligible::PropertyName)?;
    Ok(Member {
        kind: MemberKind::Property,
        name,
        access: Access::Public,
    })
}

/// The one plain value among `values`, if there is exactly one.
fn single<'a>(mut values: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    match (values.next(), values.next()) {
        (Some(v), None) => v.map(str::to_owned),
        _ => None,
    }
}

/// The values of `el`'s attributes (its `KeyValue` children) named `key`,
/// ASCII case-insensitively: `Some` when the value is plain
/// ([`cfparse::nodes::plain_text`]), `None` when it is not or is missing.
fn attributes<'a>(
    tree: &'a Tree,
    el: &'a Element,
    key: &'a str,
) -> impl Iterator<Item = Option<&'a str>> + 'a {
    el.children
        .iter()
        .filter_map(Node::as_element)
        .filter_map(Element::as_key_value)
        .filter(move |kv| {
            matches!(kv.key(), Node::Token(name)
                if name.kind == TokenKind::Ident(Ident::AttributeName)
                    && tree.text(name).eq_ignore_ascii_case(key))
        })
        .map(move |kv| kv.plain_text(tree).map(|(text, _)| text))
}

/// A one-line comment whose words are framed by three or more of one rule
/// character at each end: `// ---- PRIVATE ----`, `/***** Helpers *****/`,
/// `/* === Getters === */`, `<!------ PUBLIC ------>`. A rule line with no
/// words (`/*******/`) is not one: it often frames a function's own header.
fn is_banner(tree: &Tree, comment: &Element) -> bool {
    fn framed(s: &str) -> bool {
        let s = s.trim();
        let rule = |c: char| "-=*#~_+/".contains(c);
        let (Some(first), Some(last)) = (s.chars().next(), s.chars().last()) else {
            return false;
        };
        rule(first)
            && rule(last)
            && s.starts_with(&first.to_string().repeat(3))
            && s.ends_with(&last.to_string().repeat(3))
            && s.chars().any(char::is_alphanumeric)
    }
    let text = tree.slice(comment.span.clone());
    if text.contains('\n') {
        return false;
    }
    if let Some(inner) = text
        .strip_prefix("<!---")
        .and_then(|t| t.strip_suffix("--->"))
    {
        return framed(inner);
    }
    if let Some(inner) = text.strip_prefix("//") {
        return framed(inner);
    }
    // `/*** Title ***/` frames with the stars of its own delimiters;
    // `/* --- Title --- */` inside them.
    text.strip_prefix('/')
        .and_then(|t| t.strip_suffix('/'))
        .is_some_and(|t| {
            framed(t)
                || t.strip_prefix('*')
                    .and_then(|t| t.strip_suffix('*'))
                    .is_some_and(framed)
        })
}

/// A doc comment with an `@access` tag.
fn doc_access(tree: &Tree, comment: &Element) -> bool {
    comment.kind == ElementKind::DocComment
        && comment
            .children
            .iter()
            .filter_map(Node::as_token)
            .any(|t| t.kind == TokenKind::DocTag && tree.text(t).eq_ignore_ascii_case("@access"))
}
