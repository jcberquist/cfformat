//! `missing-var`: a write inside a function to a name with no scope that the
//! function never declares. In classic CFML such a write lands in the
//! `variables` scope, where it outlives the call and is shared by every call
//! on the same component instance.
//!
//! A declaration covers only the code that runs after it: on Lucee a `var`
//! declares when its line runs, so `x = 1; var x = 2;` leaves `x = 1` in
//! `variables`, and so does `y = 2` after `if (false) { var y = 1; }`. The
//! static model is block dominance: each declaration is collected with its
//! offset and the block it runs in, and it covers a write that comes after
//! it inside that block. A conditional that runs exactly one of its
//! branches (an `if` with an `else`, a `switch` with a `default`) declares
//! after itself every name that each of its branches declares. A write
//! whose name the function declares only where it does not cover the write
//! gets a message of its own, since the fix is to move the `var` up and
//! out.
//!
//! A destructuring pattern writes every name it binds (`[a, b] = x` writes
//! `a` and `b`; a default's target, a rest's name and a nested pattern's
//! names included), as the assignment, declaration or `for` header holding
//! it does: `var [a, b] = x` declares both. A pattern parameter's names are
//! not arguments: Adobe ColdFusion 2025 binds them in the `variables` scope
//! on every call, so each is reported with a message of its own.

use std::collections::HashMap;
use std::ops::Range;

use cfparse::nodes::plain_text;
use cfparse::{
    Element, ElementKind, Ident, Keyword, Literal, Node, Operator, Storage, Token, TokenKind, Tree,
};

use super::result_attributes::result_attributes;
use super::{attribute, attributes};
use crate::scope::{self, Unit};
use crate::Report;

/// The rule's id, as reports print it.
pub const RULE: &str = "missing-var";

/// What a write's target starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Head<'a> {
    /// A name with no scope: `x`, `x.y`, `x[1]`. The offset is the name's.
    Unscoped { name: &'a str, offset: u32 },
    /// A name a pattern parameter binds (`function f({x})`), which the
    /// engine puts in the `variables` scope. The offset is the name's.
    PatternParameter { name: &'a str, offset: u32 },
    /// The `local` scope, and the member it writes when that is a literal
    /// name (`local.x`, `local["x"]`). The offset is `local`'s.
    Local {
        member: Option<&'a str>,
        offset: u32,
    },
    /// Any other scope, `this` or `super`.
    Scoped,
    /// Not a name: a call's result, a group, a `#…#`, a string.
    Dynamic,
}

/// One declaration of a name: where it stands, and the block it covers.
struct Declaration {
    offset: u32,
    block: Range<u32>,
}

impl Declaration {
    fn covers(&self, write: u32) -> bool {
        self.offset <= write && self.block.contains(&write)
    }

    /// Stands in `branch` and covers the rest of it: its block is the
    /// branch (or holds it), not a block nested in it.
    fn declares_in(&self, branch: &Range<u32>) -> bool {
        branch.contains(&self.offset)
            && self.block.start <= branch.start
            && branch.end <= self.block.end
    }
}

/// Every unscoped write the rule objects to, over every unit.
pub(crate) fn check(tree: &Tree, units: &[Unit<'_>]) -> Vec<Report> {
    // Per unit, every declaration of each name (lower-cased): a name
    // declared in both branches of an `if` has two, each covering its own
    // branch.
    let mut declared: Vec<HashMap<String, Vec<Declaration>>> = Vec::with_capacity(units.len());
    let mut reports = Vec::new();
    for unit in units {
        // A closure sees the enclosing function's `local` and `arguments`,
        // and it runs only once it is called, by which time the enclosing
        // function may have declared everything: every inherited name covers
        // the whole closure, wherever its `var` stands in the enclosing
        // function.
        let whole = unit.el.span.clone();
        let mut names: HashMap<String, Vec<Declaration>> = unit
            .parent
            .map(|p| {
                declared[p]
                    .keys()
                    .map(|k| {
                        let inherited = Declaration {
                            offset: whole.start,
                            block: whole.clone(),
                        };
                        (k.clone(), vec![inherited])
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Each conditional that runs exactly one of its branches, with the
        // block it runs in, outermost first.
        let mut conditionals = Vec::new();
        scope::walk(tree, unit, &mut |el, block| {
            declarations(tree, el, block, &whole, &mut |name, offset, block| {
                names
                    .entry(name.to_lowercase())
                    .or_default()
                    .push(Declaration { offset, block });
            });
            if let Some(branches) = scope::branches(tree, el) {
                conditionals.push((el.span.end, block.clone(), branches));
            }
        });
        // A name every branch declares has been declared by whichever branch
        // ran, so it is declared after the conditional, in the block that
        // holds it. Innermost first, so that a join inside a branch counts
        // as that branch's declaration.
        for (end, block, branches) in conditionals.into_iter().rev() {
            for list in names.values_mut() {
                if branches
                    .iter()
                    .all(|b| list.iter().any(|d| d.declares_in(b)))
                {
                    list.push(Declaration {
                        offset: end,
                        block: block.clone(),
                    });
                }
            }
        }
        scope::walk(tree, unit, &mut |el, _| {
            for head in writes(tree, el) {
                if let Head::PatternParameter { name, offset } = head {
                    let (line, column) = crate::position(tree, offset);
                    reports.push(Report {
                        line,
                        column,
                        rule: RULE,
                        name: name.to_owned(),
                        var_not_run: false,
                        message: format!(
                            "`{name}` is bound by a destructuring parameter of function `{}`; \
                             the engine puts it in the variables scope",
                            unit.name
                        ),
                    });
                    continue;
                }
                let Head::Unscoped { name, offset } = head else {
                    continue;
                };
                let var_not_run = match names.get(&name.to_lowercase()) {
                    Some(list) if list.iter().any(|d| d.covers(offset)) => continue,
                    Some(_) => true,
                    None => false,
                };
                let (line, column) = crate::position(tree, offset);
                let message = if var_not_run {
                    format!(
                        "`{name}` is written where its `var` may not have run in function `{}`; \
                         on Lucee the write lands in the variables scope",
                        unit.name
                    )
                } else {
                    format!(
                        "`{name}` is written without `var` or a scope in function `{}`; \
                         it lands in the variables scope",
                        unit.name
                    )
                };
                reports.push(Report {
                    line,
                    column,
                    rule: RULE,
                    name: name.to_owned(),
                    var_not_run,
                    message,
                });
            }
        });
        declared.push(names);
    }
    reports
}

/// The names `el` itself declares local to the function (not those of the
/// elements inside it), each with the offset it counts from and the block
/// it covers. `block` is the one `el` runs in, `whole` the unit's span.
fn declarations<'a>(
    tree: &'a Tree,
    el: &'a Element,
    block: &Range<u32>,
    whole: &Range<u32>,
    out: &mut dyn FnMut(&'a str, u32, Range<u32>),
) {
    // `var x = …`, `for (var i = …)`, `for (var k in …)`, `<cfset var x = …>`,
    // from the `var`, to the end of its block. A `for` header runs in the
    // block that holds the loop, so `i` is declared after the loop too.
    let mut var = |name: &'a str, offset: u32| out(name, offset, block.clone());
    var_declarations(tree, &el.children, &mut var);
    for item in &el.items {
        var_declarations(tree, &item.children, &mut var);
    }
    // `local.x = …` is `var x`: an unscoped write afterwards finds it in
    // `local`.
    for head in writes(tree, el) {
        if let Head::Local {
            member: Some(name),
            offset,
        } = head
        {
            out(name, offset, block.clone());
        }
    }
    match el.kind {
        // An unscoped write to a parameter's name sets `arguments.x`; the
        // arguments exist on entry, wherever `<cfargument>` stands.
        ElementKind::Parameters => {
            parameters(tree, el, &mut |name| out(name, whole.start, whole.clone()));
        }
        ElementKind::CfTag(..) if scope::tag_is(tree, el, "cfargument") => {
            if let Some((name, _)) = attribute(tree, el, "name") {
                out(name.trim(), whole.start, whole.clone());
            }
        }
        // A catch variable exists in its catch block alone, which runs only
        // when something throws.
        ElementKind::Catch => {
            for group in el.children.iter().filter_map(Node::as_element) {
                if group.kind == ElementKind::Group {
                    for t in group.children.iter().filter_map(Node::as_token) {
                        if t.kind == TokenKind::Ident(Ident::Variable) {
                            out(tree.text(t), el.span.start, el.span.clone());
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// The name after each `var` in a run of nodes, with the `var`'s offset.
fn var_declarations<'a>(tree: &'a Tree, nodes: &'a [Node], out: &mut dyn FnMut(&'a str, u32)) {
    let mut significant = nodes.iter().filter(|n| !n.is_trivia());
    while let Some(node) = significant.next() {
        if !is_var(tree, node) {
            continue;
        }
        let Some(declared) = significant.next() else {
            break;
        };
        let target = match declared.as_element() {
            Some(e) if e.kind == ElementKind::Assignment => e.as_assignment().map(|a| a.target()),
            Some(e) => e.as_binary().and_then(|b| b.operands().next()),
            None => None,
        }
        .unwrap_or(declared);
        for head in heads_of(tree, target) {
            if let Head::Unscoped { name, .. } = head {
                out(name, node.span().start);
            }
        }
    }
}

/// `var` in script, `storage.type "var"` in `<cfset var x = …>`.
fn is_var(tree: &Tree, node: &Node) -> bool {
    node.as_token().is_some_and(|t| match t.kind {
        TokenKind::Keyword(Keyword::Var) => true,
        TokenKind::Storage(Storage::Type) => tree.text(t).eq_ignore_ascii_case("var"),
        _ => false,
    })
}

/// The parameter names of a `Parameters` element; a closure in a default
/// value has its own.
fn parameters<'a>(tree: &'a Tree, el: &'a Element, out: &mut dyn FnMut(&'a str)) {
    for node in el.nodes() {
        match node {
            Node::Token(t) if t.kind == TokenKind::Ident(Ident::Parameter) => out(tree.text(t)),
            Node::Element(e) if !matches!(e.kind, ElementKind::Function { .. }) => {
                parameters(tree, e, out);
            }
            _ => {}
        }
    }
}

/// The heads of the targets `el` itself writes.
pub(crate) fn writes<'a>(tree: &'a Tree, el: &'a Element) -> Vec<Head<'a>> {
    let mut out = Vec::new();
    match el.kind {
        ElementKind::Assignment => {
            if let Some(a) = el.as_assignment() {
                out.extend(heads_of(tree, a.target()));
            }
        }
        // `function f({x, y = 1})`: the pattern's names are written on
        // entry, into the `variables` scope.
        ElementKind::Parameters => {
            for item in &el.items {
                let target = match item.significant().next() {
                    Some(Node::Element(a)) if a.kind == ElementKind::Assignment => {
                        a.as_assignment().map(|a| a.target())
                    }
                    node => node,
                };
                if let Some(Node::Element(p)) = target {
                    if matches!(p.kind, ElementKind::Pattern { .. }) {
                        pattern_names(p, &mut |t| {
                            out.push(Head::PatternParameter {
                                name: tree.text(t),
                                offset: t.span.start,
                            });
                        });
                    }
                }
            }
        }
        ElementKind::Unary { .. } => {
            if let Some(u) = el.as_unary() {
                if matches!(tree.text(u.op()), "++" | "--") {
                    out.push(head_of(tree, u.operand()));
                }
            }
        }
        // `for (k in s)`: the first clause is `k in s`.
        ElementKind::For => {
            let first = el
                .children
                .iter()
                .filter_map(Node::as_element)
                .find(|e| e.kind == ElementKind::Group)
                .and_then(|g| g.items.first())
                .and_then(|item| item.significant().next())
                .and_then(Node::as_element)
                .and_then(Element::as_binary);
            if let Some(binary) = first {
                let is_in = binary
                    .operators()
                    .next()
                    .is_some_and(|op| {
                        matches!(op, Node::Token(t) if t.kind == TokenKind::Operator(Operator::In))
                    });
                if let (true, Some(operand)) = (is_in, binary.operands().next()) {
                    out.extend(heads_of(tree, operand));
                }
            }
        }
        _ => {}
    }
    if matches!(
        el.kind,
        ElementKind::CfTag(cfparse::TagShape::Open | cfparse::TagShape::SelfClosed, _)
            | ElementKind::ScriptTag { .. }
            | ElementKind::Param
    ) {
        // `param x;` and `param string x = 1;` name the variable with a bare
        // word (the second as an assignment's target, read above).
        if el.kind == ElementKind::Param {
            for t in el.children.iter().filter_map(Node::as_token) {
                if t.kind == TokenKind::Literal(Literal::Unquoted) {
                    out.push(head_of_text(tree.text(t), t.span.start));
                }
            }
        }
        let names = tree.tag_name(el).map_or(&[][..], result_attributes);
        for (key, value) in attributes(el) {
            let key = tree.text(key);
            if names.iter().any(|n| key.eq_ignore_ascii_case(n)) {
                if let Some((text, offset)) = plain_text(tree, value) {
                    out.push(head_of_text(text, offset));
                }
            }
        }
    }
    out
}

/// The heads a write target node writes: every name a destructuring
/// pattern binds, else the one [`head_of`].
fn heads_of<'a>(tree: &'a Tree, node: &'a Node) -> Vec<Head<'a>> {
    match node {
        Node::Element(p) if matches!(p.kind, ElementKind::Pattern { .. }) => {
            let mut out = Vec::new();
            pattern_names(p, &mut |t| {
                out.push(Head::Unscoped {
                    name: tree.text(t),
                    offset: t.span.start,
                });
            });
            out
        }
        node => vec![head_of(tree, node)],
    }
}

/// The name tokens a pattern binds: each item's name, a default's target
/// (never its value), a rest's name, a nested pattern's names; not a
/// rename's key.
fn pattern_names<'a>(pattern: &'a Element, out: &mut dyn FnMut(&'a Token)) {
    fn names<'a>(node: &'a Node, out: &mut dyn FnMut(&'a Token)) {
        match node {
            Node::Token(t) if t.kind == TokenKind::Ident(Ident::Variable) => out(t),
            Node::Token(_) => {}
            Node::Element(e) => match e.kind {
                ElementKind::Pattern { .. } => pattern_names(e, out),
                ElementKind::Assignment => {
                    if let Some(a) = e.as_assignment() {
                        names(a.target(), out);
                    }
                }
                ElementKind::Unary { .. } => {
                    if let Some(u) = e.as_unary() {
                        names(u.operand(), out);
                    }
                }
                _ => {}
            },
        }
    }
    for item in &pattern.items {
        for node in &item.children {
            names(node, out);
        }
    }
}

/// The head of a write target node.
fn head_of<'a>(tree: &'a Tree, node: &'a Node) -> Head<'a> {
    match node {
        Node::Token(t) => head_of_token(tree, t),
        Node::Element(e) => match e.as_chain() {
            Some(chain) => match chain.head() {
                Node::Token(t) => match head_of_token(tree, t) {
                    Head::Local { offset, .. } => Head::Local {
                        member: chain.segments().next().and_then(|s| local_member(tree, s)),
                        offset,
                    },
                    head => head,
                },
                Node::Element(_) => Head::Dynamic,
            },
            None => Head::Dynamic,
        },
    }
}

fn head_of_token<'a>(tree: &'a Tree, t: &Token) -> Head<'a> {
    match t.kind {
        TokenKind::Ident(Ident::Variable) => Head::Unscoped {
            name: tree.text(t),
            offset: t.span.start,
        },
        TokenKind::Ident(Ident::ScopeVar) if tree.text(t).eq_ignore_ascii_case("local") => {
            Head::Local {
                member: None,
                offset: t.span.start,
            }
        }
        TokenKind::Ident(Ident::ScopeVar | Ident::This | Ident::Super) => Head::Scoped,
        TokenKind::Literal(Literal::Unquoted) => head_of_text(tree.text(t), t.span.start),
        _ => Head::Dynamic,
    }
}

/// The member a chain's first segment names: `.x`, or `["x"]` with a
/// literal string.
fn local_member<'a>(tree: &'a Tree, segment: &'a Element) -> Option<&'a str> {
    let segment = segment.as_segment()?;
    if let Some(name) = segment.name() {
        return Some(tree.text(name));
    }
    plain_text(tree, &segment.brackets()?.children).map(|(text, _)| text)
}

/// The head of a variable name written as text (an attribute value, a
/// `param` name): up to the first `.` or `[`. `offset` is the text's.
fn head_of_text(text: &str, offset: u32) -> Head<'_> {
    let lead = text.len() - text.trim_start().len();
    let text = text.trim();
    let end = text.find(['.', '[']).unwrap_or(text.len());
    let head = &text[..end];
    if !is_identifier(head) {
        return Head::Dynamic;
    }
    let lower = head.to_ascii_lowercase();
    if lower == "local" {
        let member = text[end..].strip_prefix('.').map(|rest| {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            &rest[..end]
        });
        return Head::Local {
            member: member.filter(|m| is_identifier(m)),
            offset: offset + lead as u32,
        };
    }
    if cfparse::script::is_scope_name(&lower) || lower == "this" || lower == "super" {
        return Head::Scoped;
    }
    Head::Unscoped {
        name: head,
        offset: offset + lead as u32,
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
mod tests {
    use cfparse::Mode;

    /// `(line, column, name)` of every report on `src`.
    fn found(src: &str) -> Vec<(usize, usize, String)> {
        crate::lint_source(src, Mode::Auto)
            .reports
            .into_iter()
            .map(|r| (r.line, r.column, r.name))
            .collect()
    }

    /// `(line, column, name)` of the reports of a write no declaration covers.
    fn var_not_run(src: &str) -> Vec<(usize, usize, String)> {
        crate::lint_source(src, Mode::Auto)
            .reports
            .into_iter()
            .filter(|r| r.var_not_run)
            .map(|r| (r.line, r.column, r.name))
            .collect()
    }

    fn at(list: &[(usize, usize, &str)]) -> Vec<(usize, usize, String)> {
        list.iter()
            .map(|&(line, column, name)| (line, column, name.to_owned()))
            .collect()
    }

    /// `body` inside a script function, one statement per line from line 2.
    fn script(body: &str) -> String {
        format!("component {{\nfunction f(a) {{\n{body}\n}}\n}}\n")
    }

    /// `body` inside a `<cffunction>`, one tag per line from line 3.
    fn tags(body: &str) -> String {
        format!("<cfcomponent>\n<cffunction name=\"f\">\n{body}\n</cffunction>\n</cfcomponent>\n")
    }

    #[test]
    fn a_var_declares() {
        let src = script(
            "var x = 1;\nx = 2;\nvar y.z = 1;\ny = 2;\nfor (var i = 1; i < 2; i++) {}\n\
             for (var k in s) {}\nk = 1;\nvar w;\nw = 1;",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags("<cfset var x = 1>\n<cfset x = 2>\n<cfset var y.z = 1>\n<cfset y = 2>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_local_write_declares() {
        let src = script("local.x = 1;\nx = 2;\nlocal[\"y\"] = 1;\ny = 2;\nlocal[n] = 1;\nn = 2;");
        assert_eq!(found(&src), at(&[(8, 1, "n")]));
        let src =
            tags("<cfset local.x = 1>\n<cfset x = 2>\n<cfset local[\"y\"] = 1>\n<cfset y = 2>");
        assert_eq!(found(&src), at(&[]));
        // So does a result attribute naming `local.x`.
        let src = tags("<cfquery name=\"local.q\">select 1</cfquery>\n<cfset q = 2>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_parameter_declares() {
        let src = script("a = 1;\nb = 1;");
        assert_eq!(found(&src), at(&[(4, 1, "b")]));
        let src = "component {\nfunction f(required string a, b = 1) {\na = 1;\nb = 2;\n}\n\
                   function g() {\nvar h = (c) => c = 1;\n}\n}\n";
        assert_eq!(found(src), at(&[]));
        let src = tags("<cfargument name=\"a\">\n<cfset a = 1>\n<cfset b = 1>");
        assert_eq!(found(&src), at(&[(5, 8, "b")]));
    }

    #[test]
    fn a_catch_variable_declares() {
        let src = script("try {} catch (any e) { e = 1; }\nerr = 1;");
        assert_eq!(found(&src), at(&[(4, 1, "err")]));
        // `cfcatch` is a scope of its own; a write in the body is checked.
        let src = tags("<cftry><cfcatch type=\"any\"><cfset e = cfcatch></cfcatch></cftry>");
        assert_eq!(found(&src), at(&[(3, 35, "e")]));
    }

    #[test]
    fn a_scoped_result_attribute_is_no_write() {
        let src = script(
            "http url=\"u\" result=\"variables.r\";\ncfhttp(url=\"u\", result=\"request.r\");",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags("<cfhttp url=\"u\" result=\"variables.r\">\n<cfsavecontent variable=\"this.s\"></cfsavecontent>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn an_assignment_writes() {
        let src = script("c = 1;\nd.e = 1;\nf[1] = 1;\nh += 1;\nm = n = 1;");
        assert_eq!(
            found(&src),
            at(&[
                (3, 1, "c"),
                (4, 1, "d"),
                (5, 1, "f"),
                (6, 1, "h"),
                (7, 1, "m"),
                (7, 5, "n")
            ])
        );
        let src = tags("<cfset c = 1>\n<cfset d.e = 1>\n<cfset f[1] = 1>\n<cfset h &= \"x\">");
        assert_eq!(
            found(&src),
            at(&[(3, 8, "c"), (4, 8, "d"), (5, 8, "f"), (6, 8, "h")])
        );
    }

    #[test]
    fn an_increment_writes() {
        let src = script("g++;\n--h;\ns.x++;\nfor (i = 1; i < 2; i++) {}");
        assert_eq!(
            found(&src),
            at(&[(3, 1, "g"), (4, 3, "h"), (5, 1, "s"), (6, 6, "i")])
        );
        let src = tags("<cfset g++>\n<cfset ++h>");
        assert_eq!(found(&src), at(&[(3, 8, "g"), (4, 10, "h")]));
    }

    #[test]
    fn a_for_in_writes() {
        let src = script("for (k in s) {}\nfor (local.m in s) {}\nm = 1;");
        assert_eq!(found(&src), at(&[(3, 6, "k")]));
    }

    #[test]
    fn a_param_writes() {
        let src = script(
            "param name=\"p\" default=1;\nparam string q = 1;\nparam r = 1;\nparam t;\n\
             param string arguments.z = 1;\nparam name=\"local.u\" default=1;",
        );
        assert_eq!(
            found(&src),
            at(&[(3, 13, "p"), (4, 14, "q"), (5, 7, "r"), (6, 7, "t")])
        );
        let src =
            tags("<cfparam name=\"pp\" default=\"1\">\n<cfparam name=\"url.x\" default=\"1\">");
        assert_eq!(found(&src), at(&[(3, 16, "pp")]));
    }

    #[test]
    fn a_result_attribute_writes() {
        let src = script(
            "query name=\"qry\" datasource=\"x\" { echo(\"select 1\"); }\nhttp url=\"x\" result=\"res\";\n\
             cfhttp(url=\"x\", result=\"res2\");\nsavecontent variable=\"sc\" {}\n\
             cfloop(index=\"i\", from=1, to=2) {}",
        );
        assert_eq!(
            found(&src),
            at(&[
                (3, 13, "qry"),
                (4, 22, "res"),
                (5, 25, "res2"),
                (6, 23, "sc"),
                (7, 15, "i")
            ])
        );
        let src = tags(
            "<cfquery name=\"q\" datasource=\"d\">select 1</cfquery>\n\
             <cfloop index=\"i\" from=\"1\" to=\"3\"></cfloop>\n<cfhttp url=\"u\" result=\"r\" />\n\
             <cffile action=\"read\" file=\"f\" variable=\"v.x\">\n<cfloop query=\"q\"></cfloop>\n\
             <cfloop from=\"1\" to=\"2\" index=ii></cfloop>",
        );
        assert_eq!(
            found(&src),
            at(&[
                (3, 16, "q"),
                (4, 16, "i"),
                (5, 25, "r"),
                (6, 42, "v"),
                (8, 31, "ii")
            ])
        );
    }

    #[test]
    fn a_dynamic_result_attribute_is_skipped() {
        let src = tags("<cfloop item=\"#it#\" array=\"#arr#\"></cfloop>\n<cfquery name=\"q#n#\">select 1</cfquery>");
        assert_eq!(found(&src), at(&[]));
        let src = script("cfhttp(url=\"x\", result=res);\nparam name=\"#p#\" default=1;");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_scope_this_or_super_is_no_write() {
        let src = script(
            "variables.v = 1;\narguments.a = 1;\nrequest.r = 1;\nthis.t = 1;\nsuper.s = 1;\n\
             static.x = 1;\nsession[\"k\"] = 1;",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags("<cfset variables.v = 1>\n<cfset this.t = 1>\n<cfset url.u = 1>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_target_not_headed_by_a_name_is_no_write() {
        let src = script("foo().x = 1;\n(x) = 1;\n\"x\" = 1;\n#x# = 1;\nsetVariable(\"dyn\", 1);");
        assert_eq!(found(&src), at(&[]));
        let src = tags("<cfset \"x\" = 1>\n<cfset foo().x = 1>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_recovered_region_is_skipped() {
        let lint = crate::lint_source(&script("x = @;\ny = 1;"), Mode::Auto);
        assert_eq!(lint.notes.len(), 1);
        assert!(lint.reports.iter().all(|r| r.name != "x"));
        // In a template the region is the innermost tag body that holds the
        // stray tag: here the whole function, so nothing in it is checked.
        let lint = crate::lint_source(
            "<cffunction name=\"f\">\n<cfif x>\n<cfset y = 1>\n</cffunction>\n",
            Mode::Auto,
        );
        assert_eq!(
            (lint.notes.len(), lint.reports.len(), lint.functions),
            (1, 0, 0)
        );
    }

    #[test]
    fn a_comment_on_the_line_suppresses() {
        let src = script("c = 1; // cfvet-ignore\ne = 1;\n\nd = 1; /* cfvet-ignore */");
        assert_eq!(found(&src), at(&[]));
        let src = script("c = 1;\nd = 1; /* cfvet-ignore */");
        assert_eq!(found(&src), at(&[(3, 1, "c")]));
        let src = tags("<cfset c = 1> <!--- cfvet-ignore --->\n<cfset d = 1>\n<cfset e = 1>");
        assert_eq!(found(&src), at(&[(5, 8, "e")]));
    }

    #[test]
    fn a_comment_on_the_line_before_suppresses() {
        let src = script("// cfvet-ignore\nc = 1;\n/** cfvet-ignore */\nd = 1;\ne = 1;");
        assert_eq!(found(&src), at(&[(7, 1, "e")]));
        let src = tags("<!--- cfvet-ignore --->\n<cfset c = 1>\n<cfset d = 1>");
        assert_eq!(found(&src), at(&[(5, 8, "d")]));
    }

    #[test]
    fn a_write_before_its_var_reports() {
        let src = script("x = 1;\nvar x = 2;\nx = 3;");
        assert_eq!(found(&src), at(&[(3, 1, "x")]));
        assert_eq!(var_not_run(&src), at(&[(3, 1, "x")]));
        let lint = crate::lint_source(&src, Mode::Auto);
        assert_eq!(
            lint.reports[0].message,
            "`x` is written where its `var` may not have run in function `f`; \
             on Lucee the write lands in the variables scope"
        );
        let src = tags("<cfset x = 1>\n<cfset var x = 2>\n<cfset x = 3>");
        assert_eq!(var_not_run(&src), at(&[(3, 8, "x")]));
        assert_eq!(found(&src), at(&[(3, 8, "x")]));
        // A name declared nowhere keeps the other message.
        assert_eq!(var_not_run(&script("y = 1;")), at(&[]));
    }

    #[test]
    fn a_local_write_declares_from_where_it_stands() {
        let src = script("local.x = 1;\nx = 2;\ny = 1;\nlocal.y = 2;\nlocal[\"z\"] = 1;\nz = 2;");
        assert_eq!(var_not_run(&src), at(&[(5, 1, "y")]));
        assert_eq!(found(&src), at(&[(5, 1, "y")]));
        let src = tags(
            "<cfset local.x = 1>\n<cfset x = 2>\n<cfset y = 1>\n<cfset local.y = 2>\n\
             <cfquery name=\"q\">select 1</cfquery>\n<cfquery name=\"local.q\">select 1</cfquery>",
        );
        assert_eq!(found(&src), at(&[(5, 8, "y"), (7, 16, "q")]));
        assert_eq!(var_not_run(&src), at(&[(5, 8, "y"), (7, 16, "q")]));
    }

    #[test]
    fn a_for_var_declares_for_the_body() {
        let src = script("for (var i = 1; i < 3; i++) {\ni++;\n}\nfor (var k in s) {\nk = 1;\n}");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_catch_variable_declares_in_its_catch() {
        let src = script("try {} catch (any e) {\ne = 1;\n}\ne = 2;");
        assert_eq!(var_not_run(&src), at(&[(6, 1, "e")]));
        let src = script("e = 0;\ntry {} catch (any e) {}");
        assert_eq!(var_not_run(&src), at(&[(3, 1, "e")]));
    }

    #[test]
    fn a_var_in_a_branch_covers_the_branch() {
        let src = script("if (a) {\nvar y = 1;\ny = 2;\n}\ny = 3;");
        assert_eq!(var_not_run(&src), at(&[(7, 1, "y")]));
        assert_eq!(found(&src), at(&[(7, 1, "y")]));
        let src = tags("<cfif a>\n<cfset var y = 1>\n<cfset y = 2>\n</cfif>\n<cfset y = 3>");
        assert_eq!(var_not_run(&src), at(&[(7, 8, "y")]));
        assert_eq!(found(&src), at(&[(7, 8, "y")]));
    }

    #[test]
    fn a_braceless_body_is_its_statement() {
        let src = script("if (a) var y = 1;\nelse y = 2;\ny = 3;");
        assert_eq!(var_not_run(&src), at(&[(4, 6, "y"), (5, 1, "y")]));
        let src = script("while (a) var w = 1;\nw = 2;\nfor (;;) var v = 1;\nv = 2;");
        assert_eq!(var_not_run(&src), at(&[(4, 1, "w"), (6, 1, "v")]));
    }

    #[test]
    fn each_branch_declares_for_itself() {
        let src = script("if (a) {\ny = 1;\n} else {\nvar y = 2;\n}");
        assert_eq!(var_not_run(&src), at(&[(4, 1, "y")]));
        let src = script(
            "if (a) {\nvar x = 1;\nx = 2;\n} else if (b) {\nvar x = 3;\nx = 4;\n} else {\n\
             var x = 5;\nx = 6;\n}",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags(
            "<cfif a>\n<cfset var y = 1>\n<cfelseif b>\n<cfset y = 2>\n<cfelse>\n\
             <cfset var y = 3>\n<cfset y = 4>\n</cfif>",
        );
        assert_eq!(var_not_run(&src), at(&[(6, 8, "y")]));
        let src = tags(
            "<cfif a>\n<cfset var y = 1>\n<cfset y = 2>\n<cfelse>\n\
             <cfset var y = 3>\n<cfset y = 4>\n</cfif>",
        );
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn each_case_is_a_block() {
        let src = script(
            "switch (a) {\ncase 1:\nvar w = 1;\nw = 2;\nbreak;\ndefault:\nw = 3;\n}\nw = 4;",
        );
        assert_eq!(var_not_run(&src), at(&[(9, 1, "w"), (11, 1, "w")]));
        let src = tags(
            "<cfswitch expression=\"#a#\">\n<cfcase value=\"1\">\n<cfset var w = 1>\n\
             <cfset w = 2>\n</cfcase>\n<cfdefaultcase>\n<cfset w = 3>\n</cfdefaultcase>\n\
             </cfswitch>",
        );
        assert_eq!(var_not_run(&src), at(&[(9, 8, "w")]));
    }

    #[test]
    fn a_try_body_always_runs() {
        // Checked on Lucee 7 and Adobe: `x` is local.
        let src = "function a() {\n    try {\n        var x = 1;\n    } catch(any e) {\n        \
                   // pass\n    }\n    x = 2;\n    return x;\n}\n";
        assert_eq!(found(src), at(&[]));
        let src =
            script("try {\nvar t = 1;\nt = 2;\n} catch (any e) {\nt = 3;\n} finally {\nt = 4;\n}");
        assert_eq!(found(&src), at(&[]));
        let src = tags(
            "<cftry>\n<cfset var t = 1>\n<cfset t = 2>\n<cfcatch>\n<cfset t = 3>\n</cfcatch>\n\
             <cffinally>\n<cfset t = 4>\n</cffinally>\n</cftry>\n<cfset t = 5>",
        );
        assert_eq!(found(&src), at(&[]));
        // A `try` inside a branch declares for that branch only.
        let src = script("if (a) {\ntry {\nvar t = 1;\n} catch (any e) {}\n}\nt = 2;");
        assert_eq!(var_not_run(&src), at(&[(8, 1, "t")]));
    }

    #[test]
    fn a_catch_is_a_block() {
        let src = script("try {} catch (any e) {\nvar c = 1;\nc = 2;\n}\nc = 3;");
        assert_eq!(var_not_run(&src), at(&[(7, 1, "c")]));
        let src = script(
            "try {} catch (foo e) {\nvar c = 1;\n} catch (any e) {\nc = 2;\n} finally {\nc = 3;\n}",
        );
        assert_eq!(var_not_run(&src), at(&[(6, 1, "c"), (8, 1, "c")]));
        let src = tags(
            "<cftry>\n<cfcatch>\n<cfset var c = 1>\n<cfset c = 2>\n</cfcatch>\n\
             <cffinally>\n<cfset c = 3>\n</cffinally>\n</cftry>\n<cfset c = 4>",
        );
        assert_eq!(var_not_run(&src), at(&[(9, 8, "c"), (12, 8, "c")]));
        // A `finally` body runs whenever the `try` does, so its `var` covers
        // what follows.
        let src = script("try {} finally {\nvar f = 1;\n}\nf = 2;");
        assert_eq!(found(&src), at(&[]));
        let src =
            tags("<cftry>\n<cffinally>\n<cfset var f = 1>\n</cffinally>\n</cftry>\n<cfset f = 2>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_name_every_branch_declares_is_declared_after() {
        let src = script("if (a) {\nvar x = 1;\n} else {\nvar x = 2;\n}\nx = 3;");
        assert_eq!(found(&src), at(&[]));
        let src = script(
            "if (a) {\nvar x = 1;\n} else if (b) {\nvar x = 2;\n} else {\nvar x = 3;\n}\nx = 4;",
        );
        assert_eq!(found(&src), at(&[]));
        // One branch short, or no `else`: a branch that ran may not declare.
        let src = script(
            "if (a) {\nvar x = 1;\n} else if (b) {\nx = 2;\n} else {\nvar x = 3;\n}\nx = 4;",
        );
        assert_eq!(var_not_run(&src), at(&[(6, 1, "x"), (10, 1, "x")]));
        let src = script("if (a) {\nvar x = 1;\n} else if (b) {\nvar x = 2;\n}\nx = 3;");
        assert_eq!(var_not_run(&src), at(&[(8, 1, "x")]));
        // Only a declaration that covers the rest of its branch counts.
        let src = script("if (a) {\nvar x = 1;\n} else {\nif (b) {\nvar x = 2;\n}\n}\nx = 3;");
        assert_eq!(var_not_run(&src), at(&[(10, 1, "x")]));
        // Nothing inside the conditional is covered by the join.
        let src = script("if (a) {\nx = 1;\nvar x = 2;\n} else {\nvar x = 3;\n}");
        assert_eq!(var_not_run(&src), at(&[(4, 1, "x")]));
    }

    #[test]
    fn a_braceless_branch_joins() {
        let src = script("if (a) var x = 1;\nelse var x = 2;\nx = 3;");
        assert_eq!(found(&src), at(&[]));
        let src = script("if (a) var x = 1;\nelse if (b) var x = 2;\nx = 3;");
        assert_eq!(var_not_run(&src), at(&[(5, 1, "x")]));
    }

    #[test]
    fn a_join_inside_a_branch_counts_for_it() {
        let src = script(
            "if (a) {\nvar x = 1;\n} else {\nif (b) {\nvar x = 2;\n} else {\nvar x = 3;\n}\n}\nx = 4;",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags(
            "<cfif a>\n<cfset var x = 1>\n<cfelse>\n<cfif b>\n<cfset var x = 2>\n<cfelse>\n\
             <cfset var x = 3>\n</cfif>\n</cfif>\n<cfset x = 4>",
        );
        assert_eq!(found(&src), at(&[]));
        // A `var` in a `try` in a branch declares for the branch.
        let src = script(
            "if (a) {\ntry {\nvar x = 1;\n} catch (any e) {}\n} else {\nvar x = 2;\n}\nx = 3;",
        );
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_switch_with_a_default_joins() {
        let src = script(
            "switch (a) {\ncase 1:\nvar w = 1;\nbreak;\ncase \"x\": case \"y\":\nvar w = 2;\nbreak;\n\
             default:\nvar w = 3;\n}\nw = 4;",
        );
        assert_eq!(found(&src), at(&[]));
        let src =
            script("switch (a) {\ncase 1:\nvar w = 1;\nbreak;\ncase 2:\nvar w = 2;\n}\nw = 3;");
        assert_eq!(var_not_run(&src), at(&[(10, 1, "w")]));
        let src = script(
            "switch (a) {\ncase 1:\nvar w = 1;\nbreak;\ncase 2:\nbreak;\ndefault:\nvar w = 2;\n}\nw = 3;",
        );
        assert_eq!(var_not_run(&src), at(&[(12, 1, "w")]));
        let src = tags(
            "<cfswitch expression=\"#a#\">\n<cfcase value=\"1\">\n<cfset var w = 1>\n</cfcase>\n\
             <cfdefaultcase>\n<cfset var w = 2>\n</cfdefaultcase>\n</cfswitch>\n<cfset w = 3>",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags(
            "<cfswitch expression=\"#a#\">\n<cfcase value=\"1\">\n<cfset var w = 1>\n</cfcase>\n\
             <cfcase value=\"2\">\n<cfset var w = 2>\n</cfcase>\n</cfswitch>\n<cfset w = 3>",
        );
        assert_eq!(var_not_run(&src), at(&[(11, 8, "w")]));
    }

    #[test]
    fn a_cfif_with_a_cfelse_joins() {
        let src = tags(
            "<cfif a>\n<cfset var y = 1>\n<cfelseif b>\n<cfset var y = 2>\n<cfelse>\n\
             <cfset var y = 3>\n</cfif>\n<cfset y = 4>",
        );
        assert_eq!(found(&src), at(&[]));
        let src = tags(
            "<cfif a>\n<cfset var y = 1>\n<cfelseif b>\n<cfset var y = 2>\n</cfif>\n<cfset y = 3>",
        );
        assert_eq!(var_not_run(&src), at(&[(8, 8, "y")]));
        let src = tags(
            "<cfif a>\n<cfset var y = 1>\n<cfelseif b>\n<cfelse>\n<cfset var y = 3>\n</cfif>\n\
             <cfset y = 4>",
        );
        assert_eq!(var_not_run(&src), at(&[(9, 8, "y")]));
    }

    #[test]
    fn a_for_var_declares_after_the_loop() {
        let src = script("for (var i = 1; i < 3; i++) {}\ni = 0;\nfor (var k in s) {}\nk = 1;");
        assert_eq!(found(&src), at(&[]));
        // Not when the loop itself stands in a branch.
        let src = script("if (a) for (var i = 1; i < 3; i++) {}\ni = 0;");
        assert_eq!(var_not_run(&src), at(&[(4, 1, "i")]));
    }

    #[test]
    fn a_var_in_a_loop_body_covers_the_body() {
        let src =
            script("for (x in xs) {\nvar y = x;\n}\ny = 1;\nwhile (a) {\nvar z = 1;\n}\nz = 1;");
        assert_eq!(var_not_run(&src), at(&[(6, 1, "y"), (10, 1, "z")]));
        assert_eq!(found(&src).len(), 3);
        let src = tags("<cfloop array=\"#xs#\" index=\"local.x\">\n<cfset var y = x>\n</cfloop>\n<cfset y = 1>\n<cfset x = 1>");
        assert_eq!(found(&src), at(&[(6, 8, "y")]));
        let src = tags("<cfoutput query=\"q\">\n<cfset var r = 1>\n</cfoutput>\n<cfset r = 1>");
        assert_eq!(var_not_run(&src), at(&[(6, 8, "r")]));
        let src =
            script("cfloop(from=1, to=2, index=\"local.i\") {\nvar m = 1;\n}\nm = 2;\ni = 3;");
        assert_eq!(found(&src), at(&[(6, 1, "m")]));
        // A `do … while` body runs at least once.
        let src = script("do {\nvar d = 1;\n} while (a);\nd = 2;");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_top_level_var_covers_nested_blocks() {
        let src = script("var y = 1;\nif (a) {\nwhile (b) {\ny = 2;\n}\n}");
        assert_eq!(found(&src), at(&[]));
        let src = tags("<cfset var y = 1>\n<cfif a>\n<cfloop condition=\"b\">\n<cfset y = 2>\n</cfloop>\n</cfif>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_block_that_always_runs_is_no_branch() {
        let src = tags(
            "<cfscript>\nvar a = 1;\n</cfscript>\n<cfset a = 2>\n<cflock name=\"l\">\n\
                        <cfset var b = 1>\n</cflock>\n<cfset b = 2>",
        );
        assert_eq!(found(&src), at(&[]));
        let src = script("lock name=\"l\" {\nvar b = 1;\n}\nb = 2;");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_local_write_in_a_branch_covers_the_branch() {
        let src = script("if (a) {\nlocal.x = 1;\nx = 2;\n}\nx = 3;");
        assert_eq!(var_not_run(&src), at(&[(7, 1, "x")]));
        let src = tags("<cfif a>\n<cfset local.x = 1>\n</cfif>\n<cfset x = 3>");
        assert_eq!(var_not_run(&src), at(&[(6, 8, "x")]));
    }

    #[test]
    fn a_closure_inherits_a_branch_declared_name() {
        let src =
            script("if (a) {\nvar total = 0;\n}\narrayEach(xs, function(x) {\ntotal += x;\n});");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_parameter_declares_from_the_start() {
        let src = tags("<cfset a = 1>\n<cfargument name=\"a\">");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_closure_inherits_a_var_below_it() {
        let src = script("var add = function(x) { total += x; };\nvar total = 0;");
        assert_eq!(found(&src), at(&[]));
        let src = script("var add = (x) => total += x;\ntotal = 1;\nvar total = 0;");
        assert_eq!(var_not_run(&src), at(&[(4, 1, "total")]));
        assert_eq!(found(&src), at(&[(4, 1, "total")]));
    }

    #[test]
    fn names_compare_without_case() {
        let src = script("var Foo = 1;\nfoo = 2;\nFOO++;");
        assert_eq!(found(&src), at(&[]));
        let src = tags("<cfset var Foo = 1>\n<cfset foo = 2>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_closure_inherits_the_enclosing_declarations() {
        let src = script(
            "var total = 0;\narrayEach(xs, function(x) { total += x; y = x; var z = x; });\nz = 1;\n\
             var fn = (x) => { w = x; };",
        );
        assert_eq!(found(&src), at(&[(4, 41, "y"), (5, 1, "z"), (6, 19, "w")]));
    }

    #[test]
    fn a_closure_names_its_function() {
        let lint = crate::lint_source(
            &script("arrayEach(xs, function(x) { y = x; });"),
            Mode::Auto,
        );
        assert!(lint.reports[0].message.contains("in function `(closure)`"));
        let lint = crate::lint_source(&tags("<cfset y = 1>"), Mode::Auto);
        assert!(lint.reports[0].message.contains("in function `f`"));
        assert_eq!(lint.functions, 1);
    }

    #[test]
    fn code_outside_a_function_is_not_checked() {
        assert_eq!(found("component {\nx = 1;\nfunction f() {}\n}\n"), at(&[]));
        assert_eq!(
            found("<cfset x = 1>\n<cfquery name=\"q\">select 1</cfquery>\n"),
            at(&[])
        );
        assert_eq!(
            found("<cfscript>\nx = 1;\nfunction t() { y = 1; }\n</cfscript>\n"),
            at(&[(3, 16, "y")])
        );
    }

    #[test]
    fn a_thread_body_is_skipped() {
        let src = script("thread name=\"t\" { c = 1; }\ncfthread(name=\"u\") { d = 1; }\ne = 1;");
        assert_eq!(found(&src), at(&[(5, 1, "e")]));
        let src = tags("<cfthread name=\"t\"><cfset c = 1></cfthread>\n<cfset e = 1>");
        assert_eq!(found(&src), at(&[(4, 8, "e")]));
    }

    #[test]
    fn a_tag_function_with_a_script_body() {
        let src = "<cffunction name=\"f\">\n<cfscript>\nvar a = 1;\nb = 2;\n</cfscript>\n\
                   <cfset a = 3>\n</cffunction>\n";
        assert_eq!(found(src), at(&[(4, 1, "b")]));
    }

    #[test]
    fn a_pattern_writes_every_name() {
        let src =
            script("[p, q = 1, ...r] = x;\n({d, e: f = 2, ...g} = x);\n[, [h], {i: {j}}] = x;");
        assert_eq!(
            found(&src),
            at(&[
                (3, 2, "p"),
                (3, 5, "q"),
                (3, 15, "r"),
                (4, 3, "d"),
                (4, 9, "f"),
                (4, 19, "g"),
                (5, 5, "h"),
                (5, 14, "j"),
            ])
        );
    }

    #[test]
    fn a_var_pattern_declares() {
        let src = script(
            "var [p, q = 1, [r]] = x;\nvar {s, t: u = 2, ...v} = x;\nfor (var [k, l] in x) {}\n\
             p = 1; q = 1; r = 1; s = 1; u = 1; v = 1; k = 1; l = 1;\nt = 1;",
        );
        assert_eq!(found(&src), at(&[(7, 1, "t")]));
        let src = tags("<cfset var [p, q] = x>\n<cfset p = 1>\n<cfset q = 1>");
        assert_eq!(found(&src), at(&[]));
    }

    #[test]
    fn a_for_pattern_writes() {
        let src = script("for ([k, v] in x) {}\nfor ({m, n: o} in x) {}");
        assert_eq!(
            found(&src),
            at(&[(3, 7, "k"), (3, 10, "v"), (4, 7, "m"), (4, 13, "o")])
        );
    }

    #[test]
    fn a_pattern_default_is_no_write() {
        // The value of a default is read; a write inside it is a write.
        let src = script("var [p = q] = x;\nvar [r = (s = 1)] = x;");
        assert_eq!(found(&src), at(&[(4, 11, "s")]));
    }

    #[test]
    fn a_write_before_its_var_pattern_reports() {
        let src = script("p = 1;\nvar [p] = x;");
        assert_eq!(var_not_run(&src), at(&[(3, 1, "p")]));
    }

    #[test]
    fn a_pattern_parameter_is_reported() {
        // The engine binds a pattern parameter's names in `variables`, not
        // in `arguments`: a write to one afterwards is unscoped too.
        let src = "component {\nfunction f({a, b = 1, p: {c}}, d, {e} = {}) {\na = 1;\nd = 1;\n}\n\
                   function g() {\nvar h = ({i}) => i;\nvar j = function({k = 2}) {};\n}\n}\n";
        assert_eq!(
            found(src),
            at(&[
                (2, 13, "a"),
                (2, 16, "b"),
                (2, 27, "c"),
                (2, 36, "e"),
                (3, 1, "a"),
                (7, 11, "i"),
                (8, 19, "k"),
            ])
        );
    }
}
