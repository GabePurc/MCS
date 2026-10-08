//! Expression evaluator (C operator precedence) for the AVR assembler.
//!
//! Values are 64-bit integers. `+ - * / % << >>` use wrapping 64-bit arithmetic (avrasm2 uses
//! 64-bit arithmetic); `& | ^ ~` operate on 32 bits (sign-extended result), like the original
//! TypeScript implementation.

use crate::lexer::{TStr, TokKind, Token};
use crate::util::utf16_len;

/// Bound on parser recursion (parentheses, unary operators, ternaries, `.equ` chains) so that
/// hostile input cannot overflow the stack.
pub(crate) const MAX_EXPR_DEPTH: u32 = 200;

#[derive(Debug)]
pub(crate) struct ExprError {
    pub msg: String,
    pub col: u32,
    /// True when the problem has already been reported (suppress a second diagnostic).
    pub silent: bool,
}

impl ExprError {
    pub(crate) fn new(msg: impl Into<String>, col: u32) -> Self {
        Self { msg: msg.into(), col, silent: false }
    }

    pub(crate) fn silent(col: u32) -> Self {
        Self { msg: String::new(), col, silent: true }
    }
}

pub(crate) trait ExprEnv {
    /// Value of a symbol (`lc` = lowercase name) or `None` when not (yet) defined. May fail
    /// (e.g. circular definitions). `depth` is the current recursion depth (for nested lookups).
    fn lookup(&mut self, lc: &str, col: u32, strict: bool, depth: u32) -> Result<Option<i64>, ExprError>;
    /// For `defined(name)`.
    fn is_defined(&self, lc: &str) -> bool;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EvalMode {
    /// Word address of the current instruction (`PC`).
    pub pc: i64,
    /// Undefined symbols are errors; otherwise they evaluate to 0 and are reported through
    /// `EvalResult::unresolved` (first pass / forward references).
    pub strict: bool,
    /// `.` evaluates to the BYTE address of the current instruction (GNU `rjmp .+2`), otherwise
    /// to the word address (same as `PC`).
    pub dot_bytes: bool,
    /// Initial recursion depth.
    pub depth: u32,
}

#[derive(Debug)]
pub(crate) struct EvalResult {
    pub value: i64,
    /// First symbol that could not be resolved (tolerant mode only): (name, column).
    pub unresolved: Option<(TStr, u32)>,
    /// True when `.` (current location) was used.
    pub used_dot: bool,
}

/// Evaluates the whole token slice `toks` as one expression.
pub(crate) fn evaluate<E: ExprEnv + ?Sized>(
    toks: &[Token],
    env: &mut E,
    mode: EvalMode,
    fallback_col: u32,
) -> Result<EvalResult, ExprError> {
    if toks.is_empty() {
        return Err(ExprError::new("missing expression", fallback_col));
    }
    let mut p = Parser { toks, pos: 0, env, mode, depth: mode.depth, unresolved: None, used_dot: false };
    let value = p.ternary()?;
    if let Some(t) = toks.get(p.pos) {
        return Err(ExprError::new(format!("unexpected '{}' in expression", t.text()), t.col));
    }
    Ok(EvalResult { value, unresolved: p.unresolved, used_dot: p.used_dot })
}

/// Binary operator precedence (higher binds tighter); 0 = not a binary operator.
fn bin_prec(op: &str) -> u8 {
    match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" => 6,
        "<" | "<=" | ">" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        _ => 0,
    }
}

#[inline]
fn shl(a: i64, b: i64) -> i64 {
    if (0..=63).contains(&b) {
        a.wrapping_shl(b as u32)
    } else {
        0
    }
}

#[inline]
fn shr(a: i64, b: i64) -> i64 {
    if (0..=63).contains(&b) {
        a >> b
    } else if a < 0 {
        -1
    } else {
        0
    }
}

#[inline]
fn i32_of(a: i64) -> i32 {
    a as i32
}

type R = Result<i64, ExprError>;

struct Parser<'t, 'e, E: ExprEnv + ?Sized> {
    toks: &'t [Token],
    pos: usize,
    env: &'e mut E,
    mode: EvalMode,
    depth: u32,
    unresolved: Option<(TStr, u32)>,
    used_dot: bool,
}

impl<'t, E: ExprEnv + ?Sized> Parser<'t, '_, E> {
    #[inline]
    fn peek_op(&self) -> &'t str {
        match self.toks.get(self.pos) {
            Some(t) if t.k == TokKind::Op => &t.s,
            _ => "",
        }
    }

    fn expect_op(&mut self, op: &str) -> Result<(), ExprError> {
        if self.peek_op() != op {
            return Err(match self.toks.get(self.pos) {
                Some(t) => ExprError::new(format!("expected '{op}' but found '{}'", t.text()), t.col),
                None => ExprError::new(format!("expected '{op}'"), self.last_col()),
            });
        }
        self.pos += 1;
        Ok(())
    }

    fn last_col(&self) -> u32 {
        let idx = self.pos.min(self.toks.len());
        match idx.checked_sub(1).and_then(|i| self.toks.get(i)) {
            Some(t) => t.col.saturating_add(utf16_len(&t.s)),
            None => 0,
        }
    }

    fn cur_col(&self) -> u32 {
        self.toks.get(self.pos).map_or_else(|| self.last_col(), |t| t.col)
    }

    #[inline]
    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            return Err(ExprError::new("expression is nested too deeply", self.cur_col()));
        }
        Ok(())
    }

    /// Errors on values derived from unresolved symbols are deferred to the strict pass.
    fn value_error(&self, msg: impl Into<String>, col: u32) -> R {
        if self.unresolved.is_some() {
            return Ok(0);
        }
        Err(ExprError::new(msg, col))
    }

    fn ternary(&mut self) -> R {
        self.enter()?;
        let r = self.ternary_inner();
        self.depth -= 1;
        r
    }

    fn ternary_inner(&mut self) -> R {
        let c = self.binary(1)?;
        if self.peek_op() != "?" {
            return Ok(c);
        }
        self.pos += 1;
        let a = self.ternary()?;
        self.expect_op(":")?;
        let b = self.ternary()?;
        Ok(if c != 0 { a } else { b })
    }

    fn binary(&mut self, min_prec: u8) -> R {
        let mut left = self.unary()?;
        loop {
            let op = self.peek_op();
            let prec = bin_prec(op);
            if prec == 0 || prec < min_prec {
                return Ok(left);
            }
            let col = self.toks[self.pos].col;
            self.pos += 1;
            let right = self.binary(prec + 1)?;
            left = self.apply(op, left, right, col)?;
        }
    }

    fn apply(&self, op: &str, a: i64, b: i64, col: u32) -> R {
        Ok(match op {
            "+" => a.wrapping_add(b),
            "-" => a.wrapping_sub(b),
            "*" => a.wrapping_mul(b),
            "/" => {
                if b == 0 {
                    return self.value_error("division by zero", col);
                }
                a.wrapping_div(b)
            }
            "%" => {
                if b == 0 {
                    return self.value_error("division by zero", col);
                }
                a.wrapping_rem(b)
            }
            "<<" => shl(a, b),
            ">>" => shr(a, b),
            "&" => (i32_of(a) & i32_of(b)) as i64,
            "|" => (i32_of(a) | i32_of(b)) as i64,
            "^" => (i32_of(a) ^ i32_of(b)) as i64,
            "==" => (a == b) as i64,
            "!=" => (a != b) as i64,
            "<" => (a < b) as i64,
            "<=" => (a <= b) as i64,
            ">" => (a > b) as i64,
            ">=" => (a >= b) as i64,
            "&&" => (a != 0 && b != 0) as i64,
            _ => (a != 0 || b != 0) as i64, // "||"
        })
    }

    fn unary(&mut self) -> R {
        self.enter()?;
        let r = match self.peek_op() {
            "-" => {
                self.pos += 1;
                self.unary().map(i64::wrapping_neg)
            }
            "+" => {
                self.pos += 1;
                self.unary()
            }
            "~" => {
                self.pos += 1;
                self.unary().map(|v| (!i32_of(v)) as i64)
            }
            "!" => {
                self.pos += 1;
                self.unary().map(|v| (v == 0) as i64)
            }
            _ => self.primary(),
        };
        self.depth -= 1;
        r
    }

    fn primary(&mut self) -> R {
        let Some(t) = self.toks.get(self.pos) else {
            return Err(ExprError::new("missing operand", self.last_col()));
        };
        self.pos += 1;
        match t.k {
            TokKind::Num => Ok(t.v),
            TokKind::Id => {
                if self.peek_op() == "(" {
                    return self.call(t);
                }
                if &*t.lc == "pc" {
                    return Ok(self.mode.pc);
                }
                if let Some(v) = self.env.lookup(&t.lc, t.col, self.mode.strict, self.depth)? {
                    return Ok(v);
                }
                if self.mode.strict {
                    let msg = if crate::assembler::reg_index(&t.lc) >= 0 {
                        format!("register '{}' used where a constant is expected", t.s)
                    } else {
                        format!("undefined symbol '{}'", t.s)
                    };
                    return Err(ExprError::new(msg, t.col));
                }
                if self.unresolved.is_none() {
                    self.unresolved = Some((t.s.clone(), t.col));
                }
                Ok(0)
            }
            TokKind::Op => match &*t.s {
                "(" => {
                    let v = self.ternary()?;
                    self.expect_op(")")?;
                    Ok(v)
                }
                "." => {
                    self.used_dot = true;
                    Ok(if self.mode.dot_bytes { self.mode.pc.wrapping_mul(2) } else { self.mode.pc })
                }
                s => Err(ExprError::new(format!("unexpected '{s}' in expression"), t.col)),
            },
            TokKind::Param => {
                Err(ExprError::new(format!("macro parameter {} has no value (missing macro argument?)", t.s), t.col))
            }
            TokKind::Str => Err(ExprError::new("string not allowed in expression", t.col)),
            TokKind::Dir => Err(ExprError::new(format!("unexpected '{}' in expression", t.text()), t.col)),
        }
    }

    fn call(&mut self, f: &Token) -> R {
        self.pos += 1; // '('
        if &*f.lc == "defined" {
            return match self.toks.get(self.pos) {
                Some(t) if t.k == TokKind::Id => {
                    self.pos += 1;
                    self.expect_op(")")?;
                    Ok(self.env.is_defined(&t.lc) as i64)
                }
                Some(t) => Err(ExprError::new("defined() expects a symbol name", t.col)),
                None => Err(ExprError::new("defined() expects a symbol name", self.last_col())),
            };
        }
        let a = self.ternary()?;
        self.expect_op(")")?;
        Ok(match &*f.lc {
            "low" | "lo8" | "byte1" | "pm_lo8" => a & 0xff,
            "high" | "hi8" | "byte2" | "pm_hi8" => shr(a, 8) & 0xff,
            "byte3" | "hh8" | "hlo8" | "pm_hh8" => shr(a, 16) & 0xff,
            "byte4" | "hhi8" => shr(a, 24) & 0xff,
            "lwrd" => a & 0xffff,
            "hwrd" => shr(a, 16) & 0xffff,
            "page" => shr(a, 16) & 0x3f,
            "exp2" => {
                if !(0..=52).contains(&a) {
                    return self.value_error(format!("EXP2 argument {a} out of range (0..52)"), f.col);
                }
                1i64 << a
            }
            "log2" => {
                if a <= 0 {
                    return self.value_error("LOG2 argument must be positive", f.col);
                }
                63 - a.leading_zeros() as i64
            }
            "abs" => a.wrapping_abs(),
            // Code labels are already word addresses, so GNU's pm()/gs() are the identity here.
            "pm" | "gs" => a,
            _ => return Err(ExprError::new(format!("unknown function '{}'", f.s), f.col)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    struct Env;
    impl ExprEnv for Env {
        fn lookup(&mut self, lc: &str, _col: u32, _strict: bool, _depth: u32) -> Result<Option<i64>, ExprError> {
            Ok(if lc == "ten" { Some(10) } else { None })
        }
        fn is_defined(&self, lc: &str) -> bool {
            lc == "ten"
        }
    }

    fn ev(src: &str, strict: bool) -> Result<EvalResult, ExprError> {
        let l = tokenize(src);
        evaluate(&l[0].toks, &mut Env, EvalMode { pc: 5, strict, dot_bytes: false, depth: 0 }, 1)
    }

    fn val(src: &str) -> i64 {
        ev(src, true).unwrap().value
    }

    #[test]
    fn precedence_and_functions() {
        assert_eq!(val("1 + 2 * 3"), 7);
        assert_eq!(val("(1 << 4) | 3"), 0x13);
        assert_eq!(val("~0x0F & 0xFF"), 0xf0);
        assert_eq!(val("~0"), -1);
        assert_eq!(val("0xFFFFFFFF & 0xFFFFFFFF"), -1);
        assert_eq!(val("10 > 3 ? 4 : 5"), 4);
        assert_eq!(val("-7 / 2"), -3);
        assert_eq!(val("-7 % 2"), -1);
        assert_eq!(val("-1 >> 70"), -1);
        assert_eq!(val("LOG2(64) + EXP2(2)"), 10);
        assert_eq!(val("defined(ten) + defined(nope)"), 1);
        assert_eq!(val("PC + ten"), 15);
        assert_eq!(val("high(0x1234) + byte3(0x123456)"), 0x12 + 0x12);
    }

    #[test]
    fn errors() {
        assert_eq!(ev("1/0", true).unwrap_err().msg, "division by zero");
        assert_eq!(ev("(1 + 2", true).unwrap_err().msg, "expected ')'");
        assert_eq!(ev("nope", true).unwrap_err().msg, "undefined symbol 'nope'");
        assert_eq!(ev("r5 + 1", true).unwrap_err().msg, "register 'r5' used where a constant is expected");
        assert_eq!(ev("foo(1)", true).unwrap_err().msg, "unknown function 'foo'");
        assert_eq!(ev("1 2", true).unwrap_err().msg, "unexpected '2' in expression");
        let r = ev("nope / 0", false).unwrap();
        assert_eq!(r.value, 0);
        assert_eq!(&*r.unresolved.unwrap().0, "nope");
        let deep = format!("{}1{}", "(".repeat(1000), ")".repeat(1000));
        assert_eq!(ev(&deep, true).unwrap_err().msg, "expression is nested too deeply");
        let neg = format!("{}1", "-".repeat(100_000));
        assert!(ev(&neg, true).is_err());
    }
}
