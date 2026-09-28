//! `str_width` (Prettier's `getStringWidth`).

use cfdoc::width::str_width;

#[test]
fn widths() {
    let cases: &[(&str, usize)] = &[
        ("", 0),
        ("abc", 3),
        ("a b~", 4),
        // DEL is inside Prettier's ASCII fast path.
        ("\u{7f}", 1),
        // CJK wide / fullwidth.
        ("日本語", 6),
        ("ＡＢ", 4),
        ("a日b", 4),
        // Combining marks are zero width.
        ("e\u{301}", 1),
        ("a\u{20dd}", 1),
        // Control characters are zero width (and leave the fast path).
        ("a\u{7}b", 2),
        ("\t", 0),
        ("a\nb", 2),
        ("\u{85}x", 1),
        // Ambiguous width is narrow.
        ("±§", 2),
        ("\u{1f600}", 2),
        // Emoji ZWJ sequences and flags count once (Prettier: 2).
        ("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}", 2),
        ("\u{1f1fa}\u{1f1f8}", 2),
        // Known differences from Prettier: a lone ZWJ and a soft
        // hyphen are 0 here and 1 in Prettier.
        ("\u{200d}", 0),
        ("\u{ad}", 0),
    ];
    for (text, width) in cases {
        assert_eq!(str_width(text), *width, "{text:?}");
    }
}
