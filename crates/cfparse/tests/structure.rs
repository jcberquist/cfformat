//! Expression-structure tests: key-value nodes, fused
//! declarations, the Pratt expression pass and statement kinds. Each case
//! renders the relevant subtree with [`sx`] and compares it to the target
//! shape.

mod common;

use cfparse::{parse_source, Element, ElementKind, Mode, Node, Tree};

fn parse(src: &str, mode: Mode) -> Tree {
    let tree = parse_source(src, mode);
    common::assert_covers_source(src, &tree);
    tree
}

/// Compact rendering of a node: tokens are their text (whitespace and
/// newlines dropped, comments kept as text); elements are
/// `kind:qualifier(…)` with delimiters as text, items flattened in span
/// order. Qualifiers: `binary:<prec>`, `unary:postfix`, `function:arrow`,
/// `segment:<kind>[:safe][:static]`, `block:<kind>`.
fn sx(tree: &Tree, node: &Node) -> String {
    match node {
        Node::Token(t) => tree.text(t).to_string(),
        Node::Element(e) => sx_el(tree, e),
    }
}

fn sx_el(tree: &Tree, el: &Element) -> String {
    if el.kind.is_comment() {
        return tree.slice(el.span.clone()).to_string();
    }
    let mut name = el.kind.name().to_string();
    match &el.kind {
        ElementKind::Binary { prec } => name = format!("{name}:{}", prec.name()),
        ElementKind::Unary { postfix: true } => name.push_str(":postfix"),
        ElementKind::Function { arrow: true } => name.push_str(":arrow"),
        ElementKind::Block(k) => name = format!("{name}:{}", k.name()),
        ElementKind::Recovered(r) => name = format!("{name}:{}", r.name()),
        ElementKind::Segment(k) => {
            name = format!("{name}:{}", k.name());
            let seg = el.as_segment().unwrap();
            if seg.is_safe() {
                name.push_str(":safe");
            }
            if seg.is_static() {
                name.push_str(":static");
            }
        }
        _ => {}
    }
    let mut nodes: Vec<(u32, String)> = Vec::new();
    for n in el
        .children
        .iter()
        .chain(el.items.iter().flat_map(|i| i.nodes()))
    {
        if matches!(n, Node::Token(t) if matches!(t.kind, cfparse::TokenKind::Whitespace | cfparse::TokenKind::Newline))
        {
            continue;
        }
        nodes.push((n.span().start, sx(tree, n)));
    }
    for t in el.items.iter().filter_map(|i| i.separator.as_ref()) {
        nodes.push((t.span.start, tree.text(t).to_string()));
    }
    nodes.sort_by_key(|p| p.0);
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &el.open {
        parts.push(tree.text(t).to_string());
    }
    parts.extend(nodes.into_iter().map(|p| p.1));
    if let Some(t) = &el.close {
        parts.push(tree.text(t).to_string());
    }
    format!("{name}({})", parts.join(" "))
}

/// Renderings of the root's significant children.
fn roots(tree: &Tree) -> Vec<String> {
    tree.root
        .children
        .iter()
        .filter(|n| !n.is_trivia())
        .map(|n| sx(tree, n))
        .collect()
}

fn script(src: &str) -> Vec<String> {
    roots(&parse(src, Mode::Script))
}

fn tags(src: &str) -> Vec<String> {
    roots(&parse(src, Mode::Tags))
}

// ---------------------------------------------------------------------------
// KeyValue
// ---------------------------------------------------------------------------

#[test]
fn struct_members_are_key_values() {
    assert_eq!(
        script("y = { a: 1, \"b\" = f(x), c = [1, 2], #k#: 3 };"),
        ["statement(assignment(y = struct({ key-value(a : 1) , key-value(string(\" b \") = call-expr(f call(( x )))) , \
          key-value(c = array([ 1 , 2 ])) , key-value(template-expression(# k #) : 3) })) ;)"]
    );
    let tree = parse("y = { c = 1 };", Mode::Script);
    let kv = find(&tree, ElementKind::KeyValue)[0]
        .as_key_value()
        .unwrap();
    assert_eq!(sx(&tree, kv.key()), "c");
    assert_eq!(tree.text(kv.separator()), "=");
    assert_eq!(
        kv.separator().kind,
        cfparse::TokenKind::Punct(cfparse::Punct::KeyValue)
    );
    let value: Vec<_> = kv.value().iter().filter(|n| !n.is_trivia()).collect();
    assert_eq!(value.len(), 1);
    assert_eq!(sx(&tree, value[0]), "1");
}

#[test]
fn a_quoted_key_reads_the_same_whatever_its_value() {
    // A function value used to send its quoted key down a separate path that
    // read `#…#` and doubled quotes as plain text.
    for value in ["1", "function(){}", "() => 1"] {
        let [statement] = &script(&format!(
            "x = {{\"#f('a')#\" : {value}, 'b''c' : {value}}};"
        ))[..] else {
            panic!("one statement");
        };
        assert!(
            statement.contains(
                "string(\" template-expression(# call-expr(f call(( string(' a ') ))) #) \")"
            ),
            "{value}: {statement}"
        );
        assert!(
            statement.contains("string(' b '' c ')"),
            "{value}: {statement}"
        );
    }
}

#[test]
fn named_arguments_are_key_values_with_a_rekinded_separator() {
    let tree = parse("foo(a = 1, b: 2);", Mode::Script);
    let kvs = find(&tree, ElementKind::KeyValue);
    assert_eq!(kvs.len(), 2);
    for kv in &kvs {
        let kv = kv.as_key_value().unwrap();
        assert_eq!(
            kv.separator().kind,
            cfparse::TokenKind::Punct(cfparse::Punct::KeyValue)
        );
    }
    assert_eq!(tree.text(kvs[0].as_key_value().unwrap().separator()), "=");
    let call = find(&tree, ElementKind::Call)[0];
    let items: Vec<String> = call
        .items
        .iter()
        .map(|i| {
            i.children
                .iter()
                .filter(|n| !n.is_trivia())
                .map(|n| sx(&tree, n))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    assert_eq!(items, ["key-value(a = 1)", "key-value(b : 2)"]);
}

#[test]
fn parameter_defaults_are_key_values_after_their_modifiers() {
    let tree = parse("function f(required string a = 1, b) {}", Mode::Script);
    let params = find(&tree, ElementKind::Parameters)[0];
    let items: Vec<Vec<String>> = params
        .items
        .iter()
        .map(|i| {
            i.children
                .iter()
                .filter(|n| !n.is_trivia())
                .map(|n| sx(&tree, n))
                .collect()
        })
        .collect();
    assert_eq!(
        items,
        [vec!["required", "string", "key-value(a = 1)"], vec!["b"]]
    );
}

#[test]
fn declaration_and_script_tag_attributes_are_key_values() {
    let kinds = |src: &str| -> Vec<String> {
        let tree = parse(src, Mode::Script);
        find(&tree, ElementKind::KeyValue)
            .into_iter()
            .map(|e| sx_el(&tree, e))
            .collect()
    };
    assert_eq!(
        kinds("component accessors=true extends=\"x\" {}"),
        [
            "key-value(accessors = true)",
            "key-value(extends = string(\" x \"))"
        ]
    );
    assert_eq!(
        kinds("property name=\"a\" type=\"string\";"),
        [
            "key-value(name = string(\" a \"))",
            "key-value(type = string(\" string \"))"
        ]
    );
    assert_eq!(
        kinds("param name=\"a\" default=1;"),
        [
            "key-value(name = string(\" a \"))",
            "key-value(default = 1)"
        ]
    );
    assert_eq!(
        kinds("cfhttp(url=\"x\");"),
        ["key-value(url = string(\" x \"))"]
    );
    assert_eq!(
        kinds("http url=\"x\";"),
        ["key-value(url = string(\" x \"))"]
    );
    assert_eq!(
        kinds("function f() output=false {}"),
        ["key-value(output = false)"]
    );
    // A comment before the `=` leaves the name an attribute: the lookahead
    // that tells an attribute from `param x = 1` reads it as whitespace.
    assert_eq!(
        kinds("param name // c\n=\"a\" default=1;"),
        [
            "key-value(name // c = string(\" a \"))",
            "key-value(default = 1)"
        ]
    );
    assert_eq!(
        kinds("lock name // c\n=\"a\" timeout=1 {}"),
        [
            "key-value(name // c = string(\" a \"))",
            "key-value(timeout = 1)"
        ]
    );
    assert_eq!(
        kinds("cfhttp /* c */ (url=\"x\");"),
        ["key-value(url = string(\" x \"))"]
    );
}

#[test]
fn tag_attributes_are_key_values_and_bare_attributes_stay_tokens() {
    assert_eq!(
        // `<cfparam>`, not an unclosed `<cfloop>`: that is a recovery.
        tags(
            "<cfparam name=\"i\" default=\"#n + 1#\"><div class=\"a\" disabled data-x=un><cfabort>"
        ),
        [
            "cf-tag(< cfparam key-value(name = string(\" i \")) \
             key-value(default = string(\" template-expression(# binary:additive(n + 1) #) \")) >)",
            "html-tag(< div key-value(class = string(\" a \")) disabled key-value(data-x = un) >)",
            "cf-tag(< cfabort >)",
        ]
    );
}

fn find(tree: &Tree, kind: ElementKind) -> Vec<&Element> {
    fn walk<'a>(el: &'a Element, kind: &ElementKind, out: &mut Vec<&'a Element>) {
        if el.kind == *kind {
            out.push(el);
        }
        for n in el.nodes() {
            if let Node::Element(e) = n {
                walk(e, kind, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&tree.root, &kind, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

#[test]
fn a_function_declaration_is_fused_with_its_body() {
    let tree = parse("function f() {}", Mode::Script);
    assert_eq!(
        roots(&tree),
        ["statement(function(function-decl(function f parameters(( ))) block:function({ })))"]
    );
    let f = find(&tree, ElementKind::Function { arrow: false })[0]
        .as_decl()
        .unwrap();
    assert_eq!(
        f.header().as_element().unwrap().kind,
        ElementKind::FunctionDecl
    );
    assert_eq!(
        f.body().kind,
        ElementKind::Block(cfparse::BlockKind::Function)
    );
    assert!(f.arrow().is_none());
}

#[test]
fn an_anonymous_function_is_fused() {
    assert_eq!(
        script("x = function() {};"),
        ["statement(assignment(x = function(function-decl(function parameters(( ))) block:function({ }))) ;)"]
    );
}

#[test]
fn an_arrow_function_is_fused_with_its_arrow_and_body() {
    let tree = parse("f = (a) => a + 1;", Mode::Script);
    assert_eq!(
        roots(&tree),
        ["statement(assignment(f = function:arrow(arrow-function(parameters(( a ))) => block:function(binary:additive(a + 1)))) ;)"]
    );
    let f = find(&tree, ElementKind::Function { arrow: true })[0]
        .as_decl()
        .unwrap();
    assert_eq!(tree.text(f.arrow().unwrap()), "=>");
    assert_eq!(
        f.header().as_element().unwrap().kind,
        ElementKind::ArrowFunction
    );
}

#[test]
fn components_interfaces_and_static_blocks_are_fused() {
    assert_eq!(
        script("component { static { a = 1; } }"),
        ["statement(class(class-decl(component) block:class({ statement(static-block(static block:static({ statement(assignment(a = 1) ;) }))) })))"]
    );
    assert_eq!(
        script("interface { function g(); }"),
        ["statement(interface(interface-decl(interface) block:interface({ statement(function-decl(function g parameters(( ))) ;) })))"]
    );
    // A comment between header and body moves inside the declaration.
    assert_eq!(
        script("x = function() /* c */ {};"),
        ["statement(assignment(x = function(function-decl(function parameters(( )) /* c */) block:function({ }))) ;)"]
    );
    assert_eq!(
        script("component /* c */ {}"),
        ["statement(class(class-decl(component /* c */) block:class({ })))"]
    );
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

fn check(cases: &[(&str, &str)]) {
    for (src, want) in cases {
        assert_eq!(script(src), [*want], "{src}");
    }
}

#[test]
fn an_index_after_a_literal_is_a_segment() {
    // Only a bracket holding one quoted type name is a typed array; any
    // other `[…][` is an array and an index, and the pair is one operand.
    check(&[
        (
            "b = [1,2][1];",
            "statement(assignment(b = chain(array([ 1 , 2 ]) segment:index(brackets([ 1 ])))) ;)",
        ),
        (
            "d = [a:1]['a'];",
            "statement(assignment(d = chain(struct([ key-value(a : 1) ]) \
             segment:index(brackets([ string(' a ') ])))) ;)",
        ),
        (
            "e = [x][y].z;",
            "statement(assignment(e = chain(array([ x ]) segment:index(brackets([ y ])) \
             segment:property(. z))) ;)",
        ),
        (
            "a = ['string']['a', 'b'];",
            "statement(assignment(a = typed-array(brackets([ string(' string ') ]) \
             array([ string(' a ') , string(' b ') ]))) ;)",
        ),
        (
            "s = ['string']['a'] + ['numeric'][1];",
            "statement(assignment(s = binary:additive(\
             typed-array(brackets([ string(' string ') ]) array([ string(' a ') ])) + \
             typed-array(brackets([ string(' numeric ') ]) array([ 1 ])))) ;)",
        ),
    ]);
}

#[test]
fn chains_calls_and_precedence_in_one_statement() {
    check(&[(
        "x = a.b(1).c[2].d && foo(bar) + 1 * 2;",
        "statement(assignment(x = binary:and(chain(a segment:method(. b call(( 1 ))) \
         segment:property(. c) segment:index(brackets([ 2 ])) segment:property(. d)) && \
         binary:additive(call-expr(foo call(( bar ))) + binary:multiplicative(1 * 2)))) ;)",
    )]);
    let tree = parse("x = a.b(1).c[2].d && foo(bar) + 1 * 2;", Mode::Script);
    let chain = find(&tree, ElementKind::Chain)[0].as_chain().unwrap();
    assert_eq!(sx(&tree, chain.head()), "a");
    let segs: Vec<_> = chain.segments().map(|s| s.as_segment().unwrap()).collect();
    assert_eq!(segs.len(), 4);
    assert_eq!(tree.text(segs[0].name().unwrap()), "b");
    assert!(segs[0].call().is_some() && segs[0].accessor().is_some());
    assert!(segs[2].brackets().is_some() && segs[2].name().is_none());
    let call = find(&tree, ElementKind::CallExpr)[0]
        .as_call_expr()
        .unwrap();
    assert_eq!(sx(&tree, call.callee()), "foo");
    assert_eq!(call.args().items.len(), 1);
}

#[test]
fn a_call_is_built_in_by_its_name_alone() {
    check(&[
        ("isNull(x);", "statement(call-expr(isNull call(( x ))) ;)"),
        ("isNull (x);", "statement(call-expr(isNull call(( x ))) ;)"),
        (
            "isNull /* c */ (x);",
            "statement(call-expr(isNull /* c */ call(( x ))) ;)",
        ),
        ("foo (x);", "statement(call-expr(foo call(( x ))) ;)"),
    ]);
    // The callee's kind says which: a built-in by its name, however the
    // arguments follow it.
    for (src, ident) in [
        ("isNull(x);", cfparse::Ident::Builtin),
        ("isNull (x);", cfparse::Ident::Builtin),
        ("isNull /* c */ (x);", cfparse::Ident::Builtin),
        ("foo (x);", cfparse::Ident::Call),
    ] {
        let tree = parse(src, Mode::Script);
        let call = find(&tree, ElementKind::CallExpr)[0]
            .as_call_expr()
            .unwrap();
        assert!(
            matches!(
                call.callee(),
                Node::Token(t) if t.kind == cfparse::TokenKind::Ident(ident)
            ),
            "{src}"
        );
    }
}

#[test]
fn ternary_and_elvis() {
    let tree = parse("var z = cond ? a : b ?: c;", Mode::Script);
    assert_eq!(
        roots(&tree),
        ["statement(var assignment(z = ternary(cond ? a : binary:elvis(b ?: c))) ;)"]
    );
    let t = find(&tree, ElementKind::Ternary)[0].as_ternary().unwrap();
    assert_eq!(
        [t.cond(), t.then(), t.otherwise()].map(|n| sx(&tree, n)),
        ["cond", "a", "binary:elvis(b ?: c)"]
    );
    assert_eq!((tree.text(t.question()), tree.text(t.colon())), ("?", ":"));
    check(&[
        (
            "a ? b : c ? d : e;",
            "statement(ternary(a ? b : ternary(c ? d : e)) ;)",
        ),
        // Right-associative, one n-ary node (Lucee: elvis right side is an
        // assignment expression).
        ("a ?: b ?: c;", "statement(binary:elvis(a ?: b ?: c) ;)"),
        (
            "a ?: b ? c : d;",
            "statement(binary:elvis(a ?: ternary(b ? c : d)) ;)",
        ),
        (
            "x = a or b ? c : d;",
            "statement(assignment(x = ternary(binary:or(a or b) ? c : d)) ;)",
        ),
    ]);
}

#[test]
fn assignments_are_right_associative() {
    check(&[
        (
            "a = b = c;",
            "statement(assignment(a = assignment(b = c)) ;)",
        ),
        (
            "a.b += c & d;",
            "statement(assignment(chain(a segment:property(. b)) += binary:concat(c & d)) ;)",
        ),
        // Lucee binds an augmented assignment at its arithmetic level.
        (
            "a && b += 1;",
            "statement(binary:and(a && assignment(b += 1)) ;)",
        ),
    ]);
    let tree = parse("a.b = 1;", Mode::Script);
    let a = find(&tree, ElementKind::Assignment)[0]
        .as_assignment()
        .unwrap();
    assert_eq!(sx(&tree, a.target()), "chain(a segment:property(. b))");
    assert_eq!((tree.text(a.op()), sx(&tree, a.value())), ("=", "1".into()));
}

#[test]
fn binary_precedence_and_nary_runs() {
    check(&[
        ("a + b + c - d;", "statement(binary:additive(a + b + c - d) ;)"),
        ("a * b + c;", "statement(binary:additive(binary:multiplicative(a * b) + c) ;)"),
        ("a + b * c ^ d;", "statement(binary:additive(a + binary:multiplicative(b * binary:exponent(c ^ d))) ;)"),
        ("a mod b * c;", "statement(binary:modulus(a mod binary:multiplicative(b * c)) ;)"),
        ("a \\ b * c;", "statement(binary:multiplicative(a \\ b * c) ;)"),
        ("a & b + c eq d;", "statement(binary:comparison(binary:concat(a & binary:additive(b + c)) eq d) ;)"),
        ("a and b or c xor d eqv e imp f;", "statement(binary:imp(binary:eqv(binary:xor(binary:or(binary:and(a and b) or c) xor d) eqv e) imp f) ;)"),
        ("a || b && c;", "statement(binary:or(a || binary:and(b && c)) ;)"),
        ("a does not contain b is not c;", "statement(binary:comparison(a phrase(does not contain) b phrase(is not) c) ;)"),
        ("x = a GREATER THAN OR EQUAL TO b;", "statement(assignment(x = binary:comparison(a phrase(GREATER THAN OR EQUAL TO) b)) ;)"),
    ]);
    let tree = parse("a + b - c;", Mode::Script);
    let b = find(
        &tree,
        ElementKind::Binary {
            prec: cfparse::Prec::Additive,
        },
    )[0]
    .as_binary()
    .unwrap();
    assert_eq!(
        b.operands().map(|n| sx(&tree, n)).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(
        b.operators()
            .map(|n| tree.slice(n.span()))
            .collect::<Vec<_>>(),
        ["+", "-"]
    );
    assert_eq!(b.prec(), cfparse::Prec::Additive);
}

#[test]
fn unary_operators() {
    check(&[
        (
            "not a eq b;",
            "statement(unary(not binary:comparison(a eq b)) ;)",
        ),
        // Lucee and Adobe both bind unary minus tighter than `^`.
        ("-a ^ 2;", "statement(binary:exponent(unary(- a) ^ 2) ;)"),
        (
            "!b && -c;",
            "statement(binary:and(unary(! b) && unary(- c)) ;)",
        ),
        (
            "a++ + ++b;",
            "statement(binary:additive(unary:postfix(a ++) + unary(++ b)) ;)",
        ),
        (
            "x = -a.b();",
            "statement(assignment(x = unary(- chain(a segment:method(. b call(( )))))) ;)",
        ),
        (
            "foo(...args);",
            "statement(call-expr(foo call(( unary(... args) ))) ;)",
        ),
    ]);
    let tree = parse("a++;", Mode::Script);
    let u = find(&tree, ElementKind::Unary { postfix: true })[0]
        .as_unary()
        .unwrap();
    assert!(u.is_postfix());
    assert_eq!(
        (tree.text(u.op()), sx(&tree, u.operand())),
        ("++", "a".into())
    );
    let tree = parse("!a;", Mode::Script);
    let u = find(&tree, ElementKind::Unary { postfix: false })[0]
        .as_unary()
        .unwrap();
    assert_eq!(
        (tree.text(u.op()), sx(&tree, u.operand())),
        ("!", "a".into())
    );
}

#[test]
fn constructors() {
    check(&[
        (
            "q = new path.my.Service(1).init();",
            "statement(assignment(q = chain(new(new path.my.Service call(( 1 ))) \
             segment:method(. init call(( ))))) ;)",
        ),
        ("new '#p#'(a = b);", "statement(new(new string(' template-expression(# p #) ') call(( key-value(a = b) ))) ;)"),
        ("n = new Foo;", "statement(assignment(n = new(new Foo)) ;)"),
        ("j = new java(\"x\");", "statement(assignment(j = new(new java call(( string(\" x \") )))) ;)"),
        ("c = new component();", "statement(assignment(c = new(new component call(( )))) ;)"),
    ]);
    let tree = parse("n = new Foo(1);", Mode::Script);
    let n = find(&tree, ElementKind::New)[0].as_new().unwrap();
    assert_eq!(
        (tree.text(n.keyword()), sx(&tree, n.class())),
        ("new", "Foo".into())
    );
    assert!(n.args().is_some());
}

#[test]
fn static_and_safe_accessors() {
    check(&[(
        "a::b() + c?.d;",
        "statement(binary:additive(chain(a segment:method:static(:: b call(( )))) + \
         chain(c segment:property:safe(?. d))) ;)",
    )]);
}

#[test]
fn operands_with_postfix_parts() {
    check(&[
        (
            "(a + b) * c;",
            "statement(binary:multiplicative(group(( binary:additive(a + b) )) * c) ;)",
        ),
        (
            "[1, 2].len();",
            "statement(chain(array([ 1 , 2 ]) segment:method(. len call(( )))) ;)",
        ),
        (
            "\"x\".len();",
            "statement(chain(string(\" x \") segment:method(. len call(( )))) ;)",
        ),
        (
            "foo().bar;",
            "statement(chain(call-expr(foo call(( ))) segment:property(. bar)) ;)",
        ),
        (
            "foo()();",
            "statement(call-expr(call-expr(foo call(( ))) call(( ))) ;)",
        ),
        (
            "a.b()();",
            "statement(call-expr(chain(a segment:method(. b call(( )))) call(( ))) ;)",
        ),
    ]);
}

#[test]
fn trivia_inside_a_run_moves_into_the_node() {
    let tree = parse("a && // c\n b;", Mode::Script);
    let bin = find(
        &tree,
        ElementKind::Binary {
            prec: cfparse::Prec::And,
        },
    )[0];
    let kinds: Vec<String> = bin
        .children
        .iter()
        .map(|n| match n {
            Node::Token(t) => t.kind.name(),
            Node::Element(e) => e.kind.name().to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "ident.variable",
            "ws",
            "op.and",
            "ws",
            "line-comment",
            "nl",
            "ws",
            "ident.variable"
        ]
    );
    // A trailing comment stays a sibling of the statement.
    let tree = parse("x = 1 // c\n", Mode::Script);
    let top: Vec<&str> = tree
        .root
        .children
        .iter()
        .map(|n| match n {
            Node::Token(t) => {
                if t.kind == cfparse::TokenKind::Newline {
                    "nl"
                } else {
                    "ws"
                }
            }
            Node::Element(e) => e.kind.name(),
        })
        .collect();
    assert_eq!(top, ["statement", "ws", "line-comment", "nl"]);
    assert_eq!(roots(&tree)[0], "statement(assignment(x = 1))");
    // Leading and trailing whitespace of an item stays outside.
    let tree = parse("x = [ a + 1 , b ];", Mode::Script);
    let arr = find(&tree, ElementKind::Array)[0];
    assert!(arr.items[0].children.first().unwrap().is_trivia());
    assert!(arr.items[0].children.last().unwrap().is_trivia());
    assert_eq!(arr.items[0].children.len(), 3);
}

#[test]
fn expressions_in_keywords_cases_templates_and_tags() {
    check(&[
        ("return a + b;", "statement(return binary:additive(a + b) ;)"),
        (
            "switch (x) { case a + 1: break; }",
            "statement(switch(switch group(( x )) block:plain({ case(case binary:additive(a + 1) : statement(break ;)) })))",
        ),
        (
            "x = \"#a.b()#\";",
            "statement(assignment(x = string(\" template-expression(# chain(a segment:method(. b call(( )))) #) \")) ;)",
        ),
        (
            "for (var i = 1; i <= 10; i++) {}",
            "statement(for(for group(( var assignment(i = 1) ; binary:comparison(i <= 10) ; unary:postfix(i ++) )) block:plain({ })))",
        ),
        ("for (k in s) {}", "statement(for(for group(( binary:comparison(k in s) )) block:plain({ })))"),
        (
            "f = x => x + 1;",
            "statement(assignment(f = function:arrow(arrow-function(parameters(x)) => block:function(binary:additive(x + 1)))) ;)",
        ),
    ]);
    assert_eq!(
        tags("<cfset x = a + b.c()>"),
        ["cf-tag(< cfset assignment(x = binary:additive(a + chain(b segment:method(. c call(( )))))) >)"]
    );
    assert_eq!(
        tags("<cfif a and b></cfif>"),
        ["tag-body(cf-tag(< cfif binary:and(a and b) >) cf-tag(</ cfif >))"]
    );
    assert_eq!(
        tags("<cfoutput>#a.b()#</cfoutput>"),
        ["tag-body(cf-tag(< cfoutput >) template-expression(# chain(a segment:method(. b call(( )))) #) cf-tag(</ cfoutput >))"]
    );
}

#[test]
fn literal_values_get_no_wrapper() {
    check(&[
        (
            "x = [1, 2];",
            "statement(assignment(x = array([ 1 , 2 ])) ;)",
        ),
        (
            "x = { a: 1 };",
            "statement(assignment(x = struct({ key-value(a : 1) })) ;)",
        ),
        ("x = \"s\";", "statement(assignment(x = string(\" s \")) ;)"),
        ("x = 1;", "statement(assignment(x = 1) ;)"),
        ("x;", "statement(x ;)"),
    ]);
}

#[test]
fn items_hold_their_expression() {
    check(&[
        (
            "x = [a + 1, b];",
            "statement(assignment(x = array([ binary:additive(a + 1) , b ])) ;)",
        ),
        (
            "foo(a.b, c);",
            "statement(call-expr(foo call(( chain(a segment:property(. b)) , c ))) ;)",
        ),
    ]);
}

#[test]
fn an_unparseable_run_is_left_as_it_was() {
    // A dangling operator: the run's tokens stay as they are; nothing is
    // wrapped, dropped or reordered.
    for src in ["<cfset x = >", "<cfif a and></cfif>", "<cfset ? b : c>"] {
        let tree = parse(src, Mode::Tags);
        let wrapped = [
            ElementKind::Assignment,
            ElementKind::Ternary,
            ElementKind::Binary {
                prec: cfparse::Prec::And,
            },
        ]
        .into_iter()
        .map(|k| find(&tree, k).len())
        .sum::<usize>();
        assert_eq!(wrapped, 0, "{src}: {:?}", tags(src));
    }
}

// ---------------------------------------------------------------------------
// Statement kinds
// ---------------------------------------------------------------------------

fn statement_kinds(src: &str) -> Vec<&'static str> {
    let tree = parse(src, Mode::Script);
    tree.root
        .children
        .iter()
        .filter_map(Node::as_element)
        .filter_map(|e| match e.kind {
            ElementKind::Statement(k) => Some(k.name()),
            _ => None,
        })
        .collect()
}

#[test]
fn statements_are_classified_by_their_first_child() {
    for (src, kind) in [
        ("x = 1;", "assignment"),
        ("a.b += 1;", "assignment"),
        ("foo();", "expression"),
        ("x;", "expression"),
        ("var x = 1;", "declaration"),
        ("if (a) {}", "keyword"),
        ("try {} catch (e) {}", "keyword"),
        ("foo: while (1) {}", "keyword"),
        ("return;", "flow"),
        ("return a + b;", "flow"),
        ("abort;", "flow"),
        ("throw \"x\";", "flow"),
        ("function f() {}", "function"),
        ("component {}", "class"),
        ("interface {}", "interface"),
        ("static { a = 1; }", "static-block"),
        ("property name=\"a\";", "property"),
        ("param name=\"a\";", "param"),
        ("http url=\"x\";", "script-tag"),
        ("cfhttp(url=\"x\");", "script-tag"),
        ("cfhttp // c\n(url=\"x\");", "script-tag"),
        ("lock name // c\n=\"a\" {}", "script-tag"),
        ("import a.b;", "import"),
        (";", "empty"),
    ] {
        assert_eq!(statement_kinds(src), [kind], "{src}");
    }
    // Nested statements are classified too.
    let tree = parse("function f() { return 1; }", Mode::Script);
    let inner: Vec<_> = find(&tree, ElementKind::Statement(cfparse::StatementKind::Flow));
    assert_eq!(inner.len(), 1);
    // Inspect output.
    let json = cfparse::json::to_json(&tree, Default::default());
    assert_eq!(json["root"]["children"][0]["stmt"], "function");
    let dump = cfparse::debug::format_tree(&tree, Default::default());
    assert!(dump.contains("statement flow terminator=\";\""), "{dump}");
}

/// The parser emits these fused: the modifiers and what follows them on the
/// line are one statement.
#[test]
fn modifier_declarations_are_one_statement() {
    for (src, shape) in [
        ("static foo = 1;", "statement(static assignment(foo = 1) ;)"),
        (
            "final var y = 2;",
            "statement(final var assignment(y = 2) ;)",
        ),
        (
            "static final x = 1",
            "statement(static final assignment(x = 1))",
        ),
    ] {
        assert_eq!(script(src), [shape], "{src}");
        assert_eq!(statement_kinds(src), ["declaration"], "{src}");
    }
    // Only on one line, and only before a declaration or an assignment.
    assert_eq!(statement_kinds("final\nvar y = 2;").len(), 2);
    assert_eq!(
        statement_kinds("static; x = 1;"),
        ["declaration", "empty", "assignment"]
    );
    assert_eq!(
        statement_kinds("component { static { static foo = 9000; final brad = 'wood' } }"),
        ["class"]
    );
    let tree = parse(
        "static { static foo = 9000; final brad = 'wood' }",
        Mode::Script,
    );
    let decls = find(
        &tree,
        ElementKind::Statement(cfparse::StatementKind::Declaration),
    );
    assert_eq!(decls.len(), 2);
    assert!(find(
        &tree,
        ElementKind::Statement(cfparse::StatementKind::Assignment)
    )
    .is_empty());
}

#[test]
fn var_bindings_keep_member_access_and_new_is_case_insensitive() {
    // `var local.x`, `var x[k]`, `New`, numeric members.
    check(&[
        (
            "var local.x = 1;",
            "statement(var assignment(chain(local segment:property(. x)) = 1) ;)",
        ),
        (
            "var x[k] = 1;",
            "statement(var assignment(chain(x segment:index(brackets([ k ]))) = 1) ;)",
        ),
        (
            "var a.b().c = 1;",
            "statement(var assignment(chain(a segment:method(. b call(( ))) \
             segment:property(. c)) = 1) ;)",
        ),
        (
            "var sut = New tests.Sut();",
            "statement(var assignment(sut = new(New tests.Sut call(( )))) ;)",
        ),
        (
            "a.b.1234 = 2;",
            "statement(assignment(chain(a segment:property(. b) segment:property(. 1234)) = 2) ;)",
        ),
    ]);
    for src in [
        "var local.x = 1;",
        "var x[k] = 1;",
        "var sut = New tests.Sut();",
        "var a = 1, b.c = 2;",
    ] {
        assert_eq!(statement_kinds(src), ["declaration"], "{src}");
    }
    assert_eq!(statement_kinds("a.b.1234 = 2;"), ["assignment"]);
}

#[test]
fn a_tag_island_is_not_inside_a_statement() {
    // A ```` ``` ```` fence is a node of the statement list, never a
    // statement's child.
    let tree = parse("{\n```\n<cfset a=1>\n```\nb=2;\n}", Mode::Script);
    let island = find(&tree, ElementKind::TagIsland);
    assert_eq!(island.len(), 1);
    assert_eq!(statement_kinds("{\n```\n<cfset a=1>\n```\n}"), ["keyword"]);
}

// ---------------------------------------------------------------------------
// Recovered regions
// ---------------------------------------------------------------------------

/// The tree's recovery list as `(reason, the region's text)`.
fn recoveries(tree: &Tree) -> Vec<(&'static str, String)> {
    tree.recoveries
        .iter()
        .map(|r| (r.reason.name(), tree.slice(r.span.clone()).to_string()))
        .collect()
}

/// One case per reason in script mode: the region is the statement the
/// parse gave up in (`b = @;` is three statements to the parser, one
/// region), or outside any statement the run itself (the stray `}`).
#[test]
fn script_recoveries_are_statements() {
    let tree = parse("a = 1;\nb = @;\nc = 2;\n", Mode::Script);
    assert_eq!(
        roots(&tree)[1],
        "recovered:unmatched(statement(b =) statement(@) statement(;))"
    );
    assert_eq!(recoveries(&tree), [("unmatched", "b = @;".into())]);

    let tree = parse("a = 1;\n}\nb = 2;\n", Mode::Script);
    assert_eq!(roots(&tree)[1], "recovered:stray-closer(})");
    assert_eq!(recoveries(&tree), [("stray-closer", "}".into())]);

    // What the parser built as it read stays built inside the region (the
    // fused function); the post-passes do not build there.
    let tree = parse("function f() {\n  a = 1;\n", Mode::Script);
    assert_eq!(
        roots(&tree),
        ["recovered:unclosed(statement(function(function-decl(function f parameters(( ))) block:function({ statement(a = 1 ;)))))"]
    );
    assert_eq!(
        recoveries(&tree),
        [("unclosed", "function f() {\n  a = 1;\n".into())]
    );
    // A parenthesis left open: the innermost statement, not the block.
    let tree = parse("if (x) { a = f(1; }\n", Mode::Script);
    assert_eq!(
        roots(&tree),
        ["statement(if(if group(( x )) block:plain({ recovered:unclosed(statement(a = f call(( 1) ;)) })))"]
    );
    assert_eq!(recoveries(&tree), [("unclosed", "a = f(1;".into())]);

    // Past the front end's bound: the region is the whole cut statement,
    // `TooDeep` though every block in it is unclosed.
    let src = format!("x = 1;\n{}\n", "{".repeat(150));
    let tree = parse(&src, Mode::Script);
    assert_eq!(recoveries(&tree), [("too-deep", "{".repeat(150))]);
    // Past the expression pass's bound: the run, left flat.
    let src = format!("x = {}y;\n", "!".repeat(500));
    let tree = parse(&src, Mode::Script);
    assert_eq!(
        recoveries(&tree),
        [("too-deep", format!("x = {}y", "!".repeat(500)))]
    );
}

/// One case per reason in tag mode: the region is the innermost tag body
/// around the problem; outside any, the tag, or an unclosed tag and the
/// rest of its list. A script recovery inside a tag keeps its own region.
#[test]
fn tag_recoveries_are_tag_bodies() {
    let tree = parse("<cfif x>\n<cfparam @ name=\"y\">\n</cfif>\n", Mode::Tags);
    assert_eq!(
        roots(&tree),
        ["recovered:unmatched(tag-body(cf-tag(< cfif x >) cf-tag(< cfparam @ key-value(name = string(\" y \")) >) cf-tag(</ cfif >)))"]
    );
    assert_eq!(recoveries(&tree)[0].0, "unmatched");

    let tree = parse("<div>\n<p>x</p>\n</cfif>\n</div>\n", Mode::Tags);
    assert_eq!(
        recoveries(&tree),
        [("stray-closer", "<div>\n<p>x</p>\n</cfif>\n</div>".into())]
    );
    let tree = parse("<p>a</p>\n</cfif>\n", Mode::Tags);
    assert_eq!(roots(&tree)[1], "recovered:stray-closer(cf-tag(</ cfif >))");

    let tree = parse("<cfset a = 1>\n<cfoutput>\n<p>x</p>\n", Mode::Tags);
    assert_eq!(
        roots(&tree),
        [
            "cf-tag(< cfset assignment(a = 1) >)",
            "recovered:unclosed(cf-tag(< cfoutput >) tag-body(html-tag(< p >) x html-tag(</ p >)))"
        ]
    );
    assert_eq!(
        recoveries(&tree),
        [("unclosed", "<cfoutput>\n<p>x</p>".into())]
    );

    // The tree cannot pair `<cfif>` with a `</cfif>` inside the `<cfquery>`
    // island, the file-wide CF walk can: bare tags, no region.
    let tree = parse(
        "<cfif a>\n<cfquery name=\"q\">\nselect 1 </cfif>\n</cfquery>\n",
        Mode::Tags,
    );
    assert!(recoveries(&tree).is_empty());
    // Neither pairs the `</cfoutput>`: the region is the innermost body.
    let tree = parse("<cfif a>\n</div>\n</cfoutput>\n</cfif>\n", Mode::Tags);
    assert_eq!(
        recoveries(&tree),
        [(
            "stray-closer",
            "<cfif a>\n</div>\n</cfoutput>\n</cfif>".into()
        )]
    );

    // `<cfoutput>` × 150: the scanner cuts at the bound; every unclosed tag
    // above the cut folds into one region, `TooDeep`.
    let src = format!("<p>a</p>\n{}\n", "<cfoutput>".repeat(150));
    let tree = parse(&src, Mode::Tags);
    assert_eq!(recoveries(&tree), [("too-deep", "<cfoutput>".repeat(150))]);

    // Script inside a tag: the run itself.
    let tree = parse("<cfoutput>#a @ b#</cfoutput>\n", Mode::Tags);
    assert_eq!(
        roots(&tree),
        ["tag-body(cf-tag(< cfoutput >) template-expression(# a recovered:unmatched( @ ) b #) cf-tag(</ cfoutput >))"]
    );
    assert_eq!(recoveries(&tree), [("unmatched", " @ ".into())]);
}

/// An unclosed `#` stops at the tag boundary (`scan::Bounded`): the
/// `<cfscript>` body ends at its `</cfscript>` and the `<cfset>` at its
/// `>`, so the region is the broken statement or string, and the tags
/// after it are tags. It used to run to the end of the file: one unclosed
/// region for the rest of the file, or — for the `<cfset>` — the next tag
/// swallowed into the string with no recovery at all.
#[test]
fn an_unclosed_hash_stops_at_the_tag_boundary() {
    let p = "tag-body(html-tag(< p >) ok html-tag(</ p >))";
    let tree = parse(
        "<cfscript> x = \"price #\"; y=1; </cfscript><p>ok</p>\n",
        Mode::Tags,
    );
    assert_eq!(roots(&tree)[1], p);
    assert_eq!(
        recoveries(&tree),
        [("unclosed", "x = \"price #\"; y=1; ".into())]
    );

    let tree = parse("<cfset x = \"price #\"><cfset y=1>\n", Mode::Tags);
    assert_eq!(roots(&tree)[1], "cf-tag(< cfset assignment(y = 1) >)");
    assert_eq!(recoveries(&tree), [("unclosed", "\"price #\"".into())]);

    for (src, region) in [
        (
            "<cfscript> x = #; y=1; </cfscript><p>ok</p>\n",
            "x = #; y=1; ",
        ),
        (
            "<cfscript> x = 1 # 2; y=1; </cfscript><p>ok</p>\n",
            "x = 1 # 2; y=1; ",
        ),
    ] {
        let tree = parse(src, Mode::Tags);
        assert_eq!(roots(&tree)[1], p, "{src:?}");
        assert_eq!(recoveries(&tree), [("unmatched", region.into())], "{src:?}");
    }

    // The closed forms: no region, and the tag after is a tag.
    for src in [
        "<cfscript> x = \"price\"; y=1; </cfscript><p>ok</p>\n",
        "<cfscript> x = \"price ##\"; y = \"hi #name#\"; </cfscript><p>ok</p>\n",
        "<cfscript> z = \"a#f(\">\")#b\"; w = \"a#\"</cfscript>\"#b\"; </cfscript><p>ok</p>\n",
        "<cfscript> // #\n/* # */ x = 1; </cfscript><p>ok</p>\n",
    ] {
        let tree = parse(src, Mode::Tags);
        assert_eq!(roots(&tree)[1], p, "{src:?}");
        assert_eq!(recoveries(&tree), [], "{src:?}");
    }
    for src in [
        "<cfset x = \"#f(\">\")#\"><cfset y=1>\n",
        "<cfset x = \"a#f(\">\")#b\"><cfset y=1>\n",
        "<cfset x = 1 <!--- # ---> ><cfset y=1>\n",
    ] {
        let tree = parse(src, Mode::Tags);
        assert_eq!(
            roots(&tree)[1],
            "cf-tag(< cfset assignment(y = 1) >)",
            "{src:?}"
        );
        assert_eq!(recoveries(&tree), [], "{src:?}");
    }
}

/// A string or `#…#` still open where its fragment ends is `Unclosed`,
/// like a parenthesis: its statement is the region.
#[test]
fn an_unclosed_string_is_a_recovery() {
    let tree = parse("a = 1;\nx = \"abc\n", Mode::Script);
    assert_eq!(recoveries(&tree), [("unclosed", "x = \"abc\n".into())]);
    let tree = parse("a = 1;\nx = \"a #b#\n", Mode::Script);
    assert_eq!(recoveries(&tree), [("unclosed", "x = \"a #b#\n".into())]);
}
