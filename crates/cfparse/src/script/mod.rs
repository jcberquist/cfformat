//! The CFScript front end: a hand-written recursive-descent parser that emits
//! the [`Element`] tree.
//!
//! [`parse`] runs over *normalised* source (BOM stripped, `\r\n` → `\n`) and
//! returns the root element **before** the post-passes; `parse_source` runs
//! the same post-passes on it. A source that is not script — tag mode, or
//! `Auto` resolving to tags — is [`Declined`] and goes to the tag front end;
//! everything else parses. That is the **only** way it returns no tree: the
//! ```` ``` ```` tag island's sub-parse is the tag scanner's and cannot fail
//! either.
//!
//! It is the only script front end: the tag front end
//! ([`tags`](crate::tags)) parses tag mode and hands the script inside it
//! back through [`parse_fragment`].

mod comments;
mod expressions;
mod lexer;
mod parser;
mod tags;

use std::fmt;
use std::ops::Range;

use crate::scan::{tag_marker, Marker};
use crate::tree::{Element, Mode, Node};

/// A source the script parser does not parse because it is not script: tag
/// mode, requested or resolved from `Mode::Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declined;

impl fmt::Display for Declined {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not script: tag mode")
    }
}

impl std::error::Error for Declined {}

/// Parse normalised CFScript into the pre-post-pass root element.
///
/// `mode` is the *requested* mode: [`Mode::Tags`] always declines, and
/// [`Mode::Auto`] declines unless the source resolves to script: it starts
/// with a comment, `import`, or a `component` / `interface` declaration,
/// and its leading comments are not followed by a tag.
pub fn parse(source: &str, mode: Mode) -> Result<Element, Declined> {
    let resolved = resolve(source, mode)?;
    Ok(parser::Parser::new(source, 0).run(resolved))
}

/// What a script fragment in tag mode is: the tag scanner finds where it ends
/// and hands it over whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fragment {
    /// A `<cfscript>` body: a statement list.
    Statements,
    /// The inside of a `#…#` (tag text, an attribute value, `<cfquery>`).
    Expression,
    /// The expression of `<cfset>`, `<cfreturn>`, `<cfif>`, `<cfelseif>`:
    /// an optional leading `var`, then expressions.
    TagExpression,
}

/// Parse the script in `range` of the normalised `source`: the pre-post-pass
/// nodes, trivia included, spans absolute, to splice where the tag front end
/// found the script. Total — a fragment is script by definition, nothing
/// declines, and a ```` ``` ```` tag island inside it cannot fail either.
pub fn parse_fragment(source: &str, range: Range<u32>, kind: Fragment) -> Vec<Node> {
    parse_fragment_at(source, range, kind, 0)
}

/// [`parse_fragment`] from inside a front end already `depth` deep: the
/// fragment's nesting counts against the caller's budget
/// ([`MAX_DEPTH`](crate::MAX_DEPTH)), so a ```` ``` ```` island in a
/// `<cfscript>` in a ```` ``` ```` island cannot reset it.
pub(crate) fn parse_fragment_at(
    source: &str,
    range: Range<u32>,
    kind: Fragment,
    depth: u32,
) -> Vec<Node> {
    parser::Parser::new(&source[..range.end as usize], depth).fragment(range.start, kind)
}

/// Resolve the requested mode, declining anything that is not script.
fn resolve(source: &str, mode: Mode) -> Result<Mode, Declined> {
    match mode {
        Mode::Script => Ok(Mode::Script),
        Mode::Auto if auto_is_script(source) => Ok(Mode::Script),
        Mode::Tags | Mode::Auto => Err(Declined),
    }
}

/// `Mode::Auto` resolves to script when the source starts like a script file
/// (a leading comment, `import`, or a `component` / `interface` declaration)
/// unless leading comments are followed by a tag: a licence comment above
/// `<cfcomponent>` or `<cfscript>` does not make a tag file script.
fn auto_is_script(source: &str) -> bool {
    let text = source.trim_start_matches('\u{FEFF}');
    if leading_comments_precede_a_tag(text.trim_start()) {
        return false;
    }
    // The script head is looked for at the start of every line of the
    // preamble: blank lines and `<!--- --->` comments continue it, anything
    // else makes the file tags. So a licence in a tag comment above
    // `component {` is script.
    let mut rest = text;
    loop {
        let head = rest.trim_start();
        if script_head(head) {
            return true;
        }
        let Some(mut line) = skip_tag_comment(head) else {
            return false;
        };
        loop {
            let t = line.trim_start_matches([' ', '\t', '\x0c', '\r']);
            if let Some(after) = skip_tag_comment(t) {
                line = after;
                continue;
            }
            match t.strip_prefix('\n') {
                Some(next) => {
                    rest = next;
                    break;
                }
                None => return false,
            }
        }
    }
}

/// A `<!--- --->` comment (nested) or `cfformat-ignore` region at the start
/// of `text`, skipped; `None` when there is none or it never ends.
fn skip_tag_comment(text: &str) -> Option<&str> {
    if let Some(len) = tag_marker(text, Marker::Start) {
        let mut at = len;
        while at < text.len() {
            if let Some(len) = tag_marker(&text[at..], Marker::End) {
                return Some(&text[at + len..]);
            }
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
        return None;
    }
    let rest = text.strip_prefix("<!---")?;
    let mut depth = 1;
    let mut at = 0;
    while at < rest.len() {
        if rest[at..].starts_with("--->") {
            depth -= 1;
            at += 4;
            if depth == 0 {
                return Some(&rest[at..]);
            }
        } else if rest[at..].starts_with("<!---") {
            depth += 1;
            at += 5;
        } else {
            at += rest[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    None
}

/// Whether a line starts script at its first non-whitespace character: a
/// `//` or `/*` comment, `import`, or a component / interface declaration.
fn script_head(text: &str) -> bool {
    if text.starts_with("/*") || text.starts_with("//") {
        return true;
    }
    if lexer::keyword_at(text, 0, "import") {
        return true;
    }
    // `(component|abstract\s*component|final\s*component|interface)(\s+|\{)`,
    // comments allowed after the modifier as the parser allows them there.
    let mut heads = vec![text];
    for modifier in ["abstract", "final"] {
        if let Some(rest) = strip_prefix_ignore_case(text, modifier) {
            heads.push(&rest[lexer::skip_trivia(rest, 0, lexer::Comments::All)..]);
        }
    }
    heads.iter().any(|head| {
        ["component", "interface"].iter().any(|word| {
            strip_prefix_ignore_case(head, word).is_some_and(|rest| {
                rest.starts_with([' ', '\t', '\n', '\x0c', '\r']) || rest.starts_with('{')
            })
        })
    })
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Leading `//` / `/* */` comments followed by `<`.
fn leading_comments_precede_a_tag(text: &str) -> bool {
    let mut rest = text;
    let mut commented = false;
    loop {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            let Some((_, r)) = after.split_once("*/") else {
                return false;
            };
            rest = r.trim_start();
        } else {
            break;
        }
        commented = true;
    }
    commented && rest.starts_with('<')
}

/// Whether `word` names a scope as the script lexer reads one
/// (`variables`, `local`, `arguments`, `url`, …), ASCII case-insensitively:
/// for a name written as text (an attribute value), which the lexer never
/// sees. `this` and `super` are not scopes here; they have tokens of their
/// own.
pub fn is_scope_name(word: &str) -> bool {
    lexer::in_list(lexer::SCOPE_VARIABLES, word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{ElementKind, TokenKind};

    #[test]
    fn auto_resolves_like_parse_source() {
        for src in [
            "component {}",
            "component{}",
            "abstract component {}",
            "final  component {}",
            "abstract /* c */ component {}",
            "final\n// c\ncomponent {}",
            "interface {}",
            "  \n component {}",
            "// c\ncomponent {}",
            "/** d */\nx = 1;",
            "import a.b;",
            "<!--- licence --->\ncomponent {}",
            "<!---\n * licence\n <!--- nested ---> --->  \n\n  component {}",
            "<!--- a ---> <!--- b --->\ninterface {}",
            // An ignore region is skipped whole, its marker in any case: the
            // `--->` inside it closes nothing.
            "<!--- cfformat-ignore-start --->\n--->\n<!--- cfformat-ignore-end --->\ncomponent {}",
            "<!--- CFFORMAT-IGNORE-START --->\n--->\n<!--- Cfformat-Ignore-End --->\ncomponent {}",
            "<!--- @Formatter:Off --->\n--->\n<!--- @FORMATTER:ON --->\ncomponent {}",
        ] {
            assert!(auto_is_script(src), "{src:?}");
        }
        for src in [
            "<cfset x = 1>",
            "x = 1;",
            "componentish = 1;",
            "// c\n<cfcomponent>",
            "/** license */\n<cfscript>x=1;</cfscript>",
            "<div></div>",
            "<!--- licence ---> component {}",
            "<!--- licence --->\n<cfcomponent>",
            "<!--- never closed\ncomponent {}",
        ] {
            assert!(!auto_is_script(src), "{src:?}");
        }
    }

    #[test]
    fn nesting_past_the_limit_is_an_unmatched_run() {
        // A 2 MB stack, as a test or rayon thread has.
        let deep = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(|| {
                for src in [
                    format!("x = {}{};", "(".repeat(20_000), ")".repeat(20_000)),
                    format!("x = [{}", "{a:[#".repeat(20_000)),
                    "if(a){".repeat(20_000),
                    "<!---".repeat(20_000),
                    format!("var {};", "[".repeat(20_000)),
                ] {
                    let root = parse(&src, Mode::Script).unwrap();
                    assert_eq!(root.span.end as usize, src.len());
                }
                // The whitespace after the run is the top level's, not the
                // deepest element's.
                let src = format!("x = {}1;\n  ", "(".repeat(200));
                let root = parse(&src, Mode::Script).unwrap();
                let last = root.children.last().unwrap();
                assert!(matches!(last, Node::Token(t) if t.kind == TokenKind::Whitespace));
                assert!(matches!(&root.children[root.children.len() - 2],
                    Node::Token(t) if t.kind == TokenKind::Newline));
            })
            .unwrap();
        deep.join().unwrap();
    }

    #[test]
    fn statement_continuations_stay_off_the_stack() {
        // After `abort` or `cffile(…)` without a `;`, the parser keeps
        // reading into the same statement. Each continuation was a
        // recursive call that `depth` did not count: 4,000 overflowed a
        // 2 MB thread in release, 40,000 the CLI's 8 MB worker. Now a loop,
        // so the run is one flat statement, not a nesting past the limit.
        let deep = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(|| {
                for (unit, n) in [("abort ", 40_000), ("cffile(action=\"read\") ", 20_000)] {
                    let src = format!("{}x;", unit.repeat(n));
                    let root = parse(&src, Mode::Script).unwrap();
                    assert_eq!(root.span.end as usize, src.len());
                    let statements: Vec<_> =
                        root.children.iter().filter_map(Node::as_element).collect();
                    assert_eq!(statements.len(), 1, "{unit:?}");
                    assert!(matches!(statements[0].kind, ElementKind::Statement(_)));
                    assert!(statements[0].close.is_some(), "{unit:?}");
                }
            })
            .unwrap();
        deep.join().unwrap();
    }

    #[test]
    fn tags_mode_always_declines() {
        assert_eq!(resolve("component {}", Mode::Tags), Err(Declined));
    }
}
