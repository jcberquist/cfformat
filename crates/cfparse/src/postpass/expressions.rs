//! The expression pass: a Pratt parse of every expression
//! run into `Assignment`, `Ternary`, `Binary`, `Unary`, `CallExpr`, `New`,
//! `Chain` and `Segment` elements.
//!
//! # Runs
//!
//! Every node list (element children and item children, at every depth) is
//! split into *runs*: maximal sequences of operands, postfix parts, operators
//! (tokens, and `Phrase` elements for the multi-word ones), `new` and
//! trivia. Anything else bounds a run — keywords (`return`,
//! `var`, `case`), `Punct` tokens other than accessors (the `;` separator of a
//! `for` header, `Punct(KeyValue)`, `Punct(Colon)`, commas), tag and attribute
//! names, and every element that is not an operand (statements, keyword
//! elements, blocks, parameters, key-values, tags, islands). Operands are
//! identifier and literal tokens and the `String`, `Struct`, `Array`,
//! `TypedArray`, `Group`, `Function` and `TemplateExpression` elements;
//! postfix parts are `Call` and `Brackets` elements, accessor tokens with the
//! name after them, and `op.postfix`.
//!
//! A run is parsed as a sequence of expressions (`a b` is two lone operands).
//! If any of them fails — an operator with no operand, a `Call` with no
//! callee, a ternary without its `:` — the whole run is left exactly as it
//! was: the pass is total and never drops or reorders a node. A lone operand
//! gets no wrapper.
//!
//! # Trivia
//!
//! A node built from `[first part ..= last part]` takes every node in between
//! (whitespace, newlines, comments) as a child, in span order; trivia before
//! the first or after the last part stays in the parent.
//!
//! # Precedence
//!
//! Loosest first. The lexer gives every operator token its kind, a binary
//! one its [`Prec`] (the script lexer's `BINARY_OPERATORS` table), and the
//! pass reads the kind alone, never the text. The levels are Lucee's, one
//! per level of its expression parser, checked against the precedence
//! Adobe ColdFusion gives:
//!
//! | level | operators | assoc |
//! |---|---|---|
//! | assignment | `=`; `+= -= *= /= %= &=` (see below) | right |
//! | conditional | `? :`, `?:` | right |
//! | `imp` | `imp` | left |
//! | `eqv` | `eqv` | left |
//! | `xor` | `xor` | left |
//! | `or` | `or` <code>&#124;&#124;</code> | left |
//! | `and` | `and &&` | left |
//! | not | prefix `not !` | prefix |
//! | comparison | `eq neq is "is not" == != <> === !== lt lte le gt gte ge < <= > >= "less than" "greater than" "less than or equal to" "greater than or equal to" contains "does not contain" in` | left |
//! | concat | `&` | left |
//! | additive | `+ -` | left |
//! | modulus | `% mod` | left |
//! | multiplicative | `* / \` | left |
//! | exponent | `^` | left |
//! | prefix | `- + ++ -- ...` | prefix |
//! | postfix | `a++ a--`, calls, member access, index | postfix |
//!
//! Where the sources disagree, Lucee wins: Adobe gives `\` and `MOD` levels of
//! their own below `* /` (Lucee: `\` with `* /`, `% mod` one level looser),
//! and lists `||` with `XOR` rather than `OR`. Both put unary `+ -` above
//! `^` (`-a ^ 2` is `(-a) ^ 2`) and `NOT` between the comparisons and `AND`.
//! Lucee parses the elvis operator and the ternary at one level whose
//! right-hand sides are full assignment expressions (`a ?: b ? c : d` is
//! `a ?: (b ? c : d)`), and an augmented assignment at the level of its
//! arithmetic operator with an assignment expression on its right
//! (`a && b += 1` is `a && (b += 1)`); both are reproduced. `in` is not a
//! Lucee expression operator (`for (k in s)` is statement syntax); it sits
//! with the comparisons. Same-precedence runs are one n-ary `Binary` (the
//! elvis operator included, although it is right-associative).
//!
//! # Depth
//!
//! The parse must not overflow a thread's stack, and neither may the tree it
//! builds: every later pass, the printer and `Drop` walk it recursively. A
//! prefix chain (`!!!x`) is collected in a loop and nested from the operand
//! outwards, and an assignment chain (`a = b = c`) folds
//! from the right, so neither recurses; the recursion that remains (a
//! binary right-hand side, the ternary arms, the elvis operand) counts
//! against the crate's `MAX_DEPTH`. Every expression built records how deep
//! it nests, and a run whose nodes would end up deeper than `MAX_TREE_DEPTH`
//! — its list's depth, plus the tallest node in it, plus the nest — is left
//! flat, like a run that does not parse, and wrapped in a
//! [`Recovered`](ElementKind::Recovered)`(TooDeep)` region: it was not
//! understood, so a formatter prints it as written.

use super::element;
use crate::tree::{
    Element, ElementKind, Ident, Keyword, Literal, Node, Operator, Prec, Punct, RecoveryReason,
    SegmentKind, StatementKind, Storage, Token, TokenKind,
};
use crate::{MAX_DEPTH, MAX_TREE_DEPTH};

// Binding powers: higher binds tighter.
const ASSIGN: u8 = 1;
const TERNARY: u8 = 2;
const NOT: u8 = 8;
const PREFIX: u8 = 15;

fn binding(prec: Prec) -> u8 {
    match prec {
        Prec::Elvis => TERNARY,
        Prec::Imp => 3,
        Prec::Eqv => 4,
        Prec::Xor => 5,
        Prec::Or => 6,
        Prec::And => 7,
        Prec::Comparison => 9,
        Prec::Concat => 10,
        Prec::Additive => 11,
        Prec::Modulus => 12,
        Prec::Multiplicative => 13,
        Prec::Exponent => 14,
    }
}

/// Build expression nodes in every node list of `el` and below.
pub fn build_expressions(el: &mut Element) {
    build(el, 1, &mut Scratch::default());
}

/// Buffers the pass reuses from list to list rather than allocating per
/// list. `heights` is a stack: a list's entries stay while the lists inside
/// its nodes are built above them. The others belong to the one list being
/// parsed ([`parse_list`] builds no other list): its classes, its
/// significant nodes, a run's expressions, the list's, and its nodes while
/// the built ones are put together.
#[derive(Default)]
struct Scratch {
    heights: Vec<usize>,
    classes: Vec<Class>,
    sig: Vec<usize>,
    run: Vec<Expr>,
    exprs: Vec<Expr>,
    slots: Vec<Option<Node>>,
}

/// [`parse_list`] over every list of `el` and below, innermost first: each
/// element's `children` and each item's `children`; a
/// [`Recovered`](ElementKind::Recovered) region is left as it is. `depth` is
/// `el`'s own (the root's is 1). Returns a bound on `el`'s height — the
/// elements on the longest path down from it, itself included — after the
/// lists are built.
fn build(el: &mut Element, depth: usize, s: &mut Scratch) -> usize {
    if matches!(el.kind, ElementKind::Recovered(_)) {
        return height(el);
    }
    let mut height = 0;
    let children = s.heights.len();
    push_heights(&mut el.children, depth, s);
    for item in el.items.iter_mut() {
        for node in item.leading.iter_mut().chain(item.trailing.iter_mut()) {
            if let Node::Element(child) = node {
                height = height.max(build(child, depth + 1, s));
            }
        }
        let own = s.heights.len();
        push_heights(&mut item.children, depth, s);
        height = height.max(parse_list(&mut item.children, depth, own, s));
        s.heights.truncate(own);
    }
    height = height.max(parse_list(&mut el.children, depth, children, s));
    s.heights.truncate(children);
    if let ElementKind::Statement(kind) = &mut el.kind {
        expression_statement(kind, &el.children);
    }
    height + 1
}

/// The kind of an expression statement once its run is built: the parser
/// gave it `Expression`, or `Function` for a run starting with a function.
/// A run built into an assignment makes it `Assignment`; a function built
/// into something larger (`function() {}()`) is an `Expression` again.
fn expression_statement(kind: &mut StatementKind, children: &[Node]) {
    if !matches!(kind, StatementKind::Expression | StatementKind::Function) {
        return;
    }
    let first = children.iter().find(|n| !n.is_trivia());
    match first.and_then(Node::as_element).map(|e| &e.kind) {
        Some(ElementKind::Assignment) => *kind = StatementKind::Assignment,
        Some(
            ElementKind::Function { .. } | ElementKind::FunctionDecl | ElementKind::ArrowFunction,
        ) => {}
        _ => *kind = StatementKind::Expression,
    }
}

/// `el`'s height, unbuilt (a [`Recovered`](ElementKind::Recovered) region).
fn height(el: &Element) -> usize {
    1 + el
        .nodes()
        .filter_map(Node::as_element)
        .map(height)
        .max()
        .unwrap_or(0)
}

/// Push the height of each node of a list whose owner is `depth` deep: a
/// token's is 0, an element's is what [`build`] returns for it.
fn push_heights(nodes: &mut [Node], depth: usize, s: &mut Scratch) {
    for node in nodes.iter_mut() {
        let h = match node {
            Node::Element(child) => build(child, depth + 1, s),
            Node::Token(_) => 0,
        };
        s.heights.push(h);
    }
}

/// What a node can be in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Trivia,
    Operand,
    /// `Call` element.
    Call,
    /// `Brackets` element.
    Index,
    /// `.`, `?.`, `::`.
    Accessor,
    Operator(Operator),
    New,
    Boundary,
}

fn classify(node: &Node) -> Class {
    if node.is_trivia() {
        return Class::Trivia;
    }
    match node {
        Node::Token(t) => match t.kind {
            TokenKind::Operator(op) => Class::Operator(op),
            TokenKind::Keyword(Keyword::New) => Class::New,
            TokenKind::Punct(Punct::Accessor | Punct::SafeAccessor | Punct::StaticAccessor) => {
                Class::Accessor
            }
            TokenKind::Ident(
                Ident::Variable
                | Ident::ScopeVar
                | Ident::This
                | Ident::Super
                | Ident::Call
                | Ident::Builtin
                | Ident::ClassName
                | Ident::PropertyName
                | Ident::Parameter,
            )
            | TokenKind::Literal(
                Literal::Number
                | Literal::Bool
                | Literal::Null
                | Literal::Unquoted
                | Literal::Constant
                | Literal::StringText
                | Literal::EscapeHash,
            ) => Class::Operand,
            _ => Class::Boundary,
        },
        Node::Element(e) => match e.kind {
            ElementKind::String { .. }
            | ElementKind::Struct { .. }
            | ElementKind::Array
            | ElementKind::TypedArray
            | ElementKind::Group
            | ElementKind::Function { .. }
            | ElementKind::TemplateExpression
            // Only the parser builds one this early: an inline component.
            | ElementKind::New => Class::Operand,
            // A multi-word operator: its words' kind.
            ElementKind::Phrase => match e.children.first() {
                Some(Node::Token(Token {
                    kind: TokenKind::Operator(op),
                    ..
                })) => Class::Operator(*op),
                _ => Class::Boundary,
            },
            ElementKind::Call => Class::Call,
            ElementKind::Brackets => Class::Index,
            _ => Class::Boundary,
        },
    }
}

/// A parsed expression over node indices `lo..=hi` of the list. `kind: None`
/// is a single existing node. `subs` are the nested expressions that become
/// children, in order; every other index in the range is taken as is.
#[derive(Debug)]
struct Expr {
    kind: Option<ElementKind>,
    lo: usize,
    hi: usize,
    subs: Vec<Expr>,
    /// How many built elements nest here: 0 for a leaf, one more than the
    /// deepest of `subs` otherwise.
    nest: usize,
}

impl Expr {
    fn leaf(i: usize) -> Expr {
        Expr {
            kind: None,
            lo: i,
            hi: i,
            subs: Vec::new(),
            nest: 0,
        }
    }

    fn node(kind: ElementKind, lo: usize, hi: usize, subs: Vec<Expr>) -> Expr {
        let nest = 1 + subs.iter().map(|s| s.nest).max().unwrap_or(0);
        Expr {
            kind: Some(kind),
            lo,
            hi,
            subs,
            nest,
        }
    }
}

/// The parse failed; the run stays flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    /// An operator with no operand, a `Call` with no callee, a ternary
    /// without its `:`.
    Syntax,
    /// The run nests past [`MAX_DEPTH`] or would build a tree deeper than
    /// [`MAX_TREE_DEPTH`].
    TooDeep,
}

struct Parser<'a> {
    nodes: &'a [Node],
    classes: &'a [Class],
    /// Indices (into `nodes`) of the run's non-trivia nodes.
    sig: &'a [usize],
    pos: usize,
    /// Nesting of [`Parser::expr`], bounded by [`MAX_DEPTH`].
    depth: u32,
    /// How deep the run's nodes sit before anything is built: the list's
    /// depth plus the tallest node in the run. An expression nesting
    /// `MAX_TREE_DEPTH - base` elements is too deep.
    base: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<usize> {
        self.sig.get(self.pos).copied()
    }

    /// The operator at `i`, if it is one.
    fn operator_at(&self, i: usize) -> Option<Operator> {
        match self.classes[i] {
            Class::Operator(op) => Some(op),
            _ => None,
        }
    }

    /// An expression binding at least as tightly as `min`.
    fn expr(&mut self, min: u8) -> Result<Expr, Fail> {
        self.nested(|p| {
            let left = p.prefix()?;
            p.infix(left, min, true)
        })
    }

    /// `f` one level deeper, bounded by [`MAX_DEPTH`].
    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Result<Expr, Fail>) -> Result<Expr, Fail> {
        if self.depth >= MAX_DEPTH {
            return Err(Fail::TooDeep);
        }
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        out
    }

    /// `e`, unless it nests too deep for the tree.
    fn fits(&self, e: Expr) -> Result<Expr, Fail> {
        if self.base + e.nest > MAX_TREE_DEPTH {
            return Err(Fail::TooDeep);
        }
        Ok(e)
    }

    /// The operators after `left` that bind at least as tightly as `min`.
    /// Without `assign`, an assignment operator ends the expression: it is
    /// [`Parser::assignments`]' to fold.
    fn infix(&mut self, mut left: Expr, min: u8, assign: bool) -> Result<Expr, Fail> {
        while let Some(i) = self.peek() {
            let op = self.operator_at(i);
            let level = match op {
                Some(Operator::Binary(prec)) => Some(prec),
                Some(Operator::In) => Some(Prec::Comparison),
                _ => None,
            };
            match (op, level) {
                (_, Some(prec)) if binding(prec) >= min => {
                    self.pos += 1;
                    let rhs_min = if prec == Prec::Elvis {
                        ASSIGN
                    } else {
                        binding(prec) + 1
                    };
                    let rhs = self.expr(rhs_min)?;
                    left = self.fits(binary(left, rhs, prec))?;
                }
                (Some(Operator::AugAssign(prec)), _) if assign && binding(prec) >= min => {
                    left = self.assignments(left)?;
                }
                (Some(Operator::Assign), _) if assign && ASSIGN >= min => {
                    left = self.assignments(left)?;
                }
                (Some(Operator::TernaryQ), _) if TERNARY >= min => {
                    self.pos += 1;
                    let then = self.expr(ASSIGN)?;
                    match self.peek() {
                        Some(c) if self.classes[c] == Class::Operator(Operator::TernaryColon) => {
                            self.pos += 1;
                        }
                        _ => return Err(Fail::Syntax),
                    }
                    let otherwise = self.expr(ASSIGN)?;
                    let (lo, hi) = (left.lo, otherwise.hi);
                    left = self.fits(Expr::node(
                        ElementKind::Ternary,
                        lo,
                        hi,
                        vec![left, then, otherwise],
                    ))?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// `target = value`, `target += value`, at an assignment operator. The
    /// value is an assignment expression, so `a = b = c += d` is `a = (b =
    /// (c += d))`: each operand up to the next assignment operator is read in
    /// a loop and the chain folds from the right.
    fn assignments(&mut self, target: Expr) -> Result<Expr, Fail> {
        let mut targets = vec![target];
        loop {
            self.pos += 1;
            let value = self.nested(|p| {
                let left = p.prefix()?;
                p.infix(left, ASSIGN, false)
            })?;
            let more = self.peek().is_some_and(|i| {
                matches!(
                    self.operator_at(i),
                    Some(Operator::Assign | Operator::AugAssign(_))
                )
            });
            if more {
                targets.push(value);
                continue;
            }
            let mut value = value;
            while let Some(target) = targets.pop() {
                value = self.fits(assignment(target, value))?;
            }
            return Ok(value);
        }
    }

    /// An operand with its prefix operators and postfix parts. A prefix
    /// operator's operand is the expression after it at the operator's
    /// binding power, so a chain (`not not x`, `- -x`) is collected first and
    /// nested from the operand outwards.
    fn prefix(&mut self) -> Result<Expr, Fail> {
        let mut ops = Vec::new();
        loop {
            let i = self.peek().ok_or(Fail::Syntax)?;
            if !matches!(self.classes[i], Class::Operator(_)) {
                break;
            }
            let min = match self.operator_at(i) {
                Some(Operator::Sign | Operator::Increment | Operator::Spread) => PREFIX,
                Some(Operator::Not { .. }) => NOT,
                _ => return Err(Fail::Syntax),
            };
            self.pos += 1;
            ops.push((i, min));
        }
        let mut e = self.primary()?;
        while let Some((i, min)) = ops.pop() {
            let operand = self.infix(e, min, true)?;
            let hi = operand.hi;
            e = self.fits(Expr::node(
                ElementKind::Unary { postfix: false },
                i,
                hi,
                vec![operand],
            ))?;
        }
        Ok(e)
    }

    /// An operand (or `new`) and its postfix parts.
    fn primary(&mut self) -> Result<Expr, Fail> {
        let i = self.peek().ok_or(Fail::Syntax)?;
        self.pos += 1;
        match self.classes[i] {
            Class::New => {
                let name = self.peek().ok_or(Fail::Syntax)?;
                let is_name = match &self.nodes[name] {
                    Node::Token(t) => matches!(
                        t.kind,
                        TokenKind::Ident(_)
                            | TokenKind::Storage(Storage::Type)
                            | TokenKind::Keyword(Keyword::Component)
                    ),
                    Node::Element(e) => matches!(e.kind, ElementKind::String { .. }),
                };
                if !is_name {
                    return Err(Fail::Syntax);
                }
                self.pos += 1;
                let mut hi = name;
                if let Some(call) = self.peek().filter(|&c| self.classes[c] == Class::Call) {
                    self.pos += 1;
                    hi = call;
                }
                let new = Expr::node(ElementKind::New, i, hi, Vec::new());
                self.postfix(new)
            }
            Class::Operand => self.postfix(Expr::leaf(i)),
            _ => Err(Fail::Syntax),
        }
    }

    fn postfix(&mut self, mut cur: Expr) -> Result<Expr, Fail> {
        while let Some(i) = self.peek() {
            match self.classes[i] {
                Class::Call => {
                    self.pos += 1;
                    let method = cur.kind == Some(ElementKind::Chain)
                        && cur.subs.last().is_some_and(|s| {
                            s.kind == Some(ElementKind::Segment(SegmentKind::Property))
                        });
                    if method {
                        let seg = cur.subs.last_mut().unwrap();
                        seg.kind = Some(ElementKind::Segment(SegmentKind::Method));
                        seg.hi = i;
                        cur.hi = i;
                    } else {
                        let lo = cur.lo;
                        cur = self.fits(Expr::node(ElementKind::CallExpr, lo, i, vec![cur]))?;
                    }
                }
                Class::Index => {
                    self.pos += 1;
                    let seg =
                        Expr::node(ElementKind::Segment(SegmentKind::Index), i, i, Vec::new());
                    cur = self.fits(push_segment(cur, seg))?;
                }
                Class::Accessor => {
                    let name = self.sig.get(self.pos + 1).copied().ok_or(Fail::Syntax)?;
                    let is_name = matches!(
                        &self.nodes[name],
                        Node::Token(t) if matches!(
                            t.kind,
                            TokenKind::Ident(_) | TokenKind::Literal(_) | TokenKind::Keyword(_)
                        )
                    );
                    if !is_name {
                        return Err(Fail::Syntax);
                    }
                    self.pos += 2;
                    let seg = Expr::node(
                        ElementKind::Segment(SegmentKind::Property),
                        i,
                        name,
                        Vec::new(),
                    );
                    cur = self.fits(push_segment(cur, seg))?;
                }
                Class::Operator(Operator::Postfix) => {
                    self.pos += 1;
                    let lo = cur.lo;
                    return self.fits(Expr::node(
                        ElementKind::Unary { postfix: true },
                        lo,
                        i,
                        vec![cur],
                    ));
                }
                _ => break,
            }
        }
        Ok(cur)
    }
}

fn push_segment(cur: Expr, seg: Expr) -> Expr {
    if cur.kind == Some(ElementKind::Chain) {
        let mut chain = cur;
        chain.hi = seg.hi;
        chain.nest = chain.nest.max(seg.nest + 1);
        chain.subs.push(seg);
        chain
    } else {
        let (lo, hi) = (cur.lo, seg.hi);
        Expr::node(ElementKind::Chain, lo, hi, vec![cur, seg])
    }
}

fn binary(left: Expr, rhs: Expr, prec: Prec) -> Expr {
    let kind = ElementKind::Binary { prec };
    let mut node = if left.kind.as_ref() == Some(&kind) {
        left
    } else {
        let lo = left.lo;
        Expr::node(kind.clone(), lo, left.hi, vec![left])
    };
    node.hi = rhs.hi;
    // The elvis operator is right-associative: `a ?: b ?: c` parses its
    // right-hand side as `b ?: c`, which joins the same n-ary node.
    if rhs.kind.as_ref() == Some(&kind) {
        node.nest = node.nest.max(rhs.nest);
        node.subs.extend(rhs.subs);
    } else {
        node.nest = node.nest.max(rhs.nest + 1);
        node.subs.push(rhs);
    }
    node
}

fn assignment(target: Expr, value: Expr) -> Expr {
    let (lo, hi) = (target.lo, value.hi);
    Expr::node(ElementKind::Assignment, lo, hi, vec![target, value])
}

/// Build the expressions of one list, whose owner is `depth` deep and whose
/// nodes are as tall as `s.heights[from..]` says ([`push_heights`]).
/// Returns the tallest node of the list afterwards (a bound: a built
/// expression counts its nest above the tallest node of its run).
fn parse_list(nodes: &mut Vec<Node>, depth: usize, from: usize, s: &mut Scratch) -> usize {
    let Scratch {
        heights,
        classes,
        sig,
        run,
        exprs,
        slots,
    } = s;
    let heights = &heights[from..];
    let tallest = heights.iter().copied().max().unwrap_or(0);
    let builds = |c: Class| {
        matches!(
            c,
            Class::Operator(_) | Class::Call | Class::Index | Class::Accessor | Class::New
        )
    };
    if !nodes.iter().any(|n| builds(classify(n))) {
        return tallest;
    }
    classes.clear();
    classes.extend(nodes.iter().map(classify));
    sig.clear();
    sig.extend((0..nodes.len()).filter(|&i| classes[i] != Class::Trivia));
    let (classes, sig) = (&mut classes[..], &sig[..]);
    // `new java(…)` / `new component(…)`: the class name is a type or
    // keyword token, part of the constructor rather than a boundary.
    for w in sig.windows(2) {
        let name_like = matches!(
            &nodes[w[1]],
            Node::Token(t) if matches!(
                t.kind,
                TokenKind::Storage(Storage::Type) | TokenKind::Keyword(Keyword::Component)
            )
        );
        if classes[w[0]] == Class::New && name_like {
            classes[w[1]] = Class::Operand;
        }
    }
    exprs.clear();
    let mut built = tallest;
    let mut start = 0;
    while start < sig.len() {
        if classes[sig[start]] == Class::Boundary {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < sig.len() && classes[sig[end]] != Class::Boundary {
            end += 1;
        }
        // Everything from the run's first node to its last: trivia between
        // its parts becomes a child of what is built too.
        let run_tallest = heights[sig[start]..=sig[end - 1]]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let mut parser = Parser {
            nodes,
            classes,
            sig: &sig[start..end],
            pos: 0,
            depth: 0,
            base: depth + run_tallest,
        };
        run.clear();
        let parsed = loop {
            if parser.pos == parser.sig.len() {
                break Ok(());
            }
            match parser.expr(0) {
                Ok(e) => run.push(e),
                Err(fail) => break Err(fail),
            }
        };
        match parsed {
            Ok(()) => {
                for e in run.drain(..).filter(|e| e.kind.is_some()) {
                    built = built.max(run_tallest + e.nest);
                    exprs.push(e);
                }
            }
            // Flat, and a recovery: the run was not understood, so it
            // prints as written.
            Err(Fail::TooDeep) => {
                built = built.max(run_tallest + 1);
                exprs.push(Expr::node(
                    ElementKind::Recovered(RecoveryReason::TooDeep),
                    sig[start],
                    sig[end - 1],
                    Vec::new(),
                ));
            }
            Err(Fail::Syntax) => {}
        }
        start = end;
    }
    if exprs.is_empty() {
        return tallest;
    }

    slots.clear();
    slots.extend(nodes.drain(..).map(Some));
    let mut exprs = exprs.drain(..).peekable();
    let mut i = 0;
    while i < slots.len() {
        match exprs.next_if(|e| e.lo == i) {
            Some(e) => {
                i = e.hi + 1;
                nodes.push(materialize(slots, e));
            }
            None => {
                nodes.push(slots[i].take().expect("node taken once"));
                i += 1;
            }
        }
    }
    built
}

/// The element `e` describes, its parts taken from `slots`. Recursive over
/// `e`, which [`Parser::fits`] kept within [`MAX_TREE_DEPTH`].
fn materialize(slots: &mut [Option<Node>], e: Expr) -> Node {
    let Some(kind) = e.kind else {
        return slots[e.lo].take().expect("node taken once");
    };
    let mut children = Vec::with_capacity(e.hi - e.lo + 1);
    let mut subs = e.subs.into_iter().peekable();
    let mut i = e.lo;
    while i <= e.hi {
        match subs.next_if(|s| s.lo == i) {
            Some(sub) => {
                i = sub.hi + 1;
                children.push(materialize(slots, sub));
            }
            None => {
                children.push(slots[i].take().expect("node taken once"));
                i += 1;
            }
        }
    }
    Node::Element(Box::new(element(kind, children)))
}
