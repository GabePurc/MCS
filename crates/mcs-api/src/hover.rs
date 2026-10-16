//! Source-level symbol index behind the editor hover: what is this name (label, constant,
//! macro, function, variable, parameter ...), without building the program first.
//!
//! * Assembly follows avrasm2 syntax (`;` comments, case-insensitive names). Device constants
//!   (`RAMEND`, `FLASHEND`, ...) come from the generated device definition include
//!   ([`mcs_asm::generate_def_include`]); I/O register and bit names are left to the UI.
//! * C uses a heuristic scanner (no preprocessor, no full grammar): comments and strings are
//!   blanked, then declarations are recognised from the token stream with brace tracking.
//!
//! Both are linear in the source size and only run on hover.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// One named thing found in a source file.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSymbol {
    pub name: String,
    /// "label", "constant", "variable", "register alias", "macro", "device constant" (asm);
    /// "function", "global", "type", "constant", "macro", "parameter", "local" (C).
    pub kind: String,
    /// One-line description, e.g. a C signature or `.equ SIZE = 8`.
    pub detail: String,
    pub value: Option<i64>,
    /// 1-based definition line (0 for device constants).
    pub line: u32,
    /// Inclusive line range where the symbol is visible; `None` = whole file.
    pub scope: Option<(u32, u32)>,
    /// Comment block directly above the definition, or the trailing comment on its line.
    pub doc: String,
}

/// Symbols of `source` (`language` is "asm" or "c"; anything else yields nothing).
pub fn source_symbols(source: &str, language: &str, device_id: &str) -> Vec<SourceSymbol> {
    match language {
        "asm" => scan_asm(source, device_id),
        "c" => scan_c(source),
        _ => Vec::new(),
    }
}

fn sym(name: &str, kind: &str, detail: String, value: Option<i64>, line: u32, scope: Option<(u32, u32)>, doc: String) -> SourceSymbol {
    SourceSymbol { name: name.to_string(), kind: kind.to_string(), detail, value, line, scope, doc }
}

// ---------------------------------------------------------------------------------------------
// Comments / docs
// ---------------------------------------------------------------------------------------------

struct Comment {
    start: u32,
    end: u32,
    text: String,
    /// Nothing but whitespace before the comment on its first line.
    own_line: bool,
    /// `//` or `;` comment (may chain with the line above).
    line_style: bool,
}

struct Docs {
    by_end: HashMap<u32, usize>,
    by_start: HashMap<u32, usize>,
    list: Vec<Comment>,
}

impl Docs {
    fn new(list: Vec<Comment>) -> Self {
        let mut by_end = HashMap::new();
        let mut by_start = HashMap::new();
        for (i, c) in list.iter().enumerate() {
            by_end.insert(c.end, i);
            by_start.entry(c.start).or_insert(i);
        }
        Docs { by_end, by_start, list }
    }

    /// Comment block directly above `line`, else a trailing comment on `line`.
    fn doc(&self, line: u32) -> String {
        if line > 1 {
            if let Some(&i) = self.by_end.get(&(line - 1)) {
                if self.list[i].own_line {
                    let mut parts = vec![self.list[i].text.clone()];
                    if self.list[i].line_style {
                        let mut cur = i;
                        while self.list[cur].start > 1 {
                            match self.by_end.get(&(self.list[cur].start - 1)) {
                                Some(&p) if self.list[p].own_line && self.list[p].line_style => {
                                    parts.push(self.list[p].text.clone());
                                    cur = p;
                                }
                                _ => break,
                            }
                        }
                    }
                    parts.reverse();
                    return parts.join("\n");
                }
            }
        }
        if let Some(&i) = self.by_start.get(&line) {
            if !self.list[i].own_line {
                return self.list[i].text.clone();
            }
        }
        String::new()
    }
}

fn clean_comment(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for l in raw.lines() {
        let t = l.trim();
        let t = t.trim_start_matches(['/', ';', '*', '!', '<']).trim_end_matches(['*', '/']).trim();
        out.push(t.to_string());
    }
    while out.last().is_some_and(|s| s.is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|s| s.is_empty()) {
        out.remove(0);
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------------------------
// Constant expression evaluator (shared)
// ---------------------------------------------------------------------------------------------

struct Ev<'a> {
    b: &'a [u8],
    p: usize,
    syms: &'a HashMap<String, i64>,
    asm: bool,
    depth: u32,
}

fn eval_expr(text: &str, syms: &HashMap<String, i64>, asm: bool) -> Option<i64> {
    let mut e = Ev { b: text.as_bytes(), p: 0, syms, asm, depth: 0 };
    let v = e.or()?;
    e.ws();
    (e.p == e.b.len()).then_some(v)
}

impl Ev<'_> {
    fn ws(&mut self) {
        while self.b.get(self.p).is_some_and(|c| c.is_ascii_whitespace()) {
            self.p += 1;
        }
    }
    fn eat(&mut self, op: &str) -> bool {
        self.ws();
        let o = op.as_bytes();
        if self.b[self.p..].starts_with(o) {
            // do not split `<<`/`>>`/`||`/`&&` into single-char operators
            let next = self.b.get(self.p + o.len()).copied();
            if o.len() == 1 && matches!((o[0], next), (b'|', Some(b'|')) | (b'&', Some(b'&')) | (b'<', Some(b'<')) | (b'>', Some(b'>'))) {
                return false;
            }
            self.p += o.len();
            return true;
        }
        false
    }
    fn or(&mut self) -> Option<i64> {
        let mut v = self.xor()?;
        while self.eat("|") {
            v |= self.xor()?;
        }
        Some(v)
    }
    fn xor(&mut self) -> Option<i64> {
        let mut v = self.and()?;
        while self.eat("^") {
            v ^= self.and()?;
        }
        Some(v)
    }
    fn and(&mut self) -> Option<i64> {
        let mut v = self.shift()?;
        while self.eat("&") {
            v &= self.shift()?;
        }
        Some(v)
    }
    fn shift(&mut self) -> Option<i64> {
        let mut v = self.add()?;
        loop {
            if self.eat("<<") {
                v = v.checked_shl(u32::try_from(self.add()?).ok()?)?;
            } else if self.eat(">>") {
                v = v.checked_shr(u32::try_from(self.add()?).ok()?)?;
            } else {
                return Some(v);
            }
        }
    }
    fn add(&mut self) -> Option<i64> {
        let mut v = self.mul()?;
        loop {
            if self.eat("+") {
                v = v.checked_add(self.mul()?)?;
            } else if self.eat("-") {
                v = v.checked_sub(self.mul()?)?;
            } else {
                return Some(v);
            }
        }
    }
    fn mul(&mut self) -> Option<i64> {
        let mut v = self.unary()?;
        loop {
            if self.eat("*") {
                v = v.checked_mul(self.unary()?)?;
            } else if self.eat("/") {
                v = v.checked_div(self.unary()?)?;
            } else if self.eat("%") {
                v = v.checked_rem(self.unary()?)?;
            } else {
                return Some(v);
            }
        }
    }
    fn unary(&mut self) -> Option<i64> {
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let r = self.unary_inner();
        self.depth -= 1;
        r
    }
    fn unary_inner(&mut self) -> Option<i64> {
        if self.eat("-") {
            return self.unary()?.checked_neg();
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("~") {
            return Some(!self.unary()?);
        }
        if self.eat("(") {
            let v = self.or()?;
            return self.eat(")").then_some(v);
        }
        self.ws();
        let c = *self.b.get(self.p)?;
        if c == b'\'' {
            let ch = *self.b.get(self.p + 1)?;
            let (v, n) = if ch == b'\\' { (match *self.b.get(self.p + 2)? { b'n' => 10, b'r' => 13, b't' => 9, b'0' => 0, o => o as i64 }, 4) } else { (ch as i64, 3) };
            if self.b.get(self.p + n - 1) != Some(&b'\'') {
                return None;
            }
            self.p += n;
            return Some(v);
        }
        if self.asm && c == b'$' {
            let s = self.p + 1;
            self.p = s;
            while self.b.get(self.p).is_some_and(|c| c.is_ascii_hexdigit()) {
                self.p += 1;
            }
            return i64::from_str_radix(std::str::from_utf8(&self.b[s..self.p]).ok()?, 16).ok();
        }
        if c.is_ascii_digit() {
            return self.number();
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = self.p;
            while self.b.get(self.p).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                self.p += 1;
            }
            let name = std::str::from_utf8(&self.b[s..self.p]).ok()?;
            let key = if self.asm { name.to_ascii_lowercase() } else { name.to_string() };
            if self.asm && (key == "low" || key == "high") && self.eat("(") {
                let v = self.or()?;
                if !self.eat(")") {
                    return None;
                }
                return Some(if key == "low" { v & 0xff } else { (v >> 8) & 0xff });
            }
            return self.syms.get(&key).copied();
        }
        None
    }
    fn number(&mut self) -> Option<i64> {
        let s = self.p;
        while self.b.get(self.p).is_some_and(|c| c.is_ascii_alphanumeric()) {
            self.p += 1;
        }
        let mut t = std::str::from_utf8(&self.b[s..self.p]).ok()?.to_ascii_lowercase();
        if !self.asm {
            while t.ends_with(['u', 'l']) && !t.starts_with("0x") || (t.starts_with("0x") && t.ends_with(['u', 'l'])) {
                t.pop();
            }
        }
        if let Some(h) = t.strip_prefix("0x") {
            i64::from_str_radix(h, 16).ok()
        } else if let Some(b) = t.strip_prefix("0b") {
            i64::from_str_radix(b, 2).ok()
        } else if !self.asm && t.len() > 1 && t.starts_with('0') {
            i64::from_str_radix(&t[1..], 8).ok()
        } else {
            t.parse().ok()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------------------------

/// Index of the `;` that starts a comment (outside quotes).
fn asm_comment_start(line: &str) -> Option<usize> {
    let mut q: Option<u8> = None;
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match (q, b[i]) {
            (Some(_), b'\\') => i += 1,
            (Some(c), x) if x == c => q = None,
            (None, b'"') | (None, b'\'') => q = Some(b[i]),
            (None, b';') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Seg {
    Code,
    Data,
    Eeprom,
}

fn split_word(s: &str) -> (&str, &str) {
    match s.find(|c: char| c.is_whitespace()) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, ""),
    }
}

fn is_asm_ident(s: &str) -> bool {
    let mut ch = s.chars();
    ch.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && ch.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$')
}

fn scan_asm(source: &str, device_id: &str) -> Vec<SourceSymbol> {
    let spec = crate::avr_spec(device_id);
    let mut out: Vec<SourceSymbol> = Vec::new();
    let mut consts: HashMap<String, i64> = HashMap::new();
    let mut device: Vec<SourceSymbol> = Vec::new();
    if let Some(spec) = spec {
        device = device_constants(&mcs_asm::generate_def_include(spec));
        for d in &device {
            if let Some(v) = d.value {
                consts.insert(d.name.to_ascii_lowercase(), v);
            }
        }
    }
    let sram_start = spec.map(|s| s.sram_start as i64).unwrap_or(0x60);

    // Pass 1: comments.
    let lines: Vec<&str> = source.lines().collect();
    let mut comments = Vec::new();
    let mut codes: Vec<&str> = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        match asm_comment_start(l) {
            Some(p) => {
                comments.push(Comment { start: i as u32 + 1, end: i as u32 + 1, text: clean_comment(&l[p..]), own_line: l[..p].trim().is_empty(), line_style: true });
                codes.push(&l[..p]);
            }
            None => codes.push(l),
        }
    }
    let docs = Docs::new(comments);

    let mut seg = Seg::Code;
    let mut dpc = sram_start;
    let mut last_label: Option<usize> = None;
    let mut macro_open: Option<(usize, u32)> = None; // (symbol index, max @n + 1)
    for (i, raw) in codes.iter().enumerate() {
        let ln = i as u32 + 1;
        let mut rest = raw.trim();
        if let Some((mi, ref mut maxn)) = macro_open {
            let b = rest.as_bytes();
            for w in 0..b.len().saturating_sub(1) {
                if b[w] == b'@' && b[w + 1].is_ascii_digit() {
                    *maxn = (*maxn).max((b[w + 1] - b'0') as u32 + 1);
                }
            }
            let (w, _) = split_word(rest);
            if w.eq_ignore_ascii_case(".endmacro") || w.eq_ignore_ascii_case(".endm") {
                let name = out[mi].name.clone();
                let args: Vec<String> = (0..*maxn).map(|n| format!("@{n}")).collect();
                out[mi].detail = if args.is_empty() { format!("macro {name}") } else { format!("macro {name} {}", args.join(", ")) };
                macro_open = None;
            }
            continue;
        }
        // Label(s).
        while let Some(c) = rest.find(':') {
            let name = &rest[..c];
            if !is_asm_ident(name) || name.contains('.') && name.starts_with('.') {
                break;
            }
            let (kind_detail, value) = match seg {
                Seg::Code => ("label in code segment".to_string(), None),
                Seg::Data => ("label in data segment".to_string(), Some(dpc)),
                Seg::Eeprom => ("label in EEPROM segment".to_string(), None),
            };
            out.push(sym(name, "label", kind_detail, value, ln, None, docs.doc(ln)));
            last_label = Some(out.len() - 1);
            rest = rest[c + 1..].trim();
        }
        if rest.is_empty() {
            continue;
        }
        let (word, args) = split_word(rest);
        let lw = word.to_ascii_lowercase();
        match lw.as_str() {
            ".cseg" | ".dseg" | ".eseg" => {
                seg = match lw.as_str() {
                    ".cseg" => Seg::Code,
                    ".dseg" => Seg::Data,
                    _ => Seg::Eeprom,
                };
                last_label = None;
            }
            ".org" => {
                if seg == Seg::Data {
                    if let Some(v) = eval_expr(args, &consts, true) {
                        dpc = v;
                    }
                }
                last_label = None;
            }
            ".byte" => {
                let n = eval_expr(args, &consts, true);
                if seg == Seg::Data {
                    if let (Some(li), Some(n)) = (last_label, n) {
                        let at = out[li].value.unwrap_or(dpc);
                        out[li].kind = "variable".into();
                        out[li].detail = format!("{n} byte{} in SRAM at 0x{at:04X}", if n == 1 { "" } else { "s" });
                    }
                    dpc += n.unwrap_or(0);
                }
                last_label = None;
            }
            ".equ" | ".set" => {
                if let Some((name, expr)) = args.split_once('=') {
                    let (name, expr) = (name.trim(), expr.trim());
                    if is_asm_ident(name) {
                        let v = eval_expr(expr, &consts, true);
                        if let Some(v) = v {
                            consts.insert(name.to_ascii_lowercase(), v);
                        }
                        let kind = if lw == ".equ" { "constant" } else { "variable" };
                        out.push(sym(name, kind, format!("{lw} {name} = {expr}"), v, ln, None, docs.doc(ln)));
                    }
                }
                last_label = None;
            }
            ".def" => {
                if let Some((name, reg)) = args.split_once('=') {
                    let (name, reg) = (name.trim(), reg.trim());
                    let n = reg.strip_prefix(['r', 'R']).and_then(|d| d.parse::<i64>().ok()).filter(|n| (0..32).contains(n));
                    if is_asm_ident(name) {
                        out.push(sym(name, "register alias", format!(".def {name} = {reg}"), n, ln, None, docs.doc(ln)));
                    }
                }
                last_label = None;
            }
            ".macro" => {
                let name = split_word(args).0;
                if is_asm_ident(name) {
                    out.push(sym(name, "macro", format!("macro {name}"), None, ln, None, docs.doc(ln)));
                    macro_open = Some((out.len() - 1, 0));
                }
                last_label = None;
            }
            _ => last_label = None,
        }
    }
    out.extend(device);
    out
}

/// `.equ NAME = value` entries of the generated include after the I/O and bit sections
/// (signature bytes, memory sizes, interrupt vector addresses).
fn device_constants(include: &str) -> Vec<SourceSymbol> {
    let mut out = Vec::new();
    let mut keep = false;
    for l in include.lines() {
        if let Some(t) = l.strip_prefix("; ***** ") {
            keep = t.starts_with("DATA MEMORY") || t.starts_with("INTERRUPT VECTORS");
            continue;
        }
        if !keep {
            continue;
        }
        let Some(rest) = l.strip_prefix(".equ") else { continue };
        let (body, note) = match rest.split_once(';') {
            Some((b, n)) => (b, n.trim()),
            None => (rest, ""),
        };
        let Some((name, val)) = body.split_once('=') else { continue };
        let (name, val) = (name.trim(), val.trim());
        let v = eval_expr(val, &HashMap::new(), true);
        out.push(sym(name, "device constant", format!(".equ {name} = {val}"), v, 0, None, note.to_string()));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// C
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum TK {
    Ident,
    Num,
    Str,
    Punct,
}

struct Tok {
    s: usize,
    e: usize,
    line: u32,
    kind: TK,
}

/// Blanks comments and string/char contents; returns (code, code with strings kept, comments).
fn c_strip(src: &str) -> (Vec<u8>, Vec<u8>, Vec<Comment>) {
    let b = src.as_bytes();
    let mut code = b.to_vec();
    let mut plain = b.to_vec();
    let mut comments = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut line_has_code = false;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            line_has_code = false;
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let s = i;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            comments.push(Comment { start: line, end: line, text: clean_comment(&src[s..i]), own_line: !line_has_code, line_style: true });
            for k in s..i {
                code[k] = b' ';
                plain[k] = b' ';
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let (s, sl, own) = (i, line, !line_has_code);
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                if b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(b.len());
            comments.push(Comment { start: sl, end: line, text: clean_comment(&src[s..i]), own_line: own, line_style: false });
            for k in s..i {
                if b[k] != b'\n' {
                    code[k] = b' ';
                    plain[k] = b' ';
                }
            }
        } else if c == b'"' || (c == b'\'' && !(i > 0 && b[i - 1].is_ascii_alphanumeric())) {
            line_has_code = true;
            i += 1;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                if b[i] == b'\\' {
                    code[i] = b' ';
                    i += 1;
                    if i >= b.len() || b[i] == b'\n' {
                        break;
                    }
                }
                code[i] = b' ';
                i += 1;
            }
            if i < b.len() && b[i] == c {
                i += 1;
            }
        } else {
            if !c.is_ascii_whitespace() {
                line_has_code = true;
            }
            i += 1;
        }
    }
    (code, plain, comments)
}

const STORAGE: &[&str] = &["const", "volatile", "static", "extern", "register", "inline", "auto", "restrict", "__inline__", "__inline", "__extension__", "_Noreturn", "_Static"];
const BUILTIN: &[&str] = &[
    "void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "_Bool", "bool", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t",
    "uint32_t", "uint64_t", "intptr_t", "uintptr_t", "size_t", "ssize_t", "ptrdiff_t", "int_fast8_t", "int_fast16_t", "int_fast32_t", "uint_fast8_t", "uint_fast16_t",
    "uint_fast32_t", "int_least8_t", "int_least16_t", "int_least32_t", "uint_least8_t", "uint_least16_t", "uint_least32_t", "FILE", "wchar_t", "va_list",
];
const KEYWORDS: &[&str] = &[
    "return", "else", "goto", "case", "default", "sizeof", "if", "while", "for", "do", "switch", "break", "continue", "typedef", "struct", "union", "enum",
];

struct CScan<'a> {
    src: &'a [u8],
    plain: &'a [u8],
    toks: Vec<Tok>,
    mate: Vec<usize>,
    docs: Docs,
    out: Vec<SourceSymbol>,
    types: HashSet<String>,
    consts: HashMap<String, i64>,
    protos: Vec<(usize, String)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Global,
    Local(u32),
}

fn scan_c(source: &str) -> Vec<SourceSymbol> {
    let (mut code, plain, comments) = c_strip(source);
    let mut out = Vec::new();
    let mut consts: HashMap<String, i64> = HashMap::new();
    let docs = Docs::new(comments);

    // Preprocessor lines: record #define, then blank them for the token scan.
    let mut off = 0;
    let mut line = 1u32;
    let n = code.len();
    while off < n {
        let ls = off;
        let mut le = ls;
        let mut extra_lines = 0;
        loop {
            while le < n && code[le] != b'\n' {
                le += 1;
            }
            let cont = code[ls..le].iter().rev().find(|c| !c.is_ascii_whitespace()) == Some(&b'\\');
            if cont && le < n {
                le += 1;
                extra_lines += 1;
            } else {
                break;
            }
        }
        let first = code[ls..le].iter().position(|c| !c.is_ascii_whitespace());
        if let Some(f) = first.filter(|&f| code[ls + f] == b'#') {
            let text: String = String::from_utf8_lossy(&plain[ls + f + 1..le]).replace("\\\n", " ").replace("\\\r\n", " ");
            let text = text.trim();
            if let Some(def) = text.strip_prefix("define").filter(|r| r.starts_with(char::is_whitespace)) {
                let def = def.trim_start();
                let nlen = def.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(def.len());
                let name = &def[..nlen];
                if !name.is_empty() {
                    let after = &def[nlen..];
                    let body_of = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
                    let (detail, value) = if let Some(p) = after.strip_prefix('(') {
                        let close = p.find(')').unwrap_or(p.len());
                        let params = body_of(&p[..close]);
                        let body = body_of(p.get(close + 1..).unwrap_or(""));
                        let params = params.replace(" ,", ",");
                        (format!("#define {name}({params}) {body}").trim_end().to_string(), None)
                    } else {
                        let body = body_of(after);
                        let v = eval_expr(&body, &consts, false);
                        if let Some(v) = v {
                            consts.insert(name.to_string(), v);
                        }
                        (format!("#define {name} {body}").trim_end().to_string(), v)
                    };
                    let detail = if detail.chars().count() > 120 { format!("{}…", detail.chars().take(119).collect::<String>()) } else { detail };
                    out.push(sym(name, "macro", detail, value, line, None, docs.doc(line)));
                }
            }
            for c in &mut code[ls..le] {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
        }
        line += 1 + extra_lines;
        off = le + 1;
    }

    // Tokens.
    let mut toks = Vec::new();
    let (mut i, mut line) = (0, 1u32);
    while i < n {
        let c = code[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < n && (code[i].is_ascii_alphanumeric() || code[i] == b'_') {
                i += 1;
            }
            toks.push(Tok { s, e: i, line, kind: TK::Ident });
        } else if c.is_ascii_digit() {
            let s = i;
            while i < n && (code[i].is_ascii_alphanumeric() || code[i] == b'.' || code[i] == b'_') {
                i += 1;
            }
            toks.push(Tok { s, e: i, line, kind: TK::Num });
        } else if c == b'"' || c == b'\'' {
            let s = i;
            i += 1;
            while i < n && code[i] != c && code[i] != b'\n' {
                i += 1;
            }
            if i < n && code[i] == c {
                i += 1;
            }
            toks.push(Tok { s, e: i, line, kind: TK::Str });
        } else {
            toks.push(Tok { s: i, e: i + 1, line, kind: TK::Punct });
            i += 1;
        }
    }
    // Bracket matching.
    let mut mate = vec![usize::MAX; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (k, t) in toks.iter().enumerate() {
        if t.kind != TK::Punct {
            continue;
        }
        match code[t.s] {
            b'(' | b'[' | b'{' => stack.push(k),
            b')' | b']' | b'}' => {
                if let Some(o) = stack.pop() {
                    mate[o] = k;
                    mate[k] = o;
                }
            }
            _ => {}
        }
    }
    let mut sc = CScan { src: &code, plain: &plain, toks, mate, docs, out, types: HashSet::new(), consts, protos: Vec::new() };
    sc.top();
    let mut res = sc.out;
    // A prototype is redundant once the function is defined (keep its doc).
    let protos = std::mem::take(&mut sc.protos);
    let mut drop = HashSet::new();
    for (pi, name) in protos {
        if let Some(di) = res.iter().position(|s| s.kind == "function" && s.name == name && !s.detail.ends_with(';') && s.scope.is_none() && s.line != res[pi].line && !drop.contains(&pi) && s.line > 0) {
            if res[di].doc.is_empty() && res[di].line > res[pi].line {
                res[di].doc = res[pi].doc.clone();
            }
            if di != pi {
                drop.insert(pi);
            }
        }
    }
    let _ = &sc.consts;
    res.into_iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, s)| s).collect()
}

impl CScan<'_> {
    fn t(&self, i: usize) -> &str {
        match self.toks.get(i) {
            Some(t) => std::str::from_utf8(&self.src[t.s..t.e]).unwrap_or(""),
            None => "",
        }
    }
    fn is(&self, i: usize, s: &str) -> bool {
        self.toks.get(i).is_some_and(|t| t.kind != TK::Str) && self.t(i) == s
    }
    fn ident(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| t.kind == TK::Ident)
    }
    fn line(&self, i: usize) -> u32 {
        self.toks.get(i).or(self.toks.last()).map_or(1, |t| t.line)
    }
    fn end_of(&self, open: usize) -> usize {
        if self.mate[open] == usize::MAX {
            self.toks.len().saturating_sub(1)
        } else {
            self.mate[open]
        }
    }

    fn is_type_start(&self, i: usize) -> bool {
        let t = self.t(i);
        self.ident(i) && (STORAGE.contains(&t) || BUILTIN.contains(&t) || self.types.contains(t) || matches!(t, "struct" | "union" | "enum" | "typedef" | "__attribute__"))
    }

    /// `IDENT *... IDENT` followed by a declarator terminator: an unknown (e.g. header) type name.
    fn unknown_type_at(&self, i: usize) -> bool {
        if !self.ident(i) || KEYWORDS.contains(&self.t(i)) {
            return false;
        }
        let mut j = i + 1;
        while self.is(j, "*") {
            j += 1;
        }
        self.ident(j) && !KEYWORDS.contains(&self.t(j)) && (self.is(j + 1, ";") || self.is(j + 1, "=") || self.is(j + 1, ",") || self.is(j + 1, "[") || self.is(j + 1, "(") || self.is(j + 1, ")"))
    }

    fn skip_stmt(&self, mut i: usize) -> usize {
        while i < self.toks.len() {
            if self.is(i, ";") {
                return i + 1;
            }
            if self.is(i, "{") {
                let e = self.end_of(i) + 1;
                return if self.is(e, ";") { e + 1 } else { e };
            }
            if self.is(i, "(") || self.is(i, "[") {
                i = self.end_of(i);
            }
            i += 1;
        }
        i
    }

    fn top(&mut self) {
        let mut i = 0;
        while i < self.toks.len() {
            if self.is(i, ";") || self.is(i, "}") {
                i += 1;
            } else if self.t(i) == "extern" && self.toks.get(i + 1).is_some_and(|t| t.kind == TK::Str) && self.is(i + 2, "{") {
                i += 3;
            } else if let Some(n) = self.parse_decl(i, Ctx::Global) {
                i = n.max(i + 1);
            } else if self.ident(i) && self.is(i + 1, "(") && self.is(self.end_of(i + 1) + 1, "{") {
                // ISR(vect) { ... }: no symbol, but walk the body for locals.
                let open = self.end_of(i + 1) + 1;
                let close = self.end_of(open);
                self.walk_body(open, close);
                i = close + 1;
            } else {
                i = self.skip_stmt(i);
            }
        }
    }

    fn join(&self, from: usize, to: usize) -> String {
        let mut s = String::new();
        let mut prev = "";
        for k in from..to.min(self.toks.len()) {
            let t = self.t(k);
            let tight_before = matches!(t, ")" | "]" | "," | ";" | "[") || (t == "(" && self.toks.get(k - 1).is_some_and(|p| p.kind == TK::Ident) && !self.is_type_start(k - 1) && prev != "*");
            let tight_after = matches!(prev, "(" | "[" | "*");
            if !s.is_empty() && !tight_before && !tight_after {
                s.push(' ');
            }
            s.push_str(t);
            prev = t;
        }
        s
    }

    fn src_text(&self, from: usize, to: usize) -> String {
        if from >= to || to > self.toks.len() {
            return String::new();
        }
        let text = String::from_utf8_lossy(&self.plain[self.toks[from].s..self.toks[to - 1].e]);
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Parses `type declarator[, declarator]... (; | { body })` at `i`. `None` when `i` does not
    /// start a declaration (nothing is recorded then).
    fn parse_decl(&mut self, i: usize, ctx: Ctx) -> Option<usize> {
        let start = i;
        let mut j = i;
        let mut is_typedef = false;
        let mut seen = false;
        let mut words: Vec<String> = Vec::new();
        let mut anon_body = false;
        while j < self.toks.len() && self.ident(j) {
            let t = self.t(j).to_string();
            if t == "typedef" {
                is_typedef = true;
            } else if t == "__attribute__" || t == "__asm__" || t == "_Alignas" {
                if self.is(j + 1, "(") {
                    j = self.end_of(j + 1);
                }
            } else if STORAGE.contains(&t.as_str()) {
                words.push(t);
            } else if BUILTIN.contains(&t.as_str()) {
                words.push(t);
                seen = true;
            } else if matches!(t.as_str(), "struct" | "union" | "enum") {
                seen = true;
                let kw = t;
                let mut tag = String::new();
                let mut k = j + 1;
                if self.ident(k) && !self.is(k, "{") {
                    tag = self.t(k).to_string();
                    k += 1;
                }
                words.push(if tag.is_empty() { kw.clone() } else { format!("{kw} {tag}") });
                if self.is(k, "{") {
                    let close = self.end_of(k);
                    if !tag.is_empty() {
                        let d = self.docs.doc(self.line(start));
                        self.out.push(sym(&tag, "type", format!("{kw} {tag}"), None, self.line(j + 1), None, d));
                        self.types.insert(format!("{kw} {tag}"));
                    } else {
                        anon_body = true;
                    }
                    if kw == "enum" {
                        self.enum_body(k + 1, close, &tag);
                    }
                    j = close;
                }
                if !self.is(k, "{") {
                    j = k - 1;
                }
            } else if !seen && (self.types.contains(&t) || self.unknown_type_at(j)) {
                words.push(t);
                seen = true;
            } else {
                break;
            }
            j += 1;
        }
        if !seen {
            return None;
        }
        let ty = words.join(" ");
        let ty_for_typedef = if anon_body { format!("{ty} {{…}}") } else { ty.clone() };
        let decl_line = self.line(start);
        let mut first = true;
        loop {
            if self.is(j, ";") {
                return Some(j + 1);
            }
            // pointer stars
            let mut stars = String::new();
            while self.is(j, "*") {
                stars.push('*');
                j += 1;
                while matches!(self.t(j), "const" | "volatile" | "restrict") {
                    stars.push_str(&format!(" {} ", self.t(j)));
                    j += 1;
                }
            }
            let (name, name_idx, mut text, fnptr);
            if self.ident(j) {
                name = self.t(j).to_string();
                name_idx = j;
                text = format!("{stars}{name}");
                fnptr = false;
                j += 1;
            } else if self.is(j, "(") && self.is(j + 1, "*") && self.ident(j + 2) && self.is(j + 3, ")") {
                name = self.t(j + 2).to_string();
                name_idx = j + 2;
                text = format!("(*{name})");
                fnptr = true;
                j += 4;
            } else {
                return if first { None } else { Some(self.skip_stmt(j)) };
            }
            first = false;
            let mut params: Option<(usize, usize)> = None;
            loop {
                if self.is(j, "[") {
                    let e = self.end_of(j);
                    text.push_str(&format!("[{}]", self.join(j + 1, e)));
                    j = e + 1;
                } else if self.is(j, "(") {
                    let e = self.end_of(j);
                    if fnptr || params.is_some() {
                        text.push_str(&format!("({})", self.join(j + 1, e)));
                    } else {
                        params = Some((j, e));
                    }
                    j = e + 1;
                } else if self.t(j) == "__attribute__" && self.is(j + 1, "(") {
                    j = self.end_of(j + 1) + 1;
                } else if self.ident(j) && !self.is(j + 1, "(") && j + 1 < self.toks.len() && matches!(self.t(j + 1), "=" | ";" | "," | "{") && self.t(j).chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) {
                    j += 1; // PROGMEM-like attribute macro
                } else {
                    break;
                }
            }
            let line = self.toks[name_idx].line;
            let doc = {
                let d = self.docs.doc(decl_line);
                if d.is_empty() && line != decl_line { self.docs.doc(line) } else { d }
            };
            if is_typedef {
                self.types.insert(name.clone());
                let p = params.map(|(o, e)| format!("({})", self.join(o + 1, e))).unwrap_or_default();
                self.out.push(sym(&name, "type", format!("typedef {ty_for_typedef} {text}{p}"), None, line, None, doc));
            } else if let Some((po, pe)) = params {
                if ctx != Ctx::Global {
                    j = j.max(pe + 1);
                } else {
                    let plist = self.param_list(po, pe);
                    let sig = format!("{ty} {text}({})", plist.iter().map(|p| p.2.clone()).collect::<Vec<_>>().join(", "));
                    let is_def = self.is(j, "{");
                    self.out.push(sym(&name, "function", sig, None, line, None, doc));
                    if is_def {
                        let close = self.end_of(j);
                        let end = self.line(close);
                        for (pname, pline, ptext) in plist {
                            if let Some(pn) = pname {
                                self.out.push(sym(&pn, "parameter", ptext, None, pline, Some((line, end)), String::new()));
                            }
                        }
                        self.walk_body(j, close);
                        return Some(close + 1);
                    }
                    let idx = self.out.len() - 1;
                    self.protos.push((idx, name.clone()));
                }
            } else {
                // variable
                let mut detail = format!("{ty} {text}");
                let mut value = None;
                if self.is(j, "=") {
                    let (e, init) = self.initializer(j + 1);
                    if !init.is_empty() {
                        let shown = if init.chars().count() > 60 { format!("{}…", init.chars().take(59).collect::<String>()) } else { init.clone() };
                        detail.push_str(&format!(" = {shown}"));
                        if words.iter().any(|w| w == "const") {
                            value = eval_expr(&init, &self.consts, false);
                        }
                    }
                    j = e;
                }
                match ctx {
                    Ctx::Global => self.out.push(sym(&name, "global", detail, value, line, None, doc)),
                    Ctx::Local(end) => self.out.push(sym(&name, "local", detail, value, line, Some((decl_line.max(line).min(line), end)), doc)),
                }
            }
            if self.is(j, ",") {
                j += 1;
                continue;
            }
            if self.is(j, ";") {
                return Some(j + 1);
            }
            return Some(self.skip_stmt(j));
        }
    }

    /// Initializer starting at `i`: (index of the terminating `,`/`;`, collapsed text).
    fn initializer(&self, i: usize) -> (usize, String) {
        let mut j = i;
        while j < self.toks.len() && !self.is(j, ",") && !self.is(j, ";") {
            if self.is(j, "(") || self.is(j, "[") || self.is(j, "{") {
                j = self.end_of(j);
            }
            j += 1;
        }
        (j.min(self.toks.len()), self.src_text(i, j))
    }

    /// Parameters between `open` `(` and `close` `)`: (name, line, normalized text).
    fn param_list(&self, open: usize, close: usize) -> Vec<(Option<String>, u32, String)> {
        let mut res = Vec::new();
        let mut j = open + 1;
        let mut from = j;
        let flush = |this: &Self, from: usize, to: usize, res: &mut Vec<(Option<String>, u32, String)>| {
            if from >= to {
                return;
            }
            let text = this.join(from, to);
            if text == "void" || text == "..." {
                if text == "..." {
                    res.push((None, this.line(from), text));
                }
                return;
            }
            let mut name = None;
            let mut nl = this.line(from);
            if let Some(k) = (from..to.saturating_sub(2)).find(|&k| this.is(k, "(") && this.is(k + 1, "*") && this.ident(k + 2)) {
                name = Some(this.t(k + 2).to_string());
                nl = this.line(k + 2);
            } else {
                for k in (from..to).rev() {
                    if this.ident(k) {
                        let t = this.t(k);
                        if k > from && !BUILTIN.contains(&t) && !STORAGE.contains(&t) && !this.types.contains(t) && !matches!(t, "struct" | "union" | "enum") && !(k > from && matches!(this.t(k - 1), "struct" | "union" | "enum")) {
                            name = Some(t.to_string());
                            nl = this.line(k);
                        }
                        break;
                    }
                    if this.is(k, "]") {
                        continue;
                    }
                    if this.is(k, "[") || this.toks[k].kind == TK::Num {
                        continue;
                    }
                }
            }
            res.push((name, nl, text));
        };
        while j < close {
            if self.is(j, "(") || self.is(j, "[") || self.is(j, "{") {
                j = self.end_of(j);
            } else if self.is(j, ",") {
                flush(self, from, j, &mut res);
                from = j + 1;
            }
            j += 1;
        }
        flush(self, from, close, &mut res);
        res
    }

    fn enum_body(&mut self, from: usize, close: usize, tag: &str) {
        let mut next: Option<i64> = Some(0);
        let mut j = from;
        while j < close {
            if !self.ident(j) {
                j += 1;
                continue;
            }
            let name = self.t(j).to_string();
            let line = self.line(j);
            let mut k = j + 1;
            if self.is(k, "=") {
                let mut e = k + 1;
                while e < close && !self.is(e, ",") {
                    if self.is(e, "(") || self.is(e, "[") || self.is(e, "{") {
                        e = self.end_of(e);
                    }
                    e += 1;
                }
                let e = e.min(close);
                next = eval_expr(&self.src_text(k + 1, e), &self.consts, false);
                k = e;
            }
            let of = if tag.is_empty() { String::new() } else { format!(" (enum {tag})") };
            let detail = match next {
                Some(v) => format!("{name} = {v}{of}"),
                None => format!("{name}{of}"),
            };
            if let Some(v) = next {
                self.consts.insert(name.clone(), v);
            }
            let doc = self.docs.doc(line);
            self.out.push(sym(&name, "constant", detail, next, line, None, doc));
            next = next.map(|v| v + 1);
            while k < close && !self.is(k, ",") {
                k += 1;
            }
            j = k + 1;
        }
    }

    /// Function/block body between `open` `{` and `close` `}`: records locals.
    fn walk_body(&mut self, open: usize, close: usize) {
        let end_line = self.line(close);
        let mut j = open + 1;
        let mut stmt_start = true;
        while j < close {
            if self.is(j, "{") {
                let c = self.end_of(j);
                self.walk_body(j, c);
                j = c + 1;
                stmt_start = true;
            } else if self.t(j) == "for" && self.is(j + 1, "(") && (self.is_type_start(j + 2) || self.unknown_type_at(j + 2)) {
                let rp = self.end_of(j + 1);
                let stmt_end = if self.is(rp + 1, "{") {
                    self.line(self.end_of(rp + 1))
                } else {
                    let mut e = rp + 1;
                    while e < close && !self.is(e, ";") {
                        if self.is(e, "(") || self.is(e, "[") || self.is(e, "{") {
                            e = self.end_of(e);
                        }
                        e += 1;
                    }
                    self.line(e.min(close))
                };
                let for_line = self.line(j);
                match self.parse_decl(j + 2, Ctx::Local(stmt_end)) {
                    Some(n) => {
                        // pin the scope to start at the `for`
                        for s in self.out.iter_mut().rev().take_while(|s| s.kind == "local" && s.scope.is_some_and(|sc| sc.1 == stmt_end)) {
                            if let Some(sc) = &mut s.scope {
                                sc.0 = for_line.min(sc.0);
                            }
                        }
                        j = n.max(j + 1);
                        stmt_start = false;
                    }
                    None => j += 1,
                }
            } else if stmt_start && (self.is_type_start(j) || self.unknown_type_at(j)) {
                match self.parse_decl(j, Ctx::Local(end_line)) {
                    Some(n) => j = n.max(j + 1),
                    None => j += 1,
                }
                stmt_start = true;
            } else {
                stmt_start = self.is(j, ";") || self.is(j, "}") || self.t(j) == "else" || self.t(j) == "do";
                if self.is(j, "(") || self.is(j, "[") {
                    // keep statement-start state after a control header's `)`
                    let c = self.end_of(j);
                    let ctl = j > 0 && matches!(self.t(j - 1), "if" | "while" | "for" | "switch");
                    j = c + 1;
                    stmt_start = ctl;
                    continue;
                }
                j += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(v: &'a [SourceSymbol], name: &str) -> Vec<&'a SourceSymbol> {
        v.iter().filter(|s| s.name == name).collect()
    }

    #[test]
    fn asm_symbols() {
        let src = "\
; Number of LED blinks
.equ BLINKS = 4 * 2
.set cnt = BLINKS + 1
.def tmp = r16  ; scratch
.macro delay
    ldi r16, @0
    ldi r17, @1
.endmacro
.dseg
buf: .byte 4
.cseg
; entry point
start:
    ldi tmp, low(RAMEND)
loop: rjmp loop
";
        let v = source_symbols(src, "asm", "attiny10");
        let b = find(&v, "BLINKS")[0];
        assert_eq!((b.kind.as_str(), b.value, b.line), ("constant", Some(8), 2));
        assert_eq!(b.doc, "Number of LED blinks");
        assert_eq!(find(&v, "cnt")[0].value, Some(9));
        let t = find(&v, "tmp")[0];
        assert_eq!((t.kind.as_str(), t.detail.as_str(), t.value), ("register alias", ".def tmp = r16", Some(16)));
        assert_eq!(t.doc, "scratch");
        assert_eq!(find(&v, "delay")[0].detail, "macro delay @0, @1");
        let bf = find(&v, "buf")[0];
        assert_eq!(bf.kind, "variable");
        assert!(bf.detail.starts_with("4 bytes in SRAM"), "{}", bf.detail);
        assert_eq!(find(&v, "start")[0].doc, "entry point");
        assert!(find(&v, "start")[0].detail.contains("code"));
        let r = find(&v, "RAMEND")[0];
        assert_eq!(r.kind, "device constant");
        assert!(r.value.is_some());
        assert!(find(&v, "PORTB").is_empty());
    }

    #[test]
    fn c_functions_and_scopes() {
        let src = "\
#include <stdint.h>
#define LED_PIN 3
#define SQR(x) ((x)*(x))
typedef unsigned char byte_t;
enum Mode { OFF, ON = 5, BLINK };
/* Number of ticks. */
volatile uint8_t count = 0;
static const char *name = \"a{b\"; // not a brace
int i;

void blink(uint8_t times, uint16_t ms);

/// Blinks the LED.
/// Takes a while.
void blink(uint8_t times, uint16_t ms)
{
    int i = 0;
    for (int k = 0; k < times; k++) {
        i += k;
    }
    if (i) {
        byte_t inner = 1;
    }
}
ISR(TIMER0_OVF_vect) {
    uint8_t x = 1;
}
";
        let v = source_symbols(src, "c", "attiny10");
        let f = find(&v, "blink");
        assert_eq!(f.len(), 1, "prototype merged into definition");
        assert_eq!(f[0].detail, "void blink(uint8_t times, uint16_t ms)");
        assert_eq!(f[0].kind, "function");
        assert_eq!(f[0].doc, "Blinks the LED.\nTakes a while.");
        let p = find(&v, "times")[0];
        assert_eq!((p.kind.as_str(), p.scope), ("parameter", Some((15, 24))));
        let is = find(&v, "i");
        assert_eq!(is.len(), 2);
        assert_eq!(is[0].kind, "global");
        assert_eq!(is[1].kind, "local");
        assert_eq!(is[1].scope, Some((17, 24)));
        let k = find(&v, "k")[0];
        assert_eq!(k.scope, Some((18, 20)));
        assert_eq!(find(&v, "inner")[0].scope, Some((22, 23)));
        assert_eq!(find(&v, "x")[0].kind, "local");
        let c = find(&v, "count")[0];
        assert_eq!((c.kind.as_str(), c.detail.as_str(), c.doc.as_str()), ("global", "volatile uint8_t count = 0", "Number of ticks."));
        assert_eq!(find(&v, "name")[0].detail, "static const char *name = \"a{b\"");
        let m = find(&v, "LED_PIN")[0];
        assert_eq!((m.kind.as_str(), m.value), ("macro", Some(3)));
        assert_eq!(find(&v, "SQR")[0].detail, "#define SQR(x) ((x)*(x))");
        assert_eq!(find(&v, "byte_t")[0].kind, "type");
        assert_eq!(find(&v, "BLINK")[0].value, Some(6));
        assert_eq!(find(&v, "ON")[0].value, Some(5));
        assert_eq!(find(&v, "Mode")[0].detail, "enum Mode");
    }

    #[test]
    fn c_declarator_forms() {
        let src = "const uint8_t table[4] PROGMEM = {1, 2, 3, 4};\nchar *names[2], *last;\nstatic char *dup(const char *s, int n) { return 0; }\nvoid (*cb)(int);\nstruct Pt { int x; int y; } origin;\nFILE *log_file;\n";
        let v = source_symbols(src, "c", "attiny10");
        assert_eq!(find(&v, "table")[0].detail, "const uint8_t table[4] = {1, 2, 3, 4}");
        assert_eq!(find(&v, "names")[0].detail, "char *names[2]");
        assert_eq!(find(&v, "last")[0].detail, "char *last");
        assert_eq!(find(&v, "dup")[0].detail, "static char *dup(const char *s, int n)");
        assert_eq!(find(&v, "n")[0].kind, "parameter");
        assert_eq!(find(&v, "cb")[0].kind, "global");
        assert_eq!(find(&v, "Pt")[0].kind, "type");
        assert_eq!(find(&v, "origin")[0].detail, "struct Pt origin");
        assert_eq!(find(&v, "log_file")[0].detail, "FILE *log_file");
        assert!(find(&v, "x").is_empty());
    }
}
