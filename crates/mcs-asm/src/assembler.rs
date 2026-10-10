//! Two-pass AVR assembler (Atmel/Microchip avrasm2 dialect plus common GNU conveniences).
//!
//! Pass 1 walks the source once (expanding includes, macros and conditionals), defines labels,
//! selects the instruction form for every mnemonic (operand shapes + device features decide the
//! size, so addresses are final after this pass) and records a flat statement list.
//! Pass 2 only evaluates operand/data expressions (all labels are known by then), encodes, and
//! writes the flash/EEPROM images, line table and listing.

use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::devices;
use mcs_core::avr::isa::{encode, feature, insns, operand_range, InsnDef, OperandKind, BRANCH_ALIASES, FLAG_ALIASES};
use mcs_core::program::{
    Diagnostic, LineEntry, LoadedProgram, ProgramFormat, ProgramSymbol, Severity, SymbolKind, SymbolSpace,
};

use crate::expr::{evaluate, EvalMode, EvalResult, ExprEnv, ExprError, MAX_EXPR_DEPTH};
use crate::incgen::{def_include_symbols, DefSymbols};
use crate::lexer::{tokenize, SrcLine, TStr, TokKind, Token};
use crate::util::{basename, hex, to_fixed1, utf16_len, FxMap, FxSet};
use crate::{AssembleOptions, AssembleResult};

// ---------------------------------------------------------------------------------------------
// Static tables

const CSEG: usize = 0;
const DSEG: usize = 1;
const ESEG: usize = 2;
const SEG_NAMES: [&str; 3] = ["code", "data", "EEPROM"];
const SEG_CHARS: [char; 3] = ['C', 'D', 'E'];

const MAX_INCLUDE_DEPTH: usize = 32;
const MAX_MACRO_DEPTH: u32 = 64;
/// Guards against exponential macro recursion (bounds time and memory).
const MAX_EXPANDED_LINES: u64 = 200_000;
/// Listing column where the source text starts.
const LIST_PAD: usize = 30;

struct Alias {
    base: &'static str,
    /// `lsl Rd` -> `add Rd, Rd`.
    dup: bool,
    /// Hidden leading operand (SREG bit of branch/flag aliases).
    pre: Option<i64>,
    /// Hidden trailing operand (`ser Rd` -> `ldi Rd, 0xFF`).
    post: Option<i64>,
    /// `cbr Rd, K` -> `andi Rd, ~K`.
    cbr: bool,
}

struct Tables {
    by_name: FxMap<&'static str, Vec<&'static InsnDef>>,
    aliases: FxMap<&'static str, Alias>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut by_name: FxMap<&'static str, Vec<&'static InsnDef>> = FxMap::default();
        for d in insns() {
            by_name.entry(d.name).or_default().push(d);
        }
        let mut aliases: FxMap<&'static str, Alias> = FxMap::default();
        let plain = |base| Alias { base, dup: false, pre: None, post: None, cbr: false };
        for (name, base) in [("lsl", "add"), ("rol", "adc"), ("tst", "and"), ("clr", "eor")] {
            aliases.insert(name, Alias { dup: true, ..plain(base) });
        }
        aliases.insert("ser", Alias { post: Some(0xff), ..plain("ldi") });
        aliases.insert("sbr", plain("ori"));
        aliases.insert("cbr", Alias { cbr: true, ..plain("andi") });
        for &(name, bit, set) in BRANCH_ALIASES {
            aliases.insert(name, Alias { pre: Some(bit as i64), ..plain(if set { "brbs" } else { "brbc" }) });
        }
        for &(name, bit, set) in FLAG_ALIASES {
            aliases.insert(name, Alias { pre: Some(bit as i64), ..plain(if set { "bset" } else { "bclr" }) });
        }
        Tables { by_name, aliases }
    })
}

/// Register number for `r0`..`r31` (lowercase), else -1.
pub(crate) fn reg_index(lc: &str) -> i64 {
    let b = lc.as_bytes();
    let n = b.len();
    if !(2..=3).contains(&n) || b[0] != b'r' {
        return -1;
    }
    let a = b[1].wrapping_sub(b'0');
    if a > 9 {
        return -1;
    }
    if n == 2 {
        return a as i64;
    }
    let c = b[2].wrapping_sub(b'0');
    if c > 9 || a == 0 {
        return -1;
    }
    let v = (a * 10 + c) as i64;
    if v <= 31 {
        v
    } else {
        -1
    }
}

fn is_pair_kind(k: OperandKind) -> bool {
    matches!(k, OperandKind::RdW | OperandKind::RrW | OperandKind::RdP)
}

fn is_cond_dir(d: &str) -> bool {
    matches!(d, "if" | "ifdef" | "ifndef" | "elif" | "elseif" | "else" | "endif")
}

/// avrasm2 device include file name pattern: `(tn|m|usb|can|pwm|x)\w+def.inc` (case-insensitive).
fn is_device_include_name(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix("def.inc") else { return false };
    ["tn", "m", "usb", "can", "pwm", "x"].iter().any(|p| {
        stem.strip_prefix(p).is_some_and(|r| !r.is_empty() && r.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
    })
}

fn kind_label(kind: OperandKind) -> &'static str {
    use OperandKind as K;
    match kind {
        K::A5 | K::A6 => "I/O address",
        K::B => "bit number",
        K::S => "SREG bit number",
        K::K16 | K::K7rc => "data address",
        K::K22 => "address",
        K::YQ | K::ZQ => "displacement",
        _ => "constant",
    }
}

/// Display order of operand descriptions in "expected ..." messages.
const KIND_ORDER: [&str; 14] =
    ["a register", "a constant expression", "a label or address", "X", "X+", "-X", "Y", "Y+", "-Y", "Y+q", "Z", "Z+", "-Z", "Z+q"];

fn describe_kind(kind: OperandKind) -> &'static str {
    use OperandKind as K;
    if kind.is_register() {
        "a register"
    } else if kind.is_literal() || matches!(kind, K::YQ | K::ZQ) {
        kind.literal_text()
    } else if matches!(kind, K::K7 | K::K12 | K::K22) {
        "a label or address"
    } else {
        "a constant expression"
    }
}

fn supported_devices() -> String {
    devices::list().iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------------------------
// Internal data structures

/// A token range holding an expression (evaluated in pass 2 or on demand).
#[derive(Clone, Debug)]
struct ExprRef {
    toks: Rc<[Token]>,
    s: usize,
    e: usize,
    col: u32,
}

#[derive(Clone, Debug)]
enum Opnd {
    Reg { v: i64, pair: bool, col: u32 },
    Ptr { p: OperandKind, col: u32 },
    Disp { z: bool, x: Option<ExprRef>, col: u32 },
    Expr { x: ExprRef, col: u32 },
    Const { v: i64, col: u32 },
}

impl Opnd {
    fn col(&self) -> u32 {
        match self {
            Opnd::Reg { col, .. } | Opnd::Ptr { col, .. } | Opnd::Disp { col, .. } | Opnd::Expr { col, .. } | Opnd::Const { col, .. } => *col,
        }
    }
}

fn shape_ok(kind: OperandKind, o: &Opnd) -> bool {
    use OperandKind as K;
    match o {
        Opnd::Reg { pair, .. } => kind.is_register() && (!pair || is_pair_kind(kind)),
        Opnd::Ptr { p, .. } => kind == *p,
        Opnd::Disp { z, .. } => kind == if *z { K::ZQ } else { K::YQ },
        _ => !kind.is_register() && !kind.is_literal() && !matches!(kind, K::YQ | K::ZQ),
    }
}

#[derive(Debug)]
struct Macro {
    name: TStr,
    file: Rc<str>,
    body: Rc<[SrcLine]>,
    /// Lowercase names of labels defined in the body (local to each expansion).
    locals: FxSet<TStr>,
}

/// Macro being recorded (between `.macro` and `.endm`).
struct Recording {
    name: TStr,
    file: Rc<str>,
    body: Vec<SrcLine>,
    rec: usize,
    discard: bool,
}

/// One processed source line (diagnostic location + listing row).
struct LineRec {
    /// Listing text (macro lines show their arguments substituted).
    text: Rc<str>,
    /// Real file/line: for macro expansions, the outermost invocation site.
    file: Rc<str>,
    line: u32,
    /// Invocation column for macro-expanded lines (token columns refer to the macro body).
    site_col: u32,
    depth: u32,
    mac: Option<Rc<Macro>>,
    /// Line within the macro body (depth > 0).
    mline: u32,
    stmt: Option<usize>,
    list: bool,
}

enum Val {
    Num(i64),
    Expr(ExprRef),
}

enum DataItem {
    Expr(ExprRef),
    Bytes(Vec<u8>),
}

enum StmtKind {
    Insn {
        def: &'static InsnDef,
        /// One entry per non-literal operand: resolved value or expression for pass 2.
        vals: Vec<Val>,
        cbr: bool,
        /// Pass-1 error already reported: reserve space but do not encode.
        bad: bool,
    },
    Data {
        unit: u8,
        items: Vec<DataItem>,
    },
    Set {
        sym: usize,
        x: Option<ExprRef>,
        value: i64,
    },
}

struct Stmt {
    rec: usize,
    seg: usize,
    /// Word address (code) or byte address (data/EEPROM).
    addr: i64,
    /// Words (code) or bytes (EEPROM).
    size: i64,
    /// Code location counter when the statement was assembled (value of `PC`).
    pc: i64,
    kind: StmtKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SymKind {
    Label,
    Equ,
    Set,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SymState {
    Ok,
    /// `.equ` with a forward reference, resolved on demand.
    Lazy,
    /// `.set` whose value is only known in pass 2.
    Pending,
    /// Definition failed (already reported).
    Error,
}

struct Sym {
    name: TStr,
    kind: SymKind,
    value: i64,
    state: SymState,
    seg: usize,
    size: i64,
    rec: usize,
    lazy: Option<ExprRef>,
    lazy_pc: i64,
    resolving: bool,
    /// Macro-local label (mangled name), not exported.
    hidden: bool,
}

struct Frame {
    file: Rc<str>,
    site_file: Rc<str>,
    site_line: u32,
    site_col: u32,
    depth: u32,
    mac: Option<Rc<Macro>>,
}

struct Cond {
    parent: bool,
    taken: bool,
    active: bool,
    in_else: bool,
    rec: usize,
}

// ---------------------------------------------------------------------------------------------

pub(crate) struct Assembler<'a> {
    main_file: Rc<str>,
    requested_device: &'a str,
    includes: &'a std::collections::HashMap<String, String>,
    /// Extra (non-registry) device that `.device` may select (custom/test devices).
    custom: Option<&'a AvrDeviceSpec>,
    diags: Vec<Diagnostic>,
    device: &'a AvrDeviceSpec,
    features: u32,
    dev_syms: Arc<DefSymbols>,
    device_explicit: bool,
    default_invalid: bool,
    code_emitted: bool,

    syms: Vec<Sym>,
    sym_idx: FxMap<TStr, usize>,
    reg_defs: FxMap<TStr, i64>,
    macros: FxMap<TStr, Rc<Macro>>,
    stmts: Vec<Stmt>,
    recs: Vec<LineRec>,
    file_cache: FxMap<String, Rc<[SrcLine]>>,
    include_stack: Vec<TStr>,

    seg: usize,
    loc: [i64; 3],
    dseg_touched: bool,
    dseg_reserved: i64,
    dseg_overflow: bool,
    eseg_overflow: bool,
    conds: Vec<Cond>,
    recording: Option<Recording>,
    list_on: bool,
    exit_file: bool,
    expansion_id: u64,
    expanded_lines: u64,
    /// Data/EEPROM labels waiting for a `.byte` to give them a size.
    pending: Vec<usize>,
    rec: usize,

    // Pass-2 output
    flash: Vec<u8>,
    owner: Vec<u32>,
    eeprom: Option<Vec<u8>>,
    eeprom_written: bool,
    flash_used: u32,
    overflow_reported: bool,
    line_table: Vec<LineEntry>,
    file_idx: FxMap<Rc<str>, u32>,
    files: Vec<String>,
}

impl<'a> Assembler<'a> {
    /// `device` is the default device; `default_invalid` when the requested id was unknown;
    /// `custom` is an extra device spec that `.device` may also select.
    pub(crate) fn new(
        opts: &AssembleOptions<'a>,
        device: &'a AvrDeviceSpec,
        default_invalid: bool,
        custom: Option<&'a AvrDeviceSpec>,
    ) -> Self {
        let main_file: Rc<str> = Rc::from(opts.file_name);
        let mut asm = Assembler {
            main_file: main_file.clone(),
            requested_device: opts.device_id,
            includes: opts.includes,
            custom,
            diags: Vec::new(),
            device,
            features: device.features,
            dev_syms: def_include_symbols(device),
            device_explicit: false,
            default_invalid,
            code_emitted: false,
            syms: Vec::new(),
            sym_idx: FxMap::default(),
            reg_defs: FxMap::default(),
            macros: FxMap::default(),
            stmts: Vec::new(),
            recs: Vec::new(),
            file_cache: FxMap::default(),
            include_stack: Vec::new(),
            seg: CSEG,
            loc: [0, device.sram_start as i64, 0],
            dseg_touched: false,
            dseg_reserved: 0,
            dseg_overflow: false,
            eseg_overflow: false,
            conds: Vec::new(),
            recording: None,
            list_on: true,
            exit_file: false,
            expansion_id: 0,
            expanded_lines: 0,
            pending: Vec::new(),
            rec: 0,
            flash: Vec::new(),
            owner: Vec::new(),
            eeprom: None,
            eeprom_written: false,
            flash_used: 0,
            overflow_reported: false,
            line_table: Vec::new(),
            file_idx: FxMap::default(),
            files: Vec::new(),
        };
        // Record 0: blank location used before the first line is processed (never listed).
        asm.recs.push(LineRec {
            text: Rc::from(""),
            file: main_file.clone(),
            line: 0,
            site_col: 0,
            depth: 0,
            mac: None,
            mline: 0,
            stmt: None,
            list: false,
        });
        asm.file_index(&main_file);
        asm
    }

    // ------------------------------------------------------------------------------- driver

    pub(crate) fn run(&mut self, source: &str) {
        let lines: Rc<[SrcLine]> = Rc::from(tokenize(source));
        self.recs.reserve(lines.len());
        self.stmts.reserve(lines.len());
        let main = self.main_file.clone();
        self.include_stack.push(TStr::Shared(main.clone()));
        self.process_file(main, lines);
        if let Some(r) = self.recording.take() {
            self.report(Severity::Error, r.rec, 0, format!("missing .endm for macro '{}'", r.name));
        }
        if self.default_invalid && !self.device_explicit {
            self.diags.push(Diagnostic::error(
                format!("unknown device '{}' (using {})", self.requested_device, self.device.name),
                &*self.main_file,
                0,
                0,
            ));
        }
        self.pass2();
    }

    /// Records an unexpected internal failure as a diagnostic (the public API never fails).
    pub(crate) fn internal_error(&mut self, msg: &str) {
        self.diags.push(Diagnostic::error(format!("internal assembler error: {msg}"), &*self.main_file, 0, 0));
    }

    pub(crate) fn finish(mut self) -> AssembleResult {
        let dev = self.device;
        if self.flash.len() != dev.flash_size as usize {
            self.flash = vec![0xff; dev.flash_size as usize];
        }
        let diagnostics = self.sorted_diagnostics();
        let listing = self.build_listing(&diagnostics);
        let symbols = self.export_symbols();
        let eeprom = if self.eeprom_written { self.eeprom.take() } else { None };
        let program = LoadedProgram {
            format: ProgramFormat::Asm,
            flash: std::mem::take(&mut self.flash),
            flash_used: self.flash_used,
            flash_base: 0,
            eeprom,
            fuses: None,
            lock: None,
            segments: Vec::new(),
            entry: 0,
            symbols,
            files: std::mem::take(&mut self.files),
            lines: std::mem::take(&mut self.line_table),
            device: Some(dev.id.clone()),
            diagnostics: diagnostics.clone(),
        };
        AssembleResult {
            ok: !diagnostics.iter().any(|d| d.severity == Severity::Error),
            program,
            device_id: dev.id.clone(),
            listing,
            diagnostics,
        }
    }

    // ------------------------------------------------------------------------------- diagnostics

    fn report(&mut self, sev: Severity, rec: usize, col: u32, msg: String) {
        let Some(r) = self.recs.get(rec) else {
            self.diags.push(Diagnostic::new(sev, msg, &*self.main_file, 0, col));
            return;
        };
        let (message, column) = match (&r.mac, r.depth > 0) {
            (Some(m), true) => (format!("{msg} (in macro '{}', {}:{})", m.name, m.file, r.mline), r.site_col),
            _ => (msg, col),
        };
        self.diags.push(Diagnostic { severity: sev, message, file: r.file.to_string(), line: r.line, column });
    }

    fn error(&mut self, col: u32, msg: impl Into<String>) {
        self.report(Severity::Error, self.rec, col, msg.into());
    }

    fn warning(&mut self, col: u32, msg: impl Into<String>) {
        self.report(Severity::Warning, self.rec, col, msg.into());
    }

    fn sorted_diagnostics(&mut self) -> Vec<Diagnostic> {
        let main = self.main_file.clone();
        let mut d = std::mem::take(&mut self.diags);
        // Stable: equal keys keep their report order.
        d.sort_by(|a, b| {
            (a.file != *main)
                .cmp(&(b.file != *main))
                .then_with(|| a.file.cmp(&b.file))
                .then(a.line.cmp(&b.line))
        });
        d
    }

    // ------------------------------------------------------------------------------- pass 1

    #[inline]
    fn active(&self) -> bool {
        self.conds.last().is_none_or(|c| c.active)
    }

    fn process_file(&mut self, name: Rc<str>, lines: Rc<[SrcLine]>) {
        let cond_base = self.conds.len();
        let frame = Frame { file: name.clone(), site_file: name, site_line: 0, site_col: 0, depth: 0, mac: None };
        for line in lines.iter() {
            self.process_line(line, line.toks.clone(), &frame, line.text.clone());
            if self.exit_file {
                break;
            }
        }
        self.exit_file = false;
        self.close_conds(cond_base, "");
    }

    fn close_conds(&mut self, base: usize, location: &str) {
        while self.conds.len() > base {
            if let Some(c) = self.conds.pop() {
                self.report(Severity::Error, c.rec, 0, format!("missing .endif{location}"));
            }
        }
    }

    fn process_line(&mut self, line: &SrcLine, toks: Rc<[Token]>, fr: &Frame, text: Rc<str>) {
        let rec = if fr.depth == 0 {
            LineRec { text, file: fr.file.clone(), line: line.no, site_col: 0, depth: 0, mac: None, mline: 0, stmt: None, list: false }
        } else {
            LineRec {
                text,
                file: fr.site_file.clone(),
                line: fr.site_line,
                site_col: fr.site_col,
                depth: fr.depth,
                mac: fr.mac.clone(),
                mline: line.no,
                stmt: None,
                list: false,
            }
        };
        let ri = self.recs.len();
        self.recs.push(rec);
        self.rec = ri;
        let n = toks.len();
        let first_dir: &str = match toks.first() {
            Some(t) if t.k == TokKind::Dir => &t.lc,
            _ => "",
        };

        if self.recording.is_some() {
            self.recs[ri].list = self.list_on;
            if first_dir == "endm" || first_dir == "endmacro" {
                self.end_macro();
            } else {
                if first_dir == "macro" {
                    self.error(toks[0].col, "nested macro definitions are not supported");
                }
                if let Some(r) = self.recording.as_mut() {
                    let mut l = line.clone();
                    l.toks = toks.clone();
                    r.body.push(l);
                }
            }
            return;
        }
        if is_cond_dir(first_dir) {
            let before = self.active();
            self.conditional(first_dir, &toks);
            let list = self.list_on && (before || self.active());
            self.recs[ri].list = list;
            return;
        }
        if !self.active() {
            return;
        }
        self.recs[ri].list = self.list_on;
        if let Some(err) = &line.err {
            self.error(line.err_col, err.to_string());
        }

        let mut i = 0;
        while i + 1 < n && toks[i].k == TokKind::Id && toks[i + 1].is_op(":") {
            self.define_label(&toks[i]);
            i += 2;
        }
        if i >= n {
            return;
        }
        let t = &toks[i];
        if !(t.k == TokKind::Dir && &*t.lc == "byte" && self.seg != CSEG) {
            self.pending.clear();
        }
        match t.k {
            TokKind::Dir => self.directive(&toks, i),
            TokKind::Id => self.instruction(&toks, i, fr),
            _ => {
                let msg = format!("unexpected '{}'", t.text());
                self.error(t.col, msg);
            }
        }
    }

    // ---- symbols

    fn new_sym(&mut self, tok: &Token, kind: SymKind) -> usize {
        let idx = self.syms.len();
        self.syms.push(Sym {
            name: tok.s.clone(),
            kind,
            value: 0,
            state: SymState::Ok,
            seg: self.seg,
            size: 0,
            rec: self.rec,
            lazy: None,
            lazy_pc: 0,
            resolving: false,
            hidden: tok.lc.contains('#'),
        });
        self.sym_idx.insert(tok.lc.clone(), idx);
        idx
    }

    fn already_defined(&self, name: &str, idx: usize) -> String {
        let r = &self.recs[self.syms[idx].rec];
        format!("symbol '{name}' is already defined ({}:{})", r.file, r.line)
    }

    fn check_new_name(&mut self, tok: &Token) -> bool {
        if reg_index(&tok.lc) >= 0 || &*tok.lc == "pc" {
            self.error(tok.col, format!("'{}' is a reserved name", tok.s));
            return false;
        }
        if let Some(&prev) = self.sym_idx.get(&*tok.lc) {
            let msg = self.already_defined(&tok.s, prev);
            self.error(tok.col, msg);
            return false;
        }
        true
    }

    fn define_label(&mut self, tok: &Token) {
        if !self.check_new_name(tok) {
            return;
        }
        let idx = self.new_sym(tok, SymKind::Label);
        self.syms[idx].value = self.loc[self.seg];
        if self.seg != CSEG {
            self.pending.push(idx);
        }
    }

    /// Resolves a forward-referencing `.equ` on demand (with cycle detection).
    fn resolve_lazy(&mut self, idx: usize, col: u32, strict: bool, depth: u32) -> Result<Option<i64>, ExprError> {
        let sym = &self.syms[idx];
        if sym.resolving {
            return Err(ExprError::new(format!("circular definition of '{}'", sym.name), col));
        }
        let Some(x) = sym.lazy.clone() else { return Ok(None) };
        let mode = EvalMode { pc: sym.lazy_pc, strict, dot_bytes: false, depth: depth + 1 };
        self.syms[idx].resolving = true;
        let res = evaluate(x.toks.get(x.s..x.e).unwrap_or(&[]), self, mode, x.col);
        self.syms[idx].resolving = false;
        match res {
            Ok(r) => {
                if r.unresolved.is_some() {
                    return Ok(None);
                }
                let sym = &mut self.syms[idx];
                sym.state = SymState::Ok;
                sym.value = r.value;
                sym.lazy = None;
                Ok(Some(r.value))
            }
            Err(e) => {
                if !strict && !e.silent {
                    return Ok(None); // retried (and reported) in pass 2
                }
                if !e.silent {
                    let rec = self.syms[idx].rec;
                    self.report(Severity::Error, rec, e.col, e.msg);
                }
                self.syms[idx].state = SymState::Error;
                Err(ExprError::silent(col))
            }
        }
    }

    /// Evaluates an expression, reporting errors at `rec`; returns None on error.
    fn try_eval(&mut self, x: &ExprRef, pc: i64, strict: bool, dot_bytes: bool, rec: usize) -> Option<EvalResult> {
        let toks = x.toks.clone();
        let slice = toks.get(x.s..x.e).unwrap_or(&[]);
        match evaluate(slice, self, EvalMode { pc, strict, dot_bytes, depth: 0 }, x.col) {
            Ok(r) => Some(r),
            Err(e) => {
                if !e.silent {
                    self.report(Severity::Error, rec, e.col, e.msg);
                }
                None
            }
        }
    }

    /// Pass-1 evaluation where forward references are not allowed (.org, .byte, .if ...).
    fn eval_now(&mut self, x: &ExprRef, what: &str) -> Option<i64> {
        let r = self.try_eval(x, self.loc[CSEG], false, false, self.rec)?;
        if let Some((name, col)) = r.unresolved {
            self.error(col, format!("undefined symbol '{name}' (forward references are not allowed in {what})"));
            return None;
        }
        Some(r.value)
    }

    fn reg_number(&self, lc: &str) -> i64 {
        let r = reg_index(lc);
        if r >= 0 {
            return r;
        }
        if let Some(&v) = self.reg_defs.get(lc) {
            return v;
        }
        self.dev_syms.defs.get(lc).copied().unwrap_or(-1)
    }

    // ---- helpers

    fn mk_ref(toks: &Rc<[Token]>, s: usize, e: usize, fallback_col: u32) -> ExprRef {
        let e = e.min(toks.len());
        let s = s.min(e);
        let col = if s < e { toks[s].col } else { fallback_col };
        ExprRef { toks: toks.clone(), s, e, col }
    }

    /// Splits `toks[i..]` at top-level commas into [start, end) ranges.
    fn split(toks: &[Token], i: usize) -> Vec<(usize, usize)> {
        let n = toks.len();
        let mut out = Vec::new();
        if i >= n {
            return out;
        }
        let mut depth = 0i64;
        let mut start = i;
        for (j, t) in toks.iter().enumerate().skip(i) {
            if t.k != TokKind::Op {
                continue;
            }
            match &*t.s {
                "(" => depth += 1,
                ")" => depth -= 1,
                "," if depth <= 0 => {
                    out.push((start, j));
                    start = j + 1;
                }
                _ => {}
            }
        }
        out.push((start, n));
        out
    }

    fn expect_end(&mut self, toks: &[Token], i: usize) {
        if let Some(t) = toks.get(i) {
            let msg = format!("unexpected '{}'", t.text());
            self.error(t.col, msg);
        }
    }

    fn file_index(&mut self, name: &Rc<str>) -> u32 {
        if let Some(&i) = self.file_idx.get(name) {
            return i;
        }
        let i = self.files.len() as u32;
        self.files.push(name.to_string());
        self.file_idx.insert(name.clone(), i);
        i
    }

    fn find_device(&self, id: &str) -> Option<&'a AvrDeviceSpec> {
        self.custom.filter(|c| c.id.eq_ignore_ascii_case(id)).or_else(|| devices::get(id))
    }

    #[inline]
    fn advance(&mut self, seg: usize, n: i64) {
        self.loc[seg] = self.loc[seg].saturating_add(n);
    }

    // ---- directives

    fn directive(&mut self, toks: &Rc<[Token]>, di: usize) {
        let t = &toks[di];
        let i = di + 1;
        let nt = toks.get(i);
        let nt_col = nt.map_or(t.col, |x| x.col);
        match &*t.lc {
            "equ" => self.equ_set(toks, i, false),
            "set" => self.equ_set(toks, i, true),
            "def" => self.def_reg(toks, i),
            "undef" => {
                match nt {
                    Some(nt) if nt.k == TokKind::Id => {
                        if self.reg_defs.remove(&*nt.lc).is_none() {
                            self.warning(nt.col, format!("register alias '{}' is not defined", nt.s));
                        }
                    }
                    _ => self.error(nt_col, "expected a register alias name"),
                }
                self.expect_end(toks, i + 1);
            }
            "include" => match nt {
                Some(nt) if nt.k == TokKind::Str => self.include(&nt.s, nt.col),
                _ => self.error(nt_col, "expected a quoted file name"),
            },
            "device" => {
                let Some(nt) = nt.filter(|x| x.k == TokKind::Id) else {
                    self.error(nt_col, "expected a device name");
                    return;
                };
                match self.find_device(&nt.s) {
                    None => self.error(nt.col, format!("unknown device '{}' (supported: {})", nt.s, supported_devices())),
                    Some(spec) => self.select_device(spec, nt.col),
                }
                self.expect_end(toks, i + 1);
            }
            "org" => {
                let x = Self::mk_ref(toks, i, toks.len(), t.col);
                let Some(v) = self.eval_now(&x, ".org") else { return };
                if v < 0 {
                    self.error(t.col, format!(".org address {v} is negative"));
                } else {
                    self.loc[self.seg] = v;
                    if self.seg == DSEG {
                        self.dseg_touched = true;
                    }
                }
            }
            "cseg" => {
                self.seg = CSEG;
                self.expect_end(toks, i);
            }
            "dseg" => {
                self.seg = DSEG;
                self.dseg_touched = true;
                self.expect_end(toks, i);
            }
            "eseg" => {
                self.seg = ESEG;
                self.expect_end(toks, i);
            }
            "db" => self.data(1, toks, di),
            "dw" | "word" => self.data(2, toks, di),
            "dd" => self.data(4, toks, di),
            "dq" => self.data(8, toks, di),
            "byte" => {
                if self.seg == CSEG {
                    self.data(1, toks, di); // GNU style `.byte` emits data
                } else {
                    self.reserve(toks, di);
                }
            }
            "macro" => self.begin_macro(toks, di),
            "endm" | "endmacro" => self.error(t.col, format!(".{} without .macro", t.s)),
            "error" | "warning" | "message" => match nt {
                Some(nt) if nt.k == TokKind::Str => {
                    let sev = match &*t.lc {
                        "error" => Severity::Error,
                        "warning" => Severity::Warning,
                        _ => Severity::Info,
                    };
                    self.report(sev, self.rec, t.col, nt.s.to_string());
                }
                _ => self.error(nt_col, format!(".{} expects a quoted message", t.s)),
            },
            "exit" => {
                if i < toks.len() {
                    let x = Self::mk_ref(toks, i, toks.len(), t.col);
                    match self.eval_now(&x, ".exit") {
                        None | Some(0) => return,
                        Some(_) => {}
                    }
                }
                self.exit_file = true;
            }
            "list" => self.list_on = true,
            "nolist" => self.list_on = false,
            "listmac" | "global" | "globl" | "overlap" | "nooverlap" => {}
            other => {
                if is_cond_dir(other) {
                    self.error(t.col, format!("a label is not allowed before .{}", t.s));
                } else {
                    self.error(t.col, format!("unknown directive '.{}'", t.s));
                }
            }
        }
    }

    fn equ_set(&mut self, toks: &Rc<[Token]>, i: usize, is_set: bool) {
        let dir = if is_set { ".set" } else { ".equ" };
        let Some(nt) = toks.get(i).filter(|t| t.k == TokKind::Id) else {
            let col = toks.get(i).map_or(0, |t| t.col);
            self.error(col, format!("{dir}: expected a symbol name"));
            return;
        };
        let eq = toks.get(i + 1);
        let Some(eq) = eq.filter(|e| e.is_op("=") || e.is_op(",")) else {
            let col = eq.map_or_else(|| nt.col.saturating_add(utf16_len(&nt.s)), |e| e.col);
            self.error(col, format!("{dir}: expected '=' after '{}'", nt.s));
            return;
        };
        let existing = self.sym_idx.get(&*nt.lc).copied();
        match existing {
            Some(idx) if !(is_set && self.syms[idx].kind == SymKind::Set) => {
                let msg = self.already_defined(&nt.s, idx);
                self.error(nt.col, msg);
                return;
            }
            None if !self.check_new_name(nt) => return,
            _ => {}
        }
        let x = Self::mk_ref(toks, i + 2, toks.len(), eq.col.saturating_add(1));
        let pc = self.loc[CSEG];
        let r = self.try_eval(&x, pc, false, false, self.rec);
        let idx = match existing {
            Some(idx) => idx,
            None => self.new_sym(nt, if is_set { SymKind::Set } else { SymKind::Equ }),
        };
        let Some(r) = r else {
            self.syms[idx].state = SymState::Error;
            return;
        };
        let unresolved = r.unresolved.is_some();
        if is_set {
            let rec = self.rec;
            let sym = &mut self.syms[idx];
            sym.state = if unresolved { SymState::Pending } else { SymState::Ok };
            sym.value = r.value;
            sym.rec = rec;
            self.stmts.push(Stmt {
                rec,
                seg: self.seg,
                addr: 0,
                size: 0,
                pc,
                kind: StmtKind::Set { sym: idx, x: unresolved.then_some(x), value: r.value },
            });
        } else if unresolved {
            let sym = &mut self.syms[idx];
            sym.state = SymState::Lazy;
            sym.lazy = Some(x);
            sym.lazy_pc = pc;
        } else {
            self.syms[idx].value = r.value;
        }
    }

    fn def_reg(&mut self, toks: &Rc<[Token]>, i: usize) {
        let Some(nt) = toks.get(i).filter(|t| t.k == TokKind::Id) else {
            let col = toks.get(i).map_or(0, |t| t.col);
            self.error(col, ".def: expected an alias name");
            return;
        };
        let eq = toks.get(i + 1);
        let Some(eq) = eq.filter(|e| e.is_op("=") || e.is_op(",")) else {
            let col = eq.map_or_else(|| nt.col.saturating_add(utf16_len(&nt.s)), |e| e.col);
            self.error(col, format!(".def: expected '=' after '{}'", nt.s));
            return;
        };
        let rt = toks.get(i + 2);
        let r = match rt {
            Some(rt) if rt.k == TokKind::Id => self.reg_number(&rt.lc),
            _ => -1,
        };
        if r < 0 {
            self.error(rt.map_or(eq.col.saturating_add(1), |t| t.col), ".def: expected a register (r0..r31)");
            return;
        }
        if reg_index(&nt.lc) >= 0 {
            self.error(nt.col, format!("'{}' is a register name", nt.s));
            return;
        }
        self.reg_defs.insert(nt.lc.clone(), r);
        self.expect_end(toks, i + 3);
    }

    fn include(&mut self, name: &TStr, col: u32) {
        let base = basename(name);
        if let Some(id) = devices::id_from_include_name(base) {
            if let Some(spec) = self.find_device(id) {
                self.select_device(spec, col);
            }
            return;
        }
        let includes = self.includes;
        let Some(text) = includes.get(&**name) else {
            if is_device_include_name(base) {
                self.error(col, format!("device include '{name}' is for an unsupported device (supported: {})", supported_devices()));
            } else {
                self.error(col, format!("include file '{name}' not found"));
            }
            return;
        };
        if self.include_stack.iter().any(|n| n == name) {
            self.error(col, format!("recursive include of '{name}'"));
            return;
        }
        if self.include_stack.len() >= MAX_INCLUDE_DEPTH {
            self.error(col, "includes nested too deeply");
            return;
        }
        let lines = match self.file_cache.get(&**name) {
            Some(l) => l.clone(),
            None => {
                let l: Rc<[SrcLine]> = Rc::from(tokenize(text));
                self.file_cache.insert(name.to_string(), l.clone());
                l
            }
        };
        let saved = self.rec;
        self.include_stack.push(name.clone());
        self.process_file(name.to_rc(), lines);
        self.include_stack.pop();
        self.rec = saved;
    }

    fn select_device(&mut self, spec: &'a AvrDeviceSpec, col: u32) {
        if std::ptr::eq(spec, self.device) {
            self.device_explicit = true;
            return;
        }
        if self.device_explicit {
            self.error(col, format!("device already set to {}", self.device.name));
            return;
        }
        if self.code_emitted {
            self.error(
                col,
                format!("the device must be selected before any code or data (already assembling for {})", self.device.name),
            );
            return;
        }
        self.device = spec;
        self.features = spec.features;
        self.dev_syms = def_include_symbols(spec);
        self.device_explicit = true;
        if !self.dseg_touched {
            self.loc[DSEG] = spec.sram_start as i64;
        }
    }

    fn reserve(&mut self, toks: &Rc<[Token]>, di: usize) {
        let t = &toks[di];
        let x = Self::mk_ref(toks, di + 1, toks.len(), t.col);
        let Some(v) = self.eval_now(&x, ".byte") else { return };
        if v < 0 {
            self.error(t.col, format!(".byte: negative size {v}"));
            return;
        }
        for &s in &self.pending {
            self.syms[s].size = v;
        }
        self.pending.clear();
        let seg = self.seg;
        self.advance(seg, v);
        let dev = self.device;
        if seg == DSEG {
            self.dseg_reserved = self.dseg_reserved.saturating_add(v);
            let ram_end = dev.sram_start as i64 + dev.sram_size as i64 - 1;
            if self.loc[DSEG] > ram_end + 1 && !self.dseg_overflow {
                self.dseg_overflow = true;
                self.error(t.col, format!("data segment exceeds SRAM (RAMEND = {}, {} bytes)", hex(ram_end, 4), dev.sram_size));
            }
        } else if self.loc[ESEG] > dev.eeprom_size as i64 && !self.eseg_overflow {
            self.eseg_overflow = true;
            self.error(t.col, format!("EEPROM segment exceeds the {} EEPROM size ({} bytes)", dev.name, dev.eeprom_size));
        }
    }

    fn data(&mut self, unit: u8, toks: &Rc<[Token]>, di: usize) {
        let t = &toks[di];
        if self.seg == DSEG {
            self.error(t.col, format!(".{} is not allowed in the data segment (use .byte to reserve space)", t.s));
            return;
        }
        let ranges = Self::split(toks, di + 1);
        if ranges.is_empty() {
            self.error(t.col, format!(".{}: missing value", t.s));
            return;
        }
        let mut items = Vec::with_capacity(ranges.len());
        let mut bytes: i64 = 0;
        for (s, e) in ranges {
            if s >= e {
                let col = toks.get(s).map_or(t.col, |x| x.col);
                self.error(col, format!(".{}: missing value", t.s));
                continue;
            }
            let first = &toks[s];
            if e - s == 1 && first.k == TokKind::Str {
                if unit != 1 {
                    self.error(first.col, "strings are only allowed in .db");
                    continue;
                }
                let b: Vec<u8> = first.s.encode_utf16().map(|u| u as u8).collect();
                bytes += b.len() as i64;
                items.push(DataItem::Bytes(b));
            } else {
                items.push(DataItem::Expr(Self::mk_ref(toks, s, e, t.col)));
                bytes += unit as i64;
            }
        }
        let seg = self.seg;
        let size = if seg == CSEG { (bytes + 1) >> 1 } else { bytes };
        if seg == CSEG && bytes & 1 != 0 {
            self.warning(t.col, ".db: odd number of bytes in the code segment, padded with a zero byte");
        }
        let idx = self.stmts.len();
        self.stmts.push(Stmt { rec: self.rec, seg, addr: self.loc[seg], size, pc: self.loc[CSEG], kind: StmtKind::Data { unit, items } });
        self.advance(seg, size);
        self.code_emitted = true;
        self.recs[self.rec].stmt = Some(idx);
    }

    // ---- conditionals

    fn conditional(&mut self, d: &str, toks: &Rc<[Token]>) {
        let t0 = &toks[0];
        match d {
            "if" | "ifdef" | "ifndef" => {
                let parent = self.active();
                let mut v = false;
                if parent {
                    v = if d == "if" { self.cond_value(toks) } else { self.ifdef_value(toks) == (d == "ifdef") };
                }
                self.conds.push(Cond { parent, taken: v || !parent, active: parent && v, in_else: false, rec: self.rec });
            }
            "elif" | "elseif" => {
                let top = self.conds.last().map(|c| (c.parent, c.taken, c.in_else));
                match top {
                    None | Some((_, _, true)) => self.error(t0.col, format!(".{} without .if", t0.s)),
                    Some((parent, taken, false)) => {
                        let v = parent && !taken && self.cond_value(toks);
                        if let Some(c) = self.conds.last_mut() {
                            if parent && !taken {
                                c.taken = v;
                            }
                            c.active = v;
                        }
                    }
                }
            }
            "else" => {
                match self.conds.last_mut() {
                    Some(c) if !c.in_else => {
                        c.in_else = true;
                        c.active = c.parent && !c.taken;
                        c.taken = true;
                    }
                    _ => self.error(t0.col, ".else without .if"),
                }
                self.expect_end(toks, 1);
            }
            _ => {
                // endif
                if self.conds.pop().is_none() {
                    self.error(t0.col, ".endif without .if");
                }
                self.expect_end(toks, 1);
            }
        }
    }

    fn cond_value(&mut self, toks: &Rc<[Token]>) -> bool {
        let t0 = &toks[0];
        let x = Self::mk_ref(toks, 1, toks.len(), t0.col.saturating_add(utf16_len(&t0.s)).saturating_add(1));
        let what = format!(".{}", t0.s);
        self.eval_now(&x, &what).is_some_and(|v| v != 0)
    }

    fn ifdef_value(&mut self, toks: &Rc<[Token]>) -> bool {
        let t0 = &toks[0];
        let Some(nt) = toks.get(1).filter(|t| t.k == TokKind::Id) else {
            let col = toks.get(1).map_or(t0.col, |t| t.col);
            self.error(col, format!(".{} expects a symbol name", t0.s));
            return false;
        };
        self.expect_end(toks, 2);
        self.is_defined(&nt.lc)
    }

    // ---- macros

    fn begin_macro(&mut self, toks: &Rc<[Token]>, di: usize) {
        let t = &toks[di];
        let i = di + 1;
        let nt = toks.get(i).filter(|x| x.k == TokKind::Id);
        match nt {
            None => self.error(toks.get(i).map_or(t.col, |x| x.col), ".macro: expected a macro name"),
            Some(_) => {
                if let Some(x) = toks.get(i + 1) {
                    self.warning(x.col, "macro parameter lists are ignored; use @0..@9 in the macro body");
                }
            }
        }
        let name = nt.map_or(TStr::Static("?"), |x| x.s.clone());
        let exists = nt.is_some_and(|x| self.macros.contains_key(&*x.lc));
        if let Some(nt) = nt.filter(|_| exists) {
            self.error(nt.col, format!("macro '{}' is already defined", nt.s));
        }
        let r = &self.recs[self.rec];
        let file = match (&r.mac, r.depth > 0) {
            (Some(m), true) => m.file.clone(),
            _ => r.file.clone(),
        };
        self.recording = Some(Recording { name, file, body: Vec::new(), rec: self.rec, discard: nt.is_none() || exists });
    }

    fn end_macro(&mut self) {
        let Some(m) = self.recording.take() else { return };
        if m.discard {
            return;
        }
        let mut locals = FxSet::default();
        for l in &m.body {
            let tk = &l.toks;
            let mut j = 0;
            while j + 1 < tk.len() && tk[j].k == TokKind::Id && tk[j + 1].is_op(":") {
                locals.insert(tk[j].lc.clone());
                j += 2;
            }
        }
        let key = TStr::from(m.name.to_ascii_lowercase());
        self.macros.insert(key, Rc::new(Macro { name: m.name, file: m.file, body: Rc::from(m.body), locals }));
    }

    fn expand_macro(&mut self, m: &Rc<Macro>, toks: &Rc<[Token]>, mi: usize, fr: &Frame) {
        let mn = &toks[mi];
        let i = mi + 1;
        if fr.depth >= MAX_MACRO_DEPTH {
            self.error(mn.col, format!("macro nesting too deep (recursive macro '{}'?)", m.name));
            return;
        }
        if self.expanded_lines > MAX_EXPANDED_LINES {
            return;
        }
        let args: Vec<&[Token]> = Self::split(toks, i).into_iter().map(|(s, e)| &toks[s..e]).collect();
        if args.len() > 10 {
            let col = toks.get(i).map_or(mn.col, |t| t.col);
            self.error(col, "too many macro arguments (at most 10: @0..@9)");
        }
        let mut arg_text: Option<Vec<String>> = None;
        self.expansion_id += 1;
        let id = self.expansion_id;
        let invocation = self.rec;
        let nf = if fr.depth == 0 {
            Frame {
                file: fr.file.clone(),
                site_file: fr.file.clone(),
                site_line: self.recs[invocation].line,
                site_col: mn.col,
                depth: 1,
                mac: Some(m.clone()),
            }
        } else {
            Frame {
                file: fr.file.clone(),
                site_file: fr.site_file.clone(),
                site_line: fr.site_line,
                site_col: fr.site_col,
                depth: fr.depth + 1,
                mac: Some(m.clone()),
            }
        };
        let cond_base = self.conds.len();
        for bl in m.body.iter() {
            self.expanded_lines += 1;
            if self.expanded_lines > MAX_EXPANDED_LINES {
                if self.expanded_lines == MAX_EXPANDED_LINES + 1 {
                    self.report(Severity::Error, invocation, mn.col, "too many macro expansions (recursive macro?)".into());
                }
                break;
            }
            let text = if bl.text.contains('@') {
                let at = arg_text.get_or_insert_with(|| {
                    args.iter()
                        .map(|a| {
                            let mut s = String::new();
                            for t in a.iter() {
                                t.push_text(&mut s);
                            }
                            s
                        })
                        .collect()
                });
                Rc::from(substitute_text(&bl.text, at))
            } else {
                bl.text.clone()
            };
            let body_toks = substitute(&bl.toks, &args, m, id);
            self.process_line(bl, body_toks, &nf, text);
            if self.exit_file {
                break;
            }
        }
        if self.recording.is_none() {
            self.close_conds(cond_base, &format!(" in macro '{}'", m.name));
        }
        self.rec = invocation;
    }

    // ---- instructions

    fn instruction(&mut self, toks: &Rc<[Token]>, mi: usize, fr: &Frame) {
        let mn = &toks[mi];
        if let Some(m) = self.macros.get(&*mn.lc).cloned() {
            self.expand_macro(&m, toks, mi, fr);
            return;
        }
        let tb = tables();
        let alias = tb.aliases.get(&*mn.lc);
        let Some((&base0, cands0)) = tb.by_name.get_key_value(alias.map_or(&*mn.lc, |a| a.base)) else {
            self.error(mn.col, format!("unknown instruction or macro '{}'", mn.s));
            return;
        };
        if self.seg != CSEG {
            self.error(mn.col, format!("instructions are not allowed in the {} segment", SEG_NAMES[self.seg]));
            return;
        }
        self.code_emitted = true;
        let addr = self.loc[CSEG];
        let guess = cands0.first().map_or(1, |d| d.words as i64);
        let Some(mut ops) = self.parse_operands(toks, mi + 1) else {
            self.advance(CSEG, guess);
            return;
        };
        let mut hidden = 0usize;
        if let Some(a) = alias {
            if a.dup || a.post.is_some() {
                if ops.len() != 1 {
                    self.error(mn.col, format!("'{}' expects 1 operand", mn.lc));
                    self.advance(CSEG, guess);
                    return;
                }
                let extra = match a.post {
                    Some(v) if !a.dup => Opnd::Const { v, col: ops[0].col() },
                    _ => ops[0].clone(),
                };
                ops.push(extra);
            } else if let Some(pre) = a.pre {
                ops.insert(0, Opnd::Const { v: pre, col: mn.col });
                hidden = 1;
            }
        }
        // `ld Rd, Y+q` is `ldd`; `ldd Rd, Y` is `ldd Rd, Y+0`.
        let (mut base, mut cands) = (base0, cands0);
        if base == "ld" || base == "st" {
            if ops.iter().any(|o| matches!(o, Opnd::Disp { .. })) {
                let alt = if base == "ld" { "ldd" } else { "std" };
                if let Some((&b, c)) = tb.by_name.get_key_value(alt) {
                    base = b;
                    cands = c;
                }
            }
        } else if base == "ldd" || base == "std" {
            for o in ops.iter_mut() {
                if let Opnd::Ptr { p, col } = *o {
                    if p == OperandKind::Y || p == OperandKind::Z {
                        *o = Opnd::Disp { z: p == OperandKind::Z, x: None, col };
                    }
                }
            }
        }

        let Some(def) = self.select_form(mn, base, cands, &ops, hidden) else {
            self.advance(CSEG, guess);
            return;
        };
        let mut vals = Vec::with_capacity(def.operands.len());
        let mut bad = false;
        for (j, &kind) in def.operands.iter().enumerate() {
            if kind.is_literal() {
                continue;
            }
            match ops.get(j) {
                Some(&Opnd::Reg { v, col, .. }) => {
                    if !self.check_reg(kind, v, col, &mn.lc) {
                        bad = true;
                    }
                    vals.push(Val::Num(v));
                }
                Some(&Opnd::Const { v, .. }) => vals.push(Val::Num(v)),
                Some(Opnd::Expr { x, .. }) => vals.push(Val::Expr(x.clone())),
                Some(Opnd::Disp { x, .. }) => vals.push(x.clone().map_or(Val::Num(0), Val::Expr)),
                _ => {}
            }
        }
        let idx = self.stmts.len();
        let size = def.words as i64;
        self.stmts.push(Stmt {
            rec: self.rec,
            seg: CSEG,
            addr,
            size,
            pc: addr,
            kind: StmtKind::Insn { def, vals, cbr: alias.is_some_and(|a| a.cbr), bad },
        });
        self.advance(CSEG, size);
        self.recs[self.rec].stmt = Some(idx);
    }

    fn parse_operands(&mut self, toks: &Rc<[Token]>, i: usize) -> Option<Vec<Opnd>> {
        let mut out = Vec::with_capacity(3);
        let mut ok = true;
        for (s, e) in Self::split(toks, i) {
            if s >= e {
                let col = toks.get(s).map_or_else(|| toks.last().map_or(0, |t| t.col.saturating_add(1)), |t| t.col);
                self.error(col, "missing operand");
                ok = false;
                continue;
            }
            let t = &toks[s];
            let len = e - s;
            if t.k == TokKind::Id {
                let r = self.reg_number(&t.lc);
                if len == 1 && r >= 0 {
                    out.push(Opnd::Reg { v: r, pair: false, col: t.col });
                    continue;
                }
                // Register pair `r25:r24`.
                if len == 3 && r >= 0 && toks[s + 1].is_op(":") && toks[s + 2].k == TokKind::Id {
                    let lo = self.reg_number(&toks[s + 2].lc);
                    if lo >= 0 && r == lo + 1 && lo & 1 == 0 {
                        out.push(Opnd::Reg { v: lo, pair: true, col: t.col });
                    } else {
                        self.error(t.col, format!("invalid register pair '{}:{}'", t.s, toks[s + 2].s));
                        ok = false;
                    }
                    continue;
                }
                let p = match &*t.lc {
                    "x" => Some((OperandKind::X, OperandKind::XInc)),
                    "y" => Some((OperandKind::Y, OperandKind::YInc)),
                    "z" => Some((OperandKind::Z, OperandKind::ZInc)),
                    _ => None,
                };
                if let Some((plain, inc)) = p {
                    if len == 1 {
                        out.push(Opnd::Ptr { p: plain, col: t.col });
                        continue;
                    }
                    let op = &toks[s + 1];
                    if op.is_op("+") {
                        if len == 2 {
                            out.push(Opnd::Ptr { p: inc, col: t.col });
                        } else if plain == OperandKind::X {
                            self.error(t.col, "displacement addressing is only available with Y or Z");
                            ok = false;
                        } else {
                            let x = Self::mk_ref(toks, s + 2, e, op.col);
                            out.push(Opnd::Disp { z: plain == OperandKind::Z, x: Some(x), col: t.col });
                        }
                        continue;
                    }
                }
            } else if t.is_op("-") && len == 2 && toks[s + 1].k == TokKind::Id {
                let p = match &*toks[s + 1].lc {
                    "x" => Some(OperandKind::XDec),
                    "y" => Some(OperandKind::YDec),
                    "z" => Some(OperandKind::ZDec),
                    _ => None,
                };
                if let Some(p) = p {
                    out.push(Opnd::Ptr { p, col: t.col });
                    continue;
                }
            }
            out.push(Opnd::Expr { x: Self::mk_ref(toks, s, e, t.col), col: t.col });
        }
        ok.then_some(out)
    }

    fn select_form(
        &mut self,
        mn: &Token,
        base: &str,
        cands: &[&'static InsnDef],
        ops: &[Opnd],
        hidden: usize,
    ) -> Option<&'static InsnDef> {
        let mut count_match = false;
        let mut shape_match: Option<&'static InsnDef> = None;
        for &d in cands {
            if d.operands.len() != ops.len() {
                continue;
            }
            count_match = true;
            if !d.operands.iter().zip(ops).all(|(&k, o)| shape_ok(k, o)) {
                continue;
            }
            if d.is_available(self.features) {
                return Some(d);
            }
            shape_match = shape_match.or(Some(d));
        }
        if shape_match.is_some() {
            let msg = format!("instruction '{base}' is not supported by {} ({} core)", self.device.name, self.device.core_name);
            self.error(mn.col, msg);
            return None;
        }
        if !count_match {
            let mut counts: Vec<i64> = cands.iter().map(|d| d.operands.len() as i64 - hidden as i64).collect();
            counts.sort_unstable();
            counts.dedup();
            let n = counts.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" or ");
            let plural = if n == "1" { "" } else { "s" };
            let got = ops.len() as i64 - hidden as i64;
            self.error(mn.col, format!("'{}' expects {n} operand{plural} (got {got})", mn.lc));
            return None;
        }
        // Report the first operand position that no form accepts.
        let mut pos = ops.len();
        for d in cands.iter().filter(|d| d.operands.len() == ops.len()) {
            if let Some(j) = d.operands.iter().zip(ops).position(|(&k, o)| !shape_ok(k, o)) {
                pos = pos.min(j);
            }
        }
        if pos >= ops.len() {
            pos = hidden;
        }
        let mut expected: Vec<&'static str> = Vec::new();
        for d in cands.iter().filter(|d| d.operands.len() == ops.len()) {
            if let Some(&k) = d.operands.get(pos) {
                let s = describe_kind(k);
                if !expected.contains(&s) {
                    expected.push(s);
                }
            }
        }
        expected.sort_by_key(|s| KIND_ORDER.iter().position(|k| k == s).map_or(-1, |p| p as i64));
        let col = ops.get(pos).map_or(mn.col, Opnd::col);
        let n = pos as i64 + 1 - hidden as i64;
        self.error(col, format!("invalid operand {n} for '{}': expected {}", mn.lc, expected.join(", ")));
        None
    }

    fn check_reg(&mut self, kind: OperandKind, v: i64, col: u32, mn: &str) -> bool {
        use OperandKind as K;
        let (lo, hi) = operand_range(kind, self.features);
        let (lo, hi) = (lo as i64, hi as i64);
        if v >= lo && v <= hi && (!is_pair_kind(kind) || v & 1 == 0) {
            return true;
        }
        if self.features & feature::RC != 0 && v < 16 {
            let msg = format!("register r{v} is not available on {} (only r16-r31)", self.device.name);
            self.error(col, msg);
            return false;
        }
        let need = match kind {
            K::Rd4 | K::Rr4 => "a register in r16-r31".to_string(),
            K::Rd3 | K::Rr3 => "a register in r16-r23".to_string(),
            K::RdW | K::RrW => "an even register (r0, r2, ..., r30)".to_string(),
            K::RdP => "r24, r26, r28 or r30".to_string(),
            _ => format!("a register in r{lo}-r{hi}"),
        };
        self.error(col, format!("'{mn}' requires {need} (got r{v})"));
        false
    }

    // ------------------------------------------------------------------------------- pass 2

    fn pass2(&mut self) {
        let dev = self.device;
        self.flash = vec![0xff; dev.flash_size as usize];
        self.owner = vec![0; (dev.flash_size >> 1) as usize];
        if dev.eeprom_size > 0 {
            self.eeprom = Some(vec![0xff; dev.eeprom_size as usize]);
        }

        // Report forward-referencing .equ definitions that never resolve.
        for idx in 0..self.syms.len() {
            if self.syms[idx].state == SymState::Lazy {
                let _ = self.resolve_lazy(idx, 0, true, 0);
            }
        }

        let stmts = std::mem::take(&mut self.stmts);
        let mut last: Option<(u32, u32)> = None;
        let mut sorted = true;
        let mut last_addr: i64 = -1;
        self.line_table.reserve(stmts.len());
        for (idx, st) in stmts.iter().enumerate() {
            let words = match &st.kind {
                StmtKind::Set { sym, x, value } => {
                    let r = match x {
                        Some(x) => self.try_eval(x, st.pc, true, false, st.rec).map(|r| r.value),
                        None => Some(*value),
                    };
                    let sym = &mut self.syms[*sym];
                    match r {
                        Some(v) => {
                            sym.value = v;
                            sym.state = SymState::Ok;
                        }
                        None => sym.state = SymState::Error,
                    }
                    continue;
                }
                _ if st.seg == ESEG => {
                    self.emit_eeprom_data(st);
                    continue;
                }
                StmtKind::Insn { bad: true, .. } => None,
                StmtKind::Insn { .. } => self.encode_insn(st),
                StmtKind::Data { .. } => Some(self.data_words(st)),
            };
            self.write_code(&stmts, idx, words.as_deref());
            if st.size > 0 && st.addr >= 0 && st.addr < self.owner.len() as i64 {
                let f = self.recs[st.rec].file.clone();
                let file = self.file_index(&f);
                let line = self.recs[st.rec].line;
                let is_insn = matches!(st.kind, StmtKind::Insn { .. });
                let is_stmt = is_insn && last != Some((file, line));
                if is_insn {
                    last = Some((file, line));
                }
                let address = (st.addr * 2) as u32;
                if (address as i64) < last_addr {
                    sorted = false;
                }
                last_addr = address as i64;
                self.line_table.push(LineEntry { address, file, line, is_stmt });
            }
        }
        if !sorted {
            self.line_table.sort_by_key(|l| l.address);
        }
        self.stmts = stmts;
    }

    fn write_code(&mut self, stmts: &[Stmt], idx: usize, words: Option<&[u16]>) {
        let st = &stmts[idx];
        let limit = self.owner.len() as i64;
        let me = idx as u32 + 1;
        let mut overlap_reported = false;
        for k in 0..st.size {
            let a = st.addr.saturating_add(k);
            if a < 0 || a >= limit {
                if !self.overflow_reported {
                    self.overflow_reported = true;
                    let msg = format!(
                        "code exceeds the {} flash memory ({} bytes) at byte address {}",
                        self.device.name,
                        self.device.flash_size,
                        hex(a.saturating_mul(2), 4)
                    );
                    self.report(Severity::Error, st.rec, 0, msg);
                }
                return;
            }
            let au = a as usize;
            let prev = self.owner[au];
            if prev != 0 && prev != me && !overlap_reported {
                overlap_reported = true;
                if let Some(o) = stmts.get(prev as usize - 1).and_then(|p| self.recs.get(p.rec)) {
                    let msg = format!(
                        "code overlaps previously assembled code at byte address {} ({}:{})",
                        hex(a * 2, 4),
                        o.file,
                        o.line
                    );
                    self.report(Severity::Error, st.rec, 0, msg);
                }
            }
            self.owner[au] = me;
            if let Some(words) = words {
                let w = words.get(k as usize).copied().unwrap_or(0);
                self.flash[au * 2] = w as u8;
                self.flash[au * 2 + 1] = (w >> 8) as u8;
                self.flash_used = self.flash_used.max(au as u32 * 2 + 2);
            }
        }
    }

    fn encode_insn(&mut self, st: &Stmt) -> Option<Vec<u16>> {
        use OperandKind as K;
        let StmtKind::Insn { def, vals, cbr, .. } = &st.kind else { return None };
        let def: &'static InsnDef = def;
        let mut values: Vec<i32> = Vec::with_capacity(vals.len());
        let mut ok = true;
        for (fi, kind) in def.value_operands().enumerate() {
            let x = match vals.get(fi) {
                Some(Val::Num(v)) => {
                    values.push(*v as i32);
                    continue;
                }
                Some(Val::Expr(x)) => x,
                None => {
                    ok = false;
                    continue;
                }
            };
            let jump = matches!(kind, K::K7 | K::K12 | K::K22);
            let Some(r) = self.try_eval(x, st.addr, true, jump, st.rec) else {
                ok = false;
                continue;
            };
            let mut v = r.value;
            if jump && r.used_dot {
                if v % 2 != 0 {
                    let msg = format!("jump target {v} is not a word boundary (GNU '.' is a byte address)");
                    self.report(Severity::Error, st.rec, x.col, msg);
                    ok = false;
                    continue;
                }
                v /= 2;
            }
            match self.convert_operand(kind, v, st, def, *cbr, x.col) {
                Some(c) => values.push(c as i32),
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        match encode(def, &values) {
            Ok(w) => Some(w),
            Err(e) => {
                self.report(Severity::Error, st.rec, 0, format!("cannot encode '{}': {e}", def.name));
                None
            }
        }
    }

    /// Converts an evaluated operand to the normalized value `encode` expects, with range checks.
    fn convert_operand(&mut self, kind: OperandKind, v: i64, st: &Stmt, def: &InsnDef, cbr: bool, col: u32) -> Option<i64> {
        use OperandKind as K;
        match kind {
            K::K7 | K::K12 => {
                let (lo, hi) = if kind == K::K7 { (-64i64, 63i64) } else { (-2048, 2047) };
                let mut off = v.wrapping_sub(st.addr.wrapping_add(1));
                let flash_words = (self.device.flash_size >> 1) as i64;
                if (off < lo || off > hi) && kind == K::K12 && flash_words > 0 && flash_words <= 4096 {
                    // RJMP/RCALL wrap around on devices with <= 8 KB flash.
                    let mut w = off.rem_euclid(flash_words);
                    if w > hi {
                        w -= flash_words;
                    }
                    if w >= lo && w <= hi {
                        off = w;
                    }
                }
                if off < lo || off > hi {
                    let msg = format!("relative branch out of range (offset {off} words, allowed {lo}..{hi})");
                    self.report(Severity::Error, st.rec, col, msg);
                    return None;
                }
                Some(off)
            }
            K::K8 => {
                if !(-128..=255).contains(&v) {
                    self.report(Severity::Error, st.rec, col, format!("constant {v} out of range (-128..255)"));
                    return None;
                }
                let b = v & 0xff;
                Some(if cbr { !b & 0xff } else { b })
            }
            K::K7rc => {
                if !(0x40..=0xbf).contains(&v) {
                    let msg = format!(
                        "data address {} out of range for '{}' on the {} core (0x40-0xBF)",
                        hex(v, 2),
                        def.name,
                        self.device.core_name
                    );
                    self.report(Severity::Error, st.rec, col, msg);
                    return None;
                }
                Some(v)
            }
            _ => {
                let (lo, hi) = operand_range(kind, self.features);
                let (lo, hi) = (lo as i64, hi as i64);
                if v < lo || v > hi {
                    let addr = matches!(kind, K::A5 | K::A6 | K::K16 | K::K22);
                    let f = |x: i64| if addr { hex(x, 2) } else { x.to_string() };
                    let msg = format!("{} {} out of range ({}..{})", kind_label(kind), f(v), f(lo), f(hi));
                    self.report(Severity::Error, st.rec, col, msg);
                    return None;
                }
                Some(v)
            }
        }
    }

    /// Evaluates a data statement to bytes (little-endian).
    fn data_bytes(&mut self, st: &Stmt) -> Vec<u8> {
        let StmtKind::Data { unit, items } = &st.kind else { return Vec::new() };
        let unit = *unit;
        let (name, lo, hi): (&str, i64, i64) = match unit {
            1 => (".db", -128, 255),
            2 => (".dw", -32768, 65535),
            4 => (".dd", -0x8000_0000, 0xffff_ffff),
            _ => (".dq", -(1i64 << 53), 1i64 << 53),
        };
        let mut bytes = Vec::with_capacity(items.len() * unit as usize);
        for it in items {
            let x = match it {
                DataItem::Bytes(b) => {
                    bytes.extend_from_slice(b);
                    continue;
                }
                DataItem::Expr(x) => x,
            };
            let v = self.try_eval(x, st.pc, true, false, st.rec).map_or(0, |r| r.value);
            if v < lo || v > hi {
                let msg = format!("{name}: value {v} out of range ({lo}..{hi}), truncated");
                self.report(Severity::Warning, st.rec, x.col, msg);
            }
            if unit == 8 {
                bytes.extend_from_slice(&(v as u64).to_le_bytes());
            } else {
                bytes.extend_from_slice(&(v as u32).to_le_bytes()[..unit as usize]);
            }
        }
        bytes
    }

    fn data_words(&mut self, st: &Stmt) -> Vec<u16> {
        let b = self.data_bytes(st);
        let n = st.size.clamp(0, (b.len() as i64 + 1) / 2 + 1) as usize;
        (0..n)
            .map(|k| {
                let lo = b.get(2 * k).copied().unwrap_or(0) as u16;
                let hi = b.get(2 * k + 1).copied().unwrap_or(0) as u16;
                lo | (hi << 8)
            })
            .collect()
    }

    fn emit_eeprom_data(&mut self, st: &Stmt) {
        if !matches!(st.kind, StmtKind::Data { .. }) {
            return;
        }
        let b = self.data_bytes(st);
        let size = self.eeprom.as_ref().map_or(0, |e| e.len()) as i64;
        for (k, &byte) in b.iter().enumerate() {
            let a = st.addr.saturating_add(k as i64);
            match self.eeprom.as_mut() {
                Some(ee) if a >= 0 && a < size => {
                    ee[a as usize] = byte;
                    self.eeprom_written = true;
                }
                _ => {
                    if !self.eseg_overflow {
                        self.eseg_overflow = true;
                        let msg = format!(
                            "EEPROM data exceeds the {} EEPROM size ({} bytes)",
                            self.device.name, self.device.eeprom_size
                        );
                        self.report(Severity::Error, st.rec, 0, msg);
                    }
                    return;
                }
            }
        }
    }

    // ------------------------------------------------------------------------------- output

    fn export_symbols(&self) -> Vec<ProgramSymbol> {
        let mut out = Vec::with_capacity(self.syms.len());
        for s in &self.syms {
            if s.hidden || s.state != SymState::Ok {
                continue;
            }
            let name = s.name.to_string();
            out.push(match s.kind {
                SymKind::Label if s.seg == CSEG => ProgramSymbol {
                    name,
                    address: s.value.wrapping_mul(2) as u32,
                    size: 0,
                    kind: SymbolKind::Label,
                    space: SymbolSpace::Code,
                    global: true,
                },
                SymKind::Label => ProgramSymbol {
                    name,
                    address: s.value as u32,
                    size: s.size as u32,
                    kind: SymbolKind::Label,
                    space: if s.seg == DSEG { SymbolSpace::Data } else { SymbolSpace::Eeprom },
                    global: true,
                },
                SymKind::Equ | SymKind::Set => ProgramSymbol {
                    name,
                    address: s.value as u32,
                    size: 0,
                    kind: SymbolKind::Const,
                    space: SymbolSpace::None,
                    global: false,
                },
            });
        }
        out
    }

    fn build_listing(&self, diags: &[Diagnostic]) -> String {
        let dev = self.device;
        let mut out = String::with_capacity(64 * self.recs.len() + 512);
        let _ = write!(
            out,
            "; {} - AVR assembler listing for {} ({} core)\n;\n; Addr      Words / bytes",
            self.main_file, dev.name, dev.core_name
        );
        let flash = &self.flash;
        let flash_words = (flash.len() >> 1) as i64;
        let mut head = String::with_capacity(64);
        for r in &self.recs {
            if !r.list {
                continue;
            }
            let mark = if r.depth > 0 { '+' } else { ' ' };
            let st = r.stmt.and_then(|i| self.stmts.get(i)).filter(|st| !matches!(st.kind, StmtKind::Set { .. }) && st.size > 0);
            let Some(st) = st else {
                let _ = write!(out, "\n{:LIST_PAD$}{mark} {}", "", r.text);
                continue;
            };
            let code = st.seg == CSEG;
            let per_row: i64 = if code { 4 } else { 8 };
            let mut k = 0i64;
            while k < st.size {
                head.clear();
                let _ = write!(head, "{}:{:06x}", SEG_CHARS[st.seg.min(2)], st.addr.saturating_add(k));
                let end = (k + per_row).min(st.size);
                for c in k..end {
                    let a = st.addr.saturating_add(c);
                    if code {
                        if a >= 0 && a < flash_words {
                            let au = a as usize;
                            let w = flash[au * 2] as u16 | (flash[au * 2 + 1] as u16) << 8;
                            let _ = write!(head, " {w:04x}");
                        } else {
                            head.push_str(" ----");
                        }
                    } else {
                        match self.eeprom.as_ref().filter(|ee| a >= 0 && a < ee.len() as i64) {
                            Some(ee) => {
                                let _ = write!(head, " {:02x}", ee[a as usize]);
                            }
                            None => head.push_str(" --"),
                        }
                    }
                }
                if k == 0 {
                    let _ = write!(out, "\n{head:<LIST_PAD$}{mark} {}", r.text);
                } else {
                    out.push('\n');
                    out.push_str(&head);
                }
                k = end;
            }
        }
        let code_words = self.owner.iter().filter(|&&o| o != 0).count() as i64;
        let pct = |used: i64, total: i64| {
            if total > 0 {
                to_fixed1((100 * used) as f64 / total as f64)
            } else {
                "0.0".to_string()
            }
        };
        let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
        let warnings = diags.iter().filter(|d| d.severity == Severity::Warning).count();
        let flash_size = dev.flash_size as i64;
        let sram = dev.sram_size as i64;
        let _ = write!(
            out,
            "\n\n; {} memory use summary [bytes]:\n;   Code (flash):  {} of {} ({}%)\n;   Data (SRAM):   {} of {} ({}%)",
            dev.name,
            code_words * 2,
            flash_size,
            pct(code_words * 2, flash_size),
            self.dseg_reserved,
            sram,
            pct(self.dseg_reserved, sram)
        );
        if dev.eeprom_size > 0 {
            let used = match (&self.eeprom, self.eeprom_written) {
                (Some(ee), true) => ee.iter().filter(|&&b| b != 0xff).count(),
                _ => 0,
            };
            let _ = write!(out, "\n;   EEPROM:        {used} of {}", dev.eeprom_size);
        }
        let _ = write!(
            out,
            "\n; Assembly {}: {errors} error{}, {warnings} warning{}",
            if errors > 0 { "failed" } else { "complete" },
            if errors == 1 { "" } else { "s" },
            if warnings == 1 { "" } else { "s" }
        );
        out
    }
}

impl ExprEnv for Assembler<'_> {
    fn lookup(&mut self, lc: &str, col: u32, strict: bool, depth: u32) -> Result<Option<i64>, ExprError> {
        if let Some(&idx) = self.sym_idx.get(lc) {
            let sym = &self.syms[idx];
            return match sym.state {
                SymState::Ok => Ok(Some(sym.value)),
                SymState::Lazy => {
                    if depth >= MAX_EXPR_DEPTH {
                        return Err(ExprError::new("expression is nested too deeply", col));
                    }
                    self.resolve_lazy(idx, col, strict, depth)
                }
                SymState::Pending => Ok(None),
                SymState::Error => Err(ExprError::silent(col)),
            };
        }
        Ok(self.dev_syms.equs.get(lc).copied())
    }

    fn is_defined(&self, lc: &str) -> bool {
        self.sym_idx.contains_key(lc)
            || self.dev_syms.equs.contains_key(lc)
            || self.reg_defs.contains_key(lc)
            || self.dev_syms.defs.contains_key(lc)
    }
}

/// Replaces `@n` parameters and renames macro-local labels; returns `src` when unchanged.
fn substitute(src: &Rc<[Token]>, args: &[&[Token]], m: &Macro, id: u64) -> Rc<[Token]> {
    let has_locals = !m.locals.is_empty();
    let mut out: Option<Vec<Token>> = None;
    for (j, t) in src.iter().enumerate() {
        let local = has_locals && t.k == TokKind::Id && m.locals.contains(&*t.lc);
        let param = if t.k == TokKind::Param { args.get(t.v as usize) } else { None };
        if (local || param.is_some()) && out.is_none() {
            let mut v = Vec::with_capacity(src.len() + 4);
            v.extend_from_slice(&src[..j]);
            out = Some(v);
        }
        let Some(o) = out.as_mut() else { continue };
        if let Some(a) = param {
            o.extend_from_slice(a);
        } else if local {
            o.push(Token {
                k: TokKind::Id,
                s: TStr::from(format!("{}#{id}", t.s)),
                lc: TStr::from(format!("{}#{id}", t.lc)),
                v: 0,
                col: t.col,
            });
        } else {
            o.push(t.clone());
        }
    }
    match out {
        Some(v) => Rc::from(v),
        None => src.clone(),
    }
}

/// Listing text of a macro body line: `@n` replaced by the argument text (kept when missing).
fn substitute_text(text: &str, args: &[String]) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 16);
    let mut last = 0;
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'@' && b[i + 1].is_ascii_digit() {
            if let Some(a) = args.get((b[i + 1] - b'0') as usize) {
                out.push_str(&text[last..i]);
                out.push_str(a);
                last = i + 2;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_names() {
        assert_eq!(reg_index("r0"), 0);
        assert_eq!(reg_index("r31"), 31);
        assert_eq!(reg_index("r32"), -1);
        assert_eq!(reg_index("r05"), -1);
        assert_eq!(reg_index("rx"), -1);
        assert_eq!(reg_index("r"), -1);
    }

    #[test]
    fn device_include_pattern() {
        assert!(is_device_include_name("m328Pdef.inc"));
        assert!(is_device_include_name("TN13DEF.INC"));
        assert!(!is_device_include_name("tndef.inc"));
        assert!(!is_device_include_name("macros.inc"));
    }

    #[test]
    fn text_substitution() {
        let args = vec!["r24".to_string(), "1".to_string()];
        assert_eq!(substitute_text(" ldi @0, @1 ; @5 @", &args), " ldi r24, 1 ; @5 @");
    }
}
