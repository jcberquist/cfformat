//! `getStringWidth` (`prettier/src/utilities/get-string-width.js`).

use unicode_width::UnicodeWidthStr;

/// Display width of `text` in columns.
///
/// Printable ASCII (`0x20..=0x7F`) is its byte length (Prettier's fast path).
/// Otherwise C0/C1 control characters are 0 and the runs between them are
/// measured with `unicode-width`'s string width: combining marks 0, East
/// Asian wide/fullwidth 2, emoji (including ZWJ sequences and flags) 2,
/// ambiguous width narrow. Prettier's emoji and format-character handling
/// differs at the margins: a lone ZWJ (U+200D) and a soft hyphen (U+00AD)
/// are 0 here and 1 in Prettier.
pub fn str_width(text: &str) -> usize {
    if text.bytes().all(|b| (0x20..=0x7f).contains(&b)) {
        return text.len();
    }
    text.split(is_control).map(UnicodeWidthStr::width).sum()
}

/// get-string-width.js: `codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)`.
fn is_control(c: char) -> bool {
    matches!(c as u32, 0..=0x1f | 0x7f..=0x9f)
}
