//! Line-oriented tokenizer for the AVR assembler.
//!
//! Every source line is tokenized exactly once (results are cached per file by the assembler and
//! reused by both passes and by every macro expansion). The scanner is a hand-written byte loop:
//! no regular expressions, no backtracking on pathological input.
//!
//! Comments: `;` and `//` to end of line, `/* ... */` (may span lines).
//!
//! Columns and string lengths are counted in UTF-16 code units, like the original TypeScript
//! implementation (identical to byte/char counts for ASCII sources).

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

use crate::util::utf16_len;

/// Largest literal accepted.
const MAX_LITERAL: u64 = 0xffff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokKind {
    /// Identifier (symbol, mnemonic, register).
    Id,
    /// Integer or character literal (`v` holds the value).
    Num,
    /// String literal (`s` holds the decoded text).
    Str,
    /// Operator / punctuation.
    Op,
    /// Directive `.name` (`s` holds the name without the dot).
    Dir,
    /// Macro parameter `@0`..`@9` (`v` holds the index).
    Param,
}

/// Cheap-to-clone immutable string: static text for operators, shared text otherwise.
#[derive(Clone, Debug)]
pub(crate) enum TStr {
    Static(&'static str),
    Shared(Rc<str>),
}

impl TStr {
    pub(crate) fn to_rc(&self) -> Rc<str> {
        match self {
            TStr::Static(s) => Rc::from(*s),
            TStr::Shared(s) => s.clone(),
        }
    }
}

impl Deref for TStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        match self {
            TStr::Static(s) => s,
            TStr::Shared(s) => s,
        }
    }
}

impl Borrow<str> for TStr {
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}

impl PartialEq for TStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for TStr {}

impl Hash for TStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl fmt::Display for TStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self)
    }
}

impl From<String> for TStr {
    fn from(s: String) -> Self {
        TStr::Shared(Rc::from(s))
    }
}

impl From<&str> for TStr {
    fn from(s: &str) -> Self {
        TStr::Shared(Rc::from(s))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub k: TokKind,
    /// Identifier/directive name (original case), operator text, decoded string or literal text.
    pub s: TStr,
    /// Lowercase `s` for identifiers and directives (case-insensitive matching); `s` otherwise.
    pub lc: TStr,
    /// Numeric value (num), parameter index (param), 0 otherwise.
    pub v: i64,
    /// 1-based column.
    pub col: u32,
}

impl Token {
    #[inline]
    pub(crate) fn is_op(&self, op: &str) -> bool {
        self.k == TokKind::Op && &*self.s == op
    }

    /// Appends the human readable text of the token (for diagnostics and macro listings).
    pub(crate) fn push_text(&self, out: &mut String) {
        match self.k {
            TokKind::Dir => {
                out.push('.');
                out.push_str(&self.s);
            }
            TokKind::Str => {
                out.push('"');
                out.push_str(&self.s);
                out.push('"');
            }
            _ => out.push_str(&self.s),
        }
    }

    /// Human readable text of the token.
    pub(crate) fn text(&self) -> String {
        let mut s = String::with_capacity(self.s.len() + 2);
        self.push_text(&mut s);
        s
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SrcLine {
    /// 1-based line number.
    pub no: u32,
    /// Raw line text (without the line terminator).
    pub text: Rc<str>,
    pub toks: Rc<[Token]>,
    /// First lexical error on the line.
    pub err: Option<Rc<str>>,
    pub err_col: u32,
}

const PARAMS: [&str; 10] = ["@0", "@1", "@2", "@3", "@4", "@5", "@6", "@7", "@8", "@9"];

#[inline]
fn is_id_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

#[inline]
fn is_id_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[inline]
fn hex_val(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'a'..=b'f' => (c - b'a' + 10) as i32,
        b'A'..=b'F' => (c - b'A' + 10) as i32,
        _ => -1,
    }
}

fn two_char_op(a: u8, b: u8) -> Option<&'static str> {
    Some(match (a, b) {
        (b'<', b'<') => "<<",
        (b'>', b'>') => ">>",
        (b'<', b'=') => "<=",
        (b'>', b'=') => ">=",
        (b'=', b'=') => "==",
        (b'!', b'=') => "!=",
        (b'&', b'&') => "&&",
        (b'|', b'|') => "||",
        _ => return None,
    })
}

fn one_char_op(c: u8) -> Option<&'static str> {
    Some(match c {
        b'+' => "+",
        b'-' => "-",
        b'*' => "*",
        b'/' => "/",
        b'%' => "%",
        b'&' => "&",
        b'|' => "|",
        b'^' => "^",
        b'~' => "~",
        b'!' => "!",
        b'<' => "<",
        b'>' => ">",
        b'(' => "(",
        b')' => ")",
        b',' => ",",
        b'=' => "=",
        b'?' => "?",
        b':' => ":",
        b'.' => ".",
        _ => return None,
    })
}

/// Splits `src` into lines and tokenizes each one.
pub(crate) fn tokenize(src: &str) -> Vec<SrcLine> {
    let mut lines = Vec::with_capacity(src.len() / 24 + 1);
    let mut comment = false;
    let bytes = src.as_bytes();
    let len = src.len();
    let mut start = 0usize;
    let mut no = 1u32;
    loop {
        let end = src[start..].find('\n').map_or(len, |p| start + p);
        let mut text_end = end;
        if text_end > start && bytes[text_end - 1] == b'\r' {
            text_end -= 1;
        }
        lines.push(lex_line(&src[start..text_end], no, &mut comment));
        no = no.saturating_add(1);
        if end >= len {
            break;
        }
        start = end + 1;
    }
    lines
}

struct LineLexer<'t> {
    text: &'t str,
    b: &'t [u8],
    ascii: bool,
    toks: Vec<Token>,
    err: Option<(String, u32)>,
}

impl<'t> LineLexer<'t> {
    /// 1-based (UTF-16) column of byte offset `i` (always a char boundary).
    #[inline]
    fn col(&self, i: usize) -> u32 {
        if self.ascii {
            u32::try_from(i).unwrap_or(u32::MAX - 1) + 1
        } else {
            utf16_len(self.text.get(..i).unwrap_or(self.text)).saturating_add(1)
        }
    }

    #[inline]
    fn failed(&self) -> bool {
        self.err.is_some()
    }

    fn fail(&mut self, col: u32, msg: String) {
        if self.err.is_none() {
            self.err = Some((msg, col));
        }
    }

    fn push(&mut self, k: TokKind, s: TStr, lc: TStr, v: i64, col: u32) {
        self.toks.push(Token { k, s, lc, v, col });
    }

    /// Identifier or directive name starting at `i` (ends at the first non-identifier byte).
    fn word(&self, i: usize) -> (usize, TStr, TStr) {
        let mut j = i;
        while j < self.b.len() && is_id_char(self.b[j]) {
            j += 1;
        }
        let raw = &self.text[i..j];
        let s: Rc<str> = Rc::from(raw);
        let lc = if raw.bytes().any(|c| c.is_ascii_uppercase()) {
            TStr::Shared(Rc::from(raw.to_ascii_lowercase()))
        } else {
            TStr::Shared(s.clone())
        };
        (j, TStr::Shared(s), lc)
    }

    fn run(&mut self, comment: &mut bool) {
        let b = self.b;
        let n = b.len();
        let mut i = 0usize;
        while i < n {
            if *comment {
                match self.text[i..].find("*/") {
                    None => return,
                    Some(e) => {
                        i += e + 2;
                        *comment = false;
                        continue;
                    }
                }
            }
            let c = b[i];
            if matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
                i += 1;
                continue;
            }
            if c == b';' {
                break;
            }
            let col = self.col(i);
            let d = if i + 1 < n { b[i + 1] } else { 0 };
            if c == b'/' {
                if d == b'/' {
                    break;
                }
                if d == b'*' {
                    *comment = true;
                    i += 2;
                    continue;
                }
            }
            if is_id_start(c) {
                let (j, s, lc) = self.word(i);
                self.push(TokKind::Id, s, lc, 0, col);
                i = j;
                continue;
            }
            if c.is_ascii_digit() || (c == b'$' && hex_val(d) >= 0) {
                i = self.number(i, col);
                continue;
            }
            if c == b'.' && is_id_start(d) {
                let (j, s, lc) = self.word(i + 1);
                self.push(TokKind::Dir, s, lc, 0, col);
                i = j;
                continue;
            }
            if c == b'@' && d.is_ascii_digit() {
                let p = PARAMS[(d - b'0') as usize];
                self.push(TokKind::Param, TStr::Static(p), TStr::Static(p), (d - b'0') as i64, col);
                i += 2;
                continue;
            }
            if c == b'\'' || c == b'"' {
                i = self.quoted(i, c, col);
                continue;
            }
            if let Some(op) = two_char_op(c, d) {
                self.push(TokKind::Op, TStr::Static(op), TStr::Static(op), 0, col);
                i += 2;
                continue;
            }
            if let Some(op) = one_char_op(c) {
                self.push(TokKind::Op, TStr::Static(op), TStr::Static(op), 0, col);
                i += 1;
                continue;
            }
            let ch = self.text[i..].chars().next().unwrap_or('\u{fffd}');
            if !self.failed() {
                self.fail(col, format!("unexpected character '{ch}'"));
            }
            i += ch.len_utf8().max(1);
        }
    }

    fn number(&mut self, start: usize, col: u32) -> usize {
        let b = self.b;
        let n = b.len();
        let mut i = start;
        let mut base: u64 = 10;
        let c0 = b[i];
        let c1 = if i + 1 < n { b[i + 1] | 0x20 } else { 0 };
        if c0 == b'$' {
            base = 16;
            i += 1;
        } else if c0 == b'0' && c1 == b'x' {
            base = 16;
            i += 2;
        } else if c0 == b'0' && c1 == b'b' && matches!(b.get(i + 2), Some(b'0' | b'1')) {
            base = 2;
            i += 2;
        } else if c0 == b'0' && i + 1 < n && b[i + 1].is_ascii_digit() {
            base = 8; // avrasm2: a leading zero denotes octal
            i += 1;
        }
        let mut v: u64 = 0;
        let mut digits = 0usize;
        let mut bad = false;
        while i < n {
            let h = hex_val(b[i]);
            if h < 0 || (base != 16 && h > 9) {
                break;
            }
            if h as u64 >= base {
                bad = true;
            }
            v = v.saturating_mul(base).saturating_add(h as u64);
            digits += 1;
            i += 1;
        }
        // Swallow trailing identifier characters so `12abc` is one bad token, not two.
        while i < n && is_id_char(b[i]) {
            bad = true;
            i += 1;
        }
        let s = &self.text[start..i];
        if !self.failed() {
            if bad || digits == 0 {
                self.fail(col, format!("invalid number '{s}'"));
            } else if v > MAX_LITERAL {
                self.fail(col, format!("number '{s}' is too large"));
            }
        }
        let text = TStr::from(s);
        let value = if bad { 0 } else { v.min(i64::MAX as u64) as i64 };
        self.push(TokKind::Num, text.clone(), text, value, col);
        i
    }

    fn quoted(&mut self, start: usize, quote: u8, col: u32) -> usize {
        let b = self.b;
        let n = b.len();
        let mut out = String::new();
        let mut j = start + 1;
        let mut closed = false;
        while j < n {
            let c = b[j];
            if c == quote {
                closed = true;
                j += 1;
                break;
            }
            if c == b'\\' && j + 1 < n {
                let e = self.text[j + 1..].chars().next().unwrap_or('\\');
                j += 1 + e.len_utf8();
                match e {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '0' => out.push('\0'),
                    'a' => out.push('\x07'),
                    'b' => out.push('\x08'),
                    'f' => out.push('\x0c'),
                    'v' => out.push('\x0b'),
                    'x' => {
                        let mut v = 0u32;
                        let mut k = 0;
                        while k < 2 && j < n && hex_val(b[j]) >= 0 {
                            v = v * 16 + hex_val(b[j]) as u32;
                            j += 1;
                            k += 1;
                        }
                        if k == 0 {
                            let at = self.col(j) - 1;
                            self.fail(at, "invalid \\x escape".into());
                        }
                        out.push(char::from_u32(v).unwrap_or('\0'));
                    }
                    // \\ \' \" and unknown escapes map to the character itself
                    other => out.push(other),
                }
                continue;
            }
            let ch = self.text[j..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            j += ch.len_utf8().max(1);
        }
        if !closed {
            let msg = if quote == b'\'' { "unterminated character constant" } else { "unterminated string" };
            self.fail(col, msg.into());
        }
        if quote == b'\'' {
            let mut units = out.encode_utf16();
            let first = units.next();
            if closed && (first.is_none() || units.next().is_some()) {
                self.fail(col, "character constant must contain exactly one character".into());
            }
            let raw = TStr::from(&self.text[start..j]);
            self.push(TokKind::Num, raw.clone(), raw, first.unwrap_or(0) as i64, col);
        } else {
            let s = TStr::from(out);
            self.push(TokKind::Str, s.clone(), s, 0, col);
        }
        j
    }
}

fn lex_line(text: &str, no: u32, comment: &mut bool) -> SrcLine {
    let mut lx = LineLexer { text, b: text.as_bytes(), ascii: text.is_ascii(), toks: Vec::new(), err: None };
    lx.run(comment);
    let (err, err_col) = match lx.err {
        Some((m, c)) => (Some(Rc::from(m)), c),
        None => (None, 0),
    };
    SrcLine { no, text: Rc::from(text), toks: Rc::from(lx.toks), err, err_col }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<(TokKind, String, i64, u32)> {
        let lines = tokenize(s);
        lines[0].toks.iter().map(|t| (t.k, t.s.to_string(), t.v, t.col)).collect()
    }

    #[test]
    fn numbers_and_ops() {
        let t = toks("ldi r16, $1F + 0x10 << 010 | 0b101 ; c");
        let v: Vec<i64> = t.iter().filter(|x| x.0 == TokKind::Num).map(|x| x.2).collect();
        assert_eq!(v, vec![0x1f, 0x10, 8, 5]);
        assert!(t.iter().any(|x| x.0 == TokKind::Op && x.1 == "<<"));
        assert_eq!(t[0].3, 1);
        assert_eq!(t[1].3, 5);
    }

    #[test]
    fn strings_chars_and_errors() {
        let l = tokenize(".db \"a\\tb\\x41\", 'Z'");
        assert_eq!(&*l[0].toks[0].s, "db");
        assert_eq!(&*l[0].toks[1].s, "a\tbA");
        assert_eq!(l[0].toks[3].v, 'Z' as i64);
        assert!(l[0].err.is_none());
        let e = tokenize("ldi r16, 0x");
        assert_eq!(e[0].err.as_deref(), Some("invalid number '0x'"));
        let e = tokenize("x = 4294967296");
        assert_eq!(e[0].err.as_deref(), Some("number '4294967296' is too large"));
        let e = tokenize("\"abc");
        assert_eq!(e[0].err.as_deref(), Some("unterminated string"));
        let e = tokenize("'ab'");
        assert_eq!(e[0].err.as_deref(), Some("character constant must contain exactly one character"));
        let e = tokenize("é # x");
        assert_eq!(e[0].err.as_deref(), Some("unexpected character 'é'"));
        assert_eq!(e[0].toks[0].col, 5);
    }

    #[test]
    fn comments_span_lines() {
        let l = tokenize("a /* x\n y */ b // c\r\n@3");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].toks.len(), 1);
        assert_eq!(&*l[1].toks[0].s, "b");
        assert_eq!(l[2].toks[0].k, TokKind::Param);
        assert_eq!(l[2].toks[0].v, 3);
    }
}
