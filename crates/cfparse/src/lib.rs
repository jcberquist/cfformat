//! CFML parse tree from the two hand-written front ends.
//!
//! Pipeline: [normalise](normalize) (BOM, `\r\n`/`\r` → `\n`) → the
//! hand-written front end for the mode ([`script`] or [`tags`]) →
//! [post-passes](postpass) (tag pairing, item comment attachment,
//! expressions) → [`Tree`].

pub mod debug;
pub mod json;
pub mod nodes;
pub mod normalize;
pub mod tree;

// The two front ends and the post-passes are not API: `script::parse` and
// `tags::parse` return a root before the post-passes (no pairing, no
// expressions, no recoveries), which only `parse_source` should hand out.
// They stay `pub` for this crate's own integration tests and benches, which
// are separate crates — `tests/script.rs` and `benches/parse.rs` run the
// script front end alone, `tests/tree.rs` reads
// `postpass::tags::MAX_TAG_DEPTH` — and hidden from the docs. Nothing
// outside the crate uses them (`cfformat` and `cfvet` go through the root).
#[doc(hidden)]
pub mod postpass;
#[doc(hidden)]
pub mod script;
#[doc(hidden)]
pub mod tags;

mod key_value;
mod scan;

/// How deeply the front ends nest, counted on one budget: the script
/// parser's statement lists, expressions, binding patterns, unbraced bodies
/// and labels, the tag scanner's content runs and the tags an island body
/// opens, a `<!--- --->` comment's nesting, and the boundary scanners of
/// [`scan`]. A fragment one front end hands the other (a `<cfscript>` body,
/// a `#…#`, a ```` ``` ```` tag island) starts at its caller's depth, so the
/// budget is the whole source's, not each fragment's.
///
/// The front ends recurse, so without a bound a pathological input —
/// minified CSS read as script nests `{` and `#` hundreds deep — overflows
/// a thread's stack, and a stack overflow is an abort no `catch_unwind`
/// sees. Past it the rest of the source is one unmatched run (`too_deep` in
/// either front end): the parse stays total and the tree stays shallow
/// enough for every pass and the printer. Real code nests a few dozen levels
/// at most.
pub(crate) const MAX_DEPTH: u32 = 100;

/// The deepest tree a parse builds, the root counted: a level of
/// [`MAX_DEPTH`] adds at most four elements (`switch (x) { case 1:` is a
/// statement, a `switch`, a block and a case; `a.b(` a call expression, a
/// chain, a call and an argument), plus room for the leaves below a cut.
/// The expression post-pass leaves a run flat rather than nest past it
/// ([`postpass::expressions`]), so every recursive walk of a tree — the
/// later passes, the printer, `Drop` — stays within it. Tag *pairing* is
/// bounded on its own ([`postpass::tags::MAX_TAG_DEPTH`]): a thousand
/// paired `<cfif>` bodies nest a thousand deep.
pub(crate) const MAX_TREE_DEPTH: usize = 4 * MAX_DEPTH as usize + 10;

pub use nodes::{
    Assignment, Binary, CallExpr, Chain, Decl, KeyValue, New, Segment, Ternary, Unary,
};
pub use normalize::Newline;
pub use tree::*;

/// Parse `src` into a tree. Script — `Mode::Script`, and
/// `Mode::Auto` when the source resolves to script — goes to the
/// hand-written script front end ([`script`]); tag mode to the hand-written
/// tag front end ([`tags`]), which hands the script inside it (a
/// `<cfscript>` body, a tag's expression, a `#…#`) back to the script one as
/// fragments. Both roots get the same post-passes.
pub fn parse_source(src: &str, mode: Mode) -> Tree {
    let norm = normalize::normalize(src);
    let root = match script::parse(&norm.text, mode) {
        Ok(root) => root,
        Err(_) => tags::parse(&norm.text),
    };
    finish_tree(root, norm)
}

/// The post-passes, over either front end's root, and the two recovery
/// passes around them.
fn finish_tree(root: Element, norm: normalize::Normalized) -> Tree {
    let mut tree = Tree {
        source: norm.text,
        root,
        bom: norm.bom,
        newline: norm.newline,
        original: norm.original,
        recoveries: Vec::new(),
    };
    postpass::tags::pair_tags(&mut tree.root, &tree.source);
    postpass::recover_tags(&mut tree.root, &tree.source);
    postpass::comments::attach_item_comments(&mut tree.root);
    postpass::expressions::build_expressions(&mut tree.root);
    postpass::collect_recoveries(&mut tree.root, &mut tree.recoveries);
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_tags_when_leading_comments_precede_a_tag() {
        for src in [
            "/**\n * license\n **/\n<cfscript>\ncomponent {}\n</cfscript>\n",
            "// output = true\n<cfcomponent output=\"true\">\n</cfcomponent>\n",
            "\u{FEFF}// a\n/* b */ <div></div>",
            "<cfset x = 1>",
        ] {
            assert_eq!(parse_source(src, Mode::Auto).mode(), Mode::Tags, "{src:?}");
        }
        for src in [
            "/** doc */\ncomponent {}\n",
            "// a\nx = 1;\n",
            "/* unterminated <b>",
        ] {
            assert_eq!(
                parse_source(src, Mode::Auto).mode(),
                Mode::Script,
                "{src:?}"
            );
        }
        assert_eq!(parse_source("// a\n<b>", Mode::Script).mode(), Mode::Script);
    }

    #[test]
    fn line_of_counts_newlines_before_the_offset() {
        let tree = parse_source("a = 1;\nb = 2;\n\nc();", Mode::Script);
        assert_eq!(tree.line_of(0), 1);
        assert_eq!(tree.line_of(6), 1); // the first `\n` is still on line 1
        assert_eq!(tree.line_of(7), 2);
        assert_eq!(tree.line_of(14), 3);
        assert_eq!(tree.line_of(15), 4);
        assert_eq!(tree.line_of(1000), 4);
    }
}
