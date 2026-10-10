//! The instruction executor: a dense `match` over the pre-decoded [`Op`].
//!
//! Reference: ARM DDI 0403E.e chapter A7 (instruction semantics) and Cortex-M4 TRM ARM DDI 0439B
//! table 3-1 (cycle counts). The hot path allocates nothing.
//!
//! Cycle model (Cortex-M4, pipeline effects simplified):
//! * data processing, bit-field, extend, saturate, MUL/MLA/MLS and the long multiplies: 1
//! * SDIV/UDIV: 2 + ceil(quotient bits / 3), at most 12 (the TRM gives 2-12 depending on operands)
//! * single loads/stores: 2, LDRD/STRD: 3, LDM/STM/PUSH/POP: 1 + N (+ 2 pipeline refill when PC
//!   is loaded), LDREX/STREX: 2
//! * taken branches: 1 + 2 (pipeline refill), not-taken conditional branches / CBZ: 1;
//!   BL/BX/BLX: 3, TBB/TBH: 2 + 2
//! * MRS/MSR: 2, ISB: 3, other barriers and hints: 1
//! * the back-to-back load/store pipelining discount (consecutive LDRs after the first cost 1)
//!   and flash wait states are not modelled.

use mcs_core::arm::disasm::it_advance;
use mcs_core::arm::thumb::*;

use super::cpu::{CONTROL_NPRIV, CONTROL_SPSEL};
use super::machine::Machine;
use super::nvic::*;
use super::scb::*;

/// AddWithCarry: (result, carry out, signed overflow).
#[inline(always)]
pub fn add_with_carry(x: u32, y: u32, c: bool) -> (u32, bool, bool) {
    let sum = x as u64 + y as u64 + c as u64;
    let r = sum as u32;
    (r, sum >> 32 != 0, ((x ^ r) & (y ^ r)) >> 31 != 0)
}

/// Shift with carry out. `amt` may be 0 (carry unchanged) up to 255.
#[inline(always)]
pub fn shift_c(v: u32, kind: u8, amt: u32, c: bool) -> (u32, bool) {
    if amt == 0 {
        return (v, c);
    }
    match kind {
        SH_LSL => {
            if amt < 32 {
                (v << amt, (v >> (32 - amt)) & 1 != 0)
            } else if amt == 32 {
                (0, v & 1 != 0)
            } else {
                (0, false)
            }
        }
        SH_LSR => {
            if amt < 32 {
                (v >> amt, (v >> (amt - 1)) & 1 != 0)
            } else if amt == 32 {
                (0, v >> 31 != 0)
            } else {
                (0, false)
            }
        }
        SH_ASR => {
            if amt < 32 {
                (((v as i32) >> amt) as u32, (v >> (amt - 1)) & 1 != 0)
            } else {
                (((v as i32) >> 31) as u32, v >> 31 != 0)
            }
        }
        SH_ROR => {
            let a = amt & 31;
            if a == 0 {
                (v, v >> 31 != 0)
            } else {
                let r = v.rotate_right(a);
                (r, r >> 31 != 0)
            }
        }
        _ => (((c as u32) << 31) | (v >> 1), v & 1 != 0),
    }
}

#[inline(always)]
fn ssat(v: i32, n: u32) -> (u32, bool) {
    let max = ((1i64 << (n - 1)) - 1) as i32;
    let min = (-(1i64 << (n - 1))) as i32;
    if v > max {
        (max as u32, true)
    } else if v < min {
        (min as u32, true)
    } else {
        (v as u32, false)
    }
}

#[inline(always)]
fn usat(v: i32, n: u32) -> (u32, bool) {
    let max = ((1i64 << n) - 1) as i32 as i64;
    if (v as i64) > max {
        (max as u32, true)
    } else if v < 0 {
        (0, true)
    } else {
        (v as u32, false)
    }
}

impl Machine {
    #[inline(always)]
    fn set_nz(&mut self, v: u32) {
        self.cpu.n = (v as i32) < 0;
        self.cpu.z = v == 0;
    }

    /// Effective address and write-back value of a load/store (`rn`, `imm`, `aux`).
    #[inline(always)]
    fn ea(&self, i: &Insn, pc: u32) -> (u32, u32) {
        let base = self.cpu.r[(i.rn & 15) as usize];
        match i.aux {
            AM_OFFSET => (base.wrapping_add(i.imm), 0),
            AM_PRE => {
                let a = base.wrapping_add(i.imm);
                (a, a)
            }
            AM_POST => (base, base.wrapping_add(i.imm)),
            AM_REG => (base.wrapping_add(self.cpu.r[(i.rm & 15) as usize] << (i.amt & 31)), 0),
            _ => ((pc.wrapping_add(4) & !3).wrapping_add(i.imm), 0),
        }
    }

    /// Branch with interworking and EXC_RETURN detection (BX, POP {pc}, LDR pc, ...).
    pub(crate) fn bx_write(&mut self, v: u32, pc: u32) {
        if v >= 0xf000_0000 && self.cpu.ipsr != 0 {
            self.exception_return(v, pc);
        } else if v & 1 == 0 {
            self.raise_fault(pc, EXC_USAGEFAULT, UFSR_INVSTATE);
        } else {
            self.cpu.pc = v & !1;
        }
    }

    #[inline(always)]
    fn unaligned_trap(&mut self, pc: u32) {
        self.raise_fault(pc, EXC_USAGEFAULT, UFSR_UNALIGNED);
    }

    /// Executes `i`, located at `pc`. `cpu.pc` already points past it and `r[15]` holds `pc + 4`.
    #[inline(always)]
    pub(crate) fn exec(&mut self, i: &Insn, pc: u32) {
        let it = self.cpu.itstate;
        if it != 0 && i.op != Op::IT {
            self.cpu.itstate = it_advance(it);
            if !self.cpu.cond(it >> 4) {
                self.cpu.cycles += 1;
                return;
            }
        }
        let setf = i.s == S_YES || (i.s == S_NOT_IT && it == 0);
        macro_rules! r {
            ($x:expr) => {
                self.cpu.r[($x & 15) as usize]
            };
        }
        // Flag-setting arithmetic: result, carry and overflow into rd and (if setf) NZCV.
        macro_rules! arith {
            ($x:expr, $y:expr, $cin:expr) => {{
                let (res, c, v) = add_with_carry($x, $y, $cin);
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                    self.cpu.c = c;
                    self.cpu.v = v;
                }
                self.cpu.cycles += 1;
            }};
        }
        macro_rules! cmp_flags {
            ($x:expr, $y:expr, $cin:expr) => {{
                let (res, c, v) = add_with_carry($x, $y, $cin);
                self.set_nz(res);
                self.cpu.c = c;
                self.cpu.v = v;
                self.cpu.cycles += 1;
            }};
        }
        // Logical op with immediate operand (carry from the modified immediate).
        macro_rules! logic_i {
            ($f:expr) => {{
                let res: u32 = $f(r!(i.rn), i.imm);
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                    if i.aux != 2 {
                        self.cpu.c = i.aux != 0;
                    }
                }
                self.cpu.cycles += 1;
            }};
        }
        macro_rules! test_i {
            ($f:expr) => {{
                let res: u32 = $f(r!(i.rn), i.imm);
                self.set_nz(res);
                if i.aux != 2 {
                    self.cpu.c = i.aux != 0;
                }
                self.cpu.cycles += 1;
            }};
        }
        // Register operand with immediate shift: (value, carry).
        macro_rules! op2 {
            () => {
                shift_c(r!(i.rm), i.shift, i.amt as u32, self.cpu.c)
            };
        }
        macro_rules! logic_r {
            ($f:expr) => {{
                let (b, co) = op2!();
                let res: u32 = $f(r!(i.rn), b);
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                    self.cpu.c = co;
                }
                self.cpu.cycles += 1;
            }};
        }
        macro_rules! test_r {
            ($f:expr) => {{
                let (b, co) = op2!();
                let res: u32 = $f(r!(i.rn), b);
                self.set_nz(res);
                self.cpu.c = co;
                self.cpu.cycles += 1;
            }};
        }
        // Loads: size in bytes, extension closure.
        macro_rules! load {
            ($size:expr, $ext:expr) => {{
                let (a, wb) = self.ea(i, pc);
                match self.mem_read(a, $size) {
                    Some(raw) => {
                        let v: u32 = $ext(raw);
                        if i.aux == AM_PRE || i.aux == AM_POST {
                            r!(i.rn) = wb;
                        }
                        if i.rd == 15 {
                            self.cpu.cycles += 4;
                            self.bx_write(v, pc);
                        } else {
                            r!(i.rd) = v;
                            self.cpu.cycles += 2;
                        }
                    }
                    None => self.data_fault(pc, a),
                }
            }};
        }
        macro_rules! store {
            ($size:expr) => {{
                let (a, wb) = self.ea(i, pc);
                let v = r!(i.rd);
                if self.mem_write(a, $size, v) {
                    if i.aux == AM_PRE || i.aux == AM_POST {
                        r!(i.rn) = wb;
                    }
                    self.cpu.cycles += 2;
                } else {
                    self.data_fault(pc, a);
                }
            }};
        }
        match i.op {
            // ---- data processing, immediate -------------------------------------------------
            Op::ADD_I => arith!(r!(i.rn), i.imm, false),
            Op::ADC_I => arith!(r!(i.rn), i.imm, self.cpu.c),
            Op::SUB_I => arith!(r!(i.rn), !i.imm, true),
            Op::SBC_I => arith!(r!(i.rn), !i.imm, self.cpu.c),
            Op::RSB_I => arith!(!r!(i.rn), i.imm, true),
            Op::AND_I => logic_i!(|a: u32, b: u32| a & b),
            Op::BIC_I => logic_i!(|a: u32, b: u32| a & !b),
            Op::ORR_I => logic_i!(|a: u32, b: u32| a | b),
            Op::ORN_I => logic_i!(|a: u32, b: u32| a | !b),
            Op::EOR_I => logic_i!(|a: u32, b: u32| a ^ b),
            Op::MOV_I | Op::MVN_I => {
                let res = if i.op == Op::MOV_I { i.imm } else { !i.imm };
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                    if i.aux != 2 {
                        self.cpu.c = i.aux != 0;
                    }
                }
                self.cpu.cycles += 1;
            }
            Op::TST_I => test_i!(|a: u32, b: u32| a & b),
            Op::TEQ_I => test_i!(|a: u32, b: u32| a ^ b),
            Op::CMP_I => cmp_flags!(r!(i.rn), !i.imm, true),
            Op::CMN_I => cmp_flags!(r!(i.rn), i.imm, false),
            // ---- data processing, register ---------------------------------------------------
            Op::ADD_R => {
                let (b, _) = op2!();
                if i.rd == 15 {
                    // ADD pc, Rm (narrow high-register form): branch, no flags.
                    self.cpu.pc = r!(i.rn).wrapping_add(b) & !1;
                    self.cpu.cycles += 3;
                } else {
                    arith!(r!(i.rn), b, false);
                }
            }
            Op::ADC_R => {
                let (b, _) = op2!();
                arith!(r!(i.rn), b, self.cpu.c)
            }
            Op::SUB_R => {
                let (b, _) = op2!();
                arith!(r!(i.rn), !b, true)
            }
            Op::SBC_R => {
                let (b, _) = op2!();
                arith!(r!(i.rn), !b, self.cpu.c)
            }
            Op::RSB_R => {
                let (b, _) = op2!();
                arith!(!r!(i.rn), b, true)
            }
            Op::AND_R => logic_r!(|a: u32, b: u32| a & b),
            Op::BIC_R => logic_r!(|a: u32, b: u32| a & !b),
            Op::ORR_R => logic_r!(|a: u32, b: u32| a | b),
            Op::ORN_R => logic_r!(|a: u32, b: u32| a | !b),
            Op::EOR_R => logic_r!(|a: u32, b: u32| a ^ b),
            Op::MOV_R => {
                let (res, co) = op2!();
                if i.rd == 15 {
                    self.cpu.pc = res & !1;
                    self.cpu.cycles += 3;
                } else {
                    r!(i.rd) = res;
                    if setf {
                        self.set_nz(res);
                        self.cpu.c = co;
                    }
                    self.cpu.cycles += 1;
                }
            }
            Op::MVN_R => {
                let (b, co) = op2!();
                let res = !b;
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                    self.cpu.c = co;
                }
                self.cpu.cycles += 1;
            }
            Op::TST_R => test_r!(|a: u32, b: u32| a & b),
            Op::TEQ_R => test_r!(|a: u32, b: u32| a ^ b),
            Op::CMP_R => {
                let (b, _) = op2!();
                cmp_flags!(r!(i.rn), !b, true)
            }
            Op::CMN_R => {
                let (b, _) = op2!();
                cmp_flags!(r!(i.rn), b, false)
            }
            Op::LSL_RV | Op::LSR_RV | Op::ASR_RV | Op::ROR_RV => {
                let kind = match i.op {
                    Op::LSL_RV => SH_LSL,
                    Op::LSR_RV => SH_LSR,
                    Op::ASR_RV => SH_ASR,
                    _ => SH_ROR,
                };
                let (res, co) = shift_c(r!(i.rn), kind, r!(i.rm) & 0xff, self.cpu.c);
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                    self.cpu.c = co;
                }
                self.cpu.cycles += 1;
            }
            Op::MOVT => {
                r!(i.rd) = (r!(i.rd) & 0xffff) | (i.imm << 16);
                self.cpu.cycles += 1;
            }
            Op::ADR => {
                r!(i.rd) = (pc.wrapping_add(4) & !3).wrapping_add(i.imm);
                self.cpu.cycles += 1;
            }
            // ---- multiply / divide ------------------------------------------------------------
            Op::MUL => {
                let res = r!(i.rn).wrapping_mul(r!(i.rm));
                r!(i.rd) = res;
                if setf {
                    self.set_nz(res);
                }
                self.cpu.cycles += 1;
            }
            Op::MLA => {
                r!(i.rd) = r!(i.ra).wrapping_add(r!(i.rn).wrapping_mul(r!(i.rm)));
                self.cpu.cycles += 1;
            }
            Op::MLS => {
                r!(i.rd) = r!(i.ra).wrapping_sub(r!(i.rn).wrapping_mul(r!(i.rm)));
                self.cpu.cycles += 1;
            }
            Op::UMULL => {
                let p = r!(i.rn) as u64 * r!(i.rm) as u64;
                r!(i.rd) = p as u32;
                r!(i.ra) = (p >> 32) as u32;
                self.cpu.cycles += 1;
            }
            Op::SMULL => {
                let p = r!(i.rn) as i32 as i64 * r!(i.rm) as i32 as i64;
                r!(i.rd) = p as u32;
                r!(i.ra) = (p >> 32) as u32;
                self.cpu.cycles += 1;
            }
            Op::UMLAL => {
                let acc = (r!(i.ra) as u64) << 32 | r!(i.rd) as u64;
                let p = (r!(i.rn) as u64 * r!(i.rm) as u64).wrapping_add(acc);
                r!(i.rd) = p as u32;
                r!(i.ra) = (p >> 32) as u32;
                self.cpu.cycles += 1;
            }
            Op::SMLAL => {
                let acc = ((r!(i.ra) as u64) << 32 | r!(i.rd) as u64) as i64;
                let p = (r!(i.rn) as i32 as i64 * r!(i.rm) as i32 as i64).wrapping_add(acc);
                r!(i.rd) = p as u32;
                r!(i.ra) = (p >> 32) as u32;
                self.cpu.cycles += 1;
            }
            #[allow(clippy::manual_checked_ops)]
            Op::UDIV | Op::SDIV => {
                let (n, d) = (r!(i.rn), r!(i.rm));
                if d == 0 {
                    if self.scb.ccr & CCR_DIV_0_TRP != 0 {
                        self.raise_fault(pc, EXC_USAGEFAULT, UFSR_DIVBYZERO);
                        return;
                    }
                    r!(i.rd) = 0;
                    self.cpu.cycles += 2;
                } else {
                    let (res, nb, db) = if i.op == Op::UDIV {
                        (n / d, n.leading_zeros(), d.leading_zeros())
                    } else {
                        ((n as i32).wrapping_div(d as i32) as u32, (n as i32).unsigned_abs().leading_zeros(), (d as i32).unsigned_abs().leading_zeros())
                    };
                    r!(i.rd) = res;
                    let qbits = db.saturating_sub(nb) + 1;
                    self.cpu.cycles += (2 + qbits.div_ceil(3)).min(12) as u64;
                }
            }
            // ---- bit fields, extend, saturate -------------------------------------------------
            Op::BFC | Op::BFI => {
                let w = i.aux as u32;
                let lsb = i.amt as u32;
                let mask = (if w >= 32 { u32::MAX } else { (1u32 << w) - 1 }).wrapping_shl(lsb);
                let src = if i.op == Op::BFI { r!(i.rn) } else { 0 };
                r!(i.rd) = (r!(i.rd) & !mask) | (src.wrapping_shl(lsb) & mask);
                self.cpu.cycles += 1;
            }
            Op::UBFX => {
                let w = i.aux as u32;
                let m = if w >= 32 { u32::MAX } else { (1u32 << w) - 1 };
                r!(i.rd) = (r!(i.rn) >> (i.amt & 31)) & m;
                self.cpu.cycles += 1;
            }
            Op::SBFX => {
                let sh = 32 - (i.aux as u32).clamp(1, 32);
                let v = r!(i.rn) >> (i.amt & 31);
                r!(i.rd) = (((v << sh) as i32) >> sh) as u32;
                self.cpu.cycles += 1;
            }
            Op::CLZ => {
                r!(i.rd) = r!(i.rm).leading_zeros();
                self.cpu.cycles += 1;
            }
            Op::RBIT => {
                r!(i.rd) = r!(i.rm).reverse_bits();
                self.cpu.cycles += 1;
            }
            Op::REV => {
                r!(i.rd) = r!(i.rm).swap_bytes();
                self.cpu.cycles += 1;
            }
            Op::REV16 => {
                let v = r!(i.rm);
                r!(i.rd) = ((v & 0x00ff_00ff) << 8) | ((v >> 8) & 0x00ff_00ff);
                self.cpu.cycles += 1;
            }
            Op::REVSH => {
                r!(i.rd) = (r!(i.rm) as u16).swap_bytes() as i16 as i32 as u32;
                self.cpu.cycles += 1;
            }
            Op::SXTB | Op::SXTH | Op::UXTB | Op::UXTH => {
                let v = r!(i.rm).rotate_right(i.amt as u32);
                let e = match i.op {
                    Op::SXTB => v as i8 as i32 as u32,
                    Op::SXTH => v as i16 as i32 as u32,
                    Op::UXTB => v & 0xff,
                    _ => v & 0xffff,
                };
                r!(i.rd) = if i.rn == 15 { e } else { r!(i.rn).wrapping_add(e) };
                self.cpu.cycles += 1;
            }
            Op::SSAT | Op::USAT => {
                let (x, _) = shift_c(r!(i.rn), i.shift, i.amt as u32, false);
                let (res, sat) = if i.op == Op::SSAT { ssat(x as i32, i.imm) } else { usat(x as i32, i.imm) };
                r!(i.rd) = res;
                if sat {
                    self.cpu.q = true;
                }
                self.cpu.cycles += 1;
            }
            Op::QADD | Op::QSUB | Op::QDADD | Op::QDSUB => {
                let a = r!(i.rm) as i32 as i64;
                let b = r!(i.rn) as i32 as i64;
                let mut sat = false;
                let mut clamp = |x: i64| -> i64 {
                    if x > i32::MAX as i64 {
                        sat = true;
                        i32::MAX as i64
                    } else if x < i32::MIN as i64 {
                        sat = true;
                        i32::MIN as i64
                    } else {
                        x
                    }
                };
                let res = match i.op {
                    Op::QADD => clamp(a + b),
                    Op::QSUB => clamp(a - b),
                    Op::QDADD => {
                        let d = clamp(2 * b);
                        clamp(a + d)
                    }
                    _ => {
                        let d = clamp(2 * b);
                        clamp(a - d)
                    }
                };
                r!(i.rd) = res as u32;
                if sat {
                    self.cpu.q = true;
                }
                self.cpu.cycles += 1;
            }
            // ---- loads and stores --------------------------------------------------------------
            Op::LDR | Op::LDRT => load!(4, |x: u32| x),
            Op::LDRB | Op::LDRBT => load!(1, |x: u32| x),
            Op::LDRH | Op::LDRHT => load!(2, |x: u32| x),
            Op::LDRSB | Op::LDRSBT => load!(1, |x: u32| x as i8 as i32 as u32),
            Op::LDRSH | Op::LDRSHT => load!(2, |x: u32| x as i16 as i32 as u32),
            Op::STR | Op::STRT => store!(4),
            Op::STRB | Op::STRBT => store!(1),
            Op::STRH | Op::STRHT => store!(2),
            Op::LDRD | Op::STRD => {
                let (a, wb) = self.ea(i, pc);
                if a & 3 != 0 {
                    return self.unaligned_trap(pc);
                }
                if i.op == Op::LDRD {
                    let (Some(lo), Some(hi)) = (self.mem_read(a, 4), self.mem_read(a.wrapping_add(4), 4)) else {
                        return self.data_fault(pc, a);
                    };
                    if i.aux == AM_PRE || i.aux == AM_POST {
                        r!(i.rn) = wb;
                    }
                    r!(i.rd) = lo;
                    r!(i.ra) = hi;
                } else {
                    let (lo, hi) = (r!(i.rd), r!(i.ra));
                    if !(self.mem_write(a, 4, lo) && self.mem_write(a.wrapping_add(4), 4, hi)) {
                        return self.data_fault(pc, a);
                    }
                    if i.aux == AM_PRE || i.aux == AM_POST {
                        r!(i.rn) = wb;
                    }
                }
                self.cpu.cycles += 3;
            }
            Op::LDM | Op::LDMDB | Op::POP => {
                let n = i.imm.count_ones();
                let base = if i.op == Op::POP { r!(13) } else { r!(i.rn) };
                let start = if i.op == Op::LDMDB { base.wrapping_sub(4 * n) } else { base };
                if start & 3 != 0 {
                    return self.unaligned_trap(pc);
                }
                let mut a = start;
                let mut list = i.imm;
                let mut target = None;
                while list != 0 {
                    let k = list.trailing_zeros();
                    list &= list - 1;
                    let Some(v) = self.mem_read(a, 4) else { return self.data_fault(pc, a) };
                    if k == 15 {
                        target = Some(v);
                    } else {
                        self.cpu.r[k as usize] = v;
                    }
                    a = a.wrapping_add(4);
                }
                if i.op == Op::POP {
                    self.cpu.r[13] = base.wrapping_add(4 * n);
                } else if i.aux != 0 && i.imm & (1 << (i.rn & 15)) == 0 {
                    self.cpu.r[(i.rn & 15) as usize] = if i.op == Op::LDMDB { start } else { base.wrapping_add(4 * n) };
                }
                self.cpu.cycles += 1 + n as u64;
                if let Some(v) = target {
                    self.cpu.cycles += 2;
                    self.bx_write(v, pc);
                }
            }
            Op::STM | Op::STMDB | Op::PUSH => {
                let n = i.imm.count_ones();
                let base = if i.op == Op::PUSH { r!(13) } else { r!(i.rn) };
                let start = if i.op == Op::STM { base } else { base.wrapping_sub(4 * n) };
                if start & 3 != 0 {
                    return self.unaligned_trap(pc);
                }
                let mut a = start;
                let mut list = i.imm;
                while list != 0 {
                    let k = list.trailing_zeros();
                    list &= list - 1;
                    let v = self.cpu.r[k as usize];
                    if !self.mem_write(a, 4, v) {
                        return self.data_fault(pc, a);
                    }
                    a = a.wrapping_add(4);
                }
                if i.op == Op::PUSH {
                    self.cpu.r[13] = start;
                } else if i.aux != 0 {
                    self.cpu.r[(i.rn & 15) as usize] = if i.op == Op::STM { base.wrapping_add(4 * n) } else { start };
                }
                self.cpu.cycles += 1 + n as u64;
            }
            Op::LDREX | Op::LDREXB | Op::LDREXH => {
                let a = r!(i.rn).wrapping_add(i.imm);
                let size = match i.op {
                    Op::LDREX => 4,
                    Op::LDREXH => 2,
                    _ => 1,
                };
                if a & (size - 1) != 0 {
                    return self.unaligned_trap(pc);
                }
                match self.mem_read(a, size) {
                    Some(v) => {
                        r!(i.rd) = v;
                        self.cpu.excl_valid = true;
                        self.cpu.excl_addr = a;
                        self.cpu.cycles += 2;
                    }
                    None => self.data_fault(pc, a),
                }
            }
            Op::STREX | Op::STREXB | Op::STREXH => {
                let a = r!(i.rn).wrapping_add(i.imm);
                let size = match i.op {
                    Op::STREX => 4,
                    Op::STREXH => 2,
                    _ => 1,
                };
                if a & (size - 1) != 0 {
                    return self.unaligned_trap(pc);
                }
                if self.cpu.excl_valid && self.cpu.excl_addr == a {
                    let v = r!(i.rm);
                    if !self.mem_write(a, size, v) {
                        return self.data_fault(pc, a);
                    }
                    r!(i.rd) = 0;
                } else {
                    r!(i.rd) = 1;
                }
                self.cpu.excl_valid = false;
                self.cpu.cycles += 2;
            }
            Op::CLREX => {
                self.cpu.excl_valid = false;
                self.cpu.cycles += 1;
            }
            Op::PLD | Op::PLI => self.cpu.cycles += 1,
            // ---- branches ----------------------------------------------------------------------
            Op::B => {
                self.cpu.pc = pc.wrapping_add(4).wrapping_add(i.imm);
                self.cpu.cycles += 3;
            }
            Op::B_COND => {
                if self.cpu.cond(i.aux) {
                    self.cpu.pc = pc.wrapping_add(4).wrapping_add(i.imm);
                    self.cpu.cycles += 3;
                } else {
                    self.cpu.cycles += 1;
                }
            }
            Op::BL => {
                self.cpu.r[14] = pc.wrapping_add(4) | 1;
                self.cpu.pc = pc.wrapping_add(4).wrapping_add(i.imm);
                self.cpu.cycles += 3;
            }
            Op::BX => {
                self.cpu.cycles += 3;
                self.bx_write(r!(i.rm), pc);
            }
            Op::BLX_R => {
                let t = r!(i.rm);
                self.cpu.r[14] = pc.wrapping_add(2) | 1;
                self.cpu.cycles += 3;
                self.bx_write(t, pc);
            }
            Op::CBZ | Op::CBNZ => {
                if (r!(i.rn) == 0) == (i.op == Op::CBZ) {
                    self.cpu.pc = pc.wrapping_add(4).wrapping_add(i.imm);
                    self.cpu.cycles += 3;
                } else {
                    self.cpu.cycles += 1;
                }
            }
            Op::TBB | Op::TBH => {
                let idx = r!(i.rm);
                let (a, size) = if i.op == Op::TBB { (r!(i.rn).wrapping_add(idx), 1) } else { (r!(i.rn).wrapping_add(idx << 1), 2) };
                match self.mem_read(a, size) {
                    Some(v) => {
                        self.cpu.pc = pc.wrapping_add(4).wrapping_add(v << 1);
                        self.cpu.cycles += 4;
                    }
                    None => self.data_fault(pc, a),
                }
            }
            // ---- system ------------------------------------------------------------------------
            Op::IT => {
                self.cpu.itstate = i.imm as u8;
                self.cpu.cycles += 1;
            }
            Op::MRS => {
                let c = &self.cpu;
                let (apsr, ipsr) = (c.apsr(), c.ipsr as u32);
                let v = match i.imm {
                    0 | 2 => apsr,
                    1 | 3 => apsr | ipsr,
                    5 | 7 => ipsr,
                    8 => c.msp(),
                    9 => c.psp(),
                    16 => c.primask as u32,
                    17 | 18 => c.basepri as u32,
                    19 => c.faultmask as u32,
                    20 => c.control as u32,
                    _ => 0,
                };
                // Privileged-only registers read as zero in unprivileged code.
                let v = if i.imm >= 8 && !c.privileged() { 0 } else { v };
                r!(i.rd) = v;
                self.cpu.cycles += 2;
            }
            Op::MSR => {
                let v = r!(i.rn);
                let priv_ = self.cpu.privileged();
                match i.imm {
                    0..=3 => {
                        if i.aux & 2 != 0 {
                            self.cpu.set_apsr(v & 0xf800_0000);
                        }
                    }
                    8 if priv_ => self.cpu.set_msp(v),
                    9 if priv_ => self.cpu.set_psp(v),
                    16 if priv_ => {
                        self.cpu.primask = v & 1 != 0;
                        self.nvic.dirty = true;
                    }
                    17 if priv_ => {
                        self.cpu.basepri = (v as u8) & (0xffu16 << (8 - self.nvic.prio_bits)) as u8;
                        self.nvic.dirty = true;
                    }
                    18 if priv_ => {
                        let nv = (v as u8) & (0xffu16 << (8 - self.nvic.prio_bits)) as u8;
                        if nv != 0 && (self.cpu.basepri == 0 || nv < self.cpu.basepri) {
                            self.cpu.basepri = nv;
                            self.nvic.dirty = true;
                        }
                    }
                    19 if priv_ => {
                        if self.exec_prio() > -1 {
                            self.cpu.faultmask = v & 1 != 0;
                            self.nvic.dirty = true;
                        }
                    }
                    20 if priv_ => {
                        let mut c = (self.cpu.control & !CONTROL_NPRIV) | (v as u8 & CONTROL_NPRIV);
                        if self.cpu.ipsr == 0 {
                            c = (c & !CONTROL_SPSEL) | (v as u8 & CONTROL_SPSEL);
                            self.cpu.select_sp(c & CONTROL_SPSEL != 0);
                        }
                        self.cpu.control = c;
                    }
                    _ => {}
                }
                self.cpu.cycles += 2;
            }
            Op::CPS => {
                if self.cpu.privileged() {
                    let dis = i.aux != 0;
                    if i.imm & 2 != 0 {
                        self.cpu.primask = dis;
                    }
                    if i.imm & 1 != 0 && (!dis || self.exec_prio() > -1) {
                        self.cpu.faultmask = dis;
                    }
                    self.nvic.dirty = true;
                }
                self.cpu.cycles += 1;
            }
            Op::SVC => {
                self.cpu.cycles += 1;
                let exec = self.exec_prio();
                if self.nvic.group_prio(EXC_SVCALL) < exec {
                    self.nvic.set_sys_pending(EXC_SVCALL);
                } else {
                    // SVCall cannot preempt: escalates to HardFault.
                    self.raise_fault(pc, EXC_HARDFAULT, 0);
                    self.scb.hfsr |= HFSR_FORCED;
                }
            }
            Op::BKPT => {
                self.cpu.cycles += 1;
                self.cpu.stop = super::cpu::StopReason::Bkpt;
                self.stop_limit = 0;
            }
            Op::NOP | Op::YIELD | Op::DBG | Op::DMB | Op::DSB => self.cpu.cycles += 1,
            Op::ISB => self.cpu.cycles += 3,
            Op::SEV => {
                self.cpu.event = true;
                self.cpu.cycles += 1;
            }
            Op::WFE => {
                self.cpu.cycles += 1;
                if self.cpu.event {
                    self.cpu.event = false;
                } else {
                    self.cpu.sleeping = true;
                    self.cpu.sleep_wfe = true;
                }
            }
            Op::WFI => {
                self.cpu.cycles += 1;
                if !self.nvic.any_pending() {
                    self.cpu.sleeping = true;
                    self.cpu.sleep_wfe = false;
                }
            }
            Op::UDF | Op::UNDEF => {
                self.cpu.cycles += 1;
                self.raise_fault(pc, EXC_USAGEFAULT, UFSR_UNDEFINSTR);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_with_carry_flags() {
        assert_eq!(add_with_carry(0x7fff_ffff, 1, false), (0x8000_0000, false, true));
        assert_eq!(add_with_carry(0xffff_ffff, 1, false), (0, true, false));
        assert_eq!(add_with_carry(5, !5, true), (0, true, false)); // 5 - 5
        assert_eq!(add_with_carry(0, !1, true), (0xffff_ffff, false, false)); // 0 - 1 borrows
    }

    #[test]
    fn shifter_carry_out() {
        assert_eq!(shift_c(0x8000_0001, SH_LSL, 1, false), (2, true));
        assert_eq!(shift_c(1, SH_LSL, 32, false), (0, true));
        assert_eq!(shift_c(1, SH_LSL, 33, true), (0, false));
        assert_eq!(shift_c(0x8000_0000, SH_LSR, 32, false), (0, true));
        assert_eq!(shift_c(0x8000_0000, SH_ASR, 40, false), (0xffff_ffff, true));
        assert_eq!(shift_c(0x1234_5678, SH_ROR, 8, false), (0x7812_3456, false));
        assert_eq!(shift_c(3, SH_RRX, 1, true), (0x8000_0001, true));
        assert_eq!(shift_c(7, SH_LSL, 0, true), (7, true), "zero shift keeps C");
    }

    #[test]
    fn saturation() {
        assert_eq!(ssat(1000, 8), (127, true));
        assert_eq!(ssat(-1000, 8), (0xffff_ff80, true));
        assert_eq!(ssat(-5, 8), (0xffff_fffb, false));
        assert_eq!(usat(-1, 8), (0, true));
        assert_eq!(usat(300, 8), (255, true));
        assert_eq!(usat(i32::MAX, 31), (i32::MAX as u32, false));
    }
}
