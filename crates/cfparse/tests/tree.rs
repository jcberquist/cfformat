//! Tree-shape tests: delimiters, items, statements, islands, normalisation.

mod common;

use cfparse::{
    parse_source, BlockKind, CfKind, Delim, Element, ElementKind, Ident, IslandSite, Lang, Literal,
    Mode, Newline, Node, Operator, Prec, Punct, StatementKind, TagShape, Token, TokenKind, Tree,
};

fn parse(src: &str, mode: Mode) -> Tree {
    let tree = parse_source(src, mode);
    common::assert_covers_source("inline", &tree);
    tree
}

/// Depth-first search for elements matching `pred`.
fn find_all<'a>(el: &'a Element, pred: &dyn Fn(&Element) -> bool, out: &mut Vec<&'a Element>) {
    if pred(el) {
        out.push(el);
    }
    for n in el.nodes() {
        if let Node::Element(e) = n {
            find_all(e, pred, out);
        }
    }
}

fn find<'a>(tree: &'a Tree, pred: &dyn Fn(&Element) -> bool) -> Vec<&'a Element> {
    let mut out = Vec::new();
    find_all(&tree.root, pred, &mut out);
    out
}

fn is_ws(n: &Node) -> bool {
    matches!(n, Node::Token(t) if matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline))
}

/// Children that are not whitespace/newline tokens.
fn significant(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().filter(|n| !is_ws(n)).collect()
}

fn statements(el: &Element) -> Vec<&Element> {
    el.children
        .iter()
        .filter_map(Node::as_element)
        .filter(|e| e.kind.is_statement())
        .collect()
}

fn text<'a>(tree: &'a Tree, n: &Node) -> &'a str {
    tree.slice(n.span())
}

fn tok(n: &Node) -> &Token {
    n.as_token().expect("token")
}

/// One-line shape of an element: `kind(child child …)`. Whitespace and
/// newline tokens are dropped, other tokens render as their text, elements
/// recurse. A statement terminator shows as a trailing `;`.
fn shape(tree: &Tree, el: &Element) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &el.open {
        parts.push(tree.text(t).to_string());
    }
    let mut nodes: Vec<&Node> = el.children.iter().collect();
    for item in &el.items {
        nodes.extend(item.nodes());
    }
    nodes.sort_by_key(|n| n.span().start);
    for n in nodes {
        if is_ws(n) {
            continue;
        }
        parts.push(match n {
            Node::Token(t) => tree.text(t).to_string(),
            Node::Element(e) => shape(tree, e),
        });
    }
    if let Some(t) = &el.close {
        parts.push(tree.text(t).to_string());
    }
    format!("{}({})", el.kind.name(), parts.join(" "))
}

/// Children of a tag, with `key-value` attributes expanded into their parts.
fn attr_nodes(el: &Element) -> Vec<&Node> {
    el.children
        .iter()
        .flat_map(|n| match n {
            Node::Element(e) if e.kind == ElementKind::KeyValue => e.children.iter().collect(),
            n => vec![n],
        })
        .collect()
}

/// Shapes of the root's significant children.
fn root_shapes(tree: &Tree) -> Vec<String> {
    tree.root
        .children
        .iter()
        .filter(|n| !is_ws(n))
        .map(|n| match n {
            Node::Token(t) => tree.text(t).to_string(),
            Node::Element(e) => shape(tree, e),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Delimiters and items
// ---------------------------------------------------------------------------

#[test]
fn delimiters_are_retained() {
    let tree = parse(
        "foo(1, 2); x = {a: [1]}; if (a) { b[1] = 2; }",
        Mode::Script,
    );
    let delims = |kind: &dyn Fn(&ElementKind) -> bool| -> Vec<(String, String)> {
        find(&tree, &|e| kind(&e.kind))
            .into_iter()
            .map(|e| {
                (
                    tree.text(e.open.as_ref().unwrap()).to_string(),
                    tree.text(e.close.as_ref().unwrap()).to_string(),
                )
            })
            .collect()
    };
    let pair = |a: &str, b: &str| vec![(a.to_string(), b.to_string())];
    assert_eq!(delims(&|k| matches!(k, ElementKind::Call)), pair("(", ")"));
    assert_eq!(
        delims(&|k| matches!(k, ElementKind::Struct { .. })),
        pair("{", "}")
    );
    assert_eq!(delims(&|k| *k == ElementKind::Array), pair("[", "]"));
    assert_eq!(delims(&|k| *k == ElementKind::Group), pair("(", ")"));
    assert_eq!(delims(&|k| *k == ElementKind::Brackets), pair("[", "]"));
    assert_eq!(
        delims(&|k| *k == ElementKind::Block(BlockKind::Plain)),
        pair("{", "}")
    );
    // The call's `(` is the element's delimiter, not a nested group.
    let call = find(&tree, &|e| matches!(e.kind, ElementKind::Call))[0];
    assert_eq!(
        call.open.as_ref().unwrap().kind,
        TokenKind::Punct(Punct::Open(Delim::Paren))
    );
    assert!(find(&tree, &|e| e.kind == ElementKind::Group).len() == 1);
}

#[test]
fn items_and_separators() {
    let tree = parse("foo(1, 2, (3, 4));", Mode::Script);
    let call = find(&tree, &|e| matches!(e.kind, ElementKind::Call))[0];
    assert_eq!(call.items.len(), 3);
    let seps: Vec<Option<&str>> = call
        .items
        .iter()
        .map(|i| i.separator.as_ref().map(|t| tree.text(t)))
        .collect();
    assert_eq!(seps, vec![Some(","), Some(","), None]);
    assert!(call.children.is_empty());
    // Commas inside a nested group are not the call's separators.
    let third = significant(&call.items[2].children);
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].as_element().unwrap().kind, ElementKind::Group);

    // Struct / array / params / empty call.
    let tree = parse(
        "function f(a, b = 1) { return [1, 2, ]; } x = {a: 1, b: 2}; g();",
        Mode::Script,
    );
    let params = find(&tree, &|e| e.kind == ElementKind::Parameters)[0];
    assert_eq!(params.items.len(), 2);
    let array = find(&tree, &|e| e.kind == ElementKind::Array)[0];
    // A trailing comma keeps its separator; the whitespace after it forms a
    // last item with no significant children (printers filter those).
    assert_eq!(array.items.len(), 3);
    assert!(array.items[1].separator.is_some());
    assert!(significant(&array.items[2].children).is_empty());
    let tree2 = parse("x = [1,];", Mode::Script);
    let array = find(&tree2, &|e| e.kind == ElementKind::Array)[0];
    assert_eq!(array.items.len(), 1, "no empty item after a trailing comma");
    let st = find(&tree, &|e| matches!(e.kind, ElementKind::Struct { .. }))[0];
    assert_eq!(st.items.len(), 2);
    let g = find(&tree, &|e| matches!(e.kind, ElementKind::Call))[0];
    assert!(g.items.is_empty() && g.open.is_some() && g.close.is_some());
}

#[test]
fn for_header_items_split_on_semicolons() {
    let tree = parse("for (var i = 1; i <= 10; i++) {}", Mode::Script);
    let header = find(&tree, &|e| e.kind == ElementKind::Group)[0];
    let seps: Vec<Option<&str>> = header
        .items
        .iter()
        .map(|i| i.separator.as_ref().map(|t| tree.text(t)))
        .collect();
    assert_eq!(seps, vec![Some(";"), Some(";"), None]);
    assert_eq!(
        header.items[0].separator.as_ref().unwrap().kind,
        TokenKind::Punct(Punct::ExprSeparator)
    );
    // The header's clauses are not statements.
    assert!(find(&tree, &|e| e.kind.is_statement()).len() == 1);
}

// ---------------------------------------------------------------------------
// Statements (the `exprTests` fixtures)
// ---------------------------------------------------------------------------

fn expr_tree(name: &str) -> Tree {
    let f = common::fixtures()
        .into_iter()
        .find(|f| f.name == format!("exprTests.{name}"))
        .unwrap();
    parse(&f.source, f.mode)
}

/// The first statement's only significant child (its top expression node),
/// rendered with [`shape`], and the number of statements. The statement's
/// top node spans the whole expression.
fn first_statement(tree: &Tree) -> (String, usize) {
    let stmts = statements(&tree.root);
    let top = significant(&stmts[0].children);
    assert_eq!(top.len(), 1, "one top node: {:?}", root_shapes(tree));
    let shape = match top[0] {
        Node::Element(e) => shape(tree, e),
        Node::Token(t) => tree.text(t).to_string(),
    };
    (shape, stmts.len())
}

#[test]
fn statement_continues_after_newline_with_binary_operator() {
    for (name, top) in [
        (
            "binaryoperator",
            "assignment(a = binary(1 line-comment(//  test) + 2))",
        ),
        (
            "group",
            "assignment(a = binary(group(( binary(1 + 2) )) + 3))",
        ),
    ] {
        let tree = expr_tree(name);
        assert_eq!(first_statement(&tree), (top.to_string(), 1), "{name}");
    }
}

#[test]
fn statement_continues_if_previous_line_ended_with_operator() {
    let tree = expr_tree("precedingoperator");
    // The comment between the lines stays inside the expression.
    assert_eq!(
        first_statement(&tree),
        (
            "assignment(a = binary(1 < line-comment(//  test) 2))".to_string(),
            1
        )
    );
}

#[test]
fn statement_continues_with_ternary_operator() {
    for name in ["ternary", "precedingternary"] {
        let tree = expr_tree(name);
        assert_eq!(
            first_statement(&tree),
            ("ternary(binary(a < 1) ? 2 : 3)".to_string(), 2),
            "{name}"
        );
        let stmts = statements(&tree.root);
        let second = significant(&stmts[1].children);
        assert_eq!(second.len(), 1);
        assert_eq!(text(&tree, second[0]), "4", "{name}");
    }
}

#[test]
fn statement_does_not_consume_following_if() {
    let tree = expr_tree("if");
    // A comment after a statement is its sibling, not its last child: the
    // statement ends at `0` and the `// test` comment follows it.
    assert_eq!(first_statement(&tree), ("assignment(a = 0)".to_string(), 2));
    let siblings = significant(&tree.root.children);
    assert!(matches!(
        siblings[1],
        Node::Element(e) if e.kind == ElementKind::LineComment
    ));
    let stmts = statements(&tree.root);
    let second = significant(&stmts[1].children);
    assert_eq!(second[0].as_element().unwrap().kind, ElementKind::If);
}

#[test]
fn statement_collects_function_call() {
    let tree = expr_tree("withfunction");
    assert_eq!(
        first_statement(&tree),
        (
            "assignment(a = binary(1 + call-expr(two call(( )))))".to_string(),
            2
        )
    );
    let stmts = statements(&tree.root);
    assert_eq!(text(&tree, significant(&stmts[1].children)[0]), "3");
}

#[test]
fn statement_collects_chained_method_calls() {
    let tree = expr_tree("accessor");
    assert_eq!(
        first_statement(&tree),
        (
            "chain(a segment(. b call(( ))) segment(. c call(( ))))".to_string(),
            2
        )
    );
    let stmts = statements(&tree.root);
    assert_eq!(text(&tree, significant(&stmts[1].children)[0]), "d");
}

#[test]
fn statement_terminator_and_trailing_space() {
    let tree = parse("a = 1;\n\nb = 2\nc()", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 3);
    assert_eq!(tree.text(stmts[0].terminator().unwrap()), ";");
    assert!(stmts[1].terminator().is_none());
    // Blank lines between statements are siblings, not statement content.
    for s in &stmts {
        assert!(!is_ws(s.children.last().unwrap()));
    }
    let newlines = tree
        .root
        .children
        .iter()
        .filter(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Newline))
        .count();
    assert_eq!(newlines, 3);
}

#[test]
fn tag_in_script_and_static_block_statements_do_not_merge() {
    // `property`, `param` and `static {}` each end their own statement:
    // the statements after them are not merged into it.
    let tree = parse(
        "property name=\"a\";\nproperty name=\"b\";\nfoo();\nparam name=\"p\" default=1;\nstatic { a = 1; }\nb = 2;\nabort;\nc();\n",
        Mode::Script,
    );
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 8);
    for (i, s) in stmts.iter().enumerate() {
        let inline_terminators = s
            .children
            .iter()
            .filter(
                |n| matches!(n, Node::Token(t) if t.kind == TokenKind::Punct(Punct::Terminator)),
            )
            .count();
        assert_eq!(
            inline_terminators, 0,
            "statement {i} has an inline terminator"
        );
        if i == 4 {
            assert!(s.terminator().is_none(), "static block has no terminator");
        } else {
            assert_eq!(tree.text(s.terminator().unwrap()), ";", "statement {i}");
        }
    }
    let kinds: Vec<_> = stmts
        .iter()
        .map(|s| {
            significant(&s.children)[0]
                .as_element()
                .map(|e| e.kind.clone())
        })
        .collect();
    assert_eq!(kinds[0], Some(ElementKind::Property));
    assert_eq!(kinds[1], Some(ElementKind::Property));
    assert_eq!(kinds[3], Some(ElementKind::Param));
}

// ---------------------------------------------------------------------------
// Islands
// ---------------------------------------------------------------------------

fn islands(tree: &Tree) -> Vec<&Element> {
    find(tree, &|e| matches!(e.kind, ElementKind::Island(_)))
}

fn island_of(e: &Element) -> &cfparse::Island {
    match &e.kind {
        ElementKind::Island(i) => i,
        _ => panic!("not an island"),
    }
}

#[test]
fn script_comparison_stays_text() {
    let tree = parse(
        "<script>\nfor (i = 0; i < 4; i++) {}\n</script>\n",
        Mode::Auto,
    );
    let isl = islands(&tree);
    assert_eq!(isl.len(), 1);
    assert_eq!(island_of(isl[0]).lang, Lang::Js);
    assert!(isl[0].is_pure_island());
    assert_eq!(
        text(&tree, &isl[0].children[0]),
        "for (i = 0; i < 4; i++) {}\n"
    );
}

#[test]
fn cf_tags_inside_islands_are_children() {
    let cases = [
        (
            "<script>a(); <cfif x>b();</cfif></script>",
            Lang::Js,
            IslandSite::ScriptTag,
        ),
        (
            "<style>.a {} <cfif x>.b {}</cfif></style>",
            Lang::Css,
            IslandSite::StyleTag,
        ),
        (
            "<cfquery name=\"q\">SELECT 1 <cfif x>AND 1</cfif></cfquery>",
            Lang::Sql,
            IslandSite::CfQuery,
        ),
    ];
    for (src, lang, site) in cases {
        let tree = parse(src, Mode::Auto);
        let isl = islands(&tree);
        assert_eq!(isl.len(), 1, "{src}");
        let island = island_of(isl[0]);
        assert_eq!((island.lang, island.site), (lang, site), "{src}");
        // The `<cfif>` … `</cfif>` pair is one tag body inside the island.
        let bodies: Vec<&Element> = isl[0]
            .children
            .iter()
            .filter_map(Node::as_element)
            .collect();
        assert_eq!(bodies.len(), 1, "{src}");
        assert_eq!(bodies[0].kind, ElementKind::TagBody { cf: true }, "{src}");
        let tags: Vec<TagShape> = [bodies[0].open_tag(), bodies[0].close_tag()]
            .into_iter()
            .map(|e| match e.unwrap().kind {
                ElementKind::CfTag(shape, _) => shape,
                ref k => panic!("{src}: unexpected {k:?}"),
            })
            .collect();
        assert_eq!(tags, vec![TagShape::Open, TagShape::Close], "{src}");
        assert!(!isl[0].is_pure_island(), "{src}");
        // Host text is coalesced: never two adjacent Text tokens.
        for pair in isl[0].children.windows(2) {
            assert!(
                !(matches!(&pair[0], Node::Token(a) if a.kind == TokenKind::Text)
                    && matches!(&pair[1], Node::Token(b) if b.kind == TokenKind::Text)),
                "{src}"
            );
        }
    }
}

#[test]
fn html_tags_stay_structured() {
    let tree = parse("<div class=\"a\"><p>text</p><br/></div>", Mode::Auto);
    let tags = find(&tree, &|e| matches!(e.kind, ElementKind::HtmlTag(_)));
    let shapes: Vec<(&str, TagShape)> = tags
        .iter()
        .map(|e| {
            let ElementKind::HtmlTag(shape) = e.kind else {
                unreachable!()
            };
            (tree.tag_name(e).unwrap(), shape)
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            ("div", TagShape::Open),
            ("p", TagShape::Open),
            ("p", TagShape::Close),
            ("br", TagShape::SelfClosed),
            ("div", TagShape::Close),
        ]
    );
    let div = tags[0];
    assert_eq!(tree.text(div.open.as_ref().unwrap()), "<");
    assert_eq!(tree.text(div.close.as_ref().unwrap()), ">");
    assert!(attr_nodes(div).into_iter().any(|n| matches!(
        n,
        Node::Token(t) if t.kind == TokenKind::Ident(Ident::AttributeName)
    )));
}

#[test]
fn island_purity() {
    let pure = |src: &str| -> bool {
        let tree = parse(src, Mode::Auto);
        let isl = islands(&tree);
        assert_eq!(isl.len(), 1, "{src}");
        isl[0].is_pure_island()
    };
    // CFML inside the island, or a `#` that CFML would evaluate, makes it impure.
    assert!(!pure("<script>if (a) { <cfif x>b();</cfif> }</script>"));
    assert!(!pure("<cfoutput><script>var a = #x#;</script></cfoutput>"));
    assert!(pure("<script>document.querySelector('#id');</script>"));
    assert!(!pure("<cfoutput><script>var a = '##';</script></cfoutput>"));
    assert!(!pure("<script>a(); <!--- note ---> b();</script>"));
    assert!(pure("<script>if (i < 4) {}</script>"));
    assert!(pure("<style>.a { color: red; }</style>"));

    // The impure children are what makes them impure.
    let tree = parse(
        "<cfoutput><script>var a = #x#;</script></cfoutput>",
        Mode::Auto,
    );
    let isl = islands(&tree)[0];
    assert!(isl
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::TemplateExpression)));
    let tree = parse(
        "<cfoutput><script>var a = '##';</script></cfoutput>",
        Mode::Auto,
    );
    assert!(islands(&tree)[0].children.iter().any(|n| matches!(
        n,
        Node::Token(t) if t.kind == TokenKind::Literal(Literal::EscapeHash)
    )));
}

#[test]
fn script_type_is_recorded() {
    let script_type = |src: &str| {
        let tree = parse(src, Mode::Auto);
        let isl = islands(&tree);
        assert_eq!(isl.len(), 1, "{src}");
        let island = island_of(isl[0]).clone();
        (island.lang, island.script_type)
    };
    assert_eq!(
        script_type("<script type=\"module\">import x from 'y';</script>"),
        (Lang::Js, Some("module".into()))
    );
    assert_eq!(
        script_type("<script type='Application/JSON'>{\"a\": 1}</script>"),
        (Lang::Json, Some("application/json".into()))
    );
    assert_eq!(script_type("<script>a();</script>"), (Lang::Js, None));
    // Every JSON type the dispatch table names is a JSON island.
    for ty in [
        "application/json",
        "application/ld+json",
        "importmap",
        "speculationrules",
        "text/json",
    ] {
        assert_eq!(
            script_type(&format!("<script type=\"{ty}\">{{}}</script>")),
            (Lang::Json, Some(ty.into())),
            "{ty}"
        );
    }
    // Unknown types are data: an island of no language, the type recorded
    // (trimmed and lowercased), the body's HTML plain text.
    assert_eq!(
        script_type("<script type=\"text/template\"><b>x</b></script>"),
        (Lang::Unknown, Some("text/template".into()))
    );
    assert_eq!(
        script_type("<style type=\" Text/LESS \">a{}</style>"),
        (Lang::Unknown, Some("text/less".into()))
    );
    let tree = parse(
        "<script type=\"text/template\"><b>x</b></script>",
        Mode::Auto,
    );
    assert!(islands(&tree)[0].is_pure_island());
}

#[test]
fn a_dynamic_type_is_no_language() {
    // The body's island, if any: its language and recorded type.
    let body = |src: &str| {
        let tree = parse(src, Mode::Auto);
        islands(&tree)
            .first()
            .map(|e| (island_of(e).lang, island_of(e).script_type.clone()))
    };
    // CFML in the value (quoted, partial or unquoted, a `#…#` or a CF tag):
    // only the server knows the type, so the body is an island of no
    // language and no recorded type.
    for src in [
        "<cfoutput><script type=\"#kind#\">hello</script></cfoutput>",
        "<cfoutput><script type=\"#prefix#application/json\">{}</script></cfoutput>",
        "<cfoutput><script type=\"text/#x#\">hello</script></cfoutput>",
        "<cfoutput><script type=#kind#>hello</script></cfoutput>",
        "<cfoutput><script type=text/#x#>hello</script></cfoutput>",
        "<cfoutput><style type=\"#t#\">a{color:red}</style></cfoutput>",
        "<cfoutput><script type=\"a<cfif x>b</cfif>\">hello</script></cfoutput>",
    ] {
        assert_eq!(body(src), Some((Lang::Unknown, None)), "{src}");
    }
    // Outside `<cfoutput>` the `#` is text: a static value no MIME type
    // matches.
    assert_eq!(
        body("<script type=\"#kind#\">hello</script>"),
        Some((Lang::Unknown, Some("#kind#".into())))
    );
    // Empty, bare and absent: JavaScript, as before.
    for src in [
        "<script type=\"\">hello</script>",
        "<script type>hello</script>",
        "<script type async>hello</script>",
        "<script>hello</script>",
        "<cfoutput><script type>hello</script></cfoutput>",
    ] {
        assert_eq!(body(src), Some((Lang::Js, None)), "{src}");
    }
    // A static value inside `<cfoutput>` still selects its language.
    assert_eq!(
        body("<cfoutput><script type=\"application/json\">{}</script></cfoutput>"),
        Some((Lang::Json, Some("application/json".into())))
    );
    assert_eq!(
        body("<cfoutput><style type=\"text/css\">a{}</style></cfoutput>"),
        Some((Lang::Css, None))
    );
}

#[test]
fn attribute_trivia_and_generated_attributes_are_no_language() {
    let body = |src: &str| {
        let tree = parse(src, Mode::Auto);
        islands(&tree)
            .first()
            .map(|e| (island_of(e).lang, island_of(e).script_type.clone()))
    };
    // A newline on either side of `=` is trivia, as a space is; CFML where
    // an attribute name or an unquoted value stands can emit `type=`.
    for src in [
        "<cfoutput><script type=\n\"#kind#\">hello</script></cfoutput>",
        "<cfoutput><script type\n=\"#kind#\">hello</script></cfoutput>",
        "<cfoutput><script type\r\n=\r\n\"#kind#\">hello</script></cfoutput>",
        "<cfoutput><script #attrs#>hello</script></cfoutput>",
        "<cfoutput><script defer #attrs#>hello</script></cfoutput>",
        "<cfoutput><script type #attrs#>hello</script></cfoutput>",
        "<cfoutput><script type=\"text/javascript\" #attrs#>hello</script></cfoutput>",
        "<script <cfif x>type=\"text/plain\"</cfif>>hello</script>",
        "<script <cfif x>async</cfif>>hello</script>",
        "<cfoutput><style type=\n\"#t#\">a{color:red}</style></cfoutput>",
        "<cfoutput><style #attrs#>a{color:red}</style></cfoutput>",
        "<cfoutput><script src=#url#>hello</script></cfoutput>",
        "<cfoutput><script src=a#url#>hello</script></cfoutput>",
        "<script src=<cfoutput>x</cfoutput>>hello</script>",
        // A static unsupported type on the next line.
        "<script type=\n\"text/plain\">hello</script>",
        "<script type\n=\ntext/plain>hello</script>",
    ] {
        let static_type = src.contains("text/plain\">hello") || src.contains("text/plain>");
        let expected = static_type.then(|| "text/plain".to_string());
        assert_eq!(body(src), Some((Lang::Unknown, expected)), "{src}");
    }
    // CFML inside another attribute's quoted value emits no attribute; a
    // tag comment emits nothing; outside `<cfoutput>` a `#` is text.
    for src in [
        "<cfoutput><script src=\"#url#\">hello</script></cfoutput>",
        "<cfoutput><script src='#url#' defer>hello</script></cfoutput>",
        "<script <!--- c --->>hello</script>",
        "<script src=a<!--- c --->b>hello</script>",
        "<script #attrs#>hello</script>",
        "<script type=\n\"text/javascript\">hello</script>",
        "<script\ntype\n=\n\"module\">hello</script>",
    ] {
        assert!(
            matches!(body(src), Some((Lang::Js, _))),
            "{src}: {:?}",
            body(src)
        );
    }
    assert_eq!(
        body("<script type=\n\"module\">import x from 'y';</script>"),
        Some((Lang::Js, Some("module".into())))
    );
    assert_eq!(
        body("<cfoutput><style type=\n\"text/css\" media=\"#m#\">a{}</style></cfoutput>"),
        Some((Lang::Css, None))
    );
}

#[test]
fn event_attribute_island() {
    let tree = parse(
        "<button onclick=\"go(1)\" style=\"color: red\">x</button>",
        Mode::Auto,
    );
    let isl = islands(&tree);
    assert_eq!(isl.len(), 2);
    let sites: Vec<(Lang, IslandSite)> = isl
        .iter()
        .map(|e| (island_of(e).lang, island_of(e).site))
        .collect();
    assert_eq!(
        sites,
        vec![
            (Lang::Js, IslandSite::EventAttribute),
            (Lang::Css, IslandSite::StyleAttribute)
        ]
    );
    assert!(isl[0].is_pure_island());
    assert_eq!(text(&tree, &isl[0].children[0]), "go(1)");
}

#[test]
fn sql_strings_in_script_stay_strings() {
    let tree = parse("q = 'SELECT * FROM t WHERE a = #b#';", Mode::Script);
    assert!(islands(&tree).is_empty());
    let s = find(&tree, &|e| matches!(e.kind, ElementKind::String { .. }))[0];
    assert!(s
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::TemplateExpression)));
    assert!(s
        .children
        .iter()
        .filter_map(Node::as_token)
        .all(|t| t.kind == TokenKind::Literal(Literal::StringText)));
}

// ---------------------------------------------------------------------------
// Normalisation and ignore regions
// ---------------------------------------------------------------------------

#[test]
fn bom_is_stripped_and_flagged() {
    let tree = parse("\u{FEFF}component {}\n", Mode::Auto);
    assert!(tree.bom);
    assert_eq!(tree.mode(), Mode::Script);
    assert_eq!(tree.source, "component {}\n");
    let tree = parse("\u{FEFF}<cfset a = 1>", Mode::Auto);
    assert!(tree.bom);
    assert_eq!(tree.mode(), Mode::Tags);
    assert!(!parse("a = 1;", Mode::Script).bom);
}

#[test]
fn crlf_and_cr_inputs() {
    for (src, newline) in [
        ("a = 1;\r\nb = 2;\r\n", Newline::CrLf),
        ("a = 1;\rb = 2;\r", Newline::Cr),
        ("a = 1;\nb = 2;\n", Newline::Lf),
    ] {
        let tree = parse(src, Mode::Script);
        assert_eq!(tree.newline, newline, "{src:?}");
        assert!(!tree.source.contains('\r'));
        assert_eq!(statements(&tree.root).len(), 2, "{src:?}");
    }
}

#[test]
fn ignore_regions_are_verbatim() {
    let src = "a = 1;\r\n// cfformat-ignore-start\r\nb  =  {x:1};\r\n// cfformat-ignore-end\r\nc = 3;\r\n";
    let tree = parse(src, Mode::Script);
    let ignore = find(&tree, &|e| e.kind == ElementKind::Ignore);
    assert_eq!(ignore.len(), 1);
    assert_eq!(ignore[0].children.len(), 1);
    let t = tok(&ignore[0].children[0]);
    assert_eq!(t.kind, TokenKind::Ignore);
    assert_eq!(
        tree.verbatim(t.span.clone()),
        "// cfformat-ignore-start\r\nb  =  {x:1};\r\n// cfformat-ignore-end\r\n"
    );
    assert_eq!(statements(&tree.root).len(), 2);

    // Tag comments, with a BOM and CR-only line endings.
    let src = "\u{FEFF}<cfset a=1>\r<!--- cfformat-ignore-start --->\r<cfset  b = 2>\r<!--- cfformat-ignore-end --->\r";
    let tree = parse(src, Mode::Auto);
    let ignore = find(&tree, &|e| e.kind == ElementKind::Ignore);
    assert_eq!(
        tree.verbatim(ignore[0].span.clone()),
        "<!--- cfformat-ignore-start --->\r<cfset  b = 2>\r<!--- cfformat-ignore-end --->"
    );
}

#[test]
fn ignore_markers_read_alike_everywhere() {
    // Spaces or tabs around a `//` marker word; a form feed is neither, so
    // the comment is a line comment for the script parser as for the
    // scanners that find where a `<cfscript>` body ends.
    let region =
        |src: &str, mode| !find(&parse(src, mode), &|e| e.kind == ElementKind::Ignore).is_empty();
    assert!(region(
        "//\t cfformat-ignore-start \t\nb  =  1;\n",
        Mode::Script
    ));
    assert!(!region(
        "//\x0ccfformat-ignore-start\nb  =  1;\n",
        Mode::Script
    ));
    assert!(!region(
        "// cfformat-ignore-start\x0c\nb  =  1;\n",
        Mode::Script
    ));
    assert!(!region(
        "<cfscript>//\x0ccfformat-ignore-start\nb  =  1;</cfscript>",
        Mode::Tags
    ));
    // The block forms take any whitespace around the word, newlines too.
    assert!(region(
        "/*\n  cfformat-ignore-start\n*/\nb  =  1;\n",
        Mode::Script
    ));
    assert!(region(
        "<!---\n  cfformat-ignore-start\n--->\n<cfset  b = 1>",
        Mode::Tags
    ));
}

#[test]
fn tags_mode_overrides_script_detection() {
    let src = "// looks like script\nx = 1; <cfset a = 1>\n";
    assert_eq!(parse(src, Mode::Auto).mode(), Mode::Script);
    // Leading comments followed by a tag are tags under Auto too.
    assert_eq!(
        parse("// a note\n<cfset a = 1>\n", Mode::Auto).mode(),
        Mode::Tags
    );
    let tree = parse(src, Mode::Tags);
    assert_eq!(tree.mode(), Mode::Tags);
    let tags = find(&tree, &|e| matches!(e.kind, ElementKind::CfTag(..)));
    assert_eq!(tags.len(), 1);
    assert_eq!(tree.tag_name(tags[0]), Some("cfset"));
}

#[test]
fn line_comment_newline_is_a_sibling() {
    let tree = parse("a = 1; // note\nb = 2;", Mode::Script);
    let comment = find(&tree, &|e| e.kind == ElementKind::LineComment)[0];
    assert_eq!(tree.text(comment.open.as_ref().unwrap()), "//");
    assert_eq!(comment.children.len(), 1);
    assert_eq!(text(&tree, &comment.children[0]), " note");
    let pos = tree
        .root
        .children
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::LineComment))
        .unwrap();
    assert!(matches!(
        &tree.root.children[pos + 1],
        Node::Token(t) if t.kind == TokenKind::Newline
    ));
}

#[test]
fn adjacent_line_comments_stay_separate() {
    // Unindented `//` lines are separate comments, not merged into one.
    let tree = parse("//a\n//b\n//\n   \n//c\nx = 1;\n", Mode::Script);
    let comments = find(&tree, &|e| e.kind == ElementKind::LineComment);
    assert_eq!(comments.len(), 4);
    let texts: Vec<_> = comments
        .iter()
        .map(|c| tree.slice(c.span.clone()))
        .collect();
    assert_eq!(texts, ["//a", "//b", "//", "//c"]);
    for c in &comments {
        assert!(c.close.is_none());
        assert!(c.children.len() <= 1);
        for n in &c.children {
            let t = tok(n);
            assert_eq!(t.kind, TokenKind::CommentText);
            assert!(!tree.text(t).contains('\n'));
        }
    }
    let newlines = tree
        .root
        .children
        .iter()
        .filter(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Newline))
        .count();
    assert_eq!(newlines, 6);
}

#[test]
fn tokens_are_a_flat_tiling_of_the_source() {
    let src = "<cfif x>#y#</cfif>\n<script>var a = 1 < 2;</script>\n";
    let tokens = parse_source(src, Mode::Auto).root.tokens();
    let mut pos = 0;
    for t in &tokens {
        assert_eq!(t.span.start, pos);
        pos = t.span.end;
    }
    assert_eq!(pos as usize, src.len());
    let texts: Vec<&str> = tokens
        .iter()
        .map(|t| &src[t.span.start as usize..t.span.end as usize])
        .collect();
    // The `<script>` body is one host-text token: the `<` inside it is not
    // a tag.
    assert!(texts.contains(&"var a = 1 < 2;"));
    assert!(texts.contains(&"#y#"));
    assert_eq!(
        tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Ident(Ident::TagName))
            .count(),
        4
    );
    // Same token boundaries as the tree, apart from coalescing.
    let tree = parse(src, Mode::Auto);
    let tree_tokens = common::flatten(&tree.root);
    assert_eq!(
        tree_tokens.first().unwrap().span,
        tokens.first().unwrap().span
    );
    assert_eq!(
        tree_tokens.last().unwrap().span,
        tokens.last().unwrap().span
    );
}

#[test]
fn tag_in_tag_expression_strings_are_not_attributes() {
    let tree = parse(
        "<cfset a = \"x\"><cfhttp url=\"y\"><cfif b is \"z\"></cfif>",
        Mode::Auto,
    );
    let strings: Vec<bool> = find(&tree, &|e| matches!(e.kind, ElementKind::String { .. }))
        .into_iter()
        .map(|e| matches!(e.kind, ElementKind::String { in_tag: true, .. }))
        .collect();
    assert_eq!(strings, vec![false, true, false]);
}

// ---------------------------------------------------------------------------
// Statement shapes
// ---------------------------------------------------------------------------

#[test]
fn static_call_at_statement_start_is_not_a_label() {
    let tree = parse("Service::get();", Mode::Script);
    assert_eq!(
        root_shapes(&tree),
        ["statement(chain(Service segment(:: get call(( )))) ;)"]
    );
    // No stray `Other` tokens.
    assert!(common::flatten(&tree.root)
        .iter()
        .all(|t| t.kind != TokenKind::Other));
}

#[test]
fn a_real_label_still_labels() {
    let tree = parse("foo: for (;;) {}", Mode::Script);
    let labels: Vec<_> = common::flatten(&tree.root)
        .into_iter()
        .filter(|t| t.kind == TokenKind::Ident(Ident::Label))
        .map(|t| tree.text(t))
        .collect();
    assert_eq!(labels, ["foo"]);
}

#[test]
fn ternary_at_statement_start_is_not_a_label() {
    let tree = parse("a ? b : c;", Mode::Script);
    assert!(common::flatten(&tree.root)
        .iter()
        .all(|t| t.kind != TokenKind::Ident(Ident::Label)));
}

#[test]
fn catch_accepts_a_quoted_exception_type() {
    for src in [
        "try {} catch (\"java.lang.Exception\" e) {}",
        "try {} catch ('TestBox.SkipSpec' e) {}",
    ] {
        let tree = parse(src, Mode::Script);
        let toks = common::flatten(&tree.root);
        assert!(
            toks.iter().all(|t| t.kind != TokenKind::Other),
            "{src}: {:?}",
            toks.iter()
                .filter(|t| t.kind == TokenKind::Other)
                .map(|t| tree.text(t))
                .collect::<Vec<_>>()
        );
        let types: Vec<_> = toks
            .iter()
            .filter(|t| t.kind == TokenKind::Ident(Ident::ClassName))
            .map(|t| tree.text(t))
            .collect();
        assert_eq!(types.len(), 1, "{src}");
        assert!(!types[0].contains('"') && !types[0].contains('\''), "{src}");
        // The type is not a String element; it is a type name.
        assert!(find(&tree, &|e| matches!(e.kind, ElementKind::String { .. })).is_empty());
    }
}

#[test]
fn catch_still_accepts_a_bare_type_and_a_bare_name() {
    for (src, types) in [
        ("try {} catch (any e) {}", 1),
        ("try {} catch (e) {}", 0),
        ("try {} catch (java.lang.Exception var e) {}", 1),
    ] {
        let tree = parse(src, Mode::Script);
        assert_eq!(
            common::flatten(&tree.root)
                .iter()
                .filter(|t| t.kind == TokenKind::Ident(Ident::ClassName))
                .count(),
            types,
            "{src}"
        );
    }
}

#[test]
fn constructor_accepts_a_string_entity_name() {
    for src in ["b = new \"com.foo\"(1);", "b = new '#path#'(a = b);"] {
        let tree = parse(src, Mode::Script);
        let stmts = statements(&tree.root);
        assert_eq!(stmts.len(), 1, "{src}: {:?}", root_shapes(&tree));
        assert_eq!(tree.text(stmts[0].terminator().unwrap()), ";");
        // The whole `new …(…)` is one statement: string, then the arguments.
        let new = find(&tree, &|e| e.kind == ElementKind::New)[0];
        let kinds: Vec<_> = new
            .children
            .iter()
            .filter_map(Node::as_element)
            .map(|e| e.kind.name())
            .collect();
        assert_eq!(kinds, ["string", "call"], "{src}");
    }
}

#[test]
fn exponent_and_integer_division_are_binary_operators_at_their_levels() {
    let ops = |tree: &Tree| -> Vec<(TokenKind, String)> {
        common::flatten(&tree.root)
            .into_iter()
            .filter(|t| matches!(tree.text(t), "^" | "\\"))
            .map(|t| (t.kind, tree.text(t).to_string()))
            .collect()
    };
    let arith = |text: &str| {
        let prec = if text == "^" {
            Prec::Exponent
        } else {
            Prec::Multiplicative
        };
        (
            TokenKind::Operator(Operator::Binary(prec)),
            text.to_string(),
        )
    };
    for (src, op) in [
        ("a = b ^ 2;", "^"),
        ("a = b \\ c;", "\\"),
        ("a = b\\c;", "\\"),
    ] {
        let tree = parse(src, Mode::Script);
        assert_eq!(
            statements(&tree.root).len(),
            1,
            "{src}: {:?}",
            root_shapes(&tree)
        );
        assert_eq!(ops(&tree), [arith(op)], "{src}");
    }
    // A leading `\` continues a semicolon-less statement, as `^` already did.
    let tree = parse("a = b\n\\ c\nd = 1", Mode::Script);
    assert_eq!(statements(&tree.root).len(), 2);
    // `\` inside a string is text, not an operator.
    let tree = parse("x = \"a\\b\" & a \\ b;", Mode::Script);
    assert_eq!(ops(&tree), [arith("\\")]);
    // Tag mode reaches the same rule through `source.cfml.script.tags`.
    let tree = parse(
        "<cfset x = a ^ 2><cfset y = a \\ 3><cfif a \\ 2 eq 1></cfif>",
        Mode::Tags,
    );
    assert_eq!(ops(&tree), [arith("^"), arith("\\"), arith("\\")]);
}

#[test]
fn static_before_an_access_modifier_and_a_return_type_declares_a_function() {
    for src in [
        "static function f() {}",
        "static public function f() {}",
        "static public struct function f() {}",
        "final public string function f() {}",
    ] {
        let tree = parse(&format!("component {{\n{src}\n}}"), Mode::Script);
        let class = find(&tree, &|e| e.kind == ElementKind::Block(BlockKind::Class))[0];
        assert_eq!(
            statements(class).len(),
            1,
            "{src}: {:?}",
            root_shapes(&tree)
        );
    }
}

#[test]
fn new_takes_a_class_name_that_starts_with_a_keyword() {
    for src in [
        "x = new component2();",
        "x = new javaLoader();",
        "x = new Component1();",
    ] {
        let tokens = parse_source(src, Mode::Script).root.tokens();
        let odd: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Other | TokenKind::Invalid))
            .collect();
        assert!(odd.is_empty(), "{src}: {odd:?}");
        assert_eq!(statements(&parse(src, Mode::Script).root).len(), 1);
    }
}

#[test]
fn generic_script_tags_may_carry_the_cf_prefix() {
    for src in [
        "cfinvoke\n    component=\"a\"\n    method=\"b\";\nreturn 1;",
        "cfhttp url=\"x\" result=\"r\";\nreturn 1;",
        "invoke\n    component=\"a\"\n    method=\"b\";\nreturn 1;",
    ] {
        let tree = parse(src, Mode::Script);
        assert_eq!(
            find(&tree, &|e| e.kind == ElementKind::ScriptTag { acf: false }).len(),
            1,
            "{src}: {:?}",
            root_shapes(&tree)
        );
        assert_eq!(statements(&tree.root).len(), 2, "{src}");
    }
    // A bare `cfquery` heading a chain on the next line is still a variable.
    let tree = parse("cfquery\n    .where(1);", Mode::Script);
    assert!(find(&tree, &|e| e.kind == ElementKind::ScriptTag { acf: false }).is_empty());
}

#[test]
fn acf_script_tags_allow_a_space_before_the_parenthesis() {
    // Lucee writes `cffile (action="write" …);` and `cfdocument (…) { }`.
    for src in [
        "cffile(action=\"write\", file=\"a\");",
        "cffile (action=\"write\" file=\"a\");",
        "cfdocument (format=\"PDF\") {\n    echo(\"x\");\n}",
    ] {
        let tree = parse(src, Mode::Script);
        assert_eq!(
            find(&tree, &|e| e.kind == ElementKind::ScriptTag { acf: true }).len(),
            1,
            "{src}: {:?}",
            root_shapes(&tree)
        );
        assert_eq!(statements(&tree.root).len(), 1, "{src}");
    }
}

#[test]
fn tag_in_script_statements_end_where_they_should() {
    // A bare tag name ending the line is only a tag when the next line does
    // not start with `.`: `query\n    .where(…)` is a member chain (branch
    // point `tag-in-script-bare`).
    let tree = parse("query\n    .where(1)\n    .where(2);\nx = 1;", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2, "{:?}", root_shapes(&tree));
    assert_eq!(
        stmts[0].kind,
        ElementKind::Statement(StatementKind::Expression)
    );
    assert!(find(&tree, &|e| e.kind == ElementKind::ScriptTag { acf: false }).is_empty());
    assert_eq!(find(&tree, &|e| e.kind == ElementKind::Chain).len(), 1);
    // A block-bodied tag in script ends its statement at the `}`; the next
    // line is a new statement, not swallowed into the same one.
    for src in [
        "transaction {\n    x = 1;\n}\ny = 2;",
        "lock name=\"a\" timeout=1 {\n    x = 1;\n}\ny = 2;",
        "transaction\n{\n}\ny = 2;",
        "http url=\"x\";\ny = 2;",
        "http\n    url=\"x\";\ny = 2;",
    ] {
        let tree = parse(src, Mode::Script);
        let stmts = statements(&tree.root);
        assert_eq!(stmts.len(), 2, "{src}: {:?}", root_shapes(&tree));
        assert_eq!(
            stmts[0].kind,
            ElementKind::Statement(StatementKind::ScriptTag),
            "{src}"
        );
        assert_eq!(
            stmts[1].kind,
            ElementKind::Statement(StatementKind::Assignment),
            "{src}"
        );
    }
    // Inside a function body as well.
    let tree = parse(
        "function f() {\n    query name=\"q\" {\n    }\n    query\n        .from(\"t\");\n}",
        Mode::Script,
    );
    let body = find(&tree, &|e| {
        e.kind == ElementKind::Block(BlockKind::Function)
    });
    assert_eq!(statements(body[0]).len(), 2);
}

#[test]
fn unquoted_parameter_attribute_values_end_at_the_delimiter() {
    // The unquoted value ends at the `)`: not `true)` with the rest of the
    // file read as attribute names and `invalid` tokens.
    let tree = parse(
        "function f(required string a key=true, string b other=x.y) {\n    return 1;\n}\nx = 1;",
        Mode::Script,
    );
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2, "{:?}", root_shapes(&tree));
    let unquoted: Vec<&str> = find(&tree, &|e| e.kind == ElementKind::KeyValue)
        .iter()
        .flat_map(|kv| kv.children.iter())
        .filter(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Literal(Literal::Unquoted)))
        .map(|n| text(&tree, n))
        .collect();
    assert_eq!(unquoted, ["true", "x.y"]);
    assert!(find(&tree, &|e| e.kind == ElementKind::Parameters).len() == 1);
    let body = find(&tree, &|e| {
        e.kind == ElementKind::Block(BlockKind::Function)
    });
    assert_eq!(statements(body[0]).len(), 1);
}

#[test]
fn default_is_an_identifier_outside_a_switch_label() {
    // `default` is not a reserved word: `x = default` is an assignment, not
    // an unmatched `d` plus the variable `efault`.
    let tree = parse("x = default; var default = 1; y = a.default;", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 3, "{:?}", root_shapes(&tree));
    assert_eq!(
        stmts[1].kind,
        ElementKind::Statement(StatementKind::Declaration)
    );
    assert!(find(&tree, &|e| e.kind == ElementKind::Case).is_empty());
    assert!(!tree
        .root
        .nodes()
        .any(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Other)));
    // The switch label still wins, also with a space before the colon, and a
    // `default` assignment inside a case body is a statement, not a label.
    let tree = parse(
        "switch (x) { case 1: default = 1; break; default : break; }",
        Mode::Script,
    );
    let cases = find(&tree, &|e| e.kind == ElementKind::Case);
    assert_eq!(cases.len(), 2);
    assert_eq!(statements(cases[0]).len(), 2);
    assert_eq!(statements(cases[1]).len(), 1);
}

#[test]
fn whitespace_inside_a_meta_region_is_its_own_token() {
    // A struct key token never holds the whitespace before its separator.
    let tree = parse("x = { c = 1, dd\t: 2, \"e\" = 3 };", Mode::Script);
    let toks: Vec<(String, &str)> = common::flatten(&tree.root)
        .into_iter()
        .map(|t| (t.kind.name(), tree.text(t)))
        .collect();
    let at = |text: &str| toks.iter().position(|(_, t)| *t == text).unwrap();
    let c = at("c");
    assert_eq!(toks[c].0, "ident.struct-key");
    assert_eq!(toks[c + 1], ("ws".to_string(), " "));
    assert_eq!(toks[c + 2], ("punct.key-value".to_string(), "="));
    let dd = at("dd");
    assert_eq!(toks[dd].0, "ident.struct-key");
    assert_eq!(toks[dd + 1], ("ws".to_string(), "\t"));
    assert_eq!(toks[dd + 2], ("punct.key-value".to_string(), ":"));
    assert!(toks
        .iter()
        .all(|(k, t)| k == "ws" || !t.chars().any(char::is_whitespace)));
}

#[test]
fn a_question_mark_directly_followed_by_a_colon_is_the_elvis_operator() {
    let ops = |src: &str, mode: Mode| -> Vec<(String, String)> {
        let tree = parse(src, mode);
        common::flatten(&tree.root)
            .into_iter()
            .filter(|t| matches!(t.kind, TokenKind::Operator(_)))
            .map(|t| (t.kind.name(), tree.text(t).to_string()))
            .collect()
    };
    let op = |k: &str, t: &str| (format!("op.{k}"), t.to_string());
    assert_eq!(
        ops("var z = cond ? a : b ?: c;", Mode::Script),
        [
            op("assign", "="),
            op("ternary-q", "?"),
            op("ternary-colon", ":"),
            op("elvis", "?:"),
        ]
    );
    // With a space between them they stay a ternary's two tokens.
    assert_eq!(
        ops("y = a ? : b;", Mode::Script),
        [
            op("assign", "="),
            op("ternary-q", "?"),
            op("ternary-colon", ":")
        ]
    );
    assert_eq!(
        ops("<cfset y = a ?: b>", Mode::Tags),
        [op("assign", "="), op("elvis", "?:")]
    );
    let tokens = parse_source("z = a ?: b;", Mode::Script).root.tokens();
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Operator(Operator::Binary(Prec::Elvis)) && t.span == (6..8)));
}

#[test]
fn import_is_a_statement() {
    let tree = parse("import a.b.c;\nimport a.b.*;\n", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2);
    for s in &stmts {
        assert_eq!(tree.text(s.terminator().unwrap()), ";");
        let kids = significant(&s.children);
        assert_eq!(kids.len(), 1);
        let import = kids[0].as_element().expect("import element");
        assert_eq!(import.kind, ElementKind::Import);
    }
    // The whole dotted path is inside the Import element.
    let paths: Vec<_> = find(&tree, &|e| e.kind == ElementKind::Import)
        .into_iter()
        .map(|e| tree.slice(e.span.clone()).to_string())
        .collect();
    assert_eq!(paths, ["import a.b.c", "import a.b.*"]);
}

#[test]
fn import_inside_a_component_body_is_a_statement() {
    let tree = parse(
        "component {\n    import foo.Bar;\n    x = 1;\n}\n",
        Mode::Script,
    );
    let block = find(&tree, &|e| e.kind == ElementKind::Block(BlockKind::Class));
    assert_eq!(block.len(), 1);
    let stmts = statements(block[0]);
    assert_eq!(stmts.len(), 2);
    assert_eq!(
        significant(&stmts[0].children)[0]
            .as_element()
            .map(|e| e.kind.clone()),
        Some(ElementKind::Import)
    );
}

#[test]
fn mandatory_semicolons_terminate_their_statement() {
    // `do … while (x);`
    let tree = parse("do foo(); while (x); baz();", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2, "{:?}", root_shapes(&tree));
    assert_eq!(tree.text(stmts[0].terminator().unwrap()), ";");
    assert_eq!(
        significant(&stmts[0].children)[0]
            .as_element()
            .map(|e| e.kind.clone()),
        Some(ElementKind::DoWhile)
    );

    // `function f();` in an interface.
    let tree = parse("interface {\n    function f();\n}\n", Mode::Script);
    let block = find(&tree, &|e| {
        e.kind == ElementKind::Block(BlockKind::Interface)
    });
    let stmts = statements(block[0]);
    assert_eq!(stmts.len(), 1, "{:?}", stmts.len());
    assert_eq!(tree.text(stmts[0].terminator().unwrap()), ";");
    assert_eq!(
        significant(&stmts[0].children)[0]
            .as_element()
            .map(|e| e.kind.clone()),
        Some(ElementKind::FunctionDecl)
    );
}

#[test]
fn a_semicolon_after_a_function_body_is_still_an_empty_statement() {
    let tree = parse("function f() {};", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2);
    assert!(stmts[0].terminator().is_none());
    assert_eq!(tree.text(stmts[1].terminator().unwrap()), ";");
    assert!(significant(&stmts[1].children).is_empty());
}

#[test]
fn an_empty_statement_after_a_block_is_kept() {
    let tree = parse("if (a) {};", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2, "{:?}", root_shapes(&tree));
    assert!(significant(&stmts[1].children).is_empty());
    assert_eq!(tree.text(stmts[1].terminator().unwrap()), ";");
}

/// The body of a keyword statement: its last significant child.
fn body_of(el: &Element) -> &Element {
    significant(&el.children)
        .last()
        .and_then(|n| n.as_element())
        .unwrap_or_else(|| panic!("{} has no body element", el.kind.name()))
}

#[test]
fn every_unbraced_body_is_a_statement_inside_the_keyword_element() {
    let cases = [
        ("if (x) foo();", ElementKind::If),
        ("else foo();", ElementKind::Else),
        ("while (x) foo();", ElementKind::While),
        ("for (;;) foo();", ElementKind::For),
        ("for (a in b) foo();", ElementKind::For),
        ("do foo(); while (x);", ElementKind::DoWhile),
    ];
    for (src, kind) in cases {
        let tree = parse(src, Mode::Script);
        let owner = find(&tree, &|e| e.kind == kind);
        assert_eq!(owner.len(), 1, "{src}");
        let body = if kind == ElementKind::DoWhile {
            // `do`'s body is followed by the `while (…)` condition.
            significant(&owner[0].children)
                .into_iter()
                .filter_map(Node::as_element)
                .next()
                .unwrap()
        } else {
            body_of(owner[0])
        };
        assert!(body.kind.is_statement(), "{src}");
        assert_eq!(tree.slice(body.span.clone()), "foo();", "{src}");
    }
}

#[test]
fn every_braced_body_is_a_block_inside_the_keyword_element() {
    for (src, kind) in [
        ("if (x) { foo(); }", ElementKind::If),
        ("else { foo(); }", ElementKind::Else),
        ("while (x) { foo(); }", ElementKind::While),
        ("for (;;) { foo(); }", ElementKind::For),
    ] {
        let tree = parse(src, Mode::Script);
        let owner = find(&tree, &|e| e.kind == kind);
        assert_eq!(owner.len(), 1, "{src}");
        assert_eq!(
            body_of(owner[0]).kind,
            ElementKind::Block(BlockKind::Plain),
            "{src}"
        );
    }
}

#[test]
fn else_with_a_braced_if_is_not_a_chain() {
    let tree = parse("if (a) {} else { if (b) c(); }", Mode::Script);
    let elses = find(&tree, &|e| e.kind == ElementKind::Else);
    assert_eq!(elses.len(), 1);
    let block = body_of(elses[0]);
    assert_eq!(block.kind, ElementKind::Block(BlockKind::Plain));
    let inner = statements(block);
    assert_eq!(inner.len(), 1);
    assert_eq!(
        significant(&inner[0].children)[0]
            .as_element()
            .map(|e| e.kind.clone()),
        Some(ElementKind::If)
    );
}

#[test]
fn an_if_else_chain_is_one_element() {
    let tree = parse("if (a) {} else if (b) {} else {}", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 1, "{:?}", root_shapes(&tree));
    let ifs = find(&tree, &|e| e.kind == ElementKind::If);
    assert_eq!(ifs.len(), 1);
    assert_eq!(
        tree.slice(ifs[0].span.clone()),
        "if (a) {} else if (b) {} else {}"
    );
    // Each clause is nested in the chain, in source order.
    let clauses: Vec<_> = significant(&ifs[0].children)
        .into_iter()
        .filter_map(Node::as_element)
        .map(|e| e.kind.name())
        .collect();
    assert_eq!(clauses, ["group", "block", "else-if", "else"]);
    let elseif = find(&tree, &|e| e.kind == ElementKind::ElseIf);
    assert_eq!(elseif.len(), 1);
    assert_eq!(tree.slice(elseif[0].span.clone()), "else if (b) {}");
    let els = find(&tree, &|e| e.kind == ElementKind::Else);
    assert_eq!(tree.slice(els[0].span.clone()), "else {}");
}

#[test]
fn a_try_catch_finally_chain_is_one_element() {
    let tree = parse(
        "try {} catch (any e) {} catch (b f) {} finally {}",
        Mode::Script,
    );
    assert_eq!(statements(&tree.root).len(), 1);
    let tries = find(&tree, &|e| e.kind == ElementKind::Try);
    assert_eq!(tries.len(), 1);
    let clauses: Vec<_> = significant(&tries[0].children)
        .into_iter()
        .filter_map(Node::as_element)
        .map(|e| e.kind.name())
        .collect();
    assert_eq!(clauses, ["block", "catch", "catch", "finally"]);
    for c in find(&tree, &|e| e.kind == ElementKind::Catch) {
        assert_eq!(body_of(c).kind, ElementKind::Block(BlockKind::Plain));
    }
}

#[test]
fn comments_between_clauses_are_children_of_the_chain() {
    let tree = parse("if (a) {}\n// why\nelse {}\n", Mode::Script);
    let ifs = find(&tree, &|e| e.kind == ElementKind::If);
    assert_eq!(ifs.len(), 1);
    let kinds: Vec<_> = significant(&ifs[0].children)
        .into_iter()
        .map(|n| match n {
            Node::Token(t) => t.kind.name(),
            Node::Element(e) => e.kind.name().to_string(),
        })
        .collect();
    assert_eq!(kinds, ["kw.if", "group", "block", "line-comment", "else"]);
}

#[test]
fn a_dangling_else_binds_to_the_inner_if() {
    let tree = parse("if (a) if (b) c(); else d();", Mode::Script);
    let ifs = find(&tree, &|e| e.kind == ElementKind::If);
    assert_eq!(ifs.len(), 2);
    // The outer `if` spans everything; the `else` belongs to the inner one.
    let inner = ifs[1];
    assert_eq!(
        find(&tree, &|e| e.kind == ElementKind::Else)
            .into_iter()
            .map(|e| tree.slice(e.span.clone()).to_string())
            .collect::<Vec<_>>(),
        ["else d();"]
    );
    assert!(inner
        .children
        .iter()
        .filter_map(Node::as_element)
        .any(|e| e.kind == ElementKind::Else));
}

#[test]
fn a_chain_ends_at_the_next_statement() {
    let tree = parse("if (a) {} foo();", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(stmts.len(), 2, "{:?}", root_shapes(&tree));
    assert_eq!(tree.slice(stmts[0].span.clone()), "if (a) {}");
}

#[test]
fn switch_cases_are_elements() {
    let tree = parse(
        "switch (x) { case 1: a(); break; default: b(); }",
        Mode::Script,
    );
    let block = find(&tree, &|e| e.kind == ElementKind::Block(BlockKind::Plain));
    assert_eq!(block.len(), 1);
    // The switch block holds Case elements, not bare `case` tokens.
    assert!(significant(&block[0].children)
        .iter()
        .all(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::Case)));
    let cases = find(&tree, &|e| e.kind == ElementKind::Case);
    assert_eq!(
        cases
            .iter()
            .map(|c| tree.slice(c.span.clone()))
            .collect::<Vec<_>>(),
        ["case 1: a(); break;", "default: b();"]
    );
    // The colon is inside the case, and the statements follow it.
    assert_eq!(statements(cases[0]).len(), 2);
    assert!(common::flatten(cases[0])
        .iter()
        .any(|t| t.kind == TokenKind::Punct(Punct::Colon)));
}

#[test]
fn fallthrough_is_two_adjacent_cases() {
    let tree = parse("switch (x) { case 1: case 2: a(); }", Mode::Script);
    let cases = find(&tree, &|e| e.kind == ElementKind::Case);
    assert_eq!(
        cases
            .iter()
            .map(|c| tree.slice(c.span.clone()))
            .collect::<Vec<_>>(),
        ["case 1:", "case 2: a();"]
    );
    assert!(statements(cases[0]).is_empty());
}

#[test]
fn a_braced_case_body_stays_inside_its_case() {
    let tree = parse(
        "switch (x) { case 1: { a(); } break; default: }",
        Mode::Script,
    );
    let cases = find(&tree, &|e| e.kind == ElementKind::Case);
    assert_eq!(cases.len(), 2);
    assert_eq!(tree.slice(cases[0].span.clone()), "case 1: { a(); } break;");
    assert_eq!(tree.slice(cases[1].span.clone()), "default:");
}

#[test]
fn tags_mode_has_its_own_entry_context() {
    // No synthetic comment is inserted: tag mode has no script lookahead,
    // so every byte of the input is covered as written.
    for src in [
        "component { x = 1; }",
        "// looks like script\n<cfset a = 1>\n",
        "import foo.Bar;\n",
        "\u{feff}component { x = 1; }",
    ] {
        let tree = parse(src, Mode::Tags);
        assert_eq!(tree.mode(), Mode::Tags, "{src:?}");
        let joined: String = common::flatten(&tree.root)
            .iter()
            .map(|t| tree.text(t))
            .collect();
        assert_eq!(joined, tree.source, "{src:?}");
    }
    // `component {` in tags mode is HTML text, not a class declaration.
    let tree = parse("component { x = 1; }", Mode::Tags);
    assert!(find(&tree, &|e| e.kind == ElementKind::ClassDecl).is_empty());
    assert_eq!(
        parse("component { x = 1; }", Mode::Auto).mode(),
        Mode::Script
    );
}

// ---------------------------------------------------------------------------
// Tag pairing
// ---------------------------------------------------------------------------

#[test]
fn cf_tags_carry_the_kind_the_scanner_read() {
    let src = concat!(
        "<cfcomponent><cffunction name=\"f\"><cfset x = 1><cfif a><cfelseif b><cfelse>",
        "<cfreturn 1></cfif></cffunction><cfproperty name=\"p\"><cfscript>y = 2;</cfscript>",
        "<cfquery name=\"q\">select 1</cfquery><cfoutput></cfoutput><cfmail></cfmail>",
        "<cfjava>x</cfjava><cfloop></cfloop><cf_custom><cfelse:x><cfset2 x>",
        "<cfoutputs></cfoutputs><cfjavax></cfjavax></cfcomponent>",
    );
    let tree = parse(src, Mode::Tags);
    let tags = find(&tree, &|e| matches!(e.kind, ElementKind::CfTag(..)));
    let kinds: Vec<String> = tags
        .iter()
        .map(|e| {
            let ElementKind::CfTag(shape, kind) = e.kind else {
                unreachable!()
            };
            let slash = if shape == TagShape::Close { "/" } else { "" };
            format!("{slash}{} {}", tree.tag_name(e).unwrap(), kind.name())
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "cfcomponent class",
            "cffunction function",
            "cfset expression",
            "cfif expression",
            "cfelseif else-if",
            "cfelse else",
            "cfreturn expression",
            "/cfif expression",
            "/cffunction function",
            "cfproperty property",
            "cfscript script",
            "/cfscript script",
            "cfquery query",
            "/cfquery query",
            "cfoutput output",
            "/cfoutput output",
            "cfmail output",
            "/cfmail output",
            "cfjava java",
            "/cfjava java",
            "cfloop generic",
            "/cfloop generic",
            "cf_custom generic",
            "cfelse:x generic",
            // The `\b` after `cfset` fails: a generic tag named `cfset`.
            "cfset generic",
            // As for `cfoutput` and `cfjava`: generic tags, as their closers.
            "cfoutputs generic",
            "/cfoutputs generic",
            "cfjavax generic",
            "/cfjavax generic",
            "/cfcomponent class",
        ]
    );
    // A `<cfcomponent>` past the head of the file is read as any tag.
    let tree = parse("<p></p><cfcomponent></cfcomponent>", Mode::Tags);
    let body = find(&tree, &|e| {
        matches!(e.kind, ElementKind::TagBody { cf: true })
    });
    assert_eq!(body[0].cf_kind(), Some(CfKind::Generic));
}

#[test]
fn matching_tags_become_tag_bodies() {
    let src = "<cfif a>\n  <cfset x = 1>\n<cfelse>\n  <div class=\"a\">hi</div>\n  <cf_custom attr=\"1\">\n  <my:tag />\n</cfif>\n<br>\n<cfabort>\n";
    let tree = parse(src, Mode::Tags);
    let kinds: Vec<String> = significant(&tree.root.children)
        .into_iter()
        .map(|n| match n {
            Node::Element(e) => format!("{} {}", e.kind.name(), tree.tag_name(e).unwrap()),
            Node::Token(t) => tree.text(t).to_string(),
        })
        .collect();
    assert_eq!(kinds, ["tag-body cfif", "html-tag br", "cf-tag cfabort"]);

    let body = tree.root.children[0].as_element().unwrap();
    assert_eq!(body.kind, ElementKind::TagBody { cf: true });
    assert!(body.open.is_none() && body.close.is_none());
    assert_eq!(
        tree.slice(body.span.clone()),
        &src[..src.find("\n<br>").unwrap()]
    );
    assert_eq!(
        body.open_tag().unwrap().kind,
        ElementKind::CfTag(TagShape::Open, CfKind::Expression)
    );
    assert_eq!(
        body.close_tag().unwrap().kind,
        ElementKind::CfTag(TagShape::Close, CfKind::Expression)
    );
    assert_eq!(body.cf_kind(), Some(CfKind::Expression));
    // `<cfelse>`, `<cf_custom>` (never closed) and `<my:tag />` stay bare
    // inside the body; `<div>…</div>` pairs.
    let inner: Vec<String> = significant(body.body())
        .into_iter()
        .map(|n| match n {
            Node::Element(e) => format!("{} {}", e.kind.name(), tree.tag_name(e).unwrap()),
            Node::Token(t) => tree.text(t).to_string(),
        })
        .collect();
    assert_eq!(
        inner,
        [
            "cf-tag cfset",
            "cf-tag cfelse",
            "tag-body div",
            "cf-tag cf_custom",
            "cf-tag my:tag"
        ]
    );
    let div = significant(body.body())[2].as_element().unwrap();
    assert_eq!(div.kind, ElementKind::TagBody { cf: false });
    // HTML text between tags stays a bare `Text` token.
    assert_eq!(
        shape(&tree, div),
        "tag-body(html-tag(< div key-value(class = string(\" a \")) >) hi html-tag(</ div >))"
    );
    assert!(matches!(&div.body()[0], Node::Token(t) if t.kind == TokenKind::Text));
    // Non-TagBody elements have no tag accessors.
    assert!(div.open_tag().unwrap().open_tag().is_none());
    assert!(div.open_tag().unwrap().body().is_empty());
}

#[test]
fn tag_pairing_is_case_insensitive_and_handles_custom_tags() {
    let tree = parse(
        "<CFIF a><cf_mail to=\"x\"></CF_MAIL><my:tag></my:TAG></cfif>",
        Mode::Tags,
    );
    let bodies: Vec<&str> = find(&tree, &|e| matches!(e.kind, ElementKind::TagBody { .. }))
        .into_iter()
        .map(|e| tree.tag_name(e).unwrap())
        .collect();
    assert_eq!(bodies, ["CFIF", "cf_mail", "my:tag"]);
}

#[test]
fn an_unmatched_opening_tag_stays_bare() {
    let tree = parse("<cfif a>\n<div>\n</cfif>\n", Mode::Tags);
    assert_eq!(
        root_shapes(&tree),
        ["tag-body(cf-tag(< cfif a >) html-tag(< div >) cf-tag(</ cfif >))"]
    );
    // The CFML walk pairs a closing tag with the *nearest* opening tag before
    // it; the earlier one stays bare.
    let tree = parse("<p>a<p>b</p>", Mode::Tags);
    assert_eq!(
        root_shapes(&tree),
        [
            "html-tag(< p >)",
            "a",
            "tag-body(html-tag(< p >) b html-tag(</ p >))"
        ]
    );
}

#[test]
fn unbalanced_closing_tags_are_content() {
    // A closing tag that never pairs stays where it is, bare, like an
    // unmatched opening tag (where the CFML throws). A CF one is a recovery:
    // the innermost tag body around it, or outside any the tag itself.
    let shapes = |src: &str| root_shapes(&parse(src, Mode::Tags));
    assert_eq!(
        shapes("</cfif>\n<cfset x = 1>\n"),
        [
            "recovered(cf-tag(</ cfif >))",
            "cf-tag(< cfset assignment(x = 1) >)"
        ]
    );
    assert_eq!(
        shapes("<cfset x = 1>\n\n</DIV>\n"),
        ["cf-tag(< cfset assignment(x = 1) >)", "html-tag(</ DIV >)"]
    );
    // Mis-nested: the CF layer pairs `<cfif>` first (rather than the
    // innermost `</div>` pairing and leaving both CF tags recoveries), and
    // the HTML layer inside the CF body releases `</div>`: both HTML tags bare.
    assert_eq!(
        shapes("<div><cfif a></div></cfif>"),
        [
            "html-tag(< div >)",
            "tag-body(cf-tag(< cfif a >) html-tag(</ div >) cf-tag(</ cfif >))"
        ]
    );
    // A closing tag inside an island never pairs with an opening tag outside
    // in the tree; the file-wide CF walk pairs them, so both are bare tags
    // and neither is a recovery (the `<cfif>` is not an unclosed region to
    // the end of its list).
    assert_eq!(
        shapes("<cfif a>\n<cfquery name=\"q\">\nselect 1 </cfif>\n</cfquery>"),
        [
            "cf-tag(< cfif a >)",
            "tag-body(cf-tag(< cfquery key-value(name = string(\" q \")) >) island(\nselect 1  cf-tag(</ cfif >) \n) cf-tag(</ cfquery >))"
        ]
    );
    // A partial and its mirror: closing tags first, opening tags last. No CF
    // walk pairs them, so both stay recoveries.
    assert_eq!(
        shapes("</cfif>\n</div>\n<p>x</p>\n<cfif y>\n<div>"),
        [
            "recovered(cf-tag(</ cfif >))",
            "html-tag(</ div >)",
            "tag-body(html-tag(< p >) x html-tag(</ p >))",
            "recovered(cf-tag(< cfif y >) html-tag(< div >))"
        ]
    );
    // The body of a closing tag that never pairs is walked again one level
    // out, so an enclosing pair still forms around it. The `</cfoutput>` is
    // stray to the file-wide CF walk too: the `<div>` body is the region.
    assert_eq!(
        shapes("<div>\n</cfoutput>\n<b>x</b>\n</div>"),
        ["recovered(tag-body(html-tag(< div >) cf-tag(</ cfoutput >) tag-body(html-tag(< b >) x html-tag(</ b >)) html-tag(</ div >)))"]
    );
    // A file that pairs completely pairs as before: the innermost pending
    // body is the only one an opening tag is compared with.
    assert_eq!(
        shapes("<x><y><x></y></x>"),
        ["tag-body(html-tag(< x >) tag-body(html-tag(< y >) html-tag(< x >) html-tag(</ y >)) html-tag(</ x >))"]
    );
    // A run of closing tags with no opening tags (Lucee's debug templates
    // close everything a page may have left open): each is released once, so
    // the walk stays linear.
    let run: String = (0..80).map(|i| format!("</t{i}>")).collect();
    let src = format!("<cfif a>{run}</cfif>");
    let tree = parse(&src, Mode::Tags);
    assert_eq!(root_shapes(&tree).len(), 1);
    assert!(root_shapes(&tree)[0].starts_with("tag-body(cf-tag(< cfif a >) html-tag(</ t0 >)"));
    assert_eq!(
        root_shapes(&parse("x = 1;\n```\n</b>\n```\n", Mode::Script)),
        [
            "statement(assignment(x = 1) ;)",
            "tag-island(``` html-tag(</ b >) ```)"
        ]
    );
}

/// The file-wide CF walk behind the recoveries releases a closer as the
/// pairing walk does: the openers it collected are walked again only when
/// one has the name of the closer below, and otherwise stay unpaired.
#[test]
fn mismatched_closers_follow_the_file_wide_walk() {
    let shapes = |src: &str| root_shapes(&parse(src, Mode::Tags));
    // Nothing pairs: the first `<cfif>` is unclosed to the end of the list.
    assert_eq!(
        shapes("<cfif a><cfif b></cfoutput></cfoutput>"),
        ["recovered(cf-tag(< cfif a >) cf-tag(< cfif b >) cf-tag(</ cfoutput >) cf-tag(</ cfoutput >))"]
    );
    // Walked again, the `<cfoutput>` in the island pairs past the stray
    // `</cfif>` with the `</cfoutput>` after it: only `</cfif>` is a region.
    let roots = shapes("<style><cfoutput></style></cfif></cfoutput>");
    assert!(roots[0].starts_with("tag-body(html-tag(< style >) island("));
    assert_eq!(
        roots[1..],
        ["recovered(cf-tag(</ cfif >))", "cf-tag(</ cfoutput >)"]
    );
    // Stray closers outside any body: a region each, in place.
    assert_eq!(
        shapes("</cfoutput></cfoutput><p>x</p>"),
        [
            "recovered(cf-tag(</ cfoutput >))",
            "recovered(cf-tag(</ cfoutput >))",
            "tag-body(html-tag(< p >) x html-tag(</ p >))"
        ]
    );
}

/// CF tags pair first, apart from HTML tags: the engines process CF
/// tags and HTML is text to them, so a CF body survives an HTML tag that
/// crosses it and the crossing HTML tags stay bare. None is a recovery.
#[test]
fn cf_tags_pair_before_html_tags() {
    let shapes = |src: &str| {
        let tree = parse(src, Mode::Tags);
        assert!(tree.recoveries.is_empty(), "{src}");
        root_shapes(&tree)
    };
    // TestBox's runner: `<html>` opened before `<cfoutput>`, closed inside it.
    // `<head>` pairs at the root; `<body>` inside the CF body.
    assert_eq!(
        shapes("<html><head></head><cfoutput><body>x</body></html></cfoutput>"),
        [
            "html-tag(< html >)",
            "tag-body(html-tag(< head >) html-tag(</ head >))",
            "tag-body(cf-tag(< cfoutput >) tag-body(html-tag(< body >) x html-tag(</ body >)) html-tag(</ html >) cf-tag(</ cfoutput >))"
        ]
    );
    // A page break inside a `<cfif>`: the outer tables pair around it.
    assert_eq!(
        shapes("<table><cfif x></table><table></cfif></table>"),
        ["tag-body(html-tag(< table >) tag-body(cf-tag(< cfif x >) html-tag(</ table >) html-tag(< table >) cf-tag(</ cfif >)) html-tag(</ table >))"]
    );
    // An extra `</div>` before a `</cfif>`.
    assert_eq!(
        shapes("<div><cfif a></div></cfif>"),
        [
            "html-tag(< div >)",
            "tag-body(cf-tag(< cfif a >) html-tag(</ div >) cf-tag(</ cfif >))"
        ]
    );
    // A crossing need not release a closer: one walk over both kinds would
    // pair the outer `<cf_x a>` around a `<div>` body holding a bare
    // `<cf_x b>`. The CF layer pairs the closer with the nearest opener of
    // its name, as the engines do, and the `<div>` it crosses goes bare.
    assert_eq!(
        shapes("<cf_x a><div><cf_x b></div></cf_x>"),
        [
            "cf-tag(< cf_x a >)",
            "html-tag(< div >)",
            "tag-body(cf-tag(< cf_x b >) html-tag(</ div >) cf-tag(</ cf_x >))"
        ]
    );
}

/// Recoveries follow the file-wide CF walk, not the tree: a CF pair
/// the walk makes across an element boundary is bare tags, no region; a CF
/// tag the walk cannot pair is still one.
#[test]
fn a_cf_pair_across_an_island_is_bare_tags() {
    // `<cfoutput>` opened in a `<style>` island, closed after it
    // (`tagStyleLeadingCf`).
    let tree = parse(
        "<style>\n<cfoutput>\na {color: #c#;}\n</style>\n<p>x</p>\n</cfoutput>\n",
        Mode::Tags,
    );
    assert!(tree.recoveries.is_empty());
    assert!(find(&tree, &|e| matches!(e.kind, ElementKind::Recovered(_))).is_empty());
    let roots = root_shapes(&tree);
    assert_eq!(roots.len(), 3);
    assert!(roots[0].starts_with("tag-body(html-tag(< style >) island("));
    assert_eq!(roots[2], "cf-tag(</ cfoutput >)");
    // A genuinely stray `</cfif>` next to it is still a region.
    let tree = parse(
        "<style>\n<cfoutput>\n</style>\n</cfoutput>\n</cfif>\n",
        Mode::Tags,
    );
    assert_eq!(
        root_shapes(&tree).last().unwrap(),
        "recovered(cf-tag(</ cfif >))"
    );
    assert_eq!(tree.recoveries.len(), 1);
}

#[test]
fn tag_bodies_nest_inside_islands_and_hold_islands() {
    let tree = parse(
        "<cfscript>\n  x = 1;\n</cfscript>\n<cfquery name=\"q\">\n  select * from t\n  <cfif a>where x = 1</cfif>\n</cfquery>\n<script type=\"module\">\n  let i = 1 < 2;\n</script>\n<style>a{}</style>\n",
        Mode::Tags,
    );
    let roots = root_shapes(&tree);
    assert_eq!(roots.len(), 4);
    // `<cfscript>` body: statements.
    assert_eq!(
        roots[0],
        "tag-body(cf-tag(< cfscript >) statement(assignment(x = 1) ;) cf-tag(</ cfscript >))"
    );
    // `<cfquery>` body: the SQL island, which holds a `<cfif>` body.
    let query = tree.root.children[2].as_element().unwrap();
    assert_eq!(tree.tag_name(query), Some("cfquery"));
    let island = significant(query.body())[0].as_element().unwrap();
    assert!(matches!(island.kind, ElementKind::Island(_)));
    assert!(island
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::TagBody { cf: true })));
    // `<script>` / `<style>`: html bodies holding the island; purity and
    // `script_type` are unaffected.
    for (i, lang) in [(4, Lang::Js), (6, Lang::Css)] {
        let body = tree.root.children[i].as_element().unwrap();
        assert_eq!(body.kind, ElementKind::TagBody { cf: false });
        let isl = body.body()[body.body().len() - 1].as_element().unwrap();
        assert_eq!(island_of(isl).lang, lang);
        assert!(isl.is_pure_island());
    }
    assert_eq!(
        island_of(islands(&tree)[1]).script_type.as_deref(),
        Some("module")
    );

    // Script tag island: tag-mode nodes pair inside the fence.
    let tree = parse("x = 1;\n```\n<cfif a><b>hi</b></cfif>\n```\n", Mode::Script);
    let fence = find(&tree, &|e| e.kind == ElementKind::TagIsland)[0];
    assert_eq!(
        significant(&fence.children)
            .into_iter()
            .map(|n| shape(&tree, n.as_element().unwrap()))
            .collect::<Vec<_>>(),
        ["tag-body(cf-tag(< cfif a >) tag-body(html-tag(< b >) hi html-tag(</ b >)) cf-tag(</ cfif >))"]
    );
}

#[test]
fn tag_bodies_in_inspect_output() {
    let tree = parse("<cfif a><b>x</b></cfif>", Mode::Tags);
    let json = cfparse::json::to_json(&tree, Default::default());
    let body = &json["root"]["children"][0];
    assert_eq!(body["kind"], "tag-body");
    assert_eq!(body["cf"], true);
    assert_eq!(body["name"], "cfif");
    assert!(body.get("open").is_none());
    assert_eq!(body["children"][0]["cf_kind"], "expression");
    let dump = cfparse::debug::format_tree(&tree, Default::default());
    assert!(dump
        .starts_with("root tags\n  tag-body cf name=cfif\n    cf-tag open expression name=cfif"));
    assert!(dump.contains("\n    tag-body html name=b\n"));
}

// ---------------------------------------------------------------------------
// Comment handback
// ---------------------------------------------------------------------------

/// Kinds of the significant nodes of `nodes` (`kw.if`, `line-comment`, …).
fn kinds(nodes: &[Node]) -> Vec<String> {
    significant(nodes)
        .into_iter()
        .map(|n| match n {
            Node::Token(t) => t.kind.name(),
            Node::Element(e) => e.kind.name().to_string(),
        })
        .collect()
}

/// Elements that end at their content, whose last child is a comment.
fn comment_tails(tree: &Tree) -> Vec<String> {
    find(tree, &|e| {
        matches!(
            e.kind,
            ElementKind::Statement(_)
                | ElementKind::If
                | ElementKind::ElseIf
                | ElementKind::Else
                | ElementKind::For
                | ElementKind::While
                | ElementKind::DoWhile
                | ElementKind::Switch
                | ElementKind::Case
                | ElementKind::Try
                | ElementKind::Catch
                | ElementKind::Finally
        ) && e
            .children
            .iter()
            .rev()
            .find(|n| !is_ws(n))
            .is_some_and(|n| matches!(n, Node::Element(c) if c.kind.is_comment()))
    })
    .into_iter()
    .map(|e| e.kind.name().to_string())
    .collect()
}

#[test]
fn function_modifiers_come_in_any_order() {
    // Lucee's reading (6.2, probed): one access word and each storage word
    // at most once, in any order; one return type, before, between or
    // after them; a second access word or a repeated storage word is the
    // return type.
    const M: &str = "storage.modifier";
    const T: &str = "storage.type";
    for (src, before_function) in [
        ("static private function f() {}", vec![M, M]),
        ("private static function f() {}", vec![M, M]),
        ("static private struct function f() {}", vec![M, M, T]),
        (
            "PUBLIC FINAL STATIC numeric FUNCTION f() {}",
            vec![M, M, M, T],
        ),
        ("string private function f() {}", vec![T, M]),
        ("boolean public function f() {}", vec![T, M]),
        ("static string private function f() {}", vec![M, T, M]),
        ("private static static function f() {}", vec![M, M, T]),
        ("package remote function f() {}", vec![M, T]),
    ] {
        let tree = parse(src, Mode::Script);
        assert_eq!(
            kinds(&tree.root.children),
            ["statement"],
            "{src}: one function statement"
        );
        let decls = find(&tree, &|e| e.kind == ElementKind::FunctionDecl);
        assert_eq!(decls.len(), 1, "{src}");
        let expected: Vec<&str> = before_function
            .into_iter()
            .chain(["kw.function", "ident.function-name"])
            .collect();
        let kinds = kinds(&decls[0].children);
        assert_eq!(&kinds[..expected.len()], &expected[..], "{src}");
    }
}

#[test]
fn a_comment_after_a_chain_is_a_sibling() {
    let tree = parse("if (a) {\n}\n// unrelated\nx = 1;", Mode::Script);
    assert_eq!(
        kinds(&tree.root.children),
        ["statement", "line-comment", "statement"]
    );
    let ifs = find(&tree, &|e| e.kind == ElementKind::If);
    assert_eq!(kinds(&ifs[0].children), ["kw.if", "group", "block"]);
    // The if's trailing newline went out with the comment.
    assert!(!is_ws(ifs[0].children.last().unwrap()));

    // try chain, same-line and own-line.
    let tree = parse(
        "try {} catch (any e) {} // same\n// own\nx = 1;",
        Mode::Script,
    );
    assert_eq!(
        kinds(&tree.root.children),
        ["statement", "line-comment", "line-comment", "statement"]
    );
    assert!(comment_tails(&tree).is_empty());
}

#[test]
fn a_comment_between_a_body_and_the_next_clause_stays_in_the_chain() {
    let tree = parse("if (a) {} // c\nelse {}\n// own\nz = 1;", Mode::Script);
    let ifs = find(&tree, &|e| e.kind == ElementKind::If);
    assert_eq!(
        kinds(&ifs[0].children),
        ["kw.if", "group", "block", "line-comment", "else"]
    );
    assert_eq!(
        kinds(&tree.root.children),
        ["statement", "line-comment", "statement"]
    );
}

#[test]
fn a_comment_at_the_tail_of_a_case_moves_to_the_switch_block() {
    for (src, same_line) in [
        (
            "switch (x) {\n  case 1:\n    a();\n    // about two\n  case 2:\n    b();\n}",
            false,
        ),
        (
            "switch (x) {\n  case 1: a(); // same\n  case 2:\n    b();\n}",
            true,
        ),
    ] {
        let tree = parse(src, Mode::Script);
        let block = find(&tree, &|e| e.kind == ElementKind::Block(BlockKind::Plain))[0];
        assert_eq!(
            kinds(&block.children),
            ["case", "line-comment", "case"],
            "{src:?}"
        );
        let cases = find(&tree, &|e| e.kind == ElementKind::Case);
        assert_eq!(
            kinds(&cases[0].children),
            ["kw.case", "lit.number", "punct.colon", "statement"],
            "{src:?}"
        );
        // "Same line" is: no `Newline` token between the case and the comment.
        let at = block
            .children
            .iter()
            .position(|n| matches!(n, Node::Element(e) if e.kind == ElementKind::LineComment))
            .unwrap();
        let newline_before = block.children[..at]
            .iter()
            .rev()
            .take_while(|n| !matches!(n, Node::Element(e) if e.kind == ElementKind::Case))
            .any(|n| matches!(n, Node::Token(t) if t.kind == TokenKind::Newline));
        assert_eq!(!newline_before, same_line, "{src:?}");
        assert!(comment_tails(&tree).is_empty(), "{src:?}");
    }
}

#[test]
fn a_semicolonless_statement_hands_back_its_comment() {
    let with = parse("x = 1; // c\ny = 2", Mode::Script);
    let without = parse("x = 1 // c\ny = 2", Mode::Script);
    for tree in [&with, &without] {
        assert_eq!(
            kinds(&tree.root.children),
            ["statement", "line-comment", "statement"]
        );
        let stmts = statements(&tree.root);
        assert_eq!(kinds(&stmts[0].children), ["assignment"]);
        // Same line: whitespace, not a newline, between statement and comment.
        let after = &tree.root.children[1];
        assert!(matches!(after, Node::Token(t) if t.kind == TokenKind::Whitespace));
    }
    // An unbraced body: the comment leaves the inner statement, the `if` and
    // the outer statement.
    let tree = parse("if (x) a // c\nb = 1;", Mode::Script);
    assert_eq!(
        kinds(&tree.root.children),
        ["statement", "line-comment", "statement"]
    );
    // A block comment after a semicolon-less statement moves too.
    let tree = parse("doThis() /* note */\nx = 1;", Mode::Script);
    assert_eq!(
        kinds(&tree.root.children),
        ["statement", "block-comment", "statement"]
    );
    assert!(comment_tails(&tree).is_empty());
}

#[test]
fn a_statement_with_a_terminator_keeps_its_content() {
    let tree = parse("x = 1; // trailing\n/* lead */ y = 2;", Mode::Script);
    assert_eq!(
        kinds(&tree.root.children),
        ["statement", "line-comment", "block-comment", "statement"]
    );
    // A comment before the `;` is not after the statement: it stays.
    let tree = parse("x = 1 /* c */;\ny = 2;", Mode::Script);
    let stmts = statements(&tree.root);
    assert_eq!(kinds(&stmts[0].children), ["assignment", "block-comment"]);
    assert_eq!(tree.text(stmts[0].terminator().unwrap()), ";");
    // A comment in the middle of a statement stays inside it.
    let tree = parse("x = 1 + // c\n  2;", Mode::Script);
    assert_eq!(kinds(&tree.root.children), ["statement"]);
}

#[test]
fn a_comment_only_statement_disappears() {
    // A `<cfscript>` body does not wrap a trailing comment in its own
    // statement.
    let tree = parse("<cfscript>\n  x = 1; // c\n</cfscript>\n", Mode::Tags);
    let body = tree.root.children[0].as_element().unwrap();
    assert_eq!(kinds(body.body()), ["statement", "line-comment"]);
}

// ---------------------------------------------------------------------------
// Item comment attachment
// ---------------------------------------------------------------------------

/// `leading | children | trailing` of an item as significant-node texts.
fn item_parts(tree: &Tree, item: &cfparse::Item) -> (Vec<String>, Vec<String>, Vec<String>) {
    let texts = |nodes: &[Node]| -> Vec<String> {
        significant(nodes)
            .into_iter()
            .map(|n| text(tree, n).to_string())
            .collect()
    };
    (
        texts(&item.leading),
        texts(&item.children),
        texts(&item.trailing),
    )
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn line_comments_attach_to_array_items() {
    let tree = parse(
        "x = [\n  1, // after one\n  // before two\n  2 /* b */,\n  3 // after three\n];",
        Mode::Script,
    );
    let array = find(&tree, &|e| e.kind == ElementKind::Array)[0];
    assert_eq!(array.items.len(), 3);
    // Same line after the separator: trailing of the item before it.
    assert_eq!(
        item_parts(&tree, &array.items[0]),
        (vec![], strs(&["1"]), strs(&["// after one"]))
    );
    // Own line before the item: leading. The block comment stays in place.
    assert_eq!(
        item_parts(&tree, &array.items[1]),
        (strs(&["// before two"]), strs(&["2", "/* b */"]), vec![])
    );
    // After the last item, before the closing bracket: trailing.
    assert_eq!(
        item_parts(&tree, &array.items[2]),
        (vec![], strs(&["3"]), strs(&["// after three"]))
    );
    // The runs keep their whitespace: the same-line comment has no newline
    // before it, and the leading run ends with the item's indentation.
    let trailing = &array.items[0].trailing;
    assert!(matches!(&trailing[0], Node::Token(t) if t.kind == TokenKind::Whitespace));
    assert!(matches!(trailing.last().unwrap(), Node::Token(t) if t.kind == TokenKind::Newline));
    assert!(matches!(
        array.items[1].leading.last().unwrap(),
        Node::Token(t) if t.kind == TokenKind::Whitespace
    ));
    // Accessors.
    let comment_texts = |it: &mut dyn Iterator<Item = &Element>| -> Vec<String> {
        it.map(|e| tree.slice(e.span.clone()).to_string()).collect()
    };
    assert_eq!(
        comment_texts(&mut array.items[0].trailing_comments()),
        ["// after one"]
    );
    assert_eq!(
        comment_texts(&mut array.items[1].leading_comments()),
        ["// before two"]
    );
    assert_eq!(
        array.items[1]
            .significant()
            .map(|n| text(&tree, n))
            .collect::<Vec<_>>(),
        ["2", "/* b */"]
    );
}

#[test]
fn a_comment_before_a_comma_on_the_next_line_is_trailing() {
    // A line comment that ends an item moves to its `trailing`: printing it
    // in place would emit `3 // c,`.
    let tree = parse("x = [\n  3 // c\n  , 4\n];", Mode::Script);
    let array = find(&tree, &|e| e.kind == ElementKind::Array)[0];
    assert_eq!(
        item_parts(&tree, &array.items[0]),
        (vec![], strs(&["3"]), strs(&["// c"]))
    );
    assert_eq!(tree.text(array.items[0].separator.as_ref().unwrap()), ",");
    assert_eq!(
        item_parts(&tree, &array.items[1]),
        (vec![], strs(&["4"]), vec![])
    );
}

#[test]
fn line_comments_attach_to_call_arguments_and_parameters() {
    let tree = parse("foo(a, // c1\n  b);", Mode::Script);
    let call = find(&tree, &|e| matches!(e.kind, ElementKind::Call))[0];
    assert_eq!(
        item_parts(&tree, &call.items[0]),
        (vec![], strs(&["a"]), strs(&["// c1"]))
    );
    assert_eq!(
        item_parts(&tree, &call.items[1]),
        (vec![], strs(&["b"]), vec![])
    );

    let tree = parse(
        "function f(\n  required string a, // first\n  // about b\n  b = 1\n) {}",
        Mode::Script,
    );
    let params = find(&tree, &|e| e.kind == ElementKind::Parameters)[0];
    assert_eq!(params.items.len(), 2);
    assert_eq!(
        item_parts(&tree, &params.items[0]),
        (
            vec![],
            strs(&["required", "string", "a"]),
            strs(&["// first"])
        )
    );
    assert_eq!(
        item_parts(&tree, &params.items[1]),
        (strs(&["// about b"]), strs(&["b = 1"]), vec![])
    );

    // Nested delimited elements attach too.
    let tree = parse("foo({\n  a: 1, // one\n  b: 2\n});", Mode::Script);
    let st = find(&tree, &|e| matches!(e.kind, ElementKind::Struct { .. }))[0];
    assert_eq!(
        item_parts(&tree, &st.items[0]),
        (vec![], strs(&["a: 1"]), strs(&["// one"]))
    );
}

#[test]
fn whitespace_and_block_comments_do_not_move() {
    let tree = parse("foo(\n  a,\n  /* x */ b /* y */,\n  c\n);", Mode::Script);
    let call = find(&tree, &|e| matches!(e.kind, ElementKind::Call))[0];
    for item in &call.items {
        assert!(item.leading.is_empty() && item.trailing.is_empty());
    }
    assert_eq!(
        item_parts(&tree, &call.items[1]).1,
        strs(&["/* x */", "b", "/* y */"])
    );
}

#[test]
fn dangling_comments_after_a_trailing_comma() {
    // The same-line comment goes to the item before the comma; the empty
    // item left behind is dropped, like `[1,]`'s.
    let tree = parse("x = [1, // c\n];", Mode::Script);
    let array = find(&tree, &|e| e.kind == ElementKind::Array)[0];
    assert_eq!(array.items.len(), 1);
    assert_eq!(
        item_parts(&tree, &array.items[0]),
        (vec![], strs(&["1"]), strs(&["// c"]))
    );
    // An own-line comment after the trailing comma keeps its item: no
    // significant children, a trailing comment (printers must not drop it).
    let tree = parse("x = [1, // a\n  // b\n];", Mode::Script);
    let array = find(&tree, &|e| e.kind == ElementKind::Array)[0];
    assert_eq!(array.items.len(), 2);
    assert_eq!(array.items[1].significant().count(), 0);
    assert_eq!(
        item_parts(&tree, &array.items[1]),
        (vec![], vec![], strs(&["// b"]))
    );
    // A whitespace-only trailing item stays as it is.
    let tree = parse("x = [1, \n];", Mode::Script);
    let array = find(&tree, &|e| e.kind == ElementKind::Array)[0];
    assert_eq!(array.items.len(), 2);
    assert!(array.items[1].leading.is_empty() && array.items[1].trailing.is_empty());
}

#[test]
fn item_comments_in_inspect_output() {
    let tree = parse("foo(a, // c1\n  // own\n  b);", Mode::Script);
    let json = cfparse::json::to_json(&tree, Default::default());
    let items = &json["root"]["children"][0]["children"][0]["children"][1]["items"];
    assert_eq!(items[0]["trailing"][1]["kind"], "line-comment");
    assert_eq!(items[0]["sep"], ",");
    assert!(items[0].get("leading").is_none());
    assert_eq!(items[1]["leading"][1]["kind"], "line-comment");
    assert!(items[1].get("trailing").is_none());
    let dump = cfparse::debug::format_tree(&tree, Default::default());
    assert!(
        dump.contains("        item sep=\",\"\n          ident.variable \"a\"\n          trailing:\n            ws \" \"\n            line-comment open=\"//\"\n"),
        "{dump}"
    );
    assert!(
        dump.contains("        item\n          leading:\n"),
        "{dump}"
    );
}

// ---------------------------------------------------------------------------
// HTML attribute values
// ---------------------------------------------------------------------------

#[test]
fn html_attribute_values_are_strings() {
    let tree = parse(
        "<cfoutput><div class=\"a #b#\" id='x' data-x=un title=\"\"></div></cfoutput>",
        Mode::Tags,
    );
    let div = find(&tree, &|e| e.kind == ElementKind::HtmlTag(TagShape::Open))[0];
    let strings: Vec<&Element> = attr_nodes(div)
        .into_iter()
        .filter_map(Node::as_element)
        .collect();
    assert_eq!(strings.len(), 3, "{}", shape(&tree, div));
    let quotes: Vec<_> = strings
        .iter()
        .map(|s| match s.kind {
            ElementKind::String { quote, in_tag } => {
                assert!(in_tag);
                (
                    quote,
                    tree.text(s.open.as_ref().unwrap()).to_string(),
                    tree.text(s.close.as_ref().unwrap()).to_string(),
                )
            }
            ref k => panic!("unexpected {k:?}"),
        })
        .collect();
    assert_eq!(
        quotes,
        [
            (cfparse::Quote::Double, "\"".into(), "\"".into()),
            (cfparse::Quote::Single, "'".into(), "'".into()),
            (cfparse::Quote::Double, "\"".into(), "\"".into()),
        ]
    );
    // The value's text keeps its token kinds; `#b#` is a template
    // expression because the value sits inside `<cfoutput>`.
    let kinds_of = |s: &Element| -> Vec<(String, String)> {
        s.children
            .iter()
            .map(|n| match n {
                Node::Token(t) => (t.kind.name(), tree.text(t).to_string()),
                Node::Element(e) => (e.kind.name().to_string(), text(&tree, n).to_string()),
            })
            .collect()
    };
    let pair = |k: &str, t: &str| (k.to_string(), t.to_string());
    assert_eq!(
        kinds_of(strings[0]),
        [
            pair("lit.string", "a"),
            pair("ws", " "),
            pair("template-expression", "#b#")
        ]
    );
    // Outside `<cfoutput>` `#` is not special in an HTML attribute value,
    // so the text stays `lit.string`.
    let plain = parse("<div class=\"a #b#\">", Mode::Tags);
    let s = find(&plain, &|e| matches!(e.kind, ElementKind::String { .. }))[0];
    assert_eq!(
        s.children
            .iter()
            .map(|n| (tok(n).kind.name(), text(&plain, n)))
            .collect::<Vec<_>>(),
        [
            ("lit.string".to_string(), "a"),
            ("ws".to_string(), " "),
            ("lit.string".to_string(), "#b#")
        ]
    );
    assert!(strings[2].children.is_empty());
    // An unquoted value has no delimiters and stays a bare token.
    assert!(attr_nodes(div).into_iter().any(|n| matches!(
        n,
        Node::Token(t) if t.kind == TokenKind::Literal(Literal::Unquoted) && tree.text(t) == "un"
    )));
}

#[test]
fn attribute_islands_open_inside_the_attribute_string() {
    let tree = parse(
        "<button onclick=\"go(1)\" style='color: red'>x</button>",
        Mode::Auto,
    );
    let tag = find(&tree, &|e| e.kind == ElementKind::HtmlTag(TagShape::Open))[0];
    let sites: Vec<(cfparse::Quote, IslandSite)> = attr_nodes(tag)
        .into_iter()
        .filter_map(Node::as_element)
        .map(|s| {
            let ElementKind::String { quote, .. } = s.kind else {
                panic!("{:?}", s.kind)
            };
            assert_eq!(s.children.len(), 1);
            let isl = s.children[0].as_element().unwrap();
            assert!(isl.is_pure_island());
            (quote, island_of(isl).site)
        })
        .collect();
    assert_eq!(
        sites,
        [
            (cfparse::Quote::Double, IslandSite::EventAttribute),
            (cfparse::Quote::Single, IslandSite::StyleAttribute)
        ]
    );
    // CFScript strings never hold an island, tags-mode CFML attribute
    // strings neither.
    let tree = parse(
        "<cfquery name=\"q\">select 1</cfquery><cfset q = 'SELECT * FROM t'>",
        Mode::Tags,
    );
    for s in find(&tree, &|e| matches!(e.kind, ElementKind::String { .. })) {
        assert!(s.children.iter().all(|n| n.as_token().is_some()));
    }
}

// ---------------------------------------------------------------------------
// The nesting bound
// ---------------------------------------------------------------------------

/// `body` wrapped in `n` copies of `open` … `close`, one line.
fn nested(open: &str, close: &str, body: &str, n: usize) -> String {
    format!("{}{body}{}\n", open.repeat(n), close.repeat(n))
}

/// The longest chain of elements from the root, the root counted.
fn element_depth(el: &Element) -> usize {
    1 + el
        .nodes()
        .filter_map(Node::as_element)
        .map(element_depth)
        .max()
        .unwrap_or(0)
}

/// Past `MAX_DEPTH` (100) the scanner does what the script parser's
/// `too_deep` does: the rest of the source up to its
/// trailing whitespace is unmatched text inside the deepest element, every
/// enclosing element closes without a closing tag, and the entry point takes
/// the trailing whitespace. A comment is the one exception (it stops
/// *nesting* instead, keeping the closers the source has): the printer
/// writes `--->` for a comment the source never closed, so a cut inside one
/// would grow the file on every run.
#[test]
fn nesting_past_max_depth_becomes_unmatched_text() {
    for (open, close) in [
        ("<cfoutput>", "</cfoutput>"),
        ("<cffunction name=\"f\">", "</cffunction>"),
        ("<!--- ", " --->"),
    ] {
        let src = nested(open, close, "x", 101);
        // `parse` asserts the tree tiles its source.
        let tree = parse(&src, Mode::Tags);
        assert_eq!(tree.mode(), Mode::Tags, "{open}");
        // `MAX_DEPTH` elements plus the root, at most: the tag cases stay
        // flat (the open tags are siblings and nothing pairs them), the
        // comment case is the root plus 100 comments.
        assert!(
            element_depth(&tree.root) <= 101,
            "{open}: {} deep",
            element_depth(&tree.root)
        );
        // The last line's whitespace is the root's, not the deepest
        // element's, so a format does not grow a blank line.
        let once = cfformat::format_source(&src, Mode::Tags, &Default::default());
        let twice = cfformat::format_source(&once, Mode::Tags, &Default::default());
        assert_eq!(once, twice, "{open}: not idempotent");
    }
}

/// Three times the bound is no different: the tree stops at the bound and
/// the printer's own bound (`cfformat::print::MAX_DEPTH`, 260) is never
/// reached from tag nesting.
#[test]
fn nesting_far_past_max_depth_is_the_same() {
    let src = nested("<cfoutput>", "</cfoutput>", "x", 300);
    let tree = parse(&src, Mode::Tags);
    assert!(
        element_depth(&tree.root) <= 103,
        "{}",
        element_depth(&tree.root)
    );
    let once = cfformat::format_source(&src, Mode::Tags, &Default::default());
    assert_eq!(
        once,
        cfformat::format_source(&once, Mode::Tags, &Default::default())
    );
}

/// `cfparse::MAX_DEPTH` (crate-private): the one nesting budget of both
/// front ends and the boundary scanners.
const MAX_DEPTH: usize = 100;

/// `cfparse::MAX_TREE_DEPTH` (crate-private): the deepest tree the front
/// ends and the expression post-pass build. A level of the budget adds at
/// most four elements (`switch (x) { case 1:` is a statement, a `switch`, a
/// block and a case; `a.b(` a call expression, a chain, a call and an
/// argument), plus the root and the leaves below the cut. Tag *pairing* is
/// bounded on its own (`postpass::tags::MAX_TAG_DEPTH`).
const MAX_TREE_DEPTH: usize = 4 * MAX_DEPTH + 10;

/// A fragment one front end hands the other starts at its caller's depth: a
/// `<cfscript>` 99 blocks deep whose ```` ``` ```` tag island holds another
/// `<cfscript>` 99 blocks deep would otherwise have had two budgets of 100.
#[test]
fn a_fragment_counts_against_its_callers_depth() {
    let script = |n| format!("<cfscript>{}x();</cfscript>", "if(x){".repeat(n));
    let src = format!(
        "<cfscript>{}```{}```</cfscript>\n",
        "if(x){".repeat(99),
        script(99)
    );
    let tree = parse(&src, Mode::Tags);
    let depth = element_depth(&tree.root);
    assert!(depth <= MAX_TREE_DEPTH, "{depth} deep");
    let src = format!("{}```{}```\n", "if(x){".repeat(99), script(99));
    let tree = parse(&src, Mode::Script);
    let depth = element_depth(&tree.root);
    assert!(depth <= MAX_TREE_DEPTH, "{depth} deep");
}

/// Parse `src` on a thread with the stack of `common::SMALL_STACK` (the 2 MB
/// a default thread has, in release), so that an overflow fails here whatever
/// `RUST_MIN_STACK` says.
fn depth_on_a_small_stack(src: String, mode: Mode) -> usize {
    std::thread::Builder::new()
        .name("depth on a small stack".into())
        .stack_size(common::SMALL_STACK)
        .spawn(move || element_depth(&parse(&src, mode).root))
        .unwrap()
        .join()
        .unwrap()
}

/// An unbraced body nests like a block and a label like a body: each chain
/// is cut at `MAX_DEPTH`, its rest unmatched text.
#[test]
fn unbraced_bodies_and_labels_nest_to_the_bound() {
    for src in [
        format!("{}x();", "if(x) ".repeat(20_000)),
        format!("{}x();", "if(x) x(); else ".repeat(20_000)),
        format!("{}x();", "while(x) ".repeat(20_000)),
        format!("{}x();", "for(;;) ".repeat(20_000)),
        format!("{}x();", "do ".repeat(20_000)),
        format!("{}x();", "a: ".repeat(200_000)),
        "switch(x){".repeat(20_000),
    ] {
        let depth = depth_on_a_small_stack(src.clone(), Mode::Script);
        assert!(depth <= MAX_TREE_DEPTH, "{}…: {depth} deep", &src[..20]);
    }
}

/// The expression post-pass neither recurses nor nests without bound:
/// prefix chains are a loop, assignment chains a fold, and a run that would
/// nest past `MAX_TREE_DEPTH` stays flat. A chain that fits still nests:
/// 150 `!` are 150 unary elements.
#[test]
fn expression_chains_stay_within_the_tree_depth() {
    for src in [
        format!("x={}x;", "!".repeat(20_000)),
        format!("x={}x;", "-".repeat(20_000)),
        format!("x={}x;", "not ".repeat(20_000)),
        format!("x={}x;", "++".repeat(20_000)),
        format!("{}0;", "x=".repeat(20_000)),
        format!("x={}c;", "a+=".repeat(20_000)),
        format!("x={}c;", "a?b:".repeat(20_000)),
        format!("x=a{};", "()".repeat(20_000)),
        // 100 groups of 99 `!` each: a bound per run would still nest
        // 10,000 deep.
        format!("x={}x;", format!("{}(", "!".repeat(99)).repeat(100)),
    ] {
        let depth = depth_on_a_small_stack(src.clone(), Mode::Script);
        assert!(depth <= MAX_TREE_DEPTH, "{}…: {depth} deep", &src[..20]);
    }
    let tree = parse(&format!("x={}x;", "!".repeat(150)), Mode::Script);
    let unary = find(&tree, &|e| matches!(e.kind, ElementKind::Unary { .. }));
    assert_eq!(unary.len(), 150);
    assert!(element_depth(&tree.root) > 150);
}

/// The boundary scanners (`scan.rs`) count against `MAX_DEPTH` and scan
/// flat past it: brackets inside a `#…#`, and the string ↔ `#…#` cycle in
/// tag text, an attribute value and a `<cfscript>` body.
#[test]
fn boundary_scanners_stay_within_the_bound() {
    let n = 400_000;
    for src in [
        format!("<cfoutput>#{}x{}#</cfoutput>", "(".repeat(n), ")".repeat(n)),
        format!("<cfoutput>#{}x", "f(\"#".repeat(20_000)),
        format!("<cfoutput><a onclick=\"#{}x\">", "f('#".repeat(20_000)),
        format!("<cfscript>x=\"#{}x", "f(\"#".repeat(20_000)),
    ] {
        let depth = depth_on_a_small_stack(src.clone(), Mode::Tags);
        assert!(depth <= MAX_TREE_DEPTH, "{}…: {depth} deep", &src[..24]);
    }
}

/// A CF tag an island body opens counts against `MAX_DEPTH` like a content
/// run: `<cfquery>` in a `<cfquery>` body, in a `<style>` body, in an event
/// attribute.
#[test]
fn island_bodies_count_the_tags_they_open() {
    for src in [
        "<cfquery>".repeat(20_000),
        format!("<style>{}", "<cfquery>".repeat(20_000)),
        format!(
            "<cfquery>{}",
            "<cfoutput><a onclick=\"<cfquery>".repeat(20_000)
        ),
    ] {
        let depth = depth_on_a_small_stack(src.clone(), Mode::Tags);
        assert!(depth <= MAX_TREE_DEPTH, "{}…: {depth} deep", &src[..20]);
    }
}

/// Every input shape that once overflowed the stack
/// (`common::generators`, the fixed cases of `tests/fuzz.rs`) parses and
/// formats on a `common::SMALL_STACK` thread (2 MB in release), into a tree no
/// deeper than `MAX_TREE_DEPTH` — except paired tags, which
/// `postpass::tags::MAX_TAG_DEPTH` bounds on its own.
#[test]
fn every_generator_parses_and_formats_on_a_small_stack() {
    for g in common::generators() {
        for &n in g.sizes {
            let (src, mode) = ((g.source)(n), g.mode);
            let (depth, formatted) = std::thread::Builder::new()
                .name(format!("{} × {n}", g.name))
                .stack_size(common::SMALL_STACK)
                .spawn(move || {
                    let depth = element_depth(&parse(&src, mode).root);
                    let formatted = cfformat::format_source(&src, mode, &Default::default());
                    (depth, !formatted.is_empty())
                })
                .unwrap()
                .join()
                .unwrap();
            let bound = if g.name == "paired cfif" {
                cfparse::postpass::tags::MAX_TAG_DEPTH + MAX_TREE_DEPTH
            } else {
                MAX_TREE_DEPTH
            };
            assert!(depth <= bound, "{} × {n}: {depth} deep", g.name);
            assert!(formatted, "{} × {n}", g.name);
            println!("{} × {n}: {depth} deep", g.name);
        }
    }
}

/// `Element::rfind_token` is the last token of `Element::tokens` the
/// predicate accepts, for every element of every fixture.
#[test]
fn rfind_token_is_the_last_of_tokens() {
    let preds: [&dyn Fn(&Token) -> bool; 3] = [
        &|_| true,
        &|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline),
        &|t| matches!(t.kind, TokenKind::Punct(_)),
    ];
    for f in common::fixtures() {
        let tree = parse_source(&f.source, f.mode);
        for e in find(&tree, &|_| true) {
            let tokens = e.tokens();
            for pred in preds {
                let key = |t: &Token| (t.span.clone(), t.kind);
                assert_eq!(
                    e.rfind_token(pred).map(key),
                    tokens.iter().rev().find(|t| pred(t)).map(key),
                    "{}: {:?}",
                    f.name,
                    e.span
                );
            }
        }
    }
}
