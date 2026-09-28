//! Input normalisation: strip a leading BOM and turn `\r\n` and
//! lone `\r` into `\n`, remembering both so output can reproduce them.

use crate::tree::Original;

/// Dominant line ending of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Newline {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl Newline {
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::CrLf => "\r\n",
            Newline::Cr => "\r",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Newline::Lf => "lf",
            Newline::CrLf => "crlf",
            Newline::Cr => "cr",
        }
    }
}

pub(crate) struct Normalized {
    pub(crate) text: String,
    pub(crate) bom: bool,
    pub(crate) newline: Newline,
    pub(crate) original: Option<Original>,
}

const BOM: char = '\u{FEFF}';

/// The normalised text a tree's token spans index into.
pub fn normalized_text(src: &str) -> String {
    normalize(src).text
}

pub(crate) fn normalize(src: &str) -> Normalized {
    let (bom, body) = match src.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, src),
    };

    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let text;
    let mut crlf_at = Vec::new();
    if body.contains('\r') {
        let bytes = body.as_bytes();
        let mut out = String::with_capacity(body.len());
        let mut last = 0;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' => {
                    out.push_str(&body[last..i]);
                    if bytes.get(i + 1) == Some(&b'\n') {
                        crlf += 1;
                        crlf_at.push(out.len() as u32);
                        i += 1;
                    } else {
                        cr += 1;
                    }
                    out.push('\n');
                    last = i + 1;
                }
                b'\n' => lf += 1,
                _ => {}
            }
            i += 1;
        }
        out.push_str(&body[last..]);
        text = out;
    } else {
        lf = body.bytes().filter(|&b| b == b'\n').count();
        text = body.to_string();
    }

    let newline = if crlf > lf && crlf >= cr {
        Newline::CrLf
    } else if cr > lf && cr > crlf {
        Newline::Cr
    } else {
        Newline::Lf
    };

    let original = (bom || crlf > 0 || cr > 0).then(|| Original {
        text: src.to_string(),
        prefix: if bom { BOM.len_utf8() as u32 } else { 0 },
        crlf_at,
    });

    Normalized {
        text,
        bom,
        newline,
        original,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lf() {
        let n = normalize("a\nb\n");
        assert_eq!(n.text, "a\nb\n");
        assert!(!n.bom);
        assert_eq!(n.newline, Newline::Lf);
        assert!(n.original.is_none());
    }

    #[test]
    fn crlf_and_bom() {
        let n = normalize("\u{FEFF}a\r\nb\r\nc\n");
        assert_eq!(n.text, "a\nb\nc\n");
        assert!(n.bom);
        assert_eq!(n.newline, Newline::CrLf);
        assert_eq!(n.original.unwrap().crlf_at, vec![1, 3]);
    }

    #[test]
    fn lone_cr() {
        let n = normalize("a\rb\rc");
        assert_eq!(n.text, "a\nb\nc");
        assert_eq!(n.newline, Newline::Cr);
        // Same length, but the input is kept so ignore regions keep `\r`.
        assert!(n.original.unwrap().crlf_at.is_empty());
    }
}
