//! Executor for the ARMv7E-M DSP extension and the floating-point extensions (FPv4-SP,
//! FPv5-D16). Kept out of line so the integer hot path in [`super::exec`] stays compact.
//!
//! References: ARM DDI 0403E.e chapter A7 (DSP instructions: SADD16 ... USAT16; floating-point
//! VADD ... VSTR), A2.7 (FP pseudocode, see [`super::fpu`]), B1.4.7 (FPSCR) and B1.5.6 (exception
//! entry with floating-point context); Cortex-M4 TRM (ARM DDI 0439B) table 3-1 and chapter 7
//! (FPU cycle counts); Cortex-M7 TRM (ARM DDI 0489).
//!
//! Cycle model:
//! * every DSP instruction takes 1 cycle (Cortex-M4 TRM table 3-1: SIMD, saturating, packing,
//!   dual / word / halfword multiplies, long accumulates, USAD8);
//! * FPv4-SP (Cortex-M4F): VADD/VSUB/VMUL/VNMUL/VABS/VNEG/VMOV/VCMP/VCVT/VMRS/VMSR/VSEL 1,
//!   VMLA/VMLS/VNMLA/VNMLS/VFMA/VFMS/VFNMA/VFNMS 3, VDIV/VSQRT 14, VLDR/VSTR 2, VLDM/VSTM/VPUSH/
//!   VPOP 1 + number of words, two-register VMOV 2;
//! * FPv5-D16 (Cortex-M7) is dual issue with deeper latencies; it is approximated with the same
//!   single-issue counts, except double-precision VDIV/VSQRT which take 29 cycles.

use mcs_core::arm::thumb::*;
use mcs_core::arm::vfp::*;

use super::cpu::CONTROL_FPCA;
use super::exec::{shift_c, ssat, usat};
use super::fpu::*;
use super::machine::Machine;
use super::scb::*;

/// One lane of a packed operand, sign- or zero-extended.
#[inline(always)]
fn lane(v: u32, sh: u32, bits: u32, signed: bool) -> i32 {
    let x = (v >> sh) & ((1u32 << bits) - 1);
    if signed {
        ((x << (32 - bits)) as i32) >> (32 - bits)
    } else {
        x as i32
    }
}

/// Parallel add / subtract (SADD16 ... UHSUB8): (result, GE bits).
#[inline]
pub(crate) fn parallel(prefix: u8, kind: u8, a: u32, b: u32) -> (u32, u8) {
    let signed = prefix <= PP_SH;
    let eight = kind >= PK_ADD8;
    let (bits, lanes) = if eight { (8u32, 4u32) } else { (16, 2) };
    let mask = (1u32 << bits) - 1;
    let (mut res, mut ge) = (0u32, 0u8);
    for k in 0..lanes {
        let sh = k * bits;
        let x = lane(a, sh, bits, signed);
        // (lane of b paired with lane k of a, true when the lane adds)
        let (bl, add) = match kind {
            PK_ADD16 | PK_ADD8 => (k, true),
            PK_SUB16 | PK_SUB8 => (k, false),
            PK_ASX => {
                if k == 0 {
                    (1, false)
                } else {
                    (0, true)
                }
            }
            _ => {
                if k == 0 {
                    (1, true)
                } else {
                    (0, false)
                }
            }
        };
        let y = lane(b, bl * bits, bits, signed);
        let v = if add { x + y } else { x - y };
        let out = match prefix {
            PP_S | PP_U => v as u32,
            PP_Q => v.clamp(-(1 << (bits - 1)), (1 << (bits - 1)) - 1) as u32,
            PP_UQ => v.clamp(0, (1 << bits) - 1) as u32,
            _ => (v >> 1) as u32,
        };
        res |= (out & mask) << sh;
        let g = if signed || !add { v >= 0 } else { v >= 1 << bits };
        if g && (prefix == PP_S || prefix == PP_U) {
            ge |= if eight { 1 << k } else { 3 << (2 * k) };
        }
    }
    (res, ge)
}

/// Selects a half of a 32-bit register as a signed 16-bit value.
#[inline(always)]
fn half(v: u32, top: bool) -> i64 {
    (if top { (v >> 16) as i16 } else { v as i16 }) as i64
}

impl Machine {
    /// DSP and floating-point operations (everything outside the integer fast path).
    #[inline(never)]
    pub(crate) fn exec_ext(&mut self, i: &Insn, pc: u32) {
        if (i.op as u8) >= (Op::VLDR as u8) {
            self.exec_fp(i, pc);
        } else {
            self.exec_dsp(i);
        }
    }

    fn exec_dsp(&mut self, i: &Insn) {
        macro_rules! r {
            ($x:expr) => {
                self.cpu.r[($x & 15) as usize]
            };
        }
        let (rn, rm) = (r!(i.rn), r!(i.rm));
        match i.op {
            Op::PAR => {
                let (res, ge) = parallel(i.aux, i.shift, rn, rm);
                r!(i.rd) = res;
                if i.aux == PP_S || i.aux == PP_U {
                    self.cpu.ge = ge;
                }
            }
            Op::SEL => {
                let ge = self.cpu.ge;
                let mut m = 0u32;
                for k in 0..4 {
                    if ge >> k & 1 != 0 {
                        m |= 0xff << (8 * k);
                    }
                }
                r!(i.rd) = (rn & m) | (rm & !m);
            }
            Op::USAD8 | Op::USADA8 => {
                let mut sum = 0u32;
                for k in 0..4 {
                    let (x, y) = ((rn >> (8 * k)) & 0xff, (rm >> (8 * k)) & 0xff);
                    sum += x.abs_diff(y);
                }
                r!(i.rd) = if i.op == Op::USADA8 { sum.wrapping_add(r!(i.ra)) } else { sum };
            }
            Op::SSAT16 | Op::USAT16 => {
                let mut res = 0u32;
                for h in 0..2 {
                    let v = ((rn >> (16 * h)) as i16) as i32;
                    let (x, sat) = if i.op == Op::SSAT16 { ssat(v, i.imm) } else { usat(v, i.imm) };
                    res |= (x & 0xffff) << (16 * h);
                    self.cpu.q |= sat;
                }
                r!(i.rd) = res;
            }
            Op::PKH => {
                let (x, _) = shift_c(rm, i.shift, i.amt as u32, false);
                r!(i.rd) = if i.shift == SH_ASR { (rn & 0xffff_0000) | (x & 0xffff) } else { (x & 0xffff_0000) | (rn & 0xffff) };
            }
            Op::SXTB16 | Op::UXTB16 => {
                let v = rm.rotate_right(i.amt as u32);
                let (lo, hi) = if i.op == Op::SXTB16 { (v as i8 as i32 as u32 & 0xffff, (v >> 16) as i8 as i32 as u32 & 0xffff) } else { (v & 0xff, (v >> 16) & 0xff) };
                r!(i.rd) = if i.rn == 15 {
                    lo | hi << 16
                } else {
                    let a = r!(i.rn);
                    (a.wrapping_add(lo) & 0xffff) | (a.wrapping_add(hi << 16) & 0xffff_0000)
                };
            }
            Op::SMUL_XY | Op::SMLA_XY => {
                let p = half(rn, i.aux & 1 != 0) * half(rm, i.aux & 2 != 0);
                if i.op == Op::SMUL_XY {
                    r!(i.rd) = p as u32;
                } else {
                    let s = p + r!(i.ra) as i32 as i64;
                    r!(i.rd) = s as u32;
                    self.cpu.q |= s != s as i32 as i64;
                }
            }
            Op::SMULW | Op::SMLAW => {
                let mut p = rn as i32 as i64 * half(rm, i.aux & 2 != 0);
                if i.op == Op::SMLAW {
                    p += (r!(i.ra) as i32 as i64) << 16;
                }
                let res = (p >> 16) as u32;
                r!(i.rd) = res;
                if i.op == Op::SMLAW {
                    self.cpu.q |= (p >> 16) != res as i32 as i64;
                }
            }
            Op::SMUAD | Op::SMUSD | Op::SMLAD | Op::SMLSD => {
                let m = if i.aux & 1 != 0 { rm.rotate_right(16) } else { rm };
                let (p1, p2) = (half(rn, false) * half(m, false), half(rn, true) * half(m, true));
                let mut s = if matches!(i.op, Op::SMUAD | Op::SMLAD) { p1 + p2 } else { p1 - p2 };
                if matches!(i.op, Op::SMLAD | Op::SMLSD) {
                    s += r!(i.ra) as i32 as i64;
                }
                r!(i.rd) = s as u32;
                self.cpu.q |= s != s as i32 as i64;
            }
            Op::SMLAL_XY | Op::SMLALD | Op::SMLSLD | Op::UMAAL => {
                let lo = r!(i.rd);
                let hi = r!(i.ra);
                let acc = ((hi as u64) << 32 | lo as u64) as i64;
                let res: u64 = match i.op {
                    Op::SMLAL_XY => acc.wrapping_add(half(rn, i.aux & 1 != 0) * half(rm, i.aux & 2 != 0)) as u64,
                    Op::UMAAL => rn as u64 * rm as u64 + lo as u64 + hi as u64,
                    _ => {
                        let m = if i.aux & 1 != 0 { rm.rotate_right(16) } else { rm };
                        let (p1, p2) = (half(rn, false) * half(m, false), half(rn, true) * half(m, true));
                        acc.wrapping_add(if i.op == Op::SMLALD { p1 + p2 } else { p1 - p2 }) as u64
                    }
                };
                r!(i.rd) = res as u32;
                r!(i.ra) = (res >> 32) as u32;
            }
            Op::SMMUL | Op::SMMLA | Op::SMMLS => {
                let p = rn as i32 as i64 * rm as i32 as i64;
                let mut res = match i.op {
                    Op::SMMUL => p,
                    Op::SMMLA => ((r!(i.ra) as i32 as i64) << 32).wrapping_add(p),
                    _ => ((r!(i.ra) as i32 as i64) << 32).wrapping_sub(p),
                };
                if i.aux & 1 != 0 {
                    res = res.wrapping_add(0x8000_0000);
                }
                r!(i.rd) = (res >> 32) as u32;
            }
            _ => {}
        }
        self.cpu.cycles += 1;
    }

    // ---- floating point -----------------------------------------------------------------

    #[inline(always)]
    fn sreg(&self, n: u8) -> u32 {
        self.cpu.fpr[(n & 31) as usize]
    }

    #[inline(always)]
    fn dreg(&self, n: u8) -> u64 {
        let k = ((n & 15) * 2) as usize;
        self.cpu.fpr[k] as u64 | (self.cpu.fpr[k + 1] as u64) << 32
    }

    #[inline(always)]
    fn set_dreg(&mut self, n: u8, v: u64) {
        let k = ((n & 15) * 2) as usize;
        self.cpu.fpr[k] = v as u32;
        self.cpu.fpr[k + 1] = (v >> 32) as u32;
    }

    /// Source / destination register of the given precision as raw bits.
    #[inline(always)]
    fn fget(&self, n: u8, dp: bool) -> u64 {
        if dp {
            self.dreg(n)
        } else {
            self.sreg(n) as u64
        }
    }

    #[inline(always)]
    fn fset(&mut self, n: u8, dp: bool, v: u64) {
        if dp {
            self.set_dreg(n, v);
        } else {
            self.cpu.fpr[(n & 31) as usize] = v as u32;
        }
    }

    /// Value of an FP system register for VMRS.
    pub(crate) fn fp_sysreg(&self, reg: u32) -> u32 {
        let dp = self.cfg.features.has(mcs_core::arm::thumb::ArmFeatures::FPV5_DP);
        match reg {
            FPREG_FPSCR => self.cpu.fpscr,
            // Implementer ARM; subarchitecture FPv4 (Cortex-M4F) / FPv5 (Cortex-M7).
            FPREG_FPSID => 0x4100_0000 | if dp { 5 } else { 4 } << 16,
            FPREG_MVFR0 => {
                if dp {
                    0x1011_0221
                } else {
                    0x1011_0021
                }
            }
            FPREG_MVFR1 => {
                if dp {
                    0x1200_0011
                } else {
                    0x1100_0011
                }
            }
            _ => 0x40, // MVFR2: VFP miscellaneous (VSEL, VMAXNM, VRINT*, VCVT{A,N,P,M})
        }
    }

    /// Executes a floating-point instruction.
    fn exec_fp(&mut self, i: &Insn, pc: u32) {
        // CPACR.CP10 (CP11 is programmed identically): 0b01 privileged only, 0b11 full access.
        let cp = (self.scb.cpacr >> 20) & 3;
        if cp != 3 && !(cp == 1 && self.cpu.privileged()) {
            self.cpu.cycles += 1;
            self.raise_fault(pc, EXC_USAGEFAULT, UFSR_NOCP);
            return;
        }
        if self.scb.fpccr & FPCCR_ASPEN != 0 {
            self.cpu.control |= CONTROL_FPCA;
        }
        let dp = i.aux & 1 != 0;
        let rd = i.rd;
        let (rn, rm) = (i.rn, i.rm);
        macro_rules! r {
            ($x:expr) => {
                self.cpu.r[($x & 15) as usize]
            };
        }
        let mut cycles = 1u64;
        match i.op {
            // ---- arithmetic -----------------------------------------------------------------
            Op::VADD | Op::VSUB => {
                let sub = i.op == Op::VSUB;
                if dp {
                    let v = add64(self.dreg(rn), self.dreg(rm), sub, &mut self.cpu.fpscr);
                    self.set_dreg(rd, v);
                } else {
                    self.cpu.fpr[rd as usize & 31] = add32(self.sreg(rn), self.sreg(rm), sub, &mut self.cpu.fpscr);
                }
            }
            Op::VMUL | Op::VNMUL => {
                let neg = i.op == Op::VNMUL;
                if dp {
                    let v = mul64(self.dreg(rn), self.dreg(rm), &mut self.cpu.fpscr) ^ ((neg as u64) << 63);
                    self.set_dreg(rd, v);
                } else {
                    self.cpu.fpr[rd as usize & 31] = mul32(self.sreg(rn), self.sreg(rm), &mut self.cpu.fpscr) ^ ((neg as u32) << 31);
                }
            }
            Op::VDIV => {
                if dp {
                    let v = div64(self.dreg(rn), self.dreg(rm), &mut self.cpu.fpscr);
                    self.set_dreg(rd, v);
                    cycles = 29;
                } else {
                    self.cpu.fpr[rd as usize & 31] = div32(self.sreg(rn), self.sreg(rm), &mut self.cpu.fpscr);
                    cycles = 14;
                }
            }
            Op::VSQRT => {
                if dp {
                    let v = sqrt64(self.dreg(rm), &mut self.cpu.fpscr);
                    self.set_dreg(rd, v);
                    cycles = 29;
                } else {
                    self.cpu.fpr[rd as usize & 31] = sqrt32(self.sreg(rm), &mut self.cpu.fpscr);
                    cycles = 14;
                }
            }
            // Non-fused multiply-accumulate: the product is rounded before the addition.
            Op::VMLA | Op::VMLS | Op::VNMLA | Op::VNMLS => {
                cycles = 3;
                let (neg_p, neg_d) = match i.op {
                    Op::VMLA => (false, false),
                    Op::VMLS => (true, false),
                    Op::VNMLA => (true, true),
                    _ => (false, true),
                };
                if dp {
                    let p = mul64(self.dreg(rn), self.dreg(rm), &mut self.cpu.fpscr) ^ ((neg_p as u64) << 63);
                    let d = self.dreg(rd) ^ ((neg_d as u64) << 63);
                    let v = add64(d, p, false, &mut self.cpu.fpscr);
                    self.set_dreg(rd, v);
                } else {
                    let p = mul32(self.sreg(rn), self.sreg(rm), &mut self.cpu.fpscr) ^ ((neg_p as u32) << 31);
                    let d = self.sreg(rd) ^ ((neg_d as u32) << 31);
                    self.cpu.fpr[rd as usize & 31] = add32(d, p, false, &mut self.cpu.fpscr);
                }
            }
            // Fused multiply-accumulate: a single rounding.
            Op::VFMA | Op::VFMS | Op::VFNMA | Op::VFNMS => {
                cycles = 3;
                let (neg_n, neg_d) = match i.op {
                    Op::VFMA => (false, false),
                    Op::VFMS => (true, false),
                    Op::VFNMA => (true, true),
                    _ => (false, true),
                };
                if dp {
                    let (d, n, m) = (self.dreg(rd) ^ ((neg_d as u64) << 63), self.dreg(rn) ^ ((neg_n as u64) << 63), self.dreg(rm));
                    let v = fma_soft(F64, d, n, m, &mut self.cpu.fpscr);
                    self.set_dreg(rd, v);
                } else {
                    let (d, n, m) = (self.sreg(rd) ^ ((neg_d as u32) << 31), self.sreg(rn) ^ ((neg_n as u32) << 31), self.sreg(rm));
                    self.cpu.fpr[rd as usize & 31] = fma_soft(F32, d as u64, n as u64, m as u64, &mut self.cpu.fpscr) as u32;
                }
            }
            Op::VABS | Op::VNEG => {
                let neg = i.op == Op::VNEG;
                if dp {
                    let v = self.dreg(rm);
                    self.set_dreg(rd, if neg { v ^ 1 << 63 } else { v & !(1 << 63) });
                } else {
                    let v = self.sreg(rm);
                    self.cpu.fpr[rd as usize & 31] = if neg { v ^ 1 << 31 } else { v & !(1 << 31) };
                }
            }
            Op::VCMP | Op::VCMPE => {
                let f = if dp { F64 } else { F32 };
                let a = self.fget(rn, dp);
                let b = if i.s == 1 { 0 } else { self.fget(rm, dp) };
                let nzcv = compare(f, a, b, i.op == Op::VCMPE, &mut self.cpu.fpscr);
                self.cpu.fpscr = (self.cpu.fpscr & 0x0fff_ffff) | nzcv;
            }
            Op::VMAXNM | Op::VMINNM => {
                let f = if dp { F64 } else { F32 };
                let v = max_min_num(f, self.fget(rn, dp), self.fget(rm, dp), i.op == Op::VMAXNM, &mut self.cpu.fpscr);
                self.fset(rd, dp, v);
            }
            Op::VSEL => {
                let c = &self.cpu;
                let take_n = match i.s {
                    0 => c.z,
                    1 => c.v,
                    2 => c.n == c.v,
                    _ => !c.z && c.n == c.v,
                };
                let v = self.fget(if take_n { rn } else { rm }, dp);
                self.fset(rd, dp, v);
            }
            Op::VRINT => {
                let f = if dp { F64 } else { F32 };
                let fp_mode = Round::from_fpscr(self.cpu.fpscr);
                let (rm_, exact) = match i.s {
                    RI_R => (fp_mode, false),
                    RI_Z => (Round::Zero, false),
                    RI_X => (fp_mode, true),
                    RI_A => (Round::Away, false),
                    RI_N => (Round::Nearest, false),
                    RI_P => (Round::PlusInf, false),
                    _ => (Round::MinusInf, false),
                };
                let v = round_int(f, self.fget(rm, dp), rm_, exact, &mut self.cpu.fpscr);
                self.fset(rd, dp, v);
            }
            // ---- conversions ----------------------------------------------------------------
            Op::VCVT_FI => {
                let f = if dp { F64 } else { F32 };
                let mode = match i.s {
                    FR_ZERO => Round::Zero,
                    FR_FPSCR => Round::from_fpscr(self.cpu.fpscr),
                    FR_AWAY => Round::Away,
                    FR_NEAREST => Round::Nearest,
                    FR_PLUS => Round::PlusInf,
                    _ => Round::MinusInf,
                };
                let v = to_int(f, self.fget(rm, dp), i.ra != 0, 32, 0, mode, &mut self.cpu.fpscr);
                self.cpu.fpr[rd as usize & 31] = v;
            }
            Op::VCVT_IF => {
                let f = if dp { F64 } else { F32 };
                let x = self.sreg(rm);
                let (neg, mag) = if i.ra != 0 { ((x as i32) < 0, (x as i32).unsigned_abs() as u64) } else { (false, x as u64) };
                let mode = Round::from_fpscr(self.cpu.fpscr);
                let v = from_int(f, neg, mag, 0, mode, &mut self.cpu.fpscr);
                self.fset(rd, dp, v);
            }
            Op::VCVT_FX => {
                let f = if dp { F64 } else { F32 };
                let size = if i.aux & 4 != 0 { 32 } else { 16 };
                let signed = i.ra != 0;
                if i.aux & 2 != 0 {
                    let v = to_int(f, self.fget(rd, dp), signed, size, i.imm, Round::Zero, &mut self.cpu.fpscr);
                    if dp {
                        let ext = if signed { v as i32 as i64 as u64 } else { v as u64 };
                        self.set_dreg(rd, ext);
                    } else {
                        self.cpu.fpr[rd as usize & 31] = v;
                    }
                } else {
                    let raw = self.sreg(if dp { rd * 2 } else { rd });
                    let x = if size == 16 {
                        if signed {
                            raw as i16 as i32 as u32
                        } else {
                            raw & 0xffff
                        }
                    } else {
                        raw
                    };
                    let (neg, mag) = if signed { ((x as i32) < 0, (x as i32).unsigned_abs() as u64) } else { (false, x as u64) };
                    // Fixed point -> float always rounds to nearest (ARM ARM VCVT, fixed-point form).
                    let v = from_int(f, neg, mag, i.imm, Round::Nearest, &mut self.cpu.fpscr);
                    self.fset(rd, dp, v);
                }
            }
            Op::VCVT_DS => {
                if dp {
                    let v = convert(F32, F64, self.sreg(rm) as u64, &mut self.cpu.fpscr);
                    self.set_dreg(rd, v);
                } else {
                    let v = convert(F64, F32, self.dreg(rm), &mut self.cpu.fpscr);
                    self.cpu.fpr[rd as usize & 31] = v as u32;
                }
            }
            Op::VCVTB | Op::VCVTT => {
                let top = i.op == Op::VCVTT;
                let wide_dp = i.aux & 2 != 0;
                let wide = if wide_dp { F64 } else { F32 };
                if i.aux & 1 != 0 {
                    // float -> half, into one half of Sd
                    let h = float_to_half(wide, self.fget(rm, wide_dp), &mut self.cpu.fpscr) as u32;
                    let old = self.sreg(rd);
                    self.cpu.fpr[rd as usize & 31] = if top { (old & 0xffff) | h << 16 } else { (old & 0xffff_0000) | h };
                } else {
                    let raw = self.sreg(rm);
                    let h = if top { raw >> 16 } else { raw & 0xffff } as u16;
                    let v = half_to_float(h, wide, &mut self.cpu.fpscr);
                    self.fset(rd, wide_dp, v);
                }
            }
            // ---- moves ----------------------------------------------------------------------
            Op::VMOV_I => {
                if dp {
                    self.set_dreg(rd, (i.imm as u64) << 32);
                } else {
                    self.cpu.fpr[rd as usize & 31] = i.imm;
                }
            }
            Op::VMOV_F => {
                let v = self.fget(rm, dp);
                self.fset(rd, dp, v);
            }
            Op::VMOV_RS => {
                r!(rd) = self.sreg(rm);
            }
            Op::VMOV_SR => {
                self.cpu.fpr[rm as usize & 31] = r!(rd);
            }
            Op::VMOV_2S => {
                cycles = 2;
                let k = (rm & 31) as usize;
                if i.aux & 1 != 0 {
                    r!(rd) = self.cpu.fpr[k];
                    r!(rn) = self.cpu.fpr[(k + 1) & 31];
                } else {
                    self.cpu.fpr[k] = r!(rd);
                    self.cpu.fpr[(k + 1) & 31] = r!(rn);
                }
            }
            Op::VMOV_D2 => {
                cycles = 2;
                if i.aux & 1 != 0 {
                    let v = self.dreg(rm);
                    r!(rd) = v as u32;
                    r!(rn) = (v >> 32) as u32;
                } else {
                    let v = r!(rd) as u64 | (r!(rn) as u64) << 32;
                    self.set_dreg(rm, v);
                }
            }
            Op::VMOV_SC => {
                let k = ((rm & 15) * 2 + i.amt) as usize;
                if i.aux & 1 != 0 {
                    r!(rd) = self.cpu.fpr[k];
                } else {
                    self.cpu.fpr[k] = r!(rd);
                }
            }
            Op::VMRS => {
                let v = self.fp_sysreg(i.imm);
                if rd == 15 {
                    self.cpu.n = v & PSR_N != 0;
                    self.cpu.z = v & PSR_Z != 0;
                    self.cpu.c = v & PSR_C != 0;
                    self.cpu.v = v & PSR_V != 0;
                } else {
                    r!(rd) = v;
                }
            }
            Op::VMSR => {
                self.cpu.fpscr = r!(rd) & FPSCR_WMASK;
            }
            // ---- loads and stores -----------------------------------------------------------
            Op::VLDR | Op::VSTR => {
                cycles = 2;
                let base = if rn == 15 { pc.wrapping_add(4) & !3 } else { r!(rn) };
                let a = base.wrapping_add(i.imm);
                if a & 3 != 0 {
                    self.cpu.cycles += cycles;
                    self.raise_fault(pc, EXC_USAGEFAULT, UFSR_UNALIGNED);
                    return;
                }
                if i.op == Op::VLDR {
                    let Some(lo) = self.mem_read(a, 4) else { return self.data_fault(pc, a) };
                    if dp {
                        let Some(hi) = self.mem_read(a.wrapping_add(4), 4) else { return self.data_fault(pc, a.wrapping_add(4)) };
                        self.set_dreg(rd, lo as u64 | (hi as u64) << 32);
                    } else {
                        self.cpu.fpr[rd as usize & 31] = lo;
                    }
                } else {
                    let v = self.fget(rd, dp);
                    if !self.mem_write(a, 4, v as u32) {
                        return self.data_fault(pc, a);
                    }
                    if dp && !self.mem_write(a.wrapping_add(4), 4, (v >> 32) as u32) {
                        return self.data_fault(pc, a.wrapping_add(4));
                    }
                }
            }
            Op::VLDM | Op::VSTM | Op::VPUSH | Op::VPOP => {
                let words = i.imm * if dp { 2 } else { 1 };
                let (load, db, wb, base_reg) = match i.op {
                    Op::VLDM => (true, i.aux & 4 != 0, i.aux & 2 != 0, rn),
                    Op::VSTM => (false, i.aux & 4 != 0, i.aux & 2 != 0, rn),
                    Op::VPOP => (true, false, true, 13),
                    _ => (false, true, true, 13),
                };
                let base = r!(base_reg);
                let start = if db { base.wrapping_sub(4 * words) } else { base };
                cycles = 1 + words as u64;
                if start & 3 != 0 {
                    self.cpu.cycles += cycles;
                    self.raise_fault(pc, EXC_USAGEFAULT, UFSR_UNALIGNED);
                    return;
                }
                let first = (if dp { rd as u32 * 2 } else { rd as u32 }) as usize;
                for k in 0..words as usize {
                    let a = start.wrapping_add(4 * k as u32);
                    if load {
                        let Some(v) = self.mem_read(a, 4) else {
                            self.cpu.cycles += cycles;
                            return self.data_fault(pc, a);
                        };
                        self.cpu.fpr[(first + k) & 31] = v;
                    } else {
                        let v = self.cpu.fpr[(first + k) & 31];
                        if !self.mem_write(a, 4, v) {
                            self.cpu.cycles += cycles;
                            return self.data_fault(pc, a);
                        }
                    }
                }
                if wb {
                    r!(base_reg) = if db { start } else { base.wrapping_add(4 * words) };
                }
            }
            _ => {}
        }
        self.cpu.cycles += cycles;
    }
}

use super::cpu::{PSR_C, PSR_N, PSR_V, PSR_Z};
use super::nvic::EXC_USAGEFAULT;
