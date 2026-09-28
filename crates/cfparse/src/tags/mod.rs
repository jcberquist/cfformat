//! The hand-written tag-mode front end: a scanner producing the pre-post-pass
//! [`Element`] tree of a template.
//!
//! [`parse`] runs over *normalised* source (BOM stripped, `\r\n` → `\n`) and
//! returns the root element **before** the post-passes; `parse_source` runs
//! the same post-passes on it. Every script span — a `<cfscript>` body, the
//! expression of `<cfset>` / `<cfreturn>` / `<cfif>` / `<cfelseif>`, a `#…#`
//! — goes to [`script::parse_fragment`](crate::script::parse_fragment), which
//! is the scanner's only interface to script.
//!
//! It is the **only** tag front end: there is no switch and no fallback. The
//! tree is the contract: its kinds, token boundaries, spans and elements, as
//! the snapshot tests pin them.

mod cf;
mod html;
mod islands;
mod scanner;

use std::ops::Range;

use crate::tree::{Element, ElementKind, Mode, Node};

use scanner::Scanner;

/// Parse normalised tag-mode source into the pre-post-pass root element.
///
/// The root is [`ElementKind::Root`]`(`[`Mode::Tags`]`)`; `parse_source` runs
/// the post-passes (tag pairing included) over it.
pub fn parse(source: &str) -> Element {
    let mut scanner = Scanner::new(source);
    let mut children = Vec::new();
    scanner.document(&mut children);
    scanner.finish(&mut children);
    let mut root = scanner.element(ElementKind::Root(Mode::Tags), 0..source.len() as u32);
    root.children = children;
    root
}

/// Parse the tag-mode text in `range` of the normalised `source`: the
/// pre-post-pass nodes, spans absolute, to splice where a ```` ``` ```` tag
/// island in script was. `depth` is the script parser's, so the fragment
/// counts against the same budget ([`crate::MAX_DEPTH`]).
pub(crate) fn parse_fragment(source: &str, range: Range<u32>, depth: u32) -> Vec<Node> {
    let mut scanner = Scanner::fragment(source, range, depth);
    let mut children = Vec::new();
    scanner.document(&mut children);
    scanner.finish(&mut children);
    children
}
